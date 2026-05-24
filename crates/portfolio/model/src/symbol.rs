use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

use mfm_evm_core::encoding::normalize_address;
use mfm_program_derive::MfmValue;
use mfm_values::string_map_secret_marker_key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::aave::AaveProtocolPositionConfig;

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
#[mfm(
    namespace = "mfm.portfolio",
    name = "symbol-config",
    schema = "mfm.portfolio.symbol_config"
)]
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
    pub metadata: BTreeMap<String, String>,
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
        token_address: String,
    },
    /// Read a protocol-backed position through a protocol-specific reader.
    ProtocolPosition {
        /// Stable protocol identifier.
        protocol: String,
        /// Stable reader identifier inside the protocol module.
        reader: String,
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
        self.quotes
            .sort_by(|left, right| left.quote.cmp(&right.quote));
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
    pub priced_symbol_id: String,
    /// Reader configuration used to obtain the unit price.
    pub reader: ValuationReaderConfig,
}

/// Source reference used by direct and derived valuation readers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "price-source-ref",
    schema = "mfm.portfolio.price_source_ref"
)]
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

/// Typed registry of valuation sources referenced by symbol valuation routes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "valuation-source-registry",
    schema = "mfm.portfolio.valuation_source_registry"
)]
pub struct ValuationSourceRegistry {
    /// Configured valuation sources keyed by `source_id`.
    pub sources: Vec<ValuationSourceConfig>,
}

impl ValuationSourceRegistry {
    /// Sorts nested collections into the canonical order used for persistence.
    pub fn normalize(&mut self) {
        self.sources
            .sort_by(|left, right| left.source_id.cmp(&right.source_id));
    }

    /// Returns a normalized clone of the valuation source registry.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }
}

/// Typed registry entry for one valuation source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "valuation-source-config",
    schema = "mfm.portfolio.valuation_source_config"
)]
pub struct ValuationSourceConfig {
    /// Stable logical source identifier.
    pub source_id: String,
    /// Network on which the source must be pinned.
    pub network_id: String,
    /// Symbol identity used by the source.
    pub base_symbol_id: String,
    /// Quote unit returned by the source.
    pub quote: QuoteCode,
    /// Reader family and its typed config blob.
    pub reader: ValuationSourceReaderConfig,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Resolved balance reader selected by runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-symbol-balance-reader",
    schema = "mfm.portfolio.resolved_symbol_balance_reader"
)]
pub struct ResolvedSymbolBalanceReader {
    /// Canonical runtime reader kind.
    pub kind: String,
    /// Opaque runtime implementation reference.
    pub implementation_ref: String,
}

/// Resolved valuation reader selected by runtime planning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "resolved-symbol-valuation-reader",
    schema = "mfm.portfolio.resolved_symbol_valuation_reader"
)]
pub struct ResolvedSymbolValuationReader {
    /// Quote handled by the resolved reader.
    pub quote: QuoteCode,
    /// Canonical runtime reader kind.
    pub kind: String,
    /// Opaque runtime implementation reference.
    pub implementation_ref: String,
}

/// Supported valuation source reader kinds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "valuation-source-reader-config",
    schema = "mfm.portfolio.valuation_source_reader_config"
)]
pub enum ValuationSourceReaderConfig {
    /// EVM oracle reader selected by `oracle_kind`.
    EvmOracle {
        /// Concrete oracle family used at runtime.
        oracle_kind: String,
        /// Typed oracle config.
        config: EvmOracleConfig,
    },
}

/// Typed EVM oracle reader config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm-oracle-config",
    schema = "mfm.portfolio.evm_oracle_config"
)]
pub struct EvmOracleConfig {
    /// Normalized oracle contract address.
    pub contract_address: String,
}

/// Supported valuation reader kinds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "valuation-reader-config",
    schema = "mfm.portfolio.valuation_reader_config"
)]
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
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl Observation {
    /// Sorts nested value collections into canonical order.
    pub fn normalize(&mut self) {
        self.values
            .sort_by(|left, right| left.quote.cmp(&right.quote));
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
    /// Canonical valuation reader kind.
    pub valuation_reader_kind: String,
    /// Concrete source refs used for the valuation.
    pub source_refs: Vec<ObservationValueSourceRef>,
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
    /// EVM observation pinned to one block on one chain.
    Evm {
        /// EVM chain id.
        chain_id: u64,
        /// Concrete pinned block number.
        block_number: u64,
    },
    /// Bitcoin observation pinned to one height and block hash.
    Bitcoin {
        /// Concrete pinned block height.
        height: u64,
        /// Concrete pinned block hash.
        block_hash: String,
    },
}

/// Concrete price source ref pinned to a network anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "observation-value-source-ref",
    schema = "mfm.portfolio.observation_value_source_ref"
)]
pub struct ObservationValueSourceRef {
    /// Stable source identifier.
    pub source_id: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Concrete pinned execution anchor.
    pub anchor: ObservationAnchor,
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
    /// Symbol metadata contained a secret-shaped key or value.
    #[error("metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Metadata key associated with the rejected content.
        key: String,
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

/// Validation errors for valuation source registries.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ValuationSourceRegistryError {
    /// The JSON payload could not be decoded into the canonical type.
    #[error("valuation source registry decode failed: {0}")]
    Decode(String),
    /// `source_id` was empty.
    #[error("source_id must be non-empty")]
    EmptySourceId,
    /// `network_id` was empty.
    #[error("network_id must be non-empty")]
    EmptyNetworkId,
    /// `base_symbol_id` was empty.
    #[error("base_symbol_id must be non-empty")]
    EmptyBaseSymbolId,
    /// Registry metadata contained a secret-shaped key or value.
    #[error("valuation source `{source_id}` metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Valuation source associated with the rejected metadata.
        source_id: String,
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// `oracle_kind` was empty.
    #[error("evm_oracle.oracle_kind must be non-empty")]
    EmptyOracleKind,
    /// Oracle contract address was invalid or not normalized.
    #[error(
        "evm_oracle.config.contract_address must be a normalized EVM address: {contract_address}"
    )]
    InvalidOracleContractAddress {
        /// Invalid oracle contract address input.
        contract_address: String,
    },
    /// The same `source_id` appeared more than once.
    #[error("valuation source `{source_id}` must be unique")]
    DuplicateSourceId {
        /// Duplicate valuation source id.
        source_id: String,
    },
}

/// Decodes and validates a canonical symbol config.
pub fn decode_symbol_config(value: &Value) -> Result<SymbolConfig, SymbolConfigError> {
    let cfg = serde_json::from_value(value.clone())
        .map_err(|err| SymbolConfigError::Decode(err.to_string()))?;
    validate_symbol_config(&cfg)?;
    Ok(cfg)
}

/// Decodes and validates a valuation source registry.
pub fn decode_valuation_source_registry(
    value: &Value,
) -> Result<ValuationSourceRegistry, ValuationSourceRegistryError> {
    let registry = serde_json::from_value(value.clone())
        .map_err(|err| ValuationSourceRegistryError::Decode(err.to_string()))?;
    validate_valuation_source_registry(&registry)?;
    Ok(registry)
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
    if let Some(key) = string_map_secret_marker_key(&cfg.metadata) {
        return Err(SymbolConfigError::MetadataContainsSecret {
            key: key.to_string(),
        });
    }

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
            config: _,
        } => {
            if protocol.trim().is_empty() {
                return Err(SymbolConfigError::EmptyProtocolReaderProtocol);
            }
            if reader.trim().is_empty() {
                return Err(SymbolConfigError::EmptyProtocolReader);
            }
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

/// Validates a valuation source registry.
pub fn validate_valuation_source_registry(
    registry: &ValuationSourceRegistry,
) -> Result<(), ValuationSourceRegistryError> {
    let mut seen_ids = HashSet::new();
    for source in &registry.sources {
        if source.source_id.trim().is_empty() {
            return Err(ValuationSourceRegistryError::EmptySourceId);
        }
        if !seen_ids.insert(source.source_id.clone()) {
            return Err(ValuationSourceRegistryError::DuplicateSourceId {
                source_id: source.source_id.clone(),
            });
        }
        if source.network_id.trim().is_empty() {
            return Err(ValuationSourceRegistryError::EmptyNetworkId);
        }
        if source.base_symbol_id.trim().is_empty() {
            return Err(ValuationSourceRegistryError::EmptyBaseSymbolId);
        }
        if let Some(key) = string_map_secret_marker_key(&source.metadata) {
            return Err(ValuationSourceRegistryError::MetadataContainsSecret {
                source_id: source.source_id.clone(),
                key: key.to_string(),
            });
        }
        match &source.reader {
            ValuationSourceReaderConfig::EvmOracle {
                oracle_kind,
                config,
            } => {
                if oracle_kind.trim().is_empty() {
                    return Err(ValuationSourceRegistryError::EmptyOracleKind);
                }
                validate_normalized_address(&config.contract_address).map_err(|_| {
                    ValuationSourceRegistryError::InvalidOracleContractAddress {
                        contract_address: config.contract_address.clone(),
                    }
                })?;
            }
        }
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

pub(crate) fn validate_normalized_address(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("address must be non-empty".to_string());
    }
    let normalized = normalize_address(raw).map_err(|err| err.message)?;
    if normalized != raw {
        return Err("address must already be normalized".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valuation_source_metadata_rejects_secret_markers() {
        let mut metadata = BTreeMap::new();
        metadata.insert("authorization".to_string(), "redacted".to_string());

        let registry = ValuationSourceRegistry {
            sources: vec![ValuationSourceConfig {
                source_id: "chainlink_eth_usd".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                base_symbol_id: "eth.native.ethereum-mainnet".to_string(),
                quote: QuoteCode::Usd,
                reader: ValuationSourceReaderConfig::EvmOracle {
                    oracle_kind: "chainlink".to_string(),
                    config: EvmOracleConfig {
                        contract_address: "0x0000000000000000000000000000000000000001".to_string(),
                    },
                },
                metadata,
            }],
        };

        assert_eq!(
            validate_valuation_source_registry(&registry).unwrap_err(),
            ValuationSourceRegistryError::MetadataContainsSecret {
                source_id: "chainlink_eth_usd".to_string(),
                key: "authorization".to_string(),
            }
        );
    }
}
