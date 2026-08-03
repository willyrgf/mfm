#![warn(missing_docs)]
//! Canonical structured-program and public entry-point contracts.
//!
//! This crate contains value contracts only. It does not author programs, run
//! planners, execute states, bind live capabilities, or grant admission
//! authority.

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityError};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, IdentityError, SchemaId};

/// Result type for canonical specification contracts.
pub type Result<T> = std::result::Result<T, SpecError>;

/// Failure returned by canonical specification construction or decoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    /// A checked identity could not be constructed.
    #[error("invalid specification identity: {0}")]
    Identity(String),
    /// A canonical value violated the frozen recoverability contract.
    #[error("invalid recoverability contract value: {0}")]
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

impl From<RecoverabilityError> for SpecError {
    fn from(error: RecoverabilityError) -> Self {
        Self::Contract(error.message().to_owned())
    }
}

/// Returns a lightweight content reference for exact canonical bytes.
///
/// The schema selects interpretation and must use `sha256-jcs-v1`; the content
/// digest always uses raw `sha256-v1` over the exact retained bytes.
pub fn exact_content_ref(
    schema_id: SchemaId,
    canonical: &PlainCanonicalJsonBytes,
) -> Result<ContentRef> {
    ContentRef::new(
        schema_id,
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(canonical.as_bytes()),
        ),
    )
    .map_err(Into::into)
}

mod public;
pub mod structured;

pub use public::*;
pub use structured::CertifiedFactSlot;
