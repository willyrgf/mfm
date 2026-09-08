#![warn(missing_docs)]
//! Checked transient recoverable-secp256k1 values and signer capability.
//!
//! Private-key custody belongs to signer implementations. This crate exposes no persisted signer
//! identity: consumers bind durable intent to their own domain identities and use the public key as
//! a transient witness for the captured capability.

use std::future::Future;
use std::pin::Pin;

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use mfm_ids::StableId;

/// Result type for signing contracts.
pub type Result<T> = std::result::Result<T, SigningError>;

/// Redaction-safe signing error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    /// A checked public value or transient signing input was invalid.
    #[error("signing input is invalid")]
    Invalid,
    /// Signing or public verification failed.
    #[error("signing operation failed")]
    Failed,
}

/// Exact 32-byte prehashed signing digest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SigningDigest([u8; 32]);

impl SigningDigest {
    /// Constructs a digest from its exact 32 bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the exact digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Exact uncompressed SEC1 secp256k1 public key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Secp256k1PublicKey {
    bytes: [u8; 65],
}

impl Secp256k1PublicKey {
    /// Validates the uncompressed prefix and secp256k1 curve point.
    pub fn new(bytes: [u8; 65]) -> Result<Self> {
        if bytes[0] != 0x04 {
            return Err(SigningError::Invalid);
        }
        VerifyingKey::from_sec1_bytes(&bytes).map_err(|_| SigningError::Invalid)?;
        Ok(Self { bytes })
    }

    /// Returns the exact SEC1 bytes.
    pub const fn as_bytes(&self) -> &[u8; 65] {
        &self.bytes
    }
}

impl TryFrom<VerifyingKey> for Secp256k1PublicKey {
    type Error = SigningError;

    fn try_from(verifying_key: VerifyingKey) -> Result<Self> {
        let encoded = verifying_key.to_encoded_point(false);
        let bytes = encoded
            .as_bytes()
            .try_into()
            .map_err(|_| SigningError::Failed)?;
        Ok(Self { bytes })
    }
}

/// Canonical compact low-S signature plus recoverable secp256k1 recovery ID.
#[derive(Clone, PartialEq, Eq)]
pub struct CompactRecoverableSignature {
    bytes: [u8; 64],
    signature: Signature,
    recovery_id: RecoveryId,
}

impl CompactRecoverableSignature {
    /// Validates compact scalar encoding, low-S normalization, and recovery ID `0..=3`.
    pub fn new(bytes: [u8; 64], recovery_id: u8) -> Result<Self> {
        let signature = Signature::from_slice(&bytes).map_err(|_| SigningError::Invalid)?;
        let recovery_id = RecoveryId::from_byte(recovery_id).ok_or(SigningError::Invalid)?;
        Self::try_from((signature, recovery_id))
    }

    /// Returns the exact compact signature bytes.
    pub const fn as_bytes(&self) -> &[u8; 64] {
        &self.bytes
    }

    /// Returns the checked recovery ID.
    pub const fn recovery_id(&self) -> u8 {
        self.recovery_id.to_byte()
    }
}

impl TryFrom<(Signature, RecoveryId)> for CompactRecoverableSignature {
    type Error = SigningError;

    fn try_from((signature, recovery_id): (Signature, RecoveryId)) -> Result<Self> {
        if signature.normalize_s().is_some() {
            return Err(SigningError::Invalid);
        }
        let bytes = signature.to_bytes().into();
        Ok(Self {
            bytes,
            signature,
            recovery_id,
        })
    }
}

/// Recovers the public key that produced a compact signature over the exact digest.
pub fn recover_public_key(
    digest: SigningDigest,
    checked_signature: &CompactRecoverableSignature,
) -> Result<Secp256k1PublicKey> {
    let key = VerifyingKey::recover_from_prehash(
        digest.as_bytes(),
        &checked_signature.signature,
        checked_signature.recovery_id,
    )
    .map_err(|_| SigningError::Failed)?;
    Secp256k1PublicKey::try_from(key)
}

/// Boxed signer future carrying only owned request and result data.
pub type SigningFuture =
    Pin<Box<dyn Future<Output = Result<CompactRecoverableSignature>> + Send + 'static>>;

/// Reusable key- and purpose-bound recoverable-secp256k1 signer interface.
pub trait Secp256k1Signer: Send + Sync + 'static {
    /// Returns the immutable checked public key controlled by this handle.
    fn public_key(&self) -> &Secp256k1PublicKey;

    /// Returns the immutable purpose authorized for this handle.
    fn purpose(&self) -> &StableId;

    /// Signs one exact digest under the handle's immutable purpose.
    fn sign(&self, digest: SigningDigest) -> SigningFuture;
}
