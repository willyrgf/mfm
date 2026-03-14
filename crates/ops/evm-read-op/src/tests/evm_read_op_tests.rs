use super::*;

use std::collections::HashMap;

use async_trait::async_trait;
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase};
use mfm_machine::errors::{ErrorCategory, ErrorInfo};
use mfm_machine::events::event_envelopes_from_stream_records;
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{FactIndex, LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::replay_io::ReplayIo;
use mfm_machine::runtime::{DefaultExecutionEngine, EngineFailpoints};
use mfm_machine::stores::StreamId;
use mfm_sdk::unstable::SdkPlanResolver;
use mfm_state_common::test_support as op_test_support;
use tokio::sync::Mutex;

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

    fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
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
    async fn call(
        &mut self,
        call: IoCall,
    ) -> Result<serde_json::Value, mfm_machine::errors::IoError> {
        let method = call
            .request
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let mut inner = self.counts.lock().await;
        *inner.entry(method.clone()).or_insert(0) += 1;
        drop(inner);

        match method.as_str() {
            "eth_chainId" => Ok(serde_json::json!("0x1")),
            "eth_blockNumber" => Ok(serde_json::json!("0x7b")),
            _ => Err(mfm_machine::errors::IoError::Other(info(
                "unknown_method",
                ErrorCategory::Unknown,
                "unknown jsonrpc method",
            ))),
        }
    }
}

fn topo(graph: &StateGraph) -> Vec<&StateNode> {
    // Kahn, stable for this small DAG.
    let mut in_deg: HashMap<&StateId, usize> = HashMap::new();
    for s in &graph.states {
        in_deg.insert(&s.id, 0);
    }
    for e in &graph.edges {
        *in_deg.get_mut(&e.to).expect("to") += 1;
    }

    let mut ready: Vec<&StateId> = in_deg
        .iter()
        .filter_map(|(id, deg)| (*deg == 0).then_some(*id))
        .collect();
    ready.sort_by(|a, b| a.as_str().cmp(b.as_str()));

    let mut out = Vec::new();
    while let Some(id) = ready.pop() {
        let node = graph.states.iter().find(|n| &n.id == id).expect("node");
        out.push(node);

        for e in graph.edges.iter().filter(|e| &e.from == id) {
            let deg = in_deg.get_mut(&e.to).expect("deg");
            *deg -= 1;
            if *deg == 0 {
                ready.push(&e.to);
                ready.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            }
        }
    }
    out
}

#[tokio::test]
async fn at_live_then_replay_determinism() {
    let counts = Arc::new(Mutex::new(HashMap::new()));
    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

    let op: mfm_sdk::op::DynOperation = Arc::new(EvmReadOp);
    let (registry, planner, pipeline) =
        op_test_support::single_op_plan(op, serde_json::json!({})).expect("pipeline");

    let resolver = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));
    let engine =
        DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
    let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);
    let stores = op_test_support::in_memory_stores();

    let cfg = op_test_support::run_config_live();
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

    // Manual replay (reconstruct from initial snapshot + recorded facts).
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
        .expect("get initial snapshot");
    let initial = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
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

    let v = ctx.dump().expect("dump");
    let computed = artifact_id_for_json(&v).expect("hash");
    assert_eq!(computed, final_snapshot_id);

    // Sanity: live run performed exactly 2 JSON-RPC calls.
    let got = counts.lock().await.clone();
    assert_eq!(got.get("eth_chainId").copied().unwrap_or(0), 1);
    assert_eq!(got.get("eth_blockNumber").copied().unwrap_or(0), 1);
}

struct NoopRecorder;
#[async_trait]
impl EventRecorder for NoopRecorder {
    async fn emit(
        &mut self,
        _event: mfm_machine::events::DomainEvent,
    ) -> Result<(), mfm_machine::errors::RunError> {
        Ok(())
    }
    async fn emit_many(
        &mut self,
        _events: Vec<mfm_machine::events::DomainEvent>,
    ) -> Result<(), mfm_machine::errors::RunError> {
        Ok(())
    }
}

#[tokio::test]
async fn at_crash_resume_orphan_attempt_reuses_facts() {
    let counts = Arc::new(Mutex::new(HashMap::new()));
    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

    let failpoints = EngineFailpoints::default();
    failpoints
        .stop_after_handler_once
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let op: mfm_sdk::op::DynOperation = Arc::new(EvmReadOp);
    let (registry, planner, pipeline) =
        op_test_support::single_op_plan(op, serde_json::json!({})).expect("pipeline");

    let resolver = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));
    let engine = DefaultExecutionEngine::new(resolver)
        .with_live_transport_factory(Arc::clone(&factory))
        .with_failpoints(failpoints.clone());
    let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

    let stores = op_test_support::in_memory_stores();

    let cfg = op_test_support::run_config_live();
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

    // ChainId handler was executed twice, but transport should be called once due to fact reuse.
    let got = counts.lock().await.clone();
    assert_eq!(got.get("eth_chainId").copied().unwrap_or(0), 1);
    assert_eq!(got.get("eth_blockNumber").copied().unwrap_or(0), 1);
}
