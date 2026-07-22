#![warn(missing_docs)]
//! Generic MFM keystore-backed signing provider.
//!
//! One provider instance binds exactly one process-local signer reference to
//! one keystore entry. Protocol callers own signing domain and purpose policy;
//! this provider enforces only the key algorithm, deterministic profile,
//! binding, and expected public identity. File access, unlock/KDF, key access,
//! and signing all run on a blocking worker, and decrypted key wrappers never
//! leave the provider.
//!
//! ```rust
//! use mfm_keystore::KeystoreSignerProvider;
//! use mfm_signing::SignerRef;
//! use uuid::Uuid;
//!
//! let provider = KeystoreSignerProvider::new(
//!     SignerRef::new("deployer")?,
//!     Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8")?,
//!     "/run/mfm/wallet.keystore",
//!     "/run/mfm/wallet.password",
//! )?;
//! assert_eq!(provider.signer_ref().as_str(), "deployer");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SignerRef, SigningError,
    SigningFuture, SigningProvider, SigningRequest, SigningResult,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{Keystore, KeystoreConfig, KeystoreError};

const MAX_RUNTIME_PATH_BYTES: usize = 4_096;
const MAX_UNLOCK_FILE_BYTES: usize = 64 * 1_024;

/// Runtime implementation identity for the deterministic local-keystore signer.
pub const KEYSTORE_SIGNING_IMPLEMENTATION_ID: &str = "mfm.signing.keystore.rfc6979.v1";

/// One-binding MFM keystore signing provider.
#[derive(Clone)]
pub struct KeystoreSignerProvider {
    signer_ref: SignerRef,
    entry_id: Uuid,
    keystore_path: CheckedRuntimePath,
    unlock_file: CheckedRuntimePath,
    keystore_config: KeystoreConfig,
}

impl KeystoreSignerProvider {
    /// Binds one signer using default keystore security settings.
    pub fn new(
        signer_ref: SignerRef,
        entry_id: Uuid,
        keystore_path: impl Into<PathBuf>,
        unlock_file: impl Into<PathBuf>,
    ) -> Result<Self, SigningError> {
        Self::new_with_config(
            signer_ref,
            entry_id,
            keystore_path,
            unlock_file,
            KeystoreConfig::default(),
        )
    }

    /// Binds one signer using explicit keystore security settings.
    pub fn new_with_config(
        signer_ref: SignerRef,
        entry_id: Uuid,
        keystore_path: impl Into<PathBuf>,
        unlock_file: impl Into<PathBuf>,
        keystore_config: KeystoreConfig,
    ) -> Result<Self, SigningError> {
        let keystore_path =
            CheckedRuntimePath::new(keystore_path.into(), RuntimeSourceKind::KeystorePath)
                .map_err(signing_error_from_provider)?;
        let unlock_file =
            CheckedRuntimePath::new(unlock_file.into(), RuntimeSourceKind::UnlockFile)
                .map_err(signing_error_from_provider)?;
        Ok(Self {
            signer_ref,
            entry_id,
            keystore_path,
            unlock_file,
            keystore_config,
        })
    }

    /// Returns the exact signer reference bound by this provider.
    pub const fn signer_ref(&self) -> &SignerRef {
        &self.signer_ref
    }

    fn validate_request(&self, request: &SigningRequest) -> Result<(), KeystoreSignerError> {
        if request.signer_ref() != &self.signer_ref {
            return Err(KeystoreSignerError::UnknownSigner);
        }
        if request.algorithm().as_str() != SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID {
            return Err(KeystoreSignerError::UnsupportedAlgorithm);
        }
        if request.profile().as_str() != SECP256K1_RFC6979_LOW_S_PROFILE_ID {
            return Err(KeystoreSignerError::UnsupportedProfile);
        }
        if request.expected_identity().is_none() {
            return Err(KeystoreSignerError::MissingExpectedIdentity);
        }
        Ok(())
    }

    fn sign_on_blocking_worker(
        &self,
        request: &SigningRequest,
    ) -> Result<SigningResult, KeystoreSignerError> {
        let keystore_path = self.keystore_path.as_path();
        if !fs::metadata(keystore_path)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return Err(KeystoreSignerError::MissingRuntimeSource {
                kind: RuntimeSourceKind::KeystorePath,
            });
        }
        let unlock_secret = unlock_secret_from_file(&self.unlock_file)?;
        let mut keystore = Keystore::new_with_config(keystore_path, self.keystore_config.clone())
            .map_err(keystore_open_error)?;
        keystore
            .unlock(unlock_secret.as_str())
            .map_err(keystore_unlock_error)?;
        drop(unlock_secret);
        let secure_key = keystore
            .get_private_key(self.entry_id)
            .map_err(keystore_key_error)?;
        let address = secure_key
            .ethereum_address()
            .map_err(|_| KeystoreSignerError::SigningFailed)?;
        let signature = secure_key
            .sign_hash_recoverable(request.digest().as_bytes())
            .map_err(|_| KeystoreSignerError::SigningFailed)?;
        let identity = PublicSigningIdentity::new(
            request.algorithm().clone(),
            None,
            Some(format!("{address:?}")),
        )?;
        let signature = SignatureBytes::new(signature.as_bytes().to_vec())?;
        Ok(SigningResult::for_request(request, identity, signature)?)
    }
}

impl fmt::Debug for KeystoreSignerProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeystoreSignerProvider")
            .field("signer_ref", &self.signer_ref)
            .field("entry_id", &self.entry_id)
            .field("keystore_path", &"<redacted>")
            .field("unlock_file", &"<redacted>")
            .field("keystore_config", &"<redacted>")
            .finish()
    }
}

impl SigningProvider for KeystoreSignerProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        if let Err(error) = self.validate_request(request) {
            return Box::pin(async move { Err(signing_error_from_provider(error)) });
        }

        let provider = self.clone();
        let request = request.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || provider.sign_on_blocking_worker(&request))
                .await
                .map_err(SigningError::redacted_provider_failure)?
                .map_err(signing_error_from_provider)
        })
    }
}

impl DeterministicSigningProvider for KeystoreSignerProvider {
    fn implementation_id(&self) -> &'static str {
        KEYSTORE_SIGNING_IMPLEMENTATION_ID
    }

    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

#[derive(Clone)]
struct CheckedRuntimePath(PathBuf);

impl CheckedRuntimePath {
    fn new(path: PathBuf, kind: RuntimeSourceKind) -> Result<Self, KeystoreSignerError> {
        let Some(encoded) = path.to_str() else {
            return Err(KeystoreSignerError::InvalidRuntimeSource { kind });
        };
        if encoded.is_empty() || encoded.len() > MAX_RUNTIME_PATH_BYTES {
            return Err(KeystoreSignerError::InvalidRuntimeSource { kind });
        }
        Ok(Self(path))
    }

    fn as_path(&self) -> &Path {
        &self.0
    }
}

struct ResolvedUnlockSecret {
    secret: Zeroizing<String>,
    #[cfg(test)]
    witness: Option<ZeroizeWitness>,
}

impl ResolvedUnlockSecret {
    fn as_str(&self) -> &str {
        self.secret.as_str()
    }

    #[cfg(test)]
    fn with_drop_witness(mut self, witness: ZeroizeWitness) -> Self {
        self.witness = Some(witness);
        self
    }
}

impl Drop for ResolvedUnlockSecret {
    fn drop(&mut self) {
        self.secret.zeroize();
        #[cfg(test)]
        if let Some(witness) = &self.witness {
            witness.record(self.secret.as_bytes().iter().all(|byte| *byte == 0));
        }
    }
}

struct ProtectedSecretBytes {
    bytes: Zeroizing<Vec<u8>>,
    #[cfg(test)]
    witness: Option<ZeroizeWitness>,
}

impl ProtectedSecretBytes {
    fn new() -> Self {
        Self {
            bytes: Zeroizing::new(Vec::with_capacity(MAX_UNLOCK_FILE_BYTES + 1)),
            #[cfg(test)]
            witness: None,
        }
    }

    #[cfg(test)]
    fn with_witness(witness: ZeroizeWitness) -> Self {
        Self {
            bytes: Zeroizing::new(Vec::with_capacity(MAX_UNLOCK_FILE_BYTES + 1)),
            witness: Some(witness),
        }
    }
}

impl Drop for ProtectedSecretBytes {
    fn drop(&mut self) {
        self.bytes.zeroize();
        #[cfg(test)]
        if let Some(witness) = &self.witness {
            witness.record(self.bytes.iter().all(|byte| *byte == 0));
        }
    }
}

fn unlock_secret_from_file(
    path: &CheckedRuntimePath,
) -> Result<ResolvedUnlockSecret, KeystoreSignerError> {
    let file =
        fs::File::open(path.as_path()).map_err(|_| KeystoreSignerError::MissingRuntimeSource {
            kind: RuntimeSourceKind::UnlockFile,
        })?;
    unlock_secret_from_reader(file)
}

fn unlock_secret_from_reader(
    reader: impl Read,
) -> Result<ResolvedUnlockSecret, KeystoreSignerError> {
    unlock_secret_from_reader_with_storage(reader, ProtectedSecretBytes::new())
}

#[cfg(test)]
fn unlock_secret_from_reader_with_witness(
    reader: impl Read,
    witness: ZeroizeWitness,
) -> Result<ResolvedUnlockSecret, KeystoreSignerError> {
    unlock_secret_from_reader_with_storage(reader, ProtectedSecretBytes::with_witness(witness))
}

fn unlock_secret_from_reader_with_storage(
    reader: impl Read,
    mut contents: ProtectedSecretBytes,
) -> Result<ResolvedUnlockSecret, KeystoreSignerError> {
    reader
        .take((MAX_UNLOCK_FILE_BYTES + 1) as u64)
        .read_to_end(&mut contents.bytes)
        .map_err(|_| KeystoreSignerError::UnreadableRuntimeSource {
            kind: RuntimeSourceKind::UnlockFile,
        })?;
    if contents.bytes.len() > MAX_UNLOCK_FILE_BYTES {
        return Err(KeystoreSignerError::InvalidRuntimeSource {
            kind: RuntimeSourceKind::UnlockFile,
        });
    }
    strip_one_line_ending(&mut contents.bytes);
    if contents.bytes.is_empty() {
        return Err(KeystoreSignerError::InvalidRuntimeSource {
            kind: RuntimeSourceKind::UnlockFile,
        });
    }
    let decoded = std::str::from_utf8(&contents.bytes).map_err(|_| {
        KeystoreSignerError::InvalidRuntimeSource {
            kind: RuntimeSourceKind::UnlockFile,
        }
    })?;
    let mut secret = Zeroizing::new(String::with_capacity(decoded.len()));
    secret.push_str(decoded);
    Ok(ResolvedUnlockSecret {
        secret,
        #[cfg(test)]
        witness: None,
    })
}

fn strip_one_line_ending(contents: &mut Vec<u8>) {
    if contents.ends_with(b"\r\n") {
        contents.truncate(contents.len() - 2);
    } else if contents.ends_with(b"\n") {
        contents.truncate(contents.len() - 1);
    }
}

fn keystore_open_error(_: KeystoreError) -> KeystoreSignerError {
    KeystoreSignerError::KeystoreUnavailable
}

fn keystore_unlock_error(error: KeystoreError) -> KeystoreSignerError {
    match error {
        KeystoreError::InvalidPassword | KeystoreError::Locked => {
            KeystoreSignerError::KeystoreUnlockFailed
        }
        _ => KeystoreSignerError::KeystoreUnavailable,
    }
}

fn keystore_key_error(error: KeystoreError) -> KeystoreSignerError {
    match error {
        KeystoreError::KeyNotFound(_) | KeystoreError::Locked => {
            KeystoreSignerError::KeyUnavailable
        }
        _ => KeystoreSignerError::SigningFailed,
    }
}

fn signing_error_from_provider(error: KeystoreSignerError) -> SigningError {
    match error {
        KeystoreSignerError::SigningContract(error) => error,
        other => SigningError::redacted_provider_failure(other),
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct ZeroizeWitness(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[cfg(test)]
impl ZeroizeWitness {
    fn record(&self, zeroized: bool) {
        self.0.store(zeroized, std::sync::atomic::Ordering::SeqCst);
    }

    fn observed_zeroized_drop(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeSourceKind {
    KeystorePath,
    UnlockFile,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum KeystoreSignerError {
    #[error("keystore signer provider binding did not match request")]
    UnknownSigner,
    #[error("keystore signer provider does not support requested algorithm")]
    UnsupportedAlgorithm,
    #[error("keystore signer provider does not support requested profile")]
    UnsupportedProfile,
    #[error("keystore signer provider requires expected public identity")]
    MissingExpectedIdentity,
    #[error("keystore signer provider runtime source was missing")]
    MissingRuntimeSource { kind: RuntimeSourceKind },
    #[error("keystore signer provider runtime source could not be read")]
    UnreadableRuntimeSource { kind: RuntimeSourceKind },
    #[error("keystore signer provider runtime source was invalid")]
    InvalidRuntimeSource { kind: RuntimeSourceKind },
    #[error("keystore signer provider could not open keystore")]
    KeystoreUnavailable,
    #[error("keystore signer provider could not unlock keystore")]
    KeystoreUnlockFailed,
    #[error("keystore signer provider key entry was unavailable")]
    KeyUnavailable,
    #[error("keystore signer provider failed to sign request")]
    SigningFailed,
    #[error("keystore signer provider result failed signing contract validation")]
    SigningContract(#[from] SigningError),
}

#[cfg(test)]
#[path = "signer_tests.rs"]
mod tests;
