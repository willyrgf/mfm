use super::*;
use std::collections::BTreeMap;

use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, PortfolioConfig,
};
use mfm_portfolio_model::symbol::{
    BalanceReaderConfig, Observation, ObservationAnchor, ObservationQuantity, ObservationSource,
    QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole, SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};

const EVM_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

fn test_network(network_id: &str, chain_id: u64) -> NetworkConfig {
    NetworkConfig::new(
        network_id.to_owned(),
        NetworkFamilyConfig::Evm,
        Some(chain_id),
        None,
        None,
        BTreeMap::new(),
    )
    .expect("valid network config")
}

fn test_wallet(network_id: &str) -> WalletConfig {
    WalletConfig {
        wallet_id: "wallet_main".parse().expect("valid wallet id"),
        subject: WalletSubject::new(
            "0x000000000000000000000000000000000000dead",
            WalletSubjectKind::EvmAddress,
        )
        .expect("valid wallet subject"),
        network_id: network_id.parse().expect("valid network id"),
        implementation: WalletImplementationConfig::AddressOnly {},
        symbol_ids: vec!["eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id")],
        metadata: PublicMetadata::default(),
    }
}

fn test_symbol(network_id: &str) -> SymbolConfig {
    SymbolConfig {
        symbol_id: "eth.native.ethereum-mainnet"
            .parse()
            .expect("valid symbol id"),
        display_symbol: Some("ETH".to_owned()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: network_id.parse().expect("valid network id"),
        protocol: None,
        balance_reader: BalanceReaderConfig::NativeBalance {},
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: "eth.native.ethereum-mainnet"
                    .parse()
                    .expect("valid priced symbol id"),
                unit_price_dec: "2.5".parse().expect("valid unit price"),
            }],
        },
        decimals: Some(18),
        underlying_symbol_id: None,
        metadata: PublicMetadata::default(),
    }
}

fn sample_portfolio() -> PortfolioConfig {
    PortfolioConfig {
        portfolio_id: "portfolio_main".parse().expect("valid portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![test_network("ethereum-mainnet", 1)],
        wallets: vec![test_wallet("ethereum-mainnet")],
        symbol_configs: vec![test_symbol("ethereum-mainnet")],
        metadata: PublicMetadata::default(),
    }
}

#[test]
fn unknown_portfolio_config_fields_are_rejected_on_decode() {
    assert_unknown_field_rejected::<PortfolioWorkflowConfig>(
        r#"{"unexpected_field":1}"#,
        "workflow config",
    );
}

fn assert_unknown_field_rejected<T>(input: &str, label: &str)
where
    T: serde::de::DeserializeOwned,
{
    let error = match serde_json::from_str::<T>(input) {
        Ok(_) => panic!("{label} accepted unknown field"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("unknown field"),
        "{label}: {error}"
    );
}

#[test]
fn expand_requirements_projects_evm_native_and_rejects_erc20() {
    let portfolio = sample_portfolio();
    let config =
        SelectHoldingsConfig::with_default_store_scope(portfolio.clone()).expect("select config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );
    let requirements = expand_required_holdings(&config, &subjects).expect("requirements");
    assert_eq!(requirements.len(), 1);
    assert_eq!(
        requirements[0].projection,
        HoldingFactProjection::EvmNativeBalance
    );

    let mut erc20_portfolio = portfolio;
    erc20_portfolio.symbol_configs[0].kind = SymbolKind::Erc20Balance;
    erc20_portfolio.symbol_configs[0].balance_reader = BalanceReaderConfig::Erc20Balance {
        token_address: "0x0000000000000000000000000000000000000001"
            .parse()
            .expect("token"),
    };
    let config =
        SelectHoldingsConfig::with_default_store_scope(erc20_portfolio.clone()).expect("config");
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(erc20_portfolio.wallets).expect("subjects"),
    );
    let err = expand_required_holdings(&config, &subjects).expect_err("erc20 unsupported");
    assert_eq!(err.code, PortfolioHoldingErrorCode::UnsupportedRequirement);
}

#[test]
fn fixed_price_selection_assembles_report_totals() {
    let portfolio = sample_portfolio();
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(portfolio.symbol_configs.clone()).expect("valuation config"),
    )
    .expect("valuations");

    let mut observation = Observation {
        wallet_id: "wallet_main".to_owned(),
        symbol_id: "eth.native.ethereum-mainnet".to_owned(),
        display_symbol: Some("ETH".to_owned()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: "ethereum-mainnet".to_owned(),
        protocol: None,
        quantity: ObservationQuantity {
            raw_dec: "1000000000000000000".to_owned(),
            decimals: 18,
            amount_dec: "1.000000000000000000".to_owned(),
        },
        values: Vec::new(),
        source: ObservationSource {
            balance_reader_kind: "native_balance".to_owned(),
            network_id: "ethereum-mainnet".to_owned(),
            anchor: ObservationAnchor::Evm {
                chain_id: 1,
                block_number: 10,
                block_hash: EVM_HASH.to_owned(),
            },
        },
        coverage: "configured_only".to_owned(),
        metadata: PublicMetadata::default(),
    };
    observation.normalize();

    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(2, portfolio).expect("assemble config"),
        AssembleSnapshotInput {
            subjects,
            holdings: SelectedHoldings {
                observations: vec![observation],
            },
            valuations,
        },
        42,
    )
    .expect("snapshot");
    assert_eq!(snapshot.network_pins.len(), 1);
    assert_eq!(
        snapshot.network_pins[0].anchor,
        ExecutionAnchor::Evm {
            chain_id: 1,
            block_number: 10,
            block_hash: EVM_HASH.to_owned(),
        }
    );

    let report = project_report_from_snapshot(snapshot, 2).expect("report");
    assert_eq!(
        report.totals_by_quote[0].assets_value_dec,
        "2.500000000000000000"
    );
    assert_eq!(
        report.totals_by_quote[0].net_value_dec,
        "2.500000000000000000"
    );
}

#[test]
fn assemble_hard_fails_when_required_holding_observation_missing() {
    let portfolio = sample_portfolio();
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(portfolio.symbol_configs.clone()).expect("valuation config"),
    )
    .expect("valuations");

    let err = assemble_snapshot(
        &AssembleSnapshotConfig::new(2, portfolio).expect("assemble config"),
        AssembleSnapshotInput {
            subjects,
            holdings: SelectedHoldings {
                observations: Vec::new(),
            },
            valuations,
        },
        42,
    )
    .expect_err("missing required holding must hard-fail");
    let msg = err.to_string();
    assert!(
        msg.contains("missing_fact"),
        "expected missing_fact hard-fail, got {msg}"
    );
}

#[test]
fn resolve_valuations_fixed_unit_price_only() {
    let symbol = test_symbol("ethereum-mainnet");
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(vec![symbol]).expect("config"),
    )
    .expect("ok");
    assert_eq!(valuations.valuations.len(), 1);
    assert_eq!(valuations.valuations[0].unit_price_dec, "2.5");
}
