//! Security-sensitive Ethereum private-key helpers.
//!
//! This module owns parsing, validation, address derivation, and prehash signing for raw
//! secp256k1 Ethereum private keys. It intentionally does not own transaction encoding; callers
//! pass the returned non-secret signature to EVM transaction primitives in downstream crates.
//!
//! # Examples
//!
//! ```rust
//! use mfm_core::crypto::EthereumPrivateKey;
//!
//! let key = EthereumPrivateKey::from_hex_secret(
//!     "0x0000000000000000000000000000000000000000000000000000000000000001",
//! )?;
//! assert_eq!(
//!     format!("{:?}", key.address()?),
//!     "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
//! );
//! # Ok::<(), mfm_core::crypto::EthereumKeyError>(())
//! ```

use alloy_primitives::{Address, PrimitiveSignature, B256};
use k256::ecdsa::SigningKey;
use k256::SecretKey;
use tiny_keccak::{Hasher, Keccak};
use zeroize::Zeroizing;

/// Error returned by Ethereum private-key helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EthereumKeyError {
    /// The supplied key was not valid hex.
    #[error("signing key hex was invalid")]
    InvalidHex,
    /// The supplied key did not decode to exactly 32 bytes.
    #[error("signing key must be exactly 32 bytes")]
    InvalidLength,
    /// The supplied bytes did not form a valid secp256k1 private key.
    #[error("signing key did not form a valid secp256k1 key")]
    InvalidPrivateKey,
    /// The signing primitive failed.
    #[error("failed to sign prehashed payload")]
    SigningFailed,
}

/// Zeroizing wrapper for a raw Ethereum secp256k1 private key.
pub struct EthereumPrivateKey {
    key_bytes: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for EthereumPrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EthereumPrivateKey")
            .field("key_bytes", &"<redacted>")
            .finish()
    }
}

impl EthereumPrivateKey {
    /// Parses a raw private key from a `0x`-prefixed or bare hex string.
    pub fn from_hex_secret(raw: &str) -> Result<Self, EthereumKeyError> {
        let mut value = raw.trim();
        value = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);

        if value.is_empty() {
            return Err(EthereumKeyError::InvalidHex);
        }

        let normalized = Zeroizing::new(if value.len().is_multiple_of(2) {
            value.to_string()
        } else {
            format!("0{value}")
        });
        let bytes = Zeroizing::new(
            hex::decode(normalized.as_str()).map_err(|_| EthereumKeyError::InvalidHex)?,
        );
        if bytes.len() != 32 {
            return Err(EthereumKeyError::InvalidLength);
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(bytes.as_slice());
        Self::from_secret_bytes(key_bytes)
    }

    /// Builds a private key from already-decoded secret bytes.
    pub fn from_secret_bytes(key_bytes: [u8; 32]) -> Result<Self, EthereumKeyError> {
        SecretKey::from_slice(&key_bytes).map_err(|_| EthereumKeyError::InvalidPrivateKey)?;
        Ok(Self {
            key_bytes: Zeroizing::new(key_bytes),
        })
    }

    fn secret_key(&self) -> Result<SecretKey, EthereumKeyError> {
        SecretKey::from_slice(self.key_bytes.as_ref())
            .map_err(|_| EthereumKeyError::InvalidPrivateKey)
    }

    /// Derives the Ethereum address for this private key.
    pub fn address(&self) -> Result<Address, EthereumKeyError> {
        let secret_key = self.secret_key()?;
        let public_key = secret_key.public_key();

        use k256::elliptic_curve::sec1::ToEncodedPoint;
        let uncompressed_pk = public_key.to_encoded_point(false);
        let mut keccak = Keccak::v256();
        keccak.update(&uncompressed_pk.as_bytes()[1..]);
        let mut hash = [0u8; 32];
        keccak.finalize(&mut hash);

        Ok(Address::from_slice(&hash[12..]))
    }

    /// Signs a 32-byte prehash and returns a recoverable Ethereum signature.
    pub fn sign_hash_recoverable(
        &self,
        hash: &[u8; 32],
    ) -> Result<PrimitiveSignature, EthereumKeyError> {
        let secret_key = self.secret_key()?;
        let signing_key = SigningKey::from(&secret_key);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(hash)
            .map_err(|_| EthereumKeyError::SigningFailed)?;
        let signature_bytes = signature.to_bytes();
        let mut r = [0u8; 32];
        r.copy_from_slice(&signature_bytes[..32]);
        let mut s = [0u8; 32];
        s.copy_from_slice(&signature_bytes[32..]);

        Ok(PrimitiveSignature::from_scalars_and_parity(
            B256::from(r),
            B256::from(s),
            recovery_id.is_y_odd(),
        )
        .normalized_s())
    }
}

#[cfg(test)]
#[path = "crypto_tests.rs"]
mod tests;
