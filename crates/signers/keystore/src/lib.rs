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
//! use mfm_signers_keystore::KeystoreSignerProvider;
//! use mfm_signing::SignerRef;
//! use uuid::Uuid;
//!
//! let provider = KeystoreSignerProvider::new(
//!     SignerRef::new("deployer")?,
//!     Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8")?,
//!     "/run/mfm/wallet.keystore",
//!     "/run/mfm/wallet.password",
//! );
//! assert_eq!(provider.signer_ref().as_str(), "deployer");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use mfm_core::keystore::{Keystore, KeystoreConfig, KeystoreError};
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SignerRef, SigningError, SigningFuture, SigningProvider,
    SigningRequest, SigningResult, SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID,
    SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use uuid::Uuid;
use zeroize::Zeroizing;

/// One-binding MFM keystore signing provider.
#[derive(Clone)]
pub struct KeystoreSignerProvider {
    signer_ref: SignerRef,
    entry_id: Uuid,
    keystore_path: PathBuf,
    password_file: PathBuf,
    keystore_config: KeystoreConfig,
}

impl KeystoreSignerProvider {
    /// Binds one signer using default keystore security settings.
    pub fn new(
        signer_ref: SignerRef,
        entry_id: Uuid,
        keystore_path: impl Into<PathBuf>,
        password_file: impl Into<PathBuf>,
    ) -> Self {
        Self::new_with_config(
            signer_ref,
            entry_id,
            keystore_path,
            password_file,
            KeystoreConfig::default(),
        )
    }

    /// Binds one signer using explicit keystore security settings.
    pub fn new_with_config(
        signer_ref: SignerRef,
        entry_id: Uuid,
        keystore_path: impl Into<PathBuf>,
        password_file: impl Into<PathBuf>,
        keystore_config: KeystoreConfig,
    ) -> Self {
        Self {
            signer_ref,
            entry_id,
            keystore_path: keystore_path.into(),
            password_file: password_file.into(),
            keystore_config,
        }
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
        let keystore_path =
            checked_runtime_path(&self.keystore_path, RuntimeSourceKind::KeystorePath)?;
        if !fs::metadata(keystore_path)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            return Err(KeystoreSignerError::MissingRuntimeSource {
                kind: RuntimeSourceKind::KeystorePath,
            });
        }
        let password = password_from_file(&self.password_file)?;
        let mut keystore = Keystore::new_with_config(keystore_path, self.keystore_config.clone())
            .map_err(keystore_open_error)?;
        keystore
            .unlock(password.as_str())
            .map_err(keystore_unlock_error)?;
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
            .field("password_file", &"<redacted>")
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

struct ResolvedPassword(Zeroizing<String>);

impl ResolvedPassword {
    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

fn checked_runtime_path(
    path: &Path,
    kind: RuntimeSourceKind,
) -> Result<&Path, KeystoreSignerError> {
    if path.as_os_str().is_empty() {
        return Err(KeystoreSignerError::InvalidRuntimeSource { kind });
    }
    Ok(path)
}

fn password_from_file(path: impl AsRef<Path>) -> Result<ResolvedPassword, KeystoreSignerError> {
    let path = checked_runtime_path(path.as_ref(), RuntimeSourceKind::PasswordFile)?;
    let mut contents = Zeroizing::new(fs::read_to_string(path).map_err(|_| {
        KeystoreSignerError::MissingRuntimeSource {
            kind: RuntimeSourceKind::PasswordFile,
        }
    })?);
    let trimmed_len = contents.trim_end_matches(['\r', '\n']).len();
    contents.truncate(trimmed_len);
    if contents.is_empty() {
        return Err(KeystoreSignerError::InvalidRuntimeSource {
            kind: RuntimeSourceKind::PasswordFile,
        });
    }
    Ok(ResolvedPassword(contents))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeSourceKind {
    KeystorePath,
    PasswordFile,
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
#[path = "tests.rs"]
mod tests;
