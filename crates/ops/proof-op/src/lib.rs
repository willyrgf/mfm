//! Proof op (Milestone 1 acceptance tests).
//!
//! Source of truth: `REDESIGN.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::events::DomainEvent;
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_op_common::ctx as op_ctx;
use mfm_op_common::errors as op_errors;
use mfm_op_common::idempotency as op_idempotency;
use mfm_op_common::output as op_output;
use mfm_op_common::states::meta;

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

#[cfg(test)]
use mfm_machine::errors::ErrorInfo;

const OP_ID: &str = "proof";
const OP_VERSION: &str = "v1";

// Custom domain event (audit only).
const DOMAIN_EVENT_IDEMPOTENCY_KEY: &str = "proof_idempotency_key";

#[cfg(test)]
fn info(code: &'static str, category: ErrorCategory, retryable: bool, message: &str) -> ErrorInfo {
    op_errors::info(code, category, retryable, message)
}

fn state_error(code: &'static str, message: &str) -> StateError {
    op_errors::state_error(code, ErrorCategory::Unknown, false, message)
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
        meta::read_only_io_with_tag(meta::tags::READ_ONLY_IO)
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

        op_ctx::write_json(ctx, ctx_key("read_fact"), res.response)?;

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
        meta::apply_side_effect(op_idempotency::op_scope("proof:side_effect", &self.op_path))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let read_fact = op_ctx::read_json_required(
            ctx,
            &ctx_key("read_fact"),
            "missing_read_fact",
            "missing read_fact in context",
        )?;

        let id_key = idempotency_key_for_value(&read_fact)?;
        op_ctx::write_json(
            ctx,
            ctx_key("idempotency_key"),
            serde_json::json!(id_key.clone()),
        )?;

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

        op_ctx::write_json(ctx, ctx_key("side_effect_result"), res.response)?;

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
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let read_fact = op_ctx::read_json_required(
            ctx,
            &ctx_key("read_fact"),
            "missing_read_fact",
            "missing read_fact in context",
        )?;

        let side_effect = op_ctx::read_json_required(
            ctx,
            &ctx_key("side_effect_result"),
            "missing_side_effect",
            "missing side_effect_result in context",
        )?;

        let output = serde_json::json!({
            "read_fact": read_fact,
            "side_effect_result": side_effect,
        });

        op_output::write_output_artifact(
            ctx,
            io,
            rec,
            "proof.output",
            output_fact_key(&self.op_path),
            output,
            ctx_key("output_artifact_id"),
        )
        .await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{HashMap, VecDeque};

    use mfm_machine::config::RunConfig;
    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::errors::{IoError, RunError};
    use mfm_machine::events::{
        ChildRunCompleted, ChildRunSpawned, Event, EventEnvelope, KernelEvent,
        DOMAIN_EVENT_CHILD_RUN_COMPLETED, DOMAIN_EVENT_CHILD_RUN_SPAWNED,
    };
    use mfm_machine::hashing::artifact_id_for_json;
    use mfm_machine::ids::{ArtifactId, RunId, StateId};
    use mfm_machine::live_io::{FactIndex, LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::plan::ExecutionPlan;
    use mfm_machine::recorder::EventRecorder;
    use mfm_machine::replay_io::ReplayIo;
    use mfm_machine::runtime::{
        ChildRunLiveIoTransportFactory, DefaultExecutionEngine, EngineFailpoints, PlanResolver,
    };
    use mfm_op_common::test_support as op_test_support;

    use mfm_sdk::pipeline::PipelinePlanner;
    use mfm_sdk::unstable::{
        single_op_pipeline, DefaultPipelinePlanner, HashMapOperationRegistry, SdkPlanResolver,
    };

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

    fn child_run_spawned(stream: &[EventEnvelope]) -> Vec<ChildRunSpawned> {
        let mut out = Vec::new();
        for e in stream {
            let Event::Domain(de) = &e.event else {
                continue;
            };
            if de.name != DOMAIN_EVENT_CHILD_RUN_SPAWNED {
                continue;
            }
            let payload = serde_json::from_value::<ChildRunSpawned>(de.payload.clone())
                .expect("ChildRunSpawned payload");
            out.push(payload);
        }
        out
    }

    fn child_run_completed(stream: &[EventEnvelope]) -> Vec<ChildRunCompleted> {
        let mut out = Vec::new();
        for e in stream {
            let Event::Domain(de) = &e.event else {
                continue;
            };
            if de.name != DOMAIN_EVENT_CHILD_RUN_COMPLETED {
                continue;
            }
            let payload = serde_json::from_value::<ChildRunCompleted>(de.payload.clone())
                .expect("ChildRunCompleted payload");
            out.push(payload);
        }
        out
    }

    // Parent op for Milestone 5 child-run tests.
    const CHILD_PARENT_OP_ID: &str = "child_parent";
    const CHILD_PARENT_OP_VERSION: &str = "v1";

    #[derive(Clone)]
    struct OrphanAfterJoin {
        stop_after_handler_once: Arc<AtomicBool>,
        armed_once: Arc<AtomicBool>,
    }

    impl OrphanAfterJoin {
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

    #[derive(Clone, Default)]
    struct ChildParentOp {
        orphan_after_join: Option<OrphanAfterJoin>,
    }

    impl ChildParentOp {
        fn with_orphan_after_join(mut self, stop_after_handler_once: Arc<AtomicBool>) -> Self {
            self.orphan_after_join = Some(OrphanAfterJoin::arm(stop_after_handler_once));
            self
        }
    }

    impl Operation for ChildParentOp {
        fn op_id(&self) -> OpId {
            OpId(CHILD_PARENT_OP_ID.to_string())
        }

        fn op_version(&self) -> String {
            CHILD_PARENT_OP_VERSION.to_string()
        }

        fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
            Ok(OpIo {
                imports: Vec::new(),
                exports: vec![PortKey("joined".to_string())],
            })
        }

        fn expand(
            &self,
            op_path: OpPath,
            _op_config: &serde_json::Value,
            run_config: &RunConfig,
        ) -> Result<StateGraph, SdkError> {
            let spawn_sid = mfm_machine::ids::StateId(format!("{}.spawn_children", op_path.0));
            let join_sid = mfm_machine::ids::StateId(format!("{}.join_children", op_path.0));

            let spawn = Arc::new(SpawnChildrenState {
                op_path: op_path.clone(),
                child_run_config: run_config.clone(),
            });
            let join = Arc::new(JoinChildrenState {
                op_path: op_path.clone(),
                orphan_after_join: self.orphan_after_join.clone(),
            });

            Ok(StateGraph {
                states: vec![
                    StateNode {
                        id: spawn_sid.clone(),
                        state: spawn,
                    },
                    StateNode {
                        id: join_sid.clone(),
                        state: join,
                    },
                ],
                edges: vec![DependencyEdge {
                    from: spawn_sid,
                    to: join_sid,
                }],
            })
        }
    }

    struct SpawnChildrenState {
        op_path: OpPath,
        child_run_config: RunConfig,
    }

    #[async_trait]
    impl State for SpawnChildrenState {
        fn meta(&self) -> StateMeta {
            meta::apply_side_effect_with_tag(
                "child_run_spawn",
                op_idempotency::op_scope("child_parent:spawn", &self.op_path),
            )
        }

        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            let mut refs = Vec::new();
            for i in 0..2u32 {
                let fact_key = FactKey(format!("child_parent:spawn|op:{}|i:{i}", self.op_path.0));
                let rr = mfm_sdk::unstable::child_runs::spawn_child_run_v1(
                    io,
                    rec,
                    fact_key,
                    mfm_sdk::unstable::child_runs::SpawnChildRunV1 {
                        op_id: OpId(OP_ID.to_string()),
                        op_version: OP_VERSION.to_string(),
                        op_config: serde_json::json!({}),
                        input: serde_json::json!({"i": i}),
                        run_config: self.child_run_config.clone(),
                        initial_context: None,
                    },
                )
                .await
                .map_err(|_| state_error("child_spawn_failed", "failed to spawn child run"))?;

                refs.push(serde_json::json!({
                    "i": i,
                    "child_run_id": rr.child_run_id,
                    "child_manifest_id": rr.child_manifest_id,
                }));
            }

            op_ctx::write_json(ctx, ctx_key("child_refs"), serde_json::Value::Array(refs))?;

            Ok(StateOutcome {
                snapshot: SnapshotPolicy::OnSuccess,
            })
        }
    }

    struct JoinChildrenState {
        op_path: OpPath,
        orphan_after_join: Option<OrphanAfterJoin>,
    }

    #[async_trait]
    impl State for JoinChildrenState {
        fn meta(&self) -> StateMeta {
            meta::apply_side_effect_with_tag(
                "child_run_join",
                op_idempotency::op_scope("child_parent:join", &self.op_path),
            )
        }

        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            let refs = op_ctx::read_array_required(
                ctx,
                &ctx_key("child_refs"),
                "missing_child_refs",
                "missing child_refs in context",
                "invalid_child_refs",
                "child_refs must be an array",
            )?;

            let mut joined: Vec<(RunId, serde_json::Value)> = Vec::new();
            for r in refs {
                let idx = r.get("i").and_then(|v| v.as_u64()).ok_or_else(|| {
                    state_error("invalid_child_refs", "child_refs entry missing i")
                })? as u32;
                let child_run_id = serde_json::from_value::<RunId>(
                    r.get("child_run_id").cloned().ok_or_else(|| {
                        state_error(
                            "invalid_child_refs",
                            "child_refs entry missing child_run_id",
                        )
                    })?,
                )
                .map_err(|_| state_error("invalid_child_refs", "invalid child_run_id"))?;
                let child_manifest_id = serde_json::from_value::<ArtifactId>(
                    r.get("child_manifest_id").cloned().ok_or_else(|| {
                        state_error(
                            "invalid_child_refs",
                            "child_refs entry missing child_manifest_id",
                        )
                    })?,
                )
                .map_err(|_| state_error("invalid_child_refs", "invalid child_manifest_id"))?;

                let fact_key = FactKey(format!("child_parent:await|op:{}|i:{idx}", self.op_path.0));
                let rr = mfm_sdk::unstable::child_runs::await_child_run_v1(
                    io,
                    rec,
                    fact_key,
                    mfm_sdk::unstable::child_runs::AwaitChildRunV1 {
                        child_run_id,
                        child_manifest_id,
                    },
                )
                .await
                .map_err(|_| state_error("child_await_failed", "failed to await child run"))?;

                joined.push((rr.child_run_id, rr.final_snapshot));
            }

            // Locked decision (Milestone 5): deterministic join order by child_run_id bytes.
            joined.sort_by(|(a, _), (b, _)| a.0.as_bytes().cmp(b.0.as_bytes()));

            op_ctx::write_json(
                ctx,
                ctx_key("joined"),
                serde_json::Value::Array(joined.into_iter().map(|(_, v)| v).collect()),
            )?;

            if let Some(orphan) = &self.orphan_after_join {
                orphan.trigger_if_armed();
            }

            Ok(StateOutcome {
                snapshot: SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[tokio::test]
    async fn at06_live_then_replay_determinism() {
        let counts = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

        let op: mfm_sdk::op::DynOperation = Arc::new(ProofOp::default());
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

        let cfg = op_test_support::run_config_live_with_retry_attempts(3);

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

        // Manual replay using ReplayIo + recorded facts must reproduce the final snapshot id.
        let stream = stores
            .events
            .read_range(res.run_id, 1, None)
            .await
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

        let cfg = op_test_support::run_config_live_with_retry_attempts(3);

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
        let (registry2, planner2, pipeline2) =
            op_test_support::single_op_plan(op2, serde_json::json!({})).expect("pipeline");
        let resolver2 = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry2),
            Arc::clone(&planner2),
        ));
        let engine2 = DefaultExecutionEngine::new(resolver2).with_live_transport_factory(factory2);
        let engine2: Arc<dyn ExecutionEngine> = Arc::new(engine2);
        let stores2 = op_test_support::in_memory_stores();
        let clean = op_test_support::start_pipeline_with_defaults(
            engine2,
            &stores2,
            Arc::clone(&registry2),
            Arc::clone(&planner2),
            pipeline2,
            cfg,
        )
        .await
        .expect("start");
        assert_eq!(clean.phase, RunPhase::Completed);
        assert_eq!(
            clean.final_snapshot_id.expect("snapshot id"),
            final_snapshot_id
        );
    }

    #[tokio::test]
    async fn at09_child_runs_live_then_replay_determinism_across_tree() {
        let counts = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let base_factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

        let parent: mfm_sdk::op::DynOperation = Arc::new(ChildParentOp::default());
        let child: mfm_sdk::op::DynOperation = Arc::new(ProofOp::default());
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&parent));
        reg.register(Arc::clone(&child));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline =
            single_op_pipeline(parent.op_id(), parent.op_version(), serde_json::json!({}))
                .expect("pipeline");

        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(
            ChildRunLiveIoTransportFactory::new(Arc::clone(&resolver), Arc::clone(&base_factory)),
        );
        let engine: Arc<dyn ExecutionEngine> = Arc::new(
            DefaultExecutionEngine::new(Arc::clone(&resolver)).with_live_transport_factory(factory),
        );

        let stores = op_test_support::in_memory_stores();

        let cfg = op_test_support::run_config_live_with_retry_attempts(3);
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

        let parent_stream = stores
            .events
            .read_range(res.run_id, 1, None)
            .await
            .expect("read_range");

        // Linkage events are the audit trail.
        assert_eq!(
            count_domain_event(&parent_stream, DOMAIN_EVENT_CHILD_RUN_SPAWNED),
            2
        );
        assert_eq!(
            count_domain_event(&parent_stream, DOMAIN_EVENT_CHILD_RUN_COMPLETED),
            2
        );

        // Parent replay determinism: manual replay using ReplayIo reproduces the final snapshot id.
        let parent_facts = FactIndex::from_event_stream(&parent_stream);
        let (_manifest_id, initial_snapshot_id) = op_test_support::run_started(&parent_stream);

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
        for node in topo(&plan) {
            let mut io = ReplayIo::new(
                res.run_id,
                node.id.clone(),
                0,
                Arc::clone(&stores.artifacts),
                parent_facts.clone(),
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

        // Child replay determinism: each child run is replayable and matches its final snapshot id.
        let spawned = child_run_spawned(&parent_stream);
        let completed = child_run_completed(&parent_stream);
        assert_eq!(spawned.len(), 2);
        assert_eq!(completed.len(), 2);

        let mut completed_by_id: HashMap<RunId, ChildRunCompleted> = HashMap::new();
        for c in completed {
            completed_by_id.insert(c.child_run_id, c);
        }

        for s in spawned {
            let c = completed_by_id.get(&s.child_run_id).expect("completed");
            assert!(matches!(
                c.status,
                mfm_machine::events::RunStatus::Completed
            ));

            let child_stream = stores
                .events
                .read_range(s.child_run_id, 1, None)
                .await
                .expect("read child stream");
            let child_facts = FactIndex::from_event_stream(&child_stream);

            let (manifest_id, child_initial_snapshot_id) =
                op_test_support::run_started(&child_stream);
            assert_eq!(manifest_id, s.child_manifest_id);

            let bytes = stores
                .artifacts
                .get(&child_initial_snapshot_id)
                .await
                .expect("get child initial snapshot");
            let initial = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
            let mut ctx = op_test_support::MapContext::from_snapshot(initial);

            let bytes = stores
                .artifacts
                .get(&s.child_manifest_id)
                .await
                .expect("get child manifest");
            let manifest = serde_json::from_slice::<mfm_machine::config::RunManifest>(&bytes)
                .expect("manifest");
            let plan = resolver.resolve(&manifest).expect("child plan");

            for node in topo(&plan) {
                let mut io = ReplayIo::new(
                    s.child_run_id,
                    node.id.clone(),
                    0,
                    Arc::clone(&stores.artifacts),
                    child_facts.clone(),
                    false,
                );
                let mut rec = NoopRecorder;
                node.state
                    .handle(&mut ctx, &mut io, &mut rec)
                    .await
                    .expect("child handle");
            }

            let v = ctx.dump().expect("child dump");
            let computed = artifact_id_for_json(&v).expect("child hash");
            let expected = op_test_support::run_completed_snapshot_id(&child_stream)
                .expect("child RunCompleted");
            assert_eq!(computed, expected);
            assert_eq!(Some(computed), c.final_snapshot_id);
        }

        // Sanity: each child run performed all 3 proof calls once.
        let got = counts.lock().await.clone();
        assert_eq!(got.get("proof.read").copied().unwrap_or(0), 2);
        assert_eq!(got.get("proof.side_effect").copied().unwrap_or(0), 2);
        assert_eq!(got.get("proof.output").copied().unwrap_or(0), 2);
    }

    #[tokio::test]
    async fn at10_child_runs_crash_resume_spawned_but_not_completed() {
        use tokio::sync::Barrier;

        #[derive(Clone)]
        struct BlockingTransportFactory {
            counts: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
            barrier: Arc<Barrier>,
        }

        struct BlockingTransport {
            counts: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
            barrier: Arc<Barrier>,
        }

        #[async_trait]
        impl LiveIoTransport for BlockingTransport {
            async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
                {
                    let mut m = self.counts.lock().await;
                    *m.entry(call.namespace.clone()).or_insert(0) += 1;
                }

                match call.namespace.as_str() {
                    "proof.read" => Ok(serde_json::json!({"n": 1})),
                    "proof.side_effect" => {
                        // Gate children so the parent can crash/resume while they are in flight.
                        self.barrier.wait().await;
                        Ok(serde_json::json!({"tx_hash": "0xdeadbeef"}))
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

        impl LiveIoTransportFactory for BlockingTransportFactory {
            fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
                Box::new(BlockingTransport {
                    counts: Arc::clone(&self.counts),
                    barrier: Arc::clone(&self.barrier),
                })
            }
        }

        let counts = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let barrier = Arc::new(Barrier::new(3));
        let base_factory: Arc<dyn LiveIoTransportFactory> = Arc::new(BlockingTransportFactory {
            counts: Arc::clone(&counts),
            barrier: Arc::clone(&barrier),
        });

        let parent: mfm_sdk::op::DynOperation = Arc::new(ChildParentOp::default());
        let child: mfm_sdk::op::DynOperation = Arc::new(ProofOp::default());
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&parent));
        reg.register(Arc::clone(&child));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline =
            single_op_pipeline(parent.op_id(), parent.op_version(), serde_json::json!({}))
                .expect("pipeline");

        // Stop after the first handler once (spawn state), leaving an orphan attempt.
        let failpoints = EngineFailpoints::default();
        failpoints
            .stop_after_handler_once
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(
            ChildRunLiveIoTransportFactory::new(Arc::clone(&resolver), Arc::clone(&base_factory)),
        );
        let engine: Arc<dyn ExecutionEngine> = Arc::new(
            DefaultExecutionEngine::new(Arc::clone(&resolver))
                .with_live_transport_factory(factory)
                .with_failpoints(failpoints.clone()),
        );

        let stores = op_test_support::in_memory_stores();

        let cfg = op_test_support::run_config_live_with_retry_attempts(3);
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

        // Ensure children are not completed before we let them proceed.
        let parent_stream = stores
            .events
            .read_range(first.run_id, 1, None)
            .await
            .expect("read parent stream");
        let spawned = child_run_spawned(&parent_stream);
        assert_eq!(spawned.len(), 2);

        for s in &spawned {
            let child_stream = stores
                .events
                .read_range(s.child_run_id, 1, None)
                .await
                .expect("read child stream");
            assert!(op_test_support::run_completed_snapshot_id(&child_stream).is_none());
        }

        // Resume in background; it should block until children finish.
        let resume_task = tokio::spawn({
            let engine = Arc::clone(&engine);
            let stores = Stores {
                events: Arc::clone(&stores.events),
                artifacts: Arc::clone(&stores.artifacts),
            };
            let registry = Arc::clone(&registry);
            let planner = Arc::clone(&planner);
            async move {
                op_test_support::resume_pipeline_with_defaults(
                    engine,
                    &stores,
                    registry,
                    planner,
                    first.run_id,
                )
                .await
            }
        });

        // Release both child runs.
        barrier.wait().await;

        let resumed = resume_task.await.expect("join").expect("resume");
        assert_eq!(resumed.phase, RunPhase::Completed);

        let parent_stream = stores
            .events
            .read_range(first.run_id, 1, None)
            .await
            .expect("read parent stream");
        assert_eq!(
            count_domain_event(&parent_stream, DOMAIN_EVENT_CHILD_RUN_SPAWNED),
            2
        );
        assert_eq!(
            count_domain_event(&parent_stream, DOMAIN_EVENT_CHILD_RUN_COMPLETED),
            2
        );

        // Sanity: each child run performed all 3 proof calls once.
        let got = counts.lock().await.clone();
        assert_eq!(got.get("proof.read").copied().unwrap_or(0), 2);
        assert_eq!(got.get("proof.side_effect").copied().unwrap_or(0), 2);
        assert_eq!(got.get("proof.output").copied().unwrap_or(0), 2);
    }

    #[tokio::test]
    async fn at11_child_runs_crash_resume_completed_but_parent_did_not_record_join() {
        let counts = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let base_factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(CountingTransportFactory::new(Arc::clone(&counts)));

        let failpoints = EngineFailpoints::default();
        let parent: mfm_sdk::op::DynOperation = Arc::new(
            ChildParentOp::default()
                .with_orphan_after_join(Arc::clone(&failpoints.stop_after_handler_once)),
        );
        let child: mfm_sdk::op::DynOperation = Arc::new(ProofOp::default());
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&parent));
        reg.register(Arc::clone(&child));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline =
            single_op_pipeline(parent.op_id(), parent.op_version(), serde_json::json!({}))
                .expect("pipeline");

        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(
            ChildRunLiveIoTransportFactory::new(Arc::clone(&resolver), Arc::clone(&base_factory)),
        );
        let engine: Arc<dyn ExecutionEngine> = Arc::new(
            DefaultExecutionEngine::new(Arc::clone(&resolver))
                .with_live_transport_factory(factory)
                .with_failpoints(failpoints.clone()),
        );

        let stores = op_test_support::in_memory_stores();

        let cfg = op_test_support::run_config_live_with_retry_attempts(3);
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

        let parent_stream = stores
            .events
            .read_range(first.run_id, 1, None)
            .await
            .expect("read parent stream");
        assert_eq!(
            count_domain_event(&parent_stream, DOMAIN_EVENT_CHILD_RUN_SPAWNED),
            2
        );
        assert_eq!(
            count_domain_event(&parent_stream, DOMAIN_EVENT_CHILD_RUN_COMPLETED),
            2
        );

        // Proof transports should have been called exactly once per child run.
        let got = counts.lock().await.clone();
        assert_eq!(got.get("proof.read").copied().unwrap_or(0), 2);
        assert_eq!(got.get("proof.side_effect").copied().unwrap_or(0), 2);
        assert_eq!(got.get("proof.output").copied().unwrap_or(0), 2);

        // Manual replay of the parent run must still match the final snapshot id.
        let final_snapshot_id = resumed.final_snapshot_id.clone().expect("final snapshot");
        let parent_facts = FactIndex::from_event_stream(&parent_stream);
        let (_manifest_id, initial_snapshot_id) = op_test_support::run_started(&parent_stream);

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
        for node in topo(&plan) {
            let mut io = ReplayIo::new(
                first.run_id,
                node.id.clone(),
                0,
                Arc::clone(&stores.artifacts),
                parent_facts.clone(),
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
    }
}
