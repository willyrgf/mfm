#![warn(missing_docs)]
//! Typed authored-program and closed state-execution contracts.
//!
//! Values in this crate carry no store, append, runtime, or external-access
//! authority. Runtime privately validates committed proofs before borrowing the
//! value-only views defined here.

extern crate self as mfm_program;

mod authoring;
mod callbacks;
mod execution;
mod registry;

pub use authoring::*;
pub use callbacks::*;
pub use execution::*;
pub use mfm_ids::EntryPointId;
pub use registry::*;

/// Result type for typed program construction.
pub type Result<T> = std::result::Result<T, ProgramError>;

/// Typed program construction failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProgramError {
    /// A frozen specification value was invalid.
    #[error("invalid program contract: {0}")]
    Spec(String),
    /// Authored topology or typed binding was invalid.
    #[error("invalid authored program: {0}")]
    Authoring(String),
    /// A typed callback-boundary value was not exact canonical JSON.
    #[error("invalid callback-boundary value: {0}")]
    Codec(String),
    /// A qualified program registry was incomplete or inconsistent.
    #[error("invalid qualified program registry: {0}")]
    Registry(String),
}

impl From<mfm_spec::SpecError> for ProgramError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::Spec(error.to_string())
    }
}
