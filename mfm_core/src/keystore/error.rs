use thiserror::Error;
use uuid::Uuid;

// --- Error Enum ---
#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("Argon2 error: {0}")]
    Argon2Error(String), // Changed to manual wrapping for argon2::Error
    #[error("AES-GCM error: {0}")]
    AesGcm(String), // aes-gcm crate errors are not std::error::Error
    #[error("BIP-39 error: {0}")]
    Bip39(#[from] bip39::Error),
    #[error("BIP-32 error: {0}")]
    Bip32(#[from] bip32::Error),
    #[error("k256 error: {0}")]
    K256(#[from] k256::elliptic_curve::Error),
    #[error("Hex decoding error: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("Keystore is locked")]
    Locked,
    #[error("Invalid password")]
    InvalidPassword,
    #[error("Key not found: {0}")]
    KeyNotFound(Uuid),
    #[error("Invalid derivation path: {0}")]
    InvalidPath(String),
    #[error("Failed to derive key")]
    DerivationFailed,
    #[error("File system error: {0}")]
    FsError(String),
    #[error("Invalid keystore file format")]
    InvalidFormat,
    #[error("Master KDF parameters mismatch")]
    KdfParamsMismatch,
    #[error("Unsupported KDF: {0}")]
    UnsupportedKdf(String),
    #[error("Signature verification failed")]
    SignatureVerificationFailed,
    #[error("Missing master key")]
    MissingMasterKey,
    #[error("Alias already exists: {0}")]
    AliasExists(String),
    #[error("Private key is invalid")]
    InvalidPrivateKey,
    #[error("Invalid keystore configuration: {0}")]
    ConfigError(String),
    #[error("Invalid keystore file format: {0}")]
    InvalidKeystoreFormat(String),
    #[error("Master KDF parameters are missing when expected")]
    MissingMasterKdfParams,
    #[error("Failed to decrypt key entry (data may be corrupted or AAD mismatch)")]
    EntryDecryptionFailed,
    #[error("A lock was poisoned due to a panic in another thread holding the lock")]
    LockPoisoned,
    #[error("Migration attempted but not pending or no V1 data found")]
    MigrationNotPending,
    #[error("Password incorrect for V1 data migration or V1 data corrupted")]
    MigrationV1DataError, // Covers decryption failure of V1 entries
}
