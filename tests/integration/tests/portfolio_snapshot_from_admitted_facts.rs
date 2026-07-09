//! Store-backed portfolio_snapshot complete path from admitted Platform holding facts.
//!
//! Uses the merge-safe **fixture seed** `seed_platform_holding_facts_for_test` (FactRecorded-shaped
//! projection + artifact authority + source envelopes — not live collector IO). Drives certified
//! `portfolio_snapshot` to Completed with chain providers unbound. Replay freezes fact-index call
//! counts (SelectHoldings must not re-query live frontier).
//!
//! Residual (not this test): mock-transport collect observe→record entry points then report.

#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use mfm_canonical::CanonicalValue;
use mfm_events::v1 as events;
use mfm_facts::{
    CoverageStatus, FactAudience, FactProducerProvenance, FactVisibility, HoldingSourceStatus,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, AttemptId, CapabilityKind, CapabilityVersion, DigestAlgorithm,
    DigestBytes, EventId, NodeId, RunId,
};
use mfm_integration_tests::test_support::{
    fact_query_evidences, prepare_portfolio_launch_for_store, seed_platform_holding_facts_for_test,
    FactProjectionFixtureInputForTest, PlatformHoldingFactSeedForTest, ProjectionFactIndexProvider,
};
use mfm_program::MfmFactType;
use mfm_states_btc::{
    BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact, BtcAddressBalanceSubject,
};
use mfm_states_evm::{
    EvmAddressNativeBalanceResponse, EvmAddressNativeBalanceSnapshotFact,
    EvmAddressNativeBalanceSubject,
};
use mfm_store::v1::{
    AsyncInMemoryRunStore, CommitKey, RetainedArtifactReadProvider, RunEventStore,
};
use serde_json::json;

const ETH_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
const BTC_HASH: &str = "abababababababababababababababababababababababababababababababab";
const EVM_HASH: &str = "0xcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

#[tokio::test]
async fn portfolio_snapshot_completes_from_seeded_platform_holdings_without_live_crawl() {
    let store = AsyncInMemoryRunStore::default();
    seed_dual_mainnet_holdings(&store);

    let fact_index = Arc::new(ProjectionFactIndexProvider::new(store.clone()));
    let services = portfolio_services(store.clone(), fact_index.clone());

    let prepared =
        prepare_portfolio_launch_for_store(&store, &dual_mainnet_portfolio_json(), None).await;
    let report = services
        .launch_prepared_entry_point_run(prepared)
        .await
        .unwrap_or_else(|error| panic!("portfolio launch: {error:?}"));
    let launch = report.run.expect("run body");
    let run_id = launch.run_id.parse::<RunId>().expect("run id");
    if launch.run_mode != mfm_app::RunModeStatus::Completed {
        let stream = store.load_run_stream(&run_id).await.expect("run stream");
        let failures = stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::StateAttemptFailed(payload) => Some(format!(
                    "{} {} {}",
                    payload.node_id, payload.error.code, payload.error.safe_message
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        panic!(
            "report must complete from admitted Platform facts; mode={:?}; failures={failures:?}; fact_index_reads={:?}",
            launch.run_mode,
            fact_index.returned_row_counts()
        );
    }

    let public_output = report
        .public_output
        .expect("public output on completed report");
    let rendered = public_output
        .json
        .expect("public output json body on completed report");

    // Coverage honesty (W1) must survive public render on observations.
    let coverages = collect_observation_coverages(&rendered);
    assert_eq!(
        coverages.len(),
        2,
        "dual-mainnet observations must each carry coverage: {rendered}"
    );
    assert!(
        coverages.iter().all(|c| c == "configured_only"),
        "expected configured_only coverage, got {coverages:?}"
    );

    let pins = find_network_pins(&rendered).expect("network_pins in public output");
    assert_eq!(
        pins.as_array().map(|a| a.len()),
        Some(2),
        "dual-mainnet pins projected from selected anchors: {pins}"
    );
    assert_pin_anchor(&pins, "bitcoin-mainnet", 850_000, BTC_HASH);
    assert_pin_anchor_evm(&pins, "ethereum-mainnet", 21_000_000, EVM_HASH);

    // SelectHoldings must have queried Platform facts.
    let reads_before_replay = fact_index.returned_row_counts();
    assert!(
        !reads_before_replay.is_empty(),
        "SelectHoldings must query fact-index during live drive"
    );
    assert!(
        reads_before_replay.iter().all(|n| *n >= 1),
        "each SelectHoldings query should return at least one row: {reads_before_replay:?}"
    );

    let stream = store.load_run_stream(&run_id).await.expect("run stream");
    let evidences = fact_query_evidences(&store, &stream).await;
    assert!(
        !evidences.is_empty(),
        "SelectHoldings must retain FactQueryEvidence for replay"
    );

    // W7: replay must not re-query live fact-index frontier.
    let replay = services
        .verify_replay_for_run(&run_id)
        .await
        .expect("verify_replay");
    assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(
        fact_index.returned_row_counts(),
        reads_before_replay,
        "SelectHoldings replay must not re-query live fact-index"
    );
}

fn portfolio_services(
    store: AsyncInMemoryRunStore,
    fact_index: Arc<ProjectionFactIndexProvider>,
) -> mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore> {
    let receipt_trust_root = fact_index.receipt_trust_root();
    let artifacts: Arc<dyn RetainedArtifactReadProvider> = Arc::new(store.clone());
    let portfolio_capabilities =
        mfm_adapters_portfolio::PortfolioRunnerCapabilities::new(artifacts, fact_index);
    let mut runners = mfm_runtime::ErasedRunnerRegistry::new();
    mfm_adapters_portfolio::register_portfolio_runners(&mut runners, portfolio_capabilities)
        .expect("portfolio runners");
    let runtime_artifacts = Arc::new(store.clone());
    mfm_app::RunServices::new_with_certification_registry_and_fact_query_trust_root(
        mfm_runtime::SerialTypedScheduler::new(runners, runtime_artifacts),
        store.clone(),
        store,
        mfm_app::production_certification_registry().expect("certification registry"),
        Some(receipt_trust_root),
    )
}

fn seed_dual_mainnet_holdings(store: &AsyncInMemoryRunStore) {
    let btc_subject = BtcAddressBalanceSubject::new(
        "bitcoin-mainnet",
        "main",
        "public-bitcoin-core",
        BTC_ADDRESS,
    )
    .expect("btc subject");
    let btc_response = BtcAddressBalanceResponse::new(
        850_000,
        BTC_HASH,
        100_000,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("btc response");
    let btc_descriptor = BtcAddressBalanceSnapshotFact::descriptor().expect("btc descriptor");

    let evm_subject = EvmAddressNativeBalanceSubject::new("ethereum-mainnet", 1, ETH_ACCOUNT)
        .expect("evm subject");
    let evm_response = EvmAddressNativeBalanceResponse::new(
        21_000_000,
        EVM_HASH,
        "1000000000000000000",
        18,
        CoverageStatus::ConfiguredOnly,
        HoldingSourceStatus::Ok,
    )
    .expect("evm response");
    let evm_descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().expect("evm descriptor");

    seed_platform_holding_facts_for_test(
        store,
        [
            PlatformHoldingFactSeedForTest {
                descriptor: btc_descriptor.clone(),
                input: FactProjectionFixtureInputForTest {
                    run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x11)),
                    source_seq: 1,
                    source_ordinal: 0,
                    source_event_id: EventId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        digest_bytes(0x12),
                    ),
                    node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x13)),
                    attempt_id: AttemptId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        digest_bytes(0x14),
                    ),
                    commit_id: CommitKey::new("btc-holding-seed").expect("commit"),
                    store_commit_order: 17,
                    recorded_at: "2026-07-02T00:00:00Z".to_owned(),
                    observed_at: None,
                    visibility: FactVisibility::indexed_default(FactAudience::Platform),
                    subject: CanonicalValue::object([
                        (
                            "network",
                            CanonicalValue::String(btc_subject.network().to_owned()),
                        ),
                        (
                            "bitcoin_network",
                            CanonicalValue::String(btc_subject.bitcoin_network().to_owned()),
                        ),
                        (
                            "semantic_source_identity",
                            CanonicalValue::String(
                                btc_subject.semantic_source_identity().to_owned(),
                            ),
                        ),
                        (
                            "address",
                            CanonicalValue::String(btc_subject.address().to_owned()),
                        ),
                    ])
                    .expect("btc subject"),
                    response: CanonicalValue::object([
                        (
                            "anchor_height",
                            CanonicalValue::Unsigned(btc_response.anchor_height()),
                        ),
                        (
                            "anchor_hash",
                            CanonicalValue::String(btc_response.anchor_hash().to_owned()),
                        ),
                        (
                            "balance_sats",
                            CanonicalValue::Unsigned(btc_response.balance_sats()),
                        ),
                        (
                            "coverage",
                            CanonicalValue::String(btc_response.coverage().to_owned()),
                        ),
                        (
                            "source_status",
                            CanonicalValue::String(btc_response.source_status().to_owned()),
                        ),
                    ])
                    .expect("btc response"),
                    request: None,
                    response_schema_id: btc_descriptor.response_schema_id().clone(),
                    response_artifact_id: None,
                    producer: holding_producer(0x20),
                },
            },
            PlatformHoldingFactSeedForTest {
                descriptor: evm_descriptor.clone(),
                input: FactProjectionFixtureInputForTest {
                    run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x31)),
                    source_seq: 2,
                    source_ordinal: 0,
                    source_event_id: EventId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        digest_bytes(0x32),
                    ),
                    node_id: NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(0x33)),
                    attempt_id: AttemptId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        digest_bytes(0x34),
                    ),
                    commit_id: CommitKey::new("evm-holding-seed").expect("commit"),
                    store_commit_order: 19,
                    recorded_at: "2026-07-02T00:00:00Z".to_owned(),
                    observed_at: None,
                    visibility: FactVisibility::indexed_default(FactAudience::Platform),
                    subject: CanonicalValue::object([
                        (
                            "network",
                            CanonicalValue::String(evm_subject.network().to_owned()),
                        ),
                        ("chain_id", CanonicalValue::Unsigned(evm_subject.chain_id())),
                        (
                            "account",
                            CanonicalValue::String(evm_subject.account().to_owned()),
                        ),
                    ])
                    .expect("evm subject"),
                    response: CanonicalValue::object([
                        (
                            "block_number",
                            CanonicalValue::Unsigned(evm_response.block_number()),
                        ),
                        (
                            "block_hash",
                            CanonicalValue::String(evm_response.block_hash().to_owned()),
                        ),
                        (
                            "raw_wei",
                            CanonicalValue::String(evm_response.raw_wei().to_owned()),
                        ),
                        (
                            "decimals",
                            CanonicalValue::Unsigned(u64::from(evm_response.decimals())),
                        ),
                        (
                            "coverage",
                            CanonicalValue::String(evm_response.coverage().to_owned()),
                        ),
                        (
                            "source_status",
                            CanonicalValue::String(evm_response.source_status().to_owned()),
                        ),
                    ])
                    .expect("evm response"),
                    request: None,
                    response_schema_id: evm_descriptor.response_schema_id().clone(),
                    response_artifact_id: None,
                    producer: holding_producer(0x40),
                },
            },
        ],
    )
    .expect("seed platform holdings");
}

fn holding_producer(seed: u8) -> FactProducerProvenance {
    FactProducerProvenance::new(
        CapabilityKind::new(
            "mfm.fact",
            "record",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed),
        )
        .expect("cap"),
        CapabilityVersion::new("mfm.fact.record.v1").expect("cap ver"),
        AdapterKind::new(
            "mfm.test",
            "adapter",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed.wrapping_add(1)),
        )
        .expect("adapter"),
        AdapterVersion::new("mfm.test.adapter.v1").expect("adapter ver"),
    )
}

fn dual_mainnet_portfolio_json() -> serde_json::Value {
    json!({
        "portfolio": {
            "portfolio_id": "dual-mainnet",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "chain_id": 1,
                    "metadata": {}
                },
                {
                    "network_id": "bitcoin-mainnet",
                    "family": "bitcoin",
                    "bitcoin_network": "main",
                    "source_identity": "public-bitcoin-core",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_eth",
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "subject": {
                        "kind": "evm_address",
                        "address": ETH_ACCOUNT
                    },
                    "implementation": { "kind": "address_only" },
                    "metadata": {}
                },
                {
                    "wallet_id": "wallet_btc",
                    "network_id": "bitcoin-mainnet",
                    "symbol_ids": ["btc.native.bitcoin-mainnet"],
                    "subject": {
                        "kind": "bitcoin_address",
                        "address": BTC_ADDRESS
                    },
                    "implementation": { "kind": "address_only" },
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "decimals": 18,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "eth.native.ethereum-mainnet",
                            "unit_price_dec": "2.5"
                        }]
                    },
                    "metadata": {}
                },
                {
                    "symbol_id": "btc.native.bitcoin-mainnet",
                    "display_symbol": "BTC",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "bitcoin-mainnet",
                    "decimals": 8,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "btc.native.bitcoin-mainnet",
                            "unit_price_dec": "1"
                        }]
                    },
                    "metadata": {}
                }
            ],
            "metadata": {}
        }
    })
}

fn collect_observation_coverages(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    walk_coverages(value, &mut out);
    out
}

fn walk_coverages(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let (Some(coverage), Some(_)) = (map.get("coverage"), map.get("wallet_id")) {
                if let Some(s) = coverage.as_str() {
                    out.push(s.to_owned());
                }
            }
            for child in map.values() {
                walk_coverages(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_coverages(item, out);
            }
        }
        _ => {}
    }
}

fn find_network_pins(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(pins) = map.get("network_pins") {
                return Some(pins.clone());
            }
            for child in map.values() {
                if let Some(found) = find_network_pins(child) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(found) = find_network_pins(item) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn assert_pin_anchor(pins: &serde_json::Value, network_id: &str, height: u64, hash: &str) {
    let pin = pins
        .as_array()
        .into_iter()
        .flatten()
        .find(|pin| pin.get("network_id").and_then(|v| v.as_str()) == Some(network_id))
        .unwrap_or_else(|| panic!("missing pin for {network_id}: {pins}"));
    let anchor = pin.get("anchor").expect("anchor");
    // Bitcoin pin shape: { "family": "bitcoin", "height": ..., "block_hash": ... } or nested.
    let height_val = anchor
        .get("height")
        .or_else(|| anchor.pointer("/Bitcoin/height"))
        .and_then(|v| v.as_u64());
    let hash_val = anchor
        .get("block_hash")
        .or_else(|| anchor.pointer("/Bitcoin/block_hash"))
        .and_then(|v| v.as_str());
    assert_eq!(height_val, Some(height), "btc height pin: {pin}");
    assert_eq!(hash_val, Some(hash), "btc hash pin: {pin}");
}

fn assert_pin_anchor_evm(pins: &serde_json::Value, network_id: &str, block: u64, hash: &str) {
    let pin = pins
        .as_array()
        .into_iter()
        .flatten()
        .find(|pin| pin.get("network_id").and_then(|v| v.as_str()) == Some(network_id))
        .unwrap_or_else(|| panic!("missing pin for {network_id}: {pins}"));
    let anchor = pin.get("anchor").expect("anchor");
    let block_val = anchor
        .get("block_number")
        .or_else(|| anchor.pointer("/Evm/block_number"))
        .and_then(|v| v.as_u64());
    let hash_val = anchor
        .get("block_hash")
        .or_else(|| anchor.pointer("/Evm/block_hash"))
        .and_then(|v| v.as_str());
    assert_eq!(block_val, Some(block), "evm block pin: {pin}");
    assert_eq!(hash_val, Some(hash), "evm hash pin: {pin}");
}

fn digest_bytes(seed: u8) -> DigestBytes {
    DigestBytes::from_array([seed; 32])
}
