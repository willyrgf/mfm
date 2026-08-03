#![warn(missing_docs)]
//! Deterministic structured-program expansion and offline certification.
//!
//! This crate owns the one child-substitution and support-injection pipeline.
//! Certification validates complete implementation and capability-binding
//! manifests, and verification deterministically reruns expansion over retained
//! canonical inputs.

pub mod structured;

/// Result type for planning and certification.
pub type Result<T> = std::result::Result<T, CertifyError>;

/// Closed planning or certification failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CertifyError {
    /// A canonical specification value failed validation.
    #[error("invalid canonical specification: {0}")]
    Specification(String),
    /// Deterministic expansion could not resolve a complete structured program.
    #[error("planning failed: {0}")]
    Planning(String),
    /// Certification proof closure or implementation coverage was incomplete.
    #[error("certification failed: {0}")]
    Certification(String),
    /// Offline verification did not reproduce the retained result.
    #[error("verification failed: {0}")]
    Verification(String),
}

impl From<mfm_spec::SpecError> for CertifyError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::Specification(error.to_string())
    }
}

impl From<mfm_canonical::RecoverabilityError> for CertifyError {
    fn from(error: mfm_canonical::RecoverabilityError) -> Self {
        Self::Specification(error.to_string())
    }
}
