use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::aave::AaveProtocolPositionConfig;
use crate::ids::{
    NetworkId, NormalizedEvmAddress, PortfolioScalarError, ProtocolId, ProtocolReaderId, SymbolId,
    UnitPriceDecimal,
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
    pub fn as_str(self) -> &'static str {
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

/// Canonical symbol kinds supported by the portfolio model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "symbol-kind",
    schema = "mfm.portfolio.symbol_kind"
)]
pub enum SymbolKind {
    /// Native balance on a network.
    NativeBalance,
    /// ERC-20 token balance.
    Erc20Balance,
    /// Protocol-backed position.
    ProtocolPosition,
    /// Staked position.
    StakedPosition,
}

/// Canonical exposure roles for a symbol observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "symbol-role",
    schema = "mfm.portfolio.symbol_role"
)]
pub enum SymbolRole {
    /// Native asset used for fees and transfers.
    Native,
    /// Plain asset exposure.
    Asset,
    /// Collateral exposure.
    Collateral,
    /// Debt exposure. Quantities stay positive and the role carries the semantics.
    Debt,
    /// Staked exposure.
    Staked,
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
    /// Symbol implementation kind.
    pub kind: SymbolKind,
    /// Exposure role.
    pub role: SymbolRole,
    /// Stable network identifier.
    pub network_id: NetworkId,
    /// Optional protocol identifier.
    pub protocol: Option<ProtocolId>,
    /// Balance reader selection.
    pub balance_reader: BalanceReaderConfig,
    /// Quote valuation routes for the symbol.
    pub valuation: SymbolValuationConfig,
    /// Optional underlying symbol for protocol-backed exposures.
    pub underlying_symbol_id: Option<SymbolId>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl SymbolConfig {
    /// Creates a normalized and validated symbol config.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        symbol_id: String,
        display_symbol: Option<String>,
        kind: SymbolKind,
        role: SymbolRole,
        network_id: String,
        protocol: Option<String>,
        balance_reader: BalanceReaderConfig,
        valuation: SymbolValuationConfig,
        underlying_symbol_id: Option<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, SymbolConfigError> {
        let symbol_id = SymbolId::new(symbol_id)
            .map_err(|source| SymbolConfigError::InvalidSymbolId { source })?;
        let network_id = NetworkId::new(network_id)
            .map_err(|source| SymbolConfigError::InvalidNetworkId { source })?;
        let underlying_symbol_id = underlying_symbol_id
            .map(SymbolId::new)
            .transpose()
            .map_err(|source| SymbolConfigError::InvalidUnderlyingSymbolId { source })?;
        let protocol = protocol
            .map(ProtocolId::new)
            .transpose()
            .map_err(|source| SymbolConfigError::InvalidProtocol { source })?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            SymbolConfigError::MetadataContainsSecret {
                key: source.key().to_owned(),
            }
        })?;
        Self {
            symbol_id,
            display_symbol,
            kind,
            role,
            network_id,
            protocol,
            balance_reader,
            valuation,
            underlying_symbol_id,
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

/// Canonical balance reader selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "balance-reader-config",
    schema = "mfm.portfolio.balance_reader_config"
)]
pub enum BalanceReaderConfig {
    /// Read the native balance for the wallet on the configured network.
    NativeBalance {},
    /// Read an ERC-20 balance.
    Erc20Balance {
        /// Canonical token contract address.
        token_address: NormalizedEvmAddress,
    },
    /// Read a protocol-backed position through a protocol-specific reader.
    ProtocolPosition {
        /// Stable protocol identifier.
        protocol: ProtocolId,
        /// Stable reader identifier inside the protocol module.
        reader: ProtocolReaderId,
        /// Typed protocol reader config.
        config: AaveProtocolPositionConfig,
    },
}

/// Quote valuation routes configured for a symbol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
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
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Exposure role.
    pub role: SymbolRole,
    /// Stable network identifier.
    pub network_id: String,
    /// Optional protocol identifier.
    pub protocol: Option<String>,
    /// Quantity metadata.
    pub quantity: ObservationQuantity,
    /// Values in configured quote units.
    pub values: Vec<ObservationValue>,
    /// Balance source information pinned to a block.
    pub source: ObservationSource,
    /// Selected holding coverage honesty tag (`configured_only` or `complete_at_anchor`).
    ///
    /// Populated from the selected Platform holding fact so public snapshot/report
    /// success cannot be misread as full wallet discovery when coverage is configured-only.
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

/// Concrete balance source pinned to a network anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-source",
    schema = "mfm.portfolio.observation_source"
)]
pub struct ObservationSource {
    /// Canonical balance reader kind.
    pub balance_reader_kind: String,
    /// Stable network identifier.
    pub network_id: String,
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
    /// `protocol` was present but empty.
    #[error("protocol is invalid: {source}")]
    InvalidProtocol {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `underlying_symbol_id` did not satisfy the portfolio identifier grammar.
    #[error("underlying_symbol_id is invalid: {source}")]
    InvalidUnderlyingSymbolId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// Symbol metadata contained a secret-shaped key or value.
    #[error("metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
    },
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
    match &cfg.balance_reader {
        BalanceReaderConfig::NativeBalance {} => {}
        BalanceReaderConfig::Erc20Balance { .. } => {}
        BalanceReaderConfig::ProtocolPosition { .. } => {}
    }

    let mut seen_quotes = HashSet::new();
    for quote in &cfg.valuation.quotes {
        if !seen_quotes.insert(quote.quote) {
            return Err(SymbolConfigError::DuplicateQuoteValuation { quote: quote.quote });
        }
    }

    Ok(())
}
