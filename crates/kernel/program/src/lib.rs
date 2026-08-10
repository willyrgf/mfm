#![warn(missing_docs)]
//! Typed declaration-ordered program authoring and state-execution contracts.
//!
//! Values in this crate carry no store, append, runtime, or external-access
//! authority. Runtime privately validates committed proofs before borrowing the
//! value-only views defined here.

extern crate self as mfm_program;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub mod structured;

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

impl From<mfm_values::ValueError> for ProgramError {
    fn from(error: mfm_values::ValueError) -> Self {
        Self::Spec(error.to_string())
    }
}

/// Canonicalizes one typed structured-runtime boundary value.
pub fn encode_boundary<T: Serialize>(value: &T) -> Result<mfm_canonical::PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(value).map_err(|error| ProgramError::Codec(error.to_string()))?;
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| ProgramError::Codec(error.to_string()))
}

/// Decodes canonical structured-runtime boundary bytes and requires an exact round trip.
pub fn decode_boundary<T: DeserializeOwned + Serialize>(
    canonical: &mfm_canonical::PlainCanonicalJsonBytes,
) -> Result<T> {
    let value = serde_json::from_slice::<T>(canonical.as_bytes())
        .map_err(|error| ProgramError::Codec(error.to_string()))?;
    if encode_boundary(&value)? != *canonical {
        return Err(ProgramError::Codec(
            "typed decode did not round-trip exact canonical bytes".to_owned(),
        ));
    }
    Ok(value)
}

/// Derives the exact content reference for canonical structured-runtime boundary bytes.
pub fn boundary_content_ref(
    schema_id: mfm_ids::SchemaId,
    canonical: &mfm_canonical::PlainCanonicalJsonBytes,
) -> Result<mfm_ids::ContentRef> {
    mfm_ids::ContentRef::new(
        schema_id,
        mfm_ids::ContentDigest::from_digest(
            mfm_ids::DigestAlgorithm::Sha256V1,
            mfm_canonical::sha256_digest_bytes(canonical.as_bytes()),
        ),
    )
    .map_err(|error| ProgramError::Codec(error.to_string()))
}
