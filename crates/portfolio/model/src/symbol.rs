use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::aave::AaveProtocolPositionConfig;
use crate::ids::{
    NetworkId, NormalizedEvmAddress, OracleKindId, PortfolioScalarError, ProtocolId,
    ProtocolReaderId, SymbolId, UnitPriceDecimal, ValuationSourceId,
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
    /// Optional decimals used to render raw quantities.
    pub decimals: Option<u8>,
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
        decimals: Option<u8>,
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
            decimals,
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
    pub source_id: ValuationSourceId,
    /// Network on which the source must be pinned.
    pub network_id: NetworkId,
    /// Symbol identity used by the source.
    pub base_symbol_id: SymbolId,
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
    /// Creates a normalized and validated valuation source registry.
    pub fn new(sources: Vec<ValuationSourceConfig>) -> Result<Self, ValuationSourceRegistryError> {
        Self { sources }.validated()
    }

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

    /// Validates this registry, normalizes it, and returns the validated value.
    pub fn validated(mut self) -> Result<Self, ValuationSourceRegistryError> {
        validate_valuation_source_registry(&self)?;
        self.normalize();
        Ok(self)
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
    pub source_id: ValuationSourceId,
    /// Network on which the source must be pinned.
    pub network_id: NetworkId,
    /// Symbol identity used by the source.
    pub base_symbol_id: SymbolId,
    /// Quote unit returned by the source.
    pub quote: QuoteCode,
    /// Reader family and its typed config blob.
    pub reader: ValuationSourceReaderConfig,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl ValuationSourceConfig {
    /// Creates a validated valuation source config.
    pub fn new(
        source_id: String,
        network_id: String,
        base_symbol_id: String,
        quote: QuoteCode,
        reader: ValuationSourceReaderConfig,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, ValuationSourceRegistryError> {
        let source_id = ValuationSourceId::new(source_id)
            .map_err(|source| ValuationSourceRegistryError::InvalidSourceId { source })?;
        let network_id = NetworkId::new(network_id)
            .map_err(|source| ValuationSourceRegistryError::InvalidNetworkId { source })?;
        let base_symbol_id = SymbolId::new(base_symbol_id)
            .map_err(|source| ValuationSourceRegistryError::InvalidBaseSymbolId { source })?;
        let metadata = PublicMetadata::new(metadata).map_err(|source| {
            ValuationSourceRegistryError::MetadataContainsSecret {
                source_id: source_id.to_string(),
                key: source.key().to_owned(),
            }
        })?;
        let registry = ValuationSourceRegistry {
            sources: vec![Self {
                source_id,
                network_id,
                base_symbol_id,
                quote,
                reader,
                metadata,
            }],
        };
        let mut sources = registry.validated()?.sources;
        Ok(sources.remove(0))
    }
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
        oracle_kind: OracleKindId,
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
    pub contract_address: NormalizedEvmAddress,
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
        unit_price_dec: UnitPriceDecimal,
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
    /// `source_id` did not satisfy the portfolio identifier grammar.
    #[error("source_id is invalid: {source}")]
    InvalidSourceId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `network_id` did not satisfy the portfolio identifier grammar.
    #[error("network_id is invalid: {source}")]
    InvalidNetworkId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// `base_symbol_id` did not satisfy the portfolio identifier grammar.
    #[error("base_symbol_id is invalid: {source}")]
    InvalidBaseSymbolId {
        /// Underlying scalar validation failure.
        source: PortfolioScalarError,
    },
    /// Registry metadata contained a secret-shaped key or value.
    #[error("valuation source `{source_id}` metadata key `{key}` contains secret-shaped content")]
    MetadataContainsSecret {
        /// Valuation source associated with the rejected metadata.
        source_id: String,
        /// Metadata key associated with the rejected content.
        key: String,
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
    let cfg: SymbolConfig = serde_json::from_value(value.clone())
        .map_err(|err| SymbolConfigError::Decode(err.to_string()))?;
    cfg.validated()
}

/// Decodes and validates a valuation source registry.
pub fn decode_valuation_source_registry(
    value: &Value,
) -> Result<ValuationSourceRegistry, ValuationSourceRegistryError> {
    let registry: ValuationSourceRegistry = serde_json::from_value(value.clone())
        .map_err(|err| ValuationSourceRegistryError::Decode(err.to_string()))?;
    registry.validated()
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
        if !seen_ids.insert(source.source_id.clone()) {
            return Err(ValuationSourceRegistryError::DuplicateSourceId {
                source_id: source.source_id.to_string(),
            });
        }
    }

    Ok(())
}

fn validate_valuation_reader_config(
    quote: QuoteCode,
    reader: &ValuationReaderConfig,
) -> Result<(), SymbolConfigError> {
    match reader {
        ValuationReaderConfig::FixedUnitPrice { .. } => {}
        ValuationReaderConfig::DirectPrice { .. } => {}
        ValuationReaderConfig::DerivedUnitPrice {
            numerator,
            denominator,
        } => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valuation_source_metadata_rejects_secret_markers() {
        let mut metadata = BTreeMap::new();
        metadata.insert("authorization".to_string(), "redacted".to_string());

        assert_eq!(
            ValuationSourceConfig::new(
                "chainlink_eth_usd".to_string(),
                "ethereum-mainnet".to_string(),
                "eth.native.ethereum-mainnet".to_string(),
                QuoteCode::Usd,
                ValuationSourceReaderConfig::EvmOracle {
                    oracle_kind: "chainlink".parse().expect("valid oracle kind"),
                    config: EvmOracleConfig {
                        contract_address: "0x0000000000000000000000000000000000000001"
                            .parse()
                            .expect("valid address"),
                    },
                },
                metadata,
            )
            .unwrap_err(),
            ValuationSourceRegistryError::MetadataContainsSecret {
                source_id: "chainlink_eth_usd".to_string(),
                key: "authorization".to_string(),
            }
        );
    }
}
