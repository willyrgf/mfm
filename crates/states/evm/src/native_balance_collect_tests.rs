use super::*;
use alloy_primitives::{Address, B256, U256};
use mfm_evm_capabilities::{
    EvmNetworkBinding, EvmNetworkId, EvmSourcePolicyId, EvmSourceRef, RedactedEvmSourceEvidence,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_program::StateSpec;
use mfm_values::NonEmpty;

const HASH_A: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const HASH_B: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const ACCT_A: &str = "0x0000000000000000000000000000000000000001";
const ACCT_B: &str = "0x0000000000000000000000000000000000000002";

fn tip_config() -> ResolveEvmJointTipConfig {
    ResolveEvmJointTipConfig {
        network: "ethereum-mainnet".to_owned(),
        chain_id: 1,
        max_source_reads: NonZeroU64::new(1).expect("non-zero source reads"),
    }
}

fn semantic_evm_network() -> NetworkConfig {
    NetworkConfig::new(
        "ethereum-mainnet".to_owned(),
        NetworkFamilyConfig::Evm,
        Some(1),
        Some(18),
        None,
        None,
        std::collections::BTreeMap::new(),
    )
    .expect("EVM network")
}

fn observe_config(account: &str) -> ObserveEvmNativeBalanceConfig {
    ObserveEvmNativeBalanceConfig {
        network: semantic_evm_network(),
        account: account.to_owned(),
        coverage: "configured_only".to_owned(),
        max_source_reads: NonZeroU64::new(EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS)
            .expect("non-zero source reads"),
    }
}

fn evidence() -> RedactedEvmSourceEvidence {
    let binding = EvmNetworkBinding::new(EvmNetworkId::new("ethereum-mainnet").expect("net"), 1)
        .expect("binding");
    RedactedEvmSourceEvidence::from_binding(
        &binding,
        1,
        EvmSourceRef::new("primary").expect("source"),
        EvmSourcePolicyId::new("default").expect("policy"),
    )
    .expect("evidence")
}

fn block_response(number: u64, hash: &str) -> EvmBlockReadResponse {
    let hex = hash.strip_prefix("0x").unwrap_or(hash);
    EvmBlockReadResponse {
        evidence: evidence(),
        block_number: number,
        block_hash: B256::from_str(hex).expect("hash"),
    }
}

fn balance_response(wei: u64) -> EvmBalanceReadResponse {
    EvmBalanceReadResponse {
        evidence: evidence(),
        balance_wei: U256::from(wei),
    }
}

#[test]
fn joint_tip_stores_canonical_block_hash() {
    let mixed = format!("0X{}", "AB".repeat(32));
    let tip = EvmJointTip::new("ethereum-mainnet", 1, 100, mixed).expect("tip");
    assert_eq!(
        tip.block_hash(),
        "0xabababababababababababababababababababababababababababababababab"
    );
    let bare = "CD".repeat(32);
    let tip = EvmJointTip::new("ethereum-mainnet", 1, 100, bare).expect("bare hex tip");
    assert_eq!(
        tip.block_hash(),
        "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
    );
}

#[test]
fn joint_tip_config_requires_exact_state_owned_read_budget() {
    let mut excessive = tip_config();
    excessive.max_source_reads =
        NonZeroU64::new(EVM_JOINT_TIP_SOURCE_READS + 1).expect("non-zero source reads");
    assert!(validate_resolve_evm_joint_tip_config(&excessive)
        .expect_err("excessive read budget")
        .contains("must equal 1"));

    let error = materialize_evm_joint_tip(&excessive, &block_response(100, HASH_A))
        .expect_err("materialization must reject an excessive read budget");
    assert!(error.to_string().contains("must equal 1"));
}

#[test]
fn prove_before_write_rejects_missing_hash_and_tip_drift() {
    let tip = materialize_evm_joint_tip(&tip_config(), &block_response(100, HASH_A)).expect("tip");
    let drifted = EvmJointTip::new("ethereum-mainnet", 1, 101, HASH_A).expect("drift tip");
    let error = normalize_evm_native_balance_from_capability(
        &observe_config(ACCT_A),
        &tip,
        &drifted,
        &balance_response(1),
    )
    .expect_err("drift");
    assert!(error.to_string().contains("tip drift") || error.to_string().contains("hash mismatch"));

    let mismatched = EvmJointTip::new("ethereum-mainnet", 1, 100, HASH_B).expect("hash tip");
    let error = normalize_evm_native_balance_from_capability(
        &observe_config(ACCT_A),
        &tip,
        &mismatched,
        &balance_response(1),
    )
    .expect_err("hash");
    assert!(error.to_string().contains("tip drift") || error.to_string().contains("hash mismatch"));
}

#[test]
fn prove_before_write_admits_hash_bound_balance() {
    let tip = materialize_evm_joint_tip(&tip_config(), &block_response(100, HASH_A)).expect("tip");
    let obs = normalize_evm_native_balance_from_capability(
        &observe_config(ACCT_A),
        &tip,
        &tip,
        &balance_response(42),
    )
    .expect("ok");
    assert_eq!(obs.response().block_number(), 100);
    assert_eq!(obs.response().block_hash(), HASH_A);
    assert_eq!(obs.response().raw_wei(), "42");
    assert_eq!(obs.response().source_status(), "ok");
    assert_eq!(
        obs.source_read_count(),
        EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS
    );
    let json = serde_json::to_string(&obs.to_fact()).expect("json");
    for forbidden in ["wallet_id", "symbol_id", "rpc_url", "password", "http://"] {
        assert!(!json.contains(forbidden), "leaked {forbidden}");
    }
    let _ = Address::from_str(ACCT_A).expect("acct");
}

#[test]
fn observe_config_requires_canonical_account_and_exact_read_budget() {
    let mut noncanonical = observe_config(ACCT_A);
    noncanonical.account = "0x00000000000000000000000000000000000000AA".to_owned();
    assert!(validate_observe_evm_native_balance_config(&noncanonical)
        .expect_err("mixed-case account")
        .contains("normalized lowercase"));

    let mut too_small = observe_config(ACCT_A);
    too_small.max_source_reads = NonZeroU64::new(1).expect("non-zero");
    assert!(validate_observe_evm_native_balance_config(&too_small)
        .expect_err("insufficient read budget")
        .contains("must equal 2"));
}

#[test]
fn multi_subject_batch_must_share_one_joint_tip() {
    let tip = materialize_evm_joint_tip(&tip_config(), &block_response(100, HASH_A)).expect("tip");
    let receipt =
        assemble_evm_native_balance_batch_receipt(AssembleEvmNativeBalanceBatchReceiptInput {
            joint_tip: tip.clone(),
            balance_facts: NonEmpty::try_from_vec(vec![
                native_fact(&tip, ACCT_A, 1),
                native_fact(&tip, ACCT_B, 2),
            ])
            .expect("facts"),
        })
        .expect("receipt");
    assert_eq!(receipt.entries().len(), 2);
    assert_eq!(receipt.successful_observation_count(), 2);
    assert_eq!(receipt.entries()[0].coverage(), EVM_NATIVE_BALANCE_COVERAGE);
    assert_eq!(receipt.entries()[0].source_status(), "ok");
}

fn native_fact(tip: &EvmJointTip, account: &str, wei: u64) -> EvmAddressNativeBalanceSnapshotFact {
    normalize_evm_native_balance_from_capability(
        &observe_config(account),
        tip,
        tip,
        &balance_response(wei),
    )
    .expect("observation")
    .to_fact()
}

#[test]
fn native_receipt_rejects_tampered_identity_fact_anchor_and_duplicate_source() {
    let tip = materialize_evm_joint_tip(&tip_config(), &block_response(100, HASH_A)).expect("tip");
    let fact = native_fact(&tip, ACCT_A, 1);
    let receipt =
        assemble_evm_native_balance_batch_receipt(AssembleEvmNativeBalanceBatchReceiptInput {
            joint_tip: tip.clone(),
            balance_facts: NonEmpty::try_from_vec(vec![fact.clone()]).expect("fact"),
        })
        .expect("receipt");

    let mut tampered_identity = serde_json::to_value(&receipt).expect("receipt JSON");
    tampered_identity["entries"][0]["fact_content_identity"]["response_hash"] = serde_json::json!(
        "sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert!(serde_json::from_value::<EvmNativeBalanceBatchReceipt>(tampered_identity).is_err());

    let mut tampered_fact = serde_json::to_value(&receipt).expect("receipt JSON");
    tampered_fact["entries"][0]["verified_fact"]["response"]["raw_wei"] = serde_json::json!("2");
    assert!(serde_json::from_value::<EvmNativeBalanceBatchReceipt>(tampered_fact).is_err());

    assert!(
        assemble_evm_native_balance_batch_receipt(AssembleEvmNativeBalanceBatchReceiptInput {
            joint_tip: tip.clone(),
            balance_facts: NonEmpty::try_from_vec(vec![fact.clone(), fact]).expect("facts"),
        },)
        .is_err()
    );

    let non_fixed_coverage = EvmAddressNativeBalanceSnapshotFact::new(
        EvmAddressNativeBalanceSubject::new("ethereum-mainnet", 1, ACCT_A).expect("subject"),
        EvmAddressNativeBalanceResponse::new(
            tip.block_number(),
            tip.block_hash(),
            "1",
            18,
            CoverageStatus::CompleteAtAnchor,
            HoldingSourceStatus::Ok,
        )
        .expect("response"),
    );
    assert!(
        assemble_evm_native_balance_batch_receipt(AssembleEvmNativeBalanceBatchReceiptInput {
            joint_tip: tip.clone(),
            balance_facts: NonEmpty::try_from_vec(vec![non_fixed_coverage]).expect("fact"),
        },)
        .is_err()
    );

    let other =
        materialize_evm_joint_tip(&tip_config(), &block_response(99, HASH_B)).expect("other");
    assert!(
        assemble_evm_native_balance_batch_receipt(AssembleEvmNativeBalanceBatchReceiptInput {
            joint_tip: other,
            balance_facts: NonEmpty::try_from_vec(vec![native_fact(&tip, ACCT_A, 1)])
                .expect("fact"),
        },)
        .is_err()
    );
}

#[test]
fn record_state_advertises_native_balance_fact_descriptor() {
    let descriptors =
        RecordEvmNativeBalanceFactState::emitted_fact_descriptors().expect("descriptors");
    assert_eq!(descriptors.len(), 1);
    assert!(ObserveEvmNativeBalanceState::emitted_fact_descriptors()
        .expect("observe")
        .is_empty());
}
