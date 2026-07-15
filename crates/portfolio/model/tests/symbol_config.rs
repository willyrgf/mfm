use mfm_portfolio_model::symbol::{
    validate_symbol_config, HoldingSourceConfig, QuoteCode, QuoteValuationConfig, SymbolConfig,
    SymbolConfigError, SymbolValuationConfig,
};

fn valuation() -> SymbolValuationConfig {
    SymbolValuationConfig {
        quotes: vec![QuoteValuationConfig {
            quote: QuoteCode::Usd,
            priced_symbol_id: "eth.native.ethereum-mainnet"
                .parse()
                .expect("valid priced symbol id"),
            unit_price_dec: "1800.00".parse().expect("valid unit price"),
        }],
    }
}

#[test]
fn symbol_config_uses_one_direct_native_source() {
    let symbol = SymbolConfig {
        symbol_id: "eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id"),
        display_symbol: Some("ETH".to_owned()),
        network_id: "ethereum-mainnet".parse().expect("valid network id"),
        source: HoldingSourceConfig::Native,
        valuation: valuation(),
        metadata: Default::default(),
    };

    validate_symbol_config(&symbol).expect("valid symbol");
    assert!(symbol.source.is_native());
    assert_eq!(symbol.source.contract_address(), None);
}

#[test]
fn holding_source_rejects_legacy_or_extra_native_fields() {
    assert!(
        serde_json::from_value::<HoldingSourceConfig>(serde_json::json!({
            "kind": "native",
            "legacy_field": "native_balance"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<HoldingSourceConfig>(serde_json::json!({
            "kind": "native",
            "contract_address": "0x0000000000000000000000000000000000000001"
        }))
        .is_err()
    );
}

#[test]
fn symbol_config_rejects_zero_erc20_contract() {
    let symbol = SymbolConfig {
        symbol_id: "usdc.wallet.ethereum-mainnet"
            .parse()
            .expect("valid symbol id"),
        display_symbol: Some("USDC".to_owned()),
        network_id: "ethereum-mainnet".parse().expect("valid network id"),
        source: HoldingSourceConfig::Erc20 {
            contract_address: "0x0000000000000000000000000000000000000000"
                .parse()
                .expect("normalized address"),
        },
        valuation: valuation(),
        metadata: Default::default(),
    };

    assert_eq!(
        validate_symbol_config(&symbol),
        Err(SymbolConfigError::ZeroErc20ContractAddress)
    );
}

#[test]
fn symbol_config_rejects_duplicate_quote_routes() {
    let mut symbol = SymbolConfig {
        symbol_id: "eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id"),
        display_symbol: Some("ETH".to_owned()),
        network_id: "ethereum-mainnet".parse().expect("valid network id"),
        source: HoldingSourceConfig::Native,
        valuation: valuation(),
        metadata: Default::default(),
    };
    symbol.valuation.quotes.push(QuoteValuationConfig {
        quote: QuoteCode::Usd,
        priced_symbol_id: "eth.native.ethereum-mainnet"
            .parse()
            .expect("valid priced symbol id"),
        unit_price_dec: "1900.00".parse().expect("valid unit price"),
    });

    assert_eq!(
        validate_symbol_config(&symbol),
        Err(SymbolConfigError::DuplicateQuoteValuation {
            quote: QuoteCode::Usd
        })
    );
}
