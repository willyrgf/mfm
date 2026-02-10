//! Proof op (Milestone 1 acceptance tests).
//!
//! Source of truth: `REDESIGN.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, StateError};
use mfm_machine::events::{ArtifactWritten, DomainEvent, DOMAIN_EVENT_ARTIFACT_WRITTEN};
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{ContextKey, ErrorCode, FactKey, OpId, OpPath};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_machine::stores::ArtifactKind;

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

const OP_ID: &str = "proof";
const OP_VERSION: &str = "v1";

// Custom domain event (audit only).
const DOMAIN_EVENT_IDEMPOTENCY_KEY: &str = "proof_idempotency_key";

fn info(code: &'static str, category: ErrorCategory, retryable: bool, message: &str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details: None,
    }
}

fn state_error(code: &'static str, message: &str) -> StateError {
    StateError {
        state_id: None,
        info: info(code, ErrorCategory::Unknown, false, message),
    }
}

fn ctx_key(s: &'static str) -> ContextKey {
    ContextKey(s.to_string())
}

fn read_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("proof:read|op:{}", op_path.0))
}

fn output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("proof:output|op:{}", op_path.0))
}

fn idempotency_key_for_value(v: &serde_json::Value) -> Result<String, StateError> {
    let id = artifact_id_for_json(v).map_err(|_| {
        state_error(
            "idempotency_key_not_canonical",
            "value was not canonical-json-hashable",
        )
    })?;
    Ok(id.0)
}

fn side_effect_fact_key(op_path: &OpPath, idempotency_key: &str) -> FactKey {
    FactKey(format!(
        "proof:side_effect|op:{}|id:{idempotency_key}",
        op_path.0
    ))
}

#[derive(Clone)]
struct OrphanAfterSideEffect {
    stop_after_handler_once: Arc<AtomicBool>,
    armed_once: Arc<AtomicBool>,
}

impl OrphanAfterSideEffect {
    fn arm(stop_after_handler_once: Arc<AtomicBool>) -> Self {
        Self {
            stop_after_handler_once,
            armed_once: Arc::new(AtomicBool::new(true)),
        }
    }

    fn trigger_if_armed(&self) {
        if self.armed_once.swap(false, Ordering::SeqCst) {
            self.stop_after_handler_once.store(true, Ordering::SeqCst);
        }
    }
}

/// Proof op implementation used by acceptance tests.
#[derive(Clone, Default)]
pub struct ProofOp {
    orphan_after_side_effect: Option<OrphanAfterSideEffect>,
}

impl ProofOp {
    /// Configure this op to request the engine to stop after the side-effect state handler returns once.
    ///
    /// Intended for crash/resume tests (orphan attempt simulation).
    pub fn with_orphan_after_side_effect(
        mut self,
        stop_after_handler_once: Arc<AtomicBool>,
    ) -> Self {
        self.orphan_after_side_effect = Some(OrphanAfterSideEffect::arm(stop_after_handler_once));
        self
    }
}

impl Operation for ProofOp {
    fn op_id(&self) -> OpId {
        OpId(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: vec![PortKey("output".to_string())],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        _op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let read_id = format!("{}.read_facts", op_path.0);
        let side_id = format!("{}.apply_side_effect", op_path.0);
        let out_id = format!("{}.write_output", op_path.0);

        let read_sid = mfm_machine::ids::StateId(read_id);
        let side_sid = mfm_machine::ids::StateId(side_id);
        let out_sid = mfm_machine::ids::StateId(out_id);

        let read = Arc::new(ReadFactsState {
            op_path: op_path.clone(),
        });
        let side = Arc::new(ApplySideEffectState {
            op_path: op_path.clone(),
            orphan_after_side_effect: self.orphan_after_side_effect.clone(),
        });
        let out = Arc::new(WriteOutputState {
            op_path: op_path.clone(),
        });

        Ok(StateGraph {
            states: vec![
                StateNode {
                    id: read_sid.clone(),
                    state: read,
                },
                StateNode {
                    id: side_sid.clone(),
                    state: side,
                },
                StateNode {
                    id: out_sid.clone(),
                    state: out,
                },
            ],
            edges: vec![
                DependencyEdge {
                    from: read_sid.clone(),
                    to: side_sid.clone(),
                },
                DependencyEdge {
                    from: side_sid,
                    to: out_sid,
                },
            ],
        })
    }
}

struct ReadFactsState {
    op_path: OpPath,
}

#[async_trait]
impl State for ReadFactsState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: vec![Tag("read_only_io".to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let key = read_fact_key(&self.op_path);
        let res = io
            .call(IoCall {
                namespace: "proof.read".to_string(),
                request: serde_json::json!({}),
                fact_key: Some(key),
            })
            .await
            .map_err(|_| state_error("read_fact_io_failed", "failed to read input fact"))?;

        ctx.write(ctx_key("read_fact"), res.response)
            .map_err(|_| state_error("ctx_write_failed", "context write failed"))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

struct ApplySideEffectState {
    op_path: OpPath,
    orphan_after_side_effect: Option<OrphanAfterSideEffect>,
}

#[async_trait]
impl State for ApplySideEffectState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: vec![Tag("apply_side_effect".to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ApplySideEffect,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let Some(read_fact) = ctx
            .read(&ctx_key("read_fact"))
            .map_err(|_| state_error("ctx_read_failed", "context read failed"))?
        else {
            return Err(state_error(
                "missing_read_fact",
                "missing read_fact in context",
            ));
        };

        let id_key = idempotency_key_for_value(&read_fact)?;
        ctx.write(
            ctx_key("idempotency_key"),
            serde_json::json!(id_key.clone()),
        )
        .map_err(|_| state_error("ctx_write_failed", "context write failed"))?;

        let fact_key = side_effect_fact_key(&self.op_path, &id_key);
        let existing = io
            .get_recorded_fact(&fact_key)
            .await
            .map_err(|_| state_error("io_fact_lookup_failed", "failed to lookup recorded fact"))?;

        if existing.is_none() {
            rec.emit(DomainEvent {
                name: DOMAIN_EVENT_IDEMPOTENCY_KEY.to_string(),
                payload: serde_json::json!({"key": id_key}),
                payload_ref: None,
            })
            .await
            .map_err(|_| state_error("emit_failed", "failed to emit idempotency event"))?;
        }

        let res = io
            .call(IoCall {
                namespace: "proof.side_effect".to_string(),
                request: serde_json::json!({"idempotency_key": id_key}),
                fact_key: Some(fact_key),
            })
            .await
            .map_err(|_| state_error("side_effect_io_failed", "side-effect call failed"))?;

        ctx.write(ctx_key("side_effect_result"), res.response)
            .map_err(|_| state_error("ctx_write_failed", "context write failed"))?;

        if let Some(orphan) = &self.orphan_after_side_effect {
            orphan.trigger_if_armed();
        }

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

struct WriteOutputState {
    op_path: OpPath,
}

#[async_trait]
impl State for WriteOutputState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::Pure,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let read_fact = ctx
            .read(&ctx_key("read_fact"))
            .map_err(|_| state_error("ctx_read_failed", "context read failed"))?
            .ok_or_else(|| state_error("missing_read_fact", "missing read_fact in context"))?;

        let side_effect = ctx
            .read(&ctx_key("side_effect_result"))
            .map_err(|_| state_error("ctx_read_failed", "context read failed"))?
            .ok_or_else(|| {
                state_error(
                    "missing_side_effect",
                    "missing side_effect_result in context",
                )
            })?;

        let output = serde_json::json!({
            "read_fact": read_fact,
            "side_effect_result": side_effect,
        });

        let key = output_fact_key(&self.op_path);
        let existed = io
            .get_recorded_fact(&key)
            .await
            .map_err(|_| state_error("io_fact_lookup_failed", "failed to lookup recorded fact"))?
            .is_some();

        let res = io
            .call(IoCall {
                namespace: "proof.output".to_string(),
                request: output,
                fact_key: Some(key),
            })
            .await
            .map_err(|_| state_error("output_io_failed", "output call failed"))?;

        let Some(payload_id) = res.recorded_payload_id else {
            return Err(state_error(
                "missing_output_payload_id",
                "expected recorded payload id for output",
            ));
        };

        ctx.write(
            ctx_key("output_artifact_id"),
            serde_json::json!(payload_id.0.clone()),
        )
        .map_err(|_| state_error("ctx_write_failed", "context write failed"))?;

        if !existed {
            let payload = serde_json::to_value(ArtifactWritten {
                artifact_id: payload_id,
                kind: ArtifactKind::Output,
                meta: serde_json::json!({}),
            })
            .map_err(|_| {
                state_error(
                    "artifact_written_serialize_failed",
                    "failed to serialize ArtifactWritten",
                )
            })?;

            rec.emit(DomainEvent {
                name: DOMAIN_EVENT_ARTIFACT_WRITTEN.to_string(),
                payload,
                payload_ref: None,
            })
            .await
            .map_err(|_| state_error("emit_failed", "failed to emit artifact_written"))?;
        }

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{HashMap, VecDeque};

    use mfm_machine::config::{
        BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
        RetryPolicy, RunConfig,
    };
    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::errors::{ContextError, IoError, RunError, StorageError};
    use mfm_machine::events::{Event, EventEnvelope, KernelEvent};
    use mfm_machine::hashing::{artifact_id_for_bytes, artifact_id_for_json};
    use mfm_machine::ids::{ArtifactId, RunId, StateId};
    use mfm_machine::live_io::{FactIndex, LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::plan::ExecutionPlan;
    use mfm_machine::recorder::EventRecorder;
    use mfm_machine::replay_io::ReplayIo;
    use mfm_machine::runtime::{DefaultExecutionEngine, EngineFailpoints};
    use mfm_machine::stores::{ArtifactStore, EventStore};

    use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
    use mfm_sdk::pipeline::PipelinePlanner;
    use mfm_sdk::unstable::{
        single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
        SdkPlanResolver,
    };

    fn run_config_live() -> RunConfig {
        RunConfig {
            io_mode: IoMode::Live,
            retry_policy: RetryPolicy {
                max_attempts: 3,
                backoff: BackoffPolicy::Fixed {
                    delay: std::time::Duration::from_millis(0),
                },
            },
            event_profile: EventProfile::Normal,
            execution_mode: ExecutionMode::Sequential,
            context_checkpointing: ContextCheckpointing::AfterEveryState,
            replay_missing_fact_retryable: false,
            skip_tags: Vec::new(),
        }
    }

    #[derive(Default)]
    struct MapContext {
        inner: HashMap<String, serde_json::Value>,
    }

    impl MapContext {
        fn from_snapshot(v: serde_json::Value) -> Self {
            let mut ctx = MapContext::default();
            if let serde_json::Value::Object(m) = v {
                for (k, v) in m {
                    ctx.inner.insert(k, v);
                }
            }
            ctx
        }
    }

    impl DynContext for MapContext {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
            Ok(self.inner.get(&key.0).cloned())
        }

        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
            self.inner.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
            self.inner.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, ContextError> {
            let mut m = serde_json::Map::new();
            for (k, v) in &self.inner {
                m.insert(k.clone(), v.clone());
            }
            Ok(serde_json::Value::Object(m))
        }
    }

    #[derive(Clone, Default)]
    struct MemEventStore {
        inner: Arc<tokio::sync::Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
    }

    #[async_trait]
    impl EventStore for MemEventStore {
        async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError> {
            let inner = self.inner.lock().await;
            Ok(inner
                .get(&run_id)
                .and_then(|v| v.last())
                .map(|e| e.seq)
                .unwrap_or(0))
        }

        async fn append(
            &self,
            run_id: RunId,
            expected_seq: u64,
            events: Vec<EventEnvelope>,
        ) -> Result<u64, StorageError> {
            let mut inner = self.inner.lock().await;
            let stream = inner.entry(run_id).or_default();
            let head = stream.last().map(|e| e.seq).unwrap_or(0);
            if head != expected_seq {
                return Err(StorageError::Concurrency(info(
                    "event_store_concurrency",
                    ErrorCategory::Storage,
                    false,
                    "head seq did not match expected seq",
                )));
            }
            stream.extend(events);
            Ok(stream.last().map(|e| e.seq).unwrap_or(head))
        }

        async fn read_range(
            &self,
            run_id: RunId,
            from_seq: u64,
            to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, StorageError> {
            let inner = self.inner.lock().await;
            let Some(stream) = inner.get(&run_id) else {
                return Ok(Vec::new());
            };
            let from = from_seq.max(1);
            let to = to_seq.unwrap_or(u64::MAX);
            Ok(stream
                .iter()
                .filter(|e| e.seq >= from && e.seq <= to)
                .cloned()
                .collect())
        }
    }

    #[derive(Clone, Default)]
    struct MemArtifactStore {
        inner: Arc<tokio::sync::Mutex<HashMap<ArtifactId, Vec<u8>>>>,
    }

    #[async_trait]
    impl ArtifactStore for MemArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            let id = artifact_id_for_bytes(&bytes);
            self.inner.lock().await.insert(id.clone(), bytes);
            Ok(id)
        }

        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            let inner = self.inner.lock().await;
            inner.get(id).cloned().ok_or_else(|| {
                StorageError::NotFound(info(
                    "artifact_not_found",
                    ErrorCategory::Storage,
                    false,
                    "artifact not found",
                ))
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(self.inner.lock().await.contains_key(id))
        }
    }

    #[derive(Clone)]
    struct CountingTransportFactory {
        counts: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
    }

    impl CountingTransportFactory {
        fn new(counts: Arc<tokio::sync::Mutex<HashMap<String, usize>>>) -> Self {
            Self { counts }
        }
    }

    struct CountingTransport {
        counts: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
    }

    #[async_trait]
    impl LiveIoTransport for CountingTransport {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            {
                let mut m = self.counts.lock().await;
                *m.entry(call.namespace.clone()).or_insert(0) += 1;
            }

            match call.namespace.as_str() {
                "proof.read" => Ok(serde_json::json!({"n": 1})),
                "proof.side_effect" => {
                    let k = call
                        .request
                        .get("idempotency_key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("missing");
                    Ok(serde_json::json!({"tx_hash": format!("0x{}", &k[..8.min(k.len())])}))
                }
                "proof.output" => Ok(call.request),
                other => Err(mfm_machine::errors::IoError::Other(info(
                    "unknown_namespace",
                    ErrorCategory::Unknown,
                    false,
                    &format!("unknown namespace: {other}"),
                ))),
            }
        }
    }

    impl LiveIoTransportFactory for CountingTransportFactory {
        fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(CountingTransport {
                counts: Arc::clone(&self.counts),
            })
        }
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

    fn topo(plan: &ExecutionPlan) -> Vec<StateNode> {
        let mut nodes_by_id: HashMap<StateId, StateNode> = HashMap::new();
        for n in &plan.graph.states {
            nodes_by_id.insert(n.id.clone(), n.clone());
        }

        let mut indegree: HashMap<StateId, usize> = HashMap::new();
        let mut edges_from: HashMap<StateId, Vec<StateId>> = HashMap::new();
        for id in nodes_by_id.keys() {
            indegree.insert(id.clone(), 0);
            edges_from.insert(id.clone(), Vec::new());
        }

        for e in &plan.graph.edges {
            edges_from.get_mut(&e.from).unwrap().push(e.to.clone());
            *indegree.get_mut(&e.to).unwrap() += 1;
        }

        let mut q = VecDeque::new();
        for n in &plan.graph.states {
            if indegree.get(&n.id).copied().unwrap_or(0) == 0 {
                q.push_back(n.id.clone());
            }
        }

        let mut out = Vec::new();
        while let Some(id) = q.pop_front() {
            let n = nodes_by_id.get(&id).unwrap().clone();
            out.push(n);
            for to in edges_from.get(&id).unwrap() {
                let d = indegree.get_mut(to).unwrap();
                *d -= 1;
                if *d == 0 {
                    q.push_back(to.clone());
                }
            }
        }
        out
    }

    fn run_started(stream: &[EventEnvelope]) -> (ArtifactId, ArtifactId) {
        for e in stream {
            if let Event::Kernel(KernelEvent::RunStarted {
                manifest_id,
                initial_snapshot_id,
                ..
            }) = &e.event
            {
                return (manifest_id.clone(), initial_snapshot_id.clone());
            }
        }
        panic!("missing RunStarted");
    }

    fn count_state_entered_attempts(stream: &[EventEnvelope], state_id: &str) -> Vec<u32> {
        let mut atts = Vec::new();
        for e in stream {
            if let Event::Kernel(KernelEvent::StateEntered {
                state_id: sid,
                attempt,
                ..
            }) = &e.event
            {
                if sid.0 == state_id {
                    atts.push(*attempt);
                }
            }
        }
        atts
    }

    fn count_domain_event(stream: &[EventEnvelope], name: &str) -> usize {
        stream
            .iter()
            .filter(|e| matches!(&e.event, Event::Domain(de) if de.name == name))
            .count()
    }

    #[tokio::test]
    async fn at06_live_then_replay_determinism() {
        let counts = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

        let op: mfm_sdk::op::DynOperation = Arc::new(ProofOp::default());
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = single_op_pipeline(op.op_id(), op.op_version(), serde_json::json!({}))
            .expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine =
            DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);
        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = run_config_live();

        let res = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline: pipeline.clone(),
                    input: serde_json::json!({}),
                    run_config: cfg.clone(),
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(MapContext::default()),
                },
            )
            .await
            .expect("start");

        assert_eq!(res.phase, RunPhase::Completed);
        let final_snapshot_id = res.final_snapshot_id.clone().expect("final snapshot");

        // Manual replay using ReplayIo + recorded facts must reproduce the final snapshot id.
        let stream = stores
            .events
            .read_range(res.run_id, 1, None)
            .await
            .expect("read_range");
        let facts = FactIndex::from_event_stream(&stream);
        let (_manifest_id, initial_snapshot_id) = run_started(&stream);

        let bytes = stores
            .artifacts
            .get(&initial_snapshot_id)
            .await
            .expect("get initial snapshot");
        let initial = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
        let mut ctx = MapContext::from_snapshot(initial);

        let plan = planner
            .build_execution_plan(Arc::clone(&registry), &pipeline, &cfg)
            .expect("plan");
        for node in topo(&plan) {
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

        // Sanity: live run performed all 3 calls once.
        let got = counts.lock().await.clone();
        assert_eq!(got.get("proof.read").copied().unwrap_or(0), 1);
        assert_eq!(got.get("proof.side_effect").copied().unwrap_or(0), 1);
        assert_eq!(got.get("proof.output").copied().unwrap_or(0), 1);
    }

    #[tokio::test]
    async fn at07_crash_resume_determinism_and_at08_side_effect_idempotency() {
        // Crash/resume run with orphan attempt injected after side-effect handler.
        let counts = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

        let failpoints = EngineFailpoints::default();
        let op: mfm_sdk::op::DynOperation = Arc::new(
            ProofOp::default()
                .with_orphan_after_side_effect(Arc::clone(&failpoints.stop_after_handler_once)),
        );
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = single_op_pipeline(op.op_id(), op.op_version(), serde_json::json!({}))
            .expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine = DefaultExecutionEngine::new(resolver)
            .with_live_transport_factory(Arc::clone(&factory))
            .with_failpoints(failpoints.clone());
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let cfg = run_config_live();

        let first = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline: pipeline.clone(),
                    input: serde_json::json!({}),
                    run_config: cfg.clone(),
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(MapContext::default()),
                },
            )
            .await
            .expect("start");

        assert_eq!(first.phase, RunPhase::Running);

        let resumed = launcher
            .resume(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                first.run_id,
            )
            .await
            .expect("resume");
        assert_eq!(resumed.phase, RunPhase::Completed);
        let final_snapshot_id = resumed.final_snapshot_id.clone().expect("snapshot id");

        let stream = stores
            .events
            .read_range(first.run_id, 1, None)
            .await
            .expect("read_range");

        // The side-effect state must have been attempted twice (orphan + retry).
        let atts = count_state_entered_attempts(&stream, "proof.main.apply_side_effect");
        assert_eq!(atts, vec![0, 1]);

        // Idempotency key event should be emitted only once (first attempt).
        assert_eq!(count_domain_event(&stream, DOMAIN_EVENT_IDEMPOTENCY_KEY), 1);

        // The side-effect transport call must happen once (fact single-assignment).
        let got = counts.lock().await.clone();
        assert_eq!(got.get("proof.side_effect").copied().unwrap_or(0), 1);

        // Clean run without crash should produce the same final snapshot id.
        let counts2 = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let factory2: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingTransportFactory::new(Arc::clone(&counts2)));

        let op2: mfm_sdk::op::DynOperation = Arc::new(ProofOp::default());
        let mut reg2 = HashMapOperationRegistry::default();
        reg2.register(Arc::clone(&op2));
        let registry2: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg2);
        let planner2: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline2 = single_op_pipeline(op2.op_id(), op2.op_version(), serde_json::json!({}))
            .expect("pipeline");
        let resolver2 = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry2),
            Arc::clone(&planner2),
        ));
        let engine2 = DefaultExecutionEngine::new(resolver2).with_live_transport_factory(factory2);
        let engine2: Arc<dyn ExecutionEngine> = Arc::new(engine2);
        let stores2 = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };
        let launcher2: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let clean = launcher2
            .start_pipeline(
                engine2,
                Stores {
                    events: Arc::clone(&stores2.events),
                    artifacts: Arc::clone(&stores2.artifacts),
                },
                Arc::clone(&registry2),
                Arc::clone(&planner2),
                LaunchPipeline {
                    pipeline: pipeline2,
                    input: serde_json::json!({}),
                    run_config: cfg,
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(MapContext::default()),
                },
            )
            .await
            .expect("start");
        assert_eq!(clean.phase, RunPhase::Completed);
        assert_eq!(
            clean.final_snapshot_id.expect("snapshot id"),
            final_snapshot_id
        );
    }
}
