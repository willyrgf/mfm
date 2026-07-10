// Symbol-config tests for FixedUnitPrice-only cutover surface.
// View-dependent valuation registry / oracle source types were deleted; series E reintroduces
// price sources under the same authority model when needed.

use mfm_portfolio_model::symbol::{
    BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolConfigError,
    SymbolKind, SymbolRole, SymbolValuationConfig,
};

#[test]
fn symbol_config_accepts_fixed_unit_price_only() {
    let symbol = SymbolConfig {
        symbol_id: "eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id"),
        display_symbol: Some("ETH".to_owned()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: "ethereum-mainnet".parse().expect("valid network id"),
        protocol: None,
        balance_reader: BalanceReaderConfig::NativeBalance {},
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: "eth.native.ethereum-mainnet"
                    .parse()
                    .expect("valid priced symbol id"),
                unit_price_dec: "1800.00".parse().expect("valid unit price"),
            }],
        },
        underlying_symbol_id: None,
        metadata: Default::default(),
    };

    mfm_portfolio_model::symbol::validate_symbol_config(&symbol).expect("valid symbol");
}

#[test]
fn symbol_config_rejects_duplicate_quote_routes() {
    let mut symbol = SymbolConfig {
        symbol_id: "eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id"),
        display_symbol: Some("ETH".to_owned()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: "ethereum-mainnet".parse().expect("valid network id"),
        protocol: None,
        balance_reader: BalanceReaderConfig::NativeBalance {},
        valuation: SymbolValuationConfig {
            quotes: vec![
                QuoteValuationConfig {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet"
                        .parse()
                        .expect("valid priced symbol id"),
                    unit_price_dec: "1800.00".parse().expect("valid unit price"),
                },
                QuoteValuationConfig {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet"
                        .parse()
                        .expect("valid priced symbol id"),
                    unit_price_dec: "1900.00".parse().expect("valid unit price"),
                },
            ],
        },
        underlying_symbol_id: None,
        metadata: Default::default(),
    };
    symbol.normalize();

    let err = mfm_portfolio_model::symbol::validate_symbol_config(&symbol).expect_err("duplicate");
    assert_eq!(
        err,
        SymbolConfigError::DuplicateQuoteValuation {
            quote: QuoteCode::Usd
        }
    );
}
