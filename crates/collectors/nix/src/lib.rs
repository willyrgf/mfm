#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Typed adapters for Nix flake-app preflight IO.
//!
//! This crate wraps the `nix.exec` namespace so states can request flake-app resolution through
//! `IoProvider` without hand-writing transport payloads.
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_nix::{fact_key_for_resolve_flake_app, ResolveFlakeAppRequest};
//! use mfm_machine::ids::StateId;
//!
//! let request = ResolveFlakeAppRequest {
//!     app: "github:willyrgf/mfm#help".to_string(),
//!     timeout_ms: 30_000,
//! };
//! let state_id = StateId::must_new("machine.preflight.nix".to_string());
//! let key = fact_key_for_resolve_flake_app(&state_id, &request).expect("fact key");
//!
//! assert!(key.0.contains("mfm:nix:preflight"));
//! ```
#![warn(missing_docs)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

/// Canonical namespace used for Nix flake-app preflight IO calls.
pub const NAMESPACE_NIX_EXEC: &str = "nix.exec";

/// Request payload for resolving a flake app into a realized program path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveFlakeAppRequest {
    /// Flake installable reference such as `github:org/repo#app`.
    pub app: String,
    /// Timeout in milliseconds for the resolution flow.
    pub timeout_ms: u64,
}

/// Request payload for running a flake app through the Nix transport.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFlakeAppRequest {
    /// Flake installable reference such as `github:org/repo#app`.
    pub app: String,
    /// Command-line arguments passed to the flake app.
    #[serde(default)]
    pub argv: Vec<String>,
    /// JSON value serialized to stdin for the child process.
    #[serde(default)]
    pub stdin_json: serde_json::Value,
    /// Timeout in milliseconds for the resolution and execution flow.
    pub timeout_ms: u64,
    /// Extra environment variables passed to the child process.
    #[serde(default)]
    pub env: serde_json::Value,
    /// Runtime host env bindings injected by the Nix transport.
    ///
    /// Map keys are target env names exposed to the flake app. Map values are source env names
    /// read from the current host process at execution time. Only names are recorded in facts.
    #[serde(default)]
    pub host_env_bindings: BTreeMap<String, String>,
}

/// Response returned by the Nix preflight transport.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NixResolveResult {
    /// Realized program path inside `/nix/store`.
    pub program_path: String,
}

/// Result returned by the Nix run helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NixRunResult {
    /// Structured response payload returned by the transport.
    pub response: serde_json::Value,
    /// Recorded fact payload identifier, when the engine persisted one.
    pub recorded_payload_id: Option<ArtifactId>,
}

/// Error returned when a Nix fact key cannot be derived.
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

/// Derives a deterministic fact key for a Nix flake-app resolution request.
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
        state_id.as_str(),
        req_id.0
    )))
}

/// Derives a deterministic fact key for a flake-app execution request.
pub fn fact_key_for_run_flake_app(
    state_id: &StateId,
    req: &RunFlakeAppRequest,
) -> Result<FactKey, FactKeyDerivationError> {
    let request = serde_json::json!({
        "kind": "run_flake_app_v1",
        "app": req.app,
        "argv": req.argv,
        "stdin_json": req.stdin_json,
        "timeout_ms": req.timeout_ms,
        "env": req.env,
        "host_env_bindings": req.host_env_bindings,
    });
    let req_id = artifact_id_for_json(&request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:nix:run|state:{}|req:{}",
        state_id.as_str(),
        req_id.0
    )))
}

/// Thin typed client for the `nix.exec` IO namespace.
pub struct NixIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> NixIoClient<'a> {
    /// Creates a new Nix preflight client for the given state and IO provider.
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    /// Resolves a flake app into a program path through the generic IO provider.
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

    /// Runs a flake app through the generic IO provider.
    pub async fn run_flake_app(
        &mut self,
        req: RunFlakeAppRequest,
    ) -> Result<NixRunResult, IoError> {
        let key = fact_key_for_run_flake_app(&self.state_id, &req).map_err(|err| match err {
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => io_other(
                "nix_request_not_canonical",
                ErrorCategory::ParsingInput,
                "nix request was not canonical-json-hashable (floats are forbidden)",
            ),
            FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                io_other(
                    "secrets_detected",
                    ErrorCategory::Unknown,
                    "nix request contained secrets (policy forbids persisting secrets)",
                )
            }
        })?;

        let result = self
            .io
            .call(IoCall {
                namespace: NAMESPACE_NIX_EXEC.to_string(),
                request: serde_json::json!({
                    "kind": "run_flake_app_v1",
                    "app": req.app,
                    "argv": req.argv,
                    "stdin_json": req.stdin_json,
                    "timeout_ms": req.timeout_ms,
                    "env": req.env,
                    "host_env_bindings": req.host_env_bindings,
                }),
                fact_key: Some(key),
            })
            .await?;

        Ok(NixRunResult {
            response: result.response,
            recorded_payload_id: result.recorded_payload_id,
        })
    }
}
