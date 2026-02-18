use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use serde::de::DeserializeOwned;
use serde::Deserialize;

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

fn encode_response(value: serde_json::Value) -> Result<serde_json::Value, IoError> {
    serde_json::to_value(value).map_err(|_| {
        io_other(
            "local_response_serialize_failed",
            ErrorCategory::Unknown,
            "failed to serialize local io response payload",
        )
    })
}

fn handle_read_text(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: ReadTextRequest = parse_request(request)?;
    let path = decode_required_utf8(req.path, req.path_hex, "invalid_local_request", "path")
        .map_err(|err| io_other(err.code, err.category, err.message))?;
    let text = std::fs::read_to_string(&path).map_err(|e| {
        io_other(
            "InputReadError",
            ErrorCategory::Unknown,
            format!("Failed to read input file '{path}': {e}"),
        )
    })?;
    encode_response(serde_json::json!({ "text": text }))
}

#[derive(Debug, Clone)]
struct LocalError {
    code: &'static str,
    category: ErrorCategory,
    message: String,
}

impl LocalError {
    fn new(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            code,
            category,
            message: message.into(),
        }
    }
}

fn decode_hex_utf8(
    raw: &str,
    code: &'static str,
    field: &'static str,
) -> Result<String, LocalError> {
    let bytes = hex::decode(raw).map_err(|_| {
        LocalError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} must be valid hex"),
        )
    })?;
    String::from_utf8(bytes).map_err(|_| {
        LocalError::new(
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
) -> Result<Option<String>, LocalError> {
    match (raw, raw_hex) {
        (Some(_), Some(_)) => Err(LocalError::new(
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
) -> Result<String, LocalError> {
    decode_optional_utf8(raw, raw_hex, code, field)?.ok_or_else(|| {
        LocalError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} is required"),
        )
    })
}
