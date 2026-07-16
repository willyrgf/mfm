use std::future;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::{Json, Router};
use mfm_fact_capabilities::{
    FactIndexReadBatchFuture, FactIndexReadProvider, FactIndexReadRequest,
};
use mfm_facts::{FactQueryResult, FactQueryResultRow, InternalFactRef};
use mfm_integration_tests::test_support::{
    fact_query_evidences, write_collectors_runtime_config_for_test,
};
use mfm_op_portfolio_snapshot::portfolio_snapshot_program_draft;
use mfm_portfolio_model::portfolio::{PortfolioConfig, ValidatedPortfolioConfig};
use mfm_store::v1::{self as store, RunEventStore as _};
use serde_json::{json, Value};
use tokio::sync::oneshot;

const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";
const EVM_HASH_A: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const EVM_HASH_B: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BTC_HASH: &str = "abababababababababababababababababababababababababababababababab";

/// Exercises the exact production snapshot root for every supported collection family. Replay
/// runs from retained evidence only: the test removes each runtime file before it asks the
/// read-only service to verify the completed run.
#[tokio::test]
async fn snapshot_root_executes_btc_native_erc20_and_mixed_with_evidence_only_replay() {
    for (label, config) in [
        ("btc", snapshot_config(SnapshotDemand::Bitcoin)),
        ("native", snapshot_config(SnapshotDemand::EvmNative)),
        ("erc20", snapshot_config(SnapshotDemand::Erc20)),
        ("mixed", snapshot_config(SnapshotDemand::Mixed)),
    ] {
        let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
        let runtime_dir = tempfile::tempdir().expect("runtime config directory");
        let runtime_path =
            write_collectors_runtime_config_for_test(runtime_dir.path(), &server.url);
        let store = store::AsyncInMemoryRunStore::default();
        let services = snapshot_services(
            &store,
            Arc::new(mfm_app::ProjectionFactIndexProvider::new(store.clone())),
            Some(&runtime_path),
        );

        let run_id = launch_snapshot_completed(&services, &config, label).await;
        let source_calls_before_replay = server.calls();
        drop(services);
        std::fs::remove_file(&runtime_path).expect("remove live runtime config after admission");

        // This service intentionally has no runner registry, runtime config, provider, or catalog
        // input. It can only verify the certified retained evidence committed by the exact root.
        let replay_services = mfm_app::make_run_read_services(
            store.clone(),
            store.clone(),
            mfm_app::production_certification_registry().expect("snapshot certification registry"),
        );
        let replay = replay_services
            .verify_replay_for_run(&run_id)
            .await
            .unwrap_or_else(|error| panic!("{label} snapshot replay: {error:?}"));
        assert_eq!(
            replay.run_mode,
            mfm_app::RunModeStatus::Completed,
            "{label}"
        );
        assert_eq!(
            server.calls(),
            source_calls_before_replay,
            "{label} replay must not re-open a live provider"
        );
    }
}

/// A complete mixed snapshot replays the family receipts, receipt-pinned queries, hydration,
/// ordering, and report projection from retained evidence alone. Altering that query evidence is
/// rejected before it can influence the reconstructed report.
#[tokio::test]
async fn snapshot_root_replay_rejects_tampered_receipt_pinned_query_evidence() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_collectors_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let services = snapshot_services(
        &store,
        Arc::new(mfm_app::ProjectionFactIndexProvider::new(store.clone())),
        Some(&runtime_path),
    );
    let run_id = launch_snapshot_completed(
        &services,
        &snapshot_config(SnapshotDemand::Mixed),
        "tampered-receipt-pinned-query-evidence",
    )
    .await;
    let source_calls_before_replay = server.calls();
    drop(services);
    std::fs::remove_file(&runtime_path).expect("remove runtime config before replay");

    let replay_services = mfm_app::make_run_read_services(
        store.clone(),
        store.clone(),
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    );
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("complete receipt-pinned snapshot replay");
    assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Completed);

    let tampered_replay_services = mfm_app::make_run_read_services(
        store.clone(),
        TamperedSnapshotFactQueryEvidenceProvider::new(store),
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    );
    let error = tampered_replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect_err("tampered receipt-pinned query evidence must fail replay");
    assert_eq!(error.code, "ArtifactEvidenceMismatch");
    assert_eq!(
        server.calls(),
        source_calls_before_replay,
        "both normal and tampered replay must remain evidence-only"
    );
}

/// Zero balances remain successful observations in the complete graph, rather than being
/// mistaken for absent facts or failed collection.
#[tokio::test]
async fn snapshot_root_preserves_zero_btc_native_and_erc20_values() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig {
        btc_amount_json: "0",
        evm_native_wei: 0,
        erc20_raw_units: 0,
        ..SnapshotRpcConfig::default()
    })
    .await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_collectors_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let services = snapshot_services(
        &store,
        Arc::new(mfm_app::ProjectionFactIndexProvider::new(store.clone())),
        Some(&runtime_path),
    );
    let request =
        prepare_snapshot_request(&services, snapshot_config(SnapshotDemand::Mixed), "zero").await;

    let report = services
        .launch_run_and_render(request)
        .await
        .expect("zero-valued snapshot launch and render");
    assert_eq!(
        report.run.expect("completed zero snapshot").run_mode,
        mfm_app::RunModeStatus::Completed
    );
    let rendered = report
        .public_output
        .expect("zero snapshot public output")
        .json
        .expect("zero snapshot JSON")
        .to_string();
    let rendered_value: Value = serde_json::from_str(&rendered).expect("public output JSON value");
    assert_eq!(
        rendered_value["snapshot"]["schema_version"],
        json!(1),
        "the public snapshot contract starts at version 1"
    );
    assert_eq!(
        rendered_value["report"]["schema_version"],
        json!(1),
        "the public report contract starts at version 1"
    );
    assert_eq!(
        rendered.matches("\"raw_dec\":\"0\"").count(),
        3,
        "the public snapshot retains one zero quantity for every logical demand"
    );
}

/// A conflicting historical fact cannot replace the current receipt's fact identity, even when a
/// malicious store-facing provider returns the old candidate first.
#[tokio::test]
async fn snapshot_root_filters_same_anchor_conflicts_before_candidate_ordering() {
    let store = store::AsyncInMemoryRunStore::default();
    let first_server = start_snapshot_rpc_mock(SnapshotRpcConfig {
        evm_native_wei: 1,
        ..SnapshotRpcConfig::default()
    })
    .await;
    let first_dir = tempfile::tempdir().expect("first runtime config directory");
    let first_path = write_collectors_runtime_config_for_test(first_dir.path(), &first_server.url);
    let first_services = snapshot_services(
        &store,
        Arc::new(mfm_app::ProjectionFactIndexProvider::new(store.clone())),
        Some(&first_path),
    );
    launch_snapshot_completed(
        &first_services,
        &snapshot_config(SnapshotDemand::EvmNative),
        "conflict-source",
    )
    .await;
    drop(first_services);

    let second_server = start_snapshot_rpc_mock(SnapshotRpcConfig {
        evm_native_wei: 2,
        ..SnapshotRpcConfig::default()
    })
    .await;
    let second_dir = tempfile::tempdir().expect("second runtime config directory");
    let second_path =
        write_collectors_runtime_config_for_test(second_dir.path(), &second_server.url);
    let services = snapshot_services(
        &store,
        Arc::new(SnapshotProjectionFactIndex::new(
            store.clone(),
            SnapshotQueryMode::Reverse,
        )),
        Some(&second_path),
    );
    let run_id = launch_snapshot_completed(
        &services,
        &snapshot_config(SnapshotDemand::EvmNative),
        "conflict-target",
    )
    .await;

    let stream = store
        .load_run_stream(&run_id)
        .await
        .expect("conflict run stream");
    let evidences = fact_query_evidences(&store, &stream).await;
    assert_eq!(evidences.len(), 1);
    assert_eq!(
        evidences[0].selection().selected_indices(),
        &[1],
        "the current receipt identity must win after the deliberately reversed candidate order"
    );
}

/// The full root fails closed when selection cannot prove N + 1 exhaustion or when a provider
/// returns a fact from a different receipt anchor.
#[tokio::test]
async fn snapshot_root_rejects_saturated_queries_and_receipt_mismatches() {
    let saturated_server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let saturated_dir = tempfile::tempdir().expect("saturation runtime config directory");
    let saturated_path =
        write_collectors_runtime_config_for_test(saturated_dir.path(), &saturated_server.url);
    let saturated_store = store::AsyncInMemoryRunStore::default();
    let saturated_services = snapshot_services(
        &saturated_store,
        Arc::new(SnapshotProjectionFactIndex::new(
            saturated_store.clone(),
            SnapshotQueryMode::Saturate,
        )),
        Some(&saturated_path),
    );
    let saturated = launch_snapshot(
        &saturated_services,
        snapshot_config(SnapshotDemand::EvmNative),
        "saturated",
    )
    .await;
    assert_eq!(
        saturated.run_mode,
        mfm_app::RunModeStatus::FailedWithoutAcdcClaim
    );
    assert!(
        saturated
            .attempt_dispositions
            .iter()
            .any(|attempt| attempt.error_code.as_deref() == Some("candidate_bound_exhausted")),
        "the root must surface receipt-query saturation as a closed selection failure: {saturated:?}"
    );

    let mismatch_store = store::AsyncInMemoryRunStore::default();
    let old_server = start_snapshot_rpc_mock(SnapshotRpcConfig {
        evm_hash: EVM_HASH_A,
        ..SnapshotRpcConfig::default()
    })
    .await;
    let old_dir = tempfile::tempdir().expect("old anchor runtime config directory");
    let old_path = write_collectors_runtime_config_for_test(old_dir.path(), &old_server.url);
    let old_services = snapshot_services(
        &mismatch_store,
        Arc::new(mfm_app::ProjectionFactIndexProvider::new(
            mismatch_store.clone(),
        )),
        Some(&old_path),
    );
    launch_snapshot_completed(
        &old_services,
        &snapshot_config(SnapshotDemand::EvmNative),
        "old-anchor",
    )
    .await;
    let old_ref = evm_native_fact_ref(&mismatch_store);
    drop(old_services);

    let current_server = start_snapshot_rpc_mock(SnapshotRpcConfig {
        evm_hash: EVM_HASH_B,
        ..SnapshotRpcConfig::default()
    })
    .await;
    let current_dir = tempfile::tempdir().expect("current anchor runtime config directory");
    let current_path =
        write_collectors_runtime_config_for_test(current_dir.path(), &current_server.url);
    let mismatch_services = snapshot_services(
        &mismatch_store,
        Arc::new(SnapshotProjectionFactIndex::new(
            mismatch_store.clone(),
            SnapshotQueryMode::Replace(Box::new(old_ref)),
        )),
        Some(&current_path),
    );
    let mismatch = launch_snapshot(
        &mismatch_services,
        snapshot_config(SnapshotDemand::EvmNative),
        "receipt-mismatch",
    )
    .await;
    assert_eq!(
        mismatch.run_mode,
        mfm_app::RunModeStatus::FailedWithoutAcdcClaim
    );
    assert!(
        mismatch
            .attempt_dispositions
            .iter()
            .any(|attempt| attempt.error_code.as_deref() == Some("receipt_mismatch")),
        "the report half must reject a candidate whose anchor differs from the current receipt: {mismatch:?}"
    );
}

/// An interrupted exact root resumes after collection with no runtime configuration or live
/// provider. The final replay is likewise read-only and uses the same production draft helper.
#[tokio::test]
async fn snapshot_root_resumes_and_replays_after_live_inputs_disappear() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_collectors_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let (entered_tx, entered_rx) = oneshot::channel();
    let initial_services = snapshot_services(
        &store,
        Arc::new(BlockingSnapshotFactIndex::new(store.clone(), entered_tx)),
        Some(&runtime_path),
    );
    let request = prepare_snapshot_request(
        &initial_services,
        snapshot_config(SnapshotDemand::Mixed),
        "resume",
    )
    .await;
    let run_id = request.run_id.clone();
    let launch_services = initial_services.clone();
    let launch = tokio::spawn(async move { launch_services.launch_run(request).await });

    entered_rx
        .await
        .expect("selection started after complete live collection");
    let stream = store
        .load_run_stream(&run_id)
        .await
        .expect("interrupted stream");
    assert!(stream.iter().any(|event| matches!(
        event.payload(),
        mfm_events::v1::KernelEventPayload::StateAttemptStarted(payload)
            if payload.state_kind.as_str().contains("select_holdings")
    )));
    let source_calls_before_resume = server.calls();
    launch.abort();
    let _ = launch.await;
    assert!(store
        .expire_execution_claim_for_test(&run_id)
        .expect("expire interrupted execution claim"));
    drop(initial_services);
    std::fs::remove_file(&runtime_path).expect("remove runtime config before resume");

    let resumed_services = snapshot_services(
        &store,
        Arc::new(mfm_app::ProjectionFactIndexProvider::new(store.clone())),
        None,
    );
    let resumed = resumed_services
        .resume_stored_run(&run_id)
        .await
        .expect("resume receipt-pinned report without live runtime access");
    assert_eq!(resumed.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(
        server.calls(),
        source_calls_before_resume,
        "resume after collection must not call a live source"
    );
    drop(resumed_services);

    let replay_services = mfm_app::make_run_read_services(
        store.clone(),
        store.clone(),
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    );
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only snapshot replay");
    assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(server.calls(), source_calls_before_resume);
}

/// A live source failure never produces a successful complete snapshot output.
#[tokio::test]
async fn snapshot_root_fails_closed_when_a_collection_source_fails() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig {
        failing_method: Some("getblockchaininfo"),
        ..SnapshotRpcConfig::default()
    })
    .await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_collectors_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let services = snapshot_services(
        &store,
        Arc::new(mfm_app::ProjectionFactIndexProvider::new(store.clone())),
        Some(&runtime_path),
    );
    let response = launch_snapshot(
        &services,
        snapshot_config(SnapshotDemand::Bitcoin),
        "source-failure",
    )
    .await;
    assert_eq!(
        response.run_mode,
        mfm_app::RunModeStatus::FailedWithoutAcdcClaim
    );
    assert!(
        response
            .attempt_dispositions
            .iter()
            .any(|attempt| attempt.disposition == "failed"),
        "source failure must retain a failed state attempt: {response:?}"
    );
}

#[derive(Clone, Copy)]
enum SnapshotDemand {
    Bitcoin,
    EvmNative,
    Erc20,
    Mixed,
}

fn snapshot_config(demand: SnapshotDemand) -> PortfolioConfig {
    let bitcoin = matches!(demand, SnapshotDemand::Bitcoin | SnapshotDemand::Mixed);
    let evm_native = matches!(demand, SnapshotDemand::EvmNative | SnapshotDemand::Mixed);
    let erc20 = matches!(demand, SnapshotDemand::Erc20 | SnapshotDemand::Mixed);
    let mut networks = Vec::new();
    let mut wallets = Vec::new();
    let mut symbols = Vec::new();

    if bitcoin {
        networks.push(json!({
            "network_id": "bitcoin-mainnet",
            "family": "bitcoin",
            "bitcoin_network": "main",
            "source_identity": "public-bitcoin-core",
            "metadata": {}
        }));
        wallets.push(json!({
            "wallet_id": "wallet_btc",
            "network_id": "bitcoin-mainnet",
            "symbol_ids": ["btc.native.bitcoin-mainnet"],
            "subject": {"kind": "bitcoin_address", "address": BTC_ADDRESS},
            "implementation": {"kind": "address_only"},
            "metadata": {}
        }));
        symbols.push(symbol(
            "btc.native.bitcoin-mainnet",
            "bitcoin-mainnet",
            json!({"kind": "native"}),
        ));
    }
    if evm_native || erc20 {
        networks.push(json!({
            "network_id": "ethereum-mainnet",
            "family": "evm",
            "chain_id": 1,
            "native_decimals": 18,
            "metadata": {}
        }));
        let mut symbol_ids = Vec::new();
        if evm_native {
            symbol_ids.push("eth.native.ethereum-mainnet");
            symbols.push(symbol(
                "eth.native.ethereum-mainnet",
                "ethereum-mainnet",
                json!({"kind": "native"}),
            ));
        }
        if erc20 {
            symbol_ids.push("usdc.ethereum-mainnet");
            symbols.push(symbol(
                "usdc.ethereum-mainnet",
                "ethereum-mainnet",
                json!({"kind": "erc20", "contract_address": TOKEN}),
            ));
        }
        wallets.push(json!({
            "wallet_id": "wallet_eth",
            "network_id": "ethereum-mainnet",
            "symbol_ids": symbol_ids,
            "subject": {"kind": "evm_address", "address": EVM_ACCOUNT},
            "implementation": {"kind": "address_only"},
            "metadata": {}
        }));
    }

    ValidatedPortfolioConfig::new(
        serde_json::from_value(json!({
            "portfolio_id": "snapshot-root-integration",
            "quote_codes": ["USD"],
            "networks": networks,
            "wallets": wallets,
            "symbol_configs": symbols,
            "metadata": {}
        }))
        .expect("snapshot portfolio fixture"),
    )
    .expect("normalize snapshot portfolio fixture")
    .into_config()
}

fn symbol(symbol_id: &str, network_id: &str, source: Value) -> Value {
    json!({
        "symbol_id": symbol_id,
        "display_symbol": symbol_id,
        "network_id": network_id,
        "source": source,
        "valuation": {
            "quotes": [{
                "quote": "USD",
                "priced_symbol_id": symbol_id,
                "unit_price_dec": "1.00"
            }]
        },
        "metadata": {}
    })
}

fn snapshot_services(
    store: &store::AsyncInMemoryRunStore,
    fact_index: Arc<dyn FactIndexReadProvider>,
    runtime_config_path: Option<&Path>,
) -> mfm_app::RunServices<store::AsyncInMemoryRunStore, store::AsyncInMemoryRunStore> {
    let runners = mfm_app::production_runner_registry(
        Arc::new(store.clone()),
        fact_index,
        runtime_config_path,
    )
    .expect("production snapshot runners");
    mfm_app::make_run_services(
        runners,
        store.clone(),
        store.clone(),
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    )
}

async fn prepare_snapshot_request(
    services: &mfm_app::RunServices<store::AsyncInMemoryRunStore, store::AsyncInMemoryRunStore>,
    config: PortfolioConfig,
    invocation_key: &str,
) -> mfm_app::RunLaunchRequest {
    let draft = portfolio_snapshot_program_draft(config).expect("complete snapshot draft");
    mfm_app::prepare_typed_program_run_launch_for_test(
        draft,
        Default::default(),
        services.certification_registry(),
        services.load_store_scope_id().await.expect("store scope"),
        Some(mfm_app::InvocationKey::new(invocation_key).expect("invocation key")),
    )
    .expect("complete snapshot launch request")
}

async fn launch_snapshot_completed(
    services: &mfm_app::RunServices<store::AsyncInMemoryRunStore, store::AsyncInMemoryRunStore>,
    config: &PortfolioConfig,
    invocation_key: &str,
) -> mfm_ids::RunId {
    let response = launch_snapshot(services, config.clone(), invocation_key).await;
    assert_eq!(
        response.run_mode,
        mfm_app::RunModeStatus::Completed,
        "{invocation_key} complete snapshot run: {response:?}"
    );
    response.run_id.parse().expect("snapshot run id")
}

async fn launch_snapshot(
    services: &mfm_app::RunServices<store::AsyncInMemoryRunStore, store::AsyncInMemoryRunStore>,
    config: PortfolioConfig,
    invocation_key: &str,
) -> mfm_app::RunResponse {
    let request = prepare_snapshot_request(services, config, invocation_key).await;
    let outcome = services
        .launch_run(request)
        .await
        .unwrap_or_else(|error| panic!("{invocation_key} snapshot launch: {error:?}"));
    outcome
        .into_response_parts()
        .1
        .expect("snapshot launch response")
}

#[derive(Clone)]
enum SnapshotQueryMode {
    Reverse,
    Saturate,
    Replace(Box<InternalFactRef>),
}

struct SnapshotProjectionFactIndex {
    store: store::AsyncInMemoryRunStore,
    mode: SnapshotQueryMode,
}

impl SnapshotProjectionFactIndex {
    fn new(store: store::AsyncInMemoryRunStore, mode: SnapshotQueryMode) -> Self {
        Self { store, mode }
    }
}

impl FactIndexReadProvider for SnapshotProjectionFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.integration.snapshot.projection-fact-index.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            let projection = self.store.projection_snapshot().map_err(|error| {
                mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
            })?;
            let mut results = Vec::with_capacity(requests.len());
            for request in requests {
                let mut rows = store::test_support::execute_fact_query_projection_for_test(
                    &projection,
                    request.plan(),
                )
                .map_err(|error| {
                    mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                })?;
                match &self.mode {
                    SnapshotQueryMode::Reverse => rows.reverse(),
                    SnapshotQueryMode::Saturate => {
                        let row = rows.first().cloned().ok_or_else(|| {
                            mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(
                                "snapshot saturation fixture had no collected fact",
                            )
                        })?;
                        rows = (0..11).map(|_| row.clone()).collect();
                    }
                    SnapshotQueryMode::Replace(replacement) => {
                        let fields = rows.first().map(|row| row.returned_fields().to_vec()).ok_or_else(
                            || {
                                mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(
                                    "snapshot receipt mismatch fixture had no collected fact",
                                )
                            },
                        )?;
                        rows = vec![FactQueryResultRow::new((**replacement).clone(), fields)];
                    }
                }
                let receipt = store::test_support::fact_query_receipt_for_projection_for_test(
                    request.plan(),
                    &projection,
                    &rows,
                );
                results.push(FactQueryResult::new(rows, receipt).map_err(|error| {
                    mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
                })?);
            }
            Ok(results)
        })
    }
}

struct BlockingSnapshotFactIndex {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    store: store::AsyncInMemoryRunStore,
}

impl BlockingSnapshotFactIndex {
    fn new(store: store::AsyncInMemoryRunStore, entered: oneshot::Sender<()>) -> Self {
        Self {
            entered: Mutex::new(Some(entered)),
            store,
        }
    }
}

impl FactIndexReadProvider for BlockingSnapshotFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.integration.snapshot.blocking-fact-index.v1"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        _requests: &'a [FactIndexReadRequest],
    ) -> FactIndexReadBatchFuture<'a> {
        Box::pin(async move {
            let _ = self.store.projection_snapshot().map_err(|error| {
                mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(error)
            })?;
            if let Some(entered) = self
                .entered
                .lock()
                .expect("blocking fact-index state")
                .take()
            {
                let _ = entered.send(());
            }
            future::pending().await
        })
    }
}

#[derive(Clone)]
struct TamperedSnapshotFactQueryEvidenceProvider {
    inner: store::AsyncInMemoryRunStore,
}

impl TamperedSnapshotFactQueryEvidenceProvider {
    fn new(inner: store::AsyncInMemoryRunStore) -> Self {
        Self { inner }
    }
}

impl store::RetainedArtifactReadProvider for TamperedSnapshotFactQueryEvidenceProvider {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let artifact = self.inner.read_retained_artifact(requirement).await?;
            if artifact.evidence().artifact_role != mfm_events::v1::ArtifactRole::FactQueryEvidence
            {
                return Ok(artifact);
            }

            let mut tampered_bytes = artifact.bytes().to_vec();
            tampered_bytes.push(b'\n');
            store::VerifiedRunArtifactBytes::new(
                tampered_bytes,
                artifact.evidence().clone(),
                requirement,
            )
        })
    }
}

fn evm_native_fact_ref(store: &store::AsyncInMemoryRunStore) -> InternalFactRef {
    store
        .projection_snapshot()
        .expect("fact projection")
        .fact_index_entries()
        .find(|(_claim_id, entry)| {
            entry.fact_kind.as_str() == "evm.address_native_balance_snapshot"
        })
        .expect("recorded EVM native fact")
        .1
        .internal_ref()
        .expect("recorded EVM native internal ref")
}

#[derive(Clone)]
struct SnapshotRpcConfig {
    btc_amount_json: &'static str,
    evm_native_wei: u128,
    erc20_raw_units: u128,
    erc20_decimals: u8,
    evm_hash: &'static str,
    failing_method: Option<&'static str>,
}

impl Default for SnapshotRpcConfig {
    fn default() -> Self {
        Self {
            btc_amount_json: "0.001",
            evm_native_wei: 1_000_000_000_000_000_000,
            erc20_raw_units: 1_000_000,
            erc20_decimals: 6,
            evm_hash: EVM_HASH_A,
            failing_method: None,
        }
    }
}

#[derive(Clone)]
struct SnapshotRpcState {
    config: SnapshotRpcConfig,
    calls: Arc<AtomicUsize>,
}

struct SnapshotRpcServer {
    url: String,
    calls: Arc<AtomicUsize>,
}

impl SnapshotRpcServer {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

async fn start_snapshot_rpc_mock(config: SnapshotRpcConfig) -> SnapshotRpcServer {
    let calls = Arc::new(AtomicUsize::new(0));
    let state = SnapshotRpcState {
        config,
        calls: Arc::clone(&calls),
    };
    let app = Router::new()
        .route("/", axum::routing::post(snapshot_rpc_handler))
        .with_state(state);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind snapshot RPC mock");
    let addr = listener.local_addr().expect("snapshot RPC mock address");
    listener
        .set_nonblocking(true)
        .expect("set snapshot RPC listener nonblocking");
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("snapshot RPC mock runtime");
        runtime.block_on(async move {
            let listener =
                tokio::net::TcpListener::from_std(listener).expect("tokio snapshot RPC listener");
            axum::serve(listener, app)
                .await
                .expect("snapshot RPC mock serve");
        });
    });
    SnapshotRpcServer {
        url: format!("http://{addr}"),
        calls,
    }
}

async fn snapshot_rpc_handler(
    State(state): State<SnapshotRpcState>,
    Json(request): Json<Value>,
) -> Json<Value> {
    state.calls.fetch_add(1, Ordering::SeqCst);
    let id = request.get("id").cloned().unwrap_or_else(|| json!(1));
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .expect("snapshot RPC method");
    if state.config.failing_method == Some(method) {
        return Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32000, "message": "configured snapshot source failure"}
        }));
    }
    let result = match method {
        "eth_chainId" => json!("0x1"),
        "web3_clientVersion" => json!("mfm-snapshot-test-rpc"),
        "eth_getBlockByNumber" | "eth_getBlockByHash" => json!({
            "number": "0x1406f40",
            "hash": state.config.evm_hash,
        }),
        "eth_getBalance" => json!(format!("0x{:x}", state.config.evm_native_wei)),
        "eth_call" => {
            let calldata = request
                .get("params")
                .and_then(Value::as_array)
                .and_then(|params| params.first())
                .and_then(|call| call.get("data"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let value = if calldata == "0x313ce567" {
                u128::from(state.config.erc20_decimals)
            } else {
                state.config.erc20_raw_units
            };
            json!(format!("0x{value:064x}"))
        }
        "getblockchaininfo" => json!({
            "blocks": 850_100u64,
            "bestblockhash": BTC_HASH,
            "chain": "main",
            "initialblockdownload": false,
        }),
        "getblockhash" => json!(BTC_HASH),
        "getblockheader" => json!({
            "hash": BTC_HASH,
            "height": 850_100u64,
            "time": 1_720_000_000u64,
        }),
        "scantxoutset" => json!({
            "success": true,
            "height": 850_100u64,
            "bestblock": BTC_HASH,
            "total_amount": serde_json::from_str::<Value>(state.config.btc_amount_json)
                .expect("valid test Bitcoin amount"),
        }),
        other => {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("unsupported test method {other}")}
            }));
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}
