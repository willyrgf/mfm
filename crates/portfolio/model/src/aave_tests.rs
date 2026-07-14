use super::*;
use crate::portfolio::PortfolioConfig;
use crate::symbol::{
    QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole, SymbolValuationConfig,
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
                underlying_token_address: evm_address("0x00000000000000000000000000000000000000b2"),
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
                underlying_token_address: evm_address("0x00000000000000000000000000000000000000a1"),
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
        unit_price_dec: "1.00".parse().expect("valid unit price"),
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
