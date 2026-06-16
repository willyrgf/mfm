use std::collections::{BTreeMap, BTreeSet, HashSet};

use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use mfm_values::string_map_secret_marker_key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::symbol::{
    validate_symbol_config, BalanceReaderConfig, Observation, PriceSourceRef, QuoteCode,
    SymbolConfig, SymbolConfigError, SymbolKind, ValuationReaderConfig, ValuationSourceConfig,
    ValuationSourceRegistry, ValuationSourceRegistryError,
};
use crate::wallet::{validate_wallet_config, WalletConfig, WalletConfigError, WalletSubjectKind};

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
    pub portfolio_id: String,
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
    pub metadata: BTreeMap<String, String>,
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
#[mfm(
    namespace = "mfm.portfolio",
    name = "network-config",
    schema = "mfm.portfolio.network_config"
)]
pub struct NetworkConfig {
    /// Stable machine identifier for the network.
    pub network_id: String,
    /// Declared execution family for the network.
    #[serde(default)]
    pub family: NetworkFamilyConfig,
    /// EVM chain id for the network when `family = "evm"`.
    #[serde(default)]
    pub chain_id: Option<u64>,
    /// Stable control-plane scope used for managed rpc.control reads on this network.
    pub control_scope: String,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
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
        Self {
            network_id,
            family,
            chain_id,
            control_scope,
            metadata,
        }
        .validated()
    }

    /// Validates this network config and returns it unchanged.
    pub fn validated(self) -> Result<Self, PortfolioConfigError> {
        validate_network_config(&self)?;
        Ok(self)
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
        self.index
            .networks
            .get(network_id)
            .map(|index| &self.config.networks[*index])
    }

    /// Returns a wallet by stable id.
    pub fn wallet(&self, wallet_id: &str) -> Option<&WalletConfig> {
        self.index
            .wallets
            .get(wallet_id)
            .map(|index| &self.config.wallets[*index])
    }

    /// Returns a symbol by stable id.
    pub fn symbol(&self, symbol_id: &str) -> Option<&SymbolConfig> {
        self.index
            .symbols
            .get(symbol_id)
            .map(|index| &self.config.symbol_configs[*index])
    }
}

/// Validated portfolio plus valuation-source registry authority.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedPortfolioBundle {
    portfolio: ValidatedPortfolioConfig,
    valuation_source_registry: ValuationSourceRegistry,
    source_index: BTreeMap<String, usize>,
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
        self.source_index
            .get(source_id)
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
    networks: BTreeMap<String, usize>,
    wallets: BTreeMap<String, usize>,
    symbols: BTreeMap<String, usize>,
}

impl PortfolioConfigIndex {
    fn new(config: &PortfolioConfig) -> Self {
        let quotes = config.quote_codes.iter().copied().collect();
        let networks = config
            .networks
            .iter()
            .enumerate()
            .map(|(index, network)| (network.network_id.clone(), index))
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
    /// `portfolio_id` was empty.
    #[error("portfolio_id must be non-empty")]
    EmptyPortfolioId,
    /// Portfolio metadata contained a secret-shaped key or value.
    #[error("portfolio metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// One of the network ids was empty.
    #[error("network_id must be non-empty")]
    EmptyNetworkId,
    /// One of the network control scopes was empty.
    #[error("network `{network_id}` control_scope must be non-empty")]
    EmptyNetworkControlScope {
        /// Network id associated with the failure.
        network_id: String,
    },
    /// EVM networks require an explicit chain id.
    #[error("network `{network_id}` with family `evm` must declare chain_id")]
    MissingEvmChainId {
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
    /// A wallet config failed local validation.
    #[error("wallet `{wallet_id}` is invalid: {source}")]
    InvalidWalletConfig {
        /// Wallet id associated with the validation failure.
        wallet_id: String,
        /// Underlying wallet validation failure.
        source: WalletConfigError,
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
    if cfg.portfolio_id.trim().is_empty() {
        return Err(PortfolioConfigError::EmptyPortfolioId);
    }
    if let Some(key) = string_map_secret_marker_key(&cfg.metadata) {
        return Err(PortfolioConfigError::MetadataContainsSecret {
            key: key.to_string(),
        });
    }

    let mut requested_quotes = BTreeSet::new();
    for quote in &cfg.quote_codes {
        if !requested_quotes.insert(*quote) {
            return Err(PortfolioConfigError::DuplicateQuoteCode { quote: *quote });
        }
    }

    let mut network_ids = HashSet::new();
    for network in &cfg.networks {
        validate_network_config(network)?;
        if !network_ids.insert(network.network_id.clone()) {
            return Err(PortfolioConfigError::DuplicateNetworkId {
                network_id: network.network_id.clone(),
            });
        }
    }

    let mut symbol_ids = HashSet::new();
    for symbol in &cfg.symbol_configs {
        validate_symbol_config(symbol).map_err(|source| {
            PortfolioConfigError::InvalidSymbolConfig {
                symbol_id: symbol.symbol_id.clone(),
                source,
            }
        })?;
        if !symbol_ids.insert(symbol.symbol_id.clone()) {
            return Err(PortfolioConfigError::DuplicateSymbolId {
                symbol_id: symbol.symbol_id.clone(),
            });
        }
    }

    for symbol in &cfg.symbol_configs {
        if !network_ids.contains(&symbol.network_id) {
            return Err(PortfolioConfigError::UnknownSymbolNetwork {
                symbol_id: symbol.symbol_id.clone(),
                network_id: symbol.network_id.clone(),
            });
        }
        let network = cfg
            .networks
            .iter()
            .find(|network| network.network_id == symbol.network_id)
            .expect("validated network id must exist");
        validate_symbol_for_network_family(symbol, network)?;
        if let Some(underlying_symbol_id) = &symbol.underlying_symbol_id {
            if !symbol_ids.contains(underlying_symbol_id) {
                return Err(PortfolioConfigError::UnknownUnderlyingSymbol {
                    symbol_id: symbol.symbol_id.clone(),
                    underlying_symbol_id: underlying_symbol_id.clone(),
                });
            }
        }
        validate_symbol_quote_routes(symbol, &requested_quotes, &symbol_ids, &network_ids)?;
    }

    let mut wallet_ids = HashSet::new();
    for wallet in &cfg.wallets {
        validate_wallet_config(wallet).map_err(|source| {
            PortfolioConfigError::InvalidWalletConfig {
                wallet_id: wallet.wallet_id.clone(),
                source,
            }
        })?;
        if !wallet_ids.insert(wallet.wallet_id.clone()) {
            return Err(PortfolioConfigError::DuplicateWalletId {
                wallet_id: wallet.wallet_id.clone(),
            });
        }
        if !network_ids.contains(&wallet.network_id) {
            return Err(PortfolioConfigError::UnknownWalletNetwork {
                wallet_id: wallet.wallet_id.clone(),
                network_id: wallet.network_id.clone(),
            });
        }
        let network = cfg
            .networks
            .iter()
            .find(|network| network.network_id == wallet.network_id)
            .expect("validated network id must exist");
        if !wallet_subject_matches_network_family(wallet.subject_kind, network.family) {
            return Err(PortfolioConfigError::WalletSubjectNetworkFamilyMismatch {
                wallet_id: wallet.wallet_id.clone(),
                network_id: wallet.network_id.clone(),
                wallet_subject_kind: wallet.subject_kind,
                network_family: network.family,
            });
        }
        for symbol_id in &wallet.symbol_ids {
            if !symbol_ids.contains(symbol_id) {
                return Err(PortfolioConfigError::UnknownWalletSymbol {
                    wallet_id: wallet.wallet_id.clone(),
                    symbol_id: symbol_id.clone(),
                });
            }
            let symbol = cfg
                .symbol_configs
                .iter()
                .find(|symbol| symbol.symbol_id == *symbol_id)
                .expect("validated symbol id must exist");
            if symbol.network_id != wallet.network_id {
                return Err(PortfolioConfigError::WalletSymbolNetworkMismatch {
                    wallet_id: wallet.wallet_id.clone(),
                    symbol_id: symbol.symbol_id.clone(),
                    wallet_network_id: wallet.network_id.clone(),
                    symbol_network_id: symbol.network_id.clone(),
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

fn valuation_source_index(registry: &ValuationSourceRegistry) -> BTreeMap<String, usize> {
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
    source_index: &BTreeMap<String, usize>,
) -> Result<(), PortfolioConfigError> {
    let network_ids: HashSet<_> = cfg
        .networks
        .iter()
        .map(|network| network.network_id.clone())
        .collect();
    for source in &registry.sources {
        if !network_ids.contains(&source.network_id) {
            return Err(PortfolioConfigError::UnknownValuationSourceNetwork {
                source_id: source.source_id.clone(),
                network_id: source.network_id.clone(),
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

/// Validates one canonical network config.
pub fn validate_network_config(network: &NetworkConfig) -> Result<(), PortfolioConfigError> {
    if network.network_id.trim().is_empty() {
        return Err(PortfolioConfigError::EmptyNetworkId);
    }
    if network.control_scope.trim().is_empty() {
        return Err(PortfolioConfigError::EmptyNetworkControlScope {
            network_id: network.network_id.clone(),
        });
    }
    if let Some(key) = string_map_secret_marker_key(&network.metadata) {
        return Err(PortfolioConfigError::NetworkMetadataContainsSecret {
            network_id: network.network_id.clone(),
            key: key.to_string(),
        });
    }
    match network.family {
        NetworkFamilyConfig::Evm => {
            if network.chain_id.is_none() {
                return Err(PortfolioConfigError::MissingEvmChainId {
                    network_id: network.network_id.clone(),
                });
            }
        }
        NetworkFamilyConfig::Bitcoin => {
            if network.chain_id.is_some() {
                return Err(PortfolioConfigError::UnexpectedBitcoinChainId {
                    network_id: network.network_id.clone(),
                });
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
    if network.family == NetworkFamilyConfig::Bitcoin
        && !matches!(
            (&symbol.kind, &symbol.balance_reader),
            (
                SymbolKind::NativeBalance,
                BalanceReaderConfig::NativeBalance {}
            )
        )
    {
        return Err(PortfolioConfigError::UnsupportedSymbolNetworkFamily {
            symbol_id: symbol.symbol_id.clone(),
            network_id: network.network_id.clone(),
            network_family: network.family,
        });
    }
    Ok(())
}

fn validate_symbol_quote_routes(
    symbol: &SymbolConfig,
    requested_quotes: &BTreeSet<QuoteCode>,
    symbol_ids: &HashSet<String>,
    network_ids: &HashSet<String>,
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
                symbol_id: symbol.symbol_id.clone(),
                quote: *quote,
            });
        }
    }
    for quote in &symbol_quotes {
        if !requested_quotes.contains(quote) {
            return Err(PortfolioConfigError::UnexpectedValuationQuote {
                symbol_id: symbol.symbol_id.clone(),
                quote: *quote,
            });
        }
    }

    for quote in &symbol.valuation.quotes {
        if !symbol_ids.contains(&quote.priced_symbol_id) {
            return Err(PortfolioConfigError::UnknownPricedSymbol {
                symbol_id: symbol.symbol_id.clone(),
                quote: quote.quote,
                priced_symbol_id: quote.priced_symbol_id.clone(),
            });
        }
        match &quote.reader {
            ValuationReaderConfig::FixedUnitPrice { .. } => {}
            ValuationReaderConfig::DirectPrice { source } => {
                if !network_ids.contains(&source.network_id) {
                    return Err(PortfolioConfigError::UnknownPriceSourceNetwork {
                        symbol_id: symbol.symbol_id.clone(),
                        quote: quote.quote,
                        network_id: source.network_id.clone(),
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
                            symbol_id: symbol.symbol_id.clone(),
                            quote: quote.quote,
                            network_id: source.network_id.clone(),
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
    source_index: &BTreeMap<String, usize>,
) -> Result<(), PortfolioConfigError> {
    let Some(index) = source_index.get(&source_ref.source_id) else {
        return Err(PortfolioConfigError::UnknownValuationSource {
            symbol_id: symbol.symbol_id.clone(),
            quote,
            source_id: source_ref.source_id.clone(),
        });
    };
    let source_cfg = &registry.sources[*index];

    if source_cfg.network_id != source_ref.network_id {
        return Err(PortfolioConfigError::ValuationSourceMismatch {
            symbol_id: symbol.symbol_id.clone(),
            quote,
            source_id: source_ref.source_id.clone(),
            field: "network_id",
        });
    }
    if source_cfg.base_symbol_id != source_ref.base_symbol_id {
        return Err(PortfolioConfigError::ValuationSourceMismatch {
            symbol_id: symbol.symbol_id.clone(),
            quote,
            source_id: source_ref.source_id.clone(),
            field: "base_symbol_id",
        });
    }
    if source_cfg.quote != source_ref.quote {
        return Err(PortfolioConfigError::ValuationSourceMismatch {
            symbol_id: symbol.symbol_id.clone(),
            quote,
            source_id: source_ref.source_id.clone(),
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
    use crate::wallet::{WalletConfigError, WalletImplementationConfig};
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
            cfg.wallets[0].address,
            "0x000000000000000000000000000000000000beef"
        );
        assert!(matches!(
            cfg.symbol_configs[0].balance_reader,
            BalanceReaderConfig::NativeBalance {}
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
                    "address": "0x000000000000000000000000000000000000dead",
                    "subject_kind": "evm_address",
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
                .chain_id,
            Some(1)
        );
        assert_eq!(
            bundle
                .portfolio()
                .wallet("wallet_main")
                .expect("wallet")
                .address,
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
        assert_eq!(
            decode_portfolio_config(&portfolio_metadata).unwrap_err(),
            PortfolioConfigError::MetadataContainsSecret {
                key: "mnemonic".to_string(),
            }
        );

        let mut network_metadata = canonical_config_json();
        network_metadata["networks"][0]["metadata"] = json!({"label": "password=redacted"});
        assert_eq!(
            decode_portfolio_config(&network_metadata).unwrap_err(),
            PortfolioConfigError::NetworkMetadataContainsSecret {
                network_id: "ethereum-mainnet".to_string(),
                key: "label".to_string(),
            }
        );

        let mut wallet_metadata = canonical_config_json();
        wallet_metadata["wallets"][0]["metadata"] = json!({"api_key": "redacted"});
        assert_eq!(
            decode_portfolio_config(&wallet_metadata).unwrap_err(),
            PortfolioConfigError::InvalidWalletConfig {
                wallet_id: "wallet_treasury_eth".to_string(),
                source: WalletConfigError::MetadataContainsSecret {
                    key: "api_key".to_string(),
                },
            }
        );

        let mut symbol_metadata = canonical_config_json();
        symbol_metadata["symbol_configs"][0]["metadata"] = json!({"note": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"});
        assert_eq!(
            decode_portfolio_config(&symbol_metadata).unwrap_err(),
            PortfolioConfigError::InvalidSymbolConfig {
                symbol_id: "eth.native.ethereum-mainnet".to_string(),
                source: SymbolConfigError::MetadataContainsSecret {
                    key: "note".to_string(),
                },
            }
        );
    }

    #[test]
    fn validation_rejects_non_normalized_addresses() {
        let mut wallet_address = canonical_config_json();
        wallet_address["wallets"][0]["address"] =
            json!("0x000000000000000000000000000000000000DEAD");
        assert_eq!(
            decode_portfolio_config(&wallet_address).unwrap_err(),
            PortfolioConfigError::InvalidWalletConfig {
                wallet_id: "wallet_treasury_eth".to_string(),
                source: WalletConfigError::InvalidAddress {
                    address: "0x000000000000000000000000000000000000DEAD".to_string(),
                    subject_kind: WalletSubjectKind::EvmAddress,
                },
            }
        );

        let mut token_address = canonical_config_json();
        token_address["symbol_configs"][1]["balance_reader"]["token_address"] =
            json!("0x000000000000000000000000000000000000000A");
        assert_eq!(
            decode_portfolio_config(&token_address).unwrap_err(),
            PortfolioConfigError::InvalidSymbolConfig {
                symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                source: SymbolConfigError::InvalidTokenAddress {
                    token_address: "0x000000000000000000000000000000000000000A".to_string(),
                },
            }
        );
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
                .map(|network| network.network_id.as_str())
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
            cfg.wallets[1].symbol_ids,
            vec![
                "eth.native.ethereum-mainnet".to_string(),
                "usdc.wallet.ethereum-mainnet".to_string()
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
                    "address": "0x000000000000000000000000000000000000dead",
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
                    "address": "0x000000000000000000000000000000000000beef",
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
            metadata: BTreeMap::new(),
        }
    }
}
