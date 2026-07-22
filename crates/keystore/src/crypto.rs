//! Crate-private Ethereum private-key parsing, address derivation, and prehash signing.

use alloy_primitives::{Address, PrimitiveSignature, B256};
use k256::ecdsa::SigningKey;
use k256::SecretKey;
use tiny_keccak::{Hasher, Keccak};
use zeroize::Zeroizing;

/// Error returned by Ethereum private-key helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum EthereumKeyError {
    /// The supplied bytes did not form a valid secp256k1 private key.
    #[error("signing key did not form a valid secp256k1 key")]
    InvalidPrivateKey,
    /// The signing primitive failed.
    #[error("failed to sign prehashed payload")]
    SigningFailed,
}

/// Zeroizing wrapper for a raw Ethereum secp256k1 private key.
pub(crate) struct EthereumPrivateKey {
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
    /// Builds a private key from already-decoded secret bytes.
    pub(crate) fn from_secret_bytes(key_bytes: [u8; 32]) -> Result<Self, EthereumKeyError> {
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
    pub(crate) fn address(&self) -> Result<Address, EthereumKeyError> {
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
    pub(crate) fn sign_hash_recoverable(
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
