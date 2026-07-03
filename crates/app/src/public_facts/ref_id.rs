use std::fmt;

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{ContentDigest, DigestAlgorithm};
use serde::{Deserialize, Serialize};

use crate::{AppError, ErrorClass};

/// Opaque public identifier for a fact returned by app public fact services.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PublicFactRefId(String);

impl PublicFactRefId {
    /// Creates a public fact ref id from app-generated opaque text.
    pub fn new(value: impl Into<String>) -> Result<Self, AppError> {
        let value = value.into();
        let valid = value
            .strip_prefix("pfr_")
            .is_some_and(|suffix| suffix.len() == 64 && suffix.bytes().all(is_lower_hex));
        if !valid {
            return Err(AppError::new(
                ErrorClass::BadRequest,
                "PublicFactRefInvalid",
                "Public fact reference is invalid",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the opaque public ref text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const fn is_lower_hex(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

impl fmt::Display for PublicFactRefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(crate) fn public_ref_id(
    fact_ref: &mfm_facts::InternalFactRef,
) -> Result<PublicFactRefId, AppError> {
    let claim_id = mfm_facts::canonical_fact_claim_id_bytes(fact_ref.fact_claim_id())?;
    let material = [
        b"mfm.public-fact-ref.v1:".as_slice(),
        claim_id.as_bytes(),
        b":platform:default".as_slice(),
    ]
    .concat();
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&material));
    PublicFactRefId::new(format!(
        "pfr_{}",
        digest.as_str().rsplit(':').next().unwrap_or_default()
    ))
}
