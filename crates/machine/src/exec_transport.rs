//! Live IO transport for external program execution.
//!
//! This transport powers the `exec` namespace. It is NOT part of the stable API contract
//! (Appendix C.1) and may change.
//!
//! Security notes:
//! - Request payloads are not persisted by the runtime, but MUST still be treated as sensitive.
//! - Errors MUST NOT echo stdout/stderr or request payloads (avoid accidental secret leakage).

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use nix::unistd::{access, AccessFlags};
use tokio::process::Command;

use crate::errors::{ErrorCategory, ErrorInfo, IoError};
use crate::ids::ErrorCode;
use crate::io::IoCall;
use crate::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use crate::process_exec::{run_command, ProcessRunError, StreamLimit};

pub const NAMESPACE_EXEC: &str = "exec";

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
pub struct ExecPolicy {
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

#[derive(Clone, Default)]
pub struct ExecProgramTransportFactory {
    policy: ExecPolicy,
}

impl ExecProgramTransportFactory {
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
mod tests {
    use super::*;

    use crate::engine::Stores;
    use crate::errors::StorageError;
    use crate::events::EventEnvelope;
    use crate::ids::{ArtifactId, RunId, StateId};
    use crate::live_io::LiveIoEnv;
    use crate::stores::{ArtifactKind, ArtifactStore, EventStore};
    use async_trait::async_trait;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

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
            state_id: StateId::must_new("machine.main.s1".to_string()),
            attempt: 0,
        }
    }

    fn write_test_program(script_body: &str) -> (PathBuf, String) {
        let root =
            std::env::temp_dir().join(format!("mfm-exec-transport-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp test dir");
        let program = root.join("app.sh");
        std::fs::write(&program, format!("#!/bin/sh\n{script_body}\n"))
            .expect("write test program");
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&program)
                .expect("program metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&program, perms).expect("chmod test program");
        }
        let canonical_root = std::fs::canonicalize(&root).expect("canonicalize test dir");
        let allow_prefix = format!("{}/", canonical_root.display());
        (program, allow_prefix)
    }

    fn cleanup_test_program(program: &Path) {
        std::fs::remove_file(program).expect("cleanup test program");
        std::fs::remove_dir_all(program.parent().expect("program parent"))
            .expect("cleanup test dir");
    }

    #[tokio::test]
    async fn rejects_disallowed_program_path_by_default() {
        let factory = ExecProgramTransportFactory::default();
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": "/bin/echo",
                    "argv": [],
                    "stdin_json": {},
                    "timeout_ms": 10
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EXEC_PROGRAM_NOT_ALLOWED),
            other => panic!("expected Other, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_missing_program_path_even_if_allowlisted() {
        let factory = ExecProgramTransportFactory::new(ExecPolicy {
            allow_prefixes: vec!["/nix/store/".to_string()],
        });
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": "/nix/store/does-not-exist/bin/app",
                    "argv": [],
                    "stdin_json": {},
                    "timeout_ms": 10
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EXEC_PROGRAM_MISSING),
            other => panic!("expected Other, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_invalid_request_shape() {
        let factory = ExecProgramTransportFactory::default();
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!("not an object"),
                fact_key: None,
            })
            .await
            .expect_err("expected error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EXEC_REQUEST_INVALID),
            other => panic!("expected Other, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_path_traversal_that_escapes_allow_prefix() {
        let root = std::env::temp_dir().join(format!(
            "mfm-exec-transport-path-traversal-{}",
            uuid::Uuid::new_v4()
        ));
        let allowed = root.join("allowed");
        std::fs::create_dir_all(&allowed).expect("create allowlist dir");

        let program = root.join("outside.sh");
        std::fs::write(&program, "#!/bin/sh\ncat >/dev/null\nprintf '{}'\n")
            .expect("write program");
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&program).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&program, perms).expect("chmod");
        }

        let canonical_allowed =
            std::fs::canonicalize(&allowed).expect("canonicalize allowlist dir");
        let allow_prefix = format!("{}/", canonical_allowed.display());
        let traversed = allowed.join("../outside.sh");

        let factory = ExecProgramTransportFactory::new(ExecPolicy {
            allow_prefixes: vec![allow_prefix],
        });
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": traversed.to_string_lossy(),
                    "argv": [],
                    "stdin_json": {},
                    "timeout_ms": 5_000
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected allowlist rejection");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EXEC_PROGRAM_NOT_ALLOWED),
            other => panic!("expected Other, got: {other:?}"),
        }

        std::fs::remove_file(&program).expect("cleanup program");
        std::fs::remove_dir_all(&root).expect("cleanup root");
    }

    #[tokio::test]
    async fn non_zero_exit_includes_safe_failure_metadata() {
        // Drain stdin first so the test exercises non-zero exit handling instead
        // of racing against a broken pipe while writing stdin.
        let (program, allow_prefix) = write_test_program("cat >/dev/null\nexit 42");
        let factory = ExecProgramTransportFactory::new(ExecPolicy {
            allow_prefixes: vec![allow_prefix],
        });
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": program.to_string_lossy(),
                    "argv": [],
                    "stdin_json": {},
                    "timeout_ms": 5_000
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected non-zero exit");

        match err {
            IoError::Transport(info) => {
                assert_eq!(info.code.0, CODE_EXEC_FAILED);
                let details = info.details.expect("details");
                let canonical = std::fs::canonicalize(&program).expect("canonical program path");
                assert_eq!(
                    details.get("program_path").and_then(|v| v.as_str()),
                    Some(canonical.to_string_lossy().as_ref())
                );
                assert_eq!(details.get("exit_code").and_then(|v| v.as_i64()), Some(42));
                assert!(details.get("signal").is_some());
            }
            other => panic!("expected Transport, got: {other:?}"),
        }

        cleanup_test_program(&program);
    }

    #[tokio::test]
    async fn timeout_includes_program_and_timeout_metadata() {
        let (program, allow_prefix) = write_test_program("while :; do :; done");
        let factory = ExecProgramTransportFactory::new(ExecPolicy {
            allow_prefixes: vec![allow_prefix],
        });
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": program.to_string_lossy(),
                    "argv": [],
                    "stdin_json": {},
                    "timeout_ms": 20
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected timeout");

        match err {
            IoError::Transport(info) => {
                assert_eq!(info.code.0, CODE_EXEC_TIMEOUT);
                let details = info.details.expect("details");
                let canonical = std::fs::canonicalize(&program).expect("canonical program path");
                assert_eq!(
                    details.get("program_path").and_then(|v| v.as_str()),
                    Some(canonical.to_string_lossy().as_ref())
                );
                assert_eq!(details.get("timeout_ms").and_then(|v| v.as_u64()), Some(20));
            }
            other => panic!("expected Transport, got: {other:?}"),
        }

        cleanup_test_program(&program);
    }

    #[tokio::test]
    async fn timeout_applies_while_writing_stdin() {
        let (program, allow_prefix) = write_test_program("while :; do :; done");
        let factory = ExecProgramTransportFactory::new(ExecPolicy {
            allow_prefixes: vec![allow_prefix],
        });
        let mut t = factory.make(env());

        let large_payload = "a".repeat(256 * 1024);
        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": program.to_string_lossy(),
                    "argv": [],
                    "stdin_json": large_payload,
                    "timeout_ms": 20
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected timeout");

        match err {
            IoError::Transport(info) => assert_eq!(info.code.0, CODE_EXEC_TIMEOUT),
            other => panic!("expected Transport, got: {other:?}"),
        }

        cleanup_test_program(&program);
    }

    #[tokio::test]
    async fn rejects_stdin_payloads_larger_than_limit() {
        let (program, allow_prefix) = write_test_program("cat >/dev/null\nprintf '{}'");
        let factory = ExecProgramTransportFactory::new(ExecPolicy {
            allow_prefixes: vec![allow_prefix],
        });
        let mut t = factory.make(env());

        let huge = "a".repeat(MAX_EXEC_STDIN_BYTES + 1);
        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": program.to_string_lossy(),
                    "argv": [],
                    "stdin_json": huge,
                    "timeout_ms": 1_000
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected stdin size error");

        match err {
            IoError::Transport(info) => {
                assert_eq!(info.code.0, CODE_EXEC_STDIN_TOO_LARGE);
                let details = info.details.expect("details");
                assert_eq!(
                    details.get("max_stdin_bytes").and_then(|v| v.as_u64()),
                    Some(MAX_EXEC_STDIN_BYTES as u64)
                );
            }
            other => panic!("expected Transport, got: {other:?}"),
        }

        cleanup_test_program(&program);
    }

    #[tokio::test]
    async fn stdout_overflow_reports_bounded_failure_metadata() {
        let (program, allow_prefix) = write_test_program("head -c 1200000 /dev/zero");
        let factory = ExecProgramTransportFactory::new(ExecPolicy {
            allow_prefixes: vec![allow_prefix],
        });
        let mut t = factory.make(env());

        let err = t
            .call(IoCall {
                namespace: "exec".to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": program.to_string_lossy(),
                    "argv": [],
                    "stdin_json": {},
                    "timeout_ms": 5_000
                }),
                fact_key: None,
            })
            .await
            .expect_err("expected stdout overflow");

        match err {
            IoError::Transport(info) => {
                assert_eq!(info.code.0, CODE_EXEC_STDOUT_TOO_LARGE);
                let details = info.details.expect("details");
                assert_eq!(
                    details.get("max_stdout_bytes").and_then(|v| v.as_u64()),
                    Some(MAX_EXEC_STDOUT_BYTES as u64)
                );
                assert!(
                    details
                        .get("stdout_bytes")
                        .and_then(|v| v.as_u64())
                        .expect("stdout_bytes")
                        > MAX_EXEC_STDOUT_BYTES as u64
                );
            }
            other => panic!("expected Transport, got: {other:?}"),
        }

        cleanup_test_program(&program);
    }
}
