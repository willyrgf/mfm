use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::ids::{
    NetworkId, NormalizedEvmAddress, PortfolioScalarError, SymbolId, UnitPriceDecimal,
};
use crate::metadata::PublicMetadata;

/// Supported quote codes for the canonical portfolio snapshot surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "quote-code",
    schema = "mfm.portfolio.quote_code"
)]
pub enum QuoteCode {
    /// United States Dollar.
    #[serde(rename = "USD")]
    Usd,
    /// Bitcoin.
    #[serde(rename = "BTC")]
    Btc,
}

impl QuoteCode {
    /// Returns the canonical string form used in JSON payloads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Btc => "BTC",
        }
    }
}

impl fmt::Display for QuoteCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Ord for QuoteCode {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for QuoteCode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The only source algebra supported by the portfolio model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "holding-source-config",
    schema = "mfm.portfolio.holding_source_config"
)]
pub enum HoldingSourceConfig {
    /// The native balance for the configured wallet and network.
    Native,
    /// An ERC-20 balance for the configured wallet and EVM network.
    Erc20 {
        /// Canonical non-zero ERC-20 contract address.
        contract_address: NormalizedEvmAddress,
    },
}

struct PresentOptional<T> {
    value: Option<T>,
    is_present: bool,
}

impl<T> Default for PresentOptional<T> {
    fn default() -> Self {
        Self {
            value: None,
            is_present: false,
        }
    }
}

impl<'de, T> Deserialize<'de> for PresentOptional<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self {
            value: Option::<T>::deserialize(deserializer)?,
            is_present: true,
        })
    }
}

impl<'de> Deserialize<'de> for HoldingSourceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct HoldingSourceWire {
            kind: String,
            #[serde(default)]
            contract_address: PresentOptional<NormalizedEvmAddress>,
        }

        let wire = HoldingSourceWire::deserialize(deserializer)?;
        match wire.kind.as_str() {
            "native" => {
                if wire.contract_address.is_present {
                    return Err(serde::de::Error::custom(
                        "native holding sources do not accept contract_address",
                    ));
                }
                Ok(Self::Native)
            }
            "erc20" => {
                let contract_address = wire.contract_address.value.ok_or_else(|| {
                    serde::de::Error::custom("erc20 holding sources require contract_address")
                })?;
                Ok(Self::Erc20 { contract_address })
            }
            _ => Err(serde::de::Error::custom(
                "holding source kind must be native or erc20",
            )),
        }
    }
}

impl HoldingSourceConfig {
    /// Returns whether this source is the native network balance.
    pub const fn is_native(&self) -> bool {
        matches!(self, Self::Native)
    }

    /// Returns the ERC-20 contract address when this is an ERC-20 source.
    pub const fn contract_address(&self) -> Option<&NormalizedEvmAddress> {
        match self {
            Self::Native => None,
            Self::Erc20 { contract_address } => Some(contract_address),
        }
    }
}

/// Canonical symbol configuration referenced from portfolio and wallet configs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "symbol-config",
    schema = "mfm.portfolio.symbol_config"
)]
pub struct SymbolConfig {
    /// Stable machine identifier for the symbol.
    pub symbol_id: SymbolId,
    /// Optional human-facing display symbol.
    pub display_symbol: Option<String>,
    /// Stable network identifier.
    pub network_id: NetworkId,
    /// Direct balance source.
    pub source: HoldingSourceConfig,
    /// Quote valuation routes for the symbol.
    pub valuation: SymbolValuationConfig,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl SymbolConfig {
    /// Creates a normalized and validated symbol config.
    pub fn new(
        symbol_id: String,
        display_symbol: Option<String>,
        network_id: String,
        source: HoldingSourceConfig,
        valuation: SymbolValuationConfig,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, SymbolConfigError> {
        let symbol_id = SymbolId::new(symbol_id)
            .map_err(|source| SymbolConfigError::InvalidSymbolId { source })?;
        let network_id = NetworkId::new(network_id)
            .map_err(|source| SymbolConfigError::InvalidNetworkId { source })?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            SymbolConfigError::MetadataContainsSecret {
                key: source.key().to_owned(),
            }
        })?;
        Self {
            symbol_id,
            display_symbol,
            network_id,
            source,
            valuation,
            metadata,
        }
        .validated()
    }

    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.valuation.normalize();
    }

    /// Returns a normalized clone of the symbol config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Validates this symbol config, normalizes it, and returns the validated value.
    pub fn validated(mut self) -> Result<Self, SymbolConfigError> {
        validate_symbol_config(&self)?;
        self.normalize();
        Ok(self)
    }
}

/// Quote valuation routes configured for a symbol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "symbol-valuation-config",
    schema = "mfm.portfolio.symbol_valuation_config"
)]
pub struct SymbolValuationConfig {
    /// One route per requested quote code.
    pub quotes: Vec<QuoteValuationConfig>,
}

impl SymbolValuationConfig {
    /// Sorts quote routes by canonical quote code.
    pub fn normalize(&mut self) {
        self.quotes.sort_by_key(|quote| quote.quote);
    }
}

/// Valuation route for one requested quote code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "quote-valuation-config",
    schema = "mfm.portfolio.quote_valuation_config"
)]
pub struct QuoteValuationConfig {
    /// Requested quote unit.
    pub quote: QuoteCode,
    /// Symbol whose unit price applies to the observation.
    pub priced_symbol_id: SymbolId,
    /// Fixed unit price as a decimal string (cutover valuation surface).
    pub unit_price_dec: UnitPriceDecimal,
}

/// Canonical wallet observation emitted by runtime states.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation",
    schema = "mfm.portfolio.observation"
)]
pub struct Observation {
    /// Wallet that owns the observation.
    pub wallet_id: String,
    /// Canonical symbol identifier.
    pub symbol_id: String,
    /// Optional human-facing symbol display value.
    pub display_symbol: Option<String>,
    /// Stable network identifier.
    pub network_id: String,
    /// Quantity metadata.
    pub quantity: ObservationQuantity,
    /// Values in configured quote units.
    pub values: Vec<ObservationValue>,
    /// Direct holding source information pinned to a concrete anchor.
    pub source: AnchoredHoldingSource,
    /// Selected holding coverage honesty tag (`configured_only` or `complete_at_anchor`).
    pub coverage: String,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl Observation {
    /// Sorts nested value collections into canonical order.
    pub fn normalize(&mut self) {
        self.values.sort_by_key(|value| value.quote);
    }
}

/// Canonical quantity representation for an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-quantity",
    schema = "mfm.portfolio.observation_quantity"
)]
pub struct ObservationQuantity {
    /// Raw on-chain integer encoded as a decimal string.
    pub raw_dec: String,
    /// Token decimals used for rendering.
    pub decimals: u8,
    /// Human-readable decimal amount string.
    pub amount_dec: String,
}

/// Canonical value representation for one quote unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-value",
    schema = "mfm.portfolio.observation_value"
)]
pub struct ObservationValue {
    /// Quote unit.
    pub quote: QuoteCode,
    /// Symbol whose unit price was applied.
    pub priced_symbol_id: String,
    /// Total value in the quote unit.
    pub value_dec: String,
    /// Unit price in the quote unit.
    pub unit_price_dec: String,
}

/// Concrete execution anchor captured for one observation source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "family", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-anchor",
    schema = "mfm.portfolio.observation_anchor"
)]
pub enum ObservationAnchor {
    /// EVM observation pinned to one block hash and number on one chain.
    Evm {
        /// EVM chain id.
        chain_id: u64,
        /// Concrete pinned block number.
        block_number: u64,
        /// Concrete pinned block hash.
        block_hash: String,
    },
    /// Bitcoin observation pinned to one height and block hash.
    Bitcoin {
        /// Concrete pinned block height.
        height: u64,
        /// Concrete pinned block hash.
        block_hash: String,
    },
}

/// One direct holding source pinned to a concrete observation anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "anchored-holding-source",
    schema = "mfm.portfolio.anchored_holding_source"
)]
pub struct AnchoredHoldingSource {
    /// The direct semantic holding source.
    pub holding: HoldingSourceConfig,
    /// Concrete pinned execution anchor.
    pub anchor: ObservationAnchor,
}

/// Validation errors for canonical symbol configs.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SymbolConfigError {
    /// The JSON payload could not be decoded into the canonical type.
    #[error("symbol config decode failed: {0}")]
    Decode(String),
    /// `symbol_id` did not satisfy the portfolio identifier grammar.
    #[error("symbol_id is invalid: {source}")]
    InvalidSymbolId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `network_id` did not satisfy the portfolio identifier grammar.
    #[error("network_id is invalid: {source}")]
    InvalidNetworkId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// Symbol metadata contained a secret-shaped key or value.
    #[error("metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// An ERC-20 contract address was the all-zero address.
    #[error("ERC-20 contract_address must not be the zero address")]
    ZeroErc20ContractAddress,
    /// A quote route appeared more than once.
    #[error("valuation quote `{quote}` must be unique per symbol")]
    DuplicateQuoteValuation {
        /// Duplicate quote code.
        quote: QuoteCode,
    },
    /// `priced_symbol_id` did not satisfy the portfolio identifier grammar.
    #[error("priced_symbol_id is invalid for quote `{quote}`: {source}")]
    InvalidPricedSymbolId {
        /// Quote whose route was invalid.
        quote: QuoteCode,
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
}

/// Validates a canonical symbol config.
pub fn validate_symbol_config(cfg: &SymbolConfig) -> Result<(), SymbolConfigError> {
    if let HoldingSourceConfig::Erc20 { contract_address } = &cfg.source {
        if contract_address.is_zero() {
            return Err(SymbolConfigError::ZeroErc20ContractAddress);
        }
    }

    let mut seen_quotes = HashSet::new();
    for quote in &cfg.valuation.quotes {
        if !seen_quotes.insert(quote.quote) {
            return Err(SymbolConfigError::DuplicateQuoteValuation { quote: quote.quote });
        }
    }

    Ok(())
}
