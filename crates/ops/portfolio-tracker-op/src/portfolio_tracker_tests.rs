use super::*;
use mfm_certify::certify_program_draft;
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig, PortfolioConfig};
use mfm_portfolio_model::symbol::{
    BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole,
    SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn portfolio_program_lowers_to_select_centric_graph() {
    let draft = portfolio_program_draft(sample_workflow_config()).expect("draft");
    assert_eq!(draft.state_nodes().len(), 6);
    let state_keys = draft
        .state_nodes()
        .iter()
        .map(|node| node.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        state_keys,
        [
            "portfolio_inputs_ready",
            "resolve_subjects",
            "select_holdings",
            "resolve_valuations",
            "assemble_snapshot",
            "project_report",
        ]
    );
    assert!(
        !state_keys.iter().any(|key| {
            key.contains("pin_views")
                || key.contains("observe")
                || key.contains("merge_observations")
        }),
        "live pin/observe/merge must be absent: {state_keys:?}"
    );

    let certified = certify_program_draft(&draft).expect("certified portfolio spec");
    certified.envelope().verify_hash().expect("hash verifies");
    let spec_json = certified
        .envelope()
        .spec
        .canonical_json()
        .expect("canonical spec");
    let spec_value: Value = serde_json::from_slice(spec_json.as_bytes()).expect("spec json");
    let as_text = spec_value.to_string();
    assert!(as_text.contains("select_holdings"));
    assert!(!as_text.contains("pin_views"));
    assert!(!as_text.contains("observe_batch"));
    assert!(!as_text.contains("merge_observations"));
}

fn sample_workflow_config() -> PortfolioConfig {
    sample_portfolio_config()
}

fn sample_portfolio_config() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "portfolio_main".parse().expect("valid portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![NetworkConfig::new(
            "ethereum-mainnet".to_owned(),
            NetworkFamilyConfig::Evm,
            Some(1),
            None,
            None,
            BTreeMap::new(),
        )
        .expect("valid network config")],
        wallets: vec![WalletConfig {
            wallet_id: "wallet_main".parse().expect("valid wallet id"),
            subject: WalletSubject::new(
                "0x000000000000000000000000000000000000dead",
                WalletSubjectKind::EvmAddress,
            )
            .expect("valid wallet subject"),
            network_id: "ethereum-mainnet".parse().expect("valid network id"),
            implementation: WalletImplementationConfig::AddressOnly {},
            symbol_ids: vec!["eth.native.ethereum-mainnet"
                .parse()
                .expect("valid symbol id")],
            metadata: PublicMetadata::default(),
        }],
        symbol_configs: vec![SymbolConfig {
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
            metadata: PublicMetadata::default(),
        }],
        metadata: PublicMetadata::default(),
    }
}
