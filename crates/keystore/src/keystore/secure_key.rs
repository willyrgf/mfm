use alloy_primitives::{Address, PrimitiveSignature};
#[cfg(test)]
use k256::{ecdsa::SigningKey, SecretKey};
use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::crypto::{EthereumKeyError, EthereumPrivateKey};

use super::KeystoreError;

/// Crate-private secure key wrapper that zeroizes on drop.
pub(crate) struct SecureKey {
    key_bytes: Zeroizing<[u8; 32]>,
}

impl SecureKey {
    pub(super) fn new(key_bytes: [u8; 32]) -> Self {
        Self {
            key_bytes: Zeroizing::new(key_bytes),
        }
    }

    #[cfg(test)]
    fn secret_key(&self) -> Result<SecretKey, KeystoreError> {
        SecretKey::from_slice(self.key_bytes.as_ref()).map_err(|_| KeystoreError::InvalidPrivateKey)
    }

    /// Sign a 32-byte hash (returns k256::Signature).
    #[cfg(test)]
    pub(crate) fn sign_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<k256::ecdsa::Signature, KeystoreError> {
        let secret_key = self.secret_key()?;
        let signing_key = SigningKey::from(&secret_key);

        use k256::ecdsa::signature::hazmat::PrehashSigner;
        let result = signing_key
            .sign_prehash(hash)
            .map_err(|e| KeystoreError::CryptoError(format!("Signing failed: {e}")));

        // Note: SecretKey and SigningKey implement ZeroizeOnDrop automatically
        // via the k256 crate, so they will be zeroized when dropped.
        result
    }

    /// Sign a 32-byte hash and return an Ethereum recoverable signature.
    pub(crate) fn sign_hash_recoverable(
        &self,
        hash: &[u8; 32],
    ) -> Result<PrimitiveSignature, KeystoreError> {
        let key = EthereumPrivateKey::from_secret_bytes(&self.key_bytes)
            .map_err(keystore_error_from_ethereum_key)?;
        key.sign_hash_recoverable(hash)
            .map_err(keystore_error_from_ethereum_key)
    }

    /// Get Ethereum address for this key.
    pub(crate) fn ethereum_address(&self) -> Result<Address, KeystoreError> {
        ethereum_address_from_key_bytes(self.key_bytes.as_ref())
    }

    /// Get public key.
    #[cfg(test)]
    pub(crate) fn public_key(&self) -> Result<k256::PublicKey, KeystoreError> {
        let secret_key = self.secret_key()?;
        // Note: SecretKey implements ZeroizeOnDrop and will be zeroized when dropped.
        Ok(secret_key.public_key())
    }
}

impl ZeroizeOnDrop for SecureKey {}

pub(super) fn ethereum_address_from_key_bytes(key_bytes: &[u8]) -> Result<Address, KeystoreError> {
    let key_bytes: &[u8; 32] = key_bytes
        .try_into()
        .map_err(|_| KeystoreError::InvalidPrivateKey)?;
    let key = EthereumPrivateKey::from_secret_bytes(key_bytes)
        .map_err(keystore_error_from_ethereum_key)?;
    key.address().map_err(keystore_error_from_ethereum_key)
}

fn keystore_error_from_ethereum_key(err: EthereumKeyError) -> KeystoreError {
    match err {
        EthereumKeyError::InvalidPrivateKey => KeystoreError::InvalidPrivateKey,
        EthereumKeyError::SigningFailed => {
            KeystoreError::CryptoError("failed to sign prehashed payload".to_string())
        }
    }
}
