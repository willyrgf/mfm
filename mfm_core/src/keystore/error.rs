// mfm_core/src/keystore/error.rs

use thiserror::Error;

#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("functionality not yet implemented: {0}")]
    NotImplemented(String),

    #[error("path error: {0}")]
    PathError(String),

    #[error("i/o error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("serialization/deserialization error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),

    #[error("hex decoding error: {0}")]
    HexError(#[from] hex::FromHexError),

    #[error("base64 encoding/decoding error: {0}")]
    Base64Error(#[from] base64::DecodeError), // Using DecodeError as a general proxy

    #[error("argon2 hashing error: {0}")]
    Argon2Error(String), // Manual From impl needed if argon2::Error doesn't impl std::error::Error

    #[error("bip39 mnemonic error: {0}")]
    Bip39Error(String), // bip39::Error doesn't impl std::Error directly in some versions

    #[error("bip32 derivation error: {0}")]
    Bip32Error(String), // bip32::Error might not be easily convertible

    #[error("cryptographic operation error: {0}")]
    CryptoError(String), // For general crypto errors, e.g., from `ring`

    #[error("invalid password")]
    InvalidPassword,

    #[error("keystore is locked")]
    KeystoreLocked,

    #[error("key not found for uuid: {0}")]
    KeyNotFound(String), // uuid::Uuid.to_string()

    #[error("invalid private key format: {0}")]
    InvalidPrivateKey(String),

    #[error("invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    #[error("password required for operation")]
    PasswordRequired,

    #[error("keystore file already exists")]
    FileAlreadyExists,

    #[error("keystore file not found")]
    FileNotFound,

    #[error("configuration error: {0}")]
    ConfigError(String),

    #[error("unknown keystore error: {0}")]
    Unknown(String),
}

// Helper for bip39::Error if needed, as it might not implement std::error::Error directly
// depending on the version or features.
// If bip39::Error can be directly used with #[from], this can be simplified.
impl From<bip39::Error> for KeystoreError {
    fn from(e: bip39::Error) -> Self {
        KeystoreError::Bip39Error(e.to_string())
    }
}

// Similar helper for bip32::Error if needed
impl From<bip32::Error> for KeystoreError {
    fn from(e: bip32::Error) -> Self {
        KeystoreError::Bip32Error(e.to_string())
    }
}

// Manual From impl for argon2::Error
impl From<argon2::Error> for KeystoreError {
    fn from(e: argon2::Error) -> Self {
        KeystoreError::Argon2Error(e.to_string())
    }
}
