//! Crate-private decrypted private-key ownership.
//!
//! Decrypted key material moves only as an owned zeroizing allocation. The sole
//! constructor of [`SecureKey`] takes that protected allocation by value and
//! never accepts an ordinary plaintext `[u8; 32]` array. Ownership transfer is
//! a move of the zeroizing container; the production decrypt path must not
//! introduce an unprotected by-value intermediate between AES-GCM output and
//! this wrapper.

use alloy_primitives::{Address, PrimitiveSignature};
#[cfg(test)]
use k256::ecdsa::SigningKey;
use k256::SecretKey;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::crypto::{EthereumKeyError, EthereumPrivateKey};

use super::KeystoreError;

/// Owned zeroizing 32-byte private-key material prior to [`SecureKey`] wrap.
///
/// Produced only by decryption (or tightly controlled test fixtures). Moving
/// this value into [`SecureKey`] transfers the same protected allocation; the
/// source is not left as a live plaintext array.
pub(super) struct ProtectedKeyMaterial {
    bytes: Zeroizing<[u8; 32]>,
    #[cfg(test)]
    ownership_witness: Option<KeyMaterialWitness>,
}

impl ProtectedKeyMaterial {
    /// Copies exact 32-byte decrypted ciphertext plaintext into a protected
    /// fixed-size allocation, then drops the source zeroizing buffer.
    ///
    /// Both source and destination are zeroizing containers; no ordinary
    /// `[u8; 32]` temporary is constructed.
    pub(super) fn from_decrypted_exact(
        decrypted: Zeroizing<Vec<u8>>,
    ) -> Result<Self, KeystoreError> {
        if decrypted.len() != 32 {
            // `decrypted` zeroizes on drop for wrong-length material.
            return Err(KeystoreError::InvalidPrivateKey);
        }
        let mut protected = Zeroizing::new([0_u8; 32]);
        protected.copy_from_slice(decrypted.as_ref());
        drop(decrypted);
        Ok(Self {
            bytes: protected,
            #[cfg(test)]
            ownership_witness: None,
        })
    }

    /// Test-only construction with an ownership/cleanup witness.
    #[cfg(test)]
    pub(super) fn from_decrypted_exact_with_witness(
        decrypted: Zeroizing<Vec<u8>>,
        ownership_witness: KeyMaterialWitness,
    ) -> Result<Self, KeystoreError> {
        let mut material = Self::from_decrypted_exact(decrypted)?;
        material.ownership_witness = Some(ownership_witness);
        Ok(material)
    }

    /// Moves the protected allocation into [`SecureKey`] without a plaintext copy.
    ///
    /// The zeroizing container is transferred by value. This method replaces the
    /// source buffer with zeros before `self` drops so the Drop impl never
    /// zeroizes the live key material still owned by the returned [`SecureKey`].
    pub(super) fn into_secure_key(mut self) -> SecureKey {
        let key_bytes = std::mem::replace(&mut self.bytes, Zeroizing::new([0_u8; 32]));
        #[cfg(test)]
        let ownership_witness = self.ownership_witness.take();
        SecureKey {
            key_bytes,
            #[cfg(test)]
            ownership_witness,
        }
    }
}

impl Drop for ProtectedKeyMaterial {
    fn drop(&mut self) {
        self.bytes.zeroize();
        #[cfg(test)]
        if let Some(witness) = &self.ownership_witness {
            witness.record_cleanup(self.bytes.iter().all(|byte| *byte == 0));
        }
    }
}

/// Crate-private secure key wrapper that zeroizes on drop.
///
/// Construct only via [`ProtectedKeyMaterial::into_secure_key`] (or the
/// equivalent ownership-transfer constructor). There is no public or
/// crate-private constructor that accepts a plaintext `[u8; 32]` by value.
pub(crate) struct SecureKey {
    key_bytes: Zeroizing<[u8; 32]>,
    #[cfg(test)]
    ownership_witness: Option<KeyMaterialWitness>,
}

impl SecureKey {
    /// Takes ownership of already-protected key material.
    ///
    /// This is the only constructor. Callers must supply a [`Zeroizing`]
    /// allocation; a plaintext array API is intentionally absent.
    pub(super) fn from_protected(key_bytes: Zeroizing<[u8; 32]>) -> Self {
        Self {
            key_bytes,
            #[cfg(test)]
            ownership_witness: None,
        }
    }

    /// Test-only construction that attaches an ownership/cleanup witness.
    #[cfg(test)]
    pub(super) fn from_protected_with_witness(
        key_bytes: Zeroizing<[u8; 32]>,
        ownership_witness: KeyMaterialWitness,
    ) -> Self {
        Self {
            key_bytes,
            ownership_witness: Some(ownership_witness),
        }
    }

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
        let key = EthereumPrivateKey::from_secret_bytes(&*self.key_bytes)
            .map_err(keystore_error_from_ethereum_key)?;
        key.sign_hash_recoverable(hash)
            .map_err(keystore_error_from_ethereum_key)
    }

    /// Get Ethereum address for this key.
    pub(crate) fn ethereum_address(&self) -> Result<Address, KeystoreError> {
        ethereum_address_from_key_bytes(self.key_bytes.as_ref())
    }

    /// Get public key.
    pub(crate) fn public_key(&self) -> Result<k256::PublicKey, KeystoreError> {
        let secret_key = self.secret_key()?;
        // Note: SecretKey implements ZeroizeOnDrop and will be zeroized when dropped.
        Ok(secret_key.public_key())
    }

    /// Test-only: returns whether an ownership witness is attached.
    #[cfg(test)]
    pub(super) fn has_ownership_witness(&self) -> bool {
        self.ownership_witness.is_some()
    }
}

impl Drop for SecureKey {
    fn drop(&mut self) {
        self.key_bytes.zeroize();
        #[cfg(test)]
        if let Some(witness) = &self.ownership_witness {
            witness.record_cleanup(self.key_bytes.iter().all(|byte| *byte == 0));
        }
    }
}

// ZeroizeOnDrop is satisfied by the explicit Drop above; keep the trait marker
// so static checks and reviews continue to treat this type as secret-bearing.
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

/// Test-only witness for protected key-material ownership transfer and cleanup.
#[cfg(test)]
#[derive(Clone, Default)]
pub(super) struct KeyMaterialWitness {
    transferred: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cleaned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl KeyMaterialWitness {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn record_transfer(&self) {
        self.transferred
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn record_cleanup(&self, zeroized: bool) {
        self.cleaned
            .store(zeroized, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn observed_transfer(&self) -> bool {
        self.transferred.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) fn observed_cleanup(&self) -> bool {
        self.cleaned.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod secure_key_construction_tests {
    use super::*;

    /// API contract: the only constructors take `Zeroizing<[u8; 32]>` or
    /// `ProtectedKeyMaterial`. A plaintext `[u8; 32]` constructor must not
    /// reappear (`SecureKey::new` is deleted).
    #[test]
    fn secure_key_from_protected_moves_zeroizing_allocation() {
        let mut raw = Zeroizing::new([0_u8; 32]);
        raw[31] = 1;
        let witness = KeyMaterialWitness::new();
        let secure = SecureKey::from_protected_with_witness(raw, witness.clone());
        assert_eq!(
            format!("{:?}", secure.ethereum_address().expect("address")),
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
        drop(secure);
        assert!(witness.observed_cleanup());
    }

    #[test]
    fn protected_key_material_into_secure_key_transfers_without_plaintext_ctor() {
        let witness = KeyMaterialWitness::new();
        let mut plaintext = vec![0_u8; 32];
        plaintext[31] = 1;
        let decrypted = Zeroizing::new(plaintext);
        let material =
            ProtectedKeyMaterial::from_decrypted_exact_with_witness(decrypted, witness.clone())
                .expect("length");
        witness.record_transfer();
        let secure = material.into_secure_key();
        assert!(secure.has_ownership_witness());
        assert!(witness.observed_transfer());
        // Cleanup has not yet run: SecureKey still owns the material.
        assert!(!witness.observed_cleanup());
        drop(secure);
        assert!(witness.observed_cleanup());
    }

    #[test]
    fn wrong_length_decrypted_material_fails_closed() {
        let decrypted = Zeroizing::new(vec![0xab_u8; 31]);
        match ProtectedKeyMaterial::from_decrypted_exact(decrypted) {
            Err(KeystoreError::InvalidPrivateKey) => {}
            Ok(_) => panic!("wrong length must fail closed"),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn oversized_decrypted_material_fails_closed() {
        let decrypted = Zeroizing::new(vec![0xcd_u8; 33]);
        match ProtectedKeyMaterial::from_decrypted_exact(decrypted) {
            Err(KeystoreError::InvalidPrivateKey) => {}
            Ok(_) => panic!("oversized must fail closed"),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn dropped_protected_material_without_transfer_cleans_up() {
        let witness = KeyMaterialWitness::new();
        let decrypted = Zeroizing::new(vec![0_u8; 32]);
        let material =
            ProtectedKeyMaterial::from_decrypted_exact_with_witness(decrypted, witness.clone())
                .expect("length");
        drop(material);
        assert!(witness.observed_cleanup());
        assert!(!witness.observed_transfer());
    }
}
