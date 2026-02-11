//! Unstable v4 runtime implementation (executor + resume logic).
//!
//! Source of truth: `REDESIGN.md` (v4).
//! Not part of the stable API contract (Appendix C.1).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::attempt_envelope::{analyze_kernel_events, OrphanAttempt};
use crate::config::{BackoffPolicy, ExecutionMode, RunConfig, RunManifest};
use crate::context_runtime::write_full_snapshot_value;
use crate::engine::{ExecutionEngine, RunPhase, RunResult, StartRun, Stores};
use crate::errors::{ContextError, ErrorCategory, ErrorInfo, RunError, StorageError};
use crate::events::{Event, EventEnvelope, KernelEvent, RunStatus};
use crate::hashing::artifact_id_for_json;
use crate::ids::{ArtifactId, ErrorCode, OpId, RunId, StateId};
use crate::live_io::{FactIndex, LiveIoTransportFactory, UnimplementedLiveIoTransportFactory};
use crate::plan::{DependencyEdge, ExecutionPlan, PlanValidationError, StateNode};
use crate::stores::ArtifactStore;

mod attempt;
mod child_runs;
mod writer;

use writer::{append_kernel, EventWriter, SharedEventWriter};

pub use child_runs::ChildRunLiveIoTransportFactory;

const CODE_UNSUPPORTED_EXECUTION_MODE: &str = "unsupported_execution_mode";

pub trait PlanResolver: Send + Sync {
    fn resolve(&self, manifest: &RunManifest) -> Result<ExecutionPlan, RunError>;
}

#[derive(Clone, Default)]
pub struct EngineFailpoints {
    pub stop_after_handler_once: Arc<std::sync::atomic::AtomicBool>,
}

impl EngineFailpoints {
    fn should_stop_after_handler(&self) -> bool {
        self.stop_after_handler_once
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct DefaultExecutionEngine {
    resolver: Arc<dyn PlanResolver>,
    live_transport_factory: Arc<dyn LiveIoTransportFactory>,
    failpoints: Option<EngineFailpoints>,
}

impl DefaultExecutionEngine {
    pub fn new(resolver: Arc<dyn PlanResolver>) -> Self {
        Self {
            resolver,
            live_transport_factory: Arc::new(UnimplementedLiveIoTransportFactory),
            failpoints: None,
        }
    }

    pub fn with_live_transport_factory(mut self, factory: Arc<dyn LiveIoTransportFactory>) -> Self {
        self.live_transport_factory = factory;
        self
    }

    pub fn with_failpoints(mut self, failpoints: EngineFailpoints) -> Self {
        self.failpoints = Some(failpoints);
        self
    }
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

fn invalid_plan(code: &'static str, message: &'static str) -> RunError {
    RunError::InvalidPlan(info(code, ErrorCategory::Unknown, message))
}

fn storage_not_found(code: &'static str, message: &'static str) -> StorageError {
    StorageError::NotFound(info(code, ErrorCategory::Storage, message))
}

fn context_err(code: &'static str, message: &'static str) -> ContextError {
    ContextError::Serialization(info(code, ErrorCategory::Context, message))
}

fn compute_backoff(policy: &BackoffPolicy, attempt: u32) -> Duration {
    match policy {
        BackoffPolicy::Fixed { delay } => *delay,
        BackoffPolicy::Exponential {
            base_delay,
            max_delay,
        } => {
            let shift = attempt.min(31);
            let factor = 1u32.checked_shl(shift).unwrap_or(u32::MAX);
            let scaled = base_delay.saturating_mul(factor);
            if &scaled > max_delay {
                *max_delay
            } else {
                scaled
            }
        }
    }
}

fn validate_execution_mode(cfg: &RunConfig) -> Result<(), RunError> {
    match cfg.execution_mode {
        ExecutionMode::Sequential => Ok(()),
        ExecutionMode::FanOutJoin { .. } => Err(RunError::InvalidPlan(info(
            CODE_UNSUPPORTED_EXECUTION_MODE,
            ErrorCategory::Unknown,
            "execution_mode FanOutJoin is not supported in Milestone 1",
        ))),
    }
}

fn validate_start_run_contract(run: &StartRun) -> Result<(), RunError> {
    let value = serde_json::to_value(&run.manifest).map_err(|_| {
        invalid_plan(
            "manifest_serialize_failed",
            "failed to serialize run manifest",
        )
    })?;
    let computed = artifact_id_for_json(&value).map_err(|e| match e {
        crate::hashing::CanonicalJsonError::FloatNotAllowed => invalid_plan(
            "manifest_not_canonical",
            "run manifest is not canonical-json-hashable (floats are forbidden)",
        ),
        crate::hashing::CanonicalJsonError::SecretsNotAllowed => invalid_plan(
            "secrets_detected",
            "run manifest contained secrets (Milestone 1 forbids persisting secrets)",
        ),
    })?;
    if computed != run.manifest_id {
        return Err(invalid_plan(
            "manifest_id_mismatch",
            "manifest_id did not match canonical JSON hash of the manifest",
        ));
    }

    if run.manifest.op_id != run.plan.op_id {
        return Err(invalid_plan(
            "manifest_op_id_mismatch",
            "manifest.op_id did not match plan.op_id",
        ));
    }

    if run.manifest.run_config != run.run_config {
        return Err(invalid_plan(
            "run_config_mismatch",
            "run_config did not match manifest.run_config",
        ));
    }

    Ok(())
}

fn topological_order(plan: &ExecutionPlan) -> Result<Vec<StateNode>, PlanValidationError> {
    if plan.graph.states.is_empty() {
        return Err(PlanValidationError::EmptyPlan);
    }

    let mut nodes_by_id: HashMap<StateId, StateNode> = HashMap::new();
    for n in &plan.graph.states {
        if nodes_by_id.contains_key(&n.id) {
            return Err(PlanValidationError::DuplicateStateId {
                state_id: n.id.clone(),
            });
        }
        nodes_by_id.insert(n.id.clone(), n.clone());
    }

    let mut indegree: HashMap<StateId, usize> = HashMap::new();
    let mut edges_from: HashMap<StateId, Vec<StateId>> = HashMap::new();
    for id in nodes_by_id.keys() {
        indegree.insert(id.clone(), 0);
        edges_from.insert(id.clone(), Vec::new());
    }

    for DependencyEdge { from, to } in &plan.graph.edges {
        if !nodes_by_id.contains_key(from) {
            return Err(PlanValidationError::MissingStateForEdge {
                missing: from.clone(),
            });
        }
        if !nodes_by_id.contains_key(to) {
            return Err(PlanValidationError::MissingStateForEdge {
                missing: to.clone(),
            });
        }
        edges_from.get_mut(from).unwrap().push(to.clone());
        *indegree.get_mut(to).unwrap() += 1;
    }

    let mut queue = VecDeque::new();
    for n in &plan.graph.states {
        if indegree.get(&n.id).copied().unwrap_or(0) == 0 {
            queue.push_back(n.id.clone());
        }
    }

    let mut out = Vec::with_capacity(nodes_by_id.len());
    while let Some(id) = queue.pop_front() {
        let node = nodes_by_id.get(&id).unwrap().clone();
        out.push(node);
        for to in edges_from.get(&id).unwrap() {
            let entry = indegree.get_mut(to).unwrap();
            *entry -= 1;
            if *entry == 0 {
                queue.push_back(to.clone());
            }
        }
    }

    if out.len() != nodes_by_id.len() {
        let remaining: Vec<StateId> = indegree
            .into_iter()
            .filter_map(|(id, deg)| if deg > 0 { Some(id) } else { None })
            .collect();
        return Err(PlanValidationError::CircularDependency { cycle: remaining });
    }

    Ok(out)
}

#[derive(Clone, Debug)]
struct RunStartedInfo {
    op_id: OpId,
    manifest_id: ArtifactId,
    initial_snapshot_id: ArtifactId,
}

#[derive(Clone, Debug)]
struct RunHistory {
    started: RunStartedInfo,
    completed_states: HashSet<StateId>,
    last_checkpoint: ArtifactId,
    orphan_attempt: Option<OrphanAttempt>,
    last_failure_by_state: HashMap<StateId, (u32, ArtifactId, bool)>, // (attempt, base_snapshot, retryable)
    last_attempt_by_state: HashMap<StateId, u32>,
    run_completed: Option<(RunStatus, Option<ArtifactId>)>,
}

fn read_run_history(run_id: RunId, stream: &[EventEnvelope]) -> Result<RunHistory, RunError> {
    let analysis = analyze_kernel_events(stream).map_err(|_| {
        invalid_plan(
            "invalid_attempt_envelopes",
            "invalid attempt envelopes in event stream",
        )
    })?;

    let mut started: Option<RunStartedInfo> = None;
    let mut completed_states = HashSet::new();
    let mut last_checkpoint: Option<ArtifactId> = None;
    let mut open_attempt: Option<(StateId, u32, ArtifactId)> = None;
    let mut last_failure_by_state: HashMap<StateId, (u32, ArtifactId, bool)> = HashMap::new();
    let mut last_attempt_by_state: HashMap<StateId, u32> = HashMap::new();
    let mut run_completed: Option<(RunStatus, Option<ArtifactId>)> = None;

    for e in stream {
        if e.run_id != run_id {
            return Err(invalid_plan(
                "run_id_mismatch",
                "event stream run_id mismatch",
            ));
        }

        match &e.event {
            Event::Kernel(ke) => match ke {
                KernelEvent::RunStarted {
                    op_id,
                    manifest_id,
                    initial_snapshot_id,
                } => {
                    if started.is_none() {
                        started = Some(RunStartedInfo {
                            op_id: op_id.clone(),
                            manifest_id: manifest_id.clone(),
                            initial_snapshot_id: initial_snapshot_id.clone(),
                        });
                        last_checkpoint = Some(initial_snapshot_id.clone());
                    }
                }
                KernelEvent::StateEntered {
                    state_id,
                    attempt,
                    base_snapshot_id,
                } => {
                    open_attempt = Some((state_id.clone(), *attempt, base_snapshot_id.clone()));
                    last_attempt_by_state.insert(state_id.clone(), *attempt);
                }
                KernelEvent::StateCompleted {
                    state_id,
                    context_snapshot_id,
                } => {
                    completed_states.insert(state_id.clone());
                    last_checkpoint = Some(context_snapshot_id.clone());
                    open_attempt = None;
                }
                KernelEvent::StateFailed {
                    state_id, error, ..
                } => {
                    let Some((_, attempt, base_snapshot)) = open_attempt.take() else {
                        return Err(invalid_plan(
                            "terminal_without_entered",
                            "state terminal without StateEntered",
                        ));
                    };
                    last_failure_by_state.insert(
                        state_id.clone(),
                        (attempt, base_snapshot, error.info.retryable),
                    );
                }
                KernelEvent::RunCompleted {
                    status,
                    final_snapshot_id,
                } => {
                    run_completed = Some((status.clone(), final_snapshot_id.clone()));
                }
            },
            Event::Domain(_) => {}
        }
    }

    let Some(started) = started else {
        return Err(invalid_plan(
            "missing_run_started",
            "missing RunStarted kernel event",
        ));
    };

    let last_checkpoint = last_checkpoint.unwrap_or_else(|| started.initial_snapshot_id.clone());

    Ok(RunHistory {
        started,
        completed_states,
        last_checkpoint,
        orphan_attempt: analysis.orphan_attempt,
        last_failure_by_state,
        last_attempt_by_state,
        run_completed,
    })
}

async fn read_manifest(
    artifacts: &dyn ArtifactStore,
    manifest_id: &ArtifactId,
) -> Result<RunManifest, RunError> {
    let bytes = artifacts
        .get(manifest_id)
        .await
        .map_err(RunError::Storage)?;
    let value = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
        RunError::Context(context_err(
            "manifest_decode_failed",
            "failed to decode manifest JSON",
        ))
    })?;

    // Defense: manifest bytes must match its content hash.
    let computed = crate::hashing::artifact_id_for_bytes(&bytes);
    if &computed != manifest_id {
        return Err(invalid_plan(
            "manifest_corrupt",
            "manifest artifact content hash mismatch",
        ));
    }

    serde_json::from_value::<RunManifest>(value).map_err(|_| {
        RunError::Context(context_err(
            "manifest_deserialize_failed",
            "failed to deserialize manifest",
        ))
    })
}

fn next_attempt(last_attempt_by_state: &HashMap<StateId, u32>, state_id: &StateId) -> u32 {
    last_attempt_by_state
        .get(state_id)
        .copied()
        .map(|a| a + 1)
        .unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
async fn run_states(
    stores: &Stores,
    plan: &ExecutionPlan,
    run_config: &RunConfig,
    run_id: RunId,
    writer: SharedEventWriter,
    mut current_snapshot_id: ArtifactId,
    completed_states: &HashSet<StateId>,
    start_at_state: Option<(StateId, u32, ArtifactId)>,
    facts: FactIndex,
    live_factory: Arc<dyn LiveIoTransportFactory>,
    failpoints: Option<EngineFailpoints>,
) -> Result<RunResult, RunError> {
    validate_execution_mode(run_config)?;

    let ordered = topological_order(plan)
        .map_err(|_| invalid_plan("invalid_plan", "execution plan failed validation"))?;

    let mut found_start = start_at_state.is_none();
    let mut phase = RunPhase::Running;

    for node in ordered {
        if completed_states.contains(&node.id) {
            continue;
        }

        let (state_id, mut attempt, base_snapshot_id) =
            if let Some((sid, att, base)) = &start_at_state {
                if !found_start {
                    if &node.id != sid {
                        continue;
                    }
                    found_start = true;
                }
                if &node.id == sid {
                    (sid.clone(), *att, base.clone())
                } else {
                    (node.id.clone(), 0, current_snapshot_id.clone())
                }
            } else {
                (node.id.clone(), 0, current_snapshot_id.clone())
            };

        let state = Arc::clone(&node.state);
        let state_meta = state.meta();

        loop {
            let mut attempt_ctx = attempt::AttemptCtx::new(
                stores,
                run_config,
                run_id,
                state_id.clone(),
                attempt,
                base_snapshot_id.clone(),
                facts.clone(),
                Arc::clone(&writer),
                Arc::clone(&live_factory),
                failpoints.clone(),
                Arc::clone(&state),
                state_meta.clone(),
            );

            match attempt::execute_attempt(&mut attempt_ctx).await? {
                attempt::AttemptExec::Completed { snapshot_id } => {
                    current_snapshot_id = snapshot_id;
                    break;
                }
                attempt::AttemptExec::StopAfterHandler => {
                    return Ok(RunResult {
                        run_id,
                        phase: RunPhase::Running,
                        final_snapshot_id: Some(current_snapshot_id.clone()),
                    });
                }
                attempt::AttemptExec::Failed { retryable } => {
                    let next = attempt + 1;
                    if retryable && next < run_config.retry_policy.max_attempts {
                        let d = compute_backoff(&run_config.retry_policy.backoff, attempt);
                        if !d.is_zero() {
                            tokio::time::sleep(d).await;
                        }
                        attempt = next;
                        continue;
                    }

                    phase = RunPhase::Failed;
                    break;
                }
            }
        }

        if phase == RunPhase::Failed {
            break;
        }
    }

    let (status, final_snapshot_id) = match phase {
        RunPhase::Running | RunPhase::Completed => {
            (RunStatus::Completed, Some(current_snapshot_id.clone()))
        }
        RunPhase::Failed => (RunStatus::Failed, Some(current_snapshot_id.clone())),
        RunPhase::Cancelled => (RunStatus::Cancelled, Some(current_snapshot_id.clone())),
    };

    append_kernel(
        &writer,
        KernelEvent::RunCompleted {
            status: status.clone(),
            final_snapshot_id: final_snapshot_id.clone(),
        },
    )
    .await?;

    Ok(RunResult {
        run_id,
        phase: match status {
            RunStatus::Completed => RunPhase::Completed,
            RunStatus::Failed => RunPhase::Failed,
            RunStatus::Cancelled => RunPhase::Cancelled,
        },
        final_snapshot_id,
    })
}

#[async_trait]
impl ExecutionEngine for DefaultExecutionEngine {
    async fn start(&self, stores: Stores, run: StartRun) -> Result<RunResult, RunError> {
        validate_execution_mode(&run.run_config)?;
        validate_start_run_contract(&run)?;

        let exists = stores
            .artifacts
            .exists(&run.manifest_id)
            .await
            .map_err(RunError::Storage)?;
        if !exists {
            return Err(RunError::Storage(storage_not_found(
                "manifest_not_found",
                "manifest artifact was not found",
            )));
        }

        let run_id = RunId(uuid::Uuid::new_v4());

        // Store initial context snapshot as an artifact.
        let initial_snapshot = run.initial_context.dump().map_err(RunError::Context)?;
        let initial_snapshot_id =
            write_full_snapshot_value(stores.artifacts.as_ref(), initial_snapshot).await?;

        let writer: SharedEventWriter = Arc::new(Mutex::new(
            EventWriter::new(Arc::clone(&stores.events), run_id)
                .await
                .map_err(RunError::Storage)?,
        ));

        writer
            .lock()
            .await
            .append_kernel(KernelEvent::RunStarted {
                op_id: run.plan.op_id.clone(),
                manifest_id: run.manifest_id.clone(),
                initial_snapshot_id: initial_snapshot_id.clone(),
            })
            .await
            .map_err(RunError::Storage)?;

        let completed_states = HashSet::new();
        let current_snapshot_id = initial_snapshot_id.clone();
        let facts = FactIndex::default();

        run_states(
            &stores,
            &run.plan,
            &run.run_config,
            run_id,
            writer,
            current_snapshot_id,
            &completed_states,
            None,
            facts,
            Arc::clone(&self.live_transport_factory),
            self.failpoints.clone(),
        )
        .await
    }

    async fn resume(&self, stores: Stores, run_id: RunId) -> Result<RunResult, RunError> {
        let head = stores
            .events
            .head_seq(run_id)
            .await
            .map_err(RunError::Storage)?;
        if head == 0 {
            return Err(RunError::Storage(storage_not_found(
                "run_not_found",
                "run event stream was not found",
            )));
        }

        let stream = stores
            .events
            .read_range(run_id, 1, None)
            .await
            .map_err(RunError::Storage)?;

        let facts = FactIndex::from_event_stream(&stream);
        let history = read_run_history(run_id, &stream)?;

        if let Some((status, final_snapshot_id)) = &history.run_completed {
            return Ok(RunResult {
                run_id,
                phase: match status {
                    RunStatus::Completed => RunPhase::Completed,
                    RunStatus::Failed => RunPhase::Failed,
                    RunStatus::Cancelled => RunPhase::Cancelled,
                },
                final_snapshot_id: final_snapshot_id.clone(),
            });
        }

        let manifest =
            read_manifest(stores.artifacts.as_ref(), &history.started.manifest_id).await?;
        validate_execution_mode(&manifest.run_config)?;

        if history.started.op_id != manifest.op_id {
            return Err(invalid_plan(
                "run_started_op_id_mismatch",
                "RunStarted.op_id did not match manifest.op_id",
            ));
        }

        let plan = self.resolver.resolve(&manifest)?;
        if plan.op_id != manifest.op_id {
            return Err(invalid_plan(
                "plan_op_id_mismatch",
                "resolved plan.op_id did not match manifest.op_id",
            ));
        }

        let writer: SharedEventWriter = Arc::new(Mutex::new(
            EventWriter::new(Arc::clone(&stores.events), run_id)
                .await
                .map_err(RunError::Storage)?,
        ));

        // Orphan attempt handling: retry from base snapshot with attempt+1.
        if let Some(orphan) = &history.orphan_attempt {
            let start = (
                orphan.state_id.clone(),
                orphan.attempt + 1,
                orphan.base_snapshot_id.clone(),
            );
            return run_states(
                &stores,
                &plan,
                &manifest.run_config,
                run_id,
                writer,
                history.last_checkpoint.clone(),
                &history.completed_states,
                Some(start),
                facts.clone(),
                Arc::clone(&self.live_transport_factory),
                self.failpoints.clone(),
            )
            .await;
        }

        // If all states are done, finalize run.
        let ordered = topological_order(&plan)
            .map_err(|_| invalid_plan("invalid_plan", "execution plan failed validation"))?;
        let next_state = ordered
            .iter()
            .find(|n| !history.completed_states.contains(&n.id))
            .map(|n| n.id.clone());

        let Some(next_state_id) = next_state else {
            writer
                .lock()
                .await
                .append_kernel(KernelEvent::RunCompleted {
                    status: RunStatus::Completed,
                    final_snapshot_id: Some(history.last_checkpoint.clone()),
                })
                .await
                .map_err(RunError::Storage)?;
            return Ok(RunResult {
                run_id,
                phase: RunPhase::Completed,
                final_snapshot_id: Some(history.last_checkpoint.clone()),
            });
        };

        // Retry a previously failed state if retries remain; otherwise finalize failed run.
        if let Some((attempt, base_snapshot, retryable)) =
            history.last_failure_by_state.get(&next_state_id)
        {
            let next = attempt + 1;
            if !*retryable || next >= manifest.run_config.retry_policy.max_attempts {
                writer
                    .lock()
                    .await
                    .append_kernel(KernelEvent::RunCompleted {
                        status: RunStatus::Failed,
                        final_snapshot_id: Some(history.last_checkpoint.clone()),
                    })
                    .await
                    .map_err(RunError::Storage)?;
                return Ok(RunResult {
                    run_id,
                    phase: RunPhase::Failed,
                    final_snapshot_id: Some(history.last_checkpoint.clone()),
                });
            }

            let start = (next_state_id.clone(), next, base_snapshot.clone());
            return run_states(
                &stores,
                &plan,
                &manifest.run_config,
                run_id,
                writer,
                history.last_checkpoint.clone(),
                &history.completed_states,
                Some(start),
                facts.clone(),
                Arc::clone(&self.live_transport_factory),
                self.failpoints.clone(),
            )
            .await;
        }

        let start = (
            next_state_id.clone(),
            next_attempt(&history.last_attempt_by_state, &next_state_id),
            history.last_checkpoint.clone(),
        );
        run_states(
            &stores,
            &plan,
            &manifest.run_config,
            run_id,
            writer,
            history.last_checkpoint.clone(),
            &history.completed_states,
            Some(start),
            facts,
            Arc::clone(&self.live_transport_factory),
            self.failpoints.clone(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DynContext;
    use crate::context_runtime::JsonContext;
    use crate::errors::StateError;
    use crate::errors::StorageError;
    use crate::errors::{ErrorCategory, ErrorInfo, IoError};
    use crate::events::{DomainEvent, FactRecorded, DOMAIN_EVENT_FACT_RECORDED};
    use crate::hashing::artifact_id_for_bytes;
    use crate::ids::ContextKey;
    use crate::ids::ErrorCode;
    use crate::ids::FactKey;
    use crate::io::IoCall;
    use crate::io::IoProvider;
    use crate::live_io::{LiveIoTransport, LiveIoTransportFactory};
    use crate::meta::{
        standard_tags, DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag,
    };
    use crate::plan::StateGraph;
    use crate::recorder::EventRecorder;
    use crate::state::State;
    use crate::stores::{ArtifactKind, ArtifactStore, EventStore};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Clone, Default)]
    struct MemEventStore {
        inner: Arc<Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
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
        inner: Arc<Mutex<HashMap<ArtifactId, Vec<u8>>>>,
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
            inner
                .get(id)
                .cloned()
                .ok_or_else(|| storage_not_found("not_found", "artifact not found"))
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(self.inner.lock().await.contains_key(id))
        }
    }

    struct FixedResolver {
        plan: ExecutionPlan,
    }

    impl PlanResolver for FixedResolver {
        fn resolve(&self, _manifest: &RunManifest) -> Result<ExecutionPlan, RunError> {
            Ok(self.plan.clone())
        }
    }

    #[derive(Clone)]
    struct SetKeyState;

    #[async_trait]
    impl State for SetKeyState {
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
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            let existing = ctx.read(&ContextKey("x".to_string())).expect("read");
            if existing.is_some() {
                return Err(StateError {
                    state_id: None,
                    info: ErrorInfo {
                        code: ErrorCode("unexpected_context".to_string()),
                        category: ErrorCategory::Context,
                        retryable: false,
                        message: "unexpected context value".to_string(),
                        details: None,
                    },
                });
            }

            ctx.write(ContextKey("x".to_string()), serde_json::json!(1))
                .expect("write");
            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[derive(Clone)]
    struct TaggedWriteState {
        called: Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl State for TaggedWriteState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: vec![Tag(standard_tags::APPLY_SIDE_EFFECT.to_string())],
                depends_on: Vec::new(),
                depends_on_strategy: DependencyStrategy::Latest,
                side_effects: SideEffectKind::ApplySideEffect,
                idempotency: Idempotency::Key("test:tagged_write".to_string()),
            }
        }

        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            self.called
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ctx.write(ContextKey("y".to_string()), serde_json::json!(2))
                .expect("write");
            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[derive(Clone)]
    struct RecordFactAndWriteState;

    #[async_trait]
    impl State for RecordFactAndWriteState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
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
        ) -> Result<crate::state::StateOutcome, StateError> {
            let got = io
                .call(IoCall {
                    namespace: "test".to_string(),
                    request: serde_json::json!({"q": 1}),
                    fact_key: Some(FactKey("k".to_string())),
                })
                .await
                .expect("io");
            assert_eq!(got.response, serde_json::json!({ "n": 0 }));

            ctx.write(ContextKey("x".to_string()), serde_json::json!(1))
                .expect("write");

            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    fn base_run_config() -> RunConfig {
        RunConfig {
            io_mode: crate::config::IoMode::Live,
            retry_policy: crate::config::RetryPolicy {
                max_attempts: 3,
                backoff: BackoffPolicy::Fixed {
                    delay: Duration::from_millis(0),
                },
            },
            event_profile: crate::config::EventProfile::Minimal,
            execution_mode: ExecutionMode::Sequential,
            context_checkpointing: crate::config::ContextCheckpointing::AfterEveryState,
            replay_missing_fact_retryable: false,
            skip_tags: Vec::new(),
            nix_flake_allowlist: crate::config::default_nix_flake_allowlist(),
        }
    }

    fn replay_run_config() -> RunConfig {
        let mut cfg = base_run_config();
        cfg.io_mode = crate::config::IoMode::Replay;
        cfg
    }

    async fn store_manifest(artifacts: &dyn ArtifactStore, manifest: &RunManifest) -> ArtifactId {
        let value = serde_json::to_value(manifest).unwrap();
        let bytes = crate::hashing::canonical_json_bytes(&value).unwrap();
        artifacts.put(ArtifactKind::Manifest, bytes).await.unwrap()
    }

    const SECRET_MNEMONIC: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const SECRET_PASSWORD: &str = "hunter2";

    #[derive(Clone)]
    struct SecretErrorState;

    #[async_trait]
    impl State for SecretErrorState {
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
            _ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            Err(StateError {
                state_id: None,
                info: ErrorInfo {
                    code: ErrorCode("secret_error".to_string()),
                    category: ErrorCategory::Unknown,
                    retryable: false,
                    message: format!("password leaked: {SECRET_PASSWORD}"),
                    details: Some(serde_json::json!({
                        "private_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    })),
                },
            })
        }
    }

    #[derive(Clone)]
    struct EmitSecretDomainEventState;

    #[async_trait]
    impl State for EmitSecretDomainEventState {
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
            _ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            let res = rec
                .emit(DomainEvent {
                    name: "test".to_string(),
                    payload: serde_json::json!({ "password": SECRET_PASSWORD }),
                    payload_ref: None,
                })
                .await;

            if res.is_err() {
                return Err(StateError {
                    state_id: None,
                    info: ErrorInfo {
                        code: ErrorCode("emit_failed".to_string()),
                        category: ErrorCategory::Unknown,
                        retryable: false,
                        message: "emit failed".to_string(),
                        details: None,
                    },
                });
            }

            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[derive(Clone)]
    struct SecretFactIoState;

    #[async_trait]
    impl State for SecretFactIoState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: DependencyStrategy::Latest,
                side_effects: SideEffectKind::ReadOnlyIo,
                idempotency: Idempotency::None,
            }
        }

        async fn handle(
            &self,
            _ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            io.call(IoCall {
                namespace: "test".to_string(),
                request: serde_json::json!({}),
                fact_key: Some(FactKey("test:fact".to_string())),
            })
            .await
            .map_err(|_| StateError {
                state_id: None,
                info: ErrorInfo {
                    code: ErrorCode("io_failed".to_string()),
                    category: ErrorCategory::Unknown,
                    retryable: false,
                    message: "io failed".to_string(),
                    details: None,
                },
            })?;

            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[derive(Clone)]
    struct SecretTransportFactory;

    struct SecretTransport;

    #[async_trait]
    impl LiveIoTransport for SecretTransport {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            match call.namespace.as_str() {
                "test" => Ok(serde_json::json!({ "password": SECRET_PASSWORD })),
                _ => Ok(serde_json::json!({})),
            }
        }
    }

    impl LiveIoTransportFactory for SecretTransportFactory {
        fn make(&self, _env: crate::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(SecretTransport)
        }
    }

    async fn assert_no_secret_bytes_in_artifacts(artifacts: &MemArtifactStore, needles: &[&str]) {
        let inner = artifacts.inner.lock().await;
        for bytes in inner.values() {
            let s = String::from_utf8_lossy(bytes);
            for n in needles {
                assert!(!s.contains(n));
            }
        }
    }

    #[tokio::test]
    async fn secrets_in_initial_context_are_rejected_and_not_persisted() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = base_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id,
                    state: Arc::new(SetKeyState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver);

        let mut initial = JsonContext::new();
        initial
            .write(
                ContextKey("mnemonic".to_string()),
                serde_json::json!(SECRET_MNEMONIC),
            )
            .unwrap();

        let err = engine
            .start(
                stores,
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(initial),
                },
            )
            .await
            .unwrap_err();

        match err {
            RunError::Context(crate::errors::ContextError::Other(info)) => {
                assert_eq!(info.code.0, "secrets_detected");
            }
            other => panic!("unexpected error: {other:?}"),
        }

        // No event stream was created, and no artifact contains the secret.
        assert!(events.inner.lock().await.is_empty());
        assert_no_secret_bytes_in_artifacts(&artifacts, &[SECRET_MNEMONIC, "mnemonic"]).await;
    }

    #[tokio::test]
    async fn state_failed_error_messages_are_redacted_before_persisting() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = base_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id.clone(),
                    state: Arc::new(SecretErrorState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver);

        let r = engine
            .start(
                stores,
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect("start");

        assert_eq!(r.phase, RunPhase::Failed);

        let stream = events.read_range(r.run_id, 1, None).await.expect("read");
        let failed = stream.iter().find_map(|e| match &e.event {
            Event::Kernel(KernelEvent::StateFailed { error, .. }) => Some(error.clone()),
            _ => None,
        });
        let failed = failed.expect("StateFailed event");
        assert_eq!(failed.state_id, Some(state_id));
        assert_eq!(failed.info.message, "error details redacted");
        assert!(failed.info.details.is_none());

        // Persisted surfaces must not contain the leaked secret.
        let serialized = serde_json::to_string(&stream).unwrap();
        assert!(!serialized.contains(SECRET_PASSWORD));
        assert!(!serialized.to_ascii_lowercase().contains("private_key"));
    }

    #[tokio::test]
    async fn domain_events_with_secrets_are_rejected() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let mut run_config = base_run_config();
        run_config.event_profile = crate::config::EventProfile::Normal;

        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: StateId("machine.main.s1".to_string()),
                    state: Arc::new(EmitSecretDomainEventState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver);

        let r = engine
            .start(
                stores,
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect("start");

        assert_eq!(r.phase, RunPhase::Failed);

        let stream = events.read_range(r.run_id, 1, None).await.expect("read");
        assert!(!stream.iter().any(|e| matches!(e.event, Event::Domain(_))));

        let serialized = serde_json::to_string(&stream).unwrap();
        assert!(!serialized.contains(SECRET_PASSWORD));
        assert!(!serialized.to_ascii_lowercase().contains("password"));
    }

    #[tokio::test]
    async fn fact_payloads_with_secrets_are_rejected() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let mut run_config = base_run_config();
        run_config.event_profile = crate::config::EventProfile::Normal;

        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: StateId("machine.main.s1".to_string()),
                    state: Arc::new(SecretFactIoState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver)
            .with_live_transport_factory(Arc::new(SecretTransportFactory));

        let r = engine
            .start(
                stores,
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect("start");

        assert_eq!(r.phase, RunPhase::Failed);

        let stream = events.read_range(r.run_id, 1, None).await.expect("read");
        assert!(!stream.iter().any(|e| match &e.event {
            Event::Domain(de) => de.name == DOMAIN_EVENT_FACT_RECORDED,
            _ => false,
        }));

        // No persisted bytes should include the secret-bearing "password" field.
        let serialized = serde_json::to_string(&stream).unwrap();
        assert!(!serialized.to_ascii_lowercase().contains("password"));
        assert_no_secret_bytes_in_artifacts(&artifacts, &["password", SECRET_PASSWORD]).await;
    }

    #[tokio::test]
    async fn start_then_resume_retries_orphan_attempt_from_base_snapshot() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = base_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id.clone(),
                    state: Arc::new(SetKeyState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let failpoints = EngineFailpoints {
            stop_after_handler_once: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };
        let engine = DefaultExecutionEngine::new(resolver).with_failpoints(failpoints);

        let initial_ctx = Box::new(JsonContext::new());

        let r1 = engine
            .start(
                stores(),
                StartRun {
                    manifest: manifest.clone(),
                    manifest_id: manifest_id.clone(),
                    plan: plan.clone(),
                    run_config: run_config.clone(),
                    initial_context: initial_ctx,
                },
            )
            .await
            .expect("start");

        assert_eq!(r1.phase, RunPhase::Running);

        let r2 = engine.resume(stores(), r1.run_id).await.expect("resume");
        assert_eq!(r2.phase, RunPhase::Completed);
        let final_snapshot_id = r2.final_snapshot_id.expect("final snapshot");

        let snapshot = crate::context_runtime::read_full_snapshot_value(
            artifacts.as_ref(),
            &final_snapshot_id,
        )
        .await
        .expect("read snapshot");
        assert_eq!(snapshot, serde_json::json!({"x": 1}));

        let stream = events.read_range(r1.run_id, 1, None).await.expect("read");
        let entered: Vec<u32> = stream
            .iter()
            .filter_map(|e| match &e.event {
                Event::Kernel(KernelEvent::StateEntered {
                    state_id: sid,
                    attempt,
                    ..
                }) if sid == &state_id => Some(*attempt),
                _ => None,
            })
            .collect();
        assert_eq!(entered, vec![0, 1]);
    }

    #[derive(Clone)]
    struct FlakyRetryableState {
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl State for FlakyRetryableState {
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
            _ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                return Err(StateError {
                    state_id: None,
                    info: ErrorInfo {
                        code: ErrorCode("flaky".to_string()),
                        category: ErrorCategory::Unknown,
                        retryable: true,
                        message: "flaky".to_string(),
                        details: None,
                    },
                });
            }
            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[tokio::test]
    async fn retry_policy_retries_retryable_errors() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = base_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id.clone(),
                    state: Arc::new(FlakyRetryableState {
                        calls: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                    }),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver);

        let r = engine
            .start(
                stores(),
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect("start");
        assert_eq!(r.phase, RunPhase::Completed);

        let stream = events.read_range(r.run_id, 1, None).await.expect("read");
        let entered: Vec<u32> = stream
            .iter()
            .filter_map(|e| match &e.event {
                Event::Kernel(KernelEvent::StateEntered {
                    state_id: sid,
                    attempt,
                    ..
                }) if sid == &state_id => Some(*attempt),
                _ => None,
            })
            .collect();
        assert_eq!(entered, vec![0, 1]);
    }

    #[tokio::test]
    async fn rejects_fanout_join_execution_mode() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let mut run_config = base_run_config();
        run_config.execution_mode = ExecutionMode::FanOutJoin { max_concurrency: 2 };
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: StateId("machine.main.s1".to_string()),
                    state: Arc::new(SetKeyState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver);

        let err = engine
            .start(
                stores(),
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect_err("expected error");

        match err {
            RunError::InvalidPlan(info) => assert_eq!(info.code.0, CODE_UNSUPPORTED_EXECUTION_MODE),
            other => panic!("expected InvalidPlan, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn skip_tags_skips_tagged_states_without_running_handler() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let mut run_config = base_run_config();
        run_config.skip_tags = vec![Tag(standard_tags::APPLY_SIDE_EFFECT.to_string())];
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let called = Arc::new(std::sync::atomic::AtomicU32::new(0));

        let s1 = StateId("machine.main.s1".to_string());
        let s2 = StateId("machine.main.s2".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![
                    StateNode {
                        id: s1.clone(),
                        state: Arc::new(SetKeyState),
                    },
                    StateNode {
                        id: s2.clone(),
                        state: Arc::new(TaggedWriteState {
                            called: Arc::clone(&called),
                        }),
                    },
                ],
                edges: vec![DependencyEdge {
                    from: s1.clone(),
                    to: s2.clone(),
                }],
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver);

        let r = engine
            .start(
                stores(),
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect("start");

        assert_eq!(r.phase, RunPhase::Completed);
        assert_eq!(
            called.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "tagged state should be skipped"
        );

        let final_snapshot_id = r.final_snapshot_id.expect("final snapshot");
        let snapshot = crate::context_runtime::read_full_snapshot_value(
            artifacts.as_ref(),
            &final_snapshot_id,
        )
        .await
        .expect("read snapshot");
        assert_eq!(snapshot, serde_json::json!({"x": 1}));

        let stream = events.read_range(r.run_id, 1, None).await.expect("read");
        let mut s1_snapshot = None;
        let mut s2_enter_base = None;
        let mut s2_completed_snapshot = None;

        for e in &stream {
            match &e.event {
                Event::Kernel(KernelEvent::StateCompleted {
                    state_id,
                    context_snapshot_id,
                }) if state_id == &s1 => {
                    s1_snapshot = Some(context_snapshot_id.clone());
                }
                Event::Kernel(KernelEvent::StateEntered {
                    state_id,
                    base_snapshot_id,
                    ..
                }) if state_id == &s2 => {
                    s2_enter_base = Some(base_snapshot_id.clone());
                }
                Event::Kernel(KernelEvent::StateCompleted {
                    state_id,
                    context_snapshot_id,
                }) if state_id == &s2 => {
                    s2_completed_snapshot = Some(context_snapshot_id.clone());
                }
                _ => {}
            }
        }

        let s1_snapshot = s1_snapshot.expect("s1 snapshot");
        assert_eq!(s2_enter_base, Some(s1_snapshot.clone()));
        assert_eq!(s2_completed_snapshot, Some(s1_snapshot.clone()));
        assert_eq!(final_snapshot_id, s1_snapshot);
    }

    #[tokio::test]
    async fn crash_resume_orphan_attempt_reuses_facts() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = base_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(CountingTransportFactory {
            calls: Arc::clone(&calls),
        });

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id.clone(),
                    state: Arc::new(RecordFactAndWriteState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let failpoints = EngineFailpoints {
            stop_after_handler_once: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        };
        let engine = DefaultExecutionEngine::new(resolver)
            .with_live_transport_factory(factory)
            .with_failpoints(failpoints);

        let initial_ctx = Box::new(JsonContext::new());
        let r1 = engine
            .start(
                stores(),
                StartRun {
                    manifest: manifest.clone(),
                    manifest_id: manifest_id.clone(),
                    plan: plan.clone(),
                    run_config: run_config.clone(),
                    initial_context: initial_ctx,
                },
            )
            .await
            .expect("start");

        assert_eq!(r1.phase, RunPhase::Running);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let r2 = engine.resume(stores(), r1.run_id).await.expect("resume");
        assert_eq!(r2.phase, RunPhase::Completed);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let final_snapshot_id = r2.final_snapshot_id.expect("final snapshot");
        let snapshot = crate::context_runtime::read_full_snapshot_value(
            artifacts.as_ref(),
            &final_snapshot_id,
        )
        .await
        .expect("read snapshot");
        assert_eq!(snapshot, serde_json::json!({"x": 1}));

        let stream = events.read_range(r1.run_id, 1, None).await.expect("read");
        let entered: Vec<u32> = stream
            .iter()
            .filter_map(|e| match &e.event {
                Event::Kernel(KernelEvent::StateEntered {
                    state_id: sid,
                    attempt,
                    ..
                }) if sid == &state_id => Some(*attempt),
                _ => None,
            })
            .collect();
        assert_eq!(entered, vec![0, 1]);

        let facts: Vec<FactRecorded> = stream
            .iter()
            .filter_map(|e| match &e.event {
                Event::Domain(de) if de.name == DOMAIN_EVENT_FACT_RECORDED => {
                    serde_json::from_value::<FactRecorded>(de.payload.clone()).ok()
                }
                _ => None,
            })
            .collect();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key.0, "k");
    }

    struct CountingTransport {
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl LiveIoTransport for CountingTransport {
        async fn call(&mut self, _call: IoCall) -> Result<serde_json::Value, IoError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(serde_json::json!({ "n": n }))
        }
    }

    struct CountingTransportFactory {
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    impl LiveIoTransportFactory for CountingTransportFactory {
        fn make(&self, _env: crate::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(CountingTransport {
                calls: Arc::clone(&self.calls),
            })
        }
    }

    struct TrackingTransportFactory {
        makes: Arc<std::sync::atomic::AtomicU32>,
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    impl LiveIoTransportFactory for TrackingTransportFactory {
        fn make(&self, _env: crate::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            self.makes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::new(TrackingTransport {
                calls: Arc::clone(&self.calls),
            })
        }
    }

    struct TrackingTransport {
        calls: Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl LiveIoTransport for TrackingTransport {
        async fn call(&mut self, _call: IoCall) -> Result<serde_json::Value, IoError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(serde_json::json!({ "n": 999 }))
        }
    }

    #[derive(Clone)]
    struct RecordFactThenFailOnce {
        handled: Arc<std::sync::atomic::AtomicU32>,
    }

    #[async_trait]
    impl State for RecordFactThenFailOnce {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: DependencyStrategy::Latest,
                side_effects: SideEffectKind::ReadOnlyIo,
                idempotency: Idempotency::None,
            }
        }

        async fn handle(
            &self,
            _ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            let got = io
                .call(IoCall {
                    namespace: "test".to_string(),
                    request: serde_json::json!({"q": 1}),
                    fact_key: Some(FactKey("k".to_string())),
                })
                .await
                .expect("io");

            assert_eq!(got.response, serde_json::json!({ "n": 0 }));

            let n = self
                .handled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                return Err(StateError {
                    state_id: None,
                    info: ErrorInfo {
                        code: ErrorCode("flaky".to_string()),
                        category: ErrorCategory::Unknown,
                        retryable: true,
                        message: "flaky".to_string(),
                        details: None,
                    },
                });
            }

            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[tokio::test]
    async fn facts_are_single_assignment_and_reused_across_retries() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = base_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(CountingTransportFactory {
            calls: Arc::clone(&calls),
        });

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id.clone(),
                    state: Arc::new(RecordFactThenFailOnce {
                        handled: Arc::new(std::sync::atomic::AtomicU32::new(0)),
                    }),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);

        let r = engine
            .start(
                stores(),
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect("start");
        assert_eq!(r.phase, RunPhase::Completed);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "transport call should be deduped by fact key"
        );

        let stream = events.read_range(r.run_id, 1, None).await.expect("read");
        let facts: Vec<FactRecorded> = stream
            .iter()
            .filter_map(|e| match &e.event {
                Event::Domain(de) if de.name == DOMAIN_EVENT_FACT_RECORDED => {
                    serde_json::from_value::<FactRecorded>(de.payload.clone()).ok()
                }
                _ => None,
            })
            .collect();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key.0, "k");
    }

    #[derive(Clone)]
    struct TimeAndRandomState;

    #[async_trait]
    impl State for TimeAndRandomState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: DependencyStrategy::Latest,
                side_effects: SideEffectKind::ReadOnlyIo,
                idempotency: Idempotency::None,
            }
        }

        async fn handle(
            &self,
            _ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            let _t0 = io.now_millis().await.expect("time");
            let _t1 = io.now_millis().await.expect("time");
            let r = io.random_bytes(8).await.expect("random");
            assert_eq!(r.len(), 8);

            Ok(crate::state::StateOutcome {
                snapshot: crate::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[tokio::test]
    async fn time_and_random_are_recorded_as_facts() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = base_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id.clone(),
                    state: Arc::new(TimeAndRandomState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver);

        let r = engine
            .start(
                stores(),
                StartRun {
                    manifest,
                    manifest_id,
                    plan,
                    run_config,
                    initial_context: Box::new(JsonContext::new()),
                },
            )
            .await
            .expect("start");
        assert_eq!(r.phase, RunPhase::Completed);

        let stream = events.read_range(r.run_id, 1, None).await.expect("read");
        let facts: Vec<FactRecorded> = stream
            .iter()
            .filter_map(|e| match &e.event {
                Event::Domain(de) if de.name == DOMAIN_EVENT_FACT_RECORDED => {
                    serde_json::from_value::<FactRecorded>(de.payload.clone()).ok()
                }
                _ => None,
            })
            .collect();
        assert_eq!(facts.len(), 3);

        assert!(facts[0].key.0.starts_with("mfm:now_millis|"));
        assert!(facts[1].key.0.starts_with("mfm:now_millis|"));
        assert!(facts[2].key.0.starts_with("mfm:random_bytes|"));

        for fr in facts {
            assert!(artifacts.exists(&fr.payload_id).await.expect("exists"));
        }
    }

    #[tokio::test]
    async fn replay_mode_serves_recorded_facts_without_live_io() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = replay_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let initial_snapshot_id =
            write_full_snapshot_value(artifacts.as_ref(), serde_json::json!({}))
                .await
                .expect("initial snapshot");

        // Seed a recorded fact so the run can execute without hitting live IO.
        let payload = serde_json::json!({ "n": 0 });
        let bytes = crate::hashing::canonical_json_bytes(&payload).expect("canonical payload");
        let payload_id = artifacts
            .put(ArtifactKind::FactPayload, bytes)
            .await
            .expect("store payload");

        let domain = DomainEvent {
            name: DOMAIN_EVENT_FACT_RECORDED.to_string(),
            payload: serde_json::to_value(FactRecorded {
                key: FactKey("k".to_string()),
                payload_id,
                meta: serde_json::json!({}),
            })
            .expect("payload"),
            payload_ref: None,
        };

        let run_id = RunId(uuid::Uuid::new_v4());
        events
            .append(
                run_id,
                0,
                vec![
                    EventEnvelope {
                        run_id,
                        seq: 1,
                        ts_millis: None,
                        event: Event::Kernel(KernelEvent::RunStarted {
                            op_id: manifest.op_id.clone(),
                            manifest_id: manifest_id.clone(),
                            initial_snapshot_id: initial_snapshot_id.clone(),
                        }),
                    },
                    EventEnvelope {
                        run_id,
                        seq: 2,
                        ts_millis: None,
                        event: Event::Domain(domain),
                    },
                ],
            )
            .await
            .expect("seed run");

        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let makes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(TrackingTransportFactory {
            makes: Arc::clone(&makes),
            calls: Arc::clone(&calls),
        });

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id,
                    state: Arc::new(RecordFactAndWriteState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);

        let r = engine.resume(stores(), run_id).await.expect("resume");
        assert_eq!(r.phase, RunPhase::Completed);
        assert_eq!(makes.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[derive(Clone)]
    struct ReplayMissingFactState;

    #[async_trait]
    impl State for ReplayMissingFactState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: DependencyStrategy::Latest,
                side_effects: SideEffectKind::ReadOnlyIo,
                idempotency: Idempotency::None,
            }
        }

        async fn handle(
            &self,
            _ctx: &mut dyn DynContext,
            io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<crate::state::StateOutcome, StateError> {
            let err = io
                .call(IoCall {
                    namespace: "test".to_string(),
                    request: serde_json::json!({"q": 1}),
                    fact_key: Some(FactKey("k".to_string())),
                })
                .await
                .expect_err("missing fact");

            let info = match err {
                IoError::MissingFact { info, .. } => info,
                other => panic!("expected MissingFact, got: {other:?}"),
            };

            Err(StateError {
                state_id: None,
                info,
            })
        }
    }

    #[tokio::test]
    async fn replay_mode_missing_fact_fails_without_live_io() {
        let events = Arc::new(MemEventStore::default());
        let artifacts = Arc::new(MemArtifactStore::default());
        let stores = || Stores {
            events: events.clone(),
            artifacts: artifacts.clone(),
        };

        let run_config = replay_run_config();
        let manifest = RunManifest {
            op_id: OpId("op".to_string()),
            op_version: "0".to_string(),
            input_params: serde_json::json!({}),
            run_config: run_config.clone(),
            build: crate::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };
        let manifest_id = store_manifest(artifacts.as_ref(), &manifest).await;

        let initial_snapshot_id =
            write_full_snapshot_value(artifacts.as_ref(), serde_json::json!({}))
                .await
                .expect("initial snapshot");

        let run_id = RunId(uuid::Uuid::new_v4());
        events
            .append(
                run_id,
                0,
                vec![EventEnvelope {
                    run_id,
                    seq: 1,
                    ts_millis: None,
                    event: Event::Kernel(KernelEvent::RunStarted {
                        op_id: manifest.op_id.clone(),
                        manifest_id: manifest_id.clone(),
                        initial_snapshot_id,
                    }),
                }],
            )
            .await
            .expect("seed run");

        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let makes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(TrackingTransportFactory {
            makes: Arc::clone(&makes),
            calls: Arc::clone(&calls),
        });

        let state_id = StateId("machine.main.s1".to_string());
        let plan = ExecutionPlan {
            op_id: manifest.op_id.clone(),
            graph: StateGraph {
                states: vec![StateNode {
                    id: state_id,
                    state: Arc::new(ReplayMissingFactState),
                }],
                edges: Vec::new(),
            },
        };

        let resolver = Arc::new(FixedResolver { plan: plan.clone() });
        let engine = DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory);

        let r = engine.resume(stores(), run_id).await.expect("resume");
        assert_eq!(r.phase, RunPhase::Failed);
        assert_eq!(makes.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
