use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("Keystore already unlocked")]
    AlreadyUnlocked,
    #[error("Keystore is locked")]
    Locked,
    #[error("Invalid password")]
    InvalidPassword,
    #[error("Key with UUID {0} not found")]
    KeyNotFound(Uuid),
    #[error("Key with alias {0} already exists")]
    AliasAlreadyExists(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Hex decoding error: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("Base64 decoding error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Argon2 error: {0}")]
    Argon2(#[from] argon2::Error),
    #[error("AES-GCM error: {0}")]
    AesGcm(#[from] aes_gcm::Error),
    #[error("BIP39 error: {0}")]
    Bip39(#[from] bip39::Error),
    #[error("BIP32 error: {0}")]
    Bip32(#[from] bip32::Error),
    #[error("k256 error: {0}")]
    K256(#[from] k256::ecdsa::Error),
    #[error("UUID error: {0}")]
    Uuid(#[from] uuid::Error),
    #[error("Atomic writes error: {0}")]
    AtomicWrites(#[from] atomicwrites::AtomicWriteError<std::io::Error>),
    #[error("FS2 error: {0}")]
    Fs2(#[from] fs2::Error),
    #[error("Other error: {0}")]
    Other(String),
}
