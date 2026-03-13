use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

use mfm_evm_core::encoding::normalize_address;
use mfm_machine::hashing::{canonical_json_bytes, CanonicalJsonError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Supported quote codes for the canonical portfolio snapshot surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolConfig {
    /// Stable machine identifier for the symbol.
    pub symbol_id: String,
    /// Optional human-facing display symbol.
    pub display_symbol: Option<String>,
    /// Symbol implementation kind.
    pub kind: SymbolKind,
    /// Exposure role.
    pub role: SymbolRole,
    /// Stable network identifier.
    pub network_id: String,
    /// Optional protocol identifier.
    pub protocol: Option<String>,
    /// Balance reader selection.
    pub balance_reader: BalanceReaderConfig,
    /// Quote valuation routes for the symbol.
    pub valuation: SymbolValuationConfig,
    /// Optional decimals used to render raw quantities.
    pub decimals: Option<u8>,
    /// Optional underlying symbol for protocol-backed exposures.
    pub underlying_symbol_id: Option<String>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl SymbolConfig {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.valuation.normalize();
    }

    /// Returns a normalized clone of the symbol config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }
}

/// Canonical balance reader selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BalanceReaderConfig {
    /// Read the native balance for the wallet on the configured network.
    NativeBalance {},
    /// Read an ERC-20 balance.
    Erc20Balance {
        /// Canonical token contract address.
        token_address: String,
    },
    /// Read a protocol-backed position through a protocol-specific reader.
    ProtocolPosition {
        /// Stable protocol identifier.
        protocol: String,
        /// Stable reader identifier inside the protocol module.
        reader: String,
        /// Canonical protocol reader config blob.
        #[serde(default)]
        config: BTreeMap<String, Value>,
    },
}

/// Quote valuation routes configured for a symbol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolValuationConfig {
    /// One route per requested quote code.
    pub quotes: Vec<QuoteValuationConfig>,
}

impl SymbolValuationConfig {
    /// Sorts quote routes by canonical quote code.
    pub fn normalize(&mut self) {
        self.quotes
            .sort_by(|left, right| left.quote.cmp(&right.quote));
    }
}

/// Valuation route for one requested quote code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuoteValuationConfig {
    /// Requested quote unit.
    pub quote: QuoteCode,
    /// Symbol whose unit price applies to the observation.
    pub priced_symbol_id: String,
    /// Reader configuration used to obtain the unit price.
    pub reader: ValuationReaderConfig,
}

/// Source reference used by direct and derived valuation readers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceSourceRef {
    /// Stable source identifier within the runtime environment.
    pub source_id: String,
    /// Network on which the source must be pinned.
    pub network_id: String,
    /// Symbol identity used by the source.
    pub base_symbol_id: String,
    /// Quote unit returned by the source.
    pub quote: QuoteCode,
}

/// Resolved balance reader selected by runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSymbolBalanceReader {
    /// Canonical runtime reader kind.
    pub kind: String,
    /// Opaque runtime implementation reference.
    pub implementation_ref: String,
}

/// Resolved valuation reader selected by runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSymbolValuationReader {
    /// Quote handled by the resolved reader.
    pub quote: QuoteCode,
    /// Canonical runtime reader kind.
    pub kind: String,
    /// Opaque runtime implementation reference.
    pub implementation_ref: String,
}

/// Supported valuation reader kinds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValuationReaderConfig {
    /// Fixed unit price encoded as a decimal string.
    FixedUnitPrice {
        /// Decimal string for the unit price.
        unit_price_dec: String,
    },
    /// Direct unit price from one source.
    DirectPrice {
        /// Price source definition.
        source: PriceSourceRef,
    },
    /// Derived unit price computed from numerator and denominator sources.
    DerivedUnitPrice {
        /// Numerator source definition.
        numerator: PriceSourceRef,
        /// Denominator source definition.
        denominator: PriceSourceRef,
    },
}

/// Canonical wallet observation emitted by runtime states.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

impl Observation {
    /// Sorts nested value collections into canonical order.
    pub fn normalize(&mut self) {
        self.values
            .sort_by(|left, right| left.quote.cmp(&right.quote));
    }
}

/// Canonical quantity representation for an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationQuantity {
    /// Raw on-chain integer encoded as a decimal string.
    pub raw_dec: String,
    /// Token decimals used for rendering.
    pub decimals: u8,
    /// Human-readable decimal amount string.
    pub amount_dec: String,
}

/// Canonical value representation for one quote unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationValue {
    /// Quote unit.
    pub quote: QuoteCode,
    /// Symbol whose unit price was applied.
    pub priced_symbol_id: String,
    /// Total value in the quote unit.
    pub value_dec: String,
    /// Unit price in the quote unit.
    pub unit_price_dec: String,
    /// Canonical valuation reader kind.
    pub valuation_reader_kind: String,
    /// Concrete source refs used for the valuation.
    pub source_refs: Vec<ObservationValueSourceRef>,
}

/// Concrete price source ref pinned to a network block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationValueSourceRef {
    /// Stable source identifier.
    pub source_id: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Concrete pinned block number.
    pub block_number: u64,
}

/// Concrete balance source pinned to a network block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationSource {
    /// Canonical balance reader kind.
    pub balance_reader_kind: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Concrete pinned block number.
    pub block_number: u64,
}

/// Validation errors for canonical symbol configs.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SymbolConfigError {
    /// The JSON payload could not be decoded into the canonical type.
    #[error("symbol config decode failed: {0}")]
    Decode(String),
    /// `symbol_id` was empty.
    #[error("symbol_id must be non-empty")]
    EmptySymbolId,
    /// `network_id` was empty.
    #[error("network_id must be non-empty")]
    EmptyNetworkId,
    /// `protocol` was present but empty.
    #[error("protocol must be non-empty when present")]
    EmptyProtocol,
    /// `underlying_symbol_id` was present but empty.
    #[error("underlying_symbol_id must be non-empty when present")]
    EmptyUnderlyingSymbolId,
    /// Symbol metadata violated canonical JSON rules.
    #[error("metadata must be canonical JSON: {reason}")]
    MetadataNotCanonical {
        /// Underlying canonical JSON failure.
        reason: CanonicalJsonError,
    },
    /// ERC-20 token address was invalid or not normalized.
    #[error("token_address must be a normalized EVM address: {token_address}")]
    InvalidTokenAddress {
        /// Invalid token address input.
        token_address: String,
    },
    /// `protocol` in a `protocol_position` reader was empty.
    #[error("protocol_position.protocol must be non-empty")]
    EmptyProtocolReaderProtocol,
    /// `reader` in a `protocol_position` reader was empty.
    #[error("protocol_position.reader must be non-empty")]
    EmptyProtocolReader,
    /// Protocol reader config violated canonical JSON rules.
    #[error("protocol_position.config must be canonical JSON: {reason}")]
    ProtocolConfigNotCanonical {
        /// Underlying canonical JSON failure.
        reason: CanonicalJsonError,
    },
    /// A quote route appeared more than once.
    #[error("valuation quote `{quote}` must be unique per symbol")]
    DuplicateQuoteValuation {
        /// Duplicate quote code.
        quote: QuoteCode,
    },
    /// `priced_symbol_id` was empty for a quote route.
    #[error("priced_symbol_id must be non-empty for quote `{quote}`")]
    EmptyPricedSymbolId {
        /// Quote whose route was invalid.
        quote: QuoteCode,
    },
    /// `fixed_unit_price.unit_price_dec` was empty.
    #[error("fixed_unit_price.unit_price_dec must be non-empty for quote `{quote}`")]
    EmptyFixedUnitPrice {
        /// Quote whose route was invalid.
        quote: QuoteCode,
    },
    /// `source_id` was empty on a price source ref.
    #[error("{field}.source_id must be non-empty for quote `{quote}`")]
    EmptyPriceSourceId {
        /// Quote whose route was invalid.
        quote: QuoteCode,
        /// Field name that owned the invalid source ref.
        field: &'static str,
    },
    /// `network_id` was empty on a price source ref.
    #[error("{field}.network_id must be non-empty for quote `{quote}`")]
    EmptyPriceSourceNetworkId {
        /// Quote whose route was invalid.
        quote: QuoteCode,
        /// Field name that owned the invalid source ref.
        field: &'static str,
    },
    /// `base_symbol_id` was empty on a price source ref.
    #[error("{field}.base_symbol_id must be non-empty for quote `{quote}`")]
    EmptyPriceSourceBaseSymbolId {
        /// Quote whose route was invalid.
        quote: QuoteCode,
        /// Field name that owned the invalid source ref.
        field: &'static str,
    },
    /// Derived price source quotes did not match.
    #[error(
        "derived_unit_price quotes must match for quote `{quote}` (got `{numerator_quote}` and `{denominator_quote}`)"
    )]
    DerivedPriceQuoteMismatch {
        /// Quote whose route was invalid.
        quote: QuoteCode,
        /// Numerator quote code.
        numerator_quote: QuoteCode,
        /// Denominator quote code.
        denominator_quote: QuoteCode,
    },
}

/// Decodes and validates a canonical symbol config.
pub fn decode_symbol_config(value: &Value) -> Result<SymbolConfig, SymbolConfigError> {
    let cfg = serde_json::from_value(value.clone())
        .map_err(|err| SymbolConfigError::Decode(err.to_string()))?;
    validate_symbol_config(&cfg)?;
    Ok(cfg)
}

/// Validates a canonical symbol config.
pub fn validate_symbol_config(cfg: &SymbolConfig) -> Result<(), SymbolConfigError> {
    if cfg.symbol_id.trim().is_empty() {
        return Err(SymbolConfigError::EmptySymbolId);
    }
    if cfg.network_id.trim().is_empty() {
        return Err(SymbolConfigError::EmptyNetworkId);
    }
    if cfg
        .protocol
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(SymbolConfigError::EmptyProtocol);
    }
    if cfg
        .underlying_symbol_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(SymbolConfigError::EmptyUnderlyingSymbolId);
    }

    validate_canonical_json_map(&cfg.metadata)
        .map_err(|reason| SymbolConfigError::MetadataNotCanonical { reason })?;

    match &cfg.balance_reader {
        BalanceReaderConfig::NativeBalance {} => {}
        BalanceReaderConfig::Erc20Balance { token_address } => {
            validate_normalized_address(token_address).map_err(|_| {
                SymbolConfigError::InvalidTokenAddress {
                    token_address: token_address.clone(),
                }
            })?;
        }
        BalanceReaderConfig::ProtocolPosition {
            protocol,
            reader,
            config,
        } => {
            if protocol.trim().is_empty() {
                return Err(SymbolConfigError::EmptyProtocolReaderProtocol);
            }
            if reader.trim().is_empty() {
                return Err(SymbolConfigError::EmptyProtocolReader);
            }
            validate_canonical_json_map(config)
                .map_err(|reason| SymbolConfigError::ProtocolConfigNotCanonical { reason })?;
        }
    }

    let mut seen_quotes = HashSet::new();
    for quote in &cfg.valuation.quotes {
        if !seen_quotes.insert(quote.quote) {
            return Err(SymbolConfigError::DuplicateQuoteValuation { quote: quote.quote });
        }
        if quote.priced_symbol_id.trim().is_empty() {
            return Err(SymbolConfigError::EmptyPricedSymbolId { quote: quote.quote });
        }
        validate_valuation_reader_config(quote.quote, &quote.reader)?;
    }

    Ok(())
}

fn validate_valuation_reader_config(
    quote: QuoteCode,
    reader: &ValuationReaderConfig,
) -> Result<(), SymbolConfigError> {
    match reader {
        ValuationReaderConfig::FixedUnitPrice { unit_price_dec } => {
            if unit_price_dec.trim().is_empty() {
                return Err(SymbolConfigError::EmptyFixedUnitPrice { quote });
            }
        }
        ValuationReaderConfig::DirectPrice { source } => {
            validate_price_source_ref(quote, "source", source)?;
        }
        ValuationReaderConfig::DerivedUnitPrice {
            numerator,
            denominator,
        } => {
            validate_price_source_ref(quote, "numerator", numerator)?;
            validate_price_source_ref(quote, "denominator", denominator)?;
            if numerator.quote != denominator.quote {
                return Err(SymbolConfigError::DerivedPriceQuoteMismatch {
                    quote,
                    numerator_quote: numerator.quote,
                    denominator_quote: denominator.quote,
                });
            }
        }
    }

    Ok(())
}

fn validate_price_source_ref(
    quote: QuoteCode,
    field: &'static str,
    source: &PriceSourceRef,
) -> Result<(), SymbolConfigError> {
    if source.source_id.trim().is_empty() {
        return Err(SymbolConfigError::EmptyPriceSourceId { quote, field });
    }
    if source.network_id.trim().is_empty() {
        return Err(SymbolConfigError::EmptyPriceSourceNetworkId { quote, field });
    }
    if source.base_symbol_id.trim().is_empty() {
        return Err(SymbolConfigError::EmptyPriceSourceBaseSymbolId { quote, field });
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

fn validate_normalized_address(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("address must be non-empty".to_string());
    }
    let normalized = normalize_address(raw).map_err(|err| err.message)?;
    if normalized != raw {
        return Err("address must already be normalized".to_string());
    }
    Ok(())
}
