#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! Encrypted storage wrapper for secret-bearing artifacts.
//!
//! This crate encrypts secret payloads before delegating persistence to another
//! [`ArtifactStore`](mfm_machine::stores::ArtifactStore). Plaintext secrets MUST NOT be stored
//! directly in the underlying artifact store.
//!
//! # Security
//!
//! The encryption envelope uses AES-256-GCM with fixed additional authenticated data that binds
//! ciphertexts to this storage format. Secret keys and decrypted buffers are kept in zeroizing
//! containers where possible.
//!
//! # Examples
//!
//! ```rust
//! use mfm_artifact_store_secret::SecretKey;
//!
//! let _key = SecretKey::from_hex(
//!     "000102030405060708090a0b0c0d0e0f000102030405060708090a0b0c0d0e0f",
//! )
//! .expect("valid 32-byte key");
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::ids::{ArtifactId, ErrorCode};
use mfm_machine::stores::{ArtifactKind, ArtifactStore};
use ring::aead;
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

const CODE_SECRET_KEY_MISSING: &str = "secret_key_missing";
const CODE_SECRET_KEY_INVALID: &str = "secret_key_invalid";
const CODE_SECRET_ENCRYPT_FAILED: &str = "secret_encrypt_failed";
const CODE_SECRET_CIPHERTEXT_INVALID: &str = "secret_ciphertext_invalid";
const CODE_SECRET_DECRYPT_FAILED: &str = "secret_decrypt_failed";
const CODE_SECRET_PAYLOAD_REQUIRES_PROTECTED_PATH: &str = "secret_payload_requires_protected_path";

/// Environment variable that carries the secret-artifact encryption key.
const ENV_SECRET_KEY_HEX: &str = "MFM_SECRET_KEY_HEX";

/// Public envelope prefix for protected secret/capability artifacts.
///
/// Generic artifact download surfaces use this prefix to refuse returning protected ciphertext.
pub const SECRET_PAYLOAD_ENVELOPE_MAGIC: &[u8] = b"mfm:secret-payload:v1\0";

// Envelope format:
// - magic: SECRET_PAYLOAD_ENVELOPE_MAGIC
// - version: u8
// - nonce: [u8; 12]
// - ciphertext + tag: bytes
const ENVELOPE_VERSION_V1: u8 = 1;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const VERSION_OFFSET: usize = SECRET_PAYLOAD_ENVELOPE_MAGIC.len();
const NONCE_OFFSET: usize = VERSION_OFFSET + 1;
const HEADER_LEN: usize = NONCE_OFFSET + NONCE_LEN;

// AAD is constant and binds ciphertext to this specific use.
const AAD_V1: &[u8] = b"mfm:secret_payload:v1";

/// Returns true when `bytes` is a protected secret/capability artifact envelope.
pub fn is_secret_payload_envelope(bytes: &[u8]) -> bool {
    bytes.starts_with(SECRET_PAYLOAD_ENVELOPE_MAGIC)
}

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn other(code: &'static str, category: ErrorCategory, message: &'static str) -> StorageError {
    StorageError::Other(info(code, category, message))
}

/// 256-bit encryption key used for secret artifact payloads.
#[derive(Clone)]
pub struct SecretKey {
    bytes: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(REDACTED)")
    }
}

impl SecretKey {
    /// Creates a secret key from raw 32-byte key material.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    /// Parses a 32-byte secret key from a hex string.
    pub fn from_hex(s: &str) -> Result<Self, StorageError> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        let decoded = Zeroizing::new(hex::decode(s).map_err(|_| {
            other(
                CODE_SECRET_KEY_INVALID,
                ErrorCategory::ParsingInput,
                "invalid secret key hex",
            )
        })?);
        if decoded.len() != 32 {
            return Err(other(
                CODE_SECRET_KEY_INVALID,
                ErrorCategory::ParsingInput,
                "invalid secret key length",
            ));
        }

        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&decoded);
        Ok(Self::from_bytes(bytes))
    }

    /// Reads and parses [`ENV_SECRET_KEY_HEX`].
    pub fn from_env() -> Result<Self, StorageError> {
        let v = Zeroizing::new(std::env::var(ENV_SECRET_KEY_HEX).map_err(|_| {
            other(
                CODE_SECRET_KEY_MISSING,
                ErrorCategory::ParsingInput,
                "missing MFM_SECRET_KEY_HEX",
            )
        })?);
        Self::from_hex(&v)
    }

    /// Reads [`ENV_SECRET_KEY_HEX`] when present, returning `None` when protected storage is unset.
    pub fn from_optional_env() -> Result<Option<Self>, StorageError> {
        match std::env::var(ENV_SECRET_KEY_HEX) {
            Ok(value) => Self::from_hex(&value).map(Some),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(other(
                CODE_SECRET_KEY_INVALID,
                ErrorCategory::ParsingInput,
                "invalid secret key environment value",
            )),
        }
    }
}

/// Artifact-store wrapper that encrypts secret payloads before persistence.
#[derive(Clone)]
pub struct SecretArtifactStore {
    inner: Arc<dyn ArtifactStore>,
    key: SecretKey,
    rng: SystemRandom,
}

impl SecretArtifactStore {
    /// Creates a new secret store backed by `inner`.
    pub fn new(inner: Arc<dyn ArtifactStore>, key: SecretKey) -> Self {
        Self {
            inner,
            key,
            rng: SystemRandom::new(),
        }
    }

    /// Builds a secret store using [`ENV_SECRET_KEY_HEX`] for its encryption key.
    pub fn from_env(inner: Arc<dyn ArtifactStore>) -> Result<Self, StorageError> {
        let key = SecretKey::from_env()?;
        Ok(Self::new(inner, key))
    }

    /// Encrypts and stores secret bytes in the underlying artifact store.
    pub async fn put_secret_bytes(&self, plaintext: &[u8]) -> Result<ArtifactId, StorageError> {
        let ciphertext = self.seal_v1(plaintext)?;
        self.inner
            .put(ArtifactKind::SecretPayload, ciphertext)
            .await
    }

    /// Loads and decrypts secret bytes from the underlying artifact store.
    pub async fn get_secret_bytes(
        &self,
        id: &ArtifactId,
    ) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        let ciphertext = self.inner.get(id).await?;
        self.open_v1(&ciphertext)
    }

    fn seal_v1(&self, plaintext: &[u8]) -> Result<Vec<u8>, StorageError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce_bytes).map_err(|_| {
            other(
                CODE_SECRET_ENCRYPT_FAILED,
                ErrorCategory::Unknown,
                "rng failed",
            )
        })?;

        let unbound =
            aead::UnboundKey::new(&aead::AES_256_GCM, &self.key.bytes[..]).map_err(|_| {
                other(
                    CODE_SECRET_KEY_INVALID,
                    ErrorCategory::ParsingInput,
                    "invalid secret key",
                )
            })?;
        let key = aead::LessSafeKey::new(unbound);

        // Keep plaintext in an owned, zeroizing buffer while sealing in-place.
        let mut in_out = Zeroizing::new(plaintext.to_vec());
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::from(AAD_V1),
            &mut *in_out,
        )
        .map_err(|_| {
            other(
                CODE_SECRET_ENCRYPT_FAILED,
                ErrorCategory::Unknown,
                "secret encryption failed",
            )
        })?;

        let mut out = Vec::with_capacity(HEADER_LEN + in_out.len());
        out.extend_from_slice(SECRET_PAYLOAD_ENVELOPE_MAGIC);
        out.push(ENVELOPE_VERSION_V1);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&in_out);
        Ok(out)
    }

    fn open_v1(&self, envelope: &[u8]) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        if envelope.len() < HEADER_LEN + TAG_LEN {
            return Err(other(
                CODE_SECRET_CIPHERTEXT_INVALID,
                ErrorCategory::ParsingInput,
                "invalid secret envelope",
            ));
        }
        if !is_secret_payload_envelope(envelope) {
            return Err(other(
                CODE_SECRET_CIPHERTEXT_INVALID,
                ErrorCategory::ParsingInput,
                "invalid secret envelope magic",
            ));
        }
        if envelope[VERSION_OFFSET] != ENVELOPE_VERSION_V1 {
            return Err(other(
                CODE_SECRET_CIPHERTEXT_INVALID,
                ErrorCategory::ParsingInput,
                "unsupported secret envelope version",
            ));
        }

        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&envelope[NONCE_OFFSET..HEADER_LEN]);

        let unbound =
            aead::UnboundKey::new(&aead::AES_256_GCM, &self.key.bytes[..]).map_err(|_| {
                other(
                    CODE_SECRET_KEY_INVALID,
                    ErrorCategory::ParsingInput,
                    "invalid secret key",
                )
            })?;
        let key = aead::LessSafeKey::new(unbound);

        let mut in_out = Zeroizing::new(envelope[HEADER_LEN..].to_vec());
        let plaintext_len = {
            let pt = key
                .open_in_place(
                    aead::Nonce::assume_unique_for_key(nonce_bytes),
                    aead::Aad::from(AAD_V1),
                    &mut in_out,
                )
                .map_err(|_| {
                    other(
                        CODE_SECRET_DECRYPT_FAILED,
                        ErrorCategory::Unknown,
                        "secret decryption failed",
                    )
                })?;
            pt.len()
        };

        in_out.truncate(plaintext_len);
        Ok(in_out)
    }
}

#[async_trait]
impl ArtifactStore for SecretArtifactStore {
    async fn put(&self, kind: ArtifactKind, bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        if matches!(kind, ArtifactKind::SecretPayload) {
            return Err(other(
                CODE_SECRET_PAYLOAD_REQUIRES_PROTECTED_PATH,
                ErrorCategory::Storage,
                "secret payloads must be written through put_protected_bytes",
            ));
        }

        self.inner.put(kind, bytes).await
    }

    async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
        self.inner.get(id).await
    }

    async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
        self.inner.exists(id).await
    }

    async fn put_protected_bytes(
        &self,
        bytes: Zeroizing<Vec<u8>>,
    ) -> Result<ArtifactId, StorageError> {
        self.put_secret_bytes(&bytes).await
    }

    async fn get_protected_bytes(
        &self,
        id: &ArtifactId,
    ) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        self.get_secret_bytes(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_artifact_store_fs::FsArtifactStore;

    fn random_key() -> [u8; 32] {
        let rng = SystemRandom::new();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes).expect("rng");
        bytes
    }

    fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[tokio::test]
    async fn roundtrip_encrypts_and_decrypts() {
        let dir = tempfile::tempdir().unwrap();
        let inner: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(dir.path()));

        let store = SecretArtifactStore::new(inner, SecretKey::from_bytes(random_key()));

        let secret = b"MFM_TEST_SECRET_DO_NOT_PERSIST_PLAINTEXT_0123456789abcdef0123456789abcdef";
        let id = store.put_secret_bytes(secret).await.expect("put");

        // Underlying bytes are ciphertext (do not contain plaintext).
        let raw = store.inner.get(&id).await.expect("get raw");
        assert_ne!(raw, secret);
        assert!(!contains_subslice(&raw, secret));

        let got = store.get_secret_bytes(&id).await.expect("get");
        assert_eq!(&got[..], secret);
    }

    #[test]
    fn open_returns_zeroizing_plaintext_buffer() {
        fn assert_zeroizing_vec(_: &Zeroizing<Vec<u8>>) {}

        let dir = tempfile::tempdir().unwrap();
        let inner: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(dir.path()));
        let store = SecretArtifactStore::new(inner, SecretKey::from_bytes(random_key()));
        let secret = b"MFM_TEST_SECRET_ZEROIZING_BUFFER";

        let envelope = store.seal_v1(secret).expect("seal");
        let plaintext = store.open_v1(&envelope).expect("open");

        assert_zeroizing_vec(&plaintext);
        assert_eq!(&plaintext[..], secret);
    }

    #[tokio::test]
    async fn decrypt_fails_with_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let inner: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(dir.path()));

        let store1 =
            SecretArtifactStore::new(Arc::clone(&inner), SecretKey::from_bytes(random_key()));
        let store2 =
            SecretArtifactStore::new(Arc::clone(&inner), SecretKey::from_bytes(random_key()));

        let secret = b"MFM_TEST_SECRET_WRONG_KEY";
        let id = store1.put_secret_bytes(secret).await.expect("put");

        let err = store2
            .get_secret_bytes(&id)
            .await
            .expect_err("decrypt should fail");
        match err {
            StorageError::Other(info) => assert_eq!(info.code.0, CODE_SECRET_DECRYPT_FAILED),
            other => panic!("expected Other, got: {other:?}"),
        }
    }

    #[test]
    fn key_from_hex_rejects_invalid() {
        let err = SecretKey::from_hex("0x01").expect_err("invalid length");
        match err {
            StorageError::Other(info) => assert_eq!(info.code.0, CODE_SECRET_KEY_INVALID),
            other => panic!("expected Other, got: {other:?}"),
        }
    }
}
