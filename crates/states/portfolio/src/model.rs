use std::collections::{BTreeMap, BTreeSet, HashSet};

use mfm_machine::hashing::{canonical_json_bytes, CanonicalJsonError};
use mfm_state_symbol::model::{
    validate_symbol_config, validate_valuation_source_registry, Observation, PriceSourceRef,
    QuoteCode, SymbolConfig, SymbolConfigError, ValuationReaderConfig, ValuationSourceRegistry,
    ValuationSourceRegistryError,
};
use mfm_state_wallet::model::{validate_wallet_config, WalletConfig, WalletConfigError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Canonical top-level portfolio configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub metadata: BTreeMap<String, Value>,
}

impl PortfolioConfig {
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
}

/// Canonical network configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Stable machine identifier for the network.
    pub network_id: String,
    /// EVM chain id for the network.
    pub chain_id: u64,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

/// Concrete pinned network block captured in a snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPin {
    /// Stable network identifier.
    pub network_id: String,
    /// EVM chain id.
    pub chain_id: u64,
    /// Concrete pinned block number.
    pub block_number: u64,
}

/// Canonical snapshot for one wallet inside a portfolio snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WalletSnapshot {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Canonical wallet address.
    pub address: String,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortfolioSnapshot {
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioReport {
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
        self.totals_by_quote
            .sort_by(|left, right| left.quote.cmp(&right.quote));
    }
}

/// Canonical per-wallet report summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
        self.totals_by_quote
            .sort_by(|left, right| left.quote.cmp(&right.quote));
    }
}

/// Canonical quote total used by wallet and portfolio reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Portfolio metadata violated canonical JSON rules.
    #[error("portfolio metadata must be canonical JSON: {reason}")]
    MetadataNotCanonical {
        /// Underlying canonical JSON failure.
        reason: CanonicalJsonError,
    },
    /// One of the network ids was empty.
    #[error("network_id must be non-empty")]
    EmptyNetworkId,
    /// Two networks shared the same network id.
    #[error("network_id `{network_id}` must be unique")]
    DuplicateNetworkId {
        /// Duplicate network id.
        network_id: String,
    },
    /// Network metadata violated canonical JSON rules.
    #[error("network `{network_id}` metadata must be canonical JSON: {reason}")]
    NetworkMetadataNotCanonical {
        /// Network that carried the invalid metadata.
        network_id: String,
        /// Underlying canonical JSON failure.
        reason: CanonicalJsonError,
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
    let cfg = serde_json::from_value(value.clone())
        .map_err(|err| PortfolioConfigError::Decode(err.to_string()))?;
    validate_portfolio_config(&cfg)?;
    Ok(cfg)
}

/// Validates a canonical portfolio config.
pub fn validate_portfolio_config(cfg: &PortfolioConfig) -> Result<(), PortfolioConfigError> {
    if cfg.portfolio_id.trim().is_empty() {
        return Err(PortfolioConfigError::EmptyPortfolioId);
    }
    validate_canonical_json_map(&cfg.metadata)
        .map_err(|reason| PortfolioConfigError::MetadataNotCanonical { reason })?;

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

/// Validates the canonical portfolio config together with its valuation source registry.
pub fn validate_portfolio_bundle(
    cfg: &PortfolioConfig,
    registry: &ValuationSourceRegistry,
) -> Result<(), PortfolioConfigError> {
    validate_portfolio_config(cfg)?;
    validate_valuation_source_registry(registry)
        .map_err(|source| PortfolioConfigError::InvalidValuationSourceRegistry { source })?;

    let network_ids: HashSet<_> = cfg
        .networks
        .iter()
        .map(|network| network.network_id.clone())
        .collect();
    let mut sources_by_id = BTreeMap::new();
    for source in &registry.sources {
        if !network_ids.contains(&source.network_id) {
            return Err(PortfolioConfigError::UnknownValuationSourceNetwork {
                source_id: source.source_id.clone(),
                network_id: source.network_id.clone(),
            });
        }
        sources_by_id.insert(source.source_id.clone(), source);
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
                        &sources_by_id,
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
                        &sources_by_id,
                    )?;
                    validate_price_source_registry_match(
                        symbol,
                        quote.quote,
                        denominator,
                        &sources_by_id,
                    )?;
                }
            }
        }
    }

    Ok(())
}

fn validate_network_config(network: &NetworkConfig) -> Result<(), PortfolioConfigError> {
    if network.network_id.trim().is_empty() {
        return Err(PortfolioConfigError::EmptyNetworkId);
    }
    validate_canonical_json_map(&network.metadata).map_err(|reason| {
        PortfolioConfigError::NetworkMetadataNotCanonical {
            network_id: network.network_id.clone(),
            reason,
        }
    })?;
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
    sources_by_id: &BTreeMap<String, &mfm_state_symbol::model::ValuationSourceConfig>,
) -> Result<(), PortfolioConfigError> {
    let Some(source_cfg) = sources_by_id.get(&source_ref.source_id) else {
        return Err(PortfolioConfigError::UnknownValuationSource {
            symbol_id: symbol.symbol_id.clone(),
            quote,
            source_id: source_ref.source_id.clone(),
        });
    };

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

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_machine::hashing::CanonicalJsonError;
    use mfm_state_symbol::model::{
        BalanceReaderConfig, ObservationQuantity, ObservationSource, ObservationValue,
        ObservationValueSourceRef, SymbolConfigError,
    };
    use mfm_state_wallet::model::{WalletConfigError, WalletImplementationConfig};
    use serde_json::json;

    #[test]
    fn decode_canonical_portfolio_config() {
        let cfg = decode_portfolio_config(&canonical_config_json()).expect("config should decode");

        assert_eq!(cfg.portfolio_id, "portfolio_main");
        assert_eq!(cfg.quote_codes, vec![QuoteCode::Usd, QuoteCode::Btc]);
        assert_eq!(cfg.networks.len(), 2);
        assert_eq!(cfg.wallets.len(), 2);
        assert_eq!(cfg.symbol_configs.len(), 3);
        assert!(matches!(
            cfg.wallets[0].implementation,
            WalletImplementationConfig::AddressOnly {}
        ));
        assert_eq!(
            cfg.wallets[0].address,
            "0x000000000000000000000000000000000000dead"
        );
        assert!(matches!(
            cfg.symbol_configs[0].balance_reader,
            BalanceReaderConfig::NativeBalance {}
        ));
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
    fn no_float_validation_rejects_metadata_and_protocol_blobs() {
        let mut portfolio_metadata = canonical_config_json();
        portfolio_metadata["metadata"] = json!({"threshold": 1.5});
        assert_eq!(
            decode_portfolio_config(&portfolio_metadata).unwrap_err(),
            PortfolioConfigError::MetadataNotCanonical {
                reason: CanonicalJsonError::FloatNotAllowed,
            }
        );

        let mut protocol_config = canonical_config_json();
        protocol_config["symbol_configs"][0]["balance_reader"] = json!({
            "kind": "protocol_position",
            "protocol": "aave_v3",
            "reader": "debt_position",
            "config": {
                "health_factor": 1.1
            }
        });
        assert_eq!(
            decode_portfolio_config(&protocol_config).unwrap_err(),
            PortfolioConfigError::InvalidSymbolConfig {
                symbol_id: "eth.native.ethereum-mainnet".to_string(),
                source: SymbolConfigError::ProtocolConfigNotCanonical {
                    reason: CanonicalJsonError::FloatNotAllowed,
                },
            }
        );

        let mut float_decimals = canonical_config_json();
        float_decimals["symbol_configs"][0]["decimals"] = json!(18.5);
        assert!(matches!(
            decode_portfolio_config(&float_decimals),
            Err(PortfolioConfigError::Decode(_))
        ));
    }

    #[test]
    fn validation_rejects_secrets_and_non_normalized_addresses() {
        let mut secret_metadata = canonical_config_json();
        secret_metadata["metadata"] = json!({
            "mnemonic": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        });
        assert_eq!(
            decode_portfolio_config(&secret_metadata).unwrap_err(),
            PortfolioConfigError::MetadataNotCanonical {
                reason: CanonicalJsonError::SecretsNotAllowed,
            }
        );

        let mut wallet_address = canonical_config_json();
        wallet_address["wallets"][0]["address"] =
            json!("0x000000000000000000000000000000000000DEAD");
        assert_eq!(
            decode_portfolio_config(&wallet_address).unwrap_err(),
            PortfolioConfigError::InvalidWalletConfig {
                wallet_id: "wallet_treasury_eth".to_string(),
                source: WalletConfigError::InvalidAddress {
                    address: "0x000000000000000000000000000000000000DEAD".to_string(),
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
            portfolio_id: "portfolio_main".to_string(),
            generated_at_ms: 1,
            network_pins: vec![
                NetworkPin {
                    network_id: "ethereum-mainnet".to_string(),
                    chain_id: 1,
                    block_number: 10,
                },
                NetworkPin {
                    network_id: "arbitrum-mainnet".to_string(),
                    chain_id: 42161,
                    block_number: 20,
                },
            ],
            wallets: vec![
                WalletSnapshot {
                    wallet_id: "wallet_treasury_eth".to_string(),
                    address: "0x000000000000000000000000000000000000dead".to_string(),
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
                    "chain_id": 1,
                    "metadata": {}
                },
                {
                    "network_id": "arbitrum-mainnet",
                    "chain_id": 42161,
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
            kind: mfm_state_symbol::model::SymbolKind::NativeBalance,
            role: mfm_state_symbol::model::SymbolRole::Native,
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
                        block_number: 1,
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
                block_number: 1,
            },
            metadata: BTreeMap::new(),
        }
    }
}
