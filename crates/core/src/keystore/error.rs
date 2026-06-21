/// Simplified error types for minimal keystore
#[derive(Debug, Clone)]
pub enum KeystoreError {
    /// The supplied password did not unlock the keystore.
    InvalidPassword,

    /// The keystore must be unlocked before the operation can proceed.
    Locked,

    /// The requested key identifier does not exist in the keystore.
    KeyNotFound(uuid::Uuid),

    /// The supplied private key bytes or hex string were invalid.
    InvalidPrivateKey,

    /// The supplied mnemonic phrase failed validation.
    InvalidMnemonic(String),

    /// The supplied derivation path failed validation.
    InvalidDerivationPath(String),

    /// A cryptographic primitive returned an error.
    CryptoError(String),

    /// A filesystem operation failed.
    FileError(String),

    /// Serialization or deserialization failed.
    SerializationError(String),

    /// The caller supplied an invalid input value.
    InvalidInput(String),

    /// The requested operation is disabled by policy.
    OperationNotPermitted(String),
}

impl std::fmt::Display for KeystoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPassword => f.write_str("Invalid password"),
            Self::Locked => f.write_str("Keystore is locked"),
            Self::KeyNotFound(id) => write!(f, "Key not found: {id}"),
            Self::InvalidPrivateKey => f.write_str("Invalid private key format"),
            Self::InvalidMnemonic(message) => write!(f, "Invalid mnemonic: {message}"),
            Self::InvalidDerivationPath(message) => {
                write!(f, "Invalid derivation path: {message}")
            }
            Self::CryptoError(message) => {
                write!(f, "Cryptographic operation failed: {message}")
            }
            Self::FileError(message) => write!(f, "File operation failed: {message}"),
            Self::SerializationError(message) => {
                write!(f, "Serialization failed: {message}")
            }
            Self::InvalidInput(message) => write!(f, "Invalid input: {message}"),
            Self::OperationNotPermitted(message) => {
                write!(f, "Operation not permitted: {message}")
            }
        }
    }
}

impl std::error::Error for KeystoreError {}

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
