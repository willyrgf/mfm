use super::*;

use alloy_primitives::{Address, U256};
use mfm_evm_capabilities::{
    EvmBlockAnchor, EvmSessionEvidence, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_ids::LocalPublicId;
use mfm_program::{MfmFactType, ReadState, StateSpec, ValidatedConfig};

const ANCHOR_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const REORG_HASH: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const SECOND_ACCOUNT: &str = "0x000000000000000000000000000000000000beef";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";

fn address(value: &str) -> Address {
    value.parse().expect("canonical EVM address")
}

fn native_source(account: &str) -> EvmBalanceSource {
    EvmBalanceSource::new(address(account), EvmBalanceAsset::Native).expect("native source")
}

fn token_source(account: &str) -> EvmBalanceSource {
    EvmBalanceSource::new(
        address(account),
        EvmBalanceAsset::erc20(address(TOKEN)).expect("token asset"),
    )
    .expect("token source")
}

fn collection_config(sources: Vec<EvmBalanceSource>) -> EvmBalanceCollectionConfig {
    EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, sources).expect("collection config")
}

fn collection_plan(config: &EvmBalanceCollectionConfig) -> EvmBalanceCollectionPlan {
    let state = <CollectEvmBalancesState as StateSpec>::new(
        ValidatedConfig::new(config.clone()).expect("validated collection config"),
    )
    .expect("collection state");
    state
        .plan(&(), &mfm_program::CertifiedContext::no_context())
        .expect("collection plan")
}

fn session(config: &EvmBalanceCollectionConfig) -> EvmSessionEvidence {
    EvmSessionEvidence::new(
        &config.binding().expect("network binding"),
        LocalPublicId::new("primary").expect("source ref"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation id"),
    )
}

fn evidence_for(
    config: &EvmBalanceCollectionConfig,
    values: &[u64],
) -> EvmBalanceCollectionEvidence {
    assert_eq!(config.sources().len(), values.len());
    let token_decimals = collection_plan(config)
        .token_contracts()
        .into_iter()
        .map(|contract| {
            EvmTokenDecimalsEvidence::new(address(&contract), 6).expect("token decimals")
        })
        .collect();
    let balances = config
        .sources()
        .iter()
        .cloned()
        .zip(values.iter().copied())
        .map(|(source, value)| EvmBalanceReadEvidence::new(source, U256::from(value)))
        .collect();
    EvmBalanceCollectionEvidence::new(
        &session(config),
        EvmBlockAnchor::new(U256::from(10), address_hash(ANCHOR_HASH)),
        token_decimals,
        balances,
        EvmBlockAnchor::new(U256::from(10), address_hash(ANCHOR_HASH)),
    )
}

fn address_hash(value: &str) -> alloy_primitives::B256 {
    value.parse().expect("canonical EVM hash")
}

#[test]
fn collection_config_is_sorted_unique_and_bounded() {
    let token = token_source(ACCOUNT);
    let native = native_source(ACCOUNT);
    let config = collection_config(vec![token.clone(), native.clone()]);
    assert_eq!(config.sources(), &[native, token]);
    assert_eq!(collection_plan(&config).sources(), config.sources());

    assert!(EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, Vec::new()).is_err());
    assert!(EvmBalanceCollectionConfig::new(
        "ethereum-mainnet",
        1,
        18,
        vec![native_source(ACCOUNT), native_source(ACCOUNT)],
    )
    .is_err());
    assert!(EvmBalanceCollectionConfig::new(
        "ethereum-mainnet",
        0,
        18,
        vec![native_source(ACCOUNT)],
    )
    .is_err());
    assert!(EvmBalanceAsset::erc20(Address::ZERO).is_err());

    let excessive = (1..=EVM_BALANCE_COLLECTION_SOURCE_LIMIT + 1)
        .map(|index| native_source(&format!("0x{index:040x}")))
        .collect();
    assert!(EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, excessive).is_err());
}

#[test]
fn balance_asset_deserialization_preserves_the_closed_fact_shape() {
    assert_eq!(
        serde_json::from_value::<EvmBalanceAsset>(serde_json::json!({ "kind": "native" }))
            .expect("native asset"),
        EvmBalanceAsset::Native
    );
    assert!(
        serde_json::from_value::<EvmBalanceAsset>(serde_json::json!({
            "kind": "native",
            "contract_address": null
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<EvmBalanceAsset>(serde_json::json!({
            "kind": "erc20"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<EvmBalanceAsset>(serde_json::json!({
            "kind": "erc20",
            "contract_address": null
        }))
        .is_err()
    );
}

#[test]
fn reducer_deduplicates_metadata_and_records_one_unified_fact_per_source() {
    let config = collection_config(vec![
        token_source(SECOND_ACCOUNT),
        native_source(ACCOUNT),
        token_source(ACCOUNT),
    ]);
    let plan = collection_plan(&config);
    assert_eq!(plan.token_contracts(), vec![TOKEN.to_owned()]);

    let evidence = evidence_for(&config, &[10, 20, 30]);
    let batch = reduce_evm_balance_collection(&plan, &evidence).expect("reduced collection");
    assert_eq!(batch.anchor().number(), "10");
    assert_eq!(batch.balances().len(), 3);
    assert_eq!(
        batch
            .balances()
            .iter()
            .filter(|balance| matches!(balance.source().asset(), EvmBalanceAsset::Erc20 { .. }))
            .map(EvmBalanceObservation::decimals)
            .collect::<Vec<_>>(),
        vec![6, 6]
    );

    let (receipt, facts) =
        record_evm_balance_facts(&config, batch).expect("recorded fact material");
    assert_eq!(facts.len(), 3);
    assert_eq!(receipt.sources().len(), 3);
    assert_eq!(receipt.fact_content_identities().len(), 3);
    assert_eq!(receipt.block_anchor().hash(), ANCHOR_HASH);
    let descriptor = EvmBalanceSnapshotFact::descriptor().expect("unified descriptor");
    assert_eq!(descriptor.fact_kind().as_str(), "evm.balance_snapshot");
    assert!(descriptor
        .fields()
        .iter()
        .any(|field| field.field_id().as_str() == "subject.asset.kind"));
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "subject.asset.contract_address" && !field.required()
    }));
    let fact_json = serde_json::to_value(&facts).expect("fact JSON");
    assert_eq!(
        fact_json[0]["response"]["block_anchor"]["hash"],
        ANCHOR_HASH
    );
    assert_eq!(
        fact_json
            .as_array()
            .expect("fact array")
            .iter()
            .map(|fact| fact["subject"]["asset"]["kind"]
                .as_str()
                .expect("asset kind"))
            .collect::<Vec<_>>(),
        vec!["erc20", "native", "erc20"]
    );
}

#[test]
fn reducer_rejects_reorg_session_order_coverage_and_quantity_tampering() {
    let config = collection_config(vec![native_source(ACCOUNT), token_source(ACCOUNT)]);
    let plan = collection_plan(&config);
    let evidence = evidence_for(&config, &[10, 20]);

    let mut reorg = serde_json::to_value(&evidence).expect("evidence JSON");
    reorg["final_canonical_block"]["hash"] = serde_json::json!(REORG_HASH);
    let reorg = serde_json::from_value(reorg).expect("reorg evidence");
    assert!(reduce_evm_balance_collection(&plan, &reorg)
        .expect_err("reorg must fail")
        .to_string()
        .contains("no longer canonical"));

    let mut wrong_session = serde_json::to_value(&evidence).expect("evidence JSON");
    wrong_session["session"]["chain_id"] = serde_json::json!(2);
    let wrong_session = serde_json::from_value(wrong_session).expect("session evidence");
    assert!(reduce_evm_balance_collection(&plan, &wrong_session).is_err());

    let mut wrong_order = serde_json::to_value(&evidence).expect("evidence JSON");
    wrong_order["balances"]
        .as_array_mut()
        .expect("balance array")
        .swap(0, 1);
    let wrong_order = serde_json::from_value(wrong_order).expect("ordered evidence");
    assert!(reduce_evm_balance_collection(&plan, &wrong_order).is_err());

    let mut duplicated = serde_json::to_value(&evidence).expect("evidence JSON");
    let duplicate = duplicated["balances"][0].clone();
    duplicated["balances"]
        .as_array_mut()
        .expect("balance array")
        .push(duplicate);
    let duplicated = serde_json::from_value(duplicated).expect("duplicated evidence");
    assert!(reduce_evm_balance_collection(&plan, &duplicated).is_err());

    let mut missing_metadata = serde_json::to_value(&evidence).expect("evidence JSON");
    missing_metadata["token_decimals"] = serde_json::json!([]);
    let missing_metadata = serde_json::from_value(missing_metadata).expect("metadata evidence");
    assert!(reduce_evm_balance_collection(&plan, &missing_metadata).is_err());

    let mut noncanonical = serde_json::to_value(&evidence).expect("evidence JSON");
    noncanonical["balances"][0]["raw_units"] = serde_json::json!("010");
    let noncanonical = serde_json::from_value(noncanonical).expect("quantity evidence");
    assert!(reduce_evm_balance_collection(&plan, &noncanonical).is_err());
}
