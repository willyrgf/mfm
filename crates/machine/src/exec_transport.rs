//! Live IO transport for external program execution (Milestone 2).
//!
//! This transport powers the `exec` namespace. It is NOT part of the stable API contract
//! (Appendix C.1) and may change.
//!
//! Security notes:
//! - Request payloads are not persisted by the runtime, but MUST still be treated as sensitive.
//! - Errors MUST NOT echo stdout/stderr or request payloads (avoid accidental secret leakage).

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use nix::unistd::{access, AccessFlags};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::errors::{ErrorCategory, ErrorInfo, IoError};
use crate::ids::ErrorCode;
use crate::io::IoCall;
use crate::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};

pub const NAMESPACE_EXEC: &str = "exec";

const CODE_EXEC_REQUEST_INVALID: &str = "exec_request_invalid";
const CODE_EXEC_PROGRAM_NOT_ALLOWED: &str = "exec_program_not_allowed";
const CODE_EXEC_PROGRAM_MISSING: &str = "exec_program_missing";
const CODE_EXEC_PROGRAM_NOT_EXECUTABLE: &str = "exec_program_not_executable";
const CODE_EXEC_SPAWN_FAILED: &str = "exec_spawn_failed";
const CODE_EXEC_STDIN_WRITE_FAILED: &str = "exec_stdin_write_failed";
const CODE_EXEC_TIMEOUT: &str = "exec_timeout";
const CODE_EXEC_FAILED: &str = "exec_failed";
const CODE_EXEC_STDOUT_INVALID_JSON: &str = "exec_stdout_invalid_json";

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
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

fn program_allowed(policy: &ExecPolicy, program_path: &str) -> bool {
    policy
        .allow_prefixes
        .iter()
        .any(|p| program_path.starts_with(p))
}

fn ensure_program_accessible(program_path: &str) -> Result<(), IoError> {
    let path = Path::new(program_path);

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

#[async_trait]
impl LiveIoTransport for ExecProgramTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let req = parse_request(&call)?;

        if !program_allowed(&self.policy, &req.program_path) {
            return Err(IoError::Other(info(
                CODE_EXEC_PROGRAM_NOT_ALLOWED,
                ErrorCategory::Unknown,
                "program_path is not allowed by policy",
            )));
        }

        ensure_program_accessible(&req.program_path)?;

        let stdin_bytes = serde_json::to_vec(&req.stdin_json).map_err(|_| {
            IoError::Other(info(
                CODE_EXEC_REQUEST_INVALID,
                ErrorCategory::ParsingInput,
                "stdin_json must be valid JSON",
            ))
        })?;

        let mut cmd = Command::new(&req.program_path);
        cmd.kill_on_drop(true);
        cmd.args(&req.argv);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|_| {
            IoError::Transport(info(
                CODE_EXEC_SPAWN_FAILED,
                ErrorCategory::Unknown,
                "failed to spawn program",
            ))
        })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&stdin_bytes).await.map_err(|_| {
                IoError::Transport(info(
                    CODE_EXEC_STDIN_WRITE_FAILED,
                    ErrorCategory::Unknown,
                    "failed to write program stdin",
                ))
            })?;
        }

        let duration = Duration::from_millis(req.timeout_ms);
        let output = tokio::time::timeout(duration, child.wait_with_output())
            .await
            .map_err(|_| {
                IoError::Transport(info(
                    CODE_EXEC_TIMEOUT,
                    ErrorCategory::Unknown,
                    "program execution timed out",
                ))
            })?
            .map_err(|_| {
                IoError::Transport(info(
                    CODE_EXEC_FAILED,
                    ErrorCategory::Unknown,
                    "program execution failed",
                ))
            })?;

        if !output.status.success() {
            return Err(IoError::Transport(info(
                CODE_EXEC_FAILED,
                ErrorCategory::Unknown,
                "program exited with non-zero status",
            )));
        }

        serde_json::from_slice::<serde_json::Value>(&output.stdout).map_err(|_| {
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
            state_id: StateId("machine.main.s1".to_string()),
            attempt: 0,
        }
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
}
