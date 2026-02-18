use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, IoError};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use serde::Deserialize;

use crate::local_transport::{
    decode_required_utf8, encode_response, io_other, parse_request, LocalTransportError,
};

#[derive(Clone, Default)]
pub struct LocalFsIoTransportFactory;

impl LiveIoTransportFactory for LocalFsIoTransportFactory {
    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(LocalFsIoTransport)
    }
}

struct LocalFsIoTransport;

#[async_trait]
impl LiveIoTransport for LocalFsIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            "local.fs.read_text" => handle_read_text(call.request),
            _ => Err(io_other(
                "unknown_namespace",
                ErrorCategory::Unknown,
                "unknown local fs io namespace",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReadTextRequest {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    path_hex: Option<String>,
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
    encode_response(serde_json::json!({ "text": text }))
}
