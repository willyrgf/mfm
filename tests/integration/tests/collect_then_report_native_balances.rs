//! Collect-then-report e2e: one certified parent run collects and reports balances.
//!
//! The highest-value operator path after cutover is one direct typed parent launch: mock
//! transport collection (BTC + EVM natives, shared joint tip) → typed readiness → portfolio
//! snapshot/report in the same run, followed by evidence-only replay.
//!
//! The test uses the production runner registry and Postgres fact capabilities, matching the
//! production assembly boundary without catalog or CLI orchestration.

#![allow(clippy::disallowed_methods)]

#[cfg(feature = "parity-tests")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "parity-tests")]
use std::num::NonZeroU64;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, B256, U256};
use mfm_btc_capabilities::{
    BtcBalanceReadProvider, BtcBalanceReadRequest, BtcBalanceReadResponse, BtcBlockHash,
    BtcCapabilityFuture, BtcChainHeadReadProvider, BtcChainHeadRequest, BtcChainHeadResponse,
    BtcSourceBinding, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBalanceReadProvider, EvmBalanceReadRequest, EvmBalanceReadResponse, EvmBlockReadProvider,
    EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector, EvmCapabilityFuture,
    EvmNetworkBinding, EvmSourcePolicyId, EvmSourceRef, RedactedEvmSourceEvidence,
};
use mfm_facts::FactAudience;
#[cfg(feature = "parity-tests")]
use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema,
    schema_scoped_database_url, start_collectors_rpc_mock, unique_postgres_schema,
    write_collectors_runtime_config_for_test,
};
use mfm_integration_tests::test_support::{
    prepare_btc_balance_launch_for_store, prepare_evm_balance_launch_for_store,
    register_process_fact_capabilities, ProjectionFactIndexProvider,
};
#[cfg(feature = "parity-tests")]
use mfm_op_btc_collectors::BtcAddressBalanceObservationContext;
#[cfg(feature = "parity-tests")]
use mfm_op_portfolio_collect_report::{
    build_collect_then_report_config, collect_then_report_program_draft, BitcoinCollectorPolicy,
    EvmCollectorPolicy,
};
#[cfg(feature = "parity-tests")]
use mfm_program::{
    BridgeKind, CanonicalSeed, InputBindingNode, InputBindingNodeRef, TypedProgramDraft,
};
use mfm_store::v1::{
    AsyncInMemoryRunStore, ProjectionSnapshot, RetainedArtifactReadProvider, RunEventStore,
    StoreScopeStore,
};
#[cfg(feature = "parity-tests")]
use serde_json::Value;

const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";
const ETH_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
const BTC_HASH: &str = "abababababababababababababababababababababababababababababababab";
const EVM_HASH_HEX: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
const BTC_HEIGHT: u64 = 850_100;
const EVM_BLOCK: u64 = 21_000_000;
const BTC_SATS: u64 = 100_000;
const EVM_WEI: u64 = 1_000_000_000_000_000_000;

#[tokio::test]
async fn btc_address_balance_collector_admits_platform_holding_fact() {
    let store = AsyncInMemoryRunStore::default();
    let fact_index = Arc::new(ProjectionFactIndexProvider::new(store.clone()));
    let btc = Arc::new(MockBtcBalanceProvider::new());
    let services = btc_collector_services(store.clone(), btc.clone(), fact_index);

    let response = launch_btc_balance_collector(&services, &store, "btc-balance-1").await;
    assert_eq!(response.run_mode, mfm_app::RunModeStatus::Completed);
    assert!(btc.chain_head_calls() >= 1);
    assert!(btc.balance_calls() >= 1);

    let projection = store.projection_snapshot().expect("projection");
    assert_platform_holding_kind(&projection, "bitcoin.address_balance_snapshot", 1);
}

#[tokio::test]
async fn evm_native_balance_collector_admits_platform_holding_fact() {
    let store = AsyncInMemoryRunStore::default();
    let fact_index = Arc::new(ProjectionFactIndexProvider::new(store.clone()));
    let evm = Arc::new(MockEvmBalanceProvider::new());
    let services = evm_collector_services(store.clone(), evm.clone(), fact_index);

    let response = launch_evm_balance_collector(&services, &store, "evm-balance-1").await;
    assert_eq!(response.run_mode, mfm_app::RunModeStatus::Completed);
    assert!(evm.block_calls() >= 1);
    assert!(evm.balance_calls() >= 1);

    let projection = store.projection_snapshot().expect("projection");
    assert_platform_holding_kind(&projection, "evm.address_native_balance_snapshot", 1);
}

#[tokio::test]
#[cfg(feature = "parity-tests")]
async fn collect_then_report_completes_from_collector_written_platform_holdings() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_postgres_schema("collect_report");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_database_url, 20, 250).await;

    let rpc_url = start_collectors_rpc_mock().await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path =
        write_collectors_runtime_config_for_test(runtime_config_dir.path(), &rpc_url);
    let portfolio: mfm_op_portfolio_tracker::PortfolioConfig = serde_json::from_value(
        dual_mainnet_portfolio_json()
            .get("portfolio")
            .cloned()
            .expect("portfolio value"),
    )
    .expect("portfolio config");
    let config = build_collect_then_report_config(
        portfolio,
        BitcoinCollectorPolicy::new("configured_only", NonZeroU64::new(1).expect("non-zero")),
        EvmCollectorPolicy::new(
            "configured_only",
            Some(18),
            NonZeroU64::new(1).expect("non-zero"),
        ),
    )
    .expect("composed config");
    let draft = collect_then_report_program_draft(config).expect("composed draft");
    assert_composed_parent_plan(&draft);

    let seed = CanonicalSeed::from_value(&BtcAddressBalanceObservationContext {
        observed_at_unix_ms: None,
    })
    .expect("observation seed");
    let seed_material = draft
        .seeds()
        .iter()
        .map(|seed_spec| (seed_spec.seed_id.clone(), seed.canonical_json().clone()))
        .collect::<BTreeMap<_, _>>();
    let certification_registry = mfm_app::production_certification_registry().expect("cert");
    let store_scope_id = store.load_store_scope_id().await.expect("store scope");
    let prepared = mfm_app::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        &certification_registry,
        store_scope_id,
        Some(mfm_app::InvocationKey::new("collect-then-report-parent").expect("invocation key")),
    )
    .expect("prepared composed launch");
    let certified_spec = prepared.certified_spec.clone();
    let fact_index = mfm_app::production_fact_index_read_provider(store.clone());
    let runners = mfm_app::production_runner_registry(
        Arc::new(store.clone()),
        fact_index,
        Some(runtime_config_path.as_path()),
    )
    .expect("production runners");
    let runtime_spec = mfm_runtime::CertifiedRuntimeSpec::new(prepared.certified_spec.clone())
        .expect("runtime spec");
    mfm_runtime::BoundRuntimeContextLoader::new(runners.clone())
        .load(&runtime_spec)
        .expect("runner bindings");
    let services = mfm_app::make_run_services(
        runners,
        store.clone(),
        store.clone(),
        certification_registry,
    );
    let report = services
        .launch_run_and_render(prepared)
        .await
        .expect("composed launch");
    assert_eq!(report.outcome, mfm_app::RunLaunchOutcomeStatus::Admitted);
    let run = report.run.expect("composed run response");
    assert_eq!(run.run_mode, mfm_app::RunModeStatus::Completed);
    let run_id = run.run_id.parse().expect("run id");
    let public_output = report.public_output.expect("composed public output");
    let json = public_output.json.expect("composed public JSON");
    let snapshot = find_public_object(&json, is_snapshot_public_object)
        .expect("public output should carry snapshot material");
    assert_observation(
        snapshot,
        "btc.native.bitcoin-mainnet",
        "100000",
        8,
        "0.00100000",
        "50.00000000",
    );
    assert_observation(
        snapshot,
        "eth.native.ethereum-mainnet",
        "1000000000000000000",
        18,
        "1.000000000000000000",
        "1800.000000000000000000",
    );
    let report = find_public_object(&json, is_report_public_object)
        .expect("public output should carry report material");
    let usd_total = report["totals_by_quote"]
        .as_array()
        .expect("report quote totals")
        .iter()
        .find(|total| total.get("quote").and_then(Value::as_str) == Some("USD"))
        .expect("USD portfolio total");
    assert_eq!(
        usd_total.get("assets_value_dec").and_then(Value::as_str),
        Some("1850.000000000000000000")
    );
    assert_eq!(
        usd_total.get("net_value_dec").and_then(Value::as_str),
        Some("1850.000000000000000000")
    );
    let stream = store.load_run_stream(&run_id).await.expect("parent stream");
    assert!(
        !stream.is_empty(),
        "parent stream should contain committed events"
    );
    assert!(stream.iter().all(|event| event.run_id() == &run_id));
    assert_eq!(
        stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::FactRecorded(payload)
                    if payload.claim.visibility()
                        == &mfm_facts::FactVisibility::indexed_default(FactAudience::Platform) =>
                {
                    Some(payload.claim.fact_kind().as_str())
                }
                _ => None,
            })
            .filter(|fact_kind| {
                *fact_kind == "bitcoin.address_balance_snapshot"
                    || *fact_kind == "evm.address_native_balance_snapshot"
            })
            .count(),
        2
    );
    assert_eq!(
        stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::FactRecorded(payload)
                    if payload.claim.fact_kind().as_str() == "bitcoin.address_balance_snapshot" =>
                {
                    Some(())
                }
                _ => None,
            })
            .count(),
        1
    );
    assert_eq!(
        stream
            .iter()
            .filter_map(|event| match event.payload() {
                events::KernelEventPayload::FactRecorded(payload)
                    if payload.claim.fact_kind().as_str()
                        == "evm.address_native_balance_snapshot" =>
                {
                    Some(())
                }
                _ => None,
            })
            .count(),
        1
    );
    let replay = services
        .verify_replay_for_run(&run_id)
        .await
        .expect("parent replay");
    assert_eq!(replay.run_id, run_id.to_string());
    assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Completed);
    assert!(replay.retained_artifacts > 0);
    assert_eq!(replay.spec_hash, certified_spec.spec_hash().to_string());

    drop_postgres_schema(&database_url, &schema).await;
}

#[cfg(feature = "parity-tests")]
fn assert_composed_parent_plan(draft: &TypedProgramDraft) {
    let scopes = draft.scopes();
    assert_eq!(scopes.len(), 3, "one root and two collector scopes");
    let root_scope = scopes
        .iter()
        .find(|scope| scope.parent_scope_id.is_none())
        .expect("root scope");
    assert_eq!(root_scope.key.as_str(), "portfolio_collect_then_report");
    let scope_keys = scopes
        .iter()
        .map(|scope| scope.key.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        scope_keys,
        BTreeSet::from([
            "portfolio_collect_then_report",
            "bitcoin_collector_0",
            "evm_collector_0",
        ])
    );
    for child in scopes
        .iter()
        .filter(|scope| scope.scope_id != root_scope.scope_id)
    {
        assert_eq!(
            child.parent_scope_id.as_ref(),
            Some(&root_scope.scope_id),
            "collector scope must be a direct child of the certified parent"
        );
    }

    let frames = draft.operation_lineage();
    let composition = frames
        .iter()
        .find(|frame| frame.operation_name == "mfm.portfolio.collect_then_report")
        .expect("composition lineage frame");
    for frame in frames {
        if frame.operation_instance_id != composition.operation_instance_id {
            assert!(
                frame
                    .parent_operation_lineage
                    .active_instances
                    .contains(&composition.operation_instance_id),
                "{} must retain the composed parent lineage",
                frame.operation_name
            );
        }
    }

    let readiness = draft
        .state_nodes()
        .iter()
        .find(|node| node.key.as_str() == "collectors_ready")
        .expect("typed readiness state");
    let mut readiness_inputs = BTreeSet::new();
    collect_input_cell_ids(&readiness.input.root, &mut readiness_inputs);
    assert_eq!(
        readiness_inputs.len(),
        2,
        "readiness must consume one typed summary from each collector"
    );
    let exported_summary_cells = draft
        .bridge_nodes()
        .iter()
        .filter(|bridge| {
            bridge.bridge_kind == BridgeKind::ExportToParent
                && bridge.target_scope_id == root_scope.scope_id
        })
        .map(|bridge| bridge.target_cell_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(exported_summary_cells, readiness_inputs);

    let tracker = frames
        .iter()
        .find(|frame| frame.operation_name == "mfm.portfolio.tracker_workflow")
        .expect("tracker lineage frame");
    match tracker.input.root.as_ref() {
        InputBindingNodeRef::Cell(binding) => {
            assert_eq!(binding.cell_id(), &readiness.output_cell_id);
        }
        other => panic!("tracker must consume the readiness output cell, got {other:?}"),
    }
}

#[cfg(feature = "parity-tests")]
fn collect_input_cell_ids(node: &InputBindingNode, output: &mut BTreeSet<mfm_ids::CellId>) {
    match node.as_ref() {
        InputBindingNodeRef::Unit => {}
        InputBindingNodeRef::Cell(binding) => {
            output.insert(binding.cell_id().clone());
        }
        InputBindingNodeRef::Tuple(elements) => {
            for element in elements {
                collect_input_cell_ids(element, output);
            }
        }
        InputBindingNodeRef::Struct(fields) => {
            for field in fields {
                collect_input_cell_ids(&field.node, output);
            }
        }
        InputBindingNodeRef::Vec { elements, .. }
        | InputBindingNodeRef::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cell_ids(element, output);
            }
        }
    }
}

// --- services / launch -------------------------------------------------------

fn btc_collector_services(
    store: AsyncInMemoryRunStore,
    btc: Arc<MockBtcBalanceProvider>,
    fact_index: Arc<ProjectionFactIndexProvider>,
) -> mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore> {
    let artifacts: Arc<dyn RetainedArtifactReadProvider> = Arc::new(store.clone());
    let mut runners = mfm_runtime::ErasedRunnerRegistry::new();
    register_process_fact_capabilities(&mut runners, fact_index.as_ref())
        .expect("process fact capabilities");
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(
        &mut runners,
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(
            artifacts,
            btc,
            fact_index.clone(),
        ),
    )
    .expect("btc runners");
    mfm_app::RunServices::new_with_certification_registry(
        mfm_runtime::SerialTypedScheduler::new(runners, Arc::new(store.clone())),
        store.clone(),
        store,
        mfm_app::production_certification_registry().expect("cert"),
    )
}

fn evm_collector_services(
    store: AsyncInMemoryRunStore,
    evm: Arc<MockEvmBalanceProvider>,
    fact_index: Arc<ProjectionFactIndexProvider>,
) -> mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore> {
    let artifacts: Arc<dyn RetainedArtifactReadProvider> = Arc::new(store.clone());
    let mut runners = mfm_runtime::ErasedRunnerRegistry::new();
    register_process_fact_capabilities(&mut runners, fact_index.as_ref())
        .expect("process fact capabilities");
    mfm_adapters_evm::register_evm_collectors_runners(
        &mut runners,
        mfm_adapters_evm::EvmRunnerCapabilities::new(artifacts, evm),
    )
    .expect("evm runners");
    mfm_app::RunServices::new_with_certification_registry(
        mfm_runtime::SerialTypedScheduler::new(runners, Arc::new(store.clone())),
        store.clone(),
        store,
        mfm_app::production_certification_registry().expect("cert"),
    )
}

async fn launch_btc_balance_collector(
    services: &mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore>,
    store: &AsyncInMemoryRunStore,
    invocation_key: &str,
) -> mfm_app::RunResponse {
    let prepared = prepare_btc_balance_launch_for_store(
        store,
        &btc_balance_config_json(),
        Some(invocation_key),
    )
    .await;
    launch_prepared(services, store, invocation_key, prepared).await
}

async fn launch_evm_balance_collector(
    services: &mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore>,
    store: &AsyncInMemoryRunStore,
    invocation_key: &str,
) -> mfm_app::RunResponse {
    let prepared = prepare_evm_balance_launch_for_store(
        store,
        &evm_balance_config_json(),
        Some(invocation_key),
    )
    .await;
    launch_prepared(services, store, invocation_key, prepared).await
}

async fn launch_prepared(
    services: &mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore>,
    store: &AsyncInMemoryRunStore,
    invocation_key: &str,
    prepared: mfm_app::RunLaunchRequest,
) -> mfm_app::RunResponse {
    let response = services
        .launch_run_and_render(prepared)
        .await
        .unwrap_or_else(|error| panic!("{invocation_key} launch: {error:?}"));
    let run = response
        .run
        .unwrap_or_else(|| panic!("{invocation_key} expected run body"));
    if run.run_mode != mfm_app::RunModeStatus::Completed {
        let run_id = run.run_id.parse().expect("run id");
        let stream = store.load_run_stream(&run_id).await.expect("stream");
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
            "{invocation_key} mode {:?}; failures={failures:?}",
            run.run_mode
        );
    }
    run
}

fn assert_platform_holding_kind(projection: &ProjectionSnapshot, fact_kind: &str, expected: usize) {
    let count = projection
        .fact_index_entries()
        .filter(|(_claim, entry)| {
            entry.fact_kind.as_str() == fact_kind && entry.audience == FactAudience::Platform
        })
        .count();
    assert_eq!(
        count, expected,
        "expected {expected} platform facts of kind {fact_kind}"
    );
}

#[cfg(feature = "parity-tests")]
fn find_public_object(
    value: &Value,
    predicate: fn(&serde_json::Map<String, Value>) -> bool,
) -> Option<&serde_json::Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if predicate(object) {
                return Some(object);
            }
            object
                .values()
                .find_map(|child| find_public_object(child, predicate))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_public_object(child, predicate)),
        _ => None,
    }
}

#[cfg(feature = "parity-tests")]
fn is_snapshot_public_object(object: &serde_json::Map<String, Value>) -> bool {
    object.contains_key("wallets") && object.contains_key("symbol_configs")
}

#[cfg(feature = "parity-tests")]
fn is_report_public_object(object: &serde_json::Map<String, Value>) -> bool {
    object.contains_key("wallet_summaries") && object.contains_key("totals_by_quote")
}

#[cfg(feature = "parity-tests")]
fn assert_observation(
    snapshot: &serde_json::Map<String, Value>,
    symbol_id: &str,
    raw_dec: &str,
    decimals: u64,
    amount_dec: &str,
    value_dec: &str,
) {
    let observation = snapshot["wallets"]
        .as_array()
        .expect("snapshot wallets")
        .iter()
        .flat_map(|wallet| wallet["observations"].as_array().into_iter().flatten())
        .find(|observation| observation.get("symbol_id").and_then(Value::as_str) == Some(symbol_id))
        .unwrap_or_else(|| panic!("missing observation for {symbol_id}: {snapshot:?}"));
    let quantity = observation.get("quantity").expect("observation quantity");
    assert_eq!(
        quantity.get("raw_dec").and_then(Value::as_str),
        Some(raw_dec)
    );
    assert_eq!(
        quantity.get("decimals").and_then(Value::as_u64),
        Some(decimals)
    );
    assert_eq!(
        quantity.get("amount_dec").and_then(Value::as_str),
        Some(amount_dec)
    );
    let usd_value = observation["values"]
        .as_array()
        .expect("observation values")
        .iter()
        .find(|value| value.get("quote").and_then(Value::as_str) == Some("USD"))
        .expect("USD observation value");
    assert_eq!(
        usd_value.get("value_dec").and_then(Value::as_str),
        Some(value_dec)
    );
}

fn btc_balance_config_json() -> serde_json::Value {
    serde_json::json!({
        "network": "bitcoin-mainnet",
        "bitcoin_network": "main",
        "semantic_source_identity": "public-bitcoin-core",
        "addresses": [BTC_ADDRESS],
        "coverage": "configured_only",
        "max_source_reads": 1
    })
}

fn evm_balance_config_json() -> serde_json::Value {
    serde_json::json!({
        "network": "ethereum-mainnet",
        "chain_id": 1,
        "accounts": [ETH_ACCOUNT],
        "coverage": "configured_only",
        "decimals": 18,
        "max_source_reads": 1
    })
}

#[cfg(feature = "parity-tests")]
fn dual_mainnet_portfolio_json() -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "portfolio_dual_mainnet",
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
                    "subject": {
                        "kind": "evm_address",
                        "address": ETH_ACCOUNT
                    },
                    "implementation": { "kind": "address_only" },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                },
                {
                    "wallet_id": "wallet_btc",
                    "subject": {
                        "kind": "bitcoin_address",
                        "address": BTC_ADDRESS
                    },
                    "implementation": { "kind": "address_only" },
                    "network_id": "bitcoin-mainnet",
                    "symbol_ids": ["btc.native.bitcoin-mainnet"],
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
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "eth.native.ethereum-mainnet",
                            "unit_price_dec": "1800.00"
                        }]
                    },
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "btc.native.bitcoin-mainnet",
                    "display_symbol": "BTC",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "bitcoin-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "btc.native.bitcoin-mainnet",
                            "unit_price_dec": "50000.00"
                        }]
                    },
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }
    })
}

// --- mocks -------------------------------------------------------------------

#[derive(Clone)]
struct MockBtcBalanceProvider {
    chain_head_calls: Arc<Mutex<usize>>,
    balance_calls: Arc<Mutex<usize>>,
}

impl MockBtcBalanceProvider {
    fn new() -> Self {
        Self {
            chain_head_calls: Arc::new(Mutex::new(0)),
            balance_calls: Arc::new(Mutex::new(0)),
        }
    }

    fn chain_head_calls(&self) -> usize {
        *self.chain_head_calls.lock().expect("calls")
    }

    fn balance_calls(&self) -> usize {
        *self.balance_calls.lock().expect("calls")
    }
}

impl mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory for MockBtcBalanceProvider {
    fn validate_source_binding(
        &self,
        _binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<()> {
        Ok(())
    }

    fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcChainHeadReadProvider>> {
        Ok(Arc::new(BoundMockBtc {
            provider: self.clone(),
            binding,
        }))
    }

    fn bind_balance_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcBalanceReadProvider>> {
        Ok(Arc::new(BoundMockBtc {
            provider: self.clone(),
            binding,
        }))
    }
}

struct BoundMockBtc {
    provider: MockBtcBalanceProvider,
    binding: BtcSourceBinding,
}

impl BtcChainHeadReadProvider for BoundMockBtc {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move {
            *self.provider.chain_head_calls.lock().expect("calls") += 1;
            Ok(BtcChainHeadResponse {
                evidence: RedactedBtcSourceEvidence::from_binding(
                    &self.binding,
                    "main",
                    BtcSourceStatus::Synced,
                )
                .expect("evidence"),
                head_kind: request.selection().head_kind(),
                finality: request.selection().finality(),
                block_height: BTC_HEIGHT,
                block_hash: BtcBlockHash::new(BTC_HASH).expect("hash"),
                provider_time_unix_ms: Some(1_720_000_000_000),
            })
        })
    }
}

impl BtcBalanceReadProvider for BoundMockBtc {
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> BtcCapabilityFuture<'a, BtcBalanceReadResponse> {
        Box::pin(async move {
            *self.provider.balance_calls.lock().expect("calls") += 1;
            assert_eq!(request.block_height(), BTC_HEIGHT);
            assert_eq!(request.block_hash().as_str(), BTC_HASH);
            Ok(BtcBalanceReadResponse {
                evidence: RedactedBtcSourceEvidence::from_binding(
                    &self.binding,
                    "main",
                    BtcSourceStatus::Synced,
                )
                .expect("evidence"),
                address: request.address().clone(),
                balance_sats: BTC_SATS,
                block_height: request.block_height(),
                block_hash: request.block_hash().clone(),
            })
        })
    }
}

#[derive(Clone)]
struct MockEvmBalanceProvider {
    block_calls: Arc<Mutex<usize>>,
    balance_calls: Arc<Mutex<usize>>,
}

impl MockEvmBalanceProvider {
    fn new() -> Self {
        Self {
            block_calls: Arc::new(Mutex::new(0)),
            balance_calls: Arc::new(Mutex::new(0)),
        }
    }

    fn block_calls(&self) -> usize {
        *self.block_calls.lock().expect("calls")
    }

    fn balance_calls(&self) -> usize {
        *self.balance_calls.lock().expect("calls")
    }
}

impl mfm_adapters_evm::EvmProviderFactory for MockEvmBalanceProvider {
    fn validate_network_binding(
        &self,
        _binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()> {
        Ok(())
    }

    fn bind_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn mfm_adapters_evm::EvmBoundProvider>> {
        Ok(Arc::new(BoundMockEvm {
            provider: self.clone(),
            binding,
        }))
    }
}

struct BoundMockEvm {
    provider: MockEvmBalanceProvider,
    binding: EvmNetworkBinding,
}

impl BoundMockEvm {
    fn evidence(&self) -> RedactedEvmSourceEvidence {
        RedactedEvmSourceEvidence::from_binding(
            &self.binding,
            self.binding.expected_chain_id(),
            EvmSourceRef::new("primary").expect("source"),
            EvmSourcePolicyId::new("policy").expect("policy"),
        )
        .expect("evidence")
    }

    fn tip_hash(&self) -> B256 {
        B256::from_str(EVM_HASH_HEX).expect("hash")
    }
}

impl EvmBlockReadProvider for BoundMockEvm {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        Box::pin(async move {
            *self.provider.block_calls.lock().expect("calls") += 1;
            match request.block() {
                EvmBlockSelector::Latest | EvmBlockSelector::Hash(_) => Ok(EvmBlockReadResponse {
                    evidence: self.evidence(),
                    block_number: EVM_BLOCK,
                    block_hash: self.tip_hash(),
                }),
                other => panic!("unexpected block selector {other:?}"),
            }
        })
    }
}

impl EvmBalanceReadProvider for BoundMockEvm {
    fn read_balance<'a>(
        &'a self,
        request: &'a EvmBalanceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBalanceReadResponse> {
        Box::pin(async move {
            *self.provider.balance_calls.lock().expect("calls") += 1;
            assert_eq!(request.account(), Address::from_str(ETH_ACCOUNT).unwrap());
            Ok(EvmBalanceReadResponse {
                evidence: self.evidence(),
                balance_wei: U256::from(EVM_WEI),
            })
        })
    }
}
