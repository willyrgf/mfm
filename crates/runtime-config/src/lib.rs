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

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use mfm_evm_capabilities::{EvmNetworkId, EvmSourcePolicyId, EvmSourceRef};
use mfm_ids::{LocalPublicId, RuntimeEnvName};
use mfm_signing::SignerRef;
use serde::de::DeserializeOwned;
use serde::Deserialize;
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

/// Runtime-local EVM descriptors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRuntimeConfig {
    sources: BTreeMap<EvmSourceRef, EvmRpcSource>,
    policies: BTreeMap<EvmSourcePolicyId, EvmSourcePolicy>,
    routes: BTreeMap<EvmNetworkId, EvmRoute>,
}

impl EvmRuntimeConfig {
    /// Returns configured EVM JSON-RPC sources.
    pub const fn sources(&self) -> &BTreeMap<EvmSourceRef, EvmRpcSource> {
        &self.sources
    }

    /// Returns configured and synthesized EVM source policies.
    pub const fn policies(&self) -> &BTreeMap<EvmSourcePolicyId, EvmSourcePolicy> {
        &self.policies
    }

    /// Returns semantic network routes.
    pub const fn routes(&self) -> &BTreeMap<EvmNetworkId, EvmRoute> {
        &self.routes
    }

    fn from_raw(raw: RawEvmConfig) -> Result<Self> {
        reject_extra_fields(&raw.extra, RuntimeConfigLocation::Evm)?;

        let mut sources = BTreeMap::new();
        for (raw_id, raw_source) in raw.sources {
            let source_ref = parse_source_ref(
                &raw_id,
                RuntimeConfigLocation::EvmSource { source_ref: None },
            )?;
            let location = RuntimeConfigLocation::EvmSource {
                source_ref: Some(source_ref.to_string()),
            };
            let source = EvmRpcSource::from_raw(raw_source, location)?;
            sources.insert(source_ref, source);
        }

        let mut policies = BTreeMap::new();
        for (raw_id, raw_policy) in raw.policies {
            let policy_id = parse_policy_id(
                &raw_id,
                RuntimeConfigLocation::EvmPolicy { policy_id: None },
            )?;
            let location = RuntimeConfigLocation::EvmPolicy {
                policy_id: Some(policy_id.to_string()),
            };
            let policy = EvmSourcePolicy::from_raw(raw_policy, location)?;
            for source_ref in policy.ordered_sources() {
                if !sources.contains_key(source_ref) {
                    return Err(RuntimeConfigError::new(
                        RuntimeConfigLocation::EvmPolicy {
                            policy_id: Some(policy_id.to_string()),
                        },
                        RuntimeConfigErrorKind::MissingSource,
                    ));
                }
            }
            policies.insert(policy_id, policy);
        }

        let explicit_policy_ids = policies.keys().cloned().collect::<BTreeSet<_>>();
        let mut routes = BTreeMap::new();
        for (raw_network_id, raw_route) in raw.routes {
            let network_id = EvmNetworkId::new(&raw_network_id).map_err(|_| {
                RuntimeConfigError::new(
                    RuntimeConfigLocation::EvmRoute { network_id: None },
                    RuntimeConfigErrorKind::InvalidIdentifier {
                        kind: RuntimeConfigIdentifierKind::NetworkId,
                    },
                )
            })?;
            let location = RuntimeConfigLocation::EvmRoute {
                network_id: Some(network_id.to_string()),
            };
            let route = EvmRoute::from_raw(
                raw_route,
                location.clone(),
                &sources,
                &mut policies,
                &explicit_policy_ids,
            )?;
            routes.insert(network_id, route);
        }

        Ok(Self {
            sources,
            policies,
            routes,
        })
    }
}

/// Runtime EVM JSON-RPC source descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmRpcSource {
    rpc_url: RuntimeSecretValue,
    auth_header: Option<RuntimeSecretValue>,
}

impl EvmRpcSource {
    /// Returns the resolved RPC URL.
    pub const fn rpc_url(&self) -> &RuntimeSecretValue {
        &self.rpc_url
    }

    /// Returns the resolved authorization header, when configured.
    pub const fn auth_header(&self) -> Option<&RuntimeSecretValue> {
        self.auth_header.as_ref()
    }

    fn from_raw(raw: RawEvmSource, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;

        let rpc_url = resolve_required_value(
            location.clone(),
            "rpc_url",
            &raw.rpc_url,
            &raw.rpc_url_env,
            &raw.rpc_url_file,
            &raw.rpc_url_file_env,
        )?;
        validate_rpc_url(&rpc_url, location.clone().with_field("rpc_url"))?;

        let auth_header = resolve_optional_value(
            location,
            "auth_header",
            &raw.auth_header,
            &raw.auth_header_env,
            &raw.auth_header_file,
            &raw.auth_header_file_env,
        )?;

        Ok(Self {
            rpc_url,
            auth_header,
        })
    }
}

impl fmt::Debug for EvmRpcSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmRpcSource")
            .field("rpc_url", &self.rpc_url)
            .field("auth_header", &self.auth_header)
            .finish()
    }
}

/// Ordered EVM source fallback policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmSourcePolicy {
    ordered_sources: Vec<EvmSourceRef>,
    synthesized: bool,
}

impl EvmSourcePolicy {
    /// Creates an explicit runtime EVM source policy.
    pub fn explicit(ordered_sources: Vec<EvmSourceRef>) -> Self {
        Self {
            ordered_sources,
            synthesized: false,
        }
    }

    /// Returns ordered source references for this policy.
    pub fn ordered_sources(&self) -> &[EvmSourceRef] {
        &self.ordered_sources
    }

    /// Returns true when this policy was synthesized from a same-id route.
    pub const fn is_synthesized(&self) -> bool {
        self.synthesized
    }

    fn synthesized(source_ref: EvmSourceRef) -> Self {
        Self {
            ordered_sources: vec![source_ref],
            synthesized: true,
        }
    }

    fn from_raw(raw: RawEvmPolicy, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;
        let Some(raw_sources) = raw.ordered_sources else {
            return Err(RuntimeConfigError::new(
                location.with_field("ordered_sources"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        };
        if raw_sources.is_empty() {
            return Err(RuntimeConfigError::new(
                location.with_field("ordered_sources"),
                RuntimeConfigErrorKind::EmptyPolicy,
            ));
        }

        let mut seen = BTreeSet::new();
        let mut ordered_sources = Vec::new();
        for raw_source_ref in raw_sources {
            let source_ref = parse_source_ref(&raw_source_ref, location.clone())?;
            if !seen.insert(source_ref.clone()) {
                return Err(RuntimeConfigError::new(
                    location.with_field("ordered_sources"),
                    RuntimeConfigErrorKind::DuplicatePolicySource,
                ));
            }
            ordered_sources.push(source_ref);
        }
        Ok(Self::explicit(ordered_sources))
    }
}

/// Runtime EVM route from a semantic network id to local source policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRoute {
    source_ref: EvmSourceRef,
    policy_id: EvmSourcePolicyId,
}

impl EvmRoute {
    /// Returns the preferred local source reference.
    pub const fn source_ref(&self) -> &EvmSourceRef {
        &self.source_ref
    }

    /// Returns the source policy id.
    pub const fn policy_id(&self) -> &EvmSourcePolicyId {
        &self.policy_id
    }

    fn from_raw(
        raw: RawEvmRoute,
        location: RuntimeConfigLocation,
        sources: &BTreeMap<EvmSourceRef, EvmRpcSource>,
        policies: &mut BTreeMap<EvmSourcePolicyId, EvmSourcePolicy>,
        explicit_policy_ids: &BTreeSet<EvmSourcePolicyId>,
    ) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;

        let Some(raw_source_ref) = raw.source_ref else {
            return Err(RuntimeConfigError::new(
                location.with_field("source_ref"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        };
        let source_ref = parse_source_ref(&raw_source_ref, location.clone())?;
        if !sources.contains_key(&source_ref) {
            return Err(RuntimeConfigError::new(
                location.with_field("source_ref"),
                RuntimeConfigErrorKind::MissingSource,
            ));
        }

        let policy_id = match raw.policy_id {
            Some(raw_policy_id) => parse_policy_id(&raw_policy_id, location.clone())?,
            None => {
                let same_id = EvmSourcePolicyId::new(source_ref.as_str()).map_err(|_| {
                    RuntimeConfigError::new(
                        location.clone().with_field("policy_id"),
                        RuntimeConfigErrorKind::InvalidIdentifier {
                            kind: RuntimeConfigIdentifierKind::PolicyId,
                        },
                    )
                })?;
                if explicit_policy_ids.contains(&same_id) {
                    return Err(RuntimeConfigError::new(
                        location.clone().with_field("policy_id"),
                        RuntimeConfigErrorKind::SameIdPolicyRequiresExplicitPolicyId,
                    ));
                }
                policies
                    .entry(same_id.clone())
                    .or_insert_with(|| EvmSourcePolicy::synthesized(source_ref.clone()));
                same_id
            }
        };

        let policy = policies.get(&policy_id).ok_or_else(|| {
            RuntimeConfigError::new(
                location.clone().with_field("policy_id"),
                RuntimeConfigErrorKind::MissingPolicy,
            )
        })?;
        if !policy.ordered_sources().contains(&source_ref) {
            return Err(RuntimeConfigError::new(
                location.with_field("source_ref"),
                RuntimeConfigErrorKind::SourceNotInPolicy,
            ));
        }

        Ok(Self {
            source_ref,
            policy_id,
        })
    }
}

/// Runtime Bitcoin JSON-RPC descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct BtcRuntimeConfig {
    json_rpc: BtcJsonRpcRuntimeConfig,
}

impl BtcRuntimeConfig {
    /// Returns the configured Bitcoin JSON-RPC endpoint descriptor.
    pub const fn json_rpc(&self) -> &BtcJsonRpcRuntimeConfig {
        &self.json_rpc
    }

    fn from_raw(raw: RawBtcConfig) -> Result<Self> {
        reject_extra_fields(&raw.extra, RuntimeConfigLocation::Btc)?;
        let json_rpc = BtcJsonRpcRuntimeConfig::from_raw(
            raw.json_rpc,
            RuntimeConfigLocation::Btc.with_field("json_rpc"),
        )?;
        Ok(Self { json_rpc })
    }
}

impl fmt::Debug for BtcRuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BtcRuntimeConfig")
            .field("json_rpc", &self.json_rpc)
            .finish()
    }
}

/// Runtime Bitcoin Core JSON-RPC endpoint descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct BtcJsonRpcRuntimeConfig {
    rpc_url: RuntimeSecretValue,
    rpc_user: Option<RuntimeSecretValue>,
    rpc_password: Option<RuntimeSecretValue>,
}

impl BtcJsonRpcRuntimeConfig {
    /// Returns the resolved Bitcoin Core RPC URL.
    pub const fn rpc_url(&self) -> &RuntimeSecretValue {
        &self.rpc_url
    }

    /// Returns the resolved RPC username, when configured.
    pub const fn rpc_user(&self) -> Option<&RuntimeSecretValue> {
        self.rpc_user.as_ref()
    }

    /// Returns the resolved RPC password, when configured.
    pub const fn rpc_password(&self) -> Option<&RuntimeSecretValue> {
        self.rpc_password.as_ref()
    }

    fn from_raw(raw: RawBtcJsonRpcConfig, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;
        let rpc_url = resolve_required_value(
            location.clone(),
            "rpc_url",
            &raw.rpc_url,
            &raw.rpc_url_env,
            &raw.rpc_url_file,
            &raw.rpc_url_file_env,
        )?;
        validate_rpc_url(&rpc_url, location.clone().with_field("rpc_url"))?;
        let rpc_user = resolve_optional_value(
            location.clone(),
            "rpc_user",
            &raw.rpc_user,
            &raw.rpc_user_env,
            &raw.rpc_user_file,
            &raw.rpc_user_file_env,
        )?;
        let rpc_password = resolve_optional_value(
            location.clone(),
            "rpc_password",
            &raw.rpc_password,
            &raw.rpc_password_env,
            &raw.rpc_password_file,
            &raw.rpc_password_file_env,
        )?;
        if rpc_user.is_some() != rpc_password.is_some() {
            return Err(RuntimeConfigError::new(
                location,
                RuntimeConfigErrorKind::IncompleteBasicAuth,
            ));
        }
        Ok(Self {
            rpc_url,
            rpc_user,
            rpc_password,
        })
    }
}

impl fmt::Debug for BtcJsonRpcRuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BtcJsonRpcRuntimeConfig")
            .field("rpc_url", &self.rpc_url)
            .field("rpc_user", &self.rpc_user)
            .field("rpc_password", &self.rpc_password)
            .finish()
    }
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

/// Redaction-safe runtime config error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("runtime config {location}: {kind}")]
pub struct RuntimeConfigError {
    location: RuntimeConfigLocation,
    kind: RuntimeConfigErrorKind,
}

impl RuntimeConfigError {
    fn new(location: RuntimeConfigLocation, kind: RuntimeConfigErrorKind) -> Self {
        Self { location, kind }
    }

    /// Returns the redaction-safe error location.
    pub const fn location(&self) -> &RuntimeConfigLocation {
        &self.location
    }

    /// Returns the closed error kind.
    pub const fn kind(&self) -> &RuntimeConfigErrorKind {
        &self.kind
    }
}

/// Redaction-safe runtime config error location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeConfigLocation {
    /// Top-level runtime config.
    Root,
    /// EVM capability family.
    Evm,
    /// Bitcoin capability family.
    Btc,
    /// EVM source entry.
    EvmSource {
        /// Checked source ref when available.
        source_ref: Option<String>,
    },
    /// EVM source policy entry.
    EvmPolicy {
        /// Checked policy id when available.
        policy_id: Option<String>,
    },
    /// EVM semantic network route entry.
    EvmRoute {
        /// Checked semantic network id when available.
        network_id: Option<String>,
    },
    /// Keystore profile entry.
    Keystore {
        /// Checked keystore profile ref when available.
        keystore_ref: Option<String>,
    },
    /// Runtime signer entry.
    Signer {
        /// Checked signer ref when available.
        signer_ref: Option<String>,
    },
    /// Known field within a parent location.
    Field {
        /// Parent location.
        parent: Box<RuntimeConfigLocation>,
        /// Redaction-safe field name.
        field: &'static str,
    },
}

impl RuntimeConfigLocation {
    fn with_field(self, field: &'static str) -> Self {
        Self::Field {
            parent: Box::new(self),
            field,
        }
    }
}

impl fmt::Display for RuntimeConfigLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("root"),
            Self::Evm => f.write_str("evm"),
            Self::Btc => f.write_str("btc"),
            Self::EvmSource { source_ref } => match source_ref {
                Some(source_ref) => write!(f, "evm.sources[{source_ref}]"),
                None => f.write_str("evm.sources[<invalid>]"),
            },
            Self::EvmPolicy { policy_id } => match policy_id {
                Some(policy_id) => write!(f, "evm.policies[{policy_id}]"),
                None => f.write_str("evm.policies[<invalid>]"),
            },
            Self::EvmRoute { network_id } => match network_id {
                Some(network_id) => write!(f, "evm.routes[{network_id}]"),
                None => f.write_str("evm.routes[<invalid>]"),
            },
            Self::Keystore { keystore_ref } => match keystore_ref {
                Some(keystore_ref) => write!(f, "keystores[{keystore_ref}]"),
                None => f.write_str("keystores[<invalid>]"),
            },
            Self::Signer { signer_ref } => match signer_ref {
                Some(signer_ref) => write!(f, "signers[{signer_ref}]"),
                None => f.write_str("signers[<invalid>]"),
            },
            Self::Field { parent, field } => write!(f, "{parent}.{field}"),
        }
    }
}

/// Closed runtime config error kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeConfigErrorKind {
    /// File extension does not identify TOML or JSON.
    UnsupportedFormat,
    /// Runtime config syntax could not be parsed.
    Syntax {
        /// Runtime config format being parsed.
        format: RuntimeConfigFormat,
    },
    /// Runtime config file could not be read.
    ConfigFileRead,
    /// A required capability family was missing.
    MissingFamily,
    /// Capability family section did not have the expected shape.
    InvalidFamilyConfig,
    /// Runtime config contained an unknown field.
    UnknownField,
    /// Runtime config contained forbidden source chain metadata.
    ForbiddenExpectedChainId,
    /// Runtime config contained forbidden secret-bearing material.
    ForbiddenSecretMaterial,
    /// Runtime config identifier failed its grammar.
    InvalidIdentifier {
        /// Identifier kind.
        kind: RuntimeConfigIdentifierKind,
    },
    /// Required field was not present.
    MissingRequiredField,
    /// Required value source did not provide exactly one source.
    ExactlyOneValueSource,
    /// Optional value source provided more than one source.
    AtMostOneValueSource,
    /// Runtime environment variable was missing.
    MissingEnv,
    /// Runtime environment variable was not usable.
    InvalidEnv,
    /// Indirection file could not be read.
    IndirectionFileRead,
    /// Resolved value was empty.
    EmptyResolvedValue,
    /// RPC URL was not syntactically valid.
    InvalidUrl,
    /// RPC URL contained userinfo.
    UrlUserInfo,
    /// Source policy had no source refs.
    EmptyPolicy,
    /// Source policy contained the same source more than once.
    DuplicatePolicySource,
    /// Referenced source was missing.
    MissingSource,
    /// Referenced source policy was missing.
    MissingPolicy,
    /// Route source was not included in its selected policy.
    SourceNotInPolicy,
    /// Route omitted policy_id while an explicit same-id policy existed.
    SameIdPolicyRequiresExplicitPolicyId,
    /// Bitcoin JSON-RPC basic authentication had only one of user/password.
    IncompleteBasicAuth,
    /// Signer provider was unsupported.
    UnsupportedSignerProvider,
    /// Signer binding section did not have the expected shape.
    InvalidSignerConfig,
    /// Keystore profile section did not have the expected shape.
    InvalidKeystoreConfig,
    /// Referenced keystore profile was missing.
    MissingKeystore,
    /// Keystore entry id was malformed.
    InvalidEntryId,
}

impl fmt::Display for RuntimeConfigErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat => f.write_str("unsupported format"),
            Self::Syntax { format } => write!(f, "{format:?} syntax is invalid"),
            Self::ConfigFileRead => f.write_str("config file could not be read"),
            Self::MissingFamily => f.write_str("required capability family is missing"),
            Self::InvalidFamilyConfig => f.write_str("capability family config is invalid"),
            Self::UnknownField => f.write_str("unknown field"),
            Self::ForbiddenExpectedChainId => {
                f.write_str("source-level expected_chain_id is forbidden")
            }
            Self::ForbiddenSecretMaterial => f.write_str("forbidden secret material field"),
            Self::InvalidIdentifier { kind } => write!(f, "invalid {kind}"),
            Self::MissingRequiredField => f.write_str("required field is missing"),
            Self::ExactlyOneValueSource => {
                f.write_str("exactly one value source must be configured")
            }
            Self::AtMostOneValueSource => f.write_str("at most one value source may be configured"),
            Self::MissingEnv => f.write_str("runtime environment variable is missing"),
            Self::InvalidEnv => f.write_str("runtime environment variable is invalid"),
            Self::IndirectionFileRead => f.write_str("indirection file could not be read"),
            Self::EmptyResolvedValue => f.write_str("resolved value is empty"),
            Self::InvalidUrl => f.write_str("RPC URL is invalid"),
            Self::UrlUserInfo => f.write_str("RPC URL userinfo is forbidden"),
            Self::EmptyPolicy => f.write_str("source policy is empty"),
            Self::DuplicatePolicySource => f.write_str("source policy contains a duplicate source"),
            Self::MissingSource => f.write_str("referenced source is missing"),
            Self::MissingPolicy => f.write_str("referenced source policy is missing"),
            Self::SourceNotInPolicy => f.write_str("route source is not in selected policy"),
            Self::SameIdPolicyRequiresExplicitPolicyId => {
                f.write_str("explicit same-id policy requires explicit policy_id")
            }
            Self::IncompleteBasicAuth => {
                f.write_str("basic authentication requires both user and password")
            }
            Self::UnsupportedSignerProvider => f.write_str("signer provider is unsupported"),
            Self::InvalidSignerConfig => f.write_str("signer config is invalid"),
            Self::InvalidKeystoreConfig => f.write_str("keystore config is invalid"),
            Self::MissingKeystore => f.write_str("referenced keystore profile is missing"),
            Self::InvalidEntryId => f.write_str("keystore entry id is invalid"),
        }
    }
}

/// Closed runtime config identifier kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigIdentifierKind {
    /// EVM source reference.
    SourceRef,
    /// EVM source policy id.
    PolicyId,
    /// EVM semantic network id.
    NetworkId,
    /// Signer reference.
    SignerRef,
    /// Keystore profile reference.
    KeystoreRef,
    /// Runtime environment variable name.
    EnvName,
}

impl fmt::Display for RuntimeConfigIdentifierKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceRef => f.write_str("source_ref"),
            Self::PolicyId => f.write_str("policy_id"),
            Self::NetworkId => f.write_str("network_id"),
            Self::SignerRef => f.write_str("signer_ref"),
            Self::KeystoreRef => f.write_str("keystore_ref"),
            Self::EnvName => f.write_str("env_name"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawRuntimeConfig {
    #[serde(default)]
    evm: Option<Value>,
    #[serde(default)]
    btc: Option<Value>,
    #[serde(default)]
    keystores: Option<Value>,
    #[serde(default)]
    signers: Option<Value>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvmConfig {
    #[serde(default)]
    sources: BTreeMap<String, RawEvmSource>,
    #[serde(default)]
    policies: BTreeMap<String, RawEvmPolicy>,
    #[serde(default)]
    routes: BTreeMap<String, RawEvmRoute>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvmSource {
    #[serde(default)]
    rpc_url: Option<String>,
    #[serde(default)]
    rpc_url_env: Option<String>,
    #[serde(default)]
    rpc_url_file: Option<String>,
    #[serde(default)]
    rpc_url_file_env: Option<String>,
    #[serde(default)]
    auth_header: Option<String>,
    #[serde(default)]
    auth_header_env: Option<String>,
    #[serde(default)]
    auth_header_file: Option<String>,
    #[serde(default)]
    auth_header_file_env: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvmPolicy {
    #[serde(default)]
    ordered_sources: Option<Vec<String>>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawEvmRoute {
    #[serde(default)]
    source_ref: Option<String>,
    #[serde(default)]
    policy_id: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawBtcConfig {
    #[serde(default)]
    json_rpc: RawBtcJsonRpcConfig,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawBtcJsonRpcConfig {
    #[serde(default)]
    rpc_url: Option<String>,
    #[serde(default)]
    rpc_url_env: Option<String>,
    #[serde(default)]
    rpc_url_file: Option<String>,
    #[serde(default)]
    rpc_url_file_env: Option<String>,
    #[serde(default)]
    rpc_user: Option<String>,
    #[serde(default)]
    rpc_user_env: Option<String>,
    #[serde(default)]
    rpc_user_file: Option<String>,
    #[serde(default)]
    rpc_user_file_env: Option<String>,
    #[serde(default)]
    rpc_password: Option<String>,
    #[serde(default)]
    rpc_password_env: Option<String>,
    #[serde(default)]
    rpc_password_file: Option<String>,
    #[serde(default)]
    rpc_password_file_env: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKeystoreConfig {
    #[serde(default)]
    keystore_path: Option<String>,
    #[serde(default)]
    keystore_path_env: Option<String>,
    #[serde(default)]
    keystore_path_file: Option<String>,
    #[serde(default)]
    keystore_path_file_env: Option<String>,
    #[serde(default)]
    unlock_file: Option<String>,
    #[serde(default)]
    unlock_file_env: Option<String>,
    #[serde(default)]
    unlock_file_file: Option<String>,
    #[serde(default)]
    unlock_file_file_env: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSignerConfig {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    keystore_ref: Option<String>,
    #[serde(default)]
    entry_id: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

fn parse_raw_config(raw: &str, format: RuntimeConfigFormat) -> Result<RawRuntimeConfig> {
    match format {
        RuntimeConfigFormat::Toml => toml::from_str(raw).map_err(|_| {
            RuntimeConfigError::new(
                RuntimeConfigLocation::Root,
                RuntimeConfigErrorKind::Syntax { format },
            )
        }),
        RuntimeConfigFormat::Json => serde_json::from_str(raw).map_err(|_| {
            RuntimeConfigError::new(
                RuntimeConfigLocation::Root,
                RuntimeConfigErrorKind::Syntax { format },
            )
        }),
    }
}

fn reject_extra_fields(
    extra: &BTreeMap<String, Value>,
    location: RuntimeConfigLocation,
) -> Result<()> {
    if let Some(field) = extra.keys().next() {
        let kind = match forbidden_field_kind(field.as_str()) {
            Some(kind) => kind,
            None => RuntimeConfigErrorKind::UnknownField,
        };
        return Err(RuntimeConfigError::new(location, kind));
    }
    Ok(())
}

fn forbidden_field_kind(field: &str) -> Option<RuntimeConfigErrorKind> {
    let field = field.to_ascii_lowercase();
    if field == "expected_chain_id" {
        return Some(RuntimeConfigErrorKind::ForbiddenExpectedChainId);
    }
    if field.contains("private_key")
        || field.contains("mnemonic")
        || field.contains("password")
        || field.contains("signed_material")
        || field.contains("signed_transaction")
        || field.contains("raw_transaction")
        || field.contains("raw_tx")
        || field.contains("signature_scalar")
    {
        return Some(RuntimeConfigErrorKind::ForbiddenSecretMaterial);
    }
    None
}

fn parse_source_ref(raw: &str, location: RuntimeConfigLocation) -> Result<EvmSourceRef> {
    EvmSourceRef::new(raw).map_err(|_| {
        RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::SourceRef,
            },
        )
    })
}

fn parse_policy_id(raw: &str, location: RuntimeConfigLocation) -> Result<EvmSourcePolicyId> {
    EvmSourcePolicyId::new(raw).map_err(|_| {
        RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::PolicyId,
            },
        )
    })
}

fn resolve_required_value(
    location: RuntimeConfigLocation,
    field: &'static str,
    direct: &Option<String>,
    env_name: &Option<String>,
    file_path: &Option<String>,
    file_env: &Option<String>,
) -> Result<RuntimeSecretValue> {
    let location = location.with_field(field);
    let source = select_value_source(
        location.clone(),
        true,
        direct,
        env_name,
        file_path,
        file_env,
    )?
    .expect("required value source returns Some");
    resolve_selected_value(location, source)
}

fn resolve_optional_value(
    location: RuntimeConfigLocation,
    field: &'static str,
    direct: &Option<String>,
    env_name: &Option<String>,
    file_path: &Option<String>,
    file_env: &Option<String>,
) -> Result<Option<RuntimeSecretValue>> {
    let location = location.with_field(field);
    let Some(source) = select_value_source(
        location.clone(),
        false,
        direct,
        env_name,
        file_path,
        file_env,
    )?
    else {
        return Ok(None);
    };
    Ok(Some(resolve_selected_value(location, source)?))
}

fn resolve_required_path(
    location: RuntimeConfigLocation,
    field: &'static str,
    direct: &Option<String>,
    env_name: &Option<String>,
    file_path: &Option<String>,
    file_env: &Option<String>,
) -> Result<RuntimeSecretPath> {
    let value = resolve_required_value(location, field, direct, env_name, file_path, file_env)?;
    Ok(RuntimeSecretPath {
        path: PathBuf::from(value.value),
        source_kind: value.source_kind,
    })
}

enum SelectedValueSource<'a> {
    Direct(&'a str),
    Env(&'a str),
    File(&'a str),
    FileEnv(&'a str),
}

impl SelectedValueSource<'_> {
    const fn kind(&self) -> RuntimeValueSourceKind {
        match self {
            Self::Direct(_) => RuntimeValueSourceKind::Direct,
            Self::Env(_) => RuntimeValueSourceKind::Env,
            Self::File(_) => RuntimeValueSourceKind::File,
            Self::FileEnv(_) => RuntimeValueSourceKind::FileEnv,
        }
    }
}

fn select_value_source<'a>(
    location: RuntimeConfigLocation,
    required: bool,
    direct: &'a Option<String>,
    env_name: &'a Option<String>,
    file_path: &'a Option<String>,
    file_env: &'a Option<String>,
) -> Result<Option<SelectedValueSource<'a>>> {
    let mut selected = Vec::new();
    if let Some(value) = direct.as_deref() {
        selected.push(SelectedValueSource::Direct(value));
    }
    if let Some(value) = env_name.as_deref() {
        selected.push(SelectedValueSource::Env(value));
    }
    if let Some(value) = file_path.as_deref() {
        selected.push(SelectedValueSource::File(value));
    }
    if let Some(value) = file_env.as_deref() {
        selected.push(SelectedValueSource::FileEnv(value));
    }

    match selected.len() {
        0 if required => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        )),
        0 => Ok(None),
        1 => Ok(selected.into_iter().next()),
        _ if required => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        )),
        _ => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::AtMostOneValueSource,
        )),
    }
}

fn resolve_selected_value(
    location: RuntimeConfigLocation,
    source: SelectedValueSource<'_>,
) -> Result<RuntimeSecretValue> {
    let source_kind = source.kind();
    let value = match source {
        SelectedValueSource::Direct(value) => value.to_owned(),
        SelectedValueSource::Env(name) => {
            let name = parse_env_name(name, location.clone())?;
            read_env_value(name.as_str(), location.clone())?
        }
        SelectedValueSource::File(path) => read_indirection_file(path, location.clone())?,
        SelectedValueSource::FileEnv(name) => {
            let name = parse_env_name(name, location.clone())?;
            let path = read_env_value(name.as_str(), location.clone())?;
            read_indirection_file(path.trim(), location.clone())?
        }
    };
    let value = match source_kind {
        RuntimeValueSourceKind::File | RuntimeValueSourceKind::FileEnv => value.trim().to_owned(),
        RuntimeValueSourceKind::Direct | RuntimeValueSourceKind::Env => value,
    };
    if value.trim().is_empty() {
        return Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::EmptyResolvedValue,
        ));
    }
    Ok(RuntimeSecretValue { value, source_kind })
}

fn parse_env_name(name: &str, location: RuntimeConfigLocation) -> Result<RuntimeEnvName> {
    RuntimeEnvName::new(name).map_err(|_| {
        RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::EnvName,
            },
        )
    })
}

fn read_env_value(name: &str, location: RuntimeConfigLocation) -> Result<String> {
    match env::var(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::MissingEnv,
        )),
        Err(env::VarError::NotUnicode(_)) => Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::InvalidEnv,
        )),
    }
}

fn read_indirection_file(path: &str, location: RuntimeConfigLocation) -> Result<String> {
    if path.trim().is_empty() {
        return Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::EmptyResolvedValue,
        ));
    }
    fs::read_to_string(path)
        .map_err(|_| RuntimeConfigError::new(location, RuntimeConfigErrorKind::IndirectionFileRead))
}

fn validate_rpc_url(value: &RuntimeSecretValue, location: RuntimeConfigLocation) -> Result<()> {
    let url = url::Url::parse(value.expose_secret()).map_err(|_| {
        RuntimeConfigError::new(location.clone(), RuntimeConfigErrorKind::InvalidUrl)
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RuntimeConfigError::new(
            location,
            RuntimeConfigErrorKind::UrlUserInfo,
        ));
    }
    Ok(())
}
