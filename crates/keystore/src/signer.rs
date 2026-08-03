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

use k256::elliptic_curve::sec1::ToEncodedPoint;
use mfm_ids::StableId;
use mfm_signing::{
    GenerationGuardedDeterministicSigningProvider, PublicKeyBytes, PublicSigningIdentity,
    QualifiedReadSigningProvider, ReadAttestationQualificationFuture, SignatureBytes, SigningError,
    SigningFuture, SigningGenerationGuard, SigningGenerationGuardError, SigningProviderError,
    SigningRequest, SigningResult, VerifiedGenerationGuardedSignerBinding,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{Keystore, KeystoreConfig, KeystoreError};

const MAX_RUNTIME_PATH_BYTES: usize = 4_096;
const MAX_UNLOCK_FILE_BYTES: usize = 64 * 1_024;

/// Runtime implementation identity for the observational deterministic local-keystore signer.
pub const KEYSTORE_SIGNING_IMPLEMENTATION_ID: &str = "mfm.signing.keystore.rfc6979.v2";

/// Private authority to use the keystore's non-mutating Read-attestation access path.
pub(crate) struct ReadAttestationKeyAccess {
    _private: (),
}

impl ReadAttestationKeyAccess {
    const fn new() -> Self {
        Self { _private: () }
    }

    #[cfg(test)]
    pub(crate) const fn for_test() -> Self {
        Self::new()
    }
}

/// Unqualified one-binding, generation-guarded keystore signing provider.
///
/// Construction validates public evidence and runtime-source shapes only. It
/// does not access the key or establish current deployment authority. Use
/// [`KeystoreSignerProvider::qualify`] to obtain the sole production signer
/// authority. The supplied guard must also certify immutable observational
/// behavior for Read attestation.
pub struct KeystoreSignerProvider {
    binding: VerifiedGenerationGuardedSignerBinding,
    entry_id: Uuid,
    keystore_path: CheckedRuntimePath,
    unlock_file: CheckedRuntimePath,
    keystore_config: KeystoreConfig,
    generation_guard: Arc<dyn SigningGenerationGuard>,
}

impl KeystoreSignerProvider {
    /// Configures one guarded signer candidate using default keystore security settings.
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

    /// Configures one guarded signer candidate using explicit keystore security settings.
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

    /// Consumes this candidate and proves Read eligibility, complete key identity,
    /// and the current deployment guard before minting production signing authority.
    ///
    /// Key-file, unlock, and private-key work runs on a blocking worker. The
    /// guard is checked immediately before that worker is started.
    pub async fn qualify(
        self,
        semantic_signer_id: StableId,
        semantic_signer_contract_ref: mfm_signing::ContentRef,
    ) -> Result<QualifiedKeystoreSigner, SigningError> {
        let expected_binding = self.binding.clone();
        self.qualify_read_attestation(&expected_binding).await?;
        Ok(QualifiedKeystoreSigner {
            provider: self,
            semantic_signer_id,
            semantic_signer_contract_ref,
        })
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

    async fn qualify_read_attestation(
        &self,
        expected_binding: &VerifiedGenerationGuardedSignerBinding,
    ) -> Result<(), SigningError> {
        if !self.is_read_attestation_eligible() {
            return Err(read_attestation_ineligible());
        }
        if &self.binding != expected_binding {
            return Err(SigningError::Provider {
                reason: SigningProviderError::BindingMismatch,
            });
        }
        self.binding.require_complete_public_identity()?;
        self.generation_guard
            .verify_current_and_exclusive(&self.binding)
            .await
            .map_err(signing_error_from_generation_guard)?;
        if !self.is_read_attestation_eligible() {
            return Err(read_attestation_ineligible());
        }

        let inspector = self.blocking_signer();
        let actual_identity = tokio::task::spawn_blocking(move || inspector.inspect_identity())
            .await
            .map_err(SigningError::redacted_provider_failure)?
            .map_err(signing_error_from_provider)?;
        if &actual_identity != self.binding.expected_public_identity() {
            return Err(SigningError::Provider {
                reason: SigningProviderError::BindingMismatch,
            });
        }
        Ok(())
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
    fn inspect_identity(self) -> Result<PublicSigningIdentity, KeystoreSignerError> {
        let (secure_key, address) = self.load_key()?;
        public_identity(&self.binding, &secure_key, address)
    }

    fn sign(
        self,
        digest: Zeroizing<[u8; 32]>,
    ) -> Result<(PublicSigningIdentity, SignatureBytes), KeystoreSignerError> {
        let (secure_key, address) = self.load_key()?;
        let identity = public_identity(&self.binding, &secure_key, address)?;
        let signature = secure_key
            .sign_hash_recoverable(&digest)
            .map_err(|_| KeystoreSignerError::SigningFailed)?;
        let raw_signature = Zeroizing::new(signature.as_bytes());
        let signature = SignatureBytes::new(raw_signature.as_slice().to_vec())?;
        Ok((identity, signature))
    }

    fn load_key(
        &self,
    ) -> Result<(crate::keystore::SecureKey, alloy_primitives::Address), KeystoreSignerError> {
        let keystore_path = self.keystore_path.as_path();
        let access = ReadAttestationKeyAccess::new();
        let unlock_secret = unlock_secret_from_file(&self.unlock_file)?;
        let keystore = Keystore::open_existing_for_read_attestation(
            keystore_path,
            self.keystore_config.clone(),
            unlock_secret.as_str(),
            &access,
        )
        .map_err(keystore_read_attestation_open_error)?;
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
            .private_key_for_read_attestation(self.entry_id, &access)
            .map_err(keystore_key_error)?;
        let address = secure_key
            .ethereum_address()
            .map_err(|_| KeystoreSignerError::SigningFailed)?;
        if format!("{address:?}") != expected_account {
            return Err(KeystoreSignerError::BindingMismatch);
        }
        Ok((secure_key, address))
    }
}

fn public_identity(
    binding: &VerifiedGenerationGuardedSignerBinding,
    secure_key: &crate::keystore::SecureKey,
    address: alloy_primitives::Address,
) -> Result<PublicSigningIdentity, KeystoreSignerError> {
    let public_key = binding
        .expected_public_identity()
        .public_key()
        .map(|_| {
            let public_key = secure_key
                .public_key()
                .map_err(|_| KeystoreSignerError::SigningFailed)?
                .to_encoded_point(true);
            PublicKeyBytes::new(public_key.as_bytes().to_vec())
                .map_err(KeystoreSignerError::SigningContract)
        })
        .transpose()?;
    Ok(PublicSigningIdentity::new(
        binding.algorithm().clone(),
        public_key,
        Some(format!("{address:?}")),
    )?)
}

/// Keystore-owned production authority for one exact guarded signer.
///
/// This bearer is non-cloneable and non-serializable. It can be created only
/// by consuming a [`KeystoreSignerProvider`] and successfully checking the
/// deployment guard plus the actual keystore key, compressed public key, and
/// account. Its consuming handoff repeats those proofs and mints the sole
/// [`QualifiedReadSigningProvider`] accepted by live attestation. Public signing
/// bindings and raw guarded providers cannot construct either authority.
///
/// ```compile_fail
/// use mfm_keystore::QualifiedKeystoreSigner;
///
/// fn require_clone<T: Clone>() {}
/// require_clone::<QualifiedKeystoreSigner>();
/// ```
///
/// ```compile_fail
/// use mfm_keystore::QualifiedKeystoreSigner;
///
/// fn forge() -> QualifiedKeystoreSigner {
///     QualifiedKeystoreSigner {}
/// }
/// ```
pub struct QualifiedKeystoreSigner {
    provider: KeystoreSignerProvider,
    semantic_signer_id: StableId,
    semantic_signer_contract_ref: mfm_signing::ContentRef,
}

impl QualifiedKeystoreSigner {
    /// Returns the deployment-specific semantic signer identity.
    pub const fn semantic_signer_id(&self) -> &StableId {
        &self.semantic_signer_id
    }

    /// Returns the exact secret-free signer binding proven against the key.
    pub const fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        self.provider.binding()
    }

    /// Returns the semantic signer contract selected for deployment.
    pub const fn semantic_signer_contract_ref(&self) -> &mfm_signing::ContentRef {
        &self.semantic_signer_contract_ref
    }

    /// Consumes this production authority for the later affine deployment
    /// bracket, requalifying Read eligibility, key identity, generation, fence,
    /// and direct-path exclusion before releasing the guarded-only provider.
    pub async fn into_read_signing_provider(
        self,
    ) -> Result<
        (
            StableId,
            mfm_signing::ContentRef,
            QualifiedReadSigningProvider,
        ),
        SigningError,
    > {
        let provider: Arc<dyn GenerationGuardedDeterministicSigningProvider> =
            Arc::new(self.provider);
        let provider = QualifiedReadSigningProvider::try_qualify(provider).await?;
        Ok((
            self.semantic_signer_id,
            self.semantic_signer_contract_ref,
            provider,
        ))
    }
}

impl fmt::Debug for QualifiedKeystoreSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QualifiedKeystoreSigner")
            .field("binding", self.binding())
            .field("semantic_signer_id", &self.semantic_signer_id)
            .field(
                "semantic_signer_contract_ref",
                &self.semantic_signer_contract_ref,
            )
            .finish_non_exhaustive()
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
    fn is_read_attestation_eligible(&self) -> bool {
        self.generation_guard.is_read_attestation_eligible()
    }

    fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        &self.binding
    }

    fn verify_read_attestation_qualification<'a>(
        &'a self,
        expected_binding: &'a VerifiedGenerationGuardedSignerBinding,
    ) -> ReadAttestationQualificationFuture<'a> {
        Box::pin(self.qualify_read_attestation(expected_binding))
    }

    fn sign_guarded<'a>(
        &'a self,
        expected_generation_ref: &'a mfm_signing::ContentRef,
        request: &'a SigningRequest,
    ) -> SigningFuture<'a> {
        if !self.is_read_attestation_eligible() {
            return Box::pin(async { Err(read_attestation_ineligible()) });
        }
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
            if !self.is_read_attestation_eligible() {
                return Err(read_attestation_ineligible());
            }
            self.generation_guard
                .verify_current_and_exclusive(&self.binding)
                .await
                .map_err(signing_error_from_generation_guard)?;
            if !self.is_read_attestation_eligible() {
                return Err(read_attestation_ineligible());
            }

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

fn read_attestation_ineligible() -> SigningError {
    SigningError::Provider {
        reason: SigningProviderError::ReadAttestationIneligible,
    }
}

fn keystore_read_attestation_open_error(error: KeystoreError) -> KeystoreSignerError {
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
