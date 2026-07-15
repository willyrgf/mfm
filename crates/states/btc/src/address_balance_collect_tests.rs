use super::*;
use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcFinality, BtcHeadKind, BtcNetworkId, BtcSourceBinding, BtcSourceIdentity,
    RedactedBtcSourceEvidence,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_program::StateSpec;
use mfm_values::NonEmpty;

const HASH_A: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const HASH_B: &str = "00000000000000000002b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const ADDR_A: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
const ADDR_B: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

fn tip_config() -> ResolveBtcJointTipConfig {
    ResolveBtcJointTipConfig {
        network: "bitcoin-mainnet".to_owned(),
        bitcoin_network: "main".to_owned(),
        semantic_source_identity: "public-bitcoin-core".to_owned(),
        max_source_reads: NonZeroU64::new(1).expect("nz"),
    }
}

fn observe_config(address: &str) -> ObserveBtcAddressBalanceConfig {
    ObserveBtcAddressBalanceConfig {
        network: "bitcoin-mainnet".to_owned(),
        bitcoin_network: "main".to_owned(),
        semantic_source_identity: "public-bitcoin-core".to_owned(),
        address: address.to_owned(),
        coverage: "configured_only".to_owned(),
        max_source_reads: NonZeroU64::new(1).expect("nz"),
    }
}

#[test]
fn joint_tip_selection_is_always_best() {
    assert_eq!(tip_config().selection(), BtcHeadSelection::best());
}

fn binding() -> BtcSourceBinding {
    BtcSourceBinding::new(
        BtcNetworkId::new("bitcoin-mainnet").expect("network"),
        BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
        BitcoinNetworkTag::Main,
    )
}

fn chain_head_response(
    height: u64,
    hash: &str,
    status: BtcSourceStatus,
) -> CapabilityChainHeadResponse {
    CapabilityChainHeadResponse {
        evidence: RedactedBtcSourceEvidence::from_binding(&binding(), "main", status)
            .expect("evidence"),
        head_kind: BtcHeadKind::Best,
        finality: BtcFinality::BestAvailable,
        block_height: height,
        block_hash: BtcBlockHash::new(hash).expect("hash"),
        provider_time_unix_ms: None,
    }
}

fn balance_response(
    address: &str,
    height: u64,
    hash: &str,
    balance_sats: u64,
) -> BtcBalanceReadResponse {
    BtcBalanceReadResponse {
        evidence: RedactedBtcSourceEvidence::from_binding(
            &binding(),
            "main",
            BtcSourceStatus::Synced,
        )
        .expect("evidence"),
        address: BtcAddress::new(address).expect("address"),
        balance_sats,
        block_height: height,
        block_hash: BtcBlockHash::new(hash).expect("hash"),
    }
}

#[test]
fn joint_tip_rejects_ibd_and_missing_hash() {
    let ibd = chain_head_response(100, HASH_A, BtcSourceStatus::InitialBlockDownload);
    let error = materialize_btc_joint_tip(&tip_config(), &ibd).expect_err("ibd");
    assert!(error.to_string().contains("not admissible"));

    // Construct tip with empty hash via new() path.
    let empty = BtcJointTip::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        100,
        "",
        "synced",
        "main",
    );
    assert!(empty.is_err());
}

#[test]
fn prove_before_write_rejects_tip_drift_and_hash_mismatch() {
    let tip = materialize_btc_joint_tip(
        &tip_config(),
        &chain_head_response(100, HASH_A, BtcSourceStatus::Synced),
    )
    .expect("tip");
    let drifted = balance_response(ADDR_A, 101, HASH_A, 50);
    let error = normalize_btc_address_balance_observation(&observe_config(ADDR_A), &tip, &drifted)
        .expect_err("drift");
    assert!(error.to_string().contains("tip drift") || error.to_string().contains("hash mismatch"));

    let mismatched = balance_response(ADDR_A, 100, HASH_B, 50);
    let error =
        normalize_btc_address_balance_observation(&observe_config(ADDR_A), &tip, &mismatched)
            .expect_err("hash mismatch");
    assert!(error.to_string().contains("tip drift") || error.to_string().contains("hash mismatch"));
}

#[test]
fn prove_before_write_requires_height_hash_and_admissible_coverage_status() {
    let tip = materialize_btc_joint_tip(
        &tip_config(),
        &chain_head_response(100, HASH_A, BtcSourceStatus::Synced),
    )
    .expect("tip");
    let balance = balance_response(ADDR_A, 100, HASH_A, 42);
    let ok = normalize_btc_address_balance_observation(&observe_config(ADDR_A), &tip, &balance)
        .expect("ok");
    assert_eq!(ok.response().anchor_height(), 100);
    assert_eq!(ok.response().anchor_hash(), HASH_A);
    assert_eq!(ok.response().balance_sats(), 42);
    assert_eq!(ok.response().coverage(), "configured_only");
    assert_eq!(ok.response().source_status(), "ok");
    assert!(!ok.subject().address().is_empty());
    // No wallet_id / symbol_id on subject.
    let json = serde_json::to_string(&ok.to_fact()).expect("json");
    for forbidden in ["wallet_id", "symbol_id", "rpc_url", "password", "http://"] {
        assert!(!json.contains(forbidden), "leaked {forbidden}");
    }

    let mut bad_coverage = observe_config(ADDR_A);
    bad_coverage.coverage = "truncated".to_owned();
    assert!(validate_observe_btc_address_balance_config(&bad_coverage).is_err());
}

#[test]
fn multi_subject_batch_must_share_one_joint_tip() {
    let tip = materialize_btc_joint_tip(
        &tip_config(),
        &chain_head_response(100, HASH_A, BtcSourceStatus::Synced),
    )
    .expect("tip");
    let facts = vec![balance_fact(&tip, ADDR_A, 1), balance_fact(&tip, ADDR_B, 2)];
    let receipt =
        assemble_btc_network_collection_receipt(AssembleBtcNetworkCollectionReceiptInput {
            joint_tip: tip,
            balance_facts: NonEmpty::try_from_vec(facts).expect("non-empty facts"),
        })
        .expect("receipt");
    assert_eq!(receipt.entries().len(), 2);
    assert_eq!(receipt.successful_observation_count(), 2);
    assert_eq!(receipt.entries()[0].coverage(), BTC_NATIVE_BALANCE_COVERAGE);
    assert_eq!(receipt.entries()[0].source_status(), "ok");
}

fn balance_fact(
    tip: &BtcJointTip,
    address: &str,
    balance_sats: u64,
) -> BtcAddressBalanceSnapshotFact {
    normalize_btc_address_balance_observation(
        &observe_config(address),
        tip,
        &balance_response(address, tip.block_height(), tip.block_hash(), balance_sats),
    )
    .expect("observation")
    .to_fact()
}

#[test]
fn receipt_rejects_tampered_identity_fact_anchor_and_duplicate_source() {
    let tip = materialize_btc_joint_tip(
        &tip_config(),
        &chain_head_response(100, HASH_A, BtcSourceStatus::Synced),
    )
    .expect("tip");
    let fact = balance_fact(&tip, ADDR_A, 1);
    let receipt =
        assemble_btc_network_collection_receipt(AssembleBtcNetworkCollectionReceiptInput {
            joint_tip: tip.clone(),
            balance_facts: NonEmpty::try_from_vec(vec![fact.clone()]).expect("one fact"),
        })
        .expect("receipt");

    let mut tampered_identity = serde_json::to_value(&receipt).expect("receipt JSON");
    tampered_identity["entries"][0]["fact_content_identity"]["response_hash"] = serde_json::json!(
        "sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert!(serde_json::from_value::<BtcNetworkCollectionReceipt>(tampered_identity).is_err());

    let mut tampered_fact = serde_json::to_value(&receipt).expect("receipt JSON");
    tampered_fact["entries"][0]["verified_fact"]["response"]["balance_sats"] =
        serde_json::json!(2_u64);
    assert!(serde_json::from_value::<BtcNetworkCollectionReceipt>(tampered_fact).is_err());

    let duplicate =
        assemble_btc_network_collection_receipt(AssembleBtcNetworkCollectionReceiptInput {
            joint_tip: tip.clone(),
            balance_facts: NonEmpty::try_from_vec(vec![fact.clone(), fact]).expect("facts"),
        });
    assert!(duplicate.is_err());

    let non_fixed_coverage = BtcAddressBalanceSnapshotFact::new(
        BtcAddressBalanceSubject::new("bitcoin-mainnet", "main", "public-bitcoin-core", ADDR_A)
            .expect("subject"),
        BtcAddressBalanceResponse::new(
            tip.block_height(),
            tip.block_hash(),
            1,
            CoverageStatus::CompleteAtAnchor,
            HoldingSourceStatus::Ok,
        )
        .expect("response"),
    );
    assert!(
        assemble_btc_network_collection_receipt(AssembleBtcNetworkCollectionReceiptInput {
            joint_tip: tip.clone(),
            balance_facts: NonEmpty::try_from_vec(vec![non_fixed_coverage]).expect("fact"),
        })
        .is_err()
    );

    let other_tip = materialize_btc_joint_tip(
        &tip_config(),
        &chain_head_response(99, HASH_B, BtcSourceStatus::Synced),
    )
    .expect("other tip");
    let mismatched_anchor =
        assemble_btc_network_collection_receipt(AssembleBtcNetworkCollectionReceiptInput {
            joint_tip: other_tip,
            balance_facts: NonEmpty::try_from_vec(vec![balance_fact(&tip, ADDR_A, 1)])
                .expect("fact"),
        });
    assert!(mismatched_anchor.is_err());
}

#[test]
fn balance_states_require_exact_state_owned_read_budgets() {
    let mut bad_tip = tip_config();
    bad_tip.max_source_reads = NonZeroU64::new(2).expect("non-zero");
    assert!(validate_resolve_btc_joint_tip_config(&bad_tip).is_err());

    let mut bad_observation = observe_config(ADDR_A);
    bad_observation.max_source_reads = NonZeroU64::new(2).expect("non-zero");
    assert!(validate_observe_btc_address_balance_config(&bad_observation).is_err());
}

#[test]
fn record_state_advertises_address_balance_fact_descriptor() {
    let descriptors =
        RecordBtcAddressBalanceFactState::emitted_fact_descriptors().expect("descriptors");
    assert_eq!(descriptors.len(), 1);
    assert!(ObserveBtcAddressBalanceState::emitted_fact_descriptors()
        .expect("observe")
        .is_empty());
    assert!(ResolveBtcJointTipState::emitted_fact_descriptors()
        .expect("tip")
        .is_empty());
}

#[test]
fn record_rejects_inadmissible_observation_response() {
    let tip = materialize_btc_joint_tip(
        &tip_config(),
        &chain_head_response(100, HASH_A, BtcSourceStatus::Synced),
    )
    .expect("tip");
    let obs = normalize_btc_address_balance_observation(
        &observe_config(ADDR_A),
        &tip,
        &balance_response(ADDR_A, 100, HASH_A, 1),
    )
    .expect("obs");
    // Tamper by rebuilding observation with empty-hash response via serde.
    let mut value = serde_json::to_value(&obs).expect("json");
    value["response"]["anchor_hash"] = serde_json::json!("");
    let tampered: BtcAddressBalanceObservation = serde_json::from_value(value).expect("decode");
    let state = RecordBtcAddressBalanceFactState;
    let context = mfm_program::CertifiedContext::no_context();
    let result = poll_ready(state.run(
        RecordBtcAddressBalanceFactInput {
            observation: tampered,
        },
        &(FactRecordCapability,),
        &context,
    ));
    assert!(result.is_err());
}

fn poll_ready<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match std::future::Future::poll(future.as_mut(), &mut cx) {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => panic!("pending"),
    }
}
