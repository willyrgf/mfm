// mfm_core/src/keystore/types.rs

use alloy_primitives::Address;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type of key stored.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum KeyType {
    /// Key was derived from a mnemonic phrase.
    MnemonicDerived,
    /// Key was imported directly as a private key hex.
    PrivateKeyImported,
}

/// Parameters for the Key Derivation Function (Argon2).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KdfParams {
    pub salt: String, // Hex-encoded salt
    // Argon2 specific parameters
    pub m_cost: u32,       // Memory cost (in kibibytes)
    pub t_cost: u32,       // Time cost (iterations)
    pub p_cost: u32,       // Parallelism cost (degree of parallelism)
    pub output_len: usize, // Length of the derived key
}

/// Parameters for the symmetric cipher (AES-GCM).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CipherParams {
    pub nonce: String, // Hex-encoded nonce/IV
}

/// Represents an encrypted key entry as stored in the keystore file.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedKeyEntry {
    pub uuid: String,          // UUID v4 string, primary identifier
    pub alias: Option<String>, // User-defined friendly name
    pub address: String,       // Public Ethereum address, hex-encoded (checksummed?)
    pub key_type: KeyType,
    /// Base64 encoded encrypted private key bytes.
    pub encrypted_material: String,
    /// Parameters for the master key KDF (Argon2).
    /// Note: This might be global to the keystore file rather than per-entry if one master password unlocks all.
    /// For simplicity in this structure, assuming it could be per-entry if designs change,
    /// but typically the KDF params for the *master key* are stored once.
    /// If the master key is derived once and used for all entries, these KDF params might live
    /// in a top-level structure in the keystore file.
    /// Let's assume for now these are the params used to encrypt *this specific entry's material*
    /// if we were to use per-entry passwords, OR they are a copy of global KDF params if a global password.
    /// For a global password, the salt for the master password KDF would be stored once.
    /// Let's refine this: `kdf_params` here will refer to the KDF used for the *master password* that protects the whole keystore.
    /// It will be the same for all entries if there's one master password.
    pub master_kdf_params: KdfParams, // Parameters for the KDF that derived the master encryption key
    /// Parameters for the cipher used to encrypt `encrypted_material`.
    pub cipher_params: CipherParams,
    /// Derivation path, if `key_type` is `MnemonicDerived`.
    pub derivation_path: Option<String>, // e.g., "m/44'/60'/0'/0/0"
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Information about a key, suitable for listing (non-sensitive).
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub uuid: Uuid,
    pub alias: Option<String>,
    pub address: Address,
    pub key_type: KeyType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
