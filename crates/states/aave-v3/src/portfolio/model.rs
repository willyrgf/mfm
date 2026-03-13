use std::collections::{BTreeMap, HashMap};
use std::fmt;

use mfm_evm_core::encoding::normalize_address;
use mfm_state_portfolio::model::PortfolioConfig;
use mfm_state_symbol::model::{
    BalanceReaderConfig, QuoteCode, SymbolConfig, SymbolKind, SymbolRole,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Canonical protocol id handled by this module.
pub const AAVE_V3_PROTOCOL_ID: &str = "aave_v3";
/// Canonical reader id for supplied/collateral reserve positions.
pub const AAVE_V3_READER_RESERVE_POSITION: &str = "reserve_position";
/// Canonical reader id for debt-token-backed debt positions.
pub const AAVE_V3_READER_DEBT_POSITION: &str = "debt_position";

fn default_debt_kind() -> AaveDebtKind {
    AaveDebtKind::Variable
}

/// Typed Aave V3 market config embedded inside a canonical `protocol_position` reader blob.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AaveMarketConfig {
    /// Stable logical market identifier.
    pub market_id: String,
    /// Stable portfolio network identifier on which the market lives.
    pub network_id: String,
    /// EVM chain id for the market.
    pub chain_id: u64,
    /// Pool contract address used for collateral-flag reads.
    pub pool_address: String,
    /// Explicit reserve registry for the market.
    pub reserves: Vec<AaveReserveConfig>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AaveReserveConfig {
    /// Stable reserve identifier used in symbol configs.
    pub reserve_id: String,
    /// Aave reserve index used by `getUserConfiguration`.
    pub reserve_index: u16,
    /// Underlying ERC-20 token contract address.
    pub underlying_token_address: String,
    /// aToken contract address for supplied positions.
    pub a_token_address: String,
    /// Optional variable debt token contract address.
    #[serde(default)]
    pub variable_debt_token_address: Option<String>,
    /// Optional stable debt token contract address.
    #[serde(default)]
    pub stable_debt_token_address: Option<String>,
    /// Canonical metadata surface.
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

/// Typed reader config for an Aave reserve position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AaveReservePositionConfig {
    /// Explicit market config used to resolve reserve token addresses.
    pub market: AaveMarketConfig,
    /// Reserve identifier within the market.
    pub reserve_id: String,
    /// Optional collateral-flag requirement enforced against `getUserConfiguration`.
    #[serde(default)]
    pub use_as_collateral_required: Option<bool>,
}

/// Typed reader config for an Aave debt position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AaveDebtPositionConfig {
    /// Explicit market config used to resolve debt token addresses.
    pub market: AaveMarketConfig,
    /// Reserve identifier within the market.
    pub reserve_id: String,
    /// Debt token family to read.
    #[serde(default = "default_debt_kind")]
    pub debt_kind: AaveDebtKind,
}

/// Supported Aave V3 debt token families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AaveDebtKind {
    /// Variable debt token balance.
    Variable,
    /// Stable debt token balance.
    Stable,
}

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
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq, Eq, Error)]
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
    #[error(
        "symbol `{symbol_id}` debt_position requires a `{debt_kind}` debt token address for reserve `{reserve_id}`"
    )]
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
    #[error(
        "symbol `{symbol_id}` market `{market_id}` conflicted with the first declared market config"
    )]
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
                if protocol == AAVE_V3_PROTOCOL_ID
        )
}

/// Decodes and validates the typed Aave reader config embedded in `symbol.balance_reader`.
pub fn decode_aave_protocol_position_config(
    symbol: &SymbolConfig,
) -> Result<AaveProtocolPositionConfig, AavePortfolioConfigError> {
    if symbol.kind != SymbolKind::ProtocolPosition {
        return Err(AavePortfolioConfigError::SymbolKindMismatch {
            symbol_id: symbol.symbol_id.clone(),
        });
    }
    if symbol.protocol.as_deref() != Some(AAVE_V3_PROTOCOL_ID) {
        return Err(AavePortfolioConfigError::SymbolProtocolMismatch {
            symbol_id: symbol.symbol_id.clone(),
        });
    }

    let underlying_symbol_id = symbol.underlying_symbol_id.as_ref().ok_or_else(|| {
        AavePortfolioConfigError::MissingUnderlyingSymbolId {
            symbol_id: symbol.symbol_id.clone(),
        }
    })?;
    for quote in &symbol.valuation.quotes {
        if quote.priced_symbol_id != *underlying_symbol_id {
            return Err(
                AavePortfolioConfigError::ValuationMustUseUnderlyingSymbolId {
                    symbol_id: symbol.symbol_id.clone(),
                    quote: quote.quote,
                    underlying_symbol_id: underlying_symbol_id.clone(),
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
            symbol_id: symbol.symbol_id.clone(),
        });
    };
    if protocol != AAVE_V3_PROTOCOL_ID {
        return Err(AavePortfolioConfigError::SymbolProtocolMismatch {
            symbol_id: symbol.symbol_id.clone(),
        });
    }

    let config_value = Value::Object(
        config
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    match reader.as_str() {
        AAVE_V3_READER_RESERVE_POSITION => {
            let cfg = serde_json::from_value::<AaveReservePositionConfig>(config_value).map_err(
                |err| AavePortfolioConfigError::ReaderConfigDecode {
                    symbol_id: symbol.symbol_id.clone(),
                    reason: err.to_string(),
                },
            )?;
            if symbol.role == SymbolRole::Debt {
                return Err(AavePortfolioConfigError::ReserveRoleInvalid {
                    symbol_id: symbol.symbol_id.clone(),
                });
            }
            Ok(AaveProtocolPositionConfig::ReservePosition(cfg))
        }
        AAVE_V3_READER_DEBT_POSITION => {
            let cfg =
                serde_json::from_value::<AaveDebtPositionConfig>(config_value).map_err(|err| {
                    AavePortfolioConfigError::ReaderConfigDecode {
                        symbol_id: symbol.symbol_id.clone(),
                        reason: err.to_string(),
                    }
                })?;
            if symbol.role != SymbolRole::Debt {
                return Err(AavePortfolioConfigError::DebtRoleInvalid {
                    symbol_id: symbol.symbol_id.clone(),
                });
            }
            Ok(AaveProtocolPositionConfig::DebtPosition(cfg))
        }
        _ => Err(AavePortfolioConfigError::UnsupportedReader {
            symbol_id: symbol.symbol_id.clone(),
            reader: reader.clone(),
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
        .map(|network| (network.network_id.as_str(), network.chain_id))
        .collect();
    let mut markets_by_id: HashMap<String, AaveMarketConfig> = HashMap::new();

    for symbol in &portfolio.symbol_configs {
        if !is_aave_protocol_position(symbol) {
            continue;
        }

        let cfg = decode_aave_protocol_position_config(symbol)?;
        let Some(network_chain_id) = network_chain_ids.get(symbol.network_id.as_str()) else {
            return Err(AavePortfolioConfigError::UnknownPortfolioNetwork {
                symbol_id: symbol.symbol_id.clone(),
                network_id: symbol.network_id.clone(),
            });
        };

        validate_market_config(symbol, cfg.market(), *network_chain_id)?;
        let reserve = cfg.market().reserve(cfg.reserve_id()).ok_or_else(|| {
            AavePortfolioConfigError::UnknownReserve {
                symbol_id: symbol.symbol_id.clone(),
                market_id: cfg.market().market_id.clone(),
                reserve_id: cfg.reserve_id().to_string(),
            }
        })?;
        if let AaveProtocolPositionConfig::DebtPosition(debt_cfg) = &cfg {
            match debt_cfg.debt_kind {
                AaveDebtKind::Variable if reserve.variable_debt_token_address.is_none() => {
                    return Err(AavePortfolioConfigError::MissingDebtTokenAddress {
                        symbol_id: symbol.symbol_id.clone(),
                        reserve_id: debt_cfg.reserve_id.clone(),
                        debt_kind: debt_cfg.debt_kind,
                    });
                }
                AaveDebtKind::Stable if reserve.stable_debt_token_address.is_none() => {
                    return Err(AavePortfolioConfigError::MissingDebtTokenAddress {
                        symbol_id: symbol.symbol_id.clone(),
                        reserve_id: debt_cfg.reserve_id.clone(),
                        debt_kind: debt_cfg.debt_kind,
                    });
                }
                _ => {}
            }
        }

        let market_id = cfg.market().market_id.clone();
        let market = cfg.market().clone().normalized();
        if let Some(first_market) = markets_by_id.get(market_id.as_str()) {
            if first_market != &market {
                return Err(AavePortfolioConfigError::ConflictingMarketDefinition {
                    symbol_id: symbol.symbol_id.clone(),
                    market_id,
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
    if market.market_id.trim().is_empty() {
        return Err(invalid_market(symbol, "market_id must be non-empty"));
    }
    if market.network_id.trim().is_empty() {
        return Err(invalid_market(
            symbol,
            "market.network_id must be non-empty",
        ));
    }
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
    validate_address_field(symbol, "market.pool_address", market.pool_address.as_str())?;

    if market.reserves.is_empty() {
        return Err(invalid_market(symbol, "market.reserves must not be empty"));
    }

    let mut reserve_ids = HashMap::new();
    let mut reserve_indexes = HashMap::new();
    for reserve in &market.reserves {
        if reserve.reserve_id.trim().is_empty() {
            return Err(invalid_market(
                symbol,
                "market.reserves[].reserve_id must be non-empty",
            ));
        }
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
        validate_address_field(
            symbol,
            "market.reserves[].underlying_token_address",
            reserve.underlying_token_address.as_str(),
        )?;
        validate_address_field(
            symbol,
            "market.reserves[].a_token_address",
            reserve.a_token_address.as_str(),
        )?;
        if let Some(address) = &reserve.variable_debt_token_address {
            validate_address_field(
                symbol,
                "market.reserves[].variable_debt_token_address",
                address.as_str(),
            )?;
        }
        if let Some(address) = &reserve.stable_debt_token_address {
            validate_address_field(
                symbol,
                "market.reserves[].stable_debt_token_address",
                address.as_str(),
            )?;
        }
    }

    Ok(())
}

fn validate_address_field(
    symbol: &SymbolConfig,
    field: &str,
    value: &str,
) -> Result<(), AavePortfolioConfigError> {
    if value.trim().is_empty() {
        return Err(invalid_market(
            symbol,
            format!("{field} must be a normalized EVM address"),
        ));
    }
    let normalized = normalize_address(value)
        .map_err(|_| invalid_market(symbol, format!("{field} must be a normalized EVM address")))?;
    if normalized != value {
        return Err(invalid_market(
            symbol,
            format!("{field} must be a normalized EVM address"),
        ));
    }
    Ok(())
}

fn invalid_market(symbol: &SymbolConfig, reason: impl Into<String>) -> AavePortfolioConfigError {
    AavePortfolioConfigError::InvalidMarketConfig {
        symbol_id: symbol.symbol_id.clone(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_state_portfolio::model::PortfolioConfig;
    use mfm_state_symbol::model::{
        BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole,
        SymbolValuationConfig, ValuationReaderConfig,
    };
    use serde_json::json;

    fn aave_market() -> AaveMarketConfig {
        AaveMarketConfig {
            market_id: "aave-v3-mainnet".to_string(),
            network_id: "ethereum-mainnet".to_string(),
            chain_id: 1,
            pool_address: "0x0000000000000000000000000000000000000abc".to_string(),
            reserves: vec![
                AaveReserveConfig {
                    reserve_id: "wbtc".to_string(),
                    reserve_index: 1,
                    underlying_token_address: "0x00000000000000000000000000000000000000b2"
                        .to_string(),
                    a_token_address: "0x00000000000000000000000000000000000000b3".to_string(),
                    variable_debt_token_address: Some(
                        "0x00000000000000000000000000000000000000b4".to_string(),
                    ),
                    stable_debt_token_address: None,
                    metadata: BTreeMap::new(),
                },
                AaveReserveConfig {
                    reserve_id: "usdc".to_string(),
                    reserve_index: 0,
                    underlying_token_address: "0x00000000000000000000000000000000000000a1"
                        .to_string(),
                    a_token_address: "0x00000000000000000000000000000000000000a2".to_string(),
                    variable_debt_token_address: Some(
                        "0x00000000000000000000000000000000000000a3".to_string(),
                    ),
                    stable_debt_token_address: None,
                    metadata: BTreeMap::new(),
                },
            ],
            metadata: BTreeMap::new(),
        }
    }

    fn fixed_usd_quote(priced_symbol_id: &str) -> QuoteValuationConfig {
        QuoteValuationConfig {
            quote: QuoteCode::Usd,
            priced_symbol_id: priced_symbol_id.to_string(),
            reader: ValuationReaderConfig::FixedUnitPrice {
                unit_price_dec: "1.00".to_string(),
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
        SymbolConfig {
            symbol_id: symbol_id.to_string(),
            display_symbol: Some("USDC".to_string()),
            kind: SymbolKind::ProtocolPosition,
            role,
            network_id: "ethereum-mainnet".to_string(),
            protocol: Some(AAVE_V3_PROTOCOL_ID.to_string()),
            balance_reader: BalanceReaderConfig::ProtocolPosition {
                protocol: AAVE_V3_PROTOCOL_ID.to_string(),
                reader: reader.to_string(),
                config: serde_json::from_value(config).expect("config map"),
            },
            valuation: SymbolValuationConfig {
                quotes: vec![fixed_usd_quote(priced_symbol_id)],
            },
            decimals: Some(6),
            underlying_symbol_id: underlying_symbol_id.map(ToString::to_string),
            metadata: BTreeMap::new(),
        }
    }

    fn portfolio_with_aave_symbols(symbols: Vec<SymbolConfig>) -> PortfolioConfig {
        serde_json::from_value(json!({
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1,
                    "rpc_source_id": null,
                    "metadata": {}
                }
            ],
            "wallets": [],
            "symbol_configs": symbols,
            "metadata": {}
        }))
        .expect("portfolio")
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
        conflicting_market.pool_address = "0x0000000000000000000000000000000000000def".to_string();
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
}
