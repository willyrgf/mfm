#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Typed adapters for the `exec` IO namespace.
//!
//! This crate models requests and responses for running local programs through the generic
//! `IoProvider` interface while preserving deterministic fact-key derivation.
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_exec::{fact_key_for_run_program, RunProgramRequest};
//! use mfm_machine::ids::StateId;
//!
//! let request = RunProgramRequest {
//!     program_path: "/bin/echo".to_string(),
//!     argv: vec!["hello".to_string()],
//!     stdin_json: serde_json::json!(null),
//!     timeout_ms: 1_000,
//!     env: serde_json::json!({}),
//! };
//! let state_id = StateId::must_new("machine.exec.run".to_string());
//! let key = fact_key_for_run_program(&state_id, &request).expect("fact key");
//!
//! assert!(key.0.starts_with("mfm:exec|state:"));
//! ```
#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

/// Canonical namespace used for program-execution IO calls.
pub const NAMESPACE_EXEC: &str = "exec";

/// Request payload for the `run_program_v1` exec transport contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProgramRequest {
    /// Program path to execute.
    pub program_path: String,
    /// Command-line arguments passed to the program.
    pub argv: Vec<String>,
    /// JSON value serialized to stdin for the child process.
    pub stdin_json: serde_json::Value,
    /// Execution timeout in milliseconds.
    pub timeout_ms: u64,
    /// Extra environment variables passed to the child process.
    #[serde(default)]
    pub env: serde_json::Value,
}

/// Result returned by the exec IO helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecResult {
    /// Structured response payload returned by the transport.
    pub response: serde_json::Value,
    /// Recorded fact payload identifier, when the engine persisted one.
    pub recorded_payload_id: Option<ArtifactId>,
}

/// Error returned when an exec fact key cannot be derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactKeyDerivationError {
    /// The request payload could not be canonically hashed.
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

/// Derives a deterministic fact key for a program-execution request.
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
        req_id.as_str()
    )))
}

/// Thin typed client for the `exec` IO namespace.
pub struct ExecIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> ExecIoClient<'a> {
    /// Creates a new exec client for the given state and IO provider.
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    /// Executes a program through the generic IO provider.
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
