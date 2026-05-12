#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Local filesystem transport for reading small text inputs.
//!
//! This transport is intentionally narrow and currently exposes only `local.fs.read_text`, which
//! allows state logic to read files through the Live IO abstraction rather than ambient file IO.
//! Blocking filesystem work is isolated on Tokio's blocking pool before returning sanitized IO
//! errors to the async state-machine runtime.
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::live_io::LiveIoTransportFactory;
//! use mfm_transports_local_fs::LocalFsIoTransportFactory;
//!
//! let factory = LocalFsIoTransportFactory;
//! assert_eq!(factory.namespace_group(), "local.fs");
//! ```
#![warn(missing_docs)]

use async_trait::async_trait;
use mfm_collectors_local_fs::{ReadTextRequest, ReadTextResponse, NAMESPACE_LOCAL_FS_READ_TEXT};
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use serde::de::DeserializeOwned;

/// Transport factory for the `local.fs` namespace group.
#[derive(Clone, Default)]
pub struct LocalFsIoTransportFactory;

impl LiveIoTransportFactory for LocalFsIoTransportFactory {
    fn namespace_group(&self) -> &str {
        "local.fs"
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(LocalFsIoTransport)
    }
}

struct LocalFsIoTransport;

#[async_trait]
impl LiveIoTransport for LocalFsIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        run_blocking_local_fs(move || dispatch_local_fs_call(call)).await
    }
}

async fn run_blocking_local_fs<F>(f: F) -> Result<serde_json::Value, IoError>
where
    F: FnOnce() -> Result<serde_json::Value, IoError> + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|_| {
        io_other(
            "local_transport_join_failed",
            ErrorCategory::Unknown,
            "local fs transport worker failed",
        )
    })?
}

fn dispatch_local_fs_call(call: IoCall) -> Result<serde_json::Value, IoError> {
    match call.namespace.as_str() {
        NAMESPACE_LOCAL_FS_READ_TEXT => handle_read_text(call.request),
        _ => Err(io_other(
            "unknown_namespace",
            ErrorCategory::Unknown,
            "unknown local fs io namespace",
        )),
    }
}

fn handle_read_text(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: ReadTextRequest = parse_request(request)?;
    let path = decode_required_utf8(req.path, req.path_hex, "invalid_local_request", "path")
        .map_err(LocalTransportError::into_io)?;
    let text = std::fs::read_to_string(&path).map_err(|e| {
        io_other(
            "InputReadError",
            ErrorCategory::Unknown,
            format!("Failed to read input file '{path}': {e}"),
        )
    })?;
    encode_response(ReadTextResponse { text })
}

#[derive(Debug, Clone)]
struct LocalTransportError {
    code: &'static str,
    category: ErrorCategory,
    message: String,
}

impl LocalTransportError {
    fn new(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            code,
            category,
            message: message.into(),
        }
    }

    fn into_io(self) -> IoError {
        io_other(self.code, self.category, self.message)
    }
}

fn io_other(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> IoError {
    IoError::Other(ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    })
}

fn parse_request<T: DeserializeOwned>(request: serde_json::Value) -> Result<T, IoError> {
    serde_json::from_value(request).map_err(|_| {
        io_other(
            "invalid_local_request",
            ErrorCategory::ParsingInput,
            "invalid local io request payload",
        )
    })
}

fn encode_response<T: serde::Serialize>(value: T) -> Result<serde_json::Value, IoError> {
    serde_json::to_value(value).map_err(|_| {
        io_other(
            "local_response_serialize_failed",
            ErrorCategory::Unknown,
            "failed to serialize local io response payload",
        )
    })
}

fn decode_hex_utf8(
    raw: &str,
    code: &'static str,
    field: &'static str,
) -> Result<String, LocalTransportError> {
    let bytes = hex::decode(raw).map_err(|_| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} must be valid hex"),
        )
    })?;
    String::from_utf8(bytes).map_err(|_| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} did not decode to utf-8"),
        )
    })
}

fn decode_optional_utf8(
    raw: Option<String>,
    raw_hex: Option<String>,
    code: &'static str,
    field: &'static str,
) -> Result<Option<String>, LocalTransportError> {
    match (raw, raw_hex) {
        (Some(_), Some(_)) => Err(LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} must use exactly one encoding"),
        )),
        (Some(value), None) => Ok(Some(value)),
        (None, Some(value_hex)) => decode_hex_utf8(&value_hex, code, field).map(Some),
        (None, None) => Ok(None),
    }
}

fn decode_required_utf8(
    raw: Option<String>,
    raw_hex: Option<String>,
    code: &'static str,
    field: &'static str,
) -> Result<String, LocalTransportError> {
    decode_optional_utf8(raw, raw_hex, code, field)?.ok_or_else(|| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} is required"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn unique_path(name: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("mfm-local-fs-{name}-{suffix}"))
    }

    #[tokio::test]
    async fn join_failure_maps_to_sanitized_io_error() {
        let err = run_blocking_local_fs(|| panic!("secret panic payload"))
            .await
            .expect_err("panic should map to io error");

        match err {
            IoError::Other(info) => {
                assert_eq!(info.code.0, "local_transport_join_failed");
                assert!(!info.message.contains("secret panic payload"));
                assert!(info.details.is_none());
            }
            other => panic!("unexpected io error: {other:?}"),
        }
    }

    #[test]
    fn current_thread_runtime_progresses_while_local_fs_work_is_blocking() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let progressed = Arc::new(AtomicBool::new(false));
            let progressed_task = Arc::clone(&progressed);
            let blocking = run_blocking_local_fs(|| {
                std::thread::sleep(Duration::from_millis(75));
                Ok(serde_json::json!({"ok": true}))
            });
            let progress = async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                progressed_task.store(true, Ordering::SeqCst);
            };

            let (blocking_result, ()) = tokio::join!(blocking, progress);

            blocking_result.expect("blocking work should finish");
            assert!(progressed.load(Ordering::SeqCst));
        });
    }

    #[tokio::test]
    async fn read_text_response_is_unchanged() {
        let path = unique_path("read-text.txt");
        std::fs::write(&path, "hello local fs").expect("write test file");

        let mut transport = LocalFsIoTransport;
        let response = transport
            .call(IoCall {
                namespace: NAMESPACE_LOCAL_FS_READ_TEXT.to_string(),
                request: serde_json::json!({
                    "path_hex": hex::encode(path.to_string_lossy().as_bytes())
                }),
                fact_key: None,
            })
            .await
            .expect("read text");

        assert_eq!(response["text"].as_str(), Some("hello local fs"));
        let _ = std::fs::remove_file(path);
    }
}
