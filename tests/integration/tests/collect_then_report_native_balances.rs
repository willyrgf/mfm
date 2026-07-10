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
use mfm_ids::RunId;
use mfm_integration_tests::test_support::{
    prepare_entry_point_launch_for_store, prepare_portfolio_launch_for_store,
    register_process_fact_capabilities, ProjectionFactIndexProvider,
};
use mfm_store::v1::{
    AsyncInMemoryRunStore, ProjectionSnapshot, RetainedArtifactReadProvider, RunEventStore,
};

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
async fn collect_then_report_completes_from_collector_written_platform_holdings() {
    let store = AsyncInMemoryRunStore::default();
    let fact_index = Arc::new(ProjectionFactIndexProvider::new(store.clone()));
    let btc = Arc::new(MockBtcBalanceProvider::new());
    let evm = Arc::new(MockEvmBalanceProvider::new());

    // 1) Collect BTC then EVM natives through one production-equivalent registry.
    let services = unified_collect_then_report_services(
        store.clone(),
        btc.clone(),
        evm.clone(),
        fact_index.clone(),
    );
    let btc_run = launch_btc_balance_collector(&services, &store, "collect-btc").await;
    assert_eq!(btc_run.run_mode, mfm_app::RunModeStatus::Completed);
    services
        .verify_replay_for_run(&btc_run.run_id.parse().expect("BTC run id"))
        .await
        .expect("replay BTC collector");

    let evm_run = launch_evm_balance_collector(&services, &store, "collect-evm").await;
    assert_eq!(evm_run.run_mode, mfm_app::RunModeStatus::Completed);
    services
        .verify_replay_for_run(&evm_run.run_id.parse().expect("EVM run id"))
        .await
        .expect("replay EVM collector");

    let projection = store.projection_snapshot().expect("after collect");
    assert_platform_holding_kind(&projection, "bitcoin.address_balance_snapshot", 1);
    assert_platform_holding_kind(&projection, "evm.address_native_balance_snapshot", 1);
    assert!(btc.chain_head_calls() >= 1 && btc.balance_calls() >= 1);
    assert!(evm.block_calls() >= 1 && evm.balance_calls() >= 1);

    // 2) Report-only portfolio_snapshot over admitted facts (no live chain in report).
    let prepared =
        prepare_portfolio_launch_for_store(&store, &dual_mainnet_portfolio_json(), None).await;
    let report = services
        .launch_prepared_entry_point_run(prepared)
        .await
        .unwrap_or_else(|error| panic!("portfolio launch: {error:?}"));
    let launch = report.run.expect("run body");
    if launch.run_mode != mfm_app::RunModeStatus::Completed {
        let run_id = launch.run_id.parse::<RunId>().expect("run id");
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
            "report must complete after collectors; mode={:?}; failures={failures:?}",
            launch.run_mode
        );
    }

    let public_output = report
        .public_output
        .expect("completed report returns public output");
    let json = public_output
        .json
        .expect("public output json")
        .as_object()
        .expect("object")
        .clone();
    assert!(
        json.contains_key("snapshot") || json.values().any(|v| v.get("network_pins").is_some()),
        "public output should carry snapshot material: {json:?}"
    );
    services
        .verify_replay_for_run(&launch.run_id.parse().expect("report run id"))
        .await
        .expect("replay portfolio report");
}

// --- services / launch -------------------------------------------------------

fn btc_collector_services(
    store: AsyncInMemoryRunStore,
    btc: Arc<MockBtcBalanceProvider>,
    fact_index: Arc<ProjectionFactIndexProvider>,
) -> mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore> {
    let receipt_trust_root = fact_index.receipt_trust_root();
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
    mfm_app::RunServices::new_with_certification_registry_and_fact_query_trust_root(
        mfm_runtime::SerialTypedScheduler::new(runners, Arc::new(store.clone())),
        store.clone(),
        store,
        mfm_app::production_certification_registry().expect("cert"),
        Some(receipt_trust_root),
    )
}

fn evm_collector_services(
    store: AsyncInMemoryRunStore,
    evm: Arc<MockEvmBalanceProvider>,
    fact_index: Arc<ProjectionFactIndexProvider>,
) -> mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore> {
    let receipt_trust_root = fact_index.receipt_trust_root();
    let artifacts: Arc<dyn RetainedArtifactReadProvider> = Arc::new(store.clone());
    let mut runners = mfm_runtime::ErasedRunnerRegistry::new();
    register_process_fact_capabilities(&mut runners, fact_index.as_ref())
        .expect("process fact capabilities");
    mfm_adapters_evm::register_evm_collectors_runners(
        &mut runners,
        mfm_adapters_evm::EvmRunnerCapabilities::new(artifacts, evm),
    )
    .expect("evm runners");
    mfm_app::RunServices::new_with_certification_registry_and_fact_query_trust_root(
        mfm_runtime::SerialTypedScheduler::new(runners, Arc::new(store.clone())),
        store.clone(),
        store,
        mfm_app::production_certification_registry().expect("cert"),
        Some(receipt_trust_root),
    )
}

fn unified_collect_then_report_services(
    store: AsyncInMemoryRunStore,
    btc: Arc<MockBtcBalanceProvider>,
    evm: Arc<MockEvmBalanceProvider>,
    fact_index: Arc<ProjectionFactIndexProvider>,
) -> mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore> {
    let receipt_trust_root = fact_index.receipt_trust_root();
    let artifacts: Arc<dyn RetainedArtifactReadProvider> = Arc::new(store.clone());
    let mut runners = mfm_runtime::ErasedRunnerRegistry::new();
    register_process_fact_capabilities(&mut runners, fact_index.as_ref())
        .expect("process fact capabilities");
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(
        &mut runners,
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(
            artifacts.clone(),
            btc,
            fact_index.clone(),
        ),
    )
    .expect("btc runners");
    mfm_adapters_evm::register_evm_collectors_runners(
        &mut runners,
        mfm_adapters_evm::EvmRunnerCapabilities::new(artifacts.clone(), evm),
    )
    .expect("evm runners");
    mfm_adapters_portfolio::register_portfolio_runners(
        &mut runners,
        mfm_adapters_portfolio::PortfolioRunnerCapabilities::new(artifacts, fact_index),
    )
    .expect("portfolio runners");
    mfm_app::RunServices::new_with_certification_registry_and_fact_query_trust_root(
        mfm_runtime::SerialTypedScheduler::new(runners, Arc::new(store.clone())),
        store.clone(),
        store,
        mfm_app::production_certification_registry().expect("cert"),
        Some(receipt_trust_root),
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
                    "decimals": 18,
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
                    "decimals": 8,
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
