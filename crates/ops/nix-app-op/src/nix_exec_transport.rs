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

use std::path::Path;
use std::process::Stdio;
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
use mfm_machine::stores::{ArtifactStore, EventStore};

pub const NAMESPACE_NIX_EXEC: &str = "nix.exec";

const CODE_NIX_REQUEST_INVALID: &str = "nix_request_invalid";
const CODE_NIX_APP_NOT_ALLOWED: &str = "nix_app_not_allowed";
const CODE_NIX_MANIFEST_LOOKUP_FAILED: &str = "nix_manifest_lookup_failed";
const CODE_NIX_MANIFEST_INVALID: &str = "nix_manifest_invalid";
const CODE_NIX_EVAL_FAILED: &str = "nix_eval_failed";
const CODE_NIX_BUILD_FAILED: &str = "nix_build_failed";
const CODE_NIX_TIMEOUT: &str = "nix_timeout";
const MAX_STDERR_DETAIL_BYTES: usize = 4096;
const MAX_STDOUT_DETAIL_BYTES: usize = 1024;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NixFlakePolicy {
    /// Allowlisted flake ref prefixes.
    ///
    /// Example: `github:willyrgf/mfm` allows `github:willyrgf/mfm#jq_fmt_example`.
    pub allow_prefixes: Vec<String>,
}

impl Default for NixFlakePolicy {
    fn default() -> Self {
        Self {
            // Conservative default: only allow this repo.
            allow_prefixes: vec!["github:willyrgf/mfm".to_string()],
        }
    }
}

#[derive(Clone, Default)]
pub struct NixFlakeTransportFactory {
    policy: NixFlakePolicy,
}

impl NixFlakeTransportFactory {
    pub fn new(policy: NixFlakePolicy) -> Self {
        Self { policy }
    }
}

impl LiveIoTransportFactory for NixFlakeTransportFactory {
    fn make(&self, env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(NixFlakeTransport {
            fallback_policy: self.policy.clone(),
            events: Arc::clone(&env.stores.events),
            artifacts: Arc::clone(&env.stores.artifacts),
            run_id: env.run_id,
            resolved_policy: None,
        })
    }
}

struct NixFlakeTransport {
    fallback_policy: NixFlakePolicy,
    events: Arc<dyn EventStore>,
    artifacts: Arc<dyn ArtifactStore>,
    run_id: mfm_machine::ids::RunId,
    resolved_policy: Option<NixFlakePolicy>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolveFlakeAppV1 {
    app: String,
    timeout_ms: u64,
}

fn flake_ref_allowed(policy: &NixFlakePolicy, app: &str) -> bool {
    policy.allow_prefixes.iter().any(|p| app.starts_with(p))
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
    let Some((flake_url, fragment)) = app.split_once('#') else {
        return Err(IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "flake app ref must contain a '#' fragment",
        )));
    };
    if flake_url.is_empty() || fragment.is_empty() {
        return Err(IoError::Other(info(
            CODE_NIX_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "flake app ref must have non-empty flake url and fragment",
        )));
    }
    Ok((flake_url, fragment))
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

fn store_root_from_program_path(program_path: &str) -> Option<String> {
    let rest = program_path.strip_prefix("/nix/store/")?;
    let (entry, _) = rest.split_once('/').unwrap_or((rest, ""));
    if entry.is_empty() {
        return None;
    }
    Some(format!("/nix/store/{entry}"))
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
            .events
            .read_range(self.run_id, 1, None)
            .await
            .map_err(|_| {
                IoError::Other(info(
                    CODE_NIX_MANIFEST_LOOKUP_FAILED,
                    ErrorCategory::Storage,
                    "failed to read run events for nix policy",
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
    mut cmd: Command,
    timeout_ms: u64,
) -> Result<std::process::Output, IoError> {
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let duration = Duration::from_millis(timeout_ms);
    let child = cmd.spawn().map_err(|_| {
        IoError::Transport(info(
            CODE_NIX_EVAL_FAILED,
            ErrorCategory::Unknown,
            "failed to spawn nix",
        ))
    })?;

    tokio::time::timeout(duration, child.wait_with_output())
        .await
        .map_err(|_| {
            IoError::Transport(info(
                CODE_NIX_TIMEOUT,
                ErrorCategory::Unknown,
                "nix command timed out",
            ))
        })?
        .map_err(|_| {
            IoError::Transport(info(
                CODE_NIX_EVAL_FAILED,
                ErrorCategory::Unknown,
                "nix command failed",
            ))
        })
}

#[async_trait]
impl LiveIoTransport for NixFlakeTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let req = parse_resolve_request(&call)?;
        let policy = self.effective_policy().await?;

        if !flake_ref_allowed(&policy, &req.app) {
            return Err(IoError::Other(info(
                CODE_NIX_APP_NOT_ALLOWED,
                ErrorCategory::Unknown,
                "flake app ref is not allowed by policy",
            )));
        }

        let (flake_url, fragment) = split_flake_app_ref(&req.app)?;
        let system = nix_system();
        let attr = attr_path_for_program(&system, fragment);
        let target = flake_installable_target(flake_url, &attr);

        // 1) Resolve the app program path.
        let mut eval = Command::new("nix");
        eval.arg("eval")
            .arg("--option")
            .arg("eval-cache")
            .arg("false")
            .arg("--raw")
            .arg("--no-write-lock-file")
            .arg(&target);

        let out = run_with_timeout(eval, req.timeout_ms).await?;
        if !out.status.success() {
            return Err(IoError::Transport(info_with_details(
                CODE_NIX_EVAL_FAILED,
                ErrorCategory::Unknown,
                "nix eval failed",
                command_failure_details(
                    "nix eval",
                    &target,
                    out.status.code(),
                    &out.stdout,
                    &out.stderr,
                ),
            )));
        }

        let program_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
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

        // 2) Realize the app when the resolved program path is not already present.
        if !Path::new(&program_path).exists() {
            let mut build = Command::new("nix");
            build
                .arg("build")
                .arg("--no-link")
                .arg("--no-write-lock-file")
                .arg(&target);

            let out = run_with_timeout(build, req.timeout_ms).await?;
            if !out.status.success() {
                return Err(IoError::Transport(info_with_details(
                    CODE_NIX_BUILD_FAILED,
                    ErrorCategory::Unknown,
                    "nix build failed",
                    command_failure_details(
                        "nix build",
                        &target,
                        out.status.code(),
                        &out.stdout,
                        &out.stderr,
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
                        "target": target,
                        "expected_program_path": program_path,
                    }),
                )));
            }
        }

        Ok(serde_json::json!({"program_path": program_path}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mfm_machine::config::{
        BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
        RetryPolicy, RunConfig, RunManifest,
    };
    use mfm_machine::engine::Stores;
    use mfm_machine::errors::StorageError;
    use mfm_machine::events::{Event, EventEnvelope, KernelEvent};
    use mfm_machine::ids::{ArtifactId, OpId, RunId, StateId};
    use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Clone)]
    struct NoopEventStore;

    #[async_trait]
    impl EventStore for NoopEventStore {
        async fn head_seq(&self, _run_id: RunId) -> Result<u64, StorageError> {
            Ok(0)
        }
        async fn append(
            &self,
            _run_id: RunId,
            _expected_seq: u64,
            _events: Vec<EventEnvelope>,
        ) -> Result<u64, StorageError> {
            Ok(0)
        }
        async fn read_range(
            &self,
            _run_id: RunId,
            _from_seq: u64,
            _to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, StorageError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone)]
    struct NoopArtifactStore;

    #[async_trait]
    impl ArtifactStore for NoopArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            Ok(ArtifactId("0".repeat(64)))
        }
        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }
        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(false)
        }
    }

    fn env() -> LiveIoEnv {
        LiveIoEnv {
            stores: Stores {
                events: Arc::new(NoopEventStore),
                artifacts: Arc::new(NoopArtifactStore),
            },
            run_id: RunId(uuid::Uuid::new_v4()),
            state_id: StateId("machine.main.s1".to_string()),
            attempt: 0,
        }
    }

    fn run_config_with_allowlist(prefixes: Vec<String>) -> RunConfig {
        RunConfig {
            io_mode: IoMode::Live,
            retry_policy: RetryPolicy {
                max_attempts: 1,
                backoff: BackoffPolicy::Fixed {
                    delay: std::time::Duration::from_millis(0),
                },
            },
            event_profile: EventProfile::Normal,
            execution_mode: ExecutionMode::Sequential,
            context_checkpointing: ContextCheckpointing::AfterEveryState,
            replay_missing_fact_retryable: false,
            skip_tags: Vec::new(),
            nix_flake_allowlist: prefixes,
        }
    }

    #[derive(Clone)]
    struct FixedEventStore {
        stream: Arc<Vec<EventEnvelope>>,
    }

    #[async_trait]
    impl EventStore for FixedEventStore {
        async fn head_seq(&self, _run_id: RunId) -> Result<u64, StorageError> {
            Ok(self.stream.last().map(|e| e.seq).unwrap_or(0))
        }

        async fn append(
            &self,
            _run_id: RunId,
            _expected_seq: u64,
            _events: Vec<EventEnvelope>,
        ) -> Result<u64, StorageError> {
            Ok(self.stream.last().map(|e| e.seq).unwrap_or(0))
        }

        async fn read_range(
            &self,
            _run_id: RunId,
            from_seq: u64,
            to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, StorageError> {
            let to = to_seq.unwrap_or(u64::MAX);
            Ok(self
                .stream
                .iter()
                .filter(|e| e.seq >= from_seq && e.seq <= to)
                .cloned()
                .collect())
        }
    }

    #[derive(Clone, Default)]
    struct FixedArtifactStore {
        blobs: Arc<Mutex<HashMap<ArtifactId, Vec<u8>>>>,
    }

    #[async_trait]
    impl ArtifactStore for FixedArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            let id = mfm_machine::hashing::artifact_id_for_bytes(&bytes);
            self.blobs.lock().await.insert(id.clone(), bytes);
            Ok(id)
        }

        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            let inner = self.blobs.lock().await;
            inner.get(id).cloned().ok_or_else(|| {
                StorageError::NotFound(info(
                    "not_found",
                    ErrorCategory::Storage,
                    "artifact not found",
                ))
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(self.blobs.lock().await.contains_key(id))
        }
    }

    async fn env_with_manifest_allowlist(prefixes: Vec<String>) -> LiveIoEnv {
        let run_id = RunId(uuid::Uuid::new_v4());
        let manifest_id = ArtifactId("1".repeat(64));
        let initial_snapshot_id = ArtifactId("2".repeat(64));

        let manifest = RunManifest {
            op_id: OpId("nix_app".to_string()),
            op_version: "v1".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config_with_allowlist(prefixes),
            build: BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };

        let mut blobs = HashMap::new();
        blobs.insert(
            manifest_id.clone(),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        );
        let artifacts = Arc::new(FixedArtifactStore {
            blobs: Arc::new(Mutex::new(blobs)),
        });

        let stream = vec![EventEnvelope {
            run_id,
            seq: 1,
            ts_millis: Some(0),
            event: Event::Kernel(KernelEvent::RunStarted {
                op_id: OpId("nix_app".to_string()),
                manifest_id,
                initial_snapshot_id,
            }),
        }];

        LiveIoEnv {
            stores: Stores {
                events: Arc::new(FixedEventStore {
                    stream: Arc::new(stream),
                }),
                artifacts,
            },
            run_id,
            state_id: StateId("machine.main.s1".to_string()),
            attempt: 0,
        }
    }

    #[test]
    fn split_flake_app_ref_requires_fragment() {
        let err = split_flake_app_ref("github:willyrgf/mfm").expect_err("expected error");
        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_NIX_REQUEST_INVALID),
            other => panic!("expected Other, got: {other:?}"),
        }
    }

    #[test]
    fn attr_path_defaults_to_apps_system_program() {
        let system = "aarch64-darwin";
        let got = attr_path_for_program(system, "jq_fmt_example");
        assert_eq!(got, "apps.aarch64-darwin.jq_fmt_example.program");
    }

    #[test]
    fn flake_installable_target_uses_attr_path() {
        let got = flake_installable_target("path:/repo", "apps.x86_64-linux.app.program");
        assert_eq!(got, "path:/repo#apps.x86_64-linux.app.program");
    }

    #[test]
    fn store_root_from_program_path_extracts_store_root() {
        let got = store_root_from_program_path("/nix/store/hash-app/bin/app");
        assert_eq!(got.as_deref(), Some("/nix/store/hash-app"));
    }

    #[test]
    fn store_root_from_program_path_rejects_non_store_paths() {
        assert!(store_root_from_program_path("/tmp/app").is_none());
        assert!(store_root_from_program_path("/nix/store/").is_none());
    }

    #[test]
    fn command_failure_details_include_exit_code_and_stderr() {
        let details = command_failure_details(
            "nix build",
            "path:/repo#apps.x86_64-linux.app.program",
            Some(100),
            b"",
            b"error: failed to fetch\n",
        );
        assert_eq!(
            details.get("command").and_then(|v| v.as_str()),
            Some("nix build")
        );
        assert_eq!(
            details.get("target").and_then(|v| v.as_str()),
            Some("path:/repo#apps.x86_64-linux.app.program")
        );
        assert_eq!(details.get("exit_code").and_then(|v| v.as_i64()), Some(100));
        assert!(details
            .get("stderr")
            .and_then(|v| v.as_str())
            .expect("stderr")
            .contains("failed to fetch"));
    }

    #[test]
    fn command_failure_details_truncate_large_stderr() {
        let huge = "x".repeat(MAX_STDERR_DETAIL_BYTES + 32);
        let details = command_failure_details(
            "nix eval",
            "path:/repo#apps.x86_64-linux.app.program",
            Some(1),
            b"",
            huge.as_bytes(),
        );
        let stderr = details
            .get("stderr")
            .and_then(|v| v.as_str())
            .expect("stderr");
        assert!(stderr.starts_with("...[truncated]"));
    }

    #[tokio::test]
    async fn rejects_disallowed_app_ref_by_default() {
        let factory = NixFlakeTransportFactory::default();
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: NAMESPACE_NIX_EXEC.to_string(),
                request: serde_json::json!({
                    "kind": "resolve_flake_app_v1",
                    "app": "github:someone/else#app",
                    "timeout_ms": 1
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_NIX_APP_NOT_ALLOWED),
            other => panic!("expected Other, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_invalid_request_shape() {
        let factory = NixFlakeTransportFactory::default();
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: NAMESPACE_NIX_EXEC.to_string(),
                request: serde_json::json!("not an object"),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_NIX_REQUEST_INVALID),
            other => panic!("expected Other, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manifest_allowlist_overrides_factory_policy() {
        let env = env_with_manifest_allowlist(vec!["path:/definitely-missing".to_string()]).await;
        let factory = NixFlakeTransportFactory::new(NixFlakePolicy {
            allow_prefixes: vec!["github:willyrgf/mfm".to_string()],
        });
        let mut t = factory.make(env);

        let err = t
            .call(IoCall {
                namespace: NAMESPACE_NIX_EXEC.to_string(),
                request: serde_json::json!({
                    "kind": "resolve_flake_app_v1",
                    "app": "path:/definitely-missing#jq_fmt_example",
                    "timeout_ms": 1000
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Transport(info) => {
                assert!(info.code.0 == CODE_NIX_EVAL_FAILED || info.code.0 == CODE_NIX_TIMEOUT)
            }
            IoError::Other(info) => {
                assert_ne!(info.code.0, CODE_NIX_APP_NOT_ALLOWED)
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
