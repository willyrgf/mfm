#![warn(missing_docs)]
//! Checked public signing contracts and recoverable secp256k1 verification.
//!
//! This crate owns only public key identity, transient digest/signature values, and the reusable
//! key-bound signer interface. Private key custody belongs to a signer implementation.

use std::future::Future;
use std::pin::Pin;

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use mfm_canonical::CanonicalBytes;
use mfm_ids::{ContentRef, StableId};
use mfm_program_derive::MfmValue;
use mfm_values::{canonicalize_mfm_value, ValueError};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Recoverable secp256k1 algorithm used by the checked signing surface.
pub const SECP256K1_ECDSA_RECOVERABLE_ALGORITHM_ID: &str =
    "mfm.signing.secp256k1-ecdsa-recoverable@1";

/// In-process keystore signer route.
pub const IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID: &str = "mfm.signer.in-process-keystore@1";

/// Boxed signer future carrying only owned request and result data.
pub type SigningFuture =
    Pin<Box<dyn Future<Output = Result<CompactRecoverableSignature>> + Send + 'static>>;

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
pub struct UncompressedSec1PublicKey([u8; 65]);

impl UncompressedSec1PublicKey {
    /// Validates the uncompressed prefix and secp256k1 curve point.
    pub fn new(bytes: [u8; 65]) -> Result<Self> {
        if bytes[0] != 0x04 || VerifyingKey::from_sec1_bytes(&bytes).is_err() {
            return Err(SigningError::Invalid);
        }
        Ok(Self(bytes))
    }

    /// Returns the exact SEC1 bytes.
    pub const fn as_bytes(&self) -> &[u8; 65] {
        &self.0
    }
}

/// Canonical compact low-S signature plus recoverable secp256k1 recovery ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CompactRecoverableSignature {
    bytes: [u8; 64],
    recovery_id: u8,
}

impl CompactRecoverableSignature {
    /// Validates compact scalar encoding, low-S normalization, and recovery ID `0..=3`.
    pub fn new(bytes: [u8; 64], recovery_id: u8) -> Result<Self> {
        let signature = Signature::from_slice(&bytes).map_err(|_| SigningError::Invalid)?;
        if signature.normalize_s().is_some() || RecoveryId::from_byte(recovery_id).is_none() {
            return Err(SigningError::Invalid);
        }
        Ok(Self { bytes, recovery_id })
    }

    /// Returns the exact compact signature bytes.
    pub const fn as_bytes(&self) -> &[u8; 64] {
        &self.bytes
    }

    /// Returns the checked recovery ID.
    pub const fn recovery_id(&self) -> u8 {
        self.recovery_id
    }
}

/// Public algorithm and exact key bytes whose content ref identifies one key instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.signing",
    name = "public-key",
    version = "1",
    schema = "mfm.signing-public-key"
)]
pub struct PublicSigningKey {
    algorithm: StableId,
    #[mfm(minimum_bytes = 65, maximum_bytes = 65)]
    public_key: String,
}

impl PublicSigningKey {
    /// Constructs the one supported recoverable secp256k1 public-key value.
    pub fn new(public_key: UncompressedSec1PublicKey) -> Result<Self> {
        Ok(Self {
            algorithm: StableId::new(SECP256K1_ECDSA_RECOVERABLE_ALGORITHM_ID)
                .map_err(|_| SigningError::Invalid)?,
            public_key: CanonicalBytes::new(public_key.as_bytes().to_vec())
                .encoded()
                .to_owned(),
        })
    }

    /// Returns the exact algorithm identity.
    pub const fn algorithm(&self) -> &StableId {
        &self.algorithm
    }

    /// Returns the checked public key bytes.
    pub fn public_key(&self) -> Result<UncompressedSec1PublicKey> {
        let bytes = CanonicalBytes::from_base64url_no_pad(self.public_key.clone())
            .map_err(|_| SigningError::Invalid)?
            .into_bytes();
        let bytes: [u8; 65] = bytes.try_into().map_err(|_| SigningError::Invalid)?;
        UncompressedSec1PublicKey::new(bytes)
    }

    /// Derives the content reference used as the public key-instance identity.
    pub fn key_instance_ref(&self) -> Result<ContentRef> {
        canonicalize_mfm_value(self)
            .map(|(_, reference)| reference)
            .map_err(map_value_error)
    }

    fn validate(&self) -> Result<()> {
        if self.algorithm.as_str() != SECP256K1_ECDSA_RECOVERABLE_ALGORITHM_ID {
            return Err(SigningError::Invalid);
        }
        self.public_key().map(|_| ())
    }
}

impl<'de> Deserialize<'de> for PublicSigningKey {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            algorithm: StableId,
            public_key: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let value = Self {
            algorithm: wire.algorithm,
            public_key: wire.public_key,
        };
        value.validate().map_err(de::Error::custom)?;
        Ok(value)
    }
}

/// Key-bound public signer identity retained by commands and adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.signing",
    name = "public-signer-identity",
    version = "1",
    schema = "mfm.signing-public-signer-identity"
)]
pub struct PublicSignerIdentity {
    public_key: PublicSigningKey,
    signer_route: StableId,
}

impl PublicSignerIdentity {
    /// Constructs one public signer route and key-instance binding.
    pub fn new(signer_route: StableId, public_key: PublicSigningKey) -> Self {
        Self {
            public_key,
            signer_route,
        }
    }

    /// Returns the complete public signing key.
    pub const fn public_key(&self) -> &PublicSigningKey {
        &self.public_key
    }

    /// Returns the signer implementation route.
    pub const fn signer_route(&self) -> &StableId {
        &self.signer_route
    }

    /// Returns the public content reference distinguishing the key instance.
    pub fn key_instance_ref(&self) -> Result<ContentRef> {
        self.public_key.key_instance_ref()
    }
}

/// Recovers the public key that produced a compact signature over the exact digest.
pub fn recover_public_key(
    digest: SigningDigest,
    checked_signature: CompactRecoverableSignature,
) -> Result<UncompressedSec1PublicKey> {
    let signature =
        Signature::from_slice(checked_signature.as_bytes()).map_err(|_| SigningError::Invalid)?;
    let recovery_id =
        RecoveryId::from_byte(checked_signature.recovery_id()).ok_or(SigningError::Invalid)?;
    let key = VerifyingKey::recover_from_prehash(digest.as_bytes(), &signature, recovery_id)
        .map_err(|_| SigningError::Failed)?;
    let encoded = key.to_encoded_point(false);
    let bytes: [u8; 65] = encoded
        .as_bytes()
        .try_into()
        .map_err(|_| SigningError::Failed)?;
    UncompressedSec1PublicKey::new(bytes)
}

/// Verifies that a signature recovers the exact expected public key.
pub fn verify_recoverable_signature(
    digest: SigningDigest,
    signature: CompactRecoverableSignature,
    expected: &UncompressedSec1PublicKey,
) -> Result<()> {
    (recover_public_key(digest, signature)? == *expected)
        .then_some(())
        .ok_or(SigningError::Failed)
}

/// Reusable key-bound signer interface.
pub trait Signer: Send + Sync + 'static {
    /// Returns the immutable public signer/key identity.
    fn public_identity(&self) -> &PublicSignerIdentity;

    /// Signs one exact digest for one checked public purpose.
    fn sign(&self, digest: SigningDigest, purpose: StableId) -> SigningFuture;
}

fn map_value_error(_error: ValueError) -> SigningError {
    SigningError::Invalid
}
