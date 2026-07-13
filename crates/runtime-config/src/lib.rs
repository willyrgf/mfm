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
use std::fs;
use std::path::Path;

use mfm_ids::RuntimeEnvName;
use mfm_signing::SignerRef;
use serde::de::DeserializeOwned;
use serde_json::Value;

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

#[path = "signers.rs"]
mod signers;
use self::signers::{parse_keystores, parse_signers};
pub use self::signers::{KeystoreRef, KeystoreRuntimeConfig, RuntimeKeystoreSigner, RuntimeSigner};

#[path = "values.rs"]
mod values;
pub use self::values::{RuntimeSecretPath, RuntimeSecretValue, RuntimeValueSourceKind};

#[path = "raw.rs"]
mod raw;
use self::raw::{parse_raw_config, reject_extra_fields, RawRuntimeConfig};

#[path = "resolve.rs"]
mod resolve;
