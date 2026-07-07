use std::collections::HashMap;
use std::fmt;

use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::ids::{AaveMarketId, AaveReserveId, NetworkId, NormalizedEvmAddress};
use crate::metadata::PublicMetadata;
use crate::portfolio::PortfolioConfig;
use crate::symbol::{BalanceReaderConfig, QuoteCode, SymbolConfig, SymbolKind, SymbolRole};

/// Canonical protocol id handled by this module.
pub const AAVE_V3_PROTOCOL_ID: &str = "aave_v3";
/// Canonical reader id for supplied/collateral reserve positions.
pub const AAVE_V3_READER_RESERVE_POSITION: &str = "reserve_position";
/// Canonical reader id for debt-token-backed debt positions.
pub const AAVE_V3_READER_DEBT_POSITION: &str = "debt_position";

/// Typed Aave V3 market config embedded inside a canonical `protocol_position` reader blob.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "aave-market-config",
    schema = "mfm.portfolio.aave.market_config"
)]
pub struct AaveMarketConfig {
    /// Stable logical market identifier.
    pub market_id: AaveMarketId,
    /// Stable portfolio network identifier on which the market lives.
    pub network_id: NetworkId,
    /// EVM chain id for the market.
    pub chain_id: u64,
    /// Pool contract address used for collateral-flag reads.
    pub pool_address: NormalizedEvmAddress,
    /// Explicit reserve registry for the market.
    pub reserves: Vec<AaveReserveConfig>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

impl AaveMarketConfig {
    /// Sorts nested collections into a deterministic order used for semantic comparisons.
    pub fn normalize(&mut self) {
        self.reserves
            .sort_by(|left, right| left.reserve_id.cmp(&right.reserve_id));
    }

    /// Returns a normalized clone of the market config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Returns the reserve config for `reserve_id`, if present.
    pub fn reserve(&self, reserve_id: &str) -> Option<&AaveReserveConfig> {
        self.reserves
            .iter()
            .find(|reserve| reserve.reserve_id == reserve_id)
    }
}

/// Typed reserve entry inside an Aave market config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "aave-reserve-config",
    schema = "mfm.portfolio.aave.reserve_config"
)]
pub struct AaveReserveConfig {
    /// Stable reserve identifier used in symbol configs.
    pub reserve_id: AaveReserveId,
    /// Aave reserve index used by `getUserConfiguration`.
    pub reserve_index: u16,
    /// Underlying ERC-20 token contract address.
    pub underlying_token_address: NormalizedEvmAddress,
    /// aToken contract address for supplied positions.
    pub a_token_address: NormalizedEvmAddress,
    /// Optional variable debt token contract address.
    #[serde(default)]
    pub variable_debt_token_address: Option<NormalizedEvmAddress>,
    /// Optional stable debt token contract address.
    #[serde(default)]
    pub stable_debt_token_address: Option<NormalizedEvmAddress>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: PublicMetadata,
}

/// Typed reader config for an Aave reserve position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "aave-reserve-position-config",
    schema = "mfm.portfolio.aave.reserve_position_config"
)]
pub struct AaveReservePositionConfig {
    /// Explicit market config used to resolve reserve token addresses.
    pub market: AaveMarketConfig,
    /// Reserve identifier within the market.
    pub reserve_id: AaveReserveId,
    /// Optional collateral-flag requirement enforced against `getUserConfiguration`.
    #[serde(default)]
    pub use_as_collateral_required: Option<bool>,
}

/// Typed reader config for an Aave debt position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "aave-debt-position-config",
    schema = "mfm.portfolio.aave.debt_position_config"
)]
pub struct AaveDebtPositionConfig {
    /// Explicit market config used to resolve debt token addresses.
    pub market: AaveMarketConfig,
    /// Reserve identifier within the market.
    pub reserve_id: AaveReserveId,
    /// Debt token family to read.
    #[serde(default)]
    pub debt_kind: AaveDebtKind,
}

/// Supported Aave V3 debt token families.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "aave-debt-kind",
    schema = "mfm.portfolio.aave.debt_kind"
)]
pub enum AaveDebtKind {
    /// Variable debt token balance.
    #[default]
    Variable,
    /// Stable debt token balance.
    Stable,
}

impl mfm_values::MfmDefault for AaveDebtKind {}

impl AaveDebtKind {
    /// Returns the canonical string form used in JSON and observation metadata.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Variable => "variable",
            Self::Stable => "stable",
        }
    }
}

impl fmt::Display for AaveDebtKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Typed Aave protocol-position config resolved from the generic symbol reader envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "aave-protocol-position-config",
    schema = "mfm.portfolio.aave.protocol_position_config"
)]
pub enum AaveProtocolPositionConfig {
    /// Supplied/collateral position backed by an aToken balance.
    ReservePosition(AaveReservePositionConfig),
    /// Debt position backed by a stable or variable debt token balance.
    DebtPosition(AaveDebtPositionConfig),
}

impl AaveProtocolPositionConfig {
    /// Returns the canonical reader id used by the config.
    pub fn reader_name(&self) -> &'static str {
        match self {
            Self::ReservePosition(_) => AAVE_V3_READER_RESERVE_POSITION,
            Self::DebtPosition(_) => AAVE_V3_READER_DEBT_POSITION,
        }
    }

    /// Returns the market config referenced by the reader.
    pub fn market(&self) -> &AaveMarketConfig {
        match self {
            Self::ReservePosition(cfg) => &cfg.market,
            Self::DebtPosition(cfg) => &cfg.market,
        }
    }

    /// Returns the referenced reserve id.
    pub fn reserve_id(&self) -> &str {
        match self {
            Self::ReservePosition(cfg) => cfg.reserve_id.as_str(),
            Self::DebtPosition(cfg) => cfg.reserve_id.as_str(),
        }
    }
}

/// Validation and decode errors for canonical Aave portfolio-position config.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AavePortfolioConfigError {
    /// `symbol.kind` did not match the Aave protocol-position contract.
    #[error("symbol `{symbol_id}` must use kind `protocol_position` for protocol `aave_v3`")]
    SymbolKindMismatch {
        /// Symbol that violated the contract.
        symbol_id: String,
    },
    /// `symbol.protocol` did not match `aave_v3`.
    #[error("symbol `{symbol_id}` must declare protocol `aave_v3`")]
    SymbolProtocolMismatch {
        /// Symbol that violated the contract.
        symbol_id: String,
    },
    /// `symbol.underlying_symbol_id` was required but absent.
    #[error("symbol `{symbol_id}` must set underlying_symbol_id for protocol `aave_v3`")]
    MissingUnderlyingSymbolId {
        /// Symbol that violated the contract.
        symbol_id: String,
    },
    /// One quote route used a priced symbol other than the declared underlying symbol.
    #[error(
        "symbol `{symbol_id}` quote `{quote}` must use priced_symbol_id `{underlying_symbol_id}`"
    )]
    ValuationMustUseUnderlyingSymbolId {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Quote whose route was invalid.
        quote: QuoteCode,
        /// Required priced symbol id.
        underlying_symbol_id: String,
    },
    /// Reader id was not supported by the Aave portfolio module.
    #[error("symbol `{symbol_id}` reader `{reader}` is not supported for protocol `aave_v3`")]
    UnsupportedReader {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Unsupported reader id.
        reader: String,
    },
    /// The typed reader config could not be decoded.
    #[error("symbol `{symbol_id}` reader config decode failed: {reason}")]
    ReaderConfigDecode {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Serde decode failure.
        reason: String,
    },
    /// The referenced portfolio network was not found.
    #[error("symbol `{symbol_id}` referenced unknown network `{network_id}`")]
    UnknownPortfolioNetwork {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Referenced network id.
        network_id: String,
    },
    /// A market metadata entry contained a secret-shaped key or value.
    #[error("symbol `{symbol_id}` market `{market_id}` metadata key `{key}` contains secret-shaped content")]
    MarketMetadataContainsSecret {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Market identifier associated with the rejected metadata.
        market_id: String,
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// A reserve metadata entry contained a secret-shaped key or value.
    #[error("symbol `{symbol_id}` market `{market_id}` reserve `{reserve_id}` metadata key `{key}` contains secret-shaped content")]
    ReserveMetadataContainsSecret {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Market identifier associated with the rejected metadata.
        market_id: String,
        /// Reserve identifier associated with the rejected metadata.
        reserve_id: String,
        /// Metadata key associated with the rejected content.
        key: String,
    },
    /// A market-level config invariant failed.
    #[error("symbol `{symbol_id}` market config is invalid: {reason}")]
    InvalidMarketConfig {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Human-readable invariant failure.
        reason: String,
    },
    /// The configured reserve id was not present in the embedded market config.
    #[error("symbol `{symbol_id}` reserve `{reserve_id}` was not found in market `{market_id}`")]
    UnknownReserve {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Market identifier referenced by the symbol.
        market_id: String,
        /// Unknown reserve identifier.
        reserve_id: String,
    },
    /// A debt reader requested a token family that was not configured on the reserve.
    #[error("symbol `{symbol_id}` debt_position requires a `{debt_kind}` debt token address for reserve `{reserve_id}`")]
    MissingDebtTokenAddress {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Reserve identifier referenced by the symbol.
        reserve_id: String,
        /// Missing debt token family.
        debt_kind: AaveDebtKind,
    },
    /// `reserve_position` was paired with a debt role.
    #[error("symbol `{symbol_id}` reserve_position must not use role `debt`")]
    ReserveRoleInvalid {
        /// Symbol that violated the contract.
        symbol_id: String,
    },
    /// `debt_position` was paired with a non-debt role.
    #[error("symbol `{symbol_id}` debt_position must use role `debt`")]
    DebtRoleInvalid {
        /// Symbol that violated the contract.
        symbol_id: String,
    },
    /// The same logical market id appeared with different definitions.
    #[error("symbol `{symbol_id}` market `{market_id}` conflicted with the first declared market config")]
    ConflictingMarketDefinition {
        /// Symbol that violated the contract.
        symbol_id: String,
        /// Conflicting market id.
        market_id: String,
    },
}

/// Returns `true` when `symbol` is configured for the Aave V3 protocol-position runtime.
pub fn is_aave_protocol_position(symbol: &SymbolConfig) -> bool {
    symbol.protocol.as_deref() == Some(AAVE_V3_PROTOCOL_ID)
        || matches!(
            &symbol.balance_reader,
            BalanceReaderConfig::ProtocolPosition { protocol, .. }
                if protocol.as_str() == AAVE_V3_PROTOCOL_ID
        )
}

/// Decodes and validates the typed Aave reader config embedded in `symbol.balance_reader`.
pub fn decode_aave_protocol_position_config(
    symbol: &SymbolConfig,
) -> Result<AaveProtocolPositionConfig, AavePortfolioConfigError> {
    if symbol.kind != SymbolKind::ProtocolPosition {
        return Err(AavePortfolioConfigError::SymbolKindMismatch {
            symbol_id: symbol.symbol_id.to_string(),
        });
    }
    if symbol.protocol.as_deref() != Some(AAVE_V3_PROTOCOL_ID) {
        return Err(AavePortfolioConfigError::SymbolProtocolMismatch {
            symbol_id: symbol.symbol_id.to_string(),
        });
    }

    let underlying_symbol_id = symbol.underlying_symbol_id.as_ref().ok_or_else(|| {
        AavePortfolioConfigError::MissingUnderlyingSymbolId {
            symbol_id: symbol.symbol_id.to_string(),
        }
    })?;
    for quote in &symbol.valuation.quotes {
        if quote.priced_symbol_id != *underlying_symbol_id {
            return Err(
                AavePortfolioConfigError::ValuationMustUseUnderlyingSymbolId {
                    symbol_id: symbol.symbol_id.to_string(),
                    quote: quote.quote,
                    underlying_symbol_id: underlying_symbol_id.to_string(),
                },
            );
        }
    }

    let BalanceReaderConfig::ProtocolPosition {
        protocol,
        reader,
        config,
    } = &symbol.balance_reader
    else {
        return Err(AavePortfolioConfigError::SymbolKindMismatch {
            symbol_id: symbol.symbol_id.to_string(),
        });
    };
    if protocol.as_str() != AAVE_V3_PROTOCOL_ID {
        return Err(AavePortfolioConfigError::SymbolProtocolMismatch {
            symbol_id: symbol.symbol_id.to_string(),
        });
    }

    match (reader.as_str(), config) {
        (AAVE_V3_READER_RESERVE_POSITION, AaveProtocolPositionConfig::ReservePosition(cfg)) => {
            if symbol.role == SymbolRole::Debt {
                return Err(AavePortfolioConfigError::ReserveRoleInvalid {
                    symbol_id: symbol.symbol_id.to_string(),
                });
            }
            Ok(AaveProtocolPositionConfig::ReservePosition(cfg.clone()))
        }
        (AAVE_V3_READER_DEBT_POSITION, AaveProtocolPositionConfig::DebtPosition(cfg)) => {
            if symbol.role != SymbolRole::Debt {
                return Err(AavePortfolioConfigError::DebtRoleInvalid {
                    symbol_id: symbol.symbol_id.to_string(),
                });
            }
            Ok(AaveProtocolPositionConfig::DebtPosition(cfg.clone()))
        }
        (reader, _) => Err(AavePortfolioConfigError::UnsupportedReader {
            symbol_id: symbol.symbol_id.to_string(),
            reader: reader.to_string(),
        }),
    }
}

/// Validates every Aave V3 protocol-position symbol in the portfolio config.
pub fn validate_aave_portfolio_config(
    portfolio: &PortfolioConfig,
) -> Result<(), AavePortfolioConfigError> {
    let network_chain_ids: HashMap<_, _> = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str(), network.chain_id_u64()))
        .collect();
    let mut markets_by_id: HashMap<AaveMarketId, AaveMarketConfig> = HashMap::new();

    for symbol in &portfolio.symbol_configs {
        if !is_aave_protocol_position(symbol) {
            continue;
        }

        let cfg = decode_aave_protocol_position_config(symbol)?;
        let Some(network_chain_id) = network_chain_ids.get(symbol.network_id.as_str()) else {
            return Err(AavePortfolioConfigError::UnknownPortfolioNetwork {
                symbol_id: symbol.symbol_id.to_string(),
                network_id: symbol.network_id.to_string(),
            });
        };

        let network_chain_id =
            (*network_chain_id).ok_or_else(|| AavePortfolioConfigError::InvalidMarketConfig {
                symbol_id: symbol.symbol_id.to_string(),
                reason: format!(
                    "portfolio network `{}` did not declare an evm chain_id",
                    symbol.network_id
                ),
            })?;

        validate_market_config(symbol, cfg.market(), network_chain_id)?;
        let reserve = cfg.market().reserve(cfg.reserve_id()).ok_or_else(|| {
            AavePortfolioConfigError::UnknownReserve {
                symbol_id: symbol.symbol_id.to_string(),
                market_id: cfg.market().market_id.to_string(),
                reserve_id: cfg.reserve_id().to_string(),
            }
        })?;
        if let AaveProtocolPositionConfig::DebtPosition(debt_cfg) = &cfg {
            match debt_cfg.debt_kind {
                AaveDebtKind::Variable if reserve.variable_debt_token_address.is_none() => {
                    return Err(AavePortfolioConfigError::MissingDebtTokenAddress {
                        symbol_id: symbol.symbol_id.to_string(),
                        reserve_id: debt_cfg.reserve_id.to_string(),
                        debt_kind: debt_cfg.debt_kind,
                    });
                }
                AaveDebtKind::Stable if reserve.stable_debt_token_address.is_none() => {
                    return Err(AavePortfolioConfigError::MissingDebtTokenAddress {
                        symbol_id: symbol.symbol_id.to_string(),
                        reserve_id: debt_cfg.reserve_id.to_string(),
                        debt_kind: debt_cfg.debt_kind,
                    });
                }
                _ => {}
            }
        }

        let market_id = cfg.market().market_id.clone();
        let market = cfg.market().clone().normalized();
        if let Some(first_market) = markets_by_id.get(&market_id) {
            if first_market != &market {
                return Err(AavePortfolioConfigError::ConflictingMarketDefinition {
                    symbol_id: symbol.symbol_id.to_string(),
                    market_id: market_id.to_string(),
                });
            }
        } else {
            markets_by_id.insert(market_id, market);
        }
    }

    Ok(())
}

fn validate_market_config(
    symbol: &SymbolConfig,
    market: &AaveMarketConfig,
    expected_chain_id: u64,
) -> Result<(), AavePortfolioConfigError> {
    if market.network_id != symbol.network_id {
        return Err(invalid_market(
            symbol,
            format!(
                "market `{}` network_id `{}` did not match symbol network `{}`",
                market.market_id, market.network_id, symbol.network_id
            ),
        ));
    }
    if market.chain_id != expected_chain_id {
        return Err(invalid_market(
            symbol,
            format!(
                "market `{}` chain_id `{}` did not match portfolio network chain_id `{}`",
                market.market_id, market.chain_id, expected_chain_id
            ),
        ));
    }
    if market.reserves.is_empty() {
        return Err(invalid_market(symbol, "market.reserves must not be empty"));
    }

    let mut reserve_ids = HashMap::new();
    let mut reserve_indexes = HashMap::new();
    for reserve in &market.reserves {
        if reserve_ids
            .insert(reserve.reserve_id.as_str(), ())
            .is_some()
        {
            return Err(invalid_market(
                symbol,
                format!(
                    "market `{}` reserve_id `{}` must be unique",
                    market.market_id, reserve.reserve_id
                ),
            ));
        }
        if reserve_indexes
            .insert(reserve.reserve_index, reserve.reserve_id.as_str())
            .is_some()
        {
            return Err(invalid_market(
                symbol,
                format!(
                    "market `{}` reserve_index `{}` must be unique",
                    market.market_id, reserve.reserve_index
                ),
            ));
        }
    }

    Ok(())
}

fn invalid_market(symbol: &SymbolConfig, reason: impl Into<String>) -> AavePortfolioConfigError {
    AavePortfolioConfigError::InvalidMarketConfig {
        symbol_id: symbol.symbol_id.to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::PortfolioConfig;
    use crate::symbol::{
        QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole,
        SymbolValuationConfig, ValuationReaderConfig,
    };
    use serde_json::{json, Value};

    fn evm_address(value: &str) -> NormalizedEvmAddress {
        value.parse().expect("valid EVM address")
    }

    fn aave_market() -> AaveMarketConfig {
        AaveMarketConfig {
            market_id: "aave-v3-mainnet".parse().expect("valid market id"),
            network_id: "ethereum-mainnet".parse().expect("valid network id"),
            chain_id: 1,
            pool_address: evm_address("0x0000000000000000000000000000000000000abc"),
            reserves: vec![
                AaveReserveConfig {
                    reserve_id: "wbtc".parse().expect("valid reserve id"),
                    reserve_index: 1,
                    underlying_token_address: evm_address(
                        "0x00000000000000000000000000000000000000b2",
                    ),
                    a_token_address: evm_address("0x00000000000000000000000000000000000000b3"),
                    variable_debt_token_address: Some(evm_address(
                        "0x00000000000000000000000000000000000000b4",
                    )),
                    stable_debt_token_address: None,
                    metadata: PublicMetadata::default(),
                },
                AaveReserveConfig {
                    reserve_id: "usdc".parse().expect("valid reserve id"),
                    reserve_index: 0,
                    underlying_token_address: evm_address(
                        "0x00000000000000000000000000000000000000a1",
                    ),
                    a_token_address: evm_address("0x00000000000000000000000000000000000000a2"),
                    variable_debt_token_address: Some(evm_address(
                        "0x00000000000000000000000000000000000000a3",
                    )),
                    stable_debt_token_address: None,
                    metadata: PublicMetadata::default(),
                },
            ],
            metadata: PublicMetadata::default(),
        }
    }

    fn fixed_usd_quote(priced_symbol_id: &str) -> QuoteValuationConfig {
        QuoteValuationConfig {
            quote: QuoteCode::Usd,
            priced_symbol_id: priced_symbol_id.parse().expect("valid priced symbol id"),
            reader: ValuationReaderConfig::FixedUnitPrice {
                unit_price_dec: "1.00".parse().expect("valid unit price"),
            },
        }
    }

    fn aave_symbol(
        symbol_id: &str,
        role: SymbolRole,
        reader: &str,
        config: Value,
        underlying_symbol_id: Option<&str>,
        priced_symbol_id: &str,
    ) -> SymbolConfig {
        let config = match reader {
            AAVE_V3_READER_RESERVE_POSITION => json!({ "reserve_position": config }),
            AAVE_V3_READER_DEBT_POSITION => json!({ "debt_position": config }),
            _ => config,
        };
        SymbolConfig {
            symbol_id: symbol_id.parse().expect("valid symbol id"),
            display_symbol: Some("USDC".to_string()),
            kind: SymbolKind::ProtocolPosition,
            role,
            network_id: "ethereum-mainnet".parse().expect("valid network id"),
            protocol: Some(AAVE_V3_PROTOCOL_ID.parse().expect("valid protocol id")),
            balance_reader: serde_json::from_value(json!({
                "kind": "protocol_position",
                "protocol": AAVE_V3_PROTOCOL_ID,
                "reader": reader,
                "config": config
            }))
            .expect("balance reader config"),
            valuation: SymbolValuationConfig {
                quotes: vec![fixed_usd_quote(priced_symbol_id)],
            },
            decimals: Some(6),
            underlying_symbol_id: underlying_symbol_id
                .map(|symbol_id| symbol_id.parse().expect("valid underlying symbol id")),
            metadata: PublicMetadata::default(),
        }
    }

    fn portfolio_with_aave_symbols(symbols: Vec<SymbolConfig>) -> PortfolioConfig {
        serde_json::from_value(json!({
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
            "wallets": [],
            "symbol_configs": symbols,
            "metadata": {}
        }))
        .expect("portfolio")
    }

    fn assert_protocol_position_config_rejected(market: Value) {
        assert!(serde_json::from_value::<BalanceReaderConfig>(json!({
            "kind": "protocol_position",
            "protocol": AAVE_V3_PROTOCOL_ID,
            "reader": AAVE_V3_READER_RESERVE_POSITION,
            "config": {
                "reserve_position": {
                    "market": market,
                    "reserve_id": "usdc"
                }
            }
        }))
        .is_err());
    }

    #[test]
    fn validates_aave_symbols_against_underlying_symbol_identity() {
        let market = aave_market();
        let portfolio = portfolio_with_aave_symbols(vec![
            aave_symbol(
                "usdc.wallet.ethereum-mainnet",
                SymbolRole::Asset,
                AAVE_V3_READER_RESERVE_POSITION,
                json!({
                    "market": market,
                    "reserve_id": "usdc"
                }),
                Some("usdc.wallet.ethereum-mainnet"),
                "usdc.wallet.ethereum-mainnet",
            ),
            aave_symbol(
                "aave_v3.usdc.debt.ethereum-mainnet",
                SymbolRole::Debt,
                AAVE_V3_READER_DEBT_POSITION,
                json!({
                    "market": aave_market(),
                    "reserve_id": "usdc",
                    "debt_kind": "variable"
                }),
                Some("usdc.wallet.ethereum-mainnet"),
                "eth.native.ethereum-mainnet",
            ),
        ]);

        let err = validate_aave_portfolio_config(&portfolio).expect_err("must fail");
        assert!(matches!(
            err,
            AavePortfolioConfigError::ValuationMustUseUnderlyingSymbolId {
                symbol_id,
                quote: QuoteCode::Usd,
                underlying_symbol_id
            } if symbol_id == "aave_v3.usdc.debt.ethereum-mainnet"
                && underlying_symbol_id == "usdc.wallet.ethereum-mainnet"
        ));
    }

    #[test]
    fn rejects_conflicting_market_definitions_for_same_market_id() {
        let mut conflicting_market = aave_market();
        conflicting_market.pool_address = evm_address("0x0000000000000000000000000000000000000def");
        let portfolio = portfolio_with_aave_symbols(vec![
            aave_symbol(
                "aave_v3.usdc.asset.ethereum-mainnet",
                SymbolRole::Asset,
                AAVE_V3_READER_RESERVE_POSITION,
                json!({
                    "market": aave_market(),
                    "reserve_id": "usdc"
                }),
                Some("usdc.wallet.ethereum-mainnet"),
                "usdc.wallet.ethereum-mainnet",
            ),
            aave_symbol(
                "aave_v3.usdc.debt.ethereum-mainnet",
                SymbolRole::Debt,
                AAVE_V3_READER_DEBT_POSITION,
                json!({
                    "market": conflicting_market,
                    "reserve_id": "usdc",
                    "debt_kind": "variable"
                }),
                Some("usdc.wallet.ethereum-mainnet"),
                "usdc.wallet.ethereum-mainnet",
            ),
        ]);

        let err = validate_aave_portfolio_config(&portfolio).expect_err("must fail");
        assert!(matches!(
            err,
            AavePortfolioConfigError::ConflictingMarketDefinition { symbol_id, market_id }
                if symbol_id == "aave_v3.usdc.debt.ethereum-mainnet"
                    && market_id == "aave-v3-mainnet"
        ));
    }

    #[test]
    fn rejects_secret_markers_in_market_and_reserve_metadata() {
        let mut market_metadata = serde_json::to_value(aave_market()).expect("market json");
        market_metadata["metadata"] = json!({"secret_key": "redacted"});
        assert_protocol_position_config_rejected(market_metadata);

        let mut reserve_metadata = serde_json::to_value(aave_market()).expect("market json");
        reserve_metadata["reserves"][0]["metadata"] = json!({"label": "bearer redacted"});
        assert_protocol_position_config_rejected(reserve_metadata);
    }
}
