use super::*;

use std::collections::BTreeMap;

use alloy_primitives::U256;
use mfm_evm_capabilities::{
    EvmBlockAnchor, EvmSessionEvidence, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_ids::LocalPublicId;
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{
    ExecutionAnchor, NetworkConfig, NetworkFamilyConfig, PortfolioConfig, PortfolioReport,
    PortfolioSnapshot,
};
use mfm_portfolio_model::symbol::{
    HoldingSourceConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};
use mfm_program::{MfmFactType, ReadState, StateSpec, ValidatedConfig};
use mfm_states_btc::BtcAddressBalanceSnapshotFact;

const EVM_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const REORG_HASH: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const SECOND_ACCOUNT: &str = "0x000000000000000000000000000000000000beef";
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

fn native_source(account: &str) -> EvmBalanceSource {
    EvmBalanceSource::new(
        account.parse().expect("normalized account"),
        HoldingSourceConfig::Native,
    )
    .expect("native source")
}

fn token_source(account: &str) -> EvmBalanceSource {
    EvmBalanceSource::new(
        account.parse().expect("normalized account"),
        HoldingSourceConfig::Erc20 {
            contract_address: TOKEN.parse().expect("normalized token"),
        },
    )
    .expect("token source")
}

fn collection_config(sources: Vec<EvmBalanceSource>) -> EvmNetworkCollectionConfig {
    EvmNetworkCollectionConfig::new(&test_network("ethereum-mainnet", 1), sources)
        .expect("collection config")
}

fn collection_plan(config: &EvmNetworkCollectionConfig) -> CollectEvmNetworkPlan {
    let state = <CollectEvmNetworkState as StateSpec>::new(
        ValidatedConfig::new(config.clone()).expect("validated collection config"),
    )
    .expect("collection state");
    state
        .plan(&(), &mfm_program::CertifiedContext::no_context())
        .expect("collection plan")
}

fn session(config: &EvmNetworkCollectionConfig) -> EvmSessionEvidence {
    EvmSessionEvidence::new(
        &config.binding().expect("network binding"),
        LocalPublicId::new("primary").expect("source ref"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation id"),
    )
}

fn evidence_for(config: &EvmNetworkCollectionConfig, values: &[u64]) -> CollectEvmNetworkEvidence {
    assert_eq!(config.sources().len(), values.len());
    let token_decimals = collection_plan(config)
        .token_contracts()
        .into_iter()
        .map(|contract| EvmTokenDecimalsEvidence::new(contract, 6).expect("token decimals"))
        .collect();
    let balances = config
        .sources()
        .iter()
        .cloned()
        .zip(values.iter().copied())
        .map(|(source, value)| EvmBalanceReadEvidence::new(source, U256::from(value)))
        .collect();
    CollectEvmNetworkEvidence::new(
        &session(config),
        EvmBlockAnchor::new(U256::from(10), EVM_HASH.parse().expect("anchor hash")),
        token_decimals,
        balances,
        EvmBlockAnchor::new(U256::from(10), EVM_HASH.parse().expect("final anchor hash")),
    )
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
            coverage: "complete_at_anchor".to_owned(),
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
    snapshot_config["snapshot_version"] = serde_json::json!(2);
    assert!(serde_json::from_value::<AssembleSnapshotConfig>(snapshot_config).is_err());

    let mut report_config =
        serde_json::to_value(ProjectReportConfig::default()).expect("report config serializes");
    assert_eq!(report_config, serde_json::json!({}));
    report_config["report_version"] = serde_json::json!(2);
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
            &BtcAddressBalanceSnapshotFact::descriptor().expect("Bitcoin descriptor"),
            &EvmBalanceSnapshotFact::descriptor().expect("EVM descriptor"),
        )
        .expect("holding descriptors"),
    )
    .expect("config");
    assert_eq!(
        config.selection_policy_id(),
        PORTFOLIO_HOLDING_COLLECTION_RECEIPT_ANCHOR_POLICY_ID
    );
    assert_eq!(config.candidate_bound(), 10);
    assert_eq!(config.candidate_scan_limit(), 11);
    let mut value = serde_json::to_value(config).expect("config value");
    value["candidate_scan_limit"] = serde_json::json!(12);
    let tampered: SelectHoldingsConfig = serde_json::from_value(value).expect("decode config");
    assert!(ValidatedConfig::new(tampered).is_err());
}

#[test]
fn collection_config_is_sorted_unique_and_bounded() {
    let token = token_source(EVM_ACCOUNT);
    let native = native_source(EVM_ACCOUNT);
    let config = collection_config(vec![token.clone(), native.clone()]);
    assert_eq!(config.sources(), &[native, token]);
    assert_eq!(collection_plan(&config).sources(), config.sources());

    assert!(
        EvmNetworkCollectionConfig::new(&test_network("ethereum-mainnet", 1), Vec::new()).is_err()
    );
    assert!(EvmNetworkCollectionConfig::new(
        &test_network("ethereum-mainnet", 1),
        vec![native_source(EVM_ACCOUNT), native_source(EVM_ACCOUNT)]
    )
    .is_err());

    let excessive = (1..=EVM_NETWORK_HOLDING_SOURCE_LIMIT + 1)
        .map(|index| native_source(&format!("0x{index:040x}")))
        .collect();
    assert!(
        EvmNetworkCollectionConfig::new(&test_network("ethereum-mainnet", 1), excessive).is_err()
    );
}

#[test]
fn reducer_deduplicates_metadata_and_publishes_one_unified_fact_per_source() {
    let config = collection_config(vec![
        token_source(SECOND_ACCOUNT),
        native_source(EVM_ACCOUNT),
        token_source(EVM_ACCOUNT),
    ]);
    let plan = collection_plan(&config);
    assert_eq!(
        plan.token_contracts()
            .iter()
            .map(|address| address.as_str())
            .collect::<Vec<_>>(),
        vec![TOKEN]
    );

    let evidence = evidence_for(&config, &[10, 20, 30]);
    let batch = reduce_evm_network_collection(&plan, &evidence).expect("reduced collection");
    assert_eq!(batch.anchor().number(), "10");
    assert_eq!(batch.balances().len(), 3);
    assert_eq!(
        batch
            .balances()
            .iter()
            .filter(|balance| {
                matches!(balance.source().asset(), HoldingSourceConfig::Erc20 { .. })
            })
            .map(EvmCollectedBalance::decimals)
            .collect::<Vec<_>>(),
        vec![6, 6]
    );

    let (receipt, facts) = publish_evm_holdings(&config, batch).expect("published facts");
    assert_eq!(facts.len(), 3);
    assert_eq!(receipt.sources().len(), 3);
    assert_eq!(receipt.fact_content_identities().len(), 3);
    assert_eq!(receipt.block_anchor().hash(), EVM_HASH);
    let descriptor = EvmBalanceSnapshotFact::descriptor().expect("unified descriptor");
    assert_eq!(descriptor.fact_kind().as_str(), "evm.balance_snapshot");
    assert!(descriptor
        .fields()
        .iter()
        .any(|field| field.field_id().as_str() == "subject.asset.kind"));
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "subject.asset.contract_address" && !field.required()
    }));
    assert!(descriptor
        .fields()
        .iter()
        .any(|field| field.field_id().as_str() == "result.block_anchor.hash"));
    let fact_json = serde_json::to_value(&facts).expect("fact JSON");
    assert_eq!(fact_json[0]["response"]["block_anchor"]["hash"], EVM_HASH);
    assert_eq!(
        fact_json
            .as_array()
            .expect("fact array")
            .iter()
            .map(|fact| fact["subject"]["asset"]["kind"]
                .as_str()
                .expect("asset kind"))
            .collect::<Vec<_>>(),
        vec!["erc20", "native", "erc20",]
    );
    let fact_keys = fact_json
        .as_array()
        .expect("fact array")
        .iter()
        .map(|fact| {
            mfm_facts::typed_fact_subject_evidence(&descriptor, &fact["subject"])
                .expect("subject evidence")
                .fact_key()
                .as_str()
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        fact_keys.len(),
        3,
        "every configured asset has one fact key"
    );
}

#[test]
fn reducer_rejects_reorg_session_order_coverage_and_decimal_tampering() {
    let config = collection_config(vec![native_source(EVM_ACCOUNT), token_source(EVM_ACCOUNT)]);
    let plan = collection_plan(&config);
    let evidence = evidence_for(&config, &[10, 20]);

    let mut reorg = serde_json::to_value(&evidence).expect("evidence JSON");
    reorg["final_canonical_block"]["hash"] = serde_json::json!(REORG_HASH);
    let reorg = serde_json::from_value(reorg).expect("reorg evidence");
    assert!(reduce_evm_network_collection(&plan, &reorg)
        .expect_err("reorg must fail")
        .to_string()
        .contains("no longer canonical"));

    let mut wrong_session = serde_json::to_value(&evidence).expect("evidence JSON");
    wrong_session["session"]["chain_id"] = serde_json::json!(2);
    let wrong_session = serde_json::from_value(wrong_session).expect("session evidence");
    assert!(reduce_evm_network_collection(&plan, &wrong_session).is_err());

    let mut wrong_order = serde_json::to_value(&evidence).expect("evidence JSON");
    wrong_order["balances"]
        .as_array_mut()
        .expect("balance array")
        .swap(0, 1);
    let wrong_order = serde_json::from_value(wrong_order).expect("ordered evidence");
    assert!(reduce_evm_network_collection(&plan, &wrong_order).is_err());

    let mut duplicated = serde_json::to_value(&evidence).expect("evidence JSON");
    let duplicate = duplicated["balances"][0].clone();
    duplicated["balances"]
        .as_array_mut()
        .expect("balance array")
        .push(duplicate);
    let duplicated = serde_json::from_value(duplicated).expect("duplicated evidence");
    assert!(reduce_evm_network_collection(&plan, &duplicated).is_err());

    let mut missing_metadata = serde_json::to_value(&evidence).expect("evidence JSON");
    missing_metadata["token_decimals"] = serde_json::json!([]);
    let missing_metadata = serde_json::from_value(missing_metadata).expect("metadata evidence");
    assert!(reduce_evm_network_collection(&plan, &missing_metadata).is_err());

    let mut noncanonical = serde_json::to_value(&evidence).expect("evidence JSON");
    noncanonical["balances"][0]["raw_units"] = serde_json::json!("010");
    let noncanonical = serde_json::from_value(noncanonical).expect("quantity evidence");
    assert!(reduce_evm_network_collection(&plan, &noncanonical).is_err());
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
    assert_eq!(
        snapshot.wallets[0].observations[0].coverage,
        "complete_at_anchor"
    );

    let mut unsupported_snapshot = snapshot.clone();
    unsupported_snapshot.schema_version = 2;
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
