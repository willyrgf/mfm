use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::num::NonZeroU64;

use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

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
                Self::Bitcoin {
                    network_id,
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
                    source,
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
    /// EVM execution pinned to one block number on one chain id.
    Evm {
        /// EVM chain id.
        chain_id: u64,
        /// Concrete pinned block number.
        block_number: u64,
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
#[derive(Clone, Debug, PartialEq, Eq, Error)]
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
    #[error(
        "wallet `{wallet_id}` subject_kind `{:?}` did not match network `{network_id}` family `{:?}`",
        wallet_subject_kind,
        network_family
    )]
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
    #[error(
        "wallet `{wallet_id}` on network `{wallet_network_id}` referenced symbol `{symbol_id}` on network `{symbol_network_id}`"
    )]
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
        source: SymbolConfigError,
    },
    /// Symbol used a reader or symbol kind unsupported by the referenced network family.
    #[error(
        "symbol `{symbol_id}` used unsupported kind/reader for network `{network_id}` family `{:?}`",
        network_family
    )]
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
    #[error(
        "symbol `{symbol_id}` quote `{quote}` referenced unknown priced_symbol_id `{priced_symbol_id}`"
    )]
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
    #[error(
        "symbol `{symbol_id}` quote `{quote}` reader `{reader_kind}` referenced unknown network `{network_id}`"
    )]
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
    #[error(
        "symbol `{symbol_id}` quote `{quote}` source `{source_id}` mismatched registry field `{field}`"
    )]
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
                source,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbol::{
        ObservationAnchor, ObservationQuantity, ObservationSource, ObservationValue,
        ObservationValueSourceRef, SymbolConfigError, SymbolRole,
    };
    use crate::wallet::WalletImplementationConfig;
    use serde_json::json;

    #[test]
    fn decode_canonical_portfolio_config() {
        let cfg = decode_portfolio_config(&canonical_config_json()).expect("config should decode");

        assert_eq!(cfg.portfolio_id, "portfolio_main");
        assert_eq!(cfg.quote_codes, vec![QuoteCode::Btc, QuoteCode::Usd]);
        assert_eq!(cfg.networks.len(), 2);
        assert_eq!(cfg.wallets.len(), 2);
        assert_eq!(cfg.symbol_configs.len(), 3);
        assert!(matches!(
            cfg.wallets[0].implementation,
            WalletImplementationConfig::AddressOnly {}
        ));
        assert_eq!(cfg.wallets[0].wallet_id, "wallet_ops_arb");
        assert_eq!(
            cfg.wallets[0].subject.address_str(),
            "0x000000000000000000000000000000000000beef"
        );
        assert!(matches!(
            cfg.symbol_configs[0].balance_reader,
            BalanceReaderConfig::NativeBalance {}
        ));
    }

    #[test]
    fn validated_network_configs_reject_duplicate_network_ids() {
        let network = NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            "shared".to_owned(),
            BTreeMap::new(),
        )
        .expect("network config");

        let authority =
            ValidatedNetworkConfigs::new(vec![network.clone()]).expect("network authority");
        assert_eq!(authority.as_slice(), &[network.clone()]);
        assert_eq!(authority.into_vec(), vec![network.clone()]);

        assert!(matches!(
            ValidatedNetworkConfigs::new(vec![network.clone(), network]),
            Err(PortfolioConfigError::DuplicateNetworkId { network_id })
                if network_id == "ethereum-mainnet"
        ));
    }

    #[test]
    fn validated_wallet_and_symbol_configs_reject_duplicate_ids() {
        let cfg = decode_portfolio_config(&canonical_config_json()).expect("config should decode");
        let wallet = cfg.wallets[0].clone();
        let symbol = cfg.symbol_configs[0].clone();

        let wallets = ValidatedWalletConfigs::new(vec![wallet.clone()]).expect("wallets");
        assert_eq!(wallets.as_slice(), &[wallet.clone()]);
        assert_eq!(wallets.into_vec(), vec![wallet.clone()]);
        assert!(matches!(
            ValidatedWalletConfigs::new(vec![wallet.clone(), wallet]),
            Err(PortfolioConfigError::DuplicateWalletId { wallet_id })
                if wallet_id == "wallet_ops_arb"
        ));

        let expected_symbol_id = symbol.symbol_id.clone();
        let symbols = ValidatedSymbolConfigs::new(vec![symbol.clone()]).expect("symbols");
        assert_eq!(symbols.as_slice(), &[symbol.clone()]);
        assert_eq!(symbols.into_vec(), vec![symbol.clone()]);
        assert!(matches!(
            ValidatedSymbolConfigs::new(vec![symbol.clone(), symbol]),
            Err(PortfolioConfigError::DuplicateSymbolId { symbol_id })
                if symbol_id == expected_symbol_id.as_str()
        ));
    }

    #[test]
    fn validated_portfolio_bundle_indexes_normalized_authority() {
        let portfolio = decode_portfolio_config(&json!({
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "chain_id": 1,
                    "control_scope": "shared",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "network_id": "ethereum-mainnet",
                    "implementation": {"kind": "address_only"},
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {"kind": "native_balance"},
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1800.00"
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }))
        .expect("portfolio config");
        let bundle = ValidatedPortfolioBundle::new(
            portfolio,
            ValuationSourceRegistry {
                sources: Vec::new(),
            },
        )
        .expect("portfolio bundle");

        assert_eq!(
            bundle.portfolio().quote(QuoteCode::Usd),
            Some(QuoteCode::Usd)
        );
        assert_eq!(
            bundle
                .portfolio()
                .network("ethereum-mainnet")
                .expect("network")
                .chain_id_u64(),
            Some(1)
        );
        assert_eq!(
            bundle
                .portfolio()
                .wallet("wallet_main")
                .expect("wallet")
                .subject
                .address_str(),
            "0x000000000000000000000000000000000000dead"
        );
        assert!(bundle
            .portfolio()
            .symbol("eth.native.ethereum-mainnet")
            .is_some());
        assert!(bundle.valuation_source("missing").is_none());

        let (portfolio, registry) = bundle.into_parts();
        assert_eq!(portfolio.wallets[0].wallet_id, "wallet_main");
        assert!(registry.sources.is_empty());
    }

    #[test]
    fn invalid_ref_detection_catches_cross_links() {
        let mut wallet_network = canonical_config_json();
        wallet_network["wallets"][0]["network_id"] = json!("unknown-network");
        assert_eq!(
            decode_portfolio_config(&wallet_network).unwrap_err(),
            PortfolioConfigError::UnknownWalletNetwork {
                wallet_id: "wallet_treasury_eth".to_string(),
                network_id: "unknown-network".to_string(),
            }
        );

        let mut wallet_symbol = canonical_config_json();
        wallet_symbol["wallets"][0]["symbol_ids"][0] = json!("unknown-symbol");
        assert_eq!(
            decode_portfolio_config(&wallet_symbol).unwrap_err(),
            PortfolioConfigError::UnknownWalletSymbol {
                wallet_id: "wallet_treasury_eth".to_string(),
                symbol_id: "unknown-symbol".to_string(),
            }
        );

        let mut symbol_network = canonical_config_json();
        symbol_network["symbol_configs"][0]["network_id"] = json!("unknown-network");
        assert_eq!(
            decode_portfolio_config(&symbol_network).unwrap_err(),
            PortfolioConfigError::UnknownSymbolNetwork {
                symbol_id: "eth.native.ethereum-mainnet".to_string(),
                network_id: "unknown-network".to_string(),
            }
        );

        let mut priced_symbol = canonical_config_json();
        priced_symbol["symbol_configs"][1]["valuation"]["quotes"][0]["priced_symbol_id"] =
            json!("unknown-symbol");
        assert_eq!(
            decode_portfolio_config(&priced_symbol).unwrap_err(),
            PortfolioConfigError::UnknownPricedSymbol {
                symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                quote: QuoteCode::Usd,
                priced_symbol_id: "unknown-symbol".to_string(),
            }
        );

        let mut underlying_symbol = canonical_config_json();
        underlying_symbol["symbol_configs"][1]["underlying_symbol_id"] = json!("unknown-symbol");
        assert_eq!(
            decode_portfolio_config(&underlying_symbol).unwrap_err(),
            PortfolioConfigError::UnknownUnderlyingSymbol {
                symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                underlying_symbol_id: "unknown-symbol".to_string(),
            }
        );

        let mut source_network = canonical_config_json();
        source_network["symbol_configs"][0]["valuation"]["quotes"][0]["reader"]["source"]
            ["network_id"] = json!("unknown-network");
        assert_eq!(
            decode_portfolio_config(&source_network).unwrap_err(),
            PortfolioConfigError::UnknownPriceSourceNetwork {
                symbol_id: "eth.native.ethereum-mainnet".to_string(),
                quote: QuoteCode::Usd,
                network_id: "unknown-network".to_string(),
                reader_kind: "direct_price",
            }
        );
    }

    #[test]
    fn typed_metadata_rejects_non_string_values() {
        let mut portfolio_metadata = canonical_config_json();
        portfolio_metadata["metadata"] = json!({"threshold": 1.5});
        assert!(matches!(
            decode_portfolio_config(&portfolio_metadata),
            Err(PortfolioConfigError::Decode(_))
        ));

        let mut float_decimals = canonical_config_json();
        float_decimals["symbol_configs"][0]["decimals"] = json!(18.5);
        assert!(matches!(
            decode_portfolio_config(&float_decimals),
            Err(PortfolioConfigError::Decode(_))
        ));
    }

    #[test]
    fn typed_metadata_rejects_secret_markers() {
        let mut portfolio_metadata = canonical_config_json();
        portfolio_metadata["metadata"] = json!({"mnemonic": "redacted"});
        assert_decode_rejects_public_metadata(&portfolio_metadata, "mnemonic");

        let mut network_metadata = canonical_config_json();
        network_metadata["networks"][0]["metadata"] = json!({"label": "password=redacted"});
        assert_decode_rejects_public_metadata(&network_metadata, "label");

        let mut wallet_metadata = canonical_config_json();
        wallet_metadata["wallets"][0]["metadata"] = json!({"api_key": "redacted"});
        assert_decode_rejects_public_metadata(&wallet_metadata, "api_key");

        let mut symbol_metadata = canonical_config_json();
        symbol_metadata["symbol_configs"][0]["metadata"] = json!({"note": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"});
        assert_decode_rejects_public_metadata(&symbol_metadata, "note");
    }

    fn assert_decode_rejects_public_metadata(value: &Value, expected_key: &str) {
        let error = decode_portfolio_config(value).expect_err("metadata must fail");
        let PortfolioConfigError::Decode(message) = error else {
            panic!("expected decode failure, got {error}");
        };
        assert!(
            message.contains(expected_key),
            "decode message `{message}` did not include `{expected_key}`"
        );
    }

    #[test]
    fn validation_rejects_non_normalized_addresses() {
        let mut wallet_address = canonical_config_json();
        wallet_address["wallets"][0]["subject"]["address"] =
            json!("0x000000000000000000000000000000000000DEAD");
        let err = decode_portfolio_config(&wallet_address).unwrap_err();
        assert!(matches!(err, PortfolioConfigError::Decode(message)
            if message.contains("normalized EVM address")));

        let mut token_address = canonical_config_json();
        token_address["symbol_configs"][1]["balance_reader"]["token_address"] =
            json!("0x000000000000000000000000000000000000000A");
        let err = decode_portfolio_config(&token_address).unwrap_err();
        assert!(matches!(err, PortfolioConfigError::Decode(message)
            if message.contains("normalized EVM address")));
    }

    #[test]
    fn validation_rejects_quote_route_mismatches() {
        let mut duplicate_quote_codes = canonical_config_json();
        duplicate_quote_codes["quote_codes"] = json!(["USD", "USD"]);
        assert_eq!(
            decode_portfolio_config(&duplicate_quote_codes).unwrap_err(),
            PortfolioConfigError::DuplicateQuoteCode {
                quote: QuoteCode::Usd,
            }
        );

        let mut missing_quote = canonical_config_json();
        missing_quote["symbol_configs"][0]["valuation"]["quotes"] = json!([
            {
                "quote": "USD",
                "priced_symbol_id": "eth.native.ethereum-mainnet",
                "reader": {
                    "kind": "direct_price",
                    "source": {
                        "source_id": "chainlink_eth_usd",
                        "network_id": "ethereum-mainnet",
                        "base_symbol_id": "eth.native.ethereum-mainnet",
                        "quote": "USD"
                    }
                }
            }
        ]);
        assert_eq!(
            decode_portfolio_config(&missing_quote).unwrap_err(),
            PortfolioConfigError::MissingValuationQuote {
                symbol_id: "eth.native.ethereum-mainnet".to_string(),
                quote: QuoteCode::Btc,
            }
        );

        let mut unexpected_quote = canonical_config_json();
        unexpected_quote["quote_codes"] = json!(["USD"]);
        assert_eq!(
            decode_portfolio_config(&unexpected_quote).unwrap_err(),
            PortfolioConfigError::UnexpectedValuationQuote {
                symbol_id: "eth.native.ethereum-mainnet".to_string(),
                quote: QuoteCode::Btc,
            }
        );

        let mut derived_mismatch = canonical_config_json();
        derived_mismatch["symbol_configs"][1]["valuation"]["quotes"][1]["reader"]["denominator"]
            ["quote"] = json!("BTC");
        assert_eq!(
            decode_portfolio_config(&derived_mismatch).unwrap_err(),
            PortfolioConfigError::InvalidSymbolConfig {
                symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                source: SymbolConfigError::DerivedPriceQuoteMismatch {
                    quote: QuoteCode::Btc,
                    numerator_quote: QuoteCode::Usd,
                    denominator_quote: QuoteCode::Btc,
                },
            }
        );
    }

    #[test]
    fn normalization_sorts_config_and_runtime_outputs() {
        let mut cfg =
            decode_portfolio_config(&canonical_config_json()).expect("config should decode");
        cfg.quote_codes.reverse();
        cfg.networks.reverse();
        cfg.wallets.reverse();
        cfg.wallets[0].symbol_ids.reverse();
        cfg.symbol_configs.reverse();
        cfg.symbol_configs[0].valuation.quotes.reverse();
        cfg.normalize();

        assert_eq!(cfg.quote_codes, vec![QuoteCode::Btc, QuoteCode::Usd]);
        assert_eq!(
            cfg.networks
                .iter()
                .map(|network| network.network_id().as_str())
                .collect::<Vec<_>>(),
            vec!["arbitrum-mainnet", "ethereum-mainnet"]
        );
        assert_eq!(
            cfg.wallets
                .iter()
                .map(|wallet| wallet.wallet_id.as_str())
                .collect::<Vec<_>>(),
            vec!["wallet_ops_arb", "wallet_treasury_eth"]
        );
        assert_eq!(
            cfg.wallets[1]
                .symbol_ids
                .iter()
                .map(|symbol_id| symbol_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "eth.native.ethereum-mainnet",
                "usdc.wallet.ethereum-mainnet"
            ]
        );
        assert_eq!(
            cfg.symbol_configs
                .iter()
                .map(|symbol| symbol.symbol_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "eth.native.arbitrum-mainnet",
                "eth.native.ethereum-mainnet",
                "usdc.wallet.ethereum-mainnet",
            ]
        );
        assert_eq!(
            cfg.symbol_configs[1]
                .valuation
                .quotes
                .iter()
                .map(|quote| quote.quote)
                .collect::<Vec<_>>(),
            vec![QuoteCode::Btc, QuoteCode::Usd]
        );

        let mut snapshot = PortfolioSnapshot {
            schema_version: 2,
            portfolio_id: "portfolio_main".to_string(),
            generated_at_ms: 1,
            network_pins: vec![
                NetworkPin {
                    network_id: "ethereum-mainnet".to_string(),
                    anchor: ExecutionAnchor::Evm {
                        chain_id: 1,
                        block_number: 10,
                    },
                },
                NetworkPin {
                    network_id: "arbitrum-mainnet".to_string(),
                    anchor: ExecutionAnchor::Evm {
                        chain_id: 42161,
                        block_number: 20,
                    },
                },
            ],
            wallets: vec![
                WalletSnapshot {
                    wallet_id: "wallet_treasury_eth".to_string(),
                    address: "0x000000000000000000000000000000000000dead".to_string(),
                    subject_kind: WalletSubjectKind::EvmAddress,
                    network_id: "ethereum-mainnet".to_string(),
                    observations: vec![
                        observation(
                            "wallet_treasury_eth",
                            "usdc.wallet.ethereum-mainnet",
                            vec![QuoteCode::Usd, QuoteCode::Btc],
                        ),
                        observation(
                            "wallet_treasury_eth",
                            "eth.native.ethereum-mainnet",
                            vec![QuoteCode::Usd, QuoteCode::Btc],
                        ),
                    ],
                },
                WalletSnapshot {
                    wallet_id: "wallet_ops_arb".to_string(),
                    address: "0x000000000000000000000000000000000000beef".to_string(),
                    subject_kind: WalletSubjectKind::EvmAddress,
                    network_id: "arbitrum-mainnet".to_string(),
                    observations: vec![observation(
                        "wallet_ops_arb",
                        "eth.native.arbitrum-mainnet",
                        vec![QuoteCode::Usd, QuoteCode::Btc],
                    )],
                },
            ],
            symbol_configs: cfg.symbol_configs.clone(),
            errors: vec![
                PortfolioSnapshotError {
                    code: "wallet_error".to_string(),
                    message: "wallet".to_string(),
                    wallet_id: Some("wallet_treasury_eth".to_string()),
                    symbol_id: None,
                    network_id: Some("ethereum-mainnet".to_string()),
                    reader_kind: None,
                },
                PortfolioSnapshotError {
                    code: "symbol_error".to_string(),
                    message: "symbol".to_string(),
                    wallet_id: Some("wallet_ops_arb".to_string()),
                    symbol_id: Some("eth.native.arbitrum-mainnet".to_string()),
                    network_id: Some("arbitrum-mainnet".to_string()),
                    reader_kind: None,
                },
            ],
        };
        snapshot.network_pins.reverse();
        snapshot.wallets.reverse();
        snapshot.wallets[0].observations.reverse();
        snapshot.errors.reverse();
        snapshot.normalize();

        assert_eq!(
            snapshot
                .network_pins
                .iter()
                .map(|pin| pin.network_id.as_str())
                .collect::<Vec<_>>(),
            vec!["arbitrum-mainnet", "ethereum-mainnet"]
        );
        assert_eq!(
            snapshot
                .wallets
                .iter()
                .map(|wallet| wallet.wallet_id.as_str())
                .collect::<Vec<_>>(),
            vec!["wallet_ops_arb", "wallet_treasury_eth"]
        );
        assert_eq!(
            snapshot.wallets[1]
                .observations
                .iter()
                .map(|observation| observation.symbol_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "eth.native.ethereum-mainnet",
                "usdc.wallet.ethereum-mainnet",
            ]
        );
        assert_eq!(
            snapshot
                .errors
                .iter()
                .map(|error| error.code.as_str())
                .collect::<Vec<_>>(),
            vec!["symbol_error", "wallet_error"]
        );
    }

    fn canonical_config_json() -> Value {
        json!({
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD", "BTC"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "chain_id": 1,
                    "control_scope": "shared",
                    "metadata": {}
                },
                {
                    "network_id": "arbitrum-mainnet",
                    "family": "evm",
                    "chain_id": 42161,
                    "control_scope": "shared",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_treasury_eth",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": [
                        "eth.native.ethereum-mainnet",
                        "usdc.wallet.ethereum-mainnet"
                    ],
                    "metadata": {}
                },
                {
                    "wallet_id": "wallet_ops_arb",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000beef"
                    },
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": "arbitrum-mainnet",
                    "symbol_ids": [
                        "eth.native.arbitrum-mainnet"
                    ],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "native_balance"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_eth_usd",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "eth.native.ethereum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_eth_btc",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "eth.native.ethereum-mainnet",
                                        "quote": "BTC"
                                    }
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "usdc.wallet.ethereum-mainnet",
                    "display_symbol": "USDC",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": "0x0000000000000000000000000000000000000001"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_usdc_usd",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "usdc.wallet.ethereum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "derived_unit_price",
                                    "numerator": {
                                        "source_id": "chainlink_usdc_usd",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "usdc.wallet.ethereum-mainnet",
                                        "quote": "USD"
                                    },
                                    "denominator": {
                                        "source_id": "chainlink_btc_usd",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "btc.wallet.ethereum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            }
                        ]
                    },
                    "decimals": 6,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "eth.native.arbitrum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "arbitrum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "native_balance"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.arbitrum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_eth_usd",
                                        "network_id": "arbitrum-mainnet",
                                        "base_symbol_id": "eth.native.arbitrum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "eth.native.arbitrum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_eth_btc",
                                        "network_id": "arbitrum-mainnet",
                                        "base_symbol_id": "eth.native.arbitrum-mainnet",
                                        "quote": "BTC"
                                    }
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        })
    }

    fn observation(wallet_id: &str, symbol_id: &str, value_order: Vec<QuoteCode>) -> Observation {
        Observation {
            wallet_id: wallet_id.to_string(),
            symbol_id: symbol_id.to_string(),
            display_symbol: Some("ETH".to_string()),
            kind: SymbolKind::NativeBalance,
            role: SymbolRole::Native,
            network_id: if symbol_id.contains("arbitrum") {
                "arbitrum-mainnet".to_string()
            } else {
                "ethereum-mainnet".to_string()
            },
            protocol: None,
            quantity: ObservationQuantity {
                raw_dec: "1".to_string(),
                decimals: 18,
                amount_dec: "0.000000000000000001".to_string(),
            },
            values: value_order
                .into_iter()
                .map(|quote| ObservationValue {
                    quote,
                    priced_symbol_id: symbol_id.to_string(),
                    value_dec: "1".to_string(),
                    unit_price_dec: "1".to_string(),
                    valuation_reader_kind: "direct_price".to_string(),
                    source_refs: vec![ObservationValueSourceRef {
                        source_id: "src".to_string(),
                        network_id: if symbol_id.contains("arbitrum") {
                            "arbitrum-mainnet".to_string()
                        } else {
                            "ethereum-mainnet".to_string()
                        },
                        anchor: ObservationAnchor::Evm {
                            chain_id: if symbol_id.contains("arbitrum") {
                                42161
                            } else {
                                1
                            },
                            block_number: 1,
                        },
                    }],
                })
                .collect(),
            source: ObservationSource {
                balance_reader_kind: "native_balance".to_string(),
                network_id: if symbol_id.contains("arbitrum") {
                    "arbitrum-mainnet".to_string()
                } else {
                    "ethereum-mainnet".to_string()
                },
                anchor: ObservationAnchor::Evm {
                    chain_id: if symbol_id.contains("arbitrum") {
                        42161
                    } else {
                        1
                    },
                    block_number: 1,
                },
            },
            metadata: PublicMetadata::default(),
        }
    }
}
