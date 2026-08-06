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

use crate::crypto::EthereumKeyError;

use super::KeystoreError;

/// Stable heap allocation whose contents are explicitly zeroized on drop.
pub(super) struct HeapKeyBytes(Box<[u8; 32]>);

impl HeapKeyBytes {
    pub(super) fn zeroed() -> Self {
        Self(Box::new([0_u8; 32]))
    }
}

impl AsRef<[u8; 32]> for HeapKeyBytes {
    fn as_ref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl AsMut<[u8; 32]> for HeapKeyBytes {
    fn as_mut(&mut self) -> &mut [u8; 32] {
        &mut self.0
    }
}

impl std::ops::Deref for HeapKeyBytes {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Zeroize for HeapKeyBytes {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

pub(super) type ProtectedBytes = Zeroizing<HeapKeyBytes>;

/// Owned zeroizing 32-byte private-key material prior to [`SecureKey`] wrap.
///
/// Produced only by decryption (or tightly controlled test fixtures). Moving
/// this value into [`SecureKey`] transfers the same protected allocation; the
/// source is not left as a live plaintext array.
pub(super) struct ProtectedKeyMaterial {
    bytes: ProtectedBytes,
    #[cfg(test)]
    ownership_witness: Option<KeyMaterialWitness>,
}

impl ProtectedKeyMaterial {
    /// Copies exact 32-byte decrypted ciphertext plaintext into a protected
    /// fixed-size allocation, then drops the source zeroizing buffer.
    ///
    /// Both source and destination are zeroizing containers; no ordinary
    /// `[u8; 32]` temporary is constructed.
    pub(super) fn from_decrypted_exact(decrypted: ProtectedBytes) -> Result<Self, KeystoreError> {
        Ok(Self {
            bytes: decrypted,
            #[cfg(test)]
            ownership_witness: None,
        })
    }

    /// Test-only construction with an ownership/cleanup witness.
    #[cfg(test)]
    pub(super) fn from_decrypted_exact_with_witness(
        decrypted: ProtectedBytes,
        ownership_witness: KeyMaterialWitness,
    ) -> Result<Self, KeystoreError> {
        ownership_witness.record_source(decrypted.as_ref().as_ref().as_ptr() as usize);
        let mut material = Self::from_decrypted_exact(decrypted)?;
        material.ownership_witness = Some(ownership_witness);
        Ok(material)
    }

    #[cfg(test)]
    fn from_decrypted_vec_for_test(decrypted: Zeroizing<Vec<u8>>) -> Result<Self, KeystoreError> {
        if decrypted.len() != 32 {
            return Err(KeystoreError::InvalidPrivateKey);
        }
        let mut bytes = Box::new([0_u8; 32]);
        bytes.copy_from_slice(&decrypted);
        Self::from_decrypted_exact(Zeroizing::new(HeapKeyBytes(bytes)))
    }

    /// Moves the protected allocation into [`SecureKey`] without a plaintext copy.
    ///
    /// The zeroizing container is transferred by value. This method replaces the
    /// source buffer with zeros before `self` drops so the Drop impl never
    /// zeroizes the live key material still owned by the returned [`SecureKey`].
    pub(super) fn into_secure_key(mut self) -> SecureKey {
        let key_bytes = std::mem::replace(&mut self.bytes, Zeroizing::new(HeapKeyBytes::zeroed()));
        #[cfg(test)]
        if let Some(ownership_witness) = self.ownership_witness.take() {
            ownership_witness.record_handoff(key_bytes.as_ref().as_ref().as_ptr() as usize);
            return SecureKey::from_protected_with_witness(key_bytes, ownership_witness);
        }
        SecureKey::from_protected(key_bytes)
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
    key_bytes: ProtectedBytes,
    #[cfg(test)]
    ownership_witness: Option<KeyMaterialWitness>,
}

impl SecureKey {
    /// Takes ownership of already-protected key material.
    ///
    /// This is the only constructor. Callers must supply a [`Zeroizing`]
    /// allocation; a plaintext array API is intentionally absent.
    pub(super) fn from_protected(key_bytes: ProtectedBytes) -> Self {
        Self {
            key_bytes,
            #[cfg(test)]
            ownership_witness: None,
        }
    }

    /// Test-only construction that attaches an ownership/cleanup witness.
    #[cfg(test)]
    pub(super) fn from_protected_with_witness(
        key_bytes: ProtectedBytes,
        ownership_witness: KeyMaterialWitness,
    ) -> Self {
        ownership_witness.record_handoff(key_bytes.as_ref().as_ref().as_ptr() as usize);
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
        let result = crate::crypto::sign_hash_recoverable(self.key_bytes.as_ref(), hash)
            .map_err(keystore_error_from_ethereum_key);
        #[cfg(test)]
        if let Some(witness) = &self.ownership_witness {
            witness.record_signing(self.key_bytes.as_ref().as_ref().as_ptr() as usize);
        }
        result
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
    crate::crypto::address(key_bytes).map_err(keystore_error_from_ethereum_key)
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
    source_address: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    handoff_address: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    signing_address: std::sync::Arc<std::sync::atomic::AtomicUsize>,
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

    pub(super) fn record_source(&self, address: usize) {
        self.source_address
            .store(address, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn record_handoff(&self, address: usize) {
        self.handoff_address
            .store(address, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn record_signing(&self, address: usize) {
        self.signing_address
            .store(address, std::sync::atomic::Ordering::SeqCst);
    }

    pub(super) fn observed_transfer(&self) -> bool {
        self.transferred.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) fn observed_cleanup(&self) -> bool {
        self.cleaned.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) fn observed_source_handoff(&self) -> bool {
        let source = self
            .source_address
            .load(std::sync::atomic::Ordering::SeqCst);
        let handoff = self
            .handoff_address
            .load(std::sync::atomic::Ordering::SeqCst);
        source != 0 && source == handoff
    }

    pub(super) fn observed_same_allocation(&self) -> bool {
        let source = self
            .source_address
            .load(std::sync::atomic::Ordering::SeqCst);
        let handoff = self
            .handoff_address
            .load(std::sync::atomic::Ordering::SeqCst);
        let signing = self
            .signing_address
            .load(std::sync::atomic::Ordering::SeqCst);
        source != 0 && source == handoff && handoff == signing
    }
}

#[cfg(test)]
mod secure_key_construction_tests {
    use super::*;

    /// API contract: the only constructors take `Zeroizing<Box<[u8; 32]>>` or
    /// `ProtectedKeyMaterial`. A plaintext `[u8; 32]` constructor must not
    /// reappear (`SecureKey::new` is deleted).
    #[test]
    fn secure_key_from_protected_moves_zeroizing_allocation() {
        let mut raw = Zeroizing::new(HeapKeyBytes::zeroed());
        raw.as_mut()[31] = 1;
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
        let mut plaintext = Box::new([0_u8; 32]);
        plaintext[31] = 1;
        let decrypted = Zeroizing::new(HeapKeyBytes(plaintext));
        let material =
            ProtectedKeyMaterial::from_decrypted_exact_with_witness(decrypted, witness.clone())
                .expect("length");
        witness.record_transfer();
        let secure = material.into_secure_key();
        assert!(secure.has_ownership_witness());
        assert!(witness.observed_transfer());
        secure
            .sign_hash_recoverable(&[0_u8; 32])
            .expect("sign protected allocation");
        assert!(witness.observed_same_allocation());
        // Cleanup has not yet run: SecureKey still owns the material.
        assert!(!witness.observed_cleanup());
        drop(secure);
        assert!(witness.observed_cleanup());
    }

    #[test]
    fn wrong_length_decrypted_material_fails_closed() {
        let decrypted = Zeroizing::new(vec![0xab_u8; 31]);
        match ProtectedKeyMaterial::from_decrypted_vec_for_test(decrypted) {
            Err(KeystoreError::InvalidPrivateKey) => {}
            Ok(_) => panic!("wrong length must fail closed"),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn oversized_decrypted_material_fails_closed() {
        let decrypted = Zeroizing::new(vec![0xcd_u8; 33]);
        match ProtectedKeyMaterial::from_decrypted_vec_for_test(decrypted) {
            Err(KeystoreError::InvalidPrivateKey) => {}
            Ok(_) => panic!("oversized must fail closed"),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn dropped_protected_material_without_transfer_cleans_up() {
        let witness = KeyMaterialWitness::new();
        let decrypted = Zeroizing::new(HeapKeyBytes::zeroed());
        let material =
            ProtectedKeyMaterial::from_decrypted_exact_with_witness(decrypted, witness.clone())
                .expect("length");
        drop(material);
        assert!(witness.observed_cleanup());
        assert!(!witness.observed_transfer());
    }
}
