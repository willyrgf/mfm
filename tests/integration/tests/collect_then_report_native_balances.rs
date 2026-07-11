//! Collect-then-report e2e: balance collectors write Platform holdings, then report succeeds.
//!
//! Highest-value operator path after cutover:
//!   mock-transport collect (BTC + EVM natives, shared joint tip) → portfolio_snapshot Completes
//! without seed helpers and without live chain crawl in the report graph.
//!
//! The test composes all collector and portfolio runners in one registry with process-owned
//! fact capabilities, matching the production assembly boundary.

#![allow(clippy::disallowed_methods)]

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, B256, U256};
#[cfg(feature = "parity-tests")]
use assert_cmd::Command;
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
    prepare_entry_point_launch_for_store, register_process_fact_capabilities,
    ProjectionFactIndexProvider,
};
use mfm_store::v1::{
    AsyncInMemoryRunStore, ProjectionSnapshot, RetainedArtifactReadProvider, RunEventStore,
};
#[cfg(feature = "parity-tests")]
use serde_json::Value;
#[cfg(feature = "parity-tests")]
use std::path::Path;
#[cfg(feature = "parity-tests")]
use std::process::Output;

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
    drop(store);

    let rpc_url = start_collectors_rpc_mock().await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path =
        write_collectors_runtime_config_for_test(runtime_config_dir.path(), &rpc_url);
    let config_dir = tempfile::tempdir().expect("collector config tempdir");
    let btc_config_path =
        write_json_config(config_dir.path(), "btc.json", btc_balance_config_json());
    let evm_config_path =
        write_json_config(config_dir.path(), "evm.json", evm_balance_config_json());
    let portfolio_config_path = write_json_config(
        config_dir.path(),
        "portfolio.json",
        dual_mainnet_portfolio_json(),
    );

    // 1) Collect BTC then EVM natives through the actual CLI and production Postgres assembly.
    let btc = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "start".to_owned(),
        "--op".to_owned(),
        "btc_address_balance".to_owned(),
        "--config".to_owned(),
        path_arg(&btc_config_path),
        "--config-format".to_owned(),
        "json".to_owned(),
        "--runtime-config".to_owned(),
        path_arg(&runtime_config_path),
        "--database-url".to_owned(),
        scoped_database_url.clone(),
    ]);
    assert_success(&btc);
    let btc_data = parse_success_json(&btc.stdout);
    assert_eq!(btc_data["outcome"], "admitted");
    assert_eq!(btc_data["run"]["run_mode"], "completed");
    let btc_run_id = run_id_from_start(&btc_data);

    let evm = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "start".to_owned(),
        "--op".to_owned(),
        "evm_native_balance".to_owned(),
        "--config".to_owned(),
        path_arg(&evm_config_path),
        "--config-format".to_owned(),
        "json".to_owned(),
        "--runtime-config".to_owned(),
        path_arg(&runtime_config_path),
        "--database-url".to_owned(),
        scoped_database_url.clone(),
    ]);
    assert_success(&evm);
    let evm_data = parse_success_json(&evm.stdout);
    assert_eq!(evm_data["outcome"], "admitted");
    assert_eq!(evm_data["run"]["run_mode"], "completed");
    let evm_run_id = run_id_from_start(&evm_data);

    // 2) Report only over the collector-written Platform facts. The process receives no runtime
    // config, so a successful report proves it did not perform a live chain crawl.
    let report = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "start".to_owned(),
        "--op".to_owned(),
        "portfolio_snapshot".to_owned(),
        "--config".to_owned(),
        path_arg(&portfolio_config_path),
        "--config-format".to_owned(),
        "json".to_owned(),
        "--database-url".to_owned(),
        scoped_database_url.clone(),
    ]);
    assert_success(&report);
    let report_data = parse_success_json(&report.stdout);
    assert_eq!(report_data["outcome"], "admitted");
    assert_eq!(report_data["run"]["run_mode"], "completed");
    let report_run_id = run_id_from_start(&report_data);
    let json = &report_data["public_output"]["json"];
    let snapshot = find_public_object(json, is_snapshot_public_object)
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
    let report = find_public_object(json, is_report_public_object)
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
    // 3) Replay all three persisted runs through the read-only CLI path.
    for run_id in [btc_run_id, evm_run_id, report_run_id] {
        let replay = run_cli(&[
            "--output-format".to_owned(),
            "json".to_owned(),
            "run".to_owned(),
            "replay".to_owned(),
            run_id.clone(),
            "--database-url".to_owned(),
            scoped_database_url.clone(),
        ]);
        assert_success(&replay);
        let replay_data = parse_success_json(&replay.stdout);
        assert_eq!(replay_data["run_id"].as_str(), Some(run_id.as_str()));
        assert_eq!(replay_data["run_mode"], "completed");
        assert!(replay_data["retained_artifacts"].as_u64().unwrap_or(0) > 0);
    }

    drop_postgres_schema(&database_url, &schema).await;
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
    let prepared = prepare_entry_point_launch_for_store(
        store,
        "btc_address_balance",
        None,
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
    let prepared = prepare_entry_point_launch_for_store(
        store,
        "evm_native_balance",
        None,
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
    prepared: mfm_app::PreparedEntryPointRunLaunch,
) -> mfm_app::RunResponse {
    let response = services
        .launch_prepared_entry_point_run(prepared)
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
fn write_json_config(dir: &Path, name: &str, value: Value) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("config json"),
    )
    .expect("write config");
    path
}

#[cfg(feature = "parity-tests")]
fn path_arg(path: &Path) -> String {
    path.to_str().expect("utf-8 test path").to_owned()
}

#[cfg(feature = "parity-tests")]
fn run_cli(args: &[String]) -> Output {
    let mut command = Command::cargo_bin("mfm_cli").expect("mfm_cli binary");
    command
        .env_remove("MFM_RUNTIME_CONFIG_FILE")
        .env_remove("LOG_LEVEL")
        .env_remove("RUST_LOG")
        .env_remove("LOG_FORMAT")
        .env_remove("LOG_SPAN_EVENTS")
        .args(args)
        .output()
        .expect("execute mfm_cli")
}

#[cfg(feature = "parity-tests")]
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "mfm_cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(feature = "parity-tests")]
fn parse_success_json(stdout: &[u8]) -> Value {
    let response: Value = serde_json::from_slice(stdout).expect("CLI stdout must be JSON");
    assert_eq!(response["status"], "success", "CLI response: {response}");
    response["data"].clone()
}

#[cfg(feature = "parity-tests")]
fn run_id_from_start(data: &Value) -> String {
    data["run"]["run_id"]
        .as_str()
        .expect("start response run id")
        .to_owned()
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
