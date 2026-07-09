use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::num::NonZeroU64;

use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{
    ControlScopeId, NetworkId, PortfolioId, PortfolioScalarError, SymbolId, ValuationSourceId,
    WalletId,
};
use crate::metadata::PublicMetadata;
use crate::symbol::{
    validate_symbol_config, BalanceReaderConfig, Observation, PriceSourceRef, QuoteCode,
    SymbolConfig, SymbolConfigError, SymbolKind, ValuationReaderConfig, ValuationSourceConfig,
    ValuationSourceRegistry, ValuationSourceRegistryError,
};
use crate::wallet::{WalletConfig, WalletSubjectKind};

/// Supported network families on the canonical portfolio config surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "network-family-config",
    schema = "mfm.portfolio.network_family_config"
)]
pub enum NetworkFamilyConfig {
    /// Ethereum-compatible execution network.
    #[default]
    Evm,
    /// Bitcoin-family execution network.
    Bitcoin,
}

impl mfm_values::MfmDefault for NetworkFamilyConfig {}

/// Canonical top-level portfolio configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-config",
    schema = "mfm.portfolio.config",
    validate = "validate_portfolio_config_for_mfm"
)]
pub struct PortfolioConfig {
    /// Stable machine identifier for the portfolio.
    pub portfolio_id: PortfolioId,
    /// Quote units required for every configured symbol.
    pub quote_codes: Vec<QuoteCode>,
    /// Networks available to the portfolio.
    pub networks: Vec<NetworkConfig>,
    /// Wallets included in the portfolio.
    pub wallets: Vec<WalletConfig>,
    /// Symbol configs available to the portfolio.
    pub symbol_configs: Vec<SymbolConfig>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl PortfolioConfig {
    /// Creates a normalized and validated portfolio config.
    pub fn new(
        portfolio_id: String,
        quote_codes: Vec<QuoteCode>,
        networks: Vec<NetworkConfig>,
        wallets: Vec<WalletConfig>,
        symbol_configs: Vec<SymbolConfig>,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, PortfolioConfigError> {
        let portfolio_id = PortfolioId::new(portfolio_id)
            .map_err(|source| PortfolioConfigError::InvalidPortfolioId { source })?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            PortfolioConfigError::MetadataContainsSecret {
                key: source.key().to_owned(),
            }
        })?;
        Self {
            portfolio_id,
            quote_codes,
            networks,
            wallets,
            symbol_configs,
            metadata,
        }
        .validated()
    }

    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.quote_codes.sort();
        self.networks
            .sort_by(|left, right| left.network_id().cmp(right.network_id()));
        for wallet in &mut self.wallets {
            wallet.normalize();
        }
        self.wallets
            .sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
        for symbol in &mut self.symbol_configs {
            symbol.normalize();
        }
        self.symbol_configs
            .sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));
    }

    /// Returns a normalized clone of the portfolio config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Validates this config, normalizes it, and returns the validated value.
    pub fn validated(self) -> Result<Self, PortfolioConfigError> {
        Ok(ValidatedPortfolioConfig::new(self)?.into_config())
    }
}

/// Canonical network configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "family", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "network-config",
    schema = "mfm.portfolio.network_config"
)]
pub enum NetworkConfig {
    /// Ethereum-compatible execution network.
    Evm {
        /// Stable machine identifier for the network.
        network_id: NetworkId,
        /// Non-zero EVM chain id for the network.
        chain_id: NonZeroU64,
        /// Stable control-plane scope used for managed rpc.control reads on this network.
        control_scope: ControlScopeId,
        /// Canonical metadata surface.
        #[serde(default)]
        metadata: PublicMetadata,
    },
    /// Bitcoin-family execution network.
    Bitcoin {
        /// Stable machine identifier for the network.
        network_id: NetworkId,
        /// Expected Bitcoin Core network tag (`main`, `test`, `signet`, or `regtest`).
        bitcoin_network: String,
        /// Stable control-plane scope used for managed rpc.control reads on this network.
        control_scope: ControlScopeId,
        /// Canonical metadata surface.
        #[serde(default)]
        metadata: PublicMetadata,
    },
}

impl NetworkConfig {
    /// Creates a validated network config.
    pub fn new(
        network_id: String,
        family: NetworkFamilyConfig,
        chain_id: Option<u64>,
        bitcoin_network: Option<String>,
        control_scope: String,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, PortfolioConfigError> {
        let network_id = NetworkId::new(network_id)
            .map_err(|source| PortfolioConfigError::InvalidNetworkId { source })?;
        let control_scope = ControlScopeId::new(control_scope)
            .map_err(|source| PortfolioConfigError::InvalidNetworkControlScope { source })?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            PortfolioConfigError::NetworkMetadataContainsSecret {
                network_id: network_id.to_string(),
                key: source.key().to_owned(),
            }
        })?;
        match family {
            NetworkFamilyConfig::Evm => {
                if bitcoin_network.is_some() {
                    return Err(PortfolioConfigError::UnexpectedEvmBitcoinNetwork {
                        network_id: network_id.to_string(),
                    });
                }
                let Some(chain_id) = chain_id else {
                    return Err(PortfolioConfigError::MissingEvmChainId {
                        network_id: network_id.to_string(),
                    });
                };
                let Some(chain_id) = NonZeroU64::new(chain_id) else {
                    return Err(PortfolioConfigError::InvalidEvmChainId {
                        network_id: network_id.to_string(),
                    });
                };
                Self::Evm {
                    network_id,
                    chain_id,
                    control_scope,
                    metadata,
                }
                .validated()
            }
            NetworkFamilyConfig::Bitcoin => {
                if chain_id.is_some() {
                    return Err(PortfolioConfigError::UnexpectedBitcoinChainId {
                        network_id: network_id.to_string(),
                    });
                }
                let Some(bitcoin_network) = bitcoin_network else {
                    return Err(PortfolioConfigError::MissingBitcoinNetwork {
                        network_id: network_id.to_string(),
                    });
                };
                validate_bitcoin_network(&bitcoin_network).map_err(|_| {
                    PortfolioConfigError::InvalidBitcoinNetwork {
                        network_id: network_id.to_string(),
                    }
                })?;
                Self::Bitcoin {
                    network_id,
                    bitcoin_network,
                    control_scope,
                    metadata,
                }
                .validated()
            }
        }
    }

    /// Validates this network config and returns it unchanged.
    pub fn validated(self) -> Result<Self, PortfolioConfigError> {
        Ok(self)
    }

    /// Returns the stable machine identifier for this network.
    pub fn network_id(&self) -> &NetworkId {
        match self {
            Self::Evm { network_id, .. } | Self::Bitcoin { network_id, .. } => network_id,
        }
    }

    /// Returns the execution family for this network.
    pub const fn family(&self) -> NetworkFamilyConfig {
        match self {
            Self::Evm { .. } => NetworkFamilyConfig::Evm,
            Self::Bitcoin { .. } => NetworkFamilyConfig::Bitcoin,
        }
    }

    /// Returns the EVM chain id when this is an EVM network.
    pub const fn chain_id(&self) -> Option<NonZeroU64> {
        match self {
            Self::Evm { chain_id, .. } => Some(*chain_id),
            Self::Bitcoin { .. } => None,
        }
    }

    /// Returns the EVM chain id as a primitive integer when present.
    pub const fn chain_id_u64(&self) -> Option<u64> {
        match self {
            Self::Evm { chain_id, .. } => Some(chain_id.get()),
            Self::Bitcoin { .. } => None,
        }
    }

    /// Returns the Bitcoin Core network tag when this is a Bitcoin network.
    pub fn bitcoin_network(&self) -> Option<&str> {
        match self {
            Self::Evm { .. } => None,
            Self::Bitcoin {
                bitcoin_network, ..
            } => Some(bitcoin_network),
        }
    }

    /// Returns the stable control-plane scope for managed reads on this network.
    pub fn control_scope(&self) -> &ControlScopeId {
        match self {
            Self::Evm { control_scope, .. } | Self::Bitcoin { control_scope, .. } => control_scope,
        }
    }

    /// Returns the public metadata associated with this network.
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        match self {
            Self::Evm { metadata, .. } | Self::Bitcoin { metadata, .. } => metadata.as_map(),
        }
    }

    /// Returns the checked public metadata authority associated with this network.
    pub fn public_metadata(&self) -> &PublicMetadata {
        match self {
            Self::Evm { metadata, .. } | Self::Bitcoin { metadata, .. } => metadata,
        }
    }
}

/// Validated network config collection authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedNetworkConfigs {
    networks: Vec<NetworkConfig>,
}

impl ValidatedNetworkConfigs {
    /// Creates a validated network config collection authority.
    pub fn new(networks: Vec<NetworkConfig>) -> Result<Self, PortfolioConfigError> {
        let mut seen = BTreeSet::new();
        for network in &networks {
            if !seen.insert(network.network_id().as_str()) {
                return Err(PortfolioConfigError::DuplicateNetworkId {
                    network_id: network.network_id().to_string(),
                });
            }
        }
        Ok(Self { networks })
    }

    /// Returns the validated network configs.
    pub fn as_slice(&self) -> &[NetworkConfig] {
        &self.networks
    }

    /// Consumes this authority into the validated network configs.
    pub fn into_vec(self) -> Vec<NetworkConfig> {
        self.networks
    }
}

/// Validated wallet config collection authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedWalletConfigs {
    wallets: Vec<WalletConfig>,
}

impl ValidatedWalletConfigs {
    /// Creates a validated wallet config collection authority.
    pub fn new(wallets: Vec<WalletConfig>) -> Result<Self, PortfolioConfigError> {
        let mut seen = BTreeSet::new();
        for wallet in &wallets {
            if !seen.insert(wallet.wallet_id.as_str()) {
                return Err(PortfolioConfigError::DuplicateWalletId {
                    wallet_id: wallet.wallet_id.to_string(),
                });
            }
        }
        Ok(Self { wallets })
    }

    /// Returns the validated wallet configs.
    pub fn as_slice(&self) -> &[WalletConfig] {
        &self.wallets
    }

    /// Consumes this authority into the validated wallet configs.
    pub fn into_vec(self) -> Vec<WalletConfig> {
        self.wallets
    }
}

/// Validated symbol config collection authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedSymbolConfigs {
    symbols: Vec<SymbolConfig>,
}

impl ValidatedSymbolConfigs {
    /// Creates a validated symbol config collection authority.
    pub fn new(symbols: Vec<SymbolConfig>) -> Result<Self, PortfolioConfigError> {
        let mut seen = BTreeSet::new();
        for symbol in &symbols {
            validate_symbol_config(symbol).map_err(|source| {
                PortfolioConfigError::InvalidSymbolConfig {
                    symbol_id: symbol.symbol_id.to_string(),
                    source: Box::new(source),
                }
            })?;
            if !seen.insert(symbol.symbol_id.as_str()) {
                return Err(PortfolioConfigError::DuplicateSymbolId {
                    symbol_id: symbol.symbol_id.to_string(),
                });
            }
        }
        Ok(Self { symbols })
    }

    /// Returns the validated symbol configs.
    pub fn as_slice(&self) -> &[SymbolConfig] {
        &self.symbols
    }

    /// Consumes this authority into the validated symbol configs.
    pub fn into_vec(self) -> Vec<SymbolConfig> {
        self.symbols
    }
}

/// Validated, normalized portfolio config authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedPortfolioConfig {
    config: PortfolioConfig,
    index: PortfolioConfigIndex,
}

impl ValidatedPortfolioConfig {
    /// Creates a validated portfolio config authority.
    pub fn new(config: PortfolioConfig) -> Result<Self, PortfolioConfigError> {
        validate_portfolio_config_inner(&config)?;
        let mut config = config;
        config.normalize();
        let index = PortfolioConfigIndex::new(&config);
        Ok(Self { config, index })
    }

    /// Returns the normalized portfolio config.
    pub const fn as_config(&self) -> &PortfolioConfig {
        &self.config
    }

    /// Consumes this authority into the normalized portfolio config.
    pub fn into_config(self) -> PortfolioConfig {
        self.config
    }

    /// Returns a configured quote code when it exists.
    pub fn quote(&self, quote: QuoteCode) -> Option<QuoteCode> {
        self.index.quotes.contains(&quote).then_some(quote)
    }

    /// Returns a network by stable id.
    pub fn network(&self, network_id: &str) -> Option<&NetworkConfig> {
        let network_id = NetworkId::new(network_id).ok()?;
        self.index
            .networks
            .get(&network_id)
            .map(|index| &self.config.networks[*index])
    }

    /// Returns a wallet by stable id.
    pub fn wallet(&self, wallet_id: &str) -> Option<&WalletConfig> {
        let wallet_id = WalletId::new(wallet_id).ok()?;
        self.index
            .wallets
            .get(&wallet_id)
            .map(|index| &self.config.wallets[*index])
    }

    /// Returns a symbol by stable id.
    pub fn symbol(&self, symbol_id: &str) -> Option<&SymbolConfig> {
        let symbol_id = SymbolId::new(symbol_id).ok()?;
        self.index
            .symbols
            .get(&symbol_id)
            .map(|index| &self.config.symbol_configs[*index])
    }
}

/// Validated portfolio plus valuation-source registry authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedPortfolioBundle {
    portfolio: ValidatedPortfolioConfig,
    valuation_source_registry: ValuationSourceRegistry,
    source_index: BTreeMap<ValuationSourceId, usize>,
}

impl ValidatedPortfolioBundle {
    /// Creates a validated portfolio bundle authority.
    pub fn new(
        portfolio: PortfolioConfig,
        valuation_source_registry: ValuationSourceRegistry,
    ) -> Result<Self, PortfolioConfigError> {
        let portfolio = ValidatedPortfolioConfig::new(portfolio)?;
        let valuation_source_registry = valuation_source_registry
            .validated()
            .map_err(|source| PortfolioConfigError::InvalidValuationSourceRegistry { source })?;
        let source_index = valuation_source_index(&valuation_source_registry);
        validate_portfolio_bundle_sources(
            portfolio.as_config(),
            &valuation_source_registry,
            &source_index,
        )?;
        Ok(Self {
            portfolio,
            valuation_source_registry,
            source_index,
        })
    }

    /// Returns the validated portfolio config authority.
    pub const fn portfolio(&self) -> &ValidatedPortfolioConfig {
        &self.portfolio
    }

    /// Returns the normalized valuation source registry.
    pub const fn valuation_source_registry(&self) -> &ValuationSourceRegistry {
        &self.valuation_source_registry
    }

    /// Returns a valuation source by stable id.
    pub fn valuation_source(&self, source_id: &str) -> Option<&ValuationSourceConfig> {
        let source_id = ValuationSourceId::new(source_id).ok()?;
        self.source_index
            .get(&source_id)
            .map(|index| &self.valuation_source_registry.sources[*index])
    }

    /// Consumes this authority into normalized portfolio and registry parts.
    pub fn into_parts(self) -> (PortfolioConfig, ValuationSourceRegistry) {
        (self.portfolio.into_config(), self.valuation_source_registry)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PortfolioConfigIndex {
    quotes: BTreeSet<QuoteCode>,
    networks: BTreeMap<NetworkId, usize>,
    wallets: BTreeMap<WalletId, usize>,
    symbols: BTreeMap<SymbolId, usize>,
}

impl PortfolioConfigIndex {
    fn new(config: &PortfolioConfig) -> Self {
        let quotes = config.quote_codes.iter().copied().collect();
        let networks = config
            .networks
            .iter()
            .enumerate()
            .map(|(index, network)| (network.network_id().clone(), index))
            .collect();
        let wallets = config
            .wallets
            .iter()
            .enumerate()
            .map(|(index, wallet)| (wallet.wallet_id.clone(), index))
            .collect();
        let symbols = config
            .symbol_configs
            .iter()
            .enumerate()
            .map(|(index, symbol)| (symbol.symbol_id.clone(), index))
            .collect();
        Self {
            quotes,
            networks,
            wallets,
            symbols,
        }
    }
}

/// Concrete execution anchor captured for one pinned network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "family", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "execution-anchor",
    schema = "mfm.portfolio.execution_anchor"
)]
pub enum ExecutionAnchor {
    /// EVM execution pinned to one block hash and number on one chain id.
    Evm {
        /// EVM chain id.
        chain_id: u64,
        /// Concrete pinned block number.
        block_number: u64,
        /// Concrete pinned block hash.
        block_hash: String,
    },
    /// Bitcoin execution pinned to one height and block hash.
    Bitcoin {
        /// Concrete pinned block height.
        height: u64,
        /// Concrete pinned block hash.
        block_hash: String,
    },
}

/// Concrete pinned network view captured in a snapshot/report artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "network-pin",
    schema = "mfm.portfolio.network_pin"
)]
pub struct NetworkPin {
    /// Stable network identifier.
    pub network_id: String,
    /// Concrete pinned execution anchor.
    pub anchor: ExecutionAnchor,
}

/// Canonical snapshot for one wallet inside a portfolio snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-snapshot",
    schema = "mfm.portfolio.wallet_snapshot"
)]
pub struct WalletSnapshot {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Canonical wallet address.
    pub address: String,
    /// Canonical wallet subject kind.
    pub subject_kind: WalletSubjectKind,
    /// Stable network identifier.
    pub network_id: String,
    /// Observations collected for the wallet.
    pub observations: Vec<Observation>,
}

impl WalletSnapshot {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        for observation in &mut self.observations {
            observation.normalize();
        }
        self.observations.sort_by(|left, right| {
            (left.wallet_id.as_str(), left.symbol_id.as_str())
                .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
        });
    }
}

/// Canonical portfolio snapshot artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue, PublicOutputs)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-snapshot",
    schema = "mfm.portfolio.snapshot"
)]
pub struct PortfolioSnapshot {
    /// Version of the public snapshot schema.
    pub schema_version: u64,
    /// Stable portfolio identifier.
    pub portfolio_id: String,
    /// Generation timestamp in milliseconds since epoch.
    pub generated_at_ms: u64,
    /// One pin per referenced network.
    pub network_pins: Vec<NetworkPin>,
    /// Per-wallet observations.
    pub wallets: Vec<WalletSnapshot>,
    /// Symbol configs used to interpret the snapshot.
    pub symbol_configs: Vec<SymbolConfig>,
    /// Snapshot errors.
    pub errors: Vec<PortfolioSnapshotError>,
}

impl PortfolioSnapshot {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.network_pins
            .sort_by(|left, right| left.network_id.cmp(&right.network_id));
        for wallet in &mut self.wallets {
            wallet.normalize();
        }
        self.wallets
            .sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
        for symbol in &mut self.symbol_configs {
            symbol.normalize();
        }
        self.symbol_configs
            .sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));
        self.errors.sort_by(|left, right| {
            (
                left.network_id.as_deref().unwrap_or(""),
                left.wallet_id.as_deref().unwrap_or(""),
                left.symbol_id.as_deref().unwrap_or(""),
                left.code.as_str(),
            )
                .cmp(&(
                    right.network_id.as_deref().unwrap_or(""),
                    right.wallet_id.as_deref().unwrap_or(""),
                    right.symbol_id.as_deref().unwrap_or(""),
                    right.code.as_str(),
                ))
        });
    }
}

/// Canonical snapshot error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-snapshot-error",
    schema = "mfm.portfolio.snapshot_error"
)]
pub struct PortfolioSnapshotError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional wallet identifier associated with the error.
    pub wallet_id: Option<String>,
    /// Optional symbol identifier associated with the error.
    pub symbol_id: Option<String>,
    /// Optional network identifier associated with the error.
    pub network_id: Option<String>,
    /// Optional reader kind associated with the error.
    pub reader_kind: Option<String>,
}

/// Canonical report derived from the snapshot artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, PublicOutputs)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-report",
    schema = "mfm.portfolio.report"
)]
pub struct PortfolioReport {
    /// Version of the public report schema.
    pub schema_version: u64,
    /// Stable portfolio identifier.
    pub portfolio_id: String,
    /// Generation timestamp in milliseconds since epoch.
    pub generated_at_ms: u64,
    /// One pin per referenced network.
    pub network_pins: Vec<NetworkPin>,
    /// Per-wallet quote summaries.
    pub wallet_summaries: Vec<WalletReport>,
    /// Portfolio-level quote totals.
    pub totals_by_quote: Vec<PortfolioQuoteTotal>,
    /// Total number of errors in the snapshot.
    pub error_count: u64,
}

impl PortfolioReport {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.network_pins
            .sort_by(|left, right| left.network_id.cmp(&right.network_id));
        for wallet in &mut self.wallet_summaries {
            wallet.normalize();
        }
        self.wallet_summaries
            .sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
        self.totals_by_quote.sort_by_key(|total| total.quote);
    }
}

/// Canonical per-wallet report summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "wallet-report",
    schema = "mfm.portfolio.wallet_report"
)]
pub struct WalletReport {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Totals by quote for the wallet.
    pub totals_by_quote: Vec<PortfolioQuoteTotal>,
}

impl WalletReport {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.totals_by_quote.sort_by_key(|total| total.quote);
    }
}

/// Canonical quote total used by wallet and portfolio reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "portfolio-quote-total",
    schema = "mfm.portfolio.quote_total"
)]
pub struct PortfolioQuoteTotal {
    /// Quote unit.
    pub quote: QuoteCode,
    /// Assets total in the quote unit.
    pub assets_value_dec: String,
    /// Collateral total in the quote unit.
    pub collateral_value_dec: String,
    /// Debt total in the quote unit.
    pub debt_value_dec: String,
    /// Staked total in the quote unit.
    pub staked_value_dec: String,
    /// Net total in the quote unit.
    pub net_value_dec: String,
}

/// Validation errors for canonical portfolio configs.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PortfolioConfigError {
    /// The JSON payload could not be decoded into the canonical type.
    #[error("portfolio config decode failed: {0}")]
    Decode(String),
    /// `portfolio_id` did not satisfy the portfolio identifier grammar.
    #[error("portfolio_id is invalid: {source}")]
    InvalidPortfolioId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// Portfolio metadata contained a secret-shaped key or value.
    #[error("portfolio metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// `network_id` did not satisfy the portfolio identifier grammar.
    #[error("network_id is invalid: {source}")]
    InvalidNetworkId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `control_scope` did not satisfy the portfolio identifier grammar.
    #[error("control_scope is invalid: {source}")]
    InvalidNetworkControlScope {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// EVM networks require an explicit chain id.
    #[error("network `{network_id}` with family `evm` must declare chain_id")]
    MissingEvmChainId {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// EVM networks require a non-zero chain id.
    #[error("network `{network_id}` with family `evm` must declare non-zero chain_id")]
    InvalidEvmChainId {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks must not declare an EVM chain id.
    #[error("network `{network_id}` with family `bitcoin` must not declare chain_id")]
    UnexpectedBitcoinChainId {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// EVM networks must not declare a Bitcoin network tag.
    #[error("network `{network_id}` with family `evm` must not declare bitcoin_network")]
    UnexpectedEvmBitcoinNetwork {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks require an explicit Bitcoin Core network tag.
    #[error("network `{network_id}` with family `bitcoin` must declare bitcoin_network")]
    MissingBitcoinNetwork {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Bitcoin networks require a supported Bitcoin Core network tag.
    #[error("network `{network_id}` with family `bitcoin` has invalid bitcoin_network")]
    InvalidBitcoinNetwork {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// Two networks shared the same network id.
    #[error("network_id `{network_id}` must be unique")]
    DuplicateNetworkId {
        /// Duplicate network id.
        network_id: String,
    },
    /// Network metadata contained a secret-shaped key or value.
    #[error("network `{network_id}` metadata key `{key}` contains secret-shaped content")]
    NetworkMetadataContainsSecret {
        /// Network associated with the rejected metadata.
        network_id: String,
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// Two wallets shared the same wallet id.
    #[error("wallet_id `{wallet_id}` must be unique")]
    DuplicateWalletId {
        /// Duplicate wallet id.
        wallet_id: String,
    },
    /// Wallet referenced an unknown network.
    #[error("wallet `{wallet_id}` referenced unknown network `{network_id}`")]
    UnknownWalletNetwork {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Unknown network id.
        network_id: String,
    },
    /// Wallet subject family did not match the referenced network family.
    #[error("wallet `{wallet_id}` subject_kind `{wallet_subject_kind:?}` did not match network `{network_id}` family `{network_family:?}`")]
    WalletSubjectNetworkFamilyMismatch {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Network id associated with the failure.
        network_id: String,
        /// Wallet subject kind selected in config.
        wallet_subject_kind: WalletSubjectKind,
        /// Network family selected by the referenced network.
        network_family: NetworkFamilyConfig,
    },
    /// Wallet referenced an unknown symbol.
    #[error("wallet `{wallet_id}` referenced unknown symbol `{symbol_id}`")]
    UnknownWalletSymbol {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Unknown symbol id.
        symbol_id: String,
    },
    /// Wallet referenced a symbol configured for a different network.
    #[error("wallet `{wallet_id}` on network `{wallet_network_id}` referenced symbol `{symbol_id}` on network `{symbol_network_id}`")]
    WalletSymbolNetworkMismatch {
        /// Wallet id associated with the failure.
        wallet_id: String,
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Wallet network id.
        wallet_network_id: String,
        /// Symbol network id.
        symbol_network_id: String,
    },
    /// Two symbols shared the same symbol id.
    #[error("symbol_id `{symbol_id}` must be unique")]
    DuplicateSymbolId {
        /// Duplicate symbol id.
        symbol_id: String,
    },
    /// A symbol config failed local validation.
    #[error("symbol `{symbol_id}` is invalid: {source}")]
    InvalidSymbolConfig {
        /// Symbol id associated with the validation failure.
        symbol_id: String,
        /// Underlying symbol validation failure.
        source: Box<SymbolConfigError>,
    },
    /// Symbol used a reader or symbol kind unsupported by the referenced network family.
    #[error("symbol `{symbol_id}` used unsupported kind/reader for network `{network_id}` family `{network_family:?}`")]
    UnsupportedSymbolNetworkFamily {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Network id associated with the failure.
        network_id: String,
        /// Network family selected by the referenced network.
        network_family: NetworkFamilyConfig,
    },
    /// Symbol referenced an unknown network.
    #[error("symbol `{symbol_id}` referenced unknown network `{network_id}`")]
    UnknownSymbolNetwork {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Unknown network id.
        network_id: String,
    },
    /// Quote codes must be unique at the portfolio level.
    #[error("portfolio quote `{quote}` must be unique")]
    DuplicateQuoteCode {
        /// Duplicate quote code.
        quote: QuoteCode,
    },
    /// Symbol did not provide a route for one requested quote.
    #[error("symbol `{symbol_id}` is missing valuation route for quote `{quote}`")]
    MissingValuationQuote {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Missing quote code.
        quote: QuoteCode,
    },
    /// Symbol defined a quote route that was not requested by the portfolio.
    #[error("symbol `{symbol_id}` defined unexpected valuation route for quote `{quote}`")]
    UnexpectedValuationQuote {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Unexpected quote code.
        quote: QuoteCode,
    },
    /// A valuation route referenced an unknown priced symbol.
    #[error("symbol `{symbol_id}` quote `{quote}` referenced unknown priced_symbol_id `{priced_symbol_id}`")]
    UnknownPricedSymbol {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Quote associated with the invalid route.
        quote: QuoteCode,
        /// Unknown priced symbol id.
        priced_symbol_id: String,
    },
    /// `underlying_symbol_id` referenced an unknown symbol.
    #[error(
        "symbol `{symbol_id}` referenced unknown underlying_symbol_id `{underlying_symbol_id}`"
    )]
    UnknownUnderlyingSymbol {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Unknown underlying symbol id.
        underlying_symbol_id: String,
    },
    /// A price source referenced an unknown network.
    #[error("symbol `{symbol_id}` quote `{quote}` reader `{reader_kind}` referenced unknown network `{network_id}`")]
    UnknownPriceSourceNetwork {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Quote associated with the invalid route.
        quote: QuoteCode,
        /// Unknown network id.
        network_id: String,
        /// Reader kind that held the invalid ref.
        reader_kind: &'static str,
    },
    /// The valuation source registry failed local validation.
    #[error("valuation source registry is invalid: {source}")]
    InvalidValuationSourceRegistry {
        /// Underlying registry validation failure.
        source: ValuationSourceRegistryError,
    },
    /// A valuation source registry entry referenced an unknown network.
    #[error("valuation source `{source_id}` referenced unknown network `{network_id}`")]
    UnknownValuationSourceNetwork {
        /// Valuation source id associated with the failure.
        source_id: String,
        /// Unknown network id.
        network_id: String,
    },
    /// A price source ref did not resolve through the valuation source registry.
    #[error(
        "symbol `{symbol_id}` quote `{quote}` referenced unknown valuation source `{source_id}`"
    )]
    UnknownValuationSource {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Quote associated with the invalid route.
        quote: QuoteCode,
        /// Unknown valuation source id.
        source_id: String,
    },
    /// A resolved valuation source entry did not match the referring source ref.
    #[error("symbol `{symbol_id}` quote `{quote}` source `{source_id}` mismatched registry field `{field}`")]
    ValuationSourceMismatch {
        /// Symbol id associated with the failure.
        symbol_id: String,
        /// Quote associated with the invalid route.
        quote: QuoteCode,
        /// Resolved valuation source id.
        source_id: String,
        /// Registry field that mismatched the ref.
        field: &'static str,
    },
}

/// Decodes and validates a canonical portfolio config.
pub fn decode_portfolio_config(value: &Value) -> Result<PortfolioConfig, PortfolioConfigError> {
    let cfg: PortfolioConfig = serde_json::from_value(value.clone())
        .map_err(|err| PortfolioConfigError::Decode(err.to_string()))?;
    cfg.validated()
}

fn validate_bitcoin_network(value: &str) -> Result<(), ()> {
    match value {
        "main" | "test" | "signet" | "regtest" => Ok(()),
        _ => Err(()),
    }
}

fn validate_portfolio_config_inner(cfg: &PortfolioConfig) -> Result<(), PortfolioConfigError> {
    let mut requested_quotes = BTreeSet::new();
    for quote in &cfg.quote_codes {
        if !requested_quotes.insert(*quote) {
            return Err(PortfolioConfigError::DuplicateQuoteCode { quote: *quote });
        }
    }

    let mut network_ids = HashSet::new();
    for network in &cfg.networks {
        if !network_ids.insert(network.network_id().clone()) {
            return Err(PortfolioConfigError::DuplicateNetworkId {
                network_id: network.network_id().to_string(),
            });
        }
    }

    let mut symbol_ids = HashSet::new();
    for symbol in &cfg.symbol_configs {
        validate_symbol_config(symbol).map_err(|source| {
            PortfolioConfigError::InvalidSymbolConfig {
                symbol_id: symbol.symbol_id.to_string(),
                source: Box::new(source),
            }
        })?;
        if !symbol_ids.insert(symbol.symbol_id.clone()) {
            return Err(PortfolioConfigError::DuplicateSymbolId {
                symbol_id: symbol.symbol_id.to_string(),
            });
        }
    }

    for symbol in &cfg.symbol_configs {
        if !network_ids.contains(&symbol.network_id) {
            return Err(PortfolioConfigError::UnknownSymbolNetwork {
                symbol_id: symbol.symbol_id.to_string(),
                network_id: symbol.network_id.to_string(),
            });
        }
        let network = cfg
            .networks
            .iter()
            .find(|network| network.network_id() == &symbol.network_id)
            .expect("validated network id must exist");
        validate_symbol_for_network_family(symbol, network)?;
        if let Some(underlying_symbol_id) = &symbol.underlying_symbol_id {
            if !symbol_ids.contains(underlying_symbol_id) {
                return Err(PortfolioConfigError::UnknownUnderlyingSymbol {
                    symbol_id: symbol.symbol_id.to_string(),
                    underlying_symbol_id: underlying_symbol_id.to_string(),
                });
            }
        }
        validate_symbol_quote_routes(symbol, &requested_quotes, &symbol_ids, &network_ids)?;
    }

    let mut wallet_ids = HashSet::new();
    for wallet in &cfg.wallets {
        if !wallet_ids.insert(wallet.wallet_id.clone()) {
            return Err(PortfolioConfigError::DuplicateWalletId {
                wallet_id: wallet.wallet_id.to_string(),
            });
        }
        if !network_ids.contains(&wallet.network_id) {
            return Err(PortfolioConfigError::UnknownWalletNetwork {
                wallet_id: wallet.wallet_id.to_string(),
                network_id: wallet.network_id.to_string(),
            });
        }
        let network = cfg
            .networks
            .iter()
            .find(|network| network.network_id() == &wallet.network_id)
            .expect("validated network id must exist");
        let wallet_subject_kind = wallet.subject.kind();
        if !wallet_subject_matches_network_family(wallet_subject_kind, network.family()) {
            return Err(PortfolioConfigError::WalletSubjectNetworkFamilyMismatch {
                wallet_id: wallet.wallet_id.to_string(),
                network_id: wallet.network_id.to_string(),
                wallet_subject_kind,
                network_family: network.family(),
            });
        }
        for symbol_id in &wallet.symbol_ids {
            if !symbol_ids.contains(symbol_id) {
                return Err(PortfolioConfigError::UnknownWalletSymbol {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol_id.to_string(),
                });
            }
            let symbol = cfg
                .symbol_configs
                .iter()
                .find(|symbol| symbol.symbol_id == *symbol_id)
                .expect("validated symbol id must exist");
            if symbol.network_id != wallet.network_id {
                return Err(PortfolioConfigError::WalletSymbolNetworkMismatch {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol.symbol_id.to_string(),
                    wallet_network_id: wallet.network_id.to_string(),
                    symbol_network_id: symbol.network_id.to_string(),
                });
            }
        }
    }

    Ok(())
}

fn validate_portfolio_config_for_mfm(cfg: &PortfolioConfig) -> Result<(), String> {
    ValidatedPortfolioConfig::new(cfg.clone())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn valuation_source_index(
    registry: &ValuationSourceRegistry,
) -> BTreeMap<ValuationSourceId, usize> {
    registry
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| (source.source_id.clone(), index))
        .collect()
}

fn validate_portfolio_bundle_sources(
    cfg: &PortfolioConfig,
    registry: &ValuationSourceRegistry,
    source_index: &BTreeMap<ValuationSourceId, usize>,
) -> Result<(), PortfolioConfigError> {
    let network_ids: HashSet<_> = cfg
        .networks
        .iter()
        .map(|network| network.network_id().clone())
        .collect();
    for source in &registry.sources {
        if !network_ids.contains(&source.network_id) {
            return Err(PortfolioConfigError::UnknownValuationSourceNetwork {
                source_id: source.source_id.to_string(),
                network_id: source.network_id.to_string(),
            });
        }
    }

    for symbol in &cfg.symbol_configs {
        for quote in &symbol.valuation.quotes {
            match &quote.reader {
                ValuationReaderConfig::FixedUnitPrice { .. } => {}
                ValuationReaderConfig::DirectPrice { source } => {
                    validate_price_source_registry_match(
                        symbol,
                        quote.quote,
                        source,
                        registry,
                        source_index,
                    )?;
                }
                ValuationReaderConfig::DerivedUnitPrice {
                    numerator,
                    denominator,
                } => {
                    validate_price_source_registry_match(
                        symbol,
                        quote.quote,
                        numerator,
                        registry,
                        source_index,
                    )?;
                    validate_price_source_registry_match(
                        symbol,
                        quote.quote,
                        denominator,
                        registry,
                        source_index,
                    )?;
                }
            }
        }
    }

    Ok(())
}

fn wallet_subject_matches_network_family(
    subject_kind: WalletSubjectKind,
    network_family: NetworkFamilyConfig,
) -> bool {
    matches!(
        (subject_kind, network_family),
        (WalletSubjectKind::EvmAddress, NetworkFamilyConfig::Evm)
            | (
                WalletSubjectKind::BitcoinAddress,
                NetworkFamilyConfig::Bitcoin
            )
    )
}

fn validate_symbol_for_network_family(
    symbol: &SymbolConfig,
    network: &NetworkConfig,
) -> Result<(), PortfolioConfigError> {
    if network.family() == NetworkFamilyConfig::Bitcoin
        && !matches!(
            (&symbol.kind, &symbol.balance_reader),
            (
                SymbolKind::NativeBalance,
                BalanceReaderConfig::NativeBalance {}
            )
        )
    {
        return Err(PortfolioConfigError::UnsupportedSymbolNetworkFamily {
            symbol_id: symbol.symbol_id.to_string(),
            network_id: network.network_id().to_string(),
            network_family: network.family(),
        });
    }
    Ok(())
}

fn validate_symbol_quote_routes(
    symbol: &SymbolConfig,
    requested_quotes: &BTreeSet<QuoteCode>,
    symbol_ids: &HashSet<SymbolId>,
    network_ids: &HashSet<NetworkId>,
) -> Result<(), PortfolioConfigError> {
    let symbol_quotes: BTreeSet<_> = symbol
        .valuation
        .quotes
        .iter()
        .map(|quote| quote.quote)
        .collect();

    for quote in requested_quotes {
        if !symbol_quotes.contains(quote) {
            return Err(PortfolioConfigError::MissingValuationQuote {
                symbol_id: symbol.symbol_id.to_string(),
                quote: *quote,
            });
        }
    }
    for quote in &symbol_quotes {
        if !requested_quotes.contains(quote) {
            return Err(PortfolioConfigError::UnexpectedValuationQuote {
                symbol_id: symbol.symbol_id.to_string(),
                quote: *quote,
            });
        }
    }

    for quote in &symbol.valuation.quotes {
        if !symbol_ids.contains(&quote.priced_symbol_id) {
            return Err(PortfolioConfigError::UnknownPricedSymbol {
                symbol_id: symbol.symbol_id.to_string(),
                quote: quote.quote,
                priced_symbol_id: quote.priced_symbol_id.to_string(),
            });
        }
        match &quote.reader {
            ValuationReaderConfig::FixedUnitPrice { .. } => {}
            ValuationReaderConfig::DirectPrice { source } => {
                if !network_ids.contains(&source.network_id) {
                    return Err(PortfolioConfigError::UnknownPriceSourceNetwork {
                        symbol_id: symbol.symbol_id.to_string(),
                        quote: quote.quote,
                        network_id: source.network_id.to_string(),
                        reader_kind: "direct_price",
                    });
                }
            }
            ValuationReaderConfig::DerivedUnitPrice {
                numerator,
                denominator,
            } => {
                for source in [numerator, denominator] {
                    if !network_ids.contains(&source.network_id) {
                        return Err(PortfolioConfigError::UnknownPriceSourceNetwork {
                            symbol_id: symbol.symbol_id.to_string(),
                            quote: quote.quote,
                            network_id: source.network_id.to_string(),
                            reader_kind: "derived_unit_price",
                        });
                    }
                }
            }
        }
    }

    Ok(())
}

fn validate_price_source_registry_match(
    symbol: &SymbolConfig,
    quote: QuoteCode,
    source_ref: &PriceSourceRef,
    registry: &ValuationSourceRegistry,
    source_index: &BTreeMap<ValuationSourceId, usize>,
) -> Result<(), PortfolioConfigError> {
    let Some(index) = source_index.get(&source_ref.source_id) else {
        return Err(PortfolioConfigError::UnknownValuationSource {
            symbol_id: symbol.symbol_id.to_string(),
            quote,
            source_id: source_ref.source_id.to_string(),
        });
    };
    let source_cfg = &registry.sources[*index];

    if source_cfg.network_id != source_ref.network_id {
        return Err(PortfolioConfigError::ValuationSourceMismatch {
            symbol_id: symbol.symbol_id.to_string(),
            quote,
            source_id: source_ref.source_id.to_string(),
            field: "network_id",
        });
    }
    if source_cfg.base_symbol_id != source_ref.base_symbol_id {
        return Err(PortfolioConfigError::ValuationSourceMismatch {
            symbol_id: symbol.symbol_id.to_string(),
            quote,
            source_id: source_ref.source_id.to_string(),
            field: "base_symbol_id",
        });
    }
    if source_cfg.quote != source_ref.quote {
        return Err(PortfolioConfigError::ValuationSourceMismatch {
            symbol_id: symbol.symbol_id.to_string(),
            quote,
            source_id: source_ref.source_id.to_string(),
            field: "quote",
        });
    }

    Ok(())
}
