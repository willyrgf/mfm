use std::collections::BTreeMap;
use std::fmt;

use mfm_ids::LocalPublicId;
use mfm_signing::SignerRef;
use serde_json::Value;
use uuid::Uuid;

use super::raw::{RawKeystoreConfig, RawSignerConfig};
use super::resolve::resolve_required_path;
use super::{
    deserialize_family, reject_extra_fields, Result, RuntimeConfigError, RuntimeConfigErrorKind,
    RuntimeConfigIdentifierKind, RuntimeConfigLocation, RuntimeSecretPath,
};

/// Process-local keystore profile reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeystoreRef(LocalPublicId);

impl KeystoreRef {
    /// Creates a checked keystore profile reference.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = LocalPublicId::new(value).map_err(|_| {
            RuntimeConfigError::new(
                RuntimeConfigLocation::Keystore { keystore_ref: None },
                RuntimeConfigErrorKind::InvalidIdentifier {
                    kind: RuntimeConfigIdentifierKind::KeystoreRef,
                },
            )
        })?;
        Ok(Self(value))
    }

    /// Returns the canonical keystore profile reference string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for KeystoreRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for KeystoreRef {
    type Err = RuntimeConfigError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// Runtime MFM keystore profile descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct KeystoreRuntimeConfig {
    keystore_path: RuntimeSecretPath,
    unlock_file: RuntimeSecretPath,
}

impl KeystoreRuntimeConfig {
    /// Returns the resolved keystore path.
    pub const fn keystore_path(&self) -> &RuntimeSecretPath {
        &self.keystore_path
    }

    /// Returns the resolved unlock-file path.
    pub const fn unlock_file(&self) -> &RuntimeSecretPath {
        &self.unlock_file
    }

    fn from_raw(raw: RawKeystoreConfig, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;
        let keystore_path = resolve_required_path(
            location.clone(),
            "keystore_path",
            &raw.keystore_path,
            &raw.keystore_path_env,
            &raw.keystore_path_file,
            &raw.keystore_path_file_env,
        )?;
        let unlock_file = resolve_required_path(
            location,
            "unlock_file",
            &raw.unlock_file,
            &raw.unlock_file_env,
            &raw.unlock_file_file,
            &raw.unlock_file_file_env,
        )?;
        Ok(Self {
            keystore_path,
            unlock_file,
        })
    }
}

impl fmt::Debug for KeystoreRuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeystoreRuntimeConfig")
            .field("keystore_path", &self.keystore_path)
            .field("unlock_file", &self.unlock_file)
            .finish()
    }
}

pub(super) fn parse_keystores(
    raw: Option<Value>,
) -> Result<BTreeMap<KeystoreRef, KeystoreRuntimeConfig>> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let raw_keystores = deserialize_family::<BTreeMap<String, RawKeystoreConfig>>(
        raw,
        RuntimeConfigLocation::Root.with_field("keystores"),
        RuntimeConfigErrorKind::InvalidKeystoreConfig,
    )?;
    let mut keystores = BTreeMap::new();
    for (raw_keystore_ref, raw_keystore) in raw_keystores {
        let keystore_ref = parse_keystore_ref(
            &raw_keystore_ref,
            RuntimeConfigLocation::Keystore { keystore_ref: None },
        )?;
        let location = RuntimeConfigLocation::Keystore {
            keystore_ref: Some(keystore_ref.to_string()),
        };
        let keystore = KeystoreRuntimeConfig::from_raw(raw_keystore, location)?;
        keystores.insert(keystore_ref, keystore);
    }
    Ok(keystores)
}

/// Runtime signer provider binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSigner {
    /// MFM keystore-backed signer binding.
    Keystore(RuntimeKeystoreSigner),
}

impl RuntimeSigner {
    /// Returns the keystore signer binding when this entry uses the keystore provider.
    pub const fn as_keystore(&self) -> &RuntimeKeystoreSigner {
        match self {
            Self::Keystore(signer) => signer,
        }
    }

    fn from_raw(
        raw: RawSignerConfig,
        location: RuntimeConfigLocation,
        keystores: &BTreeMap<KeystoreRef, KeystoreRuntimeConfig>,
    ) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;
        let Some(provider) = raw.provider.as_deref() else {
            return Err(RuntimeConfigError::new(
                location.with_field("provider"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        };
        match provider {
            "keystore" => Ok(Self::Keystore(RuntimeKeystoreSigner::from_raw(
                raw, location, keystores,
            )?)),
            _ => Err(RuntimeConfigError::new(
                location.with_field("provider"),
                RuntimeConfigErrorKind::UnsupportedSignerProvider,
            )),
        }
    }
}

/// Runtime MFM keystore signer descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeKeystoreSigner {
    entry_id: Uuid,
    keystore_ref: KeystoreRef,
}

impl RuntimeKeystoreSigner {
    /// Returns the keystore entry id.
    pub const fn entry_id(&self) -> Uuid {
        self.entry_id
    }

    /// Returns the referenced keystore profile.
    pub const fn keystore_ref(&self) -> &KeystoreRef {
        &self.keystore_ref
    }

    fn from_raw(
        raw: RawSignerConfig,
        location: RuntimeConfigLocation,
        keystores: &BTreeMap<KeystoreRef, KeystoreRuntimeConfig>,
    ) -> Result<Self> {
        let Some(raw_keystore_ref) = raw.keystore_ref.as_deref() else {
            return Err(RuntimeConfigError::new(
                location.clone().with_field("keystore_ref"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        };
        let keystore_ref = parse_keystore_ref(raw_keystore_ref, location.clone())?;
        if !keystores.contains_key(&keystore_ref) {
            return Err(RuntimeConfigError::new(
                location.clone().with_field("keystore_ref"),
                RuntimeConfigErrorKind::MissingKeystore,
            ));
        }
        let Some(raw_entry_id) = raw.entry_id.as_deref() else {
            return Err(RuntimeConfigError::new(
                location.with_field("entry_id"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        };
        let entry_id = Uuid::parse_str(raw_entry_id).map_err(|_| {
            RuntimeConfigError::new(
                location.with_field("entry_id"),
                RuntimeConfigErrorKind::InvalidEntryId,
            )
        })?;
        Ok(Self {
            entry_id,
            keystore_ref,
        })
    }
}

impl fmt::Debug for RuntimeKeystoreSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeKeystoreSigner")
            .field("entry_id", &self.entry_id)
            .field("keystore_ref", &self.keystore_ref)
            .finish()
    }
}

pub(super) fn parse_signers(
    raw: Option<Value>,
    keystores: &BTreeMap<KeystoreRef, KeystoreRuntimeConfig>,
) -> Result<BTreeMap<SignerRef, RuntimeSigner>> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let raw_signers = deserialize_family::<BTreeMap<String, RawSignerConfig>>(
        raw,
        RuntimeConfigLocation::Root.with_field("signers"),
        RuntimeConfigErrorKind::InvalidSignerConfig,
    )?;
    let mut signers = BTreeMap::new();
    for (raw_signer_ref, raw_signer) in raw_signers {
        let signer_ref = SignerRef::new(&raw_signer_ref).map_err(|_| {
            RuntimeConfigError::new(
                RuntimeConfigLocation::Signer { signer_ref: None },
                RuntimeConfigErrorKind::InvalidIdentifier {
                    kind: RuntimeConfigIdentifierKind::SignerRef,
                },
            )
        })?;
        let location = RuntimeConfigLocation::Signer {
            signer_ref: Some(signer_ref.to_string()),
        };
        let signer = RuntimeSigner::from_raw(raw_signer, location, keystores)?;
        signers.insert(signer_ref, signer);
    }
    Ok(signers)
}

fn parse_keystore_ref(raw: &str, location: RuntimeConfigLocation) -> Result<KeystoreRef> {
    KeystoreRef::new(raw).map_err(|_| {
        RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::KeystoreRef,
            },
        )
    })
}
