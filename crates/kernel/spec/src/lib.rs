#![warn(missing_docs)]
//! Canonical structured-program and public entry-point contracts.
//!
//! This crate contains value contracts only. It does not author programs, run
//! planners, execute states, bind live capabilities, or grant admission
//! authority.

use mfm_ids::IdentityError;

/// Result type for canonical specification contracts.
pub type Result<T> = std::result::Result<T, SpecError>;

/// Failure returned by canonical specification construction or decoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    /// A checked identity could not be constructed.
    #[error("invalid specification identity: {0}")]
    Identity(String),
    /// A canonical value violated its owner's persisted contract.
    #[error("invalid persisted contract value: {0}")]
    Contract(String),
    /// A structured-program or public-contract invariant was violated.
    #[error("invalid specification invariant: {0}")]
    Invariant(String),
    /// A shared retained-value contract failed exact validation.
    #[error(transparent)]
    RetainedValueContract(#[from] mfm_values::ValueError),
}

impl From<IdentityError> for SpecError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

mod public;
pub mod structured;

pub use public::*;
pub use structured::CertifiedFactSlot;
