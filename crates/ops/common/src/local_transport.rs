use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone)]
pub struct LocalTransportError {
    pub code: &'static str,
    pub category: ErrorCategory,
    pub message: String,
}

impl LocalTransportError {
    pub fn new(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            code,
            category,
            message: message.into(),
        }
    }

    pub fn into_io(self) -> IoError {
        io_other(self.code, self.category, self.message)
    }
}

pub fn io_other(
    code: &'static str,
    category: ErrorCategory,
    message: impl Into<String>,
) -> IoError {
    IoError::Other(ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    })
}

pub fn parse_request<T: DeserializeOwned>(request: serde_json::Value) -> Result<T, IoError> {
    serde_json::from_value(request).map_err(|_| {
        io_other(
            "invalid_local_request",
            ErrorCategory::ParsingInput,
            "invalid local io request payload",
        )
    })
}

pub fn encode_response(value: serde_json::Value) -> Result<serde_json::Value, IoError> {
    serde_json::to_value(value).map_err(|_| {
        io_other(
            "local_response_serialize_failed",
            ErrorCategory::Unknown,
            "failed to serialize local io response payload",
        )
    })
}

pub fn decode_hex_utf8(
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

pub fn decode_optional_utf8(
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

pub fn decode_required_utf8(
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
