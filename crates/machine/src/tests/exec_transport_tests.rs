use super::*;

use crate::engine::Stores;
use crate::errors::StorageError;
use crate::ids::{ArtifactId, RunId, StateId};
use crate::live_io::LiveIoEnv;
use crate::stores::{
    AppendBatchResult, ArtifactKind, ArtifactStore, StreamAppend, StreamId, StreamRecord,
    StreamStore,
};
use async_trait::async_trait;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
struct NoopStreamStore;

#[async_trait]
impl StreamStore for NoopStreamStore {
    async fn head_seq(&self, _stream_id: &StreamId) -> Result<u64, StorageError> {
        Ok(0)
    }

    async fn append(&self, _append: StreamAppend) -> Result<u64, StorageError> {
        Ok(0)
    }

    async fn append_batch(
        &self,
        _appends: Vec<StreamAppend>,
    ) -> Result<AppendBatchResult, StorageError> {
        Ok(AppendBatchResult {
            stream_heads: Vec::new(),
        })
    }

    async fn read_range(
        &self,
        _stream_id: &StreamId,
        _from_seq: u64,
        _to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, StorageError> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
struct NoopArtifactStore;

#[async_trait]
impl ArtifactStore for NoopArtifactStore {
    async fn put(&self, _kind: ArtifactKind, _bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        Ok(ArtifactId::must_new("0".repeat(64)))
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
            streams: Arc::new(NoopStreamStore),
            artifacts: Arc::new(NoopArtifactStore),
        },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId::must_new("machine.main.s1".to_string()),
        attempt: 0,
    }
}

fn write_test_program(script_body: &str) -> (PathBuf, String) {
    let root = std::env::temp_dir().join(format!("mfm-exec-transport-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create temp test dir");
    let program = root.join("app.sh");
    std::fs::write(&program, format!("#!/bin/sh\n{script_body}\n")).expect("write test program");
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
    std::fs::remove_dir_all(program.parent().expect("program parent")).expect("cleanup test dir");
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
        IoError::Other(info) => assert_eq!(info.code.as_str(), CODE_EXEC_PROGRAM_NOT_ALLOWED),
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
        IoError::Other(info) => assert_eq!(info.code.as_str(), CODE_EXEC_PROGRAM_MISSING),
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
        IoError::Other(info) => assert_eq!(info.code.as_str(), CODE_EXEC_REQUEST_INVALID),
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
    std::fs::write(&program, "#!/bin/sh\ncat >/dev/null\nprintf '{}'\n").expect("write program");
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&program).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&program, perms).expect("chmod");
    }

    let canonical_allowed = std::fs::canonicalize(&allowed).expect("canonicalize allowlist dir");
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
        IoError::Other(info) => assert_eq!(info.code.as_str(), CODE_EXEC_PROGRAM_NOT_ALLOWED),
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
            assert_eq!(info.code.as_str(), CODE_EXEC_FAILED);
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
            assert_eq!(info.code.as_str(), CODE_EXEC_TIMEOUT);
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
        IoError::Transport(info) => assert_eq!(info.code.as_str(), CODE_EXEC_TIMEOUT),
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
            assert_eq!(info.code.as_str(), CODE_EXEC_STDIN_TOO_LARGE);
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
            assert_eq!(info.code.as_str(), CODE_EXEC_STDOUT_TOO_LARGE);
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
