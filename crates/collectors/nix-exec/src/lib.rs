#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Live IO transport for Nix flake app preflight.
//!
//! This transport powers the `nix` namespace group.
//!
//! Purpose:
//! - Resolve a flake app reference (e.g. `github:org/repo#app`) into a pinned Nix store program path.
//! - Optionally realize/build the store path so subsequent execution can use `exec` (`run_program_v1`).
//!
//! Security notes:
//! - Do not echo request payloads in error messages (avoid accidental secret leakage).
//! - `nix` may access the network in Live mode; the resulting *outputs* must still be recorded
//!   as facts via the engine (handled by `LiveIo`).
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_nix_exec::{NAMESPACE_NIX_EXEC, NixFlakePolicy, NixFlakeTransportFactory};
//! use mfm_machine::live_io::LiveIoTransportFactory;
//!
//! let factory = NixFlakeTransportFactory::new(NixFlakePolicy {
//!     allow_prefixes: vec!["github:willyrgf/mfm".to_string(), "path:.".to_string()],
//! });
//!
//! assert_eq!(factory.namespace_group(), NAMESPACE_NIX_EXEC);
//! ```
#![warn(missing_docs)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use mfm_machine::config::RunManifest;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::events::{Event, KernelEvent};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::process_exec::{run_command, ProcessRunError, ProcessRunResult, StreamLimit};
use mfm_machine::stores::{ArtifactStore, StreamId, StreamStore};

/// Namespace group handled by the Nix flake transport factory.
pub const NAMESPACE_NIX_EXEC: &str = "nix.exec";

const CODE_NIX_REQUEST_INVALID: &str = "nix_request_invalid";
const CODE_NIX_APP_NOT_ALLOWED: &str = "nix_app_not_allowed";
const CODE_NIX_MANIFEST_LOOKUP_FAILED: &str = "nix_manifest_lookup_failed";
const CODE_NIX_MANIFEST_INVALID: &str = "nix_manifest_invalid";
const CODE_NIX_EVAL_FAILED: &str = "nix_eval_failed";
const CODE_NIX_BUILD_FAILED: &str = "nix_build_failed";
const CODE_NIX_RUN_FAILED: &str = "nix_run_failed";
const CODE_NIX_HOST_ENV_MISSING: &str = "nix_host_env_missing";
const CODE_NIX_TIMEOUT: &str = "nix_timeout";
const CODE_NIX_STDOUT_TOO_LARGE: &str = "nix_stdout_too_large";
const CODE_NIX_STDERR_TOO_LARGE: &str = "nix_stderr_too_large";
const CODE_NIX_STDOUT_INVALID_JSON: &str = "nix_stdout_invalid_json";
const CODE_NIX_STDIN_WRITE_FAILED: &str = "nix_stdin_write_failed";
const MAX_STDERR_DETAIL_BYTES: usize = 4096;
const MAX_STDOUT_DETAIL_BYTES: usize = 1024;
const MAX_NIX_STDOUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_NIX_STDERR_BYTES: usize = 4 * 1024 * 1024;

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn info_with_details(
    code: &'static str,
    category: ErrorCategory,
    message: &'static str,
    details: serde_json::Value,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: Some(details),
    }
}

/// Allowlist policy for flake references that may be resolved at runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NixFlakePolicy {
    /// Allowlisted flake ref prefixes.
    ///
    /// Examples:
    /// - `github:willyrgf/mfm` allows `github:willyrgf/mfm#jq_fmt_example`
    /// - `path:.` allows `path:.#aave-v3-origin-fetch` and resolves against
    ///   `MFM_WORKSPACE_ROOT` when present
    pub allow_prefixes: Vec<String>,
}

impl Default for NixFlakePolicy {
    fn default() -> Self {
        Self {
            // Conservative default: only allow this repo, either via its published flake ref or
            // via the current checkout root for internal workflow backends.
            allow_prefixes: vec!["github:willyrgf/mfm".to_string(), "path:.".to_string()],
        }
    }
}

/// Live transport factory that resolves flake apps into realized store programs.
#[derive(Clone, Default)]
pub struct NixFlakeTransportFactory {
    policy: NixFlakePolicy,
}

impl NixFlakeTransportFactory {
    /// Creates a new factory with the supplied allowlist policy.
    pub fn new(policy: NixFlakePolicy) -> Self {
        Self { policy }
    }

    /// Builds a factory from environment-derived configuration.
    pub fn from_env() -> Self {
        Self::default()
    }
}

impl LiveIoTransportFactory for NixFlakeTransportFactory {
    fn namespace_group(&self) -> &str {
        NAMESPACE_NIX_EXEC
    }

    fn make(&self, env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(NixFlakeTransport {
            fallback_policy: self.policy.clone(),
            streams: Arc::clone(&env.stores.streams),
            artifacts: Arc::clone(&env.stores.artifacts),
            run_id: env.run_id,
            resolved_policy: None,
        })
    }
}

struct NixFlakeTransport {
    fallback_policy: NixFlakePolicy,
    streams: Arc<dyn StreamStore>,
    artifacts: Arc<dyn ArtifactStore>,
    run_id: mfm_machine::ids::RunId,
    resolved_policy: Option<NixFlakePolicy>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolveFlakeAppV1 {
    app: String,
    timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RunFlakeAppV1 {
    app: String,
    argv: Vec<String>,
    stdin_json: serde_json::Value,
    timeout_ms: u64,
    env: std::collections::HashMap<String, String>,
    host_env_bindings: std::collections::HashMap<String, String>,
}

fn flake_ref_allowed(policy: &NixFlakePolicy, app: &str) -> bool {
    policy
        .allow_prefixes
        .iter()
        .any(|prefix| flake_ref_matches_prefix(prefix, app))
}

fn flake_ref_matches_prefix(prefix: &str, app: &str) -> bool {
    if !app.starts_with(prefix) {
        return false;
    }

    let suffix = &app[prefix.len()..];
    suffix.is_empty()
        || suffix.starts_with('#')
        || suffix.starts_with('?')
        || suffix.starts_with('/')
}

fn nix_system() -> String {
    let arch = std::env::consts::ARCH;
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{arch}-{os}")
}

fn split_flake_app_ref(app: &str) -> Result<(&str, &str), IoError> {
    let ref_parts = FlakeAppRef::parse(app).ok_or_else(|| {
        IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "flake app ref must contain a non-empty URL and fragment",
        ))
    })?;
    Ok((ref_parts.url, ref_parts.fragment))
}

fn attr_path_for_program(system: &str, fragment: &str) -> String {
    let frag = fragment.trim();
    if frag.ends_with(".program") {
        frag.to_string()
    } else if frag.starts_with("apps.") {
        format!("{frag}.program")
    } else {
        format!("apps.{system}.{frag}.program")
    }
}

fn flake_installable_target(flake_url: &str, attr: &str) -> String {
    format!("{flake_url}#{attr}")
}

fn repo_local_workspace_root() -> Result<std::path::PathBuf, IoError> {
    let root = match std::env::var_os("MFM_WORKSPACE_ROOT") {
        Some(value) if !value.is_empty() => std::path::PathBuf::from(value),
        _ => std::env::current_dir().map_err(|_| {
            IoError::Transport(info(
                CODE_NIX_REQUEST_INVALID,
                ErrorCategory::Unknown,
                "failed to resolve workspace root for repo-local flake ref",
            ))
        })?,
    };

    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(root))
            .map_err(|_| {
                IoError::Transport(info(
                    CODE_NIX_REQUEST_INVALID,
                    ErrorCategory::Unknown,
                    "failed to resolve workspace root for repo-local flake ref",
                ))
            })?
    };

    Ok(std::fs::canonicalize(&root).unwrap_or(root))
}

fn rewrite_repo_local_flake_ref_with_root(
    app: &str,
    workspace_root: &Path,
) -> Result<String, IoError> {
    let Some(suffix) = app.strip_prefix("path:.") else {
        return Ok(app.to_string());
    };

    if !(suffix.is_empty()
        || suffix.starts_with('#')
        || suffix.starts_with('?')
        || suffix.starts_with('/'))
    {
        return Ok(app.to_string());
    }

    let root = workspace_root.to_str().ok_or_else(|| {
        IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "workspace root for repo-local flake ref must be utf-8",
        ))
    })?;

    Ok(format!("path:{root}{suffix}"))
}

fn rewrite_repo_local_flake_ref(app: &str) -> Result<String, IoError> {
    let root = repo_local_workspace_root()?;
    rewrite_repo_local_flake_ref_with_root(app, &root)
}

fn store_root_from_program_path(program_path: &str) -> Option<String> {
    let rest = program_path.strip_prefix("/nix/store/")?;
    let entry = rest.split_once('/').map(|(entry, _)| entry).unwrap_or(rest);
    if entry.is_empty() {
        return None;
    }
    Some(format!("/nix/store/{entry}"))
}

#[derive(Debug, Clone, Copy)]
struct FlakeAppRef<'a> {
    url: &'a str,
    fragment: &'a str,
}

impl<'a> FlakeAppRef<'a> {
    fn parse(value: &'a str) -> Option<Self> {
        let (url, fragment) = value.split_once('#')?;
        if url.is_empty() || fragment.is_empty() {
            return None;
        }
        Some(Self { url, fragment })
    }
}

fn trim_command_output(bytes: &[u8], max_bytes: usize) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }

    let (slice, truncated) = if bytes.len() > max_bytes {
        (&bytes[bytes.len() - max_bytes..], true)
    } else {
        (bytes, false)
    };
    let text = String::from_utf8_lossy(slice).trim().to_string();
    if text.is_empty() {
        return None;
    }
    if truncated {
        Some(format!("...[truncated]\n{text}"))
    } else {
        Some(text)
    }
}

fn command_failure_details(
    command: &str,
    target: &str,
    status_code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> serde_json::Value {
    let mut details = serde_json::Map::new();
    details.insert(
        "command".to_string(),
        serde_json::Value::String(command.to_string()),
    );
    details.insert(
        "target".to_string(),
        serde_json::Value::String(target.to_string()),
    );
    details.insert(
        "exit_code".to_string(),
        status_code
            .map(|code| serde_json::Value::Number(code.into()))
            .unwrap_or(serde_json::Value::Null),
    );
    if let Some(stderr_excerpt) = trim_command_output(stderr, MAX_STDERR_DETAIL_BYTES) {
        details.insert(
            "stderr".to_string(),
            serde_json::Value::String(stderr_excerpt),
        );
    }
    if let Some(stdout_excerpt) = trim_command_output(stdout, MAX_STDOUT_DETAIL_BYTES) {
        details.insert(
            "stdout".to_string(),
            serde_json::Value::String(stdout_excerpt),
        );
    }
    serde_json::Value::Object(details)
}

fn parse_resolve_request(call: &IoCall) -> Result<ResolveFlakeAppV1, IoError> {
    let obj = call.request.as_object().ok_or_else(|| {
        IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "nix request must be a JSON object",
        ))
    })?;

    let kind = obj.get("kind").and_then(|v| v.as_str()).ok_or_else(|| {
        IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "missing nix request kind",
        ))
    })?;

    if kind != "resolve_flake_app_v1" {
        return Err(IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "unsupported nix request kind",
        )));
    }

    let app = obj
        .get("app")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            IoError::Other(info(
                CODE_NIX_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "missing app",
            ))
        })?
        .to_string();

    let timeout_ms = obj
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(300_000);

    Ok(ResolveFlakeAppV1 { app, timeout_ms })
}

fn parse_run_request(call: &IoCall) -> Result<RunFlakeAppV1, IoError> {
    let obj = call.request.as_object().ok_or_else(|| {
        IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "nix request must be a JSON object",
        ))
    })?;

    let kind = obj.get("kind").and_then(|v| v.as_str()).ok_or_else(|| {
        IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "missing nix request kind",
        ))
    })?;

    if kind != "run_flake_app_v1" {
        return Err(IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "unsupported nix request kind",
        )));
    }

    let app = obj
        .get("app")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            IoError::Other(info(
                CODE_NIX_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "missing app",
            ))
        })?
        .to_string();

    let argv = obj
        .get("argv")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let stdin_json = obj
        .get("stdin_json")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let timeout_ms = obj
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(300_000);

    let env = obj
        .get("env")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();
    let host_env_bindings = obj
        .get("host_env_bindings")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();

    Ok(RunFlakeAppV1 {
        app,
        argv,
        stdin_json,
        timeout_ms,
        env,
        host_env_bindings,
    })
}

fn inject_host_env_bindings(
    cmd: &mut Command,
    bindings: &std::collections::HashMap<String, String>,
) -> Result<(), IoError> {
    for (target_env, source_env) in bindings {
        let source_value = std::env::var(source_env).map_err(|_| {
            IoError::Transport(info_with_details(
                CODE_NIX_HOST_ENV_MISSING,
                ErrorCategory::Unknown,
                "required host env was missing for nix run",
                serde_json::json!({
                    "target_env": target_env,
                    "source_env": source_env,
                }),
            ))
        })?;
        if source_value.is_empty() {
            return Err(IoError::Transport(info_with_details(
                CODE_NIX_HOST_ENV_MISSING,
                ErrorCategory::Unknown,
                "required host env was empty for nix run",
                serde_json::json!({
                    "target_env": target_env,
                    "source_env": source_env,
                }),
            )));
        }
        cmd.env(target_env, source_value);
    }

    Ok(())
}

fn run_started_manifest_id(
    stream: &[mfm_machine::events::EventEnvelope],
) -> Option<mfm_machine::ids::ArtifactId> {
    for e in stream {
        if let Event::Kernel(KernelEvent::RunStarted { manifest_id, .. }) = &e.event {
            return Some(manifest_id.clone());
        }
    }
    None
}

impl NixFlakeTransport {
    async fn load_manifest_policy(&self) -> Result<Option<NixFlakePolicy>, IoError> {
        let stream = self
            .streams
            .read_range(&StreamId::run(self.run_id), 1, None)
            .await
            .map_err(|_| {
                IoError::Other(info(
                    CODE_NIX_MANIFEST_LOOKUP_FAILED,
                    ErrorCategory::Storage,
                    "failed to read run stream for nix policy",
                ))
            })?;
        let stream = mfm_machine::events::event_envelopes_from_stream_records(self.run_id, stream)
            .map_err(|_| {
                IoError::Other(info(
                    CODE_NIX_MANIFEST_LOOKUP_FAILED,
                    ErrorCategory::Storage,
                    "run stream was invalid",
                ))
            })?;

        let Some(manifest_id) = run_started_manifest_id(&stream) else {
            return Ok(None);
        };

        let bytes = self.artifacts.get(&manifest_id).await.map_err(|_| {
            IoError::Other(info(
                CODE_NIX_MANIFEST_LOOKUP_FAILED,
                ErrorCategory::Storage,
                "failed to read run manifest for nix policy",
            ))
        })?;

        let manifest = serde_json::from_slice::<RunManifest>(&bytes).map_err(|_| {
            IoError::Other(info(
                CODE_NIX_MANIFEST_INVALID,
                ErrorCategory::ParsingInput,
                "run manifest was invalid",
            ))
        })?;

        Ok(Some(NixFlakePolicy {
            allow_prefixes: manifest.run_config.nix_flake_allowlist,
        }))
    }

    async fn effective_policy(&mut self) -> Result<NixFlakePolicy, IoError> {
        if let Some(policy) = &self.resolved_policy {
            return Ok(policy.clone());
        }

        let policy = self
            .load_manifest_policy()
            .await?
            .unwrap_or_else(|| self.fallback_policy.clone());
        self.resolved_policy = Some(policy.clone());
        Ok(policy)
    }
}

async fn run_with_timeout(
    cmd: Command,
    timeout_ms: u64,
) -> Result<ProcessRunResult, ProcessRunError> {
    run_command(
        cmd,
        None,
        Duration::from_millis(timeout_ms),
        StreamLimit {
            max_stdout_bytes: MAX_NIX_STDOUT_BYTES,
            max_stderr_bytes: MAX_NIX_STDERR_BYTES,
        },
    )
    .await
}

async fn run_with_input_and_timeout(
    cmd: Command,
    stdin_bytes: Option<Vec<u8>>,
    timeout_ms: u64,
) -> Result<ProcessRunResult, ProcessRunError> {
    run_command(
        cmd,
        stdin_bytes,
        Duration::from_millis(timeout_ms),
        StreamLimit {
            max_stdout_bytes: MAX_NIX_STDOUT_BYTES,
            max_stderr_bytes: MAX_NIX_STDERR_BYTES,
        },
    )
    .await
}

fn map_runner_error(
    err: ProcessRunError,
    failure_code: &'static str,
    command: &str,
    target: &str,
    timeout_ms: u64,
) -> IoError {
    match err {
        ProcessRunError::SpawnFailed => IoError::Transport(info_with_details(
            failure_code,
            ErrorCategory::Unknown,
            "failed to spawn nix command",
            serde_json::json!({
                "command": command,
                "target": target,
            }),
        )),
        ProcessRunError::Timeout => IoError::Transport(info_with_details(
            CODE_NIX_TIMEOUT,
            ErrorCategory::Unknown,
            "nix command timed out",
            serde_json::json!({
                "command": command,
                "target": target,
                "timeout_ms": timeout_ms,
            }),
        )),
        ProcessRunError::WaitFailed
        | ProcessRunError::StdoutReadFailed
        | ProcessRunError::StderrReadFailed => IoError::Transport(info_with_details(
            failure_code,
            ErrorCategory::Unknown,
            "nix command failed",
            serde_json::json!({
                "command": command,
                "target": target,
            }),
        )),
    }
}

fn ensure_bounded_output(
    command: &str,
    target: &str,
    out: &ProcessRunResult,
) -> Result<(), IoError> {
    if out.stdout.overflowed {
        return Err(IoError::Transport(info_with_details(
            CODE_NIX_STDOUT_TOO_LARGE,
            ErrorCategory::Unknown,
            "nix command stdout exceeded maximum size",
            serde_json::json!({
                "command": command,
                "target": target,
                "max_stdout_bytes": MAX_NIX_STDOUT_BYTES,
                "stdout_bytes": out.stdout.total_bytes,
            }),
        )));
    }
    if out.stderr.overflowed {
        return Err(IoError::Transport(info_with_details(
            CODE_NIX_STDERR_TOO_LARGE,
            ErrorCategory::Unknown,
            "nix command stderr exceeded maximum size",
            serde_json::json!({
                "command": command,
                "target": target,
                "max_stderr_bytes": MAX_NIX_STDERR_BYTES,
                "stderr_bytes": out.stderr.total_bytes,
            }),
        )));
    }
    Ok(())
}

fn ensure_flake_app_allowed(policy: &NixFlakePolicy, app: &str) -> Result<String, IoError> {
    if !flake_ref_allowed(policy, app) {
        return Err(IoError::Other(info(
            CODE_NIX_APP_NOT_ALLOWED,
            ErrorCategory::Unknown,
            "flake app ref is not allowed by policy",
        )));
    }
    rewrite_repo_local_flake_ref(app)
}

#[async_trait]
impl LiveIoTransport for NixFlakeTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let policy = self.effective_policy().await?;
        let kind = call
            .request
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                IoError::Other(info(
                    CODE_NIX_REQUEST_INVALID,
                    ErrorCategory::ParsingInput,
                    "missing nix request kind",
                ))
            })?;

        match kind {
            "resolve_flake_app_v1" => {
                let req = parse_resolve_request(&call)?;
                let resolved_app = ensure_flake_app_allowed(&policy, &req.app)?;
                let (flake_url, fragment) = split_flake_app_ref(&resolved_app)?;
                let system = nix_system();
                let program_attr = attr_path_for_program(&system, fragment);
                let target = flake_installable_target(flake_url, &program_attr);

                let mut eval = Command::new("nix");
                eval.arg("eval")
                    .arg("--option")
                    .arg("eval-cache")
                    .arg("false")
                    .arg("--raw")
                    .arg("--apply")
                    .arg("builtins.unsafeDiscardStringContext")
                    .arg("--no-write-lock-file")
                    .arg(&target);

                let out = run_with_timeout(eval, req.timeout_ms)
                    .await
                    .map_err(|err| {
                        map_runner_error(
                            err,
                            CODE_NIX_EVAL_FAILED,
                            "nix eval",
                            &target,
                            req.timeout_ms,
                        )
                    })?;
                ensure_bounded_output("nix eval", &target, &out)?;
                if !out.status.success() {
                    return Err(IoError::Transport(info_with_details(
                        CODE_NIX_EVAL_FAILED,
                        ErrorCategory::Unknown,
                        "nix eval failed",
                        command_failure_details(
                            "nix eval",
                            &target,
                            out.status.code(),
                            &out.stdout.bytes,
                            &out.stderr.bytes,
                        ),
                    )));
                }

                let program_path = String::from_utf8_lossy(&out.stdout.bytes)
                    .trim()
                    .to_string();
                if !program_path.starts_with("/nix/store/") {
                    return Err(IoError::Other(info(
                        CODE_NIX_REQUEST_INVALID,
                        ErrorCategory::ParsingInput,
                        "resolved program path did not start with /nix/store/",
                    )));
                }
                if store_root_from_program_path(&program_path).is_none() {
                    return Err(IoError::Other(info(
                        CODE_NIX_REQUEST_INVALID,
                        ErrorCategory::ParsingInput,
                        "resolved program path was not a valid nix store path",
                    )));
                }

                if !Path::new(&program_path).exists() {
                    let build_target = flake_installable_target(
                        flake_url,
                        &store_root_from_program_path(&program_path).ok_or_else(|| {
                            IoError::Transport(info_with_details(
                                CODE_NIX_MANIFEST_INVALID,
                                ErrorCategory::ParsingInput,
                                "resolved flake app program path did not map to a store root",
                                serde_json::json!({
                                    "command": "nix eval",
                                    "target": target,
                                    "program_path": program_path,
                                }),
                            ))
                        })?,
                    );
                    let mut build = Command::new("nix");
                    build
                        .arg("build")
                        .arg("--no-link")
                        .arg("--no-write-lock-file")
                        .arg(&build_target);

                    let out = run_with_timeout(build, req.timeout_ms)
                        .await
                        .map_err(|err| {
                            map_runner_error(
                                err,
                                CODE_NIX_BUILD_FAILED,
                                "nix build",
                                &build_target,
                                req.timeout_ms,
                            )
                        })?;
                    ensure_bounded_output("nix build", &build_target, &out)?;
                    if !out.status.success() {
                        return Err(IoError::Transport(info_with_details(
                            CODE_NIX_BUILD_FAILED,
                            ErrorCategory::Unknown,
                            "nix build failed",
                            command_failure_details(
                                "nix build",
                                &build_target,
                                out.status.code(),
                                &out.stdout.bytes,
                                &out.stderr.bytes,
                            ),
                        )));
                    }
                    if !Path::new(&program_path).exists() {
                        return Err(IoError::Transport(info_with_details(
                            CODE_NIX_BUILD_FAILED,
                            ErrorCategory::Unknown,
                            "nix build did not realize resolved program path",
                            serde_json::json!({
                                "command": "nix build",
                                "target": build_target,
                                "expected_program_path": program_path,
                            }),
                        )));
                    }
                }

                Ok(serde_json::json!({"program_path": program_path}))
            }
            "run_flake_app_v1" => {
                let req = parse_run_request(&call)?;
                let resolved_app = ensure_flake_app_allowed(&policy, &req.app)?;
                let mut cmd = Command::new("nix");
                cmd.arg("run")
                    .arg("--no-write-lock-file")
                    .arg(&resolved_app)
                    .arg("--");
                cmd.args(&req.argv);
                for (key, value) in &req.env {
                    cmd.env(key, value);
                }
                inject_host_env_bindings(&mut cmd, &req.host_env_bindings)?;

                let stdin_bytes = serde_json::to_vec(&req.stdin_json).map_err(|_| {
                    IoError::Other(info(
                        CODE_NIX_REQUEST_INVALID,
                        ErrorCategory::ParsingInput,
                        "nix run stdin_json could not be serialized",
                    ))
                })?;

                let out = run_with_input_and_timeout(cmd, Some(stdin_bytes), req.timeout_ms)
                    .await
                    .map_err(|err| {
                        map_runner_error(
                            err,
                            CODE_NIX_RUN_FAILED,
                            "nix run",
                            &resolved_app,
                            req.timeout_ms,
                        )
                    })?;
                ensure_bounded_output("nix run", &resolved_app, &out)?;

                if let Some(stdin_err) = out.stdin_write_error {
                    if stdin_err.kind != std::io::ErrorKind::BrokenPipe {
                        return Err(IoError::Transport(info_with_details(
                            CODE_NIX_STDIN_WRITE_FAILED,
                            ErrorCategory::Unknown,
                            "failed to write nix run stdin",
                            serde_json::json!({
                                "command": "nix run",
                                "target": resolved_app,
                                "io_error_kind": format!("{:?}", stdin_err.kind),
                            }),
                        )));
                    }
                }

                if let Some(stdin_err) = out.stdin_close_error {
                    if stdin_err.kind != std::io::ErrorKind::BrokenPipe {
                        return Err(IoError::Transport(info_with_details(
                            CODE_NIX_STDIN_WRITE_FAILED,
                            ErrorCategory::Unknown,
                            "failed to close nix run stdin",
                            serde_json::json!({
                                "command": "nix run",
                                "target": resolved_app,
                                "io_error_kind": format!("{:?}", stdin_err.kind),
                            }),
                        )));
                    }
                }

                if !out.status.success() {
                    return Err(IoError::Transport(info_with_details(
                        CODE_NIX_RUN_FAILED,
                        ErrorCategory::Unknown,
                        "nix run failed",
                        command_failure_details(
                            "nix run",
                            &resolved_app,
                            out.status.code(),
                            &out.stdout.bytes,
                            &out.stderr.bytes,
                        ),
                    )));
                }

                let response = serde_json::from_slice::<serde_json::Value>(&out.stdout.bytes)
                    .map_err(|_| {
                        IoError::Other(info(
                            CODE_NIX_STDOUT_INVALID_JSON,
                            ErrorCategory::ParsingInput,
                            "nix run stdout was not valid JSON",
                        ))
                    })?;

                Ok(response)
            }
            _ => Err(IoError::Other(info(
                CODE_NIX_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "unsupported nix request kind",
            ))),
        }
    }
}

#[cfg(test)]
#[path = "tests/nix_exec_tests.rs"]
mod nix_exec_tests;
