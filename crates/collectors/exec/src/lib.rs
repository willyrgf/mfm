use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

pub const NAMESPACE_EXEC: &str = "exec";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProgramRequest {
    pub program_path: String,
    pub argv: Vec<String>,
    pub stdin_json: serde_json::Value,
    pub timeout_ms: u64,
    #[serde(default)]
    pub env: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecResult {
    pub response: serde_json::Value,
    pub recorded_payload_id: Option<ArtifactId>,
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

pub fn fact_key_for_run_program(
    state_id: &StateId,
    req: &RunProgramRequest,
) -> Result<FactKey, FactKeyDerivationError> {
    let request = serde_json::json!({
        "kind": "run_program_v1",
        "program_path": req.program_path,
        "argv": req.argv,
        "stdin_json": req.stdin_json,
        "timeout_ms": req.timeout_ms,
        "env": req.env,
    });
    let req_id = artifact_id_for_json(&request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:exec|state:{}|req:{}",
        state_id.as_str(),
        req_id.0
    )))
}

pub struct ExecIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> ExecIoClient<'a> {
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    pub async fn run_program(&mut self, req: RunProgramRequest) -> Result<ExecResult, IoError> {
        let key = fact_key_for_run_program(&self.state_id, &req).map_err(|err| match err {
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => io_other(
                "exec_request_not_canonical",
                ErrorCategory::ParsingInput,
                "exec request was not canonical-json-hashable (floats are forbidden)",
            ),
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                io_other(
                    "secrets_detected",
                    ErrorCategory::Unknown,
                    "exec request contained secrets (policy forbids persisting secrets)",
                )
            }
        })?;

        let result = self
            .io
            .call(IoCall {
                namespace: NAMESPACE_EXEC.to_string(),
                request: serde_json::json!({
                    "kind": "run_program_v1",
                    "program_path": req.program_path,
                    "argv": req.argv,
                    "stdin_json": req.stdin_json,
                    "timeout_ms": req.timeout_ms,
                    "env": req.env,
                }),
                fact_key: Some(key),
            })
            .await?;

        Ok(ExecResult {
            response: result.response,
            recorded_payload_id: result.recorded_payload_id,
        })
    }
}
