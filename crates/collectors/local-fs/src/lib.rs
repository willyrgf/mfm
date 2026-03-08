#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Typed local filesystem collectors.
//!
//! This crate defines typed adapters over the generic `IoCall` surface for local filesystem reads.
//! It intentionally does NOT perform IO itself.
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

/// Canonical namespace group used for local filesystem IO calls.
pub const NAMESPACE_LOCAL_FS: &str = "local.fs";
/// Canonical namespace used for text-file reads.
pub const NAMESPACE_LOCAL_FS_READ_TEXT: &str = "local.fs.read_text";

/// Typed request for `local.fs.read_text`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadTextRequest {
    /// Optional UTF-8 path.
    #[serde(default)]
    pub path: Option<String>,
    /// Optional hex-encoded UTF-8 path.
    #[serde(default)]
    pub path_hex: Option<String>,
}

/// Typed response for `local.fs.read_text`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadTextResponse {
    /// File contents as UTF-8 text.
    pub text: String,
}

/// Error returned when a fact key cannot be derived from a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactKeyDerivationError {
    /// The request could not be canonically hashed for fact recording.
    NotCanonical(CanonicalJsonError),
}

impl std::fmt::Display for FactKeyDerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactKeyDerivationError::NotCanonical(err) => write!(f, "request not canonical: {err}"),
        }
    }
}

impl std::error::Error for FactKeyDerivationError {}

/// Derives a deterministic fact key for a local filesystem request.
pub fn fact_key_for_request(
    state_id: &StateId,
    purpose: &str,
    request: &ReadTextRequest,
) -> Result<FactKey, FactKeyDerivationError> {
    let request =
        serde_json::to_value(request).expect("ReadTextRequest must serialize to serde_json::Value");
    let req_id = artifact_id_for_json(&request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:local|state:{}|purpose:{purpose}|req:{}",
        state_id.as_str(),
        req_id.0
    )))
}

/// Wraps a typed text-read request in the generic `IoCall` envelope.
pub fn read_text_call(request: ReadTextRequest, fact_key: FactKey) -> IoCall {
    let request =
        serde_json::to_value(request).expect("ReadTextRequest must serialize to serde_json::Value");
    IoCall {
        namespace: NAMESPACE_LOCAL_FS_READ_TEXT.to_string(),
        request,
        fact_key: Some(fact_key),
    }
}

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn io_other(code: &'static str, category: ErrorCategory, message: &'static str) -> IoError {
    IoError::Other(info(code, category, message))
}

/// First-class local filesystem client wrapper over `IoProvider`.
pub struct LocalFsIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> LocalFsIoClient<'a> {
    /// Creates a new client for the given state and IO provider.
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    /// Reads a UTF-8 text file through the local filesystem transport.
    pub async fn read_text(
        &mut self,
        purpose: &str,
        request: ReadTextRequest,
    ) -> Result<ReadTextResponse, IoError> {
        let fact_key =
            fact_key_for_request(&self.state_id, purpose, &request).map_err(|err| match err {
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => {
                    io_other(
                        "local_request_not_canonical",
                        ErrorCategory::ParsingInput,
                        "local fs request was not canonical-json-hashable (floats are forbidden)",
                    )
                }
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                    io_other(
                        "secrets_detected",
                        ErrorCategory::Unknown,
                        "local fs request contained secrets",
                    )
                }
            })?;

        let result = self.io.call(read_text_call(request, fact_key)).await?;
        serde_json::from_value(result.response).map_err(|_| {
            io_other(
                "local_response_invalid",
                ErrorCategory::Unknown,
                "local fs response payload had an unexpected shape",
            )
        })
    }
}
