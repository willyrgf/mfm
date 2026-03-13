use std::collections::BTreeMap;

use mfm_evm_core::encoding::normalize_address;
use mfm_machine::hashing::{canonical_json_bytes, CanonicalJsonError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Canonical wallet config referenced by portfolio configs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WalletConfig {
    /// Stable machine identifier for the wallet.
    pub wallet_id: String,
    /// Canonical wallet address.
    pub address: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Wallet implementation selection.
    pub implementation: WalletImplementationConfig,
    /// Symbols to read for this wallet.
    pub symbol_ids: Vec<String>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl WalletConfig {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.symbol_ids.sort();
    }

    /// Returns a normalized clone of the wallet config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }
}

/// Canonical wallet implementation selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WalletImplementationConfig {
    /// Read-only address supplied directly by config.
    AddressOnly {},
    /// Address/signer backed by a keystore entry.
    KeystoreEntry {
        /// Stable keystore entry identifier.
        entry_id: String,
    },
    /// Account managed by the node.
    NodeManagedAccount {
        /// Node account index.
        account_index: usize,
    },
    /// External signer resolved by id.
    ExternalSigner {
        /// Stable signer identifier.
        signer_id: String,
    },
}

/// Wallet signer details selected by runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSignerConfig {
    /// Canonical signer kind.
    pub signer_kind: String,
    /// Opaque runtime signer reference.
    pub signer_ref: String,
}

/// Runtime wallet capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletCapabilities {
    /// Whether the implementation can resolve an address.
    pub can_resolve_address: bool,
    /// Whether the implementation can sign payloads.
    pub can_sign: bool,
    /// Whether the implementation can submit transactions.
    pub can_submit: bool,
}

/// Resolved wallet after runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedWallet {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Canonical resolved address.
    pub address: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Canonical implementation kind string.
    pub implementation_kind: String,
    /// Runtime capabilities.
    pub capabilities: WalletCapabilities,
    /// Optional signer details.
    pub signer: Option<WalletSignerConfig>,
}

/// Validation errors for canonical wallet configs.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WalletConfigError {
    /// The JSON payload could not be decoded into the canonical type.
    #[error("wallet config decode failed: {0}")]
    Decode(String),
    /// `wallet_id` was empty.
    #[error("wallet_id must be non-empty")]
    EmptyWalletId,
    /// `address` was empty.
    #[error("address must be non-empty")]
    EmptyAddress,
    /// `address` was invalid or not normalized.
    #[error("address must be a normalized EVM address: {address}")]
    InvalidAddress {
        /// Invalid wallet address input.
        address: String,
    },
    /// `network_id` was empty.
    #[error("network_id must be non-empty")]
    EmptyNetworkId,
    /// One of the symbol refs was empty.
    #[error("wallet symbol_ids must not contain empty entries")]
    EmptySymbolIdRef,
    /// `entry_id` was empty for a keystore-backed wallet.
    #[error("keystore entry_id must be non-empty")]
    EmptyKeystoreEntryId,
    /// `signer_id` was empty for an external signer wallet.
    #[error("external signer_id must be non-empty")]
    EmptyExternalSignerId,
    /// Wallet metadata violated canonical JSON rules.
    #[error("metadata must be canonical JSON: {reason}")]
    MetadataNotCanonical {
        /// Underlying canonical JSON failure.
        reason: CanonicalJsonError,
    },
}

/// Decodes and validates a canonical wallet config.
pub fn decode_wallet_config(value: &Value) -> Result<WalletConfig, WalletConfigError> {
    let cfg = serde_json::from_value(value.clone())
        .map_err(|err| WalletConfigError::Decode(err.to_string()))?;
    validate_wallet_config(&cfg)?;
    Ok(cfg)
}

/// Validates a canonical wallet config.
pub fn validate_wallet_config(cfg: &WalletConfig) -> Result<(), WalletConfigError> {
    if cfg.wallet_id.trim().is_empty() {
        return Err(WalletConfigError::EmptyWalletId);
    }
    if cfg.address.trim().is_empty() {
        return Err(WalletConfigError::EmptyAddress);
    }
    let normalized =
        normalize_address(&cfg.address).map_err(|_| WalletConfigError::InvalidAddress {
            address: cfg.address.clone(),
        })?;
    if normalized != cfg.address {
        return Err(WalletConfigError::InvalidAddress {
            address: cfg.address.clone(),
        });
    }
    if cfg.network_id.trim().is_empty() {
        return Err(WalletConfigError::EmptyNetworkId);
    }
    if cfg
        .symbol_ids
        .iter()
        .any(|symbol_id| symbol_id.trim().is_empty())
    {
        return Err(WalletConfigError::EmptySymbolIdRef);
    }
    validate_canonical_json_map(&cfg.metadata)
        .map_err(|reason| WalletConfigError::MetadataNotCanonical { reason })?;

    match &cfg.implementation {
        WalletImplementationConfig::AddressOnly {} => {}
        WalletImplementationConfig::KeystoreEntry { entry_id } => {
            if entry_id.trim().is_empty() {
                return Err(WalletConfigError::EmptyKeystoreEntryId);
            }
        }
        WalletImplementationConfig::NodeManagedAccount { .. } => {}
        WalletImplementationConfig::ExternalSigner { signer_id } => {
            if signer_id.trim().is_empty() {
                return Err(WalletConfigError::EmptyExternalSignerId);
            }
        }
    }

    Ok(())
}

fn validate_canonical_json_map(map: &BTreeMap<String, Value>) -> Result<(), CanonicalJsonError> {
    canonical_json_bytes(&json_object_value(map)).map(|_| ())
}

fn json_object_value(map: &BTreeMap<String, Value>) -> Value {
    Value::Object(
        map.iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}
