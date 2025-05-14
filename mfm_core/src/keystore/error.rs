use thiserror::Error;

#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("keystore is locked")]
    Locked,
    #[error("invalid password")]
    InvalidPassword,
    #[error("file error: {0}")]
    FileError(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("decryption error")]
    DecryptionError,
    #[error("encryption error")]
    EncryptionError,
    #[error("invalid mnemonic phrase")]
    InvalidMnemonic,
    #[error("invalid derivation path")]
    InvalidDerivationPath,
    #[error("invalid private key hex")]
    InvalidPrivateKeyHex,
    #[error("key with UUID {0} not found")]
    KeyNotFound(uuid::Uuid),
    #[error("key with alias {0} already exists")]
    AliasAlreadyExists(String),
    #[error("key with address {0} already exists")]
    AddressAlreadyExists(alloy_primitives::Address), // Added AddressAlreadyExists variant
    #[error("atomic write error: {0}")]
    AtomicWriteError(String), // Placeholder for atomicwrites or manual impl errors
    #[error("other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, KeystoreError>;
