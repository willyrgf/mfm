use std::fmt;

use mfm_store::v1 as store;

/// Runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// Certified spec hash verification failed.
    SpecHash(String),
    /// Certified spec structure is not executable by the runtime.
    InvalidSpec(String),
    /// The run stream does not match the certified spec or requested run id.
    InvalidRunStream(String),
    /// A runner binding is missing or inconsistent with certified descriptor evidence.
    RunnerBinding(String),
    /// No node is runnable and the run is not complete.
    Blocked(String),
    /// Input materialization failed.
    InputMaterialization(String),
    /// Runner output violated certified node or capability authority.
    InvalidRunnerOutput(String),
    /// Runtime validation failed inside a valid started attempt.
    RuntimeValidation(String),
    /// Store contract rejected a typed commit.
    Store(String),
    /// Identity construction failed.
    Identity(String),
    /// Canonical JSON construction failed.
    Canonical(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpecHash(message) => write!(f, "certified spec hash error: {message}"),
            Self::InvalidSpec(message) => write!(f, "invalid typed runtime spec: {message}"),
            Self::InvalidRunStream(message) => write!(f, "invalid typed run stream: {message}"),
            Self::RunnerBinding(message) => write!(f, "typed runner binding error: {message}"),
            Self::Blocked(message) => write!(f, "typed scheduler blocked: {message}"),
            Self::InputMaterialization(message) => {
                write!(f, "typed input materialization failed: {message}")
            }
            Self::InvalidRunnerOutput(message) => write!(f, "invalid runner output: {message}"),
            Self::RuntimeValidation(message) => write!(f, "runtime validation failed: {message}"),
            Self::Store(message) => write!(f, "typed store error: {message}"),
            Self::Identity(message) => write!(f, "identity error: {message}"),
            Self::Canonical(message) => write!(f, "canonical JSON error: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

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
