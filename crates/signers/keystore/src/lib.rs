#![warn(missing_docs)]
//! MFM keystore-backed signer provider.
//!
//! This crate resolves process-local keystore runtime bindings, unlocks an MFM
//! keystore for a single signing request, and returns only signatures plus
//! public account metadata. Decrypted key wrappers never leave this provider.
//!
//! ```rust
//! use mfm_signers_keystore::{
//!     KeystorePasswordSource, KeystorePathSource, KeystoreSignerProvider,
//!     KeystoreSignerRegistryEntry,
//! };
//! use mfm_signing::SignerRef;
//! use uuid::Uuid;
//!
//! let entry = KeystoreSignerRegistryEntry::new(
//!     SignerRef::new("deployer")?,
//!     Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8")?,
//!     KeystorePathSource::path("/run/mfm/wallet.keystore"),
//!     KeystorePasswordSource::file("/run/mfm/wallet.password"),
//! );
//! let provider = KeystoreSignerProvider::new([entry]);
//! assert!(provider.contains_signer(&SignerRef::new("deployer")?));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use mfm_core::keystore::{Keystore, KeystoreConfig, KeystoreError};
use mfm_evm_signing::{
    EVM_EIP1559_TRANSACTION_PURPOSE_ID, EVM_LEGACY_TRANSACTION_PURPOSE_ID,
    EVM_SIGNING_ALGORITHM_ID, EVM_TRANSACTION_DOMAIN_ID,
};
use mfm_signing::{
    PublicSigningIdentity, RuntimeSecretSource, SignatureBytes, SignerRef, SigningError,
    SigningFuture, SigningProvider, SigningRequest, SigningResult,
};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Result type for keystore signer provider operations.
pub type Result<T> = std::result::Result<T, KeystoreSignerError>;

/// Process-local keystore path source.
#[derive(Clone, PartialEq, Eq)]
pub struct KeystorePathSource {
    kind: KeystorePathSourceKind,
}

#[derive(Clone, PartialEq, Eq)]
enum KeystorePathSourceKind {
    Path(PathBuf),
    EnvPath(RuntimeSecretSource),
}

impl KeystorePathSource {
    /// Creates a direct keystore path source.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: KeystorePathSourceKind::Path(path.into()),
        }
    }

    /// Creates a source that reads the keystore path from an environment variable.
    pub fn path_env_var(name: impl AsRef<str>) -> std::result::Result<Self, SigningError> {
        Ok(Self {
            kind: KeystorePathSourceKind::EnvPath(RuntimeSecretSource::path_env_var(name)?),
        })
    }
}

impl fmt::Debug for KeystorePathSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            KeystorePathSourceKind::Path(_) => f.write_str("KeystorePathSource::Path(<redacted>)"),
            KeystorePathSourceKind::EnvPath(source) => f
                .debug_tuple("KeystorePathSource::EnvPath")
                .field(source)
                .finish(),
        }
    }
}

/// Process-local keystore password source.
#[derive(Clone, PartialEq, Eq)]
pub struct KeystorePasswordSource {
    kind: KeystorePasswordSourceKind,
}

#[derive(Clone, PartialEq, Eq)]
enum KeystorePasswordSourceKind {
    EnvVar(RuntimeSecretSource),
    File(PathBuf),
    FilePathEnv(RuntimeSecretSource),
}

impl KeystorePasswordSource {
    /// Creates a password source backed by an environment variable value.
    pub fn env_var(name: impl AsRef<str>) -> std::result::Result<Self, SigningError> {
        Ok(Self {
            kind: KeystorePasswordSourceKind::EnvVar(RuntimeSecretSource::env_var(name)?),
        })
    }

    /// Creates a password source backed by a direct file path.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: KeystorePasswordSourceKind::File(path.into()),
        }
    }

    /// Creates a password source backed by an environment variable containing a file path.
    pub fn file_path_env_var(name: impl AsRef<str>) -> std::result::Result<Self, SigningError> {
        Ok(Self {
            kind: KeystorePasswordSourceKind::FilePathEnv(RuntimeSecretSource::path_env_var(name)?),
        })
    }
}

impl fmt::Debug for KeystorePasswordSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            KeystorePasswordSourceKind::EnvVar(source) => f
                .debug_tuple("KeystorePasswordSource::EnvVar")
                .field(source)
                .finish(),
            KeystorePasswordSourceKind::File(_) => {
                f.write_str("KeystorePasswordSource::File(<redacted>)")
            }
            KeystorePasswordSourceKind::FilePathEnv(source) => f
                .debug_tuple("KeystorePasswordSource::FilePathEnv")
                .field(source)
                .finish(),
        }
    }
}

/// Runtime signer registry entry mapping a signer reference to a keystore entry.
#[derive(Clone, PartialEq, Eq)]
pub struct KeystoreSignerRegistryEntry {
    signer_ref: SignerRef,
    entry_id: Uuid,
    keystore_source: KeystorePathSource,
    password_source: KeystorePasswordSource,
}

impl KeystoreSignerRegistryEntry {
    /// Creates a registry entry for one signer reference and keystore entry id.
    pub fn new(
        signer_ref: SignerRef,
        entry_id: Uuid,
        keystore_source: KeystorePathSource,
        password_source: KeystorePasswordSource,
    ) -> Self {
        Self {
            signer_ref,
            entry_id,
            keystore_source,
            password_source,
        }
    }

    /// Returns the process-local signer reference.
    pub fn signer_ref(&self) -> &SignerRef {
        &self.signer_ref
    }

    /// Returns the keystore entry id to sign with.
    pub const fn entry_id(&self) -> Uuid {
        self.entry_id
    }

    /// Returns the keystore path source.
    pub fn keystore_source(&self) -> &KeystorePathSource {
        &self.keystore_source
    }

    /// Returns the password source.
    pub fn password_source(&self) -> &KeystorePasswordSource {
        &self.password_source
    }
}

impl fmt::Debug for KeystoreSignerRegistryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeystoreSignerRegistryEntry")
            .field("signer_ref", &self.signer_ref)
            .field("entry_id", &self.entry_id)
            .field("keystore_source", &self.keystore_source)
            .field("password_source", &self.password_source)
            .finish()
    }
}

/// MFM keystore-backed signing provider.
pub struct KeystoreSignerProvider {
    entries: BTreeMap<SignerRef, KeystoreSignerRegistryEntry>,
    keystore_config: KeystoreConfig,
}

impl KeystoreSignerProvider {
    /// Creates a provider using default keystore security settings.
    pub fn new(entries: impl IntoIterator<Item = KeystoreSignerRegistryEntry>) -> Self {
        Self::new_with_config(entries, KeystoreConfig::default())
    }

    /// Creates a provider using explicit keystore settings.
    pub fn new_with_config(
        entries: impl IntoIterator<Item = KeystoreSignerRegistryEntry>,
        keystore_config: KeystoreConfig,
    ) -> Self {
        let entries = entries
            .into_iter()
            .map(|entry| (entry.signer_ref.clone(), entry))
            .collect();
        Self {
            entries,
            keystore_config,
        }
    }

    /// Returns true when a signer reference has a runtime binding.
    pub fn contains_signer(&self, signer_ref: &SignerRef) -> bool {
        self.entries.contains_key(signer_ref)
    }

    /// Signs a request and returns signature bytes plus public account metadata.
    pub fn sign_request(&self, request: &SigningRequest) -> Result<SigningResult> {
        if request.algorithm().as_str() != EVM_SIGNING_ALGORITHM_ID {
            return Err(KeystoreSignerError::UnsupportedAlgorithm);
        }
        if request.domain().as_str() != EVM_TRANSACTION_DOMAIN_ID {
            return Err(KeystoreSignerError::UnsupportedDomain);
        }
        if !matches!(
            request.purpose().as_str(),
            EVM_LEGACY_TRANSACTION_PURPOSE_ID | EVM_EIP1559_TRANSACTION_PURPOSE_ID
        ) {
            return Err(KeystoreSignerError::UnsupportedPurpose);
        }
        if request.expected_identity().is_none() {
            return Err(KeystoreSignerError::MissingExpectedIdentity);
        }

        let entry = self.entries.get(request.signer_ref()).ok_or_else(|| {
            KeystoreSignerError::UnknownSigner {
                signer_ref: request.signer_ref().clone(),
            }
        })?;

        let keystore_path = entry.keystore_source.resolve_path()?;
        let password = entry.password_source.resolve_password()?;
        let mut keystore = Keystore::new_with_config(&keystore_path, self.keystore_config.clone())
            .map_err(keystore_open_error)?;
        keystore
            .unlock(password.as_str())
            .map_err(keystore_unlock_error)?;
        let secure_key = keystore
            .get_private_key(entry.entry_id)
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
        )
        .map_err(KeystoreSignerError::SigningContract)?;
        let signature = SignatureBytes::new(signature.as_bytes().to_vec())
            .map_err(KeystoreSignerError::SigningContract)?;
        SigningResult::for_request(request, identity, signature)
            .map_err(KeystoreSignerError::SigningContract)
    }
}

impl fmt::Debug for KeystoreSignerProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeystoreSignerProvider")
            .field("entries", &self.entries)
            .field("keystore_config", &"<redacted>")
            .finish()
    }
}

impl SigningProvider for KeystoreSignerProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        let result = self
            .sign_request(request)
            .map_err(keystore_provider_error_into_signing_error);
        Box::pin(async move { result })
    }
}

impl KeystorePathSource {
    fn resolve_path(&self) -> Result<PathBuf> {
        match &self.kind {
            KeystorePathSourceKind::Path(path) => {
                if path.as_os_str().is_empty() {
                    Err(KeystoreSignerError::InvalidRuntimeSource {
                        kind: RuntimeSourceKind::KeystorePath,
                    })
                } else {
                    Ok(path.clone())
                }
            }
            KeystorePathSourceKind::EnvPath(source) => {
                path_from_env(source, RuntimeSourceKind::KeystorePath)
            }
        }
    }
}

impl KeystorePasswordSource {
    fn resolve_password(&self) -> Result<ResolvedPassword> {
        match &self.kind {
            KeystorePasswordSourceKind::EnvVar(source) => {
                let value = env::var(source.name()).map_err(|error| match error {
                    env::VarError::NotPresent => KeystoreSignerError::MissingRuntimeSource {
                        kind: RuntimeSourceKind::PasswordEnvVar,
                    },
                    env::VarError::NotUnicode(_) => KeystoreSignerError::InvalidRuntimeSource {
                        kind: RuntimeSourceKind::PasswordEnvVar,
                    },
                })?;
                if value.is_empty() {
                    return Err(KeystoreSignerError::InvalidRuntimeSource {
                        kind: RuntimeSourceKind::PasswordEnvVar,
                    });
                }
                Ok(ResolvedPassword(Zeroizing::new(value)))
            }
            KeystorePasswordSourceKind::File(path) => password_from_file(path),
            KeystorePasswordSourceKind::FilePathEnv(source) => {
                let path = path_from_env(source, RuntimeSourceKind::PasswordFile)?;
                password_from_file(path)
            }
        }
    }
}

struct ResolvedPassword(Zeroizing<String>);

impl ResolvedPassword {
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

fn path_from_env(source: &RuntimeSecretSource, kind: RuntimeSourceKind) -> Result<PathBuf> {
    let value =
        env::var_os(source.name()).ok_or(KeystoreSignerError::MissingRuntimeSource { kind })?;
    if value.is_empty() {
        return Err(KeystoreSignerError::InvalidRuntimeSource { kind });
    }
    Ok(PathBuf::from(value))
}

fn password_from_file(path: impl AsRef<Path>) -> Result<ResolvedPassword> {
    let contents = Zeroizing::new(fs::read_to_string(path.as_ref()).map_err(|_| {
        KeystoreSignerError::MissingRuntimeSource {
            kind: RuntimeSourceKind::PasswordFile,
        }
    })?);
    let trimmed = contents
        .trim_end_matches(|ch| ch == '\r' || ch == '\n')
        .to_owned();
    if trimmed.is_empty() {
        return Err(KeystoreSignerError::InvalidRuntimeSource {
            kind: RuntimeSourceKind::PasswordFile,
        });
    }
    Ok(ResolvedPassword(Zeroizing::new(trimmed)))
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

fn keystore_provider_error_into_signing_error(error: KeystoreSignerError) -> SigningError {
    match error {
        KeystoreSignerError::SigningContract(error) => error,
        other => SigningError::redacted_provider_failure(other),
    }
}

/// Closed runtime source category used in redaction-safe errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSourceKind {
    /// Keystore path source.
    KeystorePath,
    /// Password environment variable source.
    PasswordEnvVar,
    /// Password file path source.
    PasswordFile,
}

/// Redaction-safe keystore signer provider error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeystoreSignerError {
    /// No registry entry exists for the requested signer.
    #[error("keystore signer provider has no binding for signer {signer_ref}")]
    UnknownSigner {
        /// Requested signer reference.
        signer_ref: SignerRef,
    },
    /// The request used a signing algorithm this provider does not support.
    #[error("keystore signer provider does not support the requested algorithm")]
    UnsupportedAlgorithm,
    /// The request used a signing domain this provider does not support.
    #[error("keystore signer provider does not support the requested domain")]
    UnsupportedDomain,
    /// The request used a signing purpose this provider does not support.
    #[error("keystore signer provider does not support the requested purpose")]
    UnsupportedPurpose,
    /// The request did not bind the expected public identity.
    #[error("keystore signer provider requires expected public identity")]
    MissingExpectedIdentity,
    /// A required process-local runtime source was missing.
    #[error("keystore signer provider runtime source was missing")]
    MissingRuntimeSource {
        /// Missing runtime source category.
        kind: RuntimeSourceKind,
    },
    /// A process-local runtime source was malformed.
    #[error("keystore signer provider runtime source was invalid")]
    InvalidRuntimeSource {
        /// Invalid runtime source category.
        kind: RuntimeSourceKind,
    },
    /// The keystore could not be opened.
    #[error("keystore signer provider could not open keystore")]
    KeystoreUnavailable,
    /// The keystore could not be unlocked.
    #[error("keystore signer provider could not unlock keystore")]
    KeystoreUnlockFailed,
    /// The configured keystore entry was unavailable.
    #[error("keystore signer provider key entry was unavailable")]
    KeyUnavailable,
    /// The provider could not produce a signature.
    #[error("keystore signer provider failed to sign request")]
    SigningFailed,
    /// Generic signing contract validation failed.
    #[error("keystore signer provider result failed signing contract validation")]
    SigningContract(#[source] SigningError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    use alloy_primitives::{Address, B256};
    use mfm_core::keystore::Keystore;
    use mfm_evm_signing::{
        evm_signing_algorithm_id, evm_transaction_domain_id, legacy_transaction_purpose_id,
        primitive_signature_from_bytes, recover_signing_address,
    };
    use mfm_signing::{ExpectedSignerIdentity, SigningDomainId, SigningPurposeId, SigningRequest};
    use tempfile::TempDir;

    const TEST_KEY: &str = "0x0000000000000000000000000000000000000000000000000000000000000001";
    const PASSWORD: &str = "strong_password_123";

    struct TestKeystore {
        _dir: TempDir,
        keystore_path: PathBuf,
        password_file: PathBuf,
        entry_id: Uuid,
        address: Address,
    }

    fn signer_ref() -> SignerRef {
        SignerRef::new("deployer").expect("signer ref")
    }

    fn test_keystore() -> TestKeystore {
        let dir = tempfile::tempdir().expect("tempdir");
        let keystore_path = dir.path().join("wallet.keystore");
        let password_file = dir.path().join("wallet.password");
        fs::write(&password_file, format!("{PASSWORD}\n")).expect("password file");

        let mut keystore =
            Keystore::new_with_config(&keystore_path, KeystoreConfig::insecure_integration_test())
                .expect("keystore");
        keystore.unlock(PASSWORD).expect("unlock");
        let entry_id = keystore
            .import_private_key(Some("deployer".to_owned()), TEST_KEY)
            .expect("import key");
        let secure_key = keystore.get_private_key(entry_id).expect("secure key");
        let address = secure_key.ethereum_address().expect("address");

        TestKeystore {
            _dir: dir,
            keystore_path,
            password_file,
            entry_id,
            address,
        }
    }

    fn registry_entry(keystore: &TestKeystore) -> KeystoreSignerRegistryEntry {
        KeystoreSignerRegistryEntry::new(
            signer_ref(),
            keystore.entry_id,
            KeystorePathSource::path(&keystore.keystore_path),
            KeystorePasswordSource::file(&keystore.password_file),
        )
    }

    fn provider(entry: KeystoreSignerRegistryEntry) -> KeystoreSignerProvider {
        KeystoreSignerProvider::new_with_config(
            [entry],
            KeystoreConfig::insecure_integration_test(),
        )
    }

    fn signing_request(address: Address) -> SigningRequest {
        signing_request_with(
            address,
            evm_transaction_domain_id().expect("domain"),
            legacy_transaction_purpose_id().expect("purpose"),
            true,
        )
    }

    fn signing_request_with(
        address: Address,
        domain: SigningDomainId,
        purpose: SigningPurposeId,
        require_expected_identity: bool,
    ) -> SigningRequest {
        let request = SigningRequest::from_digest(
            signer_ref(),
            evm_signing_algorithm_id().expect("algorithm"),
            domain,
            purpose,
            mfm_ids::DigestBytes::from_array([0x42; 32]),
        );
        if require_expected_identity {
            request.require_public_identity(
                ExpectedSignerIdentity::account_id(format!("{address:?}")).expect("identity"),
            )
        } else {
            request
        }
    }

    #[test]
    fn provider_signs_through_generic_signing_request_without_returning_private_key() {
        let keystore = test_keystore();
        let provider = provider(registry_entry(&keystore));
        let request = signing_request(keystore.address);

        let result = poll_ready(SigningProvider::sign(&provider, &request)).expect("sign");

        assert_eq!(result.signer_ref(), request.signer_ref());
        let account_id = format!("{:?}", keystore.address);
        assert_eq!(
            result.public_identity().account_id(),
            Some(account_id.as_str())
        );
        assert_eq!(result.signature().as_bytes().len(), 65);
        let signature = primitive_signature_from_bytes(result.signature()).expect("signature");
        let recovered =
            recover_signing_address(B256::from(*request.digest().as_bytes()), signature)
                .expect("recover");
        assert_eq!(recovered, keystore.address);
    }

    #[test]
    fn password_file_trims_line_endings_and_returns_zeroizing_password() {
        let keystore = test_keystore();
        let password_source = KeystorePasswordSource::file(&keystore.password_file);

        let resolved = password_source.resolve_password().expect("password");
        assert_eq!(resolved.as_str(), PASSWORD);
        assert_zeroizing_string(&resolved.0);

        let provider = provider(KeystoreSignerRegistryEntry::new(
            signer_ref(),
            keystore.entry_id,
            KeystorePathSource::path(&keystore.keystore_path),
            password_source,
        ));
        provider
            .sign_request(&signing_request(keystore.address))
            .expect("sign");
    }

    #[test]
    fn env_path_source_resolves_keystore_path_process_locally() {
        let keystore = test_keystore();
        let env_name = format!(
            "MFM_TEST_KEYSTORE_PATH_{}",
            Uuid::new_v4().simple().to_string().to_uppercase()
        );
        env::set_var(&env_name, &keystore.keystore_path);

        let entry = KeystoreSignerRegistryEntry::new(
            signer_ref(),
            keystore.entry_id,
            KeystorePathSource::path_env_var(&env_name).expect("path env"),
            KeystorePasswordSource::file(&keystore.password_file),
        );
        let provider = provider(entry);
        provider
            .sign_request(&signing_request(keystore.address))
            .expect("sign");

        env::remove_var(env_name);
    }

    #[test]
    fn rejects_unsupported_evm_domain() {
        let keystore = test_keystore();
        let provider = provider(registry_entry(&keystore));
        let request = signing_request_with(
            keystore.address,
            SigningDomainId::new("evm.other").expect("domain"),
            legacy_transaction_purpose_id().expect("purpose"),
            true,
        );

        assert_eq!(
            provider.sign_request(&request),
            Err(KeystoreSignerError::UnsupportedDomain)
        );
    }

    #[test]
    fn rejects_unsupported_evm_purpose() {
        let keystore = test_keystore();
        let provider = provider(registry_entry(&keystore));
        let request = signing_request_with(
            keystore.address,
            evm_transaction_domain_id().expect("domain"),
            SigningPurposeId::new("evm.transaction.other").expect("purpose"),
            true,
        );

        assert_eq!(
            provider.sign_request(&request),
            Err(KeystoreSignerError::UnsupportedPurpose)
        );
    }

    #[test]
    fn rejects_missing_expected_identity() {
        let keystore = test_keystore();
        let provider = provider(registry_entry(&keystore));
        let request = signing_request_with(
            keystore.address,
            evm_transaction_domain_id().expect("domain"),
            legacy_transaction_purpose_id().expect("purpose"),
            false,
        );

        assert_eq!(
            provider.sign_request(&request),
            Err(KeystoreSignerError::MissingExpectedIdentity)
        );
    }

    #[test]
    fn redacted_errors_do_not_leak_paths_passwords_or_key_material() {
        let dir = tempfile::tempdir().expect("tempdir");
        let secret_path = dir.path().join("secret-wallet-file-name.keystore");
        let password_file = dir.path().join("secret-password-file-name.txt");
        fs::write(&password_file, "very_secret_password\n").expect("password file");

        let entry = KeystoreSignerRegistryEntry::new(
            signer_ref(),
            Uuid::new_v4(),
            KeystorePathSource::path(&secret_path),
            KeystorePasswordSource::file(&password_file),
        );
        let provider = provider(entry);
        let error = provider
            .sign_request(&signing_request(Address::from([0x11; 20])))
            .expect_err("missing keystore");
        let rendered = format!("{error:?} {error}");
        let secret_path = secret_path.to_string_lossy();
        let password_file = password_file.to_string_lossy();

        assert!(!rendered.contains(secret_path.as_ref()));
        assert!(!rendered.contains(password_file.as_ref()));
        assert!(!rendered.contains("very_secret_password"));
        assert!(!rendered.contains(TEST_KEY.trim_start_matches("0x")));
    }

    #[test]
    fn tampered_keystore_failure_stays_redacted() {
        let keystore = test_keystore();
        fs::write(&keystore.keystore_path, b"{\"tampered\":true}").expect("tamper");
        let provider = provider(registry_entry(&keystore));

        let error = provider
            .sign_request(&signing_request(keystore.address))
            .expect_err("tampered keystore");

        assert_eq!(error, KeystoreSignerError::KeystoreUnavailable);
        assert!(!format!("{error:?} {error}").contains("tampered"));
    }

    fn assert_zeroizing_string(_: &Zeroizing<String>) {}

    fn poll_ready<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("provider future should be ready"),
        }
    }
}
