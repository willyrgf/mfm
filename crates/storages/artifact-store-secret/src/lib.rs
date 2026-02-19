//! Encrypted secret-bearing artifacts.
//!
//! This crate provides a wrapper for storing secret payloads as encrypted artifacts.
//! Plaintext secrets MUST NOT be stored directly in the underlying `ArtifactStore`.

use std::sync::Arc;

use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::ids::{ArtifactId, ErrorCode};
use mfm_machine::stores::{ArtifactKind, ArtifactStore};
use ring::aead;
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::{Zeroize, Zeroizing};

const CODE_SECRET_KEY_MISSING: &str = "secret_key_missing";
const CODE_SECRET_KEY_INVALID: &str = "secret_key_invalid";
const CODE_SECRET_ENCRYPT_FAILED: &str = "secret_encrypt_failed";
const CODE_SECRET_CIPHERTEXT_INVALID: &str = "secret_ciphertext_invalid";
const CODE_SECRET_DECRYPT_FAILED: &str = "secret_decrypt_failed";

const ENV_SECRET_KEY_HEX: &str = "MFM_SECRET_KEY_HEX";

// Envelope format:
// - version: u8
// - nonce: [u8; 12]
// - ciphertext + tag: bytes
const ENVELOPE_VERSION_V1: u8 = 1;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 1 + NONCE_LEN;

// AAD is constant and binds ciphertext to this specific use.
const AAD_V1: &[u8] = b"mfm:secret_payload:v1";

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
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

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

    pub fn from_env() -> Result<Self, StorageError> {
        let v = std::env::var(ENV_SECRET_KEY_HEX).map_err(|_| {
            other(
                CODE_SECRET_KEY_MISSING,
                ErrorCategory::ParsingInput,
                "missing MFM_SECRET_KEY_HEX",
            )
        })?;
        Self::from_hex(&v)
    }
}

#[derive(Clone)]
pub struct SecretArtifactStore {
    inner: Arc<dyn ArtifactStore>,
    key: SecretKey,
    rng: SystemRandom,
}

impl SecretArtifactStore {
    pub fn new(inner: Arc<dyn ArtifactStore>, key: SecretKey) -> Self {
        Self {
            inner,
            key,
            rng: SystemRandom::new(),
        }
    }

    pub fn from_env(inner: Arc<dyn ArtifactStore>) -> Result<Self, StorageError> {
        let key = SecretKey::from_env()?;
        Ok(Self::new(inner, key))
    }

    pub async fn put_secret_bytes(&self, plaintext: &[u8]) -> Result<ArtifactId, StorageError> {
        let ciphertext = self.seal_v1(plaintext)?;
        self.inner
            .put(ArtifactKind::SecretPayload, ciphertext)
            .await
    }

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
        let mut in_out: Vec<u8> = plaintext.to_vec();
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::from(AAD_V1),
            &mut in_out,
        )
        .map_err(|_| {
            other(
                CODE_SECRET_ENCRYPT_FAILED,
                ErrorCategory::Unknown,
                "secret encryption failed",
            )
        })?;

        let mut out = Vec::with_capacity(HEADER_LEN + in_out.len());
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
        if envelope[0] != ENVELOPE_VERSION_V1 {
            return Err(other(
                CODE_SECRET_CIPHERTEXT_INVALID,
                ErrorCategory::ParsingInput,
                "unsupported secret envelope version",
            ));
        }

        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&envelope[1..HEADER_LEN]);

        let unbound =
            aead::UnboundKey::new(&aead::AES_256_GCM, &self.key.bytes[..]).map_err(|_| {
                other(
                    CODE_SECRET_KEY_INVALID,
                    ErrorCategory::ParsingInput,
                    "invalid secret key",
                )
            })?;
        let key = aead::LessSafeKey::new(unbound);

        let mut in_out: Vec<u8> = envelope[HEADER_LEN..].to_vec();
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

        let out = Zeroizing::new(pt.to_vec());
        in_out.zeroize();
        Ok(out)
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
