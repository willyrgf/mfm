#![warn(missing_docs)]
//! Runtime-only capability configuration parsing and validation.
//!
//! This crate owns the live runtime config file surface. It parses TOML and
//! JSON, resolves direct/env/file/file_env indirections, validates local
//! deployment shape, and returns redacted descriptors for app assembly.
//! It does not construct stores, transports, signers, runners, or replay
//! services.
//!
//! ```rust
//! use mfm_runtime_config::{RuntimeConfig, RuntimeConfigFormat};
//!
//! let runtime_toml = r#"
//! [evm.sources.reth-local]
//! rpc_url = "http://127.0.0.1:8545"
//!
//! [evm.routes.reth-dev]
//! source_ref = "reth-local"
//! "#;
//!
//! let config = RuntimeConfig::from_str(runtime_toml, RuntimeConfigFormat::Toml)?;
//! assert_eq!(config.evm().expect("evm config").sources().len(), 1);
//! # Ok::<(), mfm_runtime_config::RuntimeConfigError>(())
//! ```
//!
//! ```rust
//! use mfm_runtime_config::{RuntimeConfig, RuntimeConfigFormat};
//!
//! let runtime_json = r#"
//! {
//!   "evm": {
//!     "sources": {
//!       "reth-local": { "rpc_url": "http://127.0.0.1:8545" }
//!     },
//!     "routes": {
//!       "reth-dev": { "source_ref": "reth-local" }
//!     }
//!   }
//! }
//! "#;
//!
//! let config = RuntimeConfig::from_str(runtime_json, RuntimeConfigFormat::Json)?;
//! assert!(config.evm().is_some());
//! # Ok::<(), mfm_runtime_config::RuntimeConfigError>(())
//! ```

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use mfm_ids::{LocalPublicId, RuntimeEnvName};
use mfm_signing::SignerRef;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

/// Result type for runtime config parsing and validation.
pub type Result<T> = std::result::Result<T, RuntimeConfigError>;

/// Runtime config serialization format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigFormat {
    /// TOML runtime config.
    Toml,
    /// JSON runtime config.
    Json,
}

impl RuntimeConfigFormat {
    /// Detects the runtime config format from a path extension.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        match path
            .as_ref()
            .extension()
            .and_then(|extension| extension.to_str())
        {
            Some("toml") => Ok(Self::Toml),
            Some("json") => Ok(Self::Json),
            _ => Err(RuntimeConfigError::new(
                RuntimeConfigLocation::Root,
                RuntimeConfigErrorKind::UnsupportedFormat,
            )),
        }
    }
}

/// Capability families required from a runtime config parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfigRequirement {
    evm: bool,
    btc: bool,
    keystores: bool,
    signers: bool,
    parse_all: bool,
}

impl RuntimeConfigRequirement {
    /// Creates a requirement with no mandatory capability families and validates every present family.
    pub const fn none() -> Self {
        Self {
            evm: false,
            btc: false,
            keystores: false,
            signers: false,
            parse_all: true,
        }
    }

    /// Creates a requirement for EVM routes, sources, and policies.
    pub const fn evm() -> Self {
        Self {
            evm: true,
            btc: false,
            keystores: false,
            signers: false,
            parse_all: false,
        }
    }

    /// Creates a requirement for Bitcoin JSON-RPC runtime config.
    pub const fn btc() -> Self {
        Self {
            evm: false,
            btc: true,
            keystores: false,
            signers: false,
            parse_all: false,
        }
    }

    /// Creates a requirement for EVM routes, sources, policies, and signer bindings.
    pub const fn evm_with_signers() -> Self {
        Self {
            evm: true,
            btc: false,
            keystores: true,
            signers: true,
            parse_all: false,
        }
    }

    /// Creates a requirement for keystore profiles.
    pub const fn keystores() -> Self {
        Self {
            evm: false,
            btc: false,
            keystores: true,
            signers: false,
            parse_all: false,
        }
    }

    /// Returns whether the EVM runtime family is required.
    pub const fn requires_evm(self) -> bool {
        self.evm
    }

    /// Returns whether the Bitcoin runtime family is required.
    pub const fn requires_btc(self) -> bool {
        self.btc
    }

    const fn requires_keystores(self) -> bool {
        self.keystores || self.signers
    }

    const fn requires_signers(self) -> bool {
        self.signers
    }

    const fn parse_evm(self) -> bool {
        self.parse_all || self.evm
    }

    const fn parse_btc(self) -> bool {
        self.parse_all || self.btc
    }

    const fn parse_keystores(self) -> bool {
        self.parse_all || self.keystores || self.signers
    }

    const fn parse_signers(self) -> bool {
        self.parse_all || self.signers
    }
}

impl Default for RuntimeConfigRequirement {
    fn default() -> Self {
        Self::none()
    }
}

/// Parsed runtime config descriptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    evm: Option<EvmRuntimeConfig>,
    btc: Option<BtcRuntimeConfig>,
    keystores: BTreeMap<KeystoreRef, KeystoreRuntimeConfig>,
    signers: BTreeMap<SignerRef, RuntimeSigner>,
}

impl RuntimeConfig {
    /// Parses and validates a runtime config from a string.
    pub fn from_str(raw: &str, format: RuntimeConfigFormat) -> Result<Self> {
        Self::from_str_with_requirements(raw, format, RuntimeConfigRequirement::none())
    }

    /// Parses and validates a runtime config from a string with required families.
    pub fn from_str_with_requirements(
        raw: &str,
        format: RuntimeConfigFormat,
        requirements: RuntimeConfigRequirement,
    ) -> Result<Self> {
        let raw = parse_raw_config(raw, format)?;
        Self::from_raw(raw, requirements)
    }

    /// Loads, parses, and validates a runtime config from a file path.
    pub fn load_path(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_path_with_requirements(path, RuntimeConfigRequirement::none())
    }

    /// Loads, parses, and validates a runtime config file with required families.
    pub fn load_path_with_requirements(
        path: impl AsRef<Path>,
        requirements: RuntimeConfigRequirement,
    ) -> Result<Self> {
        let path = path.as_ref();
        let format = RuntimeConfigFormat::from_path(path)?;
        let raw = fs::read_to_string(path).map_err(|_| {
            RuntimeConfigError::new(
                RuntimeConfigLocation::Root,
                RuntimeConfigErrorKind::ConfigFileRead,
            )
        })?;
        Self::from_str_with_requirements(&raw, format, requirements)
    }

    /// Returns the parsed EVM runtime config, when present.
    pub const fn evm(&self) -> Option<&EvmRuntimeConfig> {
        self.evm.as_ref()
    }

    /// Returns the parsed Bitcoin runtime config, when present.
    pub const fn btc(&self) -> Option<&BtcRuntimeConfig> {
        self.btc.as_ref()
    }

    /// Returns configured keystore profiles.
    pub const fn keystores(&self) -> &BTreeMap<KeystoreRef, KeystoreRuntimeConfig> {
        &self.keystores
    }

    /// Returns configured runtime signer bindings.
    pub const fn signers(&self) -> &BTreeMap<SignerRef, RuntimeSigner> {
        &self.signers
    }

    fn from_raw(raw: RawRuntimeConfig, requirements: RuntimeConfigRequirement) -> Result<Self> {
        reject_extra_fields(&raw.extra, RuntimeConfigLocation::Root)?;
        let evm = if requirements.parse_evm() {
            match raw.evm {
                Some(evm) => Some(EvmRuntimeConfig::from_raw(deserialize_family(
                    evm,
                    RuntimeConfigLocation::Evm,
                    RuntimeConfigErrorKind::InvalidFamilyConfig,
                )?)?),
                None if requirements.requires_evm() => {
                    return Err(RuntimeConfigError::new(
                        RuntimeConfigLocation::Evm,
                        RuntimeConfigErrorKind::MissingFamily,
                    ));
                }
                None => None,
            }
        } else {
            None
        };
        let btc = if requirements.parse_btc() {
            match raw.btc {
                Some(btc) => Some(BtcRuntimeConfig::from_raw(deserialize_family(
                    btc,
                    RuntimeConfigLocation::Btc,
                    RuntimeConfigErrorKind::InvalidFamilyConfig,
                )?)?),
                None if requirements.requires_btc() => {
                    return Err(RuntimeConfigError::new(
                        RuntimeConfigLocation::Btc,
                        RuntimeConfigErrorKind::MissingFamily,
                    ));
                }
                None => None,
            }
        } else {
            None
        };
        let keystores = if requirements.parse_keystores() {
            parse_keystores(raw.keystores)?
        } else {
            BTreeMap::new()
        };
        if requirements.requires_keystores() && keystores.is_empty() {
            return Err(RuntimeConfigError::new(
                RuntimeConfigLocation::Keystore { keystore_ref: None },
                RuntimeConfigErrorKind::MissingFamily,
            ));
        }
        let signers = if requirements.parse_signers() {
            parse_signers(raw.signers, &keystores)?
        } else {
            BTreeMap::new()
        };
        if requirements.requires_signers() && signers.is_empty() {
            return Err(RuntimeConfigError::new(
                RuntimeConfigLocation::Signer { signer_ref: None },
                RuntimeConfigErrorKind::MissingFamily,
            ));
        }
        Ok(Self {
            evm,
            btc,
            keystores,
            signers,
        })
    }
}

fn deserialize_family<T>(
    raw: Value,
    location: RuntimeConfigLocation,
    kind: RuntimeConfigErrorKind,
) -> Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(raw).map_err(|_| RuntimeConfigError::new(location, kind))
}

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

fn parse_keystores(raw: Option<Value>) -> Result<BTreeMap<KeystoreRef, KeystoreRuntimeConfig>> {
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

fn parse_signers(
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

/// Source kind used to resolve a runtime-local secret value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeValueSourceKind {
    /// Value was supplied directly in the runtime config.
    Direct,
    /// Value was read from an environment variable.
    Env,
    /// Value was read from a file path in the runtime config.
    File,
    /// Value was read from a file path obtained from an environment variable.
    FileEnv,
}

/// Resolved runtime-local value with redacted debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeSecretValue {
    value: String,
    source_kind: RuntimeValueSourceKind,
}

impl RuntimeSecretValue {
    /// Returns the resolved value.
    ///
    /// Callers must keep this below persisted, public, and replay surfaces.
    pub fn expose_secret(&self) -> &str {
        &self.value
    }

    /// Returns the source kind used to resolve this value.
    pub const fn source_kind(&self) -> RuntimeValueSourceKind {
        self.source_kind
    }
}

impl fmt::Debug for RuntimeSecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSecretValue")
            .field("value", &"<redacted>")
            .field("source_kind", &self.source_kind)
            .finish()
    }
}

/// Resolved runtime-local path with redacted debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeSecretPath {
    path: PathBuf,
    source_kind: RuntimeValueSourceKind,
}

impl RuntimeSecretPath {
    /// Returns the resolved path.
    ///
    /// Callers must keep this below persisted, public, and replay surfaces.
    pub fn expose_path(&self) -> &Path {
        &self.path
    }

    /// Returns the source kind used to resolve this path.
    pub const fn source_kind(&self) -> RuntimeValueSourceKind {
        self.source_kind
    }
}

impl fmt::Debug for RuntimeSecretPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSecretPath")
            .field("path", &"<redacted>")
            .field("source_kind", &self.source_kind)
            .finish()
    }
}

#[path = "errors.rs"]
mod errors;
pub use self::errors::{
    RuntimeConfigError, RuntimeConfigErrorKind, RuntimeConfigIdentifierKind, RuntimeConfigLocation,
};

#[path = "evm.rs"]
mod evm;
pub use self::evm::{EvmRoute, EvmRpcSource, EvmRuntimeConfig, EvmSourcePolicy};

#[path = "btc.rs"]
mod btc;
pub use self::btc::{BtcJsonRpcRuntimeConfig, BtcRuntimeConfig};

#[path = "raw.rs"]
mod raw;
use self::raw::{
    parse_raw_config, reject_extra_fields, RawKeystoreConfig, RawRuntimeConfig, RawSignerConfig,
};

#[path = "resolve.rs"]
mod resolve;
use self::resolve::resolve_required_path;
