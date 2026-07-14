#![warn(missing_docs)]
//! MFM keystore-backed signer provider.
//!
//! This crate resolves process-local keystore runtime bindings, unlocks an MFM
//! keystore for a single signing request, and returns only signatures plus
//! public account metadata. Decrypted key wrappers never leave this provider.
//!
//! ```rust
//! use mfm_signers_keystore::{KeystoreSignerProvider, KeystoreSignerRegistryEntry};
//! use mfm_signing::SignerRef;
//! use uuid::Uuid;
//!
//! let entry = KeystoreSignerRegistryEntry::new(
//!     SignerRef::new("deployer")?,
//!     Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8")?,
//!     "/run/mfm/wallet.keystore",
//!     "/run/mfm/wallet.password",
//! );
//! let provider = KeystoreSignerProvider::new([entry]);
//! assert!(provider.contains_signer(&SignerRef::new("deployer")?));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use mfm_core::keystore::{Keystore, KeystoreConfig, KeystoreError};
use mfm_evm_signing::{
    EVM_EIP1559_TRANSACTION_PURPOSE_ID, EVM_LEGACY_TRANSACTION_PURPOSE_ID,
    EVM_SIGNING_ALGORITHM_ID, EVM_TRANSACTION_DOMAIN_ID,
};
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SignerRef, SigningError, SigningFuture, SigningProvider,
    SigningRequest, SigningResult, MANUAL_RESOLUTION_SIGNING_DOMAIN_ID,
    MANUAL_RESOLUTION_SIGNING_PURPOSE_ID,
};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Result type for keystore signer provider operations.
pub type Result<T> = std::result::Result<T, KeystoreSignerError>;

/// Runtime signer registry entry mapping a signer reference to a keystore entry.
#[derive(Clone, PartialEq, Eq)]
pub struct KeystoreSignerRegistryEntry {
    signer_ref: SignerRef,
    entry_id: Uuid,
    keystore_path: PathBuf,
    password_file: PathBuf,
}

impl KeystoreSignerRegistryEntry {
    /// Creates a registry entry for one signer reference and keystore entry id.
    pub fn new(
        signer_ref: SignerRef,
        entry_id: Uuid,
        keystore_path: impl Into<PathBuf>,
        password_file: impl Into<PathBuf>,
    ) -> Self {
        Self {
            signer_ref,
            entry_id,
            keystore_path: keystore_path.into(),
            password_file: password_file.into(),
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

    /// Returns the keystore path.
    pub fn keystore_path(&self) -> &Path {
        &self.keystore_path
    }

    /// Returns the password file path.
    pub fn password_file(&self) -> &Path {
        &self.password_file
    }
}

impl fmt::Debug for KeystoreSignerRegistryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeystoreSignerRegistryEntry")
            .field("signer_ref", &self.signer_ref)
            .field("entry_id", &self.entry_id)
            .field("keystore_path", &"<redacted>")
            .field("password_file", &"<redacted>")
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
        if !matches!(
            request.domain().as_str(),
            EVM_TRANSACTION_DOMAIN_ID | MANUAL_RESOLUTION_SIGNING_DOMAIN_ID
        ) {
            return Err(KeystoreSignerError::UnsupportedDomain);
        }
        let supported_purpose = match request.domain().as_str() {
            EVM_TRANSACTION_DOMAIN_ID => matches!(
                request.purpose().as_str(),
                EVM_LEGACY_TRANSACTION_PURPOSE_ID | EVM_EIP1559_TRANSACTION_PURPOSE_ID
            ),
            MANUAL_RESOLUTION_SIGNING_DOMAIN_ID => {
                request.purpose().as_str() == MANUAL_RESOLUTION_SIGNING_PURPOSE_ID
            }
            _ => false,
        };
        if !supported_purpose {
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

        let keystore_path =
            checked_runtime_path(&entry.keystore_path, RuntimeSourceKind::KeystorePath)?;
        let password = password_from_file(&entry.password_file)?;
        let mut keystore = Keystore::new_with_config(keystore_path, self.keystore_config.clone())
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
        )?;
        let signature = SignatureBytes::new(signature.as_bytes().to_vec())?;
        Ok(SigningResult::for_request(request, identity, signature)?)
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

struct ResolvedPassword(Zeroizing<String>);

impl ResolvedPassword {
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

fn checked_runtime_path(path: &Path, kind: RuntimeSourceKind) -> Result<&Path> {
    if path.as_os_str().is_empty() {
        return Err(KeystoreSignerError::InvalidRuntimeSource { kind });
    }
    Ok(path)
}

fn password_from_file(path: impl AsRef<Path>) -> Result<ResolvedPassword> {
    let path = checked_runtime_path(path.as_ref(), RuntimeSourceKind::PasswordFile)?;
    let contents = Zeroizing::new(fs::read_to_string(path).map_err(|_| {
        KeystoreSignerError::MissingRuntimeSource {
            kind: RuntimeSourceKind::PasswordFile,
        }
    })?);
    let trimmed = contents.trim_end_matches(['\r', '\n']).to_owned();
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
    SigningContract(#[from] SigningError),
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
