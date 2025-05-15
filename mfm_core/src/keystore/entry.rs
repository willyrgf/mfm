use alloy_primitives::Address;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedKeyEntry {
    pub id: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub encrypted_private_key: Vec<u8>,
    pub nonce: Vec<u8>,                        // AES-GCM nonce
    pub kdf_params: Option<KeyEntryKdfParams>, // For future per-key KDF if needed
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyInfo {
    pub id: Uuid,
    pub alias: Option<String>,
    pub address: Address,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyEntryKdfParams {
    // Define parameters for per-key KDF if implemented
}

#[derive(Debug, Clone, Zeroize, ZeroizeOnDrop)]
pub struct DecryptedPrivateKey(pub Vec<u8>);

impl AsRef<[u8]> for DecryptedPrivateKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
