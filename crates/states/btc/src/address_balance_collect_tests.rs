use super::*;
use mfm_btc_capabilities::{
    BitcoinNetworkTag, BtcFinality, BtcHeadKind, BtcNetworkId, BtcSourceBinding, BtcSourceIdentity,
    RedactedBtcSourceEvidence,
};
use mfm_program::StateSpec;

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
    let obs_a = normalize_btc_address_balance_observation(
        &observe_config(ADDR_A),
        &tip,
        &balance_response(ADDR_A, 100, HASH_A, 1),
    )
    .expect("a");
    let obs_b = normalize_btc_address_balance_observation(
        &observe_config(ADDR_B),
        &tip,
        &balance_response(ADDR_B, 100, HASH_A, 2),
    )
    .expect("b");
    require_shared_joint_tip(&[&obs_a, &obs_b]).expect("shared");

    let other_tip = materialize_btc_joint_tip(
        &tip_config(),
        &chain_head_response(99, HASH_B, BtcSourceStatus::Synced),
    )
    .expect("other tip");
    let obs_b_drifted = normalize_btc_address_balance_observation(
        &observe_config(ADDR_B),
        &other_tip,
        &balance_response(ADDR_B, 99, HASH_B, 2),
    )
    .expect("b drifted");
    let error = require_shared_joint_tip(&[&obs_a, &obs_b_drifted]).expect_err("not shared");
    assert!(error.to_string().contains("share one joint tip"));
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
