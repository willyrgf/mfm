use std::collections::BTreeMap;

use bs58;
use mfm_evm_core::encoding::normalize_address;
use mfm_program_derive::MfmValue;
use mfm_values::string_map_secret_marker_key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::ids::{NetworkId, PortfolioScalarError, SymbolId, WalletId};

/// Canonical subject kind selected for one wallet declaration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-subject-kind",
    schema = "mfm.portfolio.wallet_subject_kind"
)]
pub enum WalletSubjectKind {
    /// Wallet resolves to an EVM address subject.
    #[default]
    EvmAddress,
    /// Wallet resolves to a Bitcoin address subject.
    BitcoinAddress,
}

impl mfm_values::MfmDefault for WalletSubjectKind {}

/// Canonical wallet config referenced by portfolio configs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-config",
    schema = "mfm.portfolio.wallet_config"
)]
pub struct WalletConfig {
    /// Stable machine identifier for the wallet.
    pub wallet_id: WalletId,
    /// Canonical wallet address.
    pub address: String,
    /// Subject family resolved for this wallet declaration.
    #[serde(default)]
    pub subject_kind: WalletSubjectKind,
    /// Stable network identifier.
    pub network_id: NetworkId,
    /// Wallet implementation selection.
    pub implementation: WalletImplementationConfig,
    /// Symbols to read for this wallet.
    pub symbol_ids: Vec<SymbolId>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl WalletConfig {
    /// Creates a normalized and validated wallet config.
    pub fn new(
        wallet_id: String,
        address: String,
        subject_kind: WalletSubjectKind,
        network_id: String,
        implementation: WalletImplementationConfig,
        symbol_ids: Vec<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, WalletConfigError> {
        let wallet_id = WalletId::new(wallet_id)
            .map_err(|source| WalletConfigError::InvalidWalletId { source })?;
        let network_id = NetworkId::new(network_id)
            .map_err(|source| WalletConfigError::InvalidNetworkId { source })?;
        let symbol_ids = symbol_ids
            .into_iter()
            .map(|symbol_id| {
                SymbolId::new(symbol_id.clone())
                    .map_err(|source| WalletConfigError::InvalidSymbolIdRef { symbol_id, source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self {
            wallet_id,
            address,
            subject_kind,
            network_id,
            implementation,
            symbol_ids,
            metadata,
        }
        .validated()
    }

    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.symbol_ids.sort();
    }

    /// Returns a normalized clone of the wallet config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Validates this wallet config, normalizes it, and returns the validated value.
    pub fn validated(mut self) -> Result<Self, WalletConfigError> {
        validate_wallet_config(&self)?;
        self.normalize();
        Ok(self)
    }
}

/// Canonical wallet implementation selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-implementation-config",
    schema = "mfm.portfolio.wallet_implementation_config"
)]
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
        account_index: u64,
    },
    /// External signer resolved by id.
    ExternalSigner {
        /// Stable signer identifier.
        signer_id: String,
    },
}

/// Wallet signer details selected by runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-signer-config",
    schema = "mfm.portfolio.wallet_signer_config"
)]
pub struct WalletSignerConfig {
    /// Canonical signer kind.
    pub signer_kind: String,
    /// Opaque runtime signer reference.
    pub signer_ref: String,
}

/// Runtime wallet capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-capabilities",
    schema = "mfm.portfolio.wallet_capabilities"
)]
pub struct WalletCapabilities {
    /// Whether the implementation can resolve an address.
    pub can_resolve_address: bool,
    /// Whether the implementation can sign payloads.
    pub can_sign: bool,
    /// Whether the implementation can submit transactions.
    pub can_submit: bool,
}

/// Resolved wallet after runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-wallet",
    schema = "mfm.portfolio.resolved_wallet"
)]
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
    /// `wallet_id` did not satisfy the portfolio identifier grammar.
    #[error("wallet_id is invalid: {source}")]
    InvalidWalletId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `address` was empty.
    #[error("address must be non-empty")]
    EmptyAddress,
    /// `address` was invalid for the selected subject kind.
    #[error("address must be valid for subject_kind `{subject_kind:?}`: {address}")]
    InvalidAddress {
        /// Invalid wallet address input.
        address: String,
        /// Subject kind that rejected the address.
        subject_kind: WalletSubjectKind,
    },
    /// `network_id` did not satisfy the portfolio identifier grammar.
    #[error("network_id is invalid: {source}")]
    InvalidNetworkId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// One of the symbol refs did not satisfy the portfolio identifier grammar.
    #[error("wallet symbol_id ref `{symbol_id}` is invalid: {source}")]
    InvalidSymbolIdRef {
        /// Rejected symbol id.
        symbol_id: String,
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `entry_id` was empty for a keystore-backed wallet.
    #[error("keystore entry_id must be non-empty")]
    EmptyKeystoreEntryId,
    /// `signer_id` was empty for an external signer wallet.
    #[error("external signer_id must be non-empty")]
    EmptyExternalSignerId,
    /// Wallet metadata contained a secret-shaped key or value.
    #[error("metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
    },
}

/// Decodes and validates a canonical wallet config.
pub fn decode_wallet_config(value: &Value) -> Result<WalletConfig, WalletConfigError> {
    let cfg: WalletConfig = serde_json::from_value(value.clone())
        .map_err(|err| WalletConfigError::Decode(err.to_string()))?;
    cfg.validated()
}

/// Validates a canonical wallet config.
pub fn validate_wallet_config(cfg: &WalletConfig) -> Result<(), WalletConfigError> {
    if cfg.address.trim().is_empty() {
        return Err(WalletConfigError::EmptyAddress);
    }
    match cfg.subject_kind {
        WalletSubjectKind::EvmAddress => {
            let normalized =
                normalize_address(&cfg.address).map_err(|_| WalletConfigError::InvalidAddress {
                    address: cfg.address.clone(),
                    subject_kind: cfg.subject_kind,
                })?;
            if normalized != cfg.address {
                return Err(WalletConfigError::InvalidAddress {
                    address: cfg.address.clone(),
                    subject_kind: cfg.subject_kind,
                });
            }
        }
        WalletSubjectKind::BitcoinAddress => {
            validate_bitcoin_address(&cfg.address).map_err(|_| {
                WalletConfigError::InvalidAddress {
                    address: cfg.address.clone(),
                    subject_kind: cfg.subject_kind,
                }
            })?;
        }
    }
    if let Some(key) = string_map_secret_marker_key(&cfg.metadata) {
        return Err(WalletConfigError::MetadataContainsSecret {
            key: key.to_string(),
        });
    }
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

fn validate_bitcoin_address(raw: &str) -> Result<(), String> {
    if raw.trim() != raw {
        return Err("address must not contain surrounding whitespace".to_string());
    }
    if raw.len() < 14 || raw.len() > 90 {
        return Err("address length was outside the supported bitcoin envelope".to_string());
    }
    if !raw.is_ascii() {
        return Err("address must be ASCII".to_string());
    }

    let base58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let bech32 = "023456789acdefghjklmnpqrstuvwxyz";
    if raw.starts_with("bc1") || raw.starts_with("tb1") || raw.starts_with("bcrt1") {
        if raw != raw.to_ascii_lowercase() {
            return Err("bech32 bitcoin addresses must already be lowercase".to_string());
        }
        let payload = raw
            .split_once('1')
            .map(|(_, payload)| payload)
            .ok_or_else(|| {
                "bech32 bitcoin address is missing the separator character".to_string()
            })?;
        if payload.is_empty() || !payload.chars().all(|ch| bech32.contains(ch)) {
            return Err("bech32 bitcoin address contained unsupported characters".to_string());
        }
        return Ok(());
    }

    let first = raw.chars().next().unwrap_or_default();
    if !matches!(first, '1' | '3' | '2' | 'm' | 'n') {
        return Err("unsupported bitcoin address prefix".to_string());
    }
    if !raw.chars().all(|ch| base58.contains(ch)) {
        return Err("base58 bitcoin address contained unsupported characters".to_string());
    }
    bs58::decode(raw)
        .with_check(None)
        .into_vec()
        .map_err(|_| "base58 bitcoin address checksum validation failed".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_bitcoin_address;

    #[test]
    fn validate_bitcoin_address_rejects_bad_base58_checksum() {
        assert!(validate_bitcoin_address("1BoatSLRHtKNngkdXEeobR76b53LETtpyT").is_ok());
        assert!(
            validate_bitcoin_address("1BoatSLRHtKNngkdXEeobR76b53LETtpyY").is_err(),
            "checksum mismatch should be rejected"
        );
    }
}
