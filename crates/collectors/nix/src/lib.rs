use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

pub const NAMESPACE_NIX_EXEC: &str = "nix.exec";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveFlakeAppRequest {
    pub app: String,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NixResolveResult {
    pub program_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactKeyDerivationError {
    NotCanonical(CanonicalJsonError),
}

impl std::fmt::Display for FactKeyDerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactKeyDerivationError::NotCanonical(e) => write!(f, "request not canonical: {e}"),
        }
    }
}

impl std::error::Error for FactKeyDerivationError {}

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

pub fn fact_key_for_resolve_flake_app(
    state_id: &StateId,
    req: &ResolveFlakeAppRequest,
) -> Result<FactKey, FactKeyDerivationError> {
    let request = serde_json::json!({
        "kind": "resolve_flake_app_v1",
        "app": req.app,
        "timeout_ms": req.timeout_ms,
    });
    let req_id = artifact_id_for_json(&request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:nix:preflight|state:{}|req:{}",
        state_id.0, req_id.0
    )))
}

pub struct NixIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> NixIoClient<'a> {
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    pub async fn resolve_flake_app(
        &mut self,
        req: ResolveFlakeAppRequest,
    ) -> Result<NixResolveResult, IoError> {
        let key =
            fact_key_for_resolve_flake_app(&self.state_id, &req).map_err(|err| match err {
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => {
                    io_other(
                        "nix_request_not_canonical",
                        ErrorCategory::ParsingInput,
                        "nix request was not canonical-json-hashable (floats are forbidden)",
                    )
                }
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                    io_other(
                        "secrets_detected",
                        ErrorCategory::Unknown,
                        "nix request contained secrets (policy forbids persisting secrets)",
                    )
                }
            })?;

        let response = self
            .io
            .call(IoCall {
                namespace: NAMESPACE_NIX_EXEC.to_string(),
                request: serde_json::json!({
                    "kind": "resolve_flake_app_v1",
                    "app": req.app,
                    "timeout_ms": req.timeout_ms,
                }),
                fact_key: Some(key),
            })
            .await?;

        let program_path = response
            .response
            .get("program_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                io_other(
                    "nix_preflight_invalid_response",
                    ErrorCategory::ParsingInput,
                    "nix preflight response missing program_path",
                )
            })?
            .to_string();

        Ok(NixResolveResult { program_path })
    }
}
