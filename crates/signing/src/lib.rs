#![warn(missing_docs)]
//! Reusable public signing metadata and bounded signing requests.
//!
//! Private key custody remains outside this crate. A live signer may retain secret material only
//! in its dedicated owner; this crate exposes reusable public signing identity and request types.

use std::future::Future;
use std::pin::Pin;

use mfm_ids::StableId;
use serde::{Deserialize, Serialize};

/// Boxed signer future carrying only owned request/result data.
pub type SigningFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>;

/// Result type for signing contracts.
pub type Result<T> = std::result::Result<T, SigningError>;

/// Redaction-safe signing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    /// The request or public metadata was invalid.
    #[error("signing request is invalid")]
    InvalidRequest,
    /// The signer rejected the request.
    #[error("signing operation failed")]
    Failed,
}

/// Reusable public signer key-instance identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSignerKeyInstance {
    /// Stable public signer identity.
    pub signer_id: StableId,
    /// Stable public key-instance identity.
    pub key_instance_id: StableId,
    /// Public algorithm identity.
    pub algorithm: StableId,
}

/// Bounded non-secret signing request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningRequest {
    /// Public signer identity.
    pub signer_id: StableId,
    /// Canonical digest spelling.
    pub digest: String,
    /// Stable purpose identity.
    pub purpose: StableId,
}

/// Public signing result; secret key material never crosses this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningResult {
    /// Public key-instance identity used for the signature.
    pub key_instance: StableId,
    /// Bounded encoded signature.
    pub signature: String,
}

impl SigningRequest {
    /// Validates one bounded request.
    pub fn new(signer_id: StableId, digest: String, purpose: StableId) -> Result<Self> {
        if digest.is_empty() || digest.len() > 256 {
            return Err(SigningError::InvalidRequest);
        }
        Ok(Self {
            signer_id,
            digest,
            purpose,
        })
    }
}

/// Reusable public signer interface.
pub trait Signer: Send + Sync + 'static {
    /// Returns the immutable public key-instance identity.
    fn public_identity(&self) -> &PublicSignerKeyInstance;
    /// Signs one owned request using the dedicated secret owner.
    fn sign(&self, request: SigningRequest) -> SigningFuture<SigningResult>;
}
