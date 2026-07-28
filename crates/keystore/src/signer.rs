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
//! use mfm_keystore::{KeystoreSignerProvider, KEYSTORE_SIGNING_IMPLEMENTATION_ID};
//!
//! fn verify_provider(provider: &KeystoreSignerProvider) {
//!     assert_eq!(
//!         provider.binding().provider_implementation_id().as_str(),
//!         KEYSTORE_SIGNING_IMPLEMENTATION_ID,
//!     );
//! }
//! ```

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mfm_signing::{
    GenerationGuardedDeterministicSigningProvider, PublicSigningIdentity, SignatureBytes,
    SigningError, SigningFuture, SigningGenerationGuard, SigningGenerationGuardError,
    SigningProviderError, SigningRequest, SigningResult, VerifiedGenerationGuardedSignerBinding,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{Keystore, KeystoreConfig, KeystoreError};

const MAX_RUNTIME_PATH_BYTES: usize = 4_096;
const MAX_UNLOCK_FILE_BYTES: usize = 64 * 1_024;

/// Runtime implementation identity for the deterministic local-keystore signer.
pub const KEYSTORE_SIGNING_IMPLEMENTATION_ID: &str = "mfm.signing.keystore.rfc6979.v1";

/// One-binding, generation-guarded MFM keystore signing provider.
pub struct KeystoreSignerProvider {
    binding: VerifiedGenerationGuardedSignerBinding,
    entry_id: Uuid,
    keystore_path: CheckedRuntimePath,
    unlock_file: CheckedRuntimePath,
    keystore_config: KeystoreConfig,
    generation_guard: Arc<dyn SigningGenerationGuard>,
}

impl KeystoreSignerProvider {
    /// Binds one qualified wallet signer using default keystore security settings.
    pub fn new(
        binding: VerifiedGenerationGuardedSignerBinding,
        entry_id: Uuid,
        keystore_path: impl Into<PathBuf>,
        unlock_file: impl Into<PathBuf>,
        generation_guard: Arc<dyn SigningGenerationGuard>,
    ) -> Result<Self, SigningError> {
        Self::new_with_config(
            binding,
            entry_id,
            keystore_path,
            unlock_file,
            generation_guard,
            KeystoreConfig::default(),
        )
    }

    /// Binds one qualified wallet signer using explicit keystore security settings.
    pub fn new_with_config(
        binding: VerifiedGenerationGuardedSignerBinding,
        entry_id: Uuid,
        keystore_path: impl Into<PathBuf>,
        unlock_file: impl Into<PathBuf>,
        generation_guard: Arc<dyn SigningGenerationGuard>,
        keystore_config: KeystoreConfig,
    ) -> Result<Self, SigningError> {
        validate_provider_binding(&binding)?;
        let keystore_path =
            CheckedRuntimePath::new(keystore_path.into(), RuntimeSourceKind::KeystorePath)
                .map_err(signing_error_from_provider)?;
        let unlock_file =
            CheckedRuntimePath::new(unlock_file.into(), RuntimeSourceKind::UnlockFile)
                .map_err(signing_error_from_provider)?;
        Ok(Self {
            binding,
            entry_id,
            keystore_path,
            unlock_file,
            keystore_config,
            generation_guard,
        })
    }

    /// Returns the exact public wallet/provider binding.
    pub const fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        &self.binding
    }

    fn validate_request(&self, request: &SigningRequest) -> Result<(), KeystoreSignerError> {
        self.binding
            .verify_request(request)
            .map_err(KeystoreSignerError::SigningContract)
    }

    fn blocking_signer(&self) -> BlockingKeystoreSigner {
        BlockingKeystoreSigner {
            binding: self.binding.clone(),
            entry_id: self.entry_id,
            keystore_path: self.keystore_path.clone(),
            unlock_file: self.unlock_file.clone(),
            keystore_config: self.keystore_config.clone(),
        }
    }
}

struct BlockingKeystoreSigner {
    binding: VerifiedGenerationGuardedSignerBinding,
    entry_id: Uuid,
    keystore_path: CheckedRuntimePath,
    unlock_file: CheckedRuntimePath,
    keystore_config: KeystoreConfig,
}

impl BlockingKeystoreSigner {
    fn sign(
        self,
        digest: Zeroizing<[u8; 32]>,
    ) -> Result<(PublicSigningIdentity, SignatureBytes), KeystoreSignerError> {
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
        let expected_account = self
            .binding
            .expected_public_identity()
            .account_id()
            .ok_or(KeystoreSignerError::BindingMismatch)?;
        let key_info = keystore
            .list_keys()
            .map_err(keystore_key_error)?
            .into_iter()
            .find(|key| key.id == self.entry_id)
            .ok_or(KeystoreSignerError::KeyUnavailable)?;
        if format!("{:?}", key_info.address) != expected_account {
            return Err(KeystoreSignerError::BindingMismatch);
        }
        let secure_key = keystore
            .get_private_key(self.entry_id)
            .map_err(keystore_key_error)?;
        let address = secure_key
            .ethereum_address()
            .map_err(|_| KeystoreSignerError::SigningFailed)?;
        if format!("{address:?}") != expected_account {
            return Err(KeystoreSignerError::BindingMismatch);
        }
        let signature = secure_key
            .sign_hash_recoverable(&digest)
            .map_err(|_| KeystoreSignerError::SigningFailed)?;
        let identity = PublicSigningIdentity::new(
            self.binding.algorithm().clone(),
            None,
            Some(format!("{address:?}")),
        )?;
        let raw_signature = Zeroizing::new(signature.as_bytes());
        let signature = SignatureBytes::new(raw_signature.as_slice().to_vec())?;
        Ok((identity, signature))
    }
}

impl fmt::Debug for KeystoreSignerProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeystoreSignerProvider")
            .field("binding", &self.binding)
            .field("entry_id", &self.entry_id)
            .field("keystore_path", &"<redacted>")
            .field("unlock_file", &"<redacted>")
            .field("keystore_config", &"<redacted>")
            .field("generation_guard", &"<redacted>")
            .finish()
    }
}

impl GenerationGuardedDeterministicSigningProvider for KeystoreSignerProvider {
    fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        &self.binding
    }

    fn sign_guarded<'a>(
        &'a self,
        expected_generation_ref: &'a mfm_signing::ContentRef,
        request: &'a SigningRequest,
    ) -> SigningFuture<'a> {
        if expected_generation_ref != self.binding.durable_generation_ref() {
            return Box::pin(async {
                Err(SigningError::Provider {
                    reason: SigningProviderError::GenerationMismatch,
                })
            });
        }
        if let Err(error) = self.validate_request(request) {
            return Box::pin(async move { Err(signing_error_from_provider(error)) });
        }

        let blocking_signer = self.blocking_signer();
        let digest = Zeroizing::new(*request.digest());
        Box::pin(async move {
            self.generation_guard
                .verify_current_and_exclusive(&self.binding)
                .await
                .map_err(signing_error_from_generation_guard)?;

            let (identity, signature) =
                tokio::task::spawn_blocking(move || blocking_signer.sign(digest))
                    .await
                    .map_err(SigningError::redacted_provider_failure)?
                    .map_err(signing_error_from_provider)?;
            SigningResult::for_request(request, identity, signature)
        })
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

fn validate_provider_binding(
    binding: &VerifiedGenerationGuardedSignerBinding,
) -> Result<(), SigningError> {
    if binding.provider_implementation_id().as_str() != KEYSTORE_SIGNING_IMPLEMENTATION_ID
        || binding.algorithm().as_str() != SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID
        || binding.profile().as_str() != SECP256K1_RFC6979_LOW_S_PROFILE_ID
    {
        return Err(SigningError::Provider {
            reason: SigningProviderError::BindingMismatch,
        });
    }
    Ok(())
}

fn signing_error_from_generation_guard(error: SigningGenerationGuardError) -> SigningError {
    let reason = match error {
        SigningGenerationGuardError::Unavailable => {
            SigningProviderError::GenerationGuardUnavailable
        }
        SigningGenerationGuardError::Fenced => SigningProviderError::GenerationFenced,
        SigningGenerationGuardError::DirectSigningOverlap => {
            SigningProviderError::DirectSigningOverlap
        }
    };
    SigningError::Provider { reason }
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
        KeystoreSignerError::BindingMismatch => SigningError::Provider {
            reason: SigningProviderError::BindingMismatch,
        },
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
    #[error("keystore signer provider binding mismatch")]
    BindingMismatch,
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
