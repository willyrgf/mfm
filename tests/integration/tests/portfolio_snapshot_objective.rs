use std::future;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::{Json, Router};
use mfm_integration_tests::test_support::write_portfolio_runtime_config_for_test;
use mfm_portfolio::portfolio_snapshot_program_draft;
use mfm_portfolio::{PortfolioConfig, ValidatedPortfolioConfig};
use mfm_store::v1::{self as store, RunEventStore as _};
use serde_json::{json, Value};
use tokio::sync::oneshot;

const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const TOKEN: &str = "0x0000000000000000000000000000000000000001";
const EVM_HASH_A: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BTC_HASH: &str = "abababababababababababababababababababababababababababababababab";

/// Exercises the exact production snapshot root for every supported collection family. Replay
/// runs from retained evidence only: the test removes each runtime file before it asks the
/// read-only service to verify the completed run.
#[tokio::test]
async fn snapshot_root_executes_btc_native_erc20_and_mixed_with_evidence_only_replay() {
    for (label, config, expected_evm_facts) in [
        ("btc", snapshot_config(SnapshotDemand::Bitcoin), 0),
        ("native", snapshot_config(SnapshotDemand::EvmNative), 1),
        ("erc20", snapshot_config(SnapshotDemand::Erc20), 1),
        ("mixed", snapshot_config(SnapshotDemand::Mixed), 2),
    ] {
        let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
        let runtime_dir = tempfile::tempdir().expect("runtime config directory");
        let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
        let store = store::AsyncInMemoryRunStore::default();
        let services = snapshot_services(Arc::new(store.clone()), Some(&runtime_path)).await;

        let run_id = launch_snapshot_completed(&services, &config, label).await;
        assert_atomic_evm_fact_publication(&store, &run_id, expected_evm_facts).await;
        let source_calls_before_replay = server.calls();
        drop(services);
        std::fs::remove_file(&runtime_path).expect("remove live runtime config after admission");

        // This service intentionally has no runner registry, runtime config, provider, or current-config
        // input. It can only verify the certified retained evidence committed by the exact root.
        let replay_services = mfm_app::make_run_read_services(
            Arc::new(store.clone()),
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

/// Repeated collection proves that receipt authority is content based: eleven byte-identical
/// publications remain selectable with one retained row, while a later different-content receipt
/// filters stale history inside the provider before limiting.
#[tokio::test]
async fn snapshot_root_selects_only_the_exact_evm_receipt_content_from_store_history() {
    let first_server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path =
        write_portfolio_runtime_config_for_test(runtime_dir.path(), &first_server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let first_store = Arc::new(SnapshotStore::recording(store.clone()));
    let services = snapshot_services(first_store.clone(), Some(&runtime_path)).await;
    let config = snapshot_config(SnapshotDemand::EvmNative);

    launch_snapshot_completed(&services, &config, "exact-content-first").await;
    for occurrence in 2..=11 {
        launch_snapshot_completed(
            &services,
            &config,
            &format!("exact-content-identical-{occurrence}"),
        )
        .await;
    }
    assert_eq!(
        first_store.returned_row_counts(),
        vec![1; 11],
        "exact content identity must narrow every query before its one-row limit"
    );
    drop(services);

    let changed_raw_units = 2_000_000_000_000_000_000_u128;
    let changed_server = start_snapshot_rpc_mock(SnapshotRpcConfig {
        evm_native_wei: changed_raw_units,
        ..SnapshotRpcConfig::default()
    })
    .await;
    write_portfolio_runtime_config_for_test(runtime_dir.path(), &changed_server.url);
    let changed_store = Arc::new(SnapshotStore::recording(store.clone()));
    let changed_services = snapshot_services(changed_store.clone(), Some(&runtime_path)).await;
    let request =
        prepare_snapshot_request(&changed_services, config, "exact-content-different").await;
    let report = changed_services
        .launch_run_and_render(request)
        .await
        .expect("different-content snapshot launch and render");
    assert_eq!(
        report.run.as_ref().expect("different-content run").run_mode,
        mfm_app::RunModeStatus::Completed
    );
    let rendered = report
        .public_output
        .expect("different-content public output")
        .json
        .expect("different-content public JSON")
        .to_string();
    let rendered: Value = serde_json::from_str(&rendered).expect("public output JSON value");
    assert_eq!(
        rendered["snapshot"]["wallets"][0]["observations"][0]["quantity"]["raw_dec"],
        json!(changed_raw_units.to_string()),
        "selection must discard older different-content facts despite their matching subject"
    );
    assert_eq!(
        changed_store.returned_row_counts(),
        vec![1],
        "the exact receipt filter must run inside the provider before limiting"
    );
}

/// A receipt cannot authorize a report when its exact fact is absent from the read result, even
/// though the producing external read committed that fact to the store immediately beforehand.
#[tokio::test]
async fn snapshot_root_fails_when_the_receipt_authorized_evm_fact_is_missing() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let query_store = Arc::new(SnapshotStore::adversarial(
        store.clone(),
        FactQueryMutation::OmitRows,
    ));
    let services = snapshot_services(query_store.clone(), Some(&runtime_path)).await;
    let response = launch_snapshot(
        &services,
        snapshot_config(SnapshotDemand::EvmNative),
        "missing-receipt-fact",
    )
    .await;

    assert_eq!(
        response.run_mode,
        mfm_app::RunModeStatus::FailedWithoutAcdcClaim
    );
    assert_eq!(query_store.batch_widths(), vec![1]);
    let run_id = response.run_id.parse().expect("failed snapshot run id");
    assert_atomic_evm_fact_publication(&store, &run_id, 1).await;
}

/// Every family query must be evaluated over one shared snapshot frontier before any candidate is
/// hydrated or reduced.
#[tokio::test]
async fn snapshot_root_rejects_mixed_fact_read_frontiers() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let query_store = Arc::new(SnapshotStore::adversarial(
        store.clone(),
        FactQueryMutation::SplitFrontier,
    ));
    let services = snapshot_services(query_store.clone(), Some(&runtime_path)).await;
    let response = launch_snapshot(
        &services,
        snapshot_config(SnapshotDemand::Mixed),
        "mixed-read-frontiers",
    )
    .await;

    assert_eq!(
        response.run_mode,
        mfm_app::RunModeStatus::FailedWithoutAcdcClaim
    );
    assert_eq!(
        query_store.batch_widths(),
        vec![3],
        "Bitcoin and both EVM holding reads must be issued as one batch"
    );
}

/// A temporary fact-query store outage blocks the read attempt for resume and never records a terminal
/// state failure after the collectors have committed their facts.
#[tokio::test]
async fn snapshot_root_blocks_when_the_fact_query_store_is_unavailable() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let query_store = Arc::new(SnapshotStore::adversarial(
        store.clone(),
        FactQueryMutation::StoreFailure,
    ));
    let services = snapshot_services(query_store.clone(), Some(&runtime_path)).await;
    let response = launch_snapshot(
        &services,
        snapshot_config(SnapshotDemand::EvmNative),
        "fact-query-store-unavailable",
    )
    .await;

    assert_eq!(response.run_mode, mfm_app::RunModeStatus::Forward);
    assert_eq!(response.scheduler_status, "blocked");
    assert!(!response
        .attempt_dispositions
        .iter()
        .any(|attempt| attempt.disposition == "failed"));
    assert_eq!(query_store.batch_widths(), vec![1]);
    let run_id = response.run_id.parse().expect("blocked snapshot run id");
    let stream = store
        .load_run_stream(&run_id)
        .await
        .expect("blocked snapshot stream");
    assert!(!stream.iter().any(|event| matches!(
        event.payload(),
        mfm_events::v1::KernelEventPayload::StateAttemptFailed(_)
    )));
}

async fn assert_atomic_evm_fact_publication(
    store: &store::AsyncInMemoryRunStore,
    run_id: &mfm_ids::RunId,
    expected_count: usize,
) {
    let projection = store.projection_snapshot().expect("fact projection");
    assert_eq!(
        projection
            .fact_query_entries()
            .filter(|(_, entry)| {
                entry.fact_kind().as_str() == "evm.balance_snapshot"
                    && entry.source_run_id() == run_id
            })
            .count(),
        expected_count,
        "the run must publish one unified EVM fact per demanded source"
    );

    let stream = store
        .load_run_stream(run_id)
        .await
        .expect("completed snapshot stream");
    let facts = stream
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                mfm_events::v1::KernelEventPayload::FactRecorded(payload)
                    if payload.claim.fact_kind().as_str() == "evm.balance_snapshot"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(facts.len(), expected_count);
    let Some(first) = facts.first() else {
        return;
    };
    let mfm_events::v1::KernelEventPayload::FactRecorded(first_payload) = first.payload() else {
        unreachable!("filtered FactRecorded event")
    };
    assert!(facts.iter().all(|event| {
        let mfm_events::v1::KernelEventPayload::FactRecorded(payload) = event.payload() else {
            return false;
        };
        payload.node_id == first_payload.node_id
            && payload.attempt_id == first_payload.attempt_id
            && event.commit_key() == first.commit_key()
            && event.store_commit_order() == first.store_commit_order()
    }));
    assert!(
        stream.iter().any(|event| {
            matches!(
                event.payload(),
                mfm_events::v1::KernelEventPayload::CellProduced(payload)
                    if payload.node_id == first_payload.node_id
                        && payload.attempt_id == first_payload.attempt_id
                        && event.commit_key() == first.commit_key()
                        && event.store_commit_order() == first.store_commit_order()
            )
        }),
        "the EVM collection receipt and every fact must share one atomic commit"
    );
}

/// A complete mixed snapshot rejects retained query or selected-holdings substitutions before
/// either can influence the reconstructed report.
#[tokio::test]
async fn snapshot_root_replay_rejects_tampered_retained_projection_evidence() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let services = snapshot_services(Arc::new(store.clone()), Some(&runtime_path)).await;
    let run_id = launch_snapshot_completed(
        &services,
        &snapshot_config(SnapshotDemand::Mixed),
        "tampered-retained-projection-evidence",
    )
    .await;
    let source_calls_before_replay = server.calls();
    drop(services);
    std::fs::remove_file(&runtime_path).expect("remove runtime config before replay");

    let replay_services = mfm_app::make_run_read_services(
        Arc::new(store.clone()),
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    );
    assert_eq!(
        replay_services
            .verify_replay_for_run(&run_id)
            .await
            .expect("complete receipt-pinned snapshot replay")
            .run_mode,
        mfm_app::RunModeStatus::Completed
    );

    for (label, tamper) in [
        (
            "evm-collection-receipt",
            SnapshotArtifactTamper::EvmCollectionReceipt,
        ),
        ("query-evidence", SnapshotArtifactTamper::FactQueryEvidence),
        ("evm-fact-response", SnapshotArtifactTamper::EvmFactResponse),
        (
            "selected-holdings",
            SnapshotArtifactTamper::SelectedHoldings,
        ),
    ] {
        let tampered_replay_services = mfm_app::make_run_read_services(
            Arc::new(SnapshotStore::tampered(store.clone(), tamper)),
            mfm_app::production_certification_registry().expect("snapshot certification registry"),
        );
        let error = tampered_replay_services
            .verify_replay_for_run(&run_id)
            .await
            .expect_err("tampered retained evidence must fail replay");
        assert_eq!(error.code, "ArtifactEvidenceMismatch", "{label}");
    }
    assert_eq!(server.calls(), source_calls_before_replay);
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
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let services = snapshot_services(Arc::new(store.clone()), Some(&runtime_path)).await;
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
        json!(2),
        "the public snapshot contract reflects the reduced holding shape"
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

/// A wallet without configured symbols is retained as a zero-total public wallet, but its network
/// remains outside the explicit holding-demand graph and never opens a collector source.
#[tokio::test]
async fn snapshot_root_keeps_zero_symbol_wallet_without_undemanded_network_work() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let services = snapshot_services(Arc::new(store.clone()), Some(&runtime_path)).await;
    let config = snapshot_config_with_zero_symbol_bitcoin_wallet();
    let draft = portfolio_snapshot_program_draft(config.clone()).expect("zero-symbol wallet draft");
    assert!(
        draft
            .operation_lineage()
            .iter()
            .all(|operation| operation.operation_name != "mfm.bitcoin.btc_network_collection"),
        "an undemanded Bitcoin wallet must not create a collection operation"
    );
    assert!(
        draft
            .state_nodes()
            .iter()
            .all(|node| !node.state_descriptor_name.starts_with("mfm.bitcoin.")),
        "an undemanded Bitcoin wallet must not create collector states"
    );

    let request = prepare_snapshot_request(&services, config, "zero-symbol-wallet").await;
    let rendered = services
        .launch_run_and_render(request)
        .await
        .expect("zero-symbol wallet snapshot launch")
        .public_output
        .expect("zero-symbol wallet public output")
        .json
        .expect("zero-symbol wallet JSON");
    let snapshot = &rendered["snapshot"];
    let report = &rendered["report"];
    let pins = snapshot["network_pins"]
        .as_array()
        .expect("snapshot network pins");
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0]["network_id"], json!("ethereum-mainnet"));
    let zero_wallet = snapshot["wallets"]
        .as_array()
        .expect("snapshot wallets")
        .iter()
        .find(|wallet| wallet["wallet_id"] == json!("wallet_btc_zero"))
        .expect("zero-symbol wallet snapshot");
    assert_eq!(zero_wallet["network_id"], json!("bitcoin-mainnet"));
    assert_eq!(zero_wallet["observations"], json!([]));
    let zero_summary = report["wallet_summaries"]
        .as_array()
        .expect("report wallet summaries")
        .iter()
        .find(|wallet| wallet["wallet_id"] == json!("wallet_btc_zero"))
        .expect("zero-symbol wallet report");
    assert_eq!(zero_summary["network_id"], json!("bitcoin-mainnet"));
    assert_eq!(
        zero_summary["totals_by_quote"],
        json!([{"quote": "USD", "total_value_dec": "0"}])
    );
    let methods = server.methods();
    assert!(
        methods.iter().all(|method| !matches!(
            method.as_str(),
            "getblockchaininfo" | "getblockhash" | "getblockheader" | "scantxoutset"
        )),
        "the zero-symbol Bitcoin wallet must not open a Bitcoin source: {methods:?}"
    );
    assert!(
        methods.iter().any(|method| method == "eth_getBalance"),
        "the independently demanded EVM wallet still collects normally: {methods:?}"
    );
}

/// An interrupted exact root resumes after collection with no runtime configuration or live
/// provider. The final replay is likewise read-only and uses the same production draft helper.
#[tokio::test]
async fn snapshot_root_resumes_and_replays_after_live_inputs_disappear() {
    let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
    let runtime_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let (entered_tx, entered_rx) = oneshot::channel();
    let initial_services = snapshot_services(
        Arc::new(SnapshotStore::blocking_query(store.clone(), entered_tx)),
        Some(&runtime_path),
    )
    .await;
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

    let prefix_replay_services = mfm_app::make_run_read_services(
        Arc::new(store.clone()),
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    );
    let prefix_replay = prefix_replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only collection-receipt prefix replay");
    assert_eq!(prefix_replay.run_mode, mfm_app::RunModeStatus::Forward);

    let resumed_services =
        snapshot_services(Arc::new(SnapshotStore::passthrough(store.clone())), None).await;
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
        Arc::new(store.clone()),
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    );
    let replay = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("evidence-only snapshot replay");
    assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(server.calls(), source_calls_before_resume);
}

/// Replay verifies every completed portfolio projection stage while accepting an interrupted
/// strict prefix whose remaining downstream stages have not produced outputs.
#[tokio::test]
async fn snapshot_root_replay_verifies_each_downstream_output_prefix() {
    for (label, blocked_input_schema) in [
        ("selected", "mfm.portfolio.selected_holdings"),
        ("snapshot", "mfm.portfolio.snapshot"),
    ] {
        let server = start_snapshot_rpc_mock(SnapshotRpcConfig::default()).await;
        let runtime_dir = tempfile::tempdir().expect("runtime config directory");
        let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
        let store = store::AsyncInMemoryRunStore::default();
        let (entered_tx, entered_rx) = oneshot::channel();
        let blocked_store = Arc::new(SnapshotStore::blocking_artifact(
            store.clone(),
            blocked_input_schema,
            entered_tx,
        ));
        let runners = mfm_app::production_runner_registry_for_test(
            blocked_store.clone(),
            Some(&runtime_path),
        )
        .await
        .expect("production snapshot runners");
        let services = mfm_app::make_run_services(
            runners,
            blocked_store,
            mfm_app::production_certification_registry().expect("snapshot certification registry"),
        );
        let draft = portfolio_snapshot_program_draft(snapshot_config(SnapshotDemand::EvmNative))
            .expect("prefix snapshot draft");
        let request = mfm_app::prepare_typed_program_run_launch_for_test(
            draft,
            Default::default(),
            services.certification_registry(),
            services.load_store_scope_id().await.expect("store scope"),
            Some(
                mfm_app::InvocationKey::new(format!("prefix-{label}"))
                    .expect("prefix invocation key"),
            ),
        )
        .expect("prefix snapshot launch request");
        let run_id = request.run_id.clone();
        let launch_services = services.clone();
        let launch = tokio::spawn(async move { launch_services.launch_run(request).await });

        tokio::time::timeout(Duration::from_secs(10), entered_rx)
            .await
            .unwrap_or_else(|_| panic!("{label} prefix was not reached"))
            .unwrap_or_else(|_| panic!("{label} prefix signal was dropped"));
        let source_calls_before_replay = server.calls();
        launch.abort();
        let _ = launch.await;
        assert!(store
            .expire_execution_claim_for_test(&run_id)
            .expect("expire interrupted execution claim"));
        drop(services);
        std::fs::remove_file(&runtime_path).expect("remove runtime config before prefix replay");

        let replay_application =
            mfm_app::in_memory_application_with_panicking_live_io_for_test(store.clone());
        let replay = replay_application
            .verify_replay(&run_id)
            .await
            .unwrap_or_else(|error| panic!("{label} output prefix replay: {error:?}"));
        assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Forward, "{label}");
        assert_eq!(
            server.calls(),
            source_calls_before_replay,
            "{label} prefix replay must not reopen a live provider"
        );
    }
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
    let runtime_path = write_portfolio_runtime_config_for_test(runtime_dir.path(), &server.url);
    let store = store::AsyncInMemoryRunStore::default();
    let services = snapshot_services(Arc::new(store.clone()), Some(&runtime_path)).await;
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

fn snapshot_config_with_zero_symbol_bitcoin_wallet() -> PortfolioConfig {
    let mut value =
        serde_json::to_value(snapshot_config(SnapshotDemand::EvmNative)).expect("portfolio value");
    value["networks"]
        .as_array_mut()
        .expect("portfolio networks")
        .push(json!({
            "network_id": "bitcoin-mainnet",
            "family": "bitcoin",
            "bitcoin_network": "main",
            "source_identity": "public-bitcoin-core",
            "metadata": {}
        }));
    value["wallets"]
        .as_array_mut()
        .expect("portfolio wallets")
        .push(json!({
            "wallet_id": "wallet_btc_zero",
            "network_id": "bitcoin-mainnet",
            "symbol_ids": [],
            "subject": {"kind": "bitcoin_address", "address": BTC_ADDRESS},
            "implementation": {"kind": "address_only"},
            "metadata": {}
        }));
    ValidatedPortfolioConfig::new(
        serde_json::from_value(value).expect("zero-symbol wallet portfolio"),
    )
    .expect("zero-symbol wallet remains valid with another demanded edge")
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

trait SnapshotServiceStore:
    store::RunEventStore<Error = store::StoreError>
    + store::StoreScopeStore<Error = store::StoreError>
    + store::ExecutionClaimStore<Error = store::StoreError>
    + store::RetainedArtifactReadProvider
    + store::FactQueryStore<Error = store::StoreError>
    + Send
    + Sync
    + 'static
{
}

impl<T> SnapshotServiceStore for T where
    T: store::RunEventStore<Error = store::StoreError>
        + store::StoreScopeStore<Error = store::StoreError>
        + store::ExecutionClaimStore<Error = store::StoreError>
        + store::RetainedArtifactReadProvider
        + store::FactQueryStore<Error = store::StoreError>
        + Send
        + Sync
        + 'static
{
}

async fn snapshot_services<S>(
    store: Arc<S>,
    runtime_config_path: Option<&Path>,
) -> mfm_app::RunServices<S>
where
    S: SnapshotServiceStore,
{
    let runners = mfm_app::production_runner_registry_for_test(store.clone(), runtime_config_path)
        .await
        .expect("production snapshot runners");
    mfm_app::make_run_services(
        runners,
        store,
        mfm_app::production_certification_registry().expect("snapshot certification registry"),
    )
}

async fn prepare_snapshot_request<S>(
    services: &mfm_app::RunServices<S>,
    config: PortfolioConfig,
    invocation_key: &str,
) -> mfm_app::RunLaunchRequest
where
    S: SnapshotServiceStore,
{
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

async fn launch_snapshot_completed<S>(
    services: &mfm_app::RunServices<S>,
    config: &PortfolioConfig,
    invocation_key: &str,
) -> mfm_ids::RunId
where
    S: SnapshotServiceStore,
{
    let response = launch_snapshot(services, config.clone(), invocation_key).await;
    assert_eq!(
        response.run_mode,
        mfm_app::RunModeStatus::Completed,
        "{invocation_key} complete snapshot run: {response:?}"
    );
    response.run_id.parse().expect("snapshot run id")
}

async fn launch_snapshot<S>(
    services: &mfm_app::RunServices<S>,
    config: PortfolioConfig,
    invocation_key: &str,
) -> mfm_app::RunResponse
where
    S: SnapshotServiceStore,
{
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

#[derive(Clone, Copy)]
enum FactQueryMutation {
    OmitRows,
    SplitFrontier,
    StoreFailure,
}

enum SnapshotQueryBehavior {
    Passthrough,
    RecordRows(Mutex<Vec<usize>>),
    Mutate {
        mutation: FactQueryMutation,
        batch_widths: Mutex<Vec<usize>>,
    },
    Block(Mutex<Option<oneshot::Sender<()>>>),
}

enum SnapshotArtifactBehavior {
    Passthrough,
    Block {
        schema: &'static str,
        entered: Mutex<Option<oneshot::Sender<()>>>,
    },
    Tamper(SnapshotArtifactTamper),
}

#[derive(Clone)]
struct SnapshotStore {
    inner: store::AsyncInMemoryRunStore,
    query: Arc<SnapshotQueryBehavior>,
    artifacts: Arc<SnapshotArtifactBehavior>,
}

impl SnapshotStore {
    fn passthrough(inner: store::AsyncInMemoryRunStore) -> Self {
        Self {
            inner,
            query: Arc::new(SnapshotQueryBehavior::Passthrough),
            artifacts: Arc::new(SnapshotArtifactBehavior::Passthrough),
        }
    }

    fn recording(inner: store::AsyncInMemoryRunStore) -> Self {
        Self {
            inner,
            query: Arc::new(SnapshotQueryBehavior::RecordRows(Mutex::new(Vec::new()))),
            artifacts: Arc::new(SnapshotArtifactBehavior::Passthrough),
        }
    }

    fn adversarial(inner: store::AsyncInMemoryRunStore, mutation: FactQueryMutation) -> Self {
        Self {
            inner,
            query: Arc::new(SnapshotQueryBehavior::Mutate {
                mutation,
                batch_widths: Mutex::new(Vec::new()),
            }),
            artifacts: Arc::new(SnapshotArtifactBehavior::Passthrough),
        }
    }

    fn blocking_query(inner: store::AsyncInMemoryRunStore, entered: oneshot::Sender<()>) -> Self {
        Self {
            inner,
            query: Arc::new(SnapshotQueryBehavior::Block(Mutex::new(Some(entered)))),
            artifacts: Arc::new(SnapshotArtifactBehavior::Passthrough),
        }
    }

    fn blocking_artifact(
        inner: store::AsyncInMemoryRunStore,
        schema: &'static str,
        entered: oneshot::Sender<()>,
    ) -> Self {
        Self {
            inner,
            query: Arc::new(SnapshotQueryBehavior::Passthrough),
            artifacts: Arc::new(SnapshotArtifactBehavior::Block {
                schema,
                entered: Mutex::new(Some(entered)),
            }),
        }
    }

    fn tampered(inner: store::AsyncInMemoryRunStore, tamper: SnapshotArtifactTamper) -> Self {
        Self {
            inner,
            query: Arc::new(SnapshotQueryBehavior::Passthrough),
            artifacts: Arc::new(SnapshotArtifactBehavior::Tamper(tamper)),
        }
    }

    fn returned_row_counts(&self) -> Vec<usize> {
        let SnapshotQueryBehavior::RecordRows(counts) = self.query.as_ref() else {
            panic!("snapshot store is not recording query rows")
        };
        counts.lock().expect("returned row counts").clone()
    }

    fn batch_widths(&self) -> Vec<usize> {
        let SnapshotQueryBehavior::Mutate { batch_widths, .. } = self.query.as_ref() else {
            panic!("snapshot store is not mutating queries")
        };
        batch_widths.lock().expect("batch widths").clone()
    }
}

impl store::FactQueryStore for SnapshotStore {
    type Error = store::StoreError;

    fn fact_query_implementation_id(&self) -> &'static str {
        "mfm.integration.snapshot.fact-query.v1"
    }

    fn execute_fact_queries<'a>(
        &'a self,
        plans: &'a [mfm_facts::CanonicalFactQueryPlan],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<mfm_facts::FactQueryResult>, store::StoreError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            match self.query.as_ref() {
                SnapshotQueryBehavior::Mutate {
                    mutation,
                    batch_widths,
                } => {
                    batch_widths.lock().expect("batch widths").push(plans.len());
                    if matches!(mutation, FactQueryMutation::StoreFailure) {
                        return Err(store::StoreError::Identity(
                            "private temporary query-store failure".to_owned(),
                        ));
                    }
                }
                SnapshotQueryBehavior::Block(entered) => {
                    let entered = entered.lock().expect("blocking query state").take();
                    if let Some(entered) = entered {
                        let _ = entered.send(());
                        future::pending().await
                    }
                }
                SnapshotQueryBehavior::Passthrough | SnapshotQueryBehavior::RecordRows(_) => {}
            }

            let mut responses =
                store::FactQueryStore::execute_fact_queries(&self.inner, plans).await?;
            match self.query.as_ref() {
                SnapshotQueryBehavior::RecordRows(counts) => {
                    counts
                        .lock()
                        .expect("returned row counts")
                        .extend(responses.iter().map(|response| response.rows().len()));
                }
                SnapshotQueryBehavior::Mutate {
                    mutation: FactQueryMutation::OmitRows,
                    ..
                } => {
                    for (plan, response) in plans.iter().zip(&mut responses) {
                        let rows = Vec::new();
                        let receipt = mfm_facts::FactQueryReceipt::from_rows(
                            response.receipt().read_frontier().clone(),
                            &rows,
                            response.receipt().returned_field_summaries().is_some(),
                            plan.limit(),
                        )
                        .map_err(|error| store::StoreError::Identity(error.to_string()))?;
                        *response = mfm_facts::FactQueryResult::new(rows, receipt)
                            .map_err(|error| store::StoreError::Identity(error.to_string()))?;
                    }
                }
                SnapshotQueryBehavior::Mutate {
                    mutation: FactQueryMutation::SplitFrontier,
                    ..
                } => {
                    if let Some((plan, response)) = plans.get(1).zip(responses.get_mut(1)) {
                        let current = response.receipt().read_frontier();
                        let store_commit_order =
                            current.store_commit_order().checked_next().ok_or_else(|| {
                                store::StoreError::Identity(
                                    "test read frontier overflowed".to_owned(),
                                )
                            })?;
                        let frontier = mfm_facts::StoreReadFrontier::new(
                            current.store_scope_id().clone(),
                            store_commit_order,
                        );
                        let rows = response.rows().to_vec();
                        let receipt = mfm_facts::FactQueryReceipt::from_rows(
                            frontier,
                            &rows,
                            response.receipt().returned_field_summaries().is_some(),
                            plan.limit(),
                        )
                        .map_err(|error| store::StoreError::Identity(error.to_string()))?;
                        *response = mfm_facts::FactQueryResult::new(rows, receipt)
                            .map_err(|error| store::StoreError::Identity(error.to_string()))?;
                    }
                }
                SnapshotQueryBehavior::Mutate {
                    mutation: FactQueryMutation::StoreFailure,
                    ..
                } => unreachable!("store failure returns before querying"),
                SnapshotQueryBehavior::Passthrough | SnapshotQueryBehavior::Block(_) => {}
            }
            Ok(responses)
        })
    }
}

impl store::RunEventStore for SnapshotStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        store::RunEventStore::append_prepared_commit_bundle(&self.inner, bundle)
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        store::RunEventStore::load_run_stream(&self.inner, run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        store::RunEventStore::load_committed_run_stream(&self.inner, run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        store::RunEventStore::expected_next_seq(&self.inner, run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        store::RunEventStore::status_projection_snapshot(&self.inner, run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        store::RunEventStore::fact_projection_snapshot(&self.inner)
    }
}

impl store::StoreScopeStore for SnapshotStore {
    type Error = store::StoreError;

    fn load_store_scope_id<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, mfm_ids::StoreScopeId, Self::Error> {
        store::StoreScopeStore::load_store_scope_id(&self.inner)
    }
}

impl store::ExecutionClaimStore for SnapshotStore {
    type Error = store::StoreError;

    fn acquire_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
        store::ExecutionClaimStore::acquire_execution_claim(
            &self.inner,
            scope,
            holder_run_id,
            token,
        )
    }

    fn execution_claim_status<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
    ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
        store::ExecutionClaimStore::execution_claim_status(&self.inner, scope)
    }

    fn renew_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
        store::ExecutionClaimStore::renew_execution_claim(&self.inner, scope, holder_run_id, token)
    }

    fn release_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
        store::ExecutionClaimStore::release_execution_claim(
            &self.inner,
            scope,
            holder_run_id,
            token,
        )
    }

    fn expired_execution_claims<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, Vec<store::ExpiredExecutionClaim>, Self::Error> {
        store::ExecutionClaimStore::expired_execution_claims(&self.inner)
    }

    fn reap_expired_execution_claim<'a>(
        &'a self,
        scope: &'a store::ExecutionClaimScope,
        holder_run_id: &'a mfm_ids::RunId,
        token: &'a store::AdmissionToken,
    ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
        store::ExecutionClaimStore::reap_expired_execution_claim(
            &self.inner,
            scope,
            holder_run_id,
            token,
        )
    }
}

impl store::RetainedArtifactReadProvider for SnapshotStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            if let SnapshotArtifactBehavior::Block { schema, entered } = self.artifacts.as_ref() {
                let blocks_output = requirement.artifact_role
                    == Some(mfm_events::v1::ArtifactRole::StateOutput)
                    && requirement
                        .schema_id
                        .as_ref()
                        .and_then(|value| value.canonical_name())
                        == Some(*schema);
                if blocks_output {
                    let entered = entered.lock().expect("blocking artifact state").take();
                    if let Some(entered) = entered {
                        let _ = entered.send(());
                        future::pending().await
                    }
                }
            }

            let artifact = store::RetainedArtifactReadProvider::read_retained_artifact(
                &self.inner,
                requirement,
            )
            .await?;
            match self.artifacts.as_ref() {
                SnapshotArtifactBehavior::Tamper(tamper) => {
                    tamper_snapshot_artifact(artifact, requirement, *tamper)
                }
                SnapshotArtifactBehavior::Passthrough | SnapshotArtifactBehavior::Block { .. } => {
                    Ok(artifact)
                }
            }
        })
    }
}

#[derive(Clone, Copy)]
enum SnapshotArtifactTamper {
    EvmCollectionReceipt,
    FactQueryEvidence,
    EvmFactResponse,
    SelectedHoldings,
}

fn tamper_snapshot_artifact(
    artifact: store::VerifiedRetainedArtifactBytes,
    requirement: &store::EventArtifactRequirement,
    tamper: SnapshotArtifactTamper,
) -> store::Result<store::VerifiedRetainedArtifactBytes> {
    let schema_name = artifact
        .evidence()
        .schema_id
        .as_ref()
        .and_then(|schema| schema.canonical_name());
    let applies = match tamper {
        SnapshotArtifactTamper::EvmCollectionReceipt => {
            artifact.evidence().artifact_role == mfm_events::v1::ArtifactRole::StateOutput
                && schema_name == Some("mfm.evm.balance_collection_receipt")
        }
        SnapshotArtifactTamper::FactQueryEvidence => {
            artifact.evidence().artifact_role == mfm_events::v1::ArtifactRole::FactQueryEvidence
        }
        SnapshotArtifactTamper::EvmFactResponse => {
            artifact.evidence().artifact_role == mfm_events::v1::ArtifactRole::FactResponse
                && schema_name == Some("mfm.evm.fact.balance_snapshot.response")
        }
        SnapshotArtifactTamper::SelectedHoldings => {
            artifact.evidence().artifact_role == mfm_events::v1::ArtifactRole::StateOutput
                && schema_name == Some("mfm.portfolio.selected_holdings")
        }
    };
    if !applies {
        return Ok(artifact);
    }

    let tampered_bytes = match tamper {
        SnapshotArtifactTamper::EvmCollectionReceipt
        | SnapshotArtifactTamper::FactQueryEvidence => {
            let mut bytes = artifact.bytes().to_vec();
            bytes.push(b'\n');
            bytes
        }
        SnapshotArtifactTamper::EvmFactResponse => {
            let mut value: Value =
                serde_json::from_slice(artifact.bytes()).expect("EVM fact response JSON");
            value["raw_units"] = json!("999");
            serde_json::to_vec(&value).expect("tampered EVM fact response JSON")
        }
        SnapshotArtifactTamper::SelectedHoldings => {
            let mut value: Value =
                serde_json::from_slice(artifact.bytes()).expect("selected holdings output JSON");
            let observations = value["observations"]
                .as_array_mut()
                .expect("selected holdings");
            let mut duplicate = observations.first().cloned().expect("selected holding");
            duplicate["display_symbol"] = json!("substituted");
            duplicate["metadata"] = json!({"source": "substituted"});
            duplicate["values"] = json!([{
                "quote": "USD",
                "priced_symbol_id": "substituted.symbol",
                "unit_price_dec": "999",
                "value_dec": "999"
            }]);
            observations.push(duplicate);
            serde_json::to_vec(&value).expect("tampered selected holdings JSON")
        }
    };
    store::VerifiedRetainedArtifactBytes::new(
        tampered_bytes,
        artifact.evidence().clone(),
        requirement,
    )
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
    methods: Arc<Mutex<Vec<String>>>,
}

struct SnapshotRpcServer {
    url: String,
    calls: Arc<AtomicUsize>,
    methods: Arc<Mutex<Vec<String>>>,
}

impl SnapshotRpcServer {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn methods(&self) -> Vec<String> {
        self.methods.lock().expect("snapshot RPC methods").clone()
    }
}

async fn start_snapshot_rpc_mock(config: SnapshotRpcConfig) -> SnapshotRpcServer {
    let calls = Arc::new(AtomicUsize::new(0));
    let methods = Arc::new(Mutex::new(Vec::new()));
    let state = SnapshotRpcState {
        config,
        calls: Arc::clone(&calls),
        methods: Arc::clone(&methods),
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
        methods,
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
    state
        .methods
        .lock()
        .expect("snapshot RPC methods")
        .push(method.to_owned());
    if state.config.failing_method == Some(method) {
        return Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32000, "message": "configured snapshot source failure"}
        }));
    }
    let result = match method {
        "eth_chainId" => json!("0x1"),
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
        "scantxoutset" => {
            let amount = serde_json::from_str::<Value>(state.config.btc_amount_json)
                .expect("valid test Bitcoin amount");
            let unspents = if state.config.btc_amount_json == "0" {
                Vec::new()
            } else {
                vec![json!({
                    "txid": "1111111111111111111111111111111111111111111111111111111111111111",
                    "vout": 0,
                    "scriptPubKey": "0014311564348890e005880a9bc834aaa5884f1b5932",
                    "desc": format!("addr({BTC_ADDRESS})"),
                    "amount": amount.clone(),
                    "height": 850_100u64,
                })]
            };
            json!({
                "success": true,
                "txouts": unspents.len(),
                "height": 850_100u64,
                "bestblock": BTC_HASH,
                "unspents": unspents,
                "total_amount": amount,
            })
        }
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
