use std::fmt;

use mfm_store::v1 as store;
use serde_json::Value;

/// Structured redaction-safe diagnostic details for runtime-owned failure evidence.
///
/// Callers must supply only closed, non-secret fields that are safe to retain in runtime
/// diagnostic artifacts. Runtime uses this value only for retained diagnostics and never as the
/// public safe message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnosticDetails {
    value: Value,
}

impl RuntimeDiagnosticDetails {
    /// Creates diagnostic details from redaction-reviewed JSON.
    pub fn from_json(value: Value) -> Self {
        Self { value }
    }

    pub(crate) const fn value(&self) -> &Value {
        &self.value
    }
}

/// Runtime failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    /// Certified spec hash verification failed.
    #[error("certified spec hash error: {0}")]
    SpecHash(String),
    /// Certified spec structure is not executable by the runtime.
    #[error("invalid typed runtime spec: {0}")]
    InvalidSpec(String),
    /// The run stream does not match the certified spec or requested run id.
    #[error("invalid typed run stream: {0}")]
    InvalidRunStream(String),
    /// A runner binding is missing or inconsistent with certified descriptor evidence.
    #[error("typed runner binding error: {0}")]
    RunnerBinding(String),
    /// No node is runnable and the run is not complete.
    #[error("typed scheduler blocked: {0}")]
    Blocked(String),
    /// The caller does not hold the run execution claim required to drive.
    #[error("typed execution claim rejected: {0}")]
    ExecutionClaim(String),
    /// Input materialization failed.
    #[error("typed input materialization failed: {0}")]
    InputMaterialization(String),
    /// Runner output violated certified node or capability authority.
    #[error("invalid runner output: {0}")]
    InvalidRunnerOutput(String),
    /// Runner output failed with structured redaction-safe diagnostic details.
    #[error("invalid runner output: {message}")]
    InvalidRunnerOutputDiagnostic {
        /// Redaction-safe summary for internal error propagation.
        message: String,
        /// Structured redaction-safe diagnostic details for retained runtime evidence.
        details: RuntimeDiagnosticDetails,
    },
    /// Runtime validation failed inside a valid started attempt.
    #[error("runtime validation failed: {0}")]
    RuntimeValidation(String),
    /// Store contract rejected a typed commit.
    #[error("typed store error: {0}")]
    Store(String),
    /// Identity construction failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// Canonical JSON construction failed.
    #[error("canonical JSON error: {0}")]
    Canonical(String),
}

impl From<store::StoreError> for RuntimeError {
    fn from(error: store::StoreError) -> Self {
        Self::Store(error.to_string())
    }
}

impl From<mfm_ids::IdentityError> for RuntimeError {
    fn from(error: mfm_ids::IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

impl From<mfm_spec::SpecError> for RuntimeError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::SpecHash(error.to_string())
    }
}

impl From<mfm_events::EventError> for RuntimeError {
    fn from(error: mfm_events::EventError) -> Self {
        Self::Identity(error.to_string())
    }
}

pub(crate) fn async_store_error(error: impl fmt::Display) -> RuntimeError {
    RuntimeError::Store(error.to_string())
}
