use super::*;

use std::collections::BTreeMap;

use crate::PublicMetadata;
use crate::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, PortfolioConfig, PortfolioReport,
    PortfolioSnapshot,
};
use crate::{
    HoldingSourceConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolValuationConfig,
};
use crate::{WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind};
use alloy_primitives::U256;
use mfm_bitcoin::BitcoinBalanceSnapshotFact;
use mfm_evm::{EvmBalanceSnapshotFact, EvmBlockAnchor};
use mfm_facts::MfmFactType;

const EVM_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";

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
        subject: WalletSubject::new(EVM_ACCOUNT, WalletSubjectKind::EvmAddress)
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

fn selected_native(raw_units: &str) -> SelectedHoldings {
    SelectedHoldings {
        observations: vec![Observation {
            wallet_id: "wallet_main".to_owned(),
            symbol_id: "eth.native.ethereum-mainnet".to_owned(),
            display_symbol: Some("ETH".to_owned()),
            network_id: "ethereum-mainnet".to_owned(),
            quantity: ObservationQuantity {
                raw_dec: raw_units.to_owned(),
                decimals: 18,
                amount_dec: "1".to_owned(),
            },
            values: Vec::new(),
            source: AnchoredHoldingSource {
                holding: HoldingSourceConfig::Native,
                anchor: ExecutionAnchor::Evm {
                    chain_id: std::num::NonZeroU64::new(1).expect("non-zero chain"),
                    block: EvmBlockAnchor::new(
                        U256::from(10),
                        EVM_HASH.parse().expect("anchor hash"),
                    ),
                },
            },
            metadata: PublicMetadata::default(),
        }],
    }
}

#[test]
fn unknown_portfolio_config_fields_are_rejected_on_decode() {
    assert_unknown_field_rejected::<PortfolioConfig>(
        r#"{"unexpected_field":1}"#,
        "portfolio config",
    );
}

#[test]
fn output_projection_configs_carry_no_version_policy() {
    let mut snapshot_config =
        serde_json::to_value(AssembleSnapshotConfig::new(sample_portfolio()).expect("config"))
            .expect("snapshot config serializes");
    assert!(snapshot_config.get("snapshot_version").is_none());
    snapshot_config["snapshot_version"] = serde_json::json!("unsupported");
    assert!(serde_json::from_value::<AssembleSnapshotConfig>(snapshot_config).is_err());

    let mut report_config =
        serde_json::to_value(ProjectReportConfig::default()).expect("report config serializes");
    assert_eq!(report_config, serde_json::json!({}));
    report_config["report_version"] = serde_json::json!("unsupported");
    assert!(serde_json::from_value::<ProjectReportConfig>(report_config).is_err());
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
fn selection_config_carries_both_family_descriptors() {
    let config = SelectHoldingsConfig::new(
        sample_portfolio(),
        SelectHoldingsFactDescriptors::new(
            &BitcoinBalanceSnapshotFact::descriptor().expect("Bitcoin descriptor"),
            &EvmBalanceSnapshotFact::descriptor().expect("EVM descriptor"),
        )
        .expect("holding descriptors"),
    )
    .expect("config");
    assert_eq!(
        config.selection_policy_id(),
        PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID
    );
    let mut value = serde_json::to_value(config).expect("config value");
    value["candidate_scan_limit"] = serde_json::json!(11);
    assert_unknown_field_rejected::<SelectHoldingsConfig>(
        &serde_json::to_string(&value).expect("config JSON"),
        "select holdings config",
    );
}

#[test]
fn store_selected_evm_holding_assembles_totals_and_exact_network_pin() {
    let snapshot = assemble_snapshot(
        &AssembleSnapshotConfig::new(sample_portfolio()).expect("assemble config"),
        AssembleSnapshotInput {
            holdings: selected_native("1000000000000000000"),
        },
    )
    .expect("snapshot");
    assert_eq!(snapshot.schema_version, PortfolioSnapshot::SCHEMA_VERSION);
    assert_eq!(snapshot.network_pins.len(), 1);
    assert_eq!(snapshot.network_pins[0].network_id, "ethereum-mainnet");
    let mut unsupported_snapshot = snapshot.clone();
    unsupported_snapshot.schema_version = u64::MAX;
    assert!(project_report_from_snapshot(unsupported_snapshot).is_err());

    let report = project_report_from_snapshot(snapshot).expect("report");
    assert_eq!(report.schema_version, PortfolioReport::SCHEMA_VERSION);
    assert_eq!(report.totals_by_quote[0].total_value_dec, "2.5");
    assert_eq!(
        report.wallet_summaries[0].totals_by_quote[0].total_value_dec,
        "2.5"
    );
}

#[test]
fn snapshot_assembly_rejects_missing_duplicate_and_tampered_selected_holdings() {
    let config = AssembleSnapshotConfig::new(sample_portfolio()).expect("assemble config");
    let input = |holdings| AssembleSnapshotInput { holdings };

    assert!(assemble_snapshot(
        &config,
        input(SelectedHoldings {
            observations: Vec::new()
        })
    )
    .is_err());
    let selected = selected_native("1000000000000000000");
    let mut duplicate = selected.clone();
    duplicate
        .observations
        .push(selected.observations[0].clone());
    assert!(assemble_snapshot(&config, input(duplicate)).is_err());

    let mut tampered = selected;
    tampered.observations[0].source.holding = HoldingSourceConfig::Erc20 {
        contract_address: TOKEN.parse().expect("token address"),
    };
    assert!(assemble_snapshot(&config, input(tampered)).is_err());
}
