use alloy_primitives::Address;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyType {
    MnemonicDerived,
    PrivateKeyImported,
    // Future: ExternalSigner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedKeyEntry {
    pub uuid: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub key_type: KeyType,
    pub encrypted_material: String, // Base64 encoded
    pub encryption_details: EncryptionDetails,
    pub derivation_path: Option<String>,
    pub created_at: String, // Consider chrono::DateTime<Utc>
    pub updated_at: String, // Consider chrono::DateTime<Utc>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionDetails {
    pub cipher: String, // e.g., "aes-256-gcm"
    pub cipher_params: CipherParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipherParams {
    pub nonce: String, // Hex encoded nonce/IV
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterKdfParams {
    pub salt: String,    // Hex encoded salt for master password Argon2 derivation
    pub m_cost: u32,     // Memory cost (KiB)
    pub t_cost: u32,     // Time cost (iterations)
    pub p_cost: u32,     // Parallelism factor
    pub output_len: u32, // Desired key length in bytes
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeystoreFile {
    pub version: String,    // Keystore format version
    pub master_kdf: String, // e.g., "argon2id"
    pub master_kdf_params: MasterKdfParams,
    pub entries: Vec<EncryptedKeyEntry>,
}

// Information returned when listing keys (non-sensitive)
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub uuid: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub key_type: KeyType,
    pub created_at: String, // Consider chrono::DateTime<Utc>
    pub updated_at: String, // Consider chrono::DateTime<Utc>
}

// Zeroize implementation for sensitive data if needed, though Zeroizing wrapper is preferred for keys
// Example:
#[derive(Zeroize)]
#[zeroize(drop)]
pub struct SensitiveBytes(pub Vec<u8>);

impl SensitiveBytes {
    pub fn new(data: Vec<u8>) -> Self {
        SensitiveBytes(data)
    }
}
