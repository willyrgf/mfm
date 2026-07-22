/// Simplified error types for minimal keystore
#[derive(Debug, Clone, thiserror::Error)]
pub enum KeystoreError {
    /// The supplied password did not unlock the keystore.
    #[error("Invalid password")]
    InvalidPassword,

    /// The keystore must be unlocked before the operation can proceed.
    #[error("Keystore is locked")]
    Locked,

    /// The requested key identifier does not exist in the keystore.
    #[error("Key not found: {0}")]
    KeyNotFound(uuid::Uuid),

    /// The supplied private key bytes or hex string were invalid.
    #[error("Invalid private key format")]
    InvalidPrivateKey,

    /// The supplied mnemonic phrase failed validation.
    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    /// The supplied derivation path failed validation.
    #[error("Invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    /// A cryptographic primitive returned an error.
    #[error("Cryptographic operation failed: {0}")]
    CryptoError(String),

    /// A filesystem operation failed.
    #[error("File operation failed: {0}")]
    FileError(String),

    /// Serialization or deserialization failed.
    #[error("Serialization failed: {0}")]
    SerializationError(String),

    /// The caller supplied an invalid input value.
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
        KeystoreError::CryptoError(format!("AES-GCM error: {err:?}"))
    }
}

impl From<argon2::Error> for KeystoreError {
    fn from(err: argon2::Error) -> Self {
        KeystoreError::CryptoError(format!("Argon2 error: {err:?}"))
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
