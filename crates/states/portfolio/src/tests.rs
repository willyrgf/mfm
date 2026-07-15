use super::*;
use std::collections::BTreeMap;

use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, PortfolioConfig,
};
use mfm_portfolio_model::symbol::{
    AnchoredHoldingSource, HoldingSourceConfig, Observation, ObservationAnchor,
    ObservationQuantity, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolValuationConfig,
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
        Some(18),
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
        network_id: network_id.parse().expect("valid network id"),
        source: HoldingSourceConfig::Native,
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: "eth.native.ethereum-mainnet"
                    .parse()
                    .expect("valid priced symbol id"),
                unit_price_dec: "2.5".parse().expect("valid unit price"),
            }],
        },
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
    assert_unknown_field_rejected::<PortfolioConfig>(
        r#"{"unexpected_field":1}"#,
        "portfolio config",
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
fn expand_requirements_projects_the_direct_native_source() {
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
    assert!(requirements[0].symbol.source.is_native());
}

#[test]
fn fixed_price_selection_assembles_direct_totals_and_anchors() {
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
        network_id: "ethereum-mainnet".to_owned(),
        quantity: ObservationQuantity {
            raw_dec: "1000000000000000000".to_owned(),
            decimals: 18,
            amount_dec: "1.000000000000000000".to_owned(),
        },
        values: Vec::new(),
        source: AnchoredHoldingSource {
            holding: HoldingSourceConfig::Native,
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
    assert_eq!(report.totals_by_quote[0].total_value_dec, "2.5");
    assert_eq!(
        report.wallet_summaries[0].totals_by_quote[0].total_value_dec,
        "2.5"
    );
}

#[test]
fn direct_total_reducer_emits_configured_zero_rows() {
    let portfolio = sample_portfolio();
    let subjects = resolve_subjects_from_config(
        &ResolveSubjectsConfig::new(portfolio.wallets.clone()).expect("subjects config"),
    );
    let valuations = resolve_valuations_from_config(
        &ResolveValuationsConfig::new(portfolio.symbol_configs.clone()).expect("valuation config"),
    )
    .expect("valuations");
    let observation = Observation {
        wallet_id: "wallet_main".to_owned(),
        symbol_id: "eth.native.ethereum-mainnet".to_owned(),
        display_symbol: Some("ETH".to_owned()),
        network_id: "ethereum-mainnet".to_owned(),
        quantity: ObservationQuantity {
            raw_dec: "0".to_owned(),
            decimals: 18,
            amount_dec: "0.000000000000000000".to_owned(),
        },
        values: Vec::new(),
        source: AnchoredHoldingSource {
            holding: HoldingSourceConfig::Native,
            anchor: ObservationAnchor::Evm {
                chain_id: 1,
                block_number: 10,
                block_hash: EVM_HASH.to_owned(),
            },
        },
        coverage: "configured_only".to_owned(),
        metadata: PublicMetadata::default(),
    };
    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(2, portfolio).expect("assemble config"),
        AssembleSnapshotInput {
            subjects,
            holdings: SelectedHoldings {
                observations: vec![observation],
            },
            valuations,
        },
    )
    .expect("snapshot");

    let report = project_report_from_snapshot(snapshot, 2).expect("report");
    assert_eq!(report.wallet_summaries[0].totals_by_quote.len(), 1);
    assert_eq!(
        report.wallet_summaries[0].totals_by_quote[0].total_value_dec,
        "0"
    );
    assert_eq!(report.totals_by_quote[0].total_value_dec, "0");
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
    )
    .expect_err("missing required holding must hard-fail");
    assert!(err.to_string().contains("missing_fact"));
}
