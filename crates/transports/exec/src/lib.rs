#![warn(missing_docs)]
//! Live IO transport for external program execution.
//!
//! This transport powers the `exec` namespace. State logic should use typed requests from
//! `mfm-collectors-exec`; this crate owns only the live local process side effect.
//!
//! Security notes:
//! - Request payloads are not persisted by the runtime, but MUST still be treated as sensitive.
//! - Errors MUST NOT echo stdout/stderr or request payloads (avoid accidental secret leakage).
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::live_io::LiveIoTransportFactory;
//! use mfm_transports_exec::{ExecProgramTransportFactory, NAMESPACE_EXEC};
//!
//! let factory = ExecProgramTransportFactory::default();
//! assert_eq!(factory.namespace_group(), NAMESPACE_EXEC);
//! ```

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use nix::unistd::{access, AccessFlags};
use tokio::process::Command;

pub use mfm_collectors_exec::NAMESPACE_EXEC;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_transports_process_exec::{run_command, ProcessRunError, StreamLimit};

const CODE_EXEC_REQUEST_INVALID: &str = "exec_request_invalid";
const CODE_EXEC_PROGRAM_NOT_ALLOWED: &str = "exec_program_not_allowed";
const CODE_EXEC_PROGRAM_MISSING: &str = "exec_program_missing";
const CODE_EXEC_PROGRAM_NOT_EXECUTABLE: &str = "exec_program_not_executable";
const CODE_EXEC_SPAWN_FAILED: &str = "exec_spawn_failed";
const CODE_EXEC_STDIN_WRITE_FAILED: &str = "exec_stdin_write_failed";
const CODE_EXEC_STDIN_TOO_LARGE: &str = "exec_stdin_too_large";
const CODE_EXEC_TIMEOUT: &str = "exec_timeout";
const CODE_EXEC_FAILED: &str = "exec_failed";
const CODE_EXEC_STDOUT_INVALID_JSON: &str = "exec_stdout_invalid_json";
const CODE_EXEC_STDOUT_TOO_LARGE: &str = "exec_stdout_too_large";
const CODE_EXEC_STDERR_TOO_LARGE: &str = "exec_stderr_too_large";

const MAX_EXEC_STDIN_BYTES: usize = 1024 * 1024;
const MAX_EXEC_STDOUT_BYTES: usize = 1024 * 1024;
const MAX_EXEC_STDERR_BYTES: usize = 1024 * 1024;

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode::must_new(code),
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
        code: ErrorCode::must_new(code),
        category,
        retryable: false,
        message: message.to_string(),
        details: Some(details),
    }
}

/// Policy used by the `exec` transport to constrain which programs may be executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecPolicy {
    /// Canonical path prefixes that executable targets must reside under.
    pub allow_prefixes: Vec<String>,
}

impl Default for ExecPolicy {
    fn default() -> Self {
        Self {
            // Conservative default to keep execution reproducible without needing a "realize" step.
            allow_prefixes: vec!["/nix/store/".to_string()],
        }
    }
}

/// Factory that produces Live IO transports for the [`NAMESPACE_EXEC`] namespace group.
#[derive(Clone, Default)]
pub struct ExecProgramTransportFactory {
    policy: ExecPolicy,
}

impl ExecProgramTransportFactory {
    /// Creates a factory that enforces the provided execution policy for all `exec` requests.
    pub fn new(policy: ExecPolicy) -> Self {
        Self { policy }
    }
}

impl LiveIoTransportFactory for ExecProgramTransportFactory {
    fn namespace_group(&self) -> &str {
        NAMESPACE_EXEC
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(ExecProgramTransport {
            policy: self.policy.clone(),
        })
    }
}

struct ExecProgramTransport {
    policy: ExecPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecRequestV1 {
    program_path: String,
    argv: Vec<String>,
    stdin_json: serde_json::Value,
    timeout_ms: u64,
    env: HashMap<String, String>,
}

fn parse_request(call: &IoCall) -> Result<ExecRequestV1, IoError> {
    let obj = call.request.as_object().ok_or_else(|| {
        IoError::Other(info(
            CODE_EXEC_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "exec request must be a JSON object",
        ))
    })?;

    let kind = obj.get("kind").and_then(|v| v.as_str()).ok_or_else(|| {
        IoError::Other(info(
            CODE_EXEC_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "missing exec request kind",
        ))
    })?;

    if kind != "run_program_v1" {
        return Err(IoError::Other(info(
            CODE_EXEC_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            "unsupported exec request kind",
        )));
    }

    let program_path = obj
        .get("program_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            IoError::Other(info(
                CODE_EXEC_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "missing program_path",
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
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    Ok(ExecRequestV1 {
        program_path,
        argv,
        stdin_json,
        timeout_ms,
        env,
    })
}

fn program_allowed(policy: &ExecPolicy, program_path: &Path) -> bool {
    policy
        .allow_prefixes
        .iter()
        .any(|prefix| program_path.starts_with(Path::new(prefix)))
}

fn ensure_program_accessible(path: &Path) -> Result<(), IoError> {
    access(path, AccessFlags::F_OK).map_err(|_| {
        IoError::Other(info(
            CODE_EXEC_PROGRAM_MISSING,
            ErrorCategory::Unknown,
            "program_path does not exist",
        ))
    })?;

    access(path, AccessFlags::X_OK).map_err(|_| {
        IoError::Other(info(
            CODE_EXEC_PROGRAM_NOT_EXECUTABLE,
            ErrorCategory::Unknown,
            "program_path is not executable",
        ))
    })?;

    Ok(())
}

fn resolve_program_path(policy: &ExecPolicy, requested_path: &str) -> Result<String, IoError> {
    let requested = Path::new(requested_path);
    ensure_program_accessible(requested)?;

    let canonical = std::fs::canonicalize(requested).map_err(|_| {
        IoError::Other(info(
            CODE_EXEC_PROGRAM_MISSING,
            ErrorCategory::Unknown,
            "program_path does not exist",
        ))
    })?;

    if !program_allowed(policy, &canonical) {
        return Err(IoError::Other(info(
            CODE_EXEC_PROGRAM_NOT_ALLOWED,
            ErrorCategory::Unknown,
            "program_path is not allowed by policy",
        )));
    }

    Ok(canonical.to_string_lossy().to_string())
}

fn failure_details(program_path: &str, status: &std::process::ExitStatus) -> serde_json::Value {
    let mut details = serde_json::Map::new();
    details.insert(
        "program_path".to_string(),
        serde_json::Value::String(program_path.to_string()),
    );
    details.insert(
        "exit_code".to_string(),
        status
            .code()
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
    );
    let signal_value = {
        #[cfg(unix)]
        {
            status
                .signal()
                .map(serde_json::Value::from)
                .unwrap_or(serde_json::Value::Null)
        }
        #[cfg(not(unix))]
        {
            serde_json::Value::Null
        }
    };
    details.insert("signal".to_string(), signal_value);
    serde_json::Value::Object(details)
}

#[async_trait]
impl LiveIoTransport for ExecProgramTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let req = parse_request(&call)?;
        let program_path = resolve_program_path(&self.policy, &req.program_path)?;

        let stdin_bytes = serde_json::to_vec(&req.stdin_json).map_err(|_| {
            IoError::Other(info(
                CODE_EXEC_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "stdin_json must be valid JSON",
            ))
        })?;

        if stdin_bytes.len() > MAX_EXEC_STDIN_BYTES {
            return Err(IoError::Transport(info_with_details(
                CODE_EXEC_STDIN_TOO_LARGE,
                ErrorCategory::ParsingInput,
                "stdin_json exceeded maximum size",
                serde_json::json!({
                    "program_path": program_path.clone(),
                    "max_stdin_bytes": MAX_EXEC_STDIN_BYTES,
                    "stdin_bytes": stdin_bytes.len(),
                }),
            )));
        }

        let mut cmd = Command::new(&program_path);
        cmd.args(&req.argv);

        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        let result = run_command(
            cmd,
            Some(stdin_bytes),
            Duration::from_millis(req.timeout_ms),
            StreamLimit {
                max_stdout_bytes: MAX_EXEC_STDOUT_BYTES,
                max_stderr_bytes: MAX_EXEC_STDERR_BYTES,
            },
        )
        .await
        .map_err(|err| match err {
            ProcessRunError::SpawnFailed => IoError::Transport(info_with_details(
                CODE_EXEC_SPAWN_FAILED,
                ErrorCategory::Unknown,
                "failed to spawn program",
                serde_json::json!({
                    "program_path": program_path.clone(),
                }),
            )),
            ProcessRunError::Timeout => IoError::Transport(info_with_details(
                CODE_EXEC_TIMEOUT,
                ErrorCategory::Unknown,
                "program execution timed out",
                serde_json::json!({
                    "program_path": program_path.clone(),
                    "timeout_ms": req.timeout_ms,
                }),
            )),
            ProcessRunError::WaitFailed
            | ProcessRunError::StdoutReadFailed
            | ProcessRunError::StderrReadFailed => IoError::Transport(info_with_details(
                CODE_EXEC_FAILED,
                ErrorCategory::Unknown,
                "program execution failed",
                serde_json::json!({
                    "program_path": program_path.clone(),
                }),
            )),
        })?;

        if result.stdout.overflowed {
            return Err(IoError::Transport(info_with_details(
                CODE_EXEC_STDOUT_TOO_LARGE,
                ErrorCategory::Unknown,
                "program stdout exceeded maximum size",
                serde_json::json!({
                    "program_path": program_path.clone(),
                    "max_stdout_bytes": MAX_EXEC_STDOUT_BYTES,
                    "stdout_bytes": result.stdout.total_bytes,
                }),
            )));
        }
        if result.stderr.overflowed {
            return Err(IoError::Transport(info_with_details(
                CODE_EXEC_STDERR_TOO_LARGE,
                ErrorCategory::Unknown,
                "program stderr exceeded maximum size",
                serde_json::json!({
                    "program_path": program_path.clone(),
                    "max_stderr_bytes": MAX_EXEC_STDERR_BYTES,
                    "stderr_bytes": result.stderr.total_bytes,
                }),
            )));
        }

        if let Some(stdin_err) = result.stdin_write_error {
            if stdin_err.kind == std::io::ErrorKind::BrokenPipe && !result.status.success() {
                return Err(IoError::Transport(info_with_details(
                    CODE_EXEC_FAILED,
                    ErrorCategory::Unknown,
                    "program exited with non-zero status",
                    failure_details(&program_path, &result.status),
                )));
            }
            if stdin_err.kind != std::io::ErrorKind::BrokenPipe {
                return Err(IoError::Transport(info_with_details(
                    CODE_EXEC_STDIN_WRITE_FAILED,
                    ErrorCategory::Unknown,
                    "failed to write program stdin",
                    serde_json::json!({
                        "program_path": program_path.clone(),
                        "io_error_kind": format!("{:?}", stdin_err.kind),
                    }),
                )));
            }
        }

        if let Some(stdin_err) = result.stdin_close_error {
            if stdin_err.kind != std::io::ErrorKind::BrokenPipe {
                return Err(IoError::Transport(info_with_details(
                    CODE_EXEC_STDIN_WRITE_FAILED,
                    ErrorCategory::Unknown,
                    "failed to close program stdin",
                    serde_json::json!({
                        "program_path": program_path.clone(),
                        "io_error_kind": format!("{:?}", stdin_err.kind),
                    }),
                )));
            }
        }

        if !result.status.success() {
            return Err(IoError::Transport(info_with_details(
                CODE_EXEC_FAILED,
                ErrorCategory::Unknown,
                "program exited with non-zero status",
                failure_details(&program_path, &result.status),
            )));
        }

        serde_json::from_slice::<serde_json::Value>(&result.stdout.bytes).map_err(|_| {
            IoError::Other(info(
                CODE_EXEC_STDOUT_INVALID_JSON,
                ErrorCategory::ParsingInput,
                "program stdout was not valid JSON",
            ))
        })
    }
}

#[cfg(test)]
#[path = "tests/exec_transport_tests.rs"]
mod exec_transport_tests;
