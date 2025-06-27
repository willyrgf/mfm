/// Simplified error types for minimal keystore
#[derive(Debug, Clone, thiserror::Error)]
pub enum KeystoreError {
    #[error("Invalid password")]
    InvalidPassword,

    #[error("Keystore is locked")]
    Locked,

    #[error("Key not found: {0}")]
    KeyNotFound(uuid::Uuid),

    #[error("Invalid private key format")]
    InvalidPrivateKey,

    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    #[error("Invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    #[error("Cryptographic operation failed: {0}")]
    CryptoError(String),

    #[error("File operation failed: {0}")]
    FileError(String),

    #[error("Serialization failed: {0}")]
    SerializationError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),
}

// Convert from common error types
impl From<std::io::Error> for KeystoreError {
    fn from(err: std::io::Error) -> Self {
        KeystoreError::FileError(err.to_string())
    }
}

impl From<serde_json::Error> for KeystoreError {
    fn from(err: serde_json::Error) -> Self {
        KeystoreError::SerializationError(err.to_string())
    }
}

impl From<aes_gcm::Error> for KeystoreError {
    fn from(err: aes_gcm::Error) -> Self {
        KeystoreError::CryptoError(format!("AES-GCM error: {:?}", err))
    }
}

impl From<argon2::Error> for KeystoreError {
    fn from(err: argon2::Error) -> Self {
        KeystoreError::CryptoError(format!("Argon2 error: {:?}", err))
    }
}

impl From<bip32::Error> for KeystoreError {
    fn from(err: bip32::Error) -> Self {
        KeystoreError::InvalidDerivationPath(err.to_string())
    }
}

impl From<bip39::Error> for KeystoreError {
    fn from(err: bip39::Error) -> Self {
        KeystoreError::InvalidMnemonic(err.to_string())
    }
}
