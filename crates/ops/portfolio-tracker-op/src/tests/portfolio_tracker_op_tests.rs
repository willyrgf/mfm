use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError, RunError};
use mfm_machine::events::{event_envelopes_from_stream_records, DomainEvent};
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{ArtifactId, ErrorCode, StateId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{FactIndex, LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::plan::{StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::replay_io::ReplayIo;
use mfm_machine::runtime::{DefaultExecutionEngine, EngineFailpoints};
use mfm_machine::stores::StreamId;
use mfm_sdk::unstable::SdkPlanResolver;
use mfm_state_common::test_support as op_test_support;
use mfm_state_portfolio::model::{PortfolioQuoteTotal, PortfolioReport, PortfolioSnapshot};
use tokio::sync::Mutex;

const ERC20_DECIMALS_SELECTOR: &str = "0x313ce567";
const LATEST_ROUND_DATA_SELECTOR: &str = "0xfeaf968c";
const BALANCE_OF_SELECTOR_PREFIX: &str = "0x70a08231";

struct Harness {
    engine: Arc<dyn ExecutionEngine>,
    registry: Arc<dyn mfm_sdk::op::OperationRegistry>,
    planner: Arc<dyn mfm_sdk::pipeline::PipelinePlanner>,
    pipeline: mfm_sdk::pipeline::Pipeline,
    stores: Stores,
    cfg: mfm_machine::config::RunConfig,
}

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

#[derive(Clone)]
struct CountingTransportFactory {
    counts: Arc<Mutex<HashMap<String, u64>>>,
}

impl CountingTransportFactory {
    fn new(counts: Arc<Mutex<HashMap<String, u64>>>) -> Self {
        Self { counts }
    }
}

impl LiveIoTransportFactory for CountingTransportFactory {
    fn namespace_group(&self) -> &str {
        "rpc.control"
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(CountingTransport {
            counts: Arc::clone(&self.counts),
        })
    }
}

struct CountingTransport {
    counts: Arc<Mutex<HashMap<String, u64>>>,
}

#[async_trait]
impl LiveIoTransport for CountingTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        let method = call
            .request
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let kind = call
            .request
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let network_id = call
            .request
            .get("network_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("default");

        let params = call
            .request
            .get("params")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        let count_key = match kind {
            "prepare_sources" => format!("prepare_sources@{network_id}"),
            _ => classify_call(method, network_id, &params),
        };
        {
            let mut counts = self.counts.lock().await;
            *counts.entry(count_key).or_default() += 1;
        }

        if kind == "prepare_sources" {
            let network_id = call
                .request
                .get("network_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let control_scope = call
                .request
                .get("control_scope")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("shared");
            return Ok(serde_json::json!({
                "control_scope": control_scope,
                "network_id": network_id,
                "pool_kind": "test",
                "available_source_ids": ["source-1"],
                "ranked_source_ids": ["source-1"],
                "sources": [{
                    "source_id": "source-1",
                    "healthy": true
                }]
            }));
        }

        match method {
            "eth_chainId" => Ok(match network_id {
                "arbitrum-mainnet" => serde_json::json!("0xa4b1"),
                _ => serde_json::json!("0x1"),
            }),
            "eth_blockNumber" => Ok(match network_id {
                "arbitrum-mainnet" => serde_json::json!("0xc8"),
                _ => serde_json::json!("0x64"),
            }),
            "eth_getBalance" => match params.first().and_then(serde_json::Value::as_str) {
                Some("0x000000000000000000000000000000000000dead") => {
                    Ok(serde_json::json!("0x0de0b6b3a7640000"))
                }
                Some("0x000000000000000000000000000000000000beef") => {
                    Ok(serde_json::json!("0x1bc16d674ec80000"))
                }
                _ => Err(IoError::Other(info(
                    "unexpected_balance_call",
                    ErrorCategory::Unknown,
                    "unexpected eth_getBalance call",
                ))),
            },
            "eth_call" => {
                let target = params
                    .first()
                    .and_then(serde_json::Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let to = target
                    .get("to")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let data = target
                    .get("data")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();

                match (to, data) {
                    ("0x0000000000000000000000000000000000000001", d)
                        if d.starts_with(BALANCE_OF_SELECTOR_PREFIX) =>
                    {
                        Ok(ok_u256_as_32byte_hex(10_000_000))
                    }
                    ("0x0000000000000000000000000000000000000001", ERC20_DECIMALS_SELECTOR) => {
                        Ok(ok_u256_as_32byte_hex(6))
                    }
                    ("0x0000000000000000000000000000000000001001", ERC20_DECIMALS_SELECTOR)
                    | ("0x0000000000000000000000000000000000001002", ERC20_DECIMALS_SELECTOR)
                    | ("0x0000000000000000000000000000000000001003", ERC20_DECIMALS_SELECTOR) => {
                        Ok(ok_u256_as_32byte_hex(8))
                    }
                    ("0x0000000000000000000000000000000000001001", LATEST_ROUND_DATA_SELECTOR) => {
                        Ok(chainlink_round_data_hex(200_000_000_000))
                    }
                    ("0x0000000000000000000000000000000000001002", LATEST_ROUND_DATA_SELECTOR) => {
                        Ok(chainlink_round_data_hex(2_000_000_000_000))
                    }
                    ("0x0000000000000000000000000000000000001003", LATEST_ROUND_DATA_SELECTOR) => {
                        Ok(chainlink_round_data_hex(100_000_000))
                    }
                    ("0x0000000000000000000000000000000000002001", d)
                        if d.starts_with(BALANCE_OF_SELECTOR_PREFIX) =>
                    {
                        Ok(ok_u256_as_32byte_hex(1_500_000))
                    }
                    ("0x0000000000000000000000000000000000002001", ERC20_DECIMALS_SELECTOR)
                    | ("0x0000000000000000000000000000000000002002", ERC20_DECIMALS_SELECTOR) => {
                        Ok(ok_u256_as_32byte_hex(6))
                    }
                    ("0x0000000000000000000000000000000000002002", d)
                        if d.starts_with(BALANCE_OF_SELECTOR_PREFIX) =>
                    {
                        Ok(ok_u256_as_32byte_hex(750_000))
                    }
                    ("0x0000000000000000000000000000000000002003", d)
                        if d.starts_with(BALANCE_OF_SELECTOR_PREFIX) =>
                    {
                        Ok(ok_u256_as_32byte_hex(200_000_000))
                    }
                    ("0x0000000000000000000000000000000000002003", ERC20_DECIMALS_SELECTOR) => {
                        Ok(ok_u256_as_32byte_hex(8))
                    }
                    _ => Err(IoError::Other(info(
                        "unexpected_eth_call",
                        ErrorCategory::Unknown,
                        "unexpected eth_call payload",
                    ))),
                }
            }
            _ => Err(IoError::Other(info(
                "unexpected_method",
                ErrorCategory::Unknown,
                "unexpected json-rpc method",
            ))),
        }
    }
}

fn classify_call(method: &str, network_id: &str, params: &[serde_json::Value]) -> String {
    match method {
        "eth_chainId" | "eth_blockNumber" => format!("{method}@{network_id}"),
        "eth_getBalance" => {
            let wallet = params
                .first()
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            format!("{method}:{wallet}@{network_id}")
        }
        "eth_call" => {
            let target = params
                .first()
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();
            let to = target
                .get("to")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let data = target
                .get("data")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let label = if data == ERC20_DECIMALS_SELECTOR {
                "decimals"
            } else if data == LATEST_ROUND_DATA_SELECTOR {
                "latest_round_data"
            } else if data.starts_with(BALANCE_OF_SELECTOR_PREFIX) {
                "balance_of"
            } else {
                "unknown"
            };
            format!("eth_call:{label}:{to}@{network_id}")
        }
        _ => format!("{method}@{network_id}"),
    }
}

fn ok_u256_as_32byte_hex(n: u64) -> serde_json::Value {
    let hex_val = format!("{:x}", n);
    let padded = format!("{}{}", "0".repeat(64 - hex_val.len()), hex_val);
    serde_json::json!(format!("0x{padded}"))
}

fn chainlink_round_data_hex(answer: u64) -> serde_json::Value {
    let answer_hex = format!("{answer:064x}");
    serde_json::json!(format!(
        "0x\
0000000000000000000000000000000000000000000000000000000000000001\
{answer_hex}\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001"
    ))
}

fn build_harness(
    op_config: serde_json::Value,
    evm_factory: Arc<dyn LiveIoTransportFactory>,
    failpoints: Option<EngineFailpoints>,
) -> Harness {
    let op: mfm_sdk::op::DynOperation = Arc::new(PortfolioTrackerOp);
    let (registry, planner, pipeline) =
        op_test_support::single_op_plan(op, op_config).expect("pipeline");
    let stores = op_test_support::in_memory_stores();

    let resolver = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));
    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(
        RouterLiveIoTransportFactory::from_factories(vec![evm_factory]).expect("router factories"),
    );

    let mut engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);
    if let Some(failpoints) = failpoints {
        engine = engine.with_failpoints(failpoints);
    }

    Harness {
        engine: Arc::new(engine),
        registry,
        planner,
        pipeline,
        stores,
        cfg: op_test_support::run_config_live(),
    }
}

async fn load_context_snapshot(stores: &Stores, snapshot_id: &ArtifactId) -> serde_json::Value {
    let bytes = stores
        .artifacts
        .get(snapshot_id)
        .await
        .expect("snapshot bytes");
    serde_json::from_slice(&bytes).expect("snapshot json")
}

fn read_required_context_value(
    snapshot: &serde_json::Value,
    key: &ContextKey,
) -> serde_json::Value {
    snapshot.get(&key.0).cloned().unwrap_or_else(|| {
        let keys = snapshot
            .as_object()
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        panic!("missing context key `{}`; keys={keys:?}", key.0)
    })
}

async fn load_snapshot_artifact(
    stores: &Stores,
    context_snapshot: &serde_json::Value,
) -> PortfolioSnapshot {
    let artifact_id: String = serde_json::from_value(read_required_context_value(
        context_snapshot,
        &portfolio_snapshot_artifact_id_context_key(),
    ))
    .expect("artifact id");
    let bytes = stores
        .artifacts
        .get(&ArtifactId(artifact_id))
        .await
        .expect("portfolio snapshot artifact");
    serde_json::from_slice(&bytes).expect("portfolio snapshot artifact json")
}

fn topo(graph: &StateGraph) -> Vec<&StateNode> {
    let mut in_deg: HashMap<&StateId, usize> = HashMap::new();
    for state in &graph.states {
        in_deg.insert(&state.id, 0);
    }
    for edge in &graph.edges {
        *in_deg.get_mut(&edge.to).expect("to") += 1;
    }

    let mut ready: Vec<&StateId> = in_deg
        .iter()
        .filter_map(|(id, deg)| (*deg == 0).then_some(*id))
        .collect();
    ready.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    let mut out = Vec::new();
    while let Some(id) = ready.pop() {
        let node = graph
            .states
            .iter()
            .find(|node| &node.id == id)
            .expect("node");
        out.push(node);

        let mut next_ids: Vec<&StateId> = graph
            .edges
            .iter()
            .filter(|edge| &edge.from == id)
            .map(|edge| &edge.to)
            .collect();
        next_ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        for next in next_ids {
            let deg = in_deg.get_mut(next).expect("deg");
            *deg -= 1;
            if *deg == 0 {
                ready.push(next);
                ready.sort_by(|left, right| left.as_str().cmp(right.as_str()));
            }
        }
    }

    out
}

struct NoopRecorder;

#[async_trait]
impl EventRecorder for NoopRecorder {
    async fn emit(&mut self, _event: DomainEvent) -> Result<(), RunError> {
        Ok(())
    }

    async fn emit_many(&mut self, _events: Vec<DomainEvent>) -> Result<(), RunError> {
        Ok(())
    }
}

#[test]
fn expand_uses_canonical_multi_network_graph() {
    let op = PortfolioTrackerOp;
    let graph = op
        .expand(
            OpPath("portfolio_tracker.main".to_string()),
            &canonical_op_config(),
            &op_test_support::run_config_live(),
        )
        .expect("expand");

    let ids: Vec<_> = graph
        .states
        .iter()
        .map(|state| state.id.as_str().to_string())
        .collect();
    assert_eq!(
        ids,
        vec![
            "portfolio_tracker.main.prepare_sources",
            "portfolio_tracker.main.pin_networks",
            "portfolio_tracker.main.resolve_wallets",
            "portfolio_tracker.main.read_direct_prices",
            "portfolio_tracker.main.collect_observations",
            "portfolio_tracker.main.collect_aave_observations",
            "portfolio_tracker.main.merge_observations",
            "portfolio_tracker.main.write_snapshot",
            "portfolio_tracker.main.write_report",
        ]
    );

    let edges: Vec<_> = graph
        .edges
        .iter()
        .map(|edge| (edge.from.as_str().to_string(), edge.to.as_str().to_string()))
        .collect();
    assert!(edges.contains(&(
        "portfolio_tracker.main.pin_networks".to_string(),
        "portfolio_tracker.main.resolve_wallets".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.prepare_sources".to_string(),
        "portfolio_tracker.main.pin_networks".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.pin_networks".to_string(),
        "portfolio_tracker.main.read_direct_prices".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.resolve_wallets".to_string(),
        "portfolio_tracker.main.collect_observations".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.resolve_wallets".to_string(),
        "portfolio_tracker.main.collect_aave_observations".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.read_direct_prices".to_string(),
        "portfolio_tracker.main.collect_observations".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.read_direct_prices".to_string(),
        "portfolio_tracker.main.collect_aave_observations".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.collect_aave_observations".to_string(),
        "portfolio_tracker.main.merge_observations".to_string()
    )));
    assert!(edges.contains(&(
        "portfolio_tracker.main.collect_observations".to_string(),
        "portfolio_tracker.main.merge_observations".to_string()
    )));
}

#[tokio::test]
async fn at_live_then_replay_determinism() {
    let counts = Arc::new(Mutex::new(HashMap::new()));
    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));
    let Harness {
        engine,
        registry,
        planner,
        pipeline,
        stores,
        cfg,
    } = build_harness(canonical_op_config(), factory, None);

    let res = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline.clone(),
        cfg.clone(),
    )
    .await
    .expect("start");

    assert_eq!(res.phase, RunPhase::Completed);
    let final_snapshot_id = res.final_snapshot_id.clone().expect("final snapshot");

    let context_snapshot = load_context_snapshot(&stores, &final_snapshot_id).await;
    let report: PortfolioReport = serde_json::from_value(read_required_context_value(
        &context_snapshot,
        &portfolio_snapshot_report_context_key(),
    ))
    .expect("typed report");
    assert_eq!(report.portfolio_id, "portfolio_main");
    assert_eq!(report.error_count, 0);
    assert_eq!(report.wallet_summaries.len(), 2);
    let portfolio_usd = find_quote_total(
        &report.totals_by_quote,
        mfm_state_symbol::model::QuoteCode::Usd,
    );
    assert_eq!(portfolio_usd.assets_value_dec, "6010.000000000000000000");
    assert_eq!(portfolio_usd.net_value_dec, "6010.000000000000000000");
    let portfolio_btc = find_quote_total(
        &report.totals_by_quote,
        mfm_state_symbol::model::QuoteCode::Btc,
    );
    assert_eq!(portfolio_btc.assets_value_dec, "0.300500000000000000");
    assert_eq!(portfolio_btc.net_value_dec, "0.300500000000000000");

    let snapshot = load_snapshot_artifact(&stores, &context_snapshot).await;
    assert_eq!(snapshot.portfolio_id, "portfolio_main");
    assert_eq!(snapshot.network_pins.len(), 2);
    assert_eq!(snapshot.wallets.len(), 2);
    assert_eq!(snapshot.wallets[0].wallet_id, "wallet_ops_arb");
    assert_eq!(snapshot.wallets[1].wallet_id, "wallet_treasury_eth");
    assert_eq!(snapshot.wallets[1].observations.len(), 2);

    let stream = stores
        .streams
        .read_range(&StreamId::run(res.run_id), 1, None)
        .await
        .and_then(|records| event_envelopes_from_stream_records(res.run_id, records))
        .expect("read_range");
    let facts = FactIndex::from_event_stream(&stream);
    let (_manifest_id, initial_snapshot_id) = op_test_support::run_started(&stream);

    let bytes = stores
        .artifacts
        .get(&initial_snapshot_id)
        .await
        .expect("initial snapshot");
    let initial = serde_json::from_slice::<serde_json::Value>(&bytes).expect("initial snapshot");
    let mut ctx = op_test_support::MapContext::from_snapshot(initial);

    let plan = planner
        .build_execution_plan(Arc::clone(&registry), &pipeline, &cfg)
        .expect("plan");
    for node in topo(&plan.graph) {
        let mut io = ReplayIo::new(
            res.run_id,
            node.id.clone(),
            0,
            Arc::clone(&stores.artifacts),
            facts.clone(),
            false,
        );
        let mut rec = NoopRecorder;
        node.state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("handle");
    }

    let computed = artifact_id_for_json(&ctx.dump().expect("dump")).expect("hash");
    assert_eq!(computed, final_snapshot_id);

    let got = counts.lock().await.clone();
    assert_eq!(
        got.get("eth_chainId@ethereum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get("eth_chainId@arbitrum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get("eth_blockNumber@ethereum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get("eth_blockNumber@arbitrum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get(
            "eth_call:latest_round_data:0x0000000000000000000000000000000000001001@ethereum-mainnet"
        )
        .copied()
        .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get(
            "eth_call:latest_round_data:0x0000000000000000000000000000000000001002@ethereum-mainnet"
        )
        .copied()
        .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get(
            "eth_call:latest_round_data:0x0000000000000000000000000000000000001003@ethereum-mainnet"
        )
        .copied()
        .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get("eth_call:balance_of:0x0000000000000000000000000000000000000001@ethereum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
}

#[tokio::test]
async fn at_crash_resume_orphan_attempt_reuses_pinned_network_facts() {
    let counts = Arc::new(Mutex::new(HashMap::new()));
    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

    let failpoints = EngineFailpoints::default();
    failpoints
        .stop_after_handler_once
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let Harness {
        engine,
        registry,
        planner,
        pipeline,
        stores,
        cfg,
    } = build_harness(canonical_op_config(), factory, Some(failpoints.clone()));

    let first = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline.clone(),
        cfg.clone(),
    )
    .await
    .expect("start");

    assert_eq!(first.phase, RunPhase::Running);

    let resumed = op_test_support::resume_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        first.run_id,
    )
    .await
    .expect("resume");

    assert_eq!(resumed.phase, RunPhase::Completed);

    let context_snapshot = load_context_snapshot(
        &stores,
        &resumed.final_snapshot_id.clone().expect("final snapshot"),
    )
    .await;
    let report: PortfolioReport = serde_json::from_value(read_required_context_value(
        &context_snapshot,
        &portfolio_snapshot_report_context_key(),
    ))
    .expect("typed report");
    assert_eq!(report.error_count, 0);
    let portfolio_usd = find_quote_total(
        &report.totals_by_quote,
        mfm_state_symbol::model::QuoteCode::Usd,
    );
    assert_eq!(portfolio_usd.assets_value_dec, "6010.000000000000000000");
    assert_eq!(portfolio_usd.net_value_dec, "6010.000000000000000000");
    let snapshot = load_snapshot_artifact(&stores, &context_snapshot).await;
    assert_eq!(snapshot.wallets.len(), 2);

    let got = counts.lock().await.clone();
    assert_eq!(
        got.get("eth_chainId@ethereum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get("eth_chainId@arbitrum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get("eth_blockNumber@ethereum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
    assert_eq!(
        got.get("eth_blockNumber@arbitrum-mainnet")
            .copied()
            .unwrap_or(0),
        1
    );
}

#[tokio::test]
async fn collects_aave_protocol_positions_through_the_op_boundary() {
    let counts = Arc::new(Mutex::new(HashMap::new()));
    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));
    let Harness {
        engine,
        registry,
        planner,
        pipeline,
        stores,
        cfg,
    } = build_harness(canonical_aave_op_config(), factory, None);

    let res = op_test_support::start_pipeline_with_defaults(
        Arc::clone(&engine),
        &stores,
        Arc::clone(&registry),
        Arc::clone(&planner),
        pipeline,
        cfg,
    )
    .await
    .expect("start");

    assert_eq!(res.phase, RunPhase::Completed);
    let final_snapshot_id = res.final_snapshot_id.expect("final snapshot");
    let context_snapshot = load_context_snapshot(&stores, &final_snapshot_id).await;
    let report: PortfolioReport = serde_json::from_value(read_required_context_value(
        &context_snapshot,
        &portfolio_snapshot_report_context_key(),
    ))
    .expect("typed report");
    assert_eq!(report.portfolio_id, "portfolio_aave");
    assert_eq!(report.error_count, 0);
    let supplier_usd = find_quote_total(
        &report
            .wallet_summaries
            .iter()
            .find(|wallet| wallet.wallet_id == "wallet_supplier")
            .expect("supplier wallet report")
            .totals_by_quote,
        mfm_state_symbol::model::QuoteCode::Usd,
    );
    assert_eq!(supplier_usd.assets_value_dec, "1.500000");
    assert_eq!(supplier_usd.net_value_dec, "1.500000");
    let borrower_usd = find_quote_total(
        &report
            .wallet_summaries
            .iter()
            .find(|wallet| wallet.wallet_id == "wallet_borrower")
            .expect("borrower wallet report")
            .totals_by_quote,
        mfm_state_symbol::model::QuoteCode::Usd,
    );
    assert_eq!(borrower_usd.collateral_value_dec, "140000.00000000");
    assert_eq!(borrower_usd.debt_value_dec, "0.750000");
    assert_eq!(borrower_usd.net_value_dec, "139999.25000000");
    let portfolio_usd = find_quote_total(
        &report.totals_by_quote,
        mfm_state_symbol::model::QuoteCode::Usd,
    );
    assert_eq!(portfolio_usd.assets_value_dec, "1.500000");
    assert_eq!(portfolio_usd.collateral_value_dec, "140000.00000000");
    assert_eq!(portfolio_usd.debt_value_dec, "0.750000");
    assert_eq!(portfolio_usd.net_value_dec, "140000.75000000");

    let snapshot = load_snapshot_artifact(&stores, &context_snapshot).await;
    assert_eq!(snapshot.wallets.len(), 2);

    let supplier = snapshot
        .wallets
        .iter()
        .find(|wallet| wallet.wallet_id == "wallet_supplier")
        .expect("supplier wallet");
    assert_eq!(supplier.observations.len(), 1);
    assert_eq!(
        supplier.observations[0].source.balance_reader_kind,
        "protocol_position:aave_v3:reserve_position"
    );
    assert_eq!(supplier.observations[0].quantity.raw_dec, "1500000");

    let borrower = snapshot
        .wallets
        .iter()
        .find(|wallet| wallet.wallet_id == "wallet_borrower")
        .expect("borrower wallet");
    assert_eq!(borrower.observations.len(), 2);
    let debt = borrower
        .observations
        .iter()
        .find(|observation| observation.symbol_id == "aave_v3.usdc.debt.ethereum-mainnet")
        .expect("debt observation");
    assert_eq!(debt.role, mfm_state_symbol::model::SymbolRole::Debt);
    assert_eq!(
        debt.metadata.get("debt_kind"),
        Some(&serde_json::json!("variable"))
    );
    assert_eq!(
        debt.source.balance_reader_kind,
        "protocol_position:aave_v3:debt_position"
    );
    assert_eq!(
        debt.values[0].priced_symbol_id,
        "usdc.wallet.ethereum-mainnet"
    );

    let collateral = borrower
        .observations
        .iter()
        .find(|observation| observation.symbol_id == "aave_v3.wbtc.collateral.ethereum-mainnet")
        .expect("collateral observation");
    assert_eq!(
        collateral.source.balance_reader_kind,
        "protocol_position:aave_v3:reserve_position"
    );
    assert_eq!(collateral.quantity.raw_dec, "200000000");
    assert_eq!(
        collateral.values[0].priced_symbol_id,
        "wbtc.wallet.ethereum-mainnet"
    );
}

fn find_quote_total(
    totals: &[PortfolioQuoteTotal],
    quote: mfm_state_symbol::model::QuoteCode,
) -> PortfolioQuoteTotal {
    totals
        .iter()
        .find(|total| total.quote == quote)
        .cloned()
        .unwrap_or_else(|| panic!("missing quote total for {}", quote))
}

fn canonical_op_config() -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD", "BTC"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1,
                    "metadata": {}
                },
                {
                    "network_id": "arbitrum-mainnet",
                    "chain_id": 42161,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_treasury_eth",
                    "address": "0x000000000000000000000000000000000000dead",
                    "network_id": "ethereum-mainnet",
                    "implementation": { "kind": "address_only" },
                    "symbol_ids": [
                        "eth.native.ethereum-mainnet",
                        "usdc.wallet.ethereum-mainnet"
                    ],
                    "metadata": {}
                },
                {
                    "wallet_id": "wallet_ops_arb",
                    "address": "0x000000000000000000000000000000000000beef",
                    "network_id": "arbitrum-mainnet",
                    "implementation": { "kind": "address_only" },
                    "symbol_ids": ["eth.native.arbitrum-mainnet"],
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
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_eth_usd_mainnet",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "eth.native.ethereum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "0.10000000"
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "usdc.wallet.ethereum-mainnet",
                    "display_symbol": "USDC",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": "0x0000000000000000000000000000000000000001"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_usdc_usd_mainnet",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "usdc.wallet.ethereum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "derived_unit_price",
                                    "numerator": {
                                        "source_id": "chainlink_usdc_usd_mainnet",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "usdc.wallet.ethereum-mainnet",
                                        "quote": "USD"
                                    },
                                    "denominator": {
                                        "source_id": "chainlink_btc_usd_mainnet",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "btc.wallet.ethereum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            }
                        ]
                    },
                    "decimals": 6,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "eth.native.arbitrum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "arbitrum-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.arbitrum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "2000.00000000"
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "eth.native.arbitrum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "0.10000000"
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        },
        "valuation_source_registry": {
            "sources": [
                {
                    "source_id": "chainlink_eth_usd_mainnet",
                    "network_id": "ethereum-mainnet",
                    "base_symbol_id": "eth.native.ethereum-mainnet",
                    "quote": "USD",
                    "reader": {
                        "kind": "evm_oracle",
                        "oracle_kind": "chainlink_aggregator_v3",
                        "config": {
                            "contract_address": "0x0000000000000000000000000000000000001001"
                        }
                    },
                    "metadata": {}
                },
                {
                    "source_id": "chainlink_btc_usd_mainnet",
                    "network_id": "ethereum-mainnet",
                    "base_symbol_id": "btc.wallet.ethereum-mainnet",
                    "quote": "USD",
                    "reader": {
                        "kind": "evm_oracle",
                        "oracle_kind": "chainlink_aggregator_v3",
                        "config": {
                            "contract_address": "0x0000000000000000000000000000000000001002"
                        }
                    },
                    "metadata": {}
                },
                {
                    "source_id": "chainlink_usdc_usd_mainnet",
                    "network_id": "ethereum-mainnet",
                    "base_symbol_id": "usdc.wallet.ethereum-mainnet",
                    "quote": "USD",
                    "reader": {
                        "kind": "evm_oracle",
                        "oracle_kind": "chainlink_aggregator_v3",
                        "config": {
                            "contract_address": "0x0000000000000000000000000000000000001003"
                        }
                    },
                    "metadata": {}
                }
            ]
        }
    })
}

fn canonical_aave_op_config() -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "portfolio_aave",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_supplier",
                    "address": "0x000000000000000000000000000000000000dead",
                    "network_id": "ethereum-mainnet",
                    "implementation": { "kind": "address_only" },
                    "symbol_ids": ["aave_v3.usdc.asset.ethereum-mainnet"],
                    "metadata": {}
                },
                {
                    "wallet_id": "wallet_borrower",
                    "address": "0x000000000000000000000000000000000000beef",
                    "network_id": "ethereum-mainnet",
                    "implementation": { "kind": "address_only" },
                    "symbol_ids": [
                        "aave_v3.wbtc.collateral.ethereum-mainnet",
                        "aave_v3.usdc.debt.ethereum-mainnet"
                    ],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "usdc.wallet.ethereum-mainnet",
                    "display_symbol": "USDC",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": "0x0000000000000000000000000000000000000001"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1.00"
                                }
                            }
                        ]
                    },
                    "decimals": 6,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "wbtc.wallet.ethereum-mainnet",
                    "display_symbol": "WBTC",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": "0x0000000000000000000000000000000000000002"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "wbtc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "70000.00"
                                }
                            }
                        ]
                    },
                    "decimals": 8,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "aave_v3.usdc.asset.ethereum-mainnet",
                    "display_symbol": "USDC",
                    "kind": "protocol_position",
                    "role": "asset",
                    "network_id": "ethereum-mainnet",
                    "protocol": "aave_v3",
                    "balance_reader": {
                        "kind": "protocol_position",
                        "protocol": "aave_v3",
                        "reader": "reserve_position",
                        "config": {
                            "market": {
                                "market_id": "aave-v3-mainnet",
                                "network_id": "ethereum-mainnet",
                                "chain_id": 1,
                                "pool_address": "0x0000000000000000000000000000000000003000",
                                "reserves": [
                                    {
                                        "reserve_id": "usdc",
                                        "reserve_index": 0,
                                        "underlying_token_address": "0x0000000000000000000000000000000000000001",
                                        "a_token_address": "0x0000000000000000000000000000000000002001",
                                        "variable_debt_token_address": "0x0000000000000000000000000000000000002002",
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    },
                                    {
                                        "reserve_id": "wbtc",
                                        "reserve_index": 1,
                                        "underlying_token_address": "0x0000000000000000000000000000000000000002",
                                        "a_token_address": "0x0000000000000000000000000000000000002003",
                                        "variable_debt_token_address": null,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    }
                                ],
                                "metadata": {}
                            },
                            "reserve_id": "usdc"
                        }
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1.00"
                                }
                            }
                        ]
                    },
                    "decimals": null,
                    "underlying_symbol_id": "usdc.wallet.ethereum-mainnet",
                    "metadata": {}
                },
                {
                    "symbol_id": "aave_v3.wbtc.collateral.ethereum-mainnet",
                    "display_symbol": "WBTC",
                    "kind": "protocol_position",
                    "role": "collateral",
                    "network_id": "ethereum-mainnet",
                    "protocol": "aave_v3",
                    "balance_reader": {
                        "kind": "protocol_position",
                        "protocol": "aave_v3",
                        "reader": "reserve_position",
                        "config": {
                            "market": {
                                "market_id": "aave-v3-mainnet",
                                "network_id": "ethereum-mainnet",
                                "chain_id": 1,
                                "pool_address": "0x0000000000000000000000000000000000003000",
                                "reserves": [
                                    {
                                        "reserve_id": "usdc",
                                        "reserve_index": 0,
                                        "underlying_token_address": "0x0000000000000000000000000000000000000001",
                                        "a_token_address": "0x0000000000000000000000000000000000002001",
                                        "variable_debt_token_address": "0x0000000000000000000000000000000000002002",
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    },
                                    {
                                        "reserve_id": "wbtc",
                                        "reserve_index": 1,
                                        "underlying_token_address": "0x0000000000000000000000000000000000000002",
                                        "a_token_address": "0x0000000000000000000000000000000000002003",
                                        "variable_debt_token_address": null,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    }
                                ],
                                "metadata": {}
                            },
                            "reserve_id": "wbtc"
                        }
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "wbtc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "70000.00"
                                }
                            }
                        ]
                    },
                    "decimals": null,
                    "underlying_symbol_id": "wbtc.wallet.ethereum-mainnet",
                    "metadata": {}
                },
                {
                    "symbol_id": "aave_v3.usdc.debt.ethereum-mainnet",
                    "display_symbol": "USDC",
                    "kind": "protocol_position",
                    "role": "debt",
                    "network_id": "ethereum-mainnet",
                    "protocol": "aave_v3",
                    "balance_reader": {
                        "kind": "protocol_position",
                        "protocol": "aave_v3",
                        "reader": "debt_position",
                        "config": {
                            "market": {
                                "market_id": "aave-v3-mainnet",
                                "network_id": "ethereum-mainnet",
                                "chain_id": 1,
                                "pool_address": "0x0000000000000000000000000000000000003000",
                                "reserves": [
                                    {
                                        "reserve_id": "usdc",
                                        "reserve_index": 0,
                                        "underlying_token_address": "0x0000000000000000000000000000000000000001",
                                        "a_token_address": "0x0000000000000000000000000000000000002001",
                                        "variable_debt_token_address": "0x0000000000000000000000000000000000002002",
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    },
                                    {
                                        "reserve_id": "wbtc",
                                        "reserve_index": 1,
                                        "underlying_token_address": "0x0000000000000000000000000000000000000002",
                                        "a_token_address": "0x0000000000000000000000000000000000002003",
                                        "variable_debt_token_address": null,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    }
                                ],
                                "metadata": {}
                            },
                            "reserve_id": "usdc",
                            "debt_kind": "variable"
                        }
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1.00"
                                }
                            }
                        ]
                    },
                    "decimals": null,
                    "underlying_symbol_id": "usdc.wallet.ethereum-mainnet",
                    "metadata": {}
                }
            ],
            "metadata": {}
        },
        "valuation_source_registry": {
            "sources": []
        }
    })
}
