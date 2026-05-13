//! Unstable v4 runtime implementation (executor + resume logic).
//!
//! Source of truth: `docs/design.md` (v4).
//! Not part of the stable API contract (Appendix C.1).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, warn};

use crate::attempt_envelope::{analyze_kernel_events, OrphanAttempt};
use crate::config::{BackoffPolicy, ExecutionMode, RunConfig, RunManifest};
use crate::context_runtime::{read_canonical_json_artifact, write_full_snapshot_value};
use crate::engine::{ExecutionEngine, RunPhase, RunResult, StartRun, Stores};
use crate::errors::{ContextError, ErrorCategory, ErrorInfo, RunError, StateError, StorageError};
use crate::events::{
    event_envelopes_from_stream_records, Event, EventEnvelope, KernelEvent, RunStatus,
};
use crate::hashing::artifact_id_for_json;
use crate::ids::{ArtifactId, ErrorCode, OpId, RunId, StateId};
use crate::live_io::{FactIndex, LiveIoTransportFactory, UnimplementedLiveIoTransportFactory};
use crate::plan::{DependencyEdge, ExecutionPlan, PlanValidationError, StateNode};
use crate::stores::{ArtifactKind, ArtifactStore, StreamId};

mod attempt;
mod child_runs;
mod writer;

use writer::{append_kernel, EventWriter, SharedEventWriter};

pub use child_runs::ChildRunLiveIoTransportFactory;

const CODE_UNSUPPORTED_EXECUTION_MODE: &str = "unsupported_execution_mode";

/// Resolves an executable plan from a validated run manifest.
///
/// Runtime entry points use this trait to recreate a plan on `start` and `resume`
/// without hard-coding domain planners into the engine.
pub trait PlanResolver: Send + Sync {
    /// Builds the execution plan for `manifest`.
    fn resolve(&self, manifest: &RunManifest) -> Result<ExecutionPlan, RunError>;
}

/// Test-only hooks that let integration tests interrupt engine progress at known points.
#[derive(Clone, Default)]
pub struct EngineFailpoints {
    /// Stops the next state attempt after the handler returns but before the engine advances.
    ///
    /// The flag is single-use: once observed, it is reset to `false`.
    pub stop_after_handler_once: Arc<std::sync::atomic::AtomicBool>,
}

impl EngineFailpoints {
    fn should_stop_after_handler(&self) -> bool {
        self.stop_after_handler_once
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

/// Default runtime implementation for the unstable v4 execution engine.
///
/// The engine validates the run contract, rehydrates facts from prior events, and
/// executes states in topological order with retry/resume semantics.
#[derive(Clone)]
pub struct DefaultExecutionEngine {
    resolver: Arc<dyn PlanResolver>,
    live_transport_factory: Arc<dyn LiveIoTransportFactory>,
    failpoints: Option<EngineFailpoints>,
}

impl DefaultExecutionEngine {
    /// Creates an engine that resolves plans with `resolver` and rejects live IO by default.
    pub fn new(resolver: Arc<dyn PlanResolver>) -> Self {
        Self {
            resolver,
            live_transport_factory: Arc::new(UnimplementedLiveIoTransportFactory),
            failpoints: None,
        }
    }

    /// Installs the live IO transport factory used for state execution in live mode.
    pub fn with_live_transport_factory(mut self, factory: Arc<dyn LiveIoTransportFactory>) -> Self {
        self.live_transport_factory = factory;
        self
    }

    /// Enables test failpoints for the returned engine instance.
    pub fn with_failpoints(mut self, failpoints: EngineFailpoints) -> Self {
        self.failpoints = Some(failpoints);
        self
    }
}

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode::must_new(code),
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

fn storage_corruption(code: &'static str, message: &'static str) -> StorageError {
    StorageError::Corruption(info(code, ErrorCategory::Storage, message))
}

fn context_err(code: &'static str, message: &'static str) -> ContextError {
    ContextError::Serialization(info(code, ErrorCategory::Context, message))
}

pub(super) fn run_stream_id(run_id: RunId) -> StreamId {
    StreamId::run(run_id)
}

pub(super) async fn run_stream_head(stores: &Stores, run_id: RunId) -> Result<u64, RunError> {
    stores
        .streams
        .head_seq(&run_stream_id(run_id))
        .await
        .map_err(RunError::Storage)
}

pub(super) async fn read_run_stream(
    stores: &Stores,
    run_id: RunId,
) -> Result<Vec<EventEnvelope>, RunError> {
    let records = stores
        .streams
        .read_range(&run_stream_id(run_id), 1, None)
        .await
        .map_err(RunError::Storage)?;
    event_envelopes_from_stream_records(run_id, records).map_err(RunError::Storage)
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
            "execution_mode FanOutJoin is not supported",
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
            "run manifest contained secrets (policy forbids persisting secrets)",
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

fn validate_plan(plan: &ExecutionPlan) -> Result<Vec<StateNode>, RunError> {
    topological_order(plan)
        .map_err(|_| invalid_plan("invalid_plan", "execution plan failed validation"))
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

fn validate_recovery_history(history: &RunHistory, ordered: &[StateNode]) -> Result<(), RunError> {
    let plan_states: HashSet<StateId> = ordered.iter().map(|node| node.id.clone()).collect();

    for state_id in &history.completed_states {
        if !plan_states.contains(state_id) {
            return Err(invalid_plan(
                "recovery_state_not_in_plan",
                "completed state from run history was not present in resolved plan",
            ));
        }
    }

    for state_id in history.last_failure_by_state.keys() {
        if !plan_states.contains(state_id) {
            return Err(invalid_plan(
                "recovery_state_not_in_plan",
                "failed state from run history was not present in resolved plan",
            ));
        }
    }

    for state_id in history.last_attempt_by_state.keys() {
        if !plan_states.contains(state_id) {
            return Err(invalid_plan(
                "recovery_state_not_in_plan",
                "attempted state from run history was not present in resolved plan",
            ));
        }
    }

    if let Some(orphan) = &history.orphan_attempt {
        if !plan_states.contains(&orphan.state_id) {
            return Err(invalid_plan(
                "recovery_state_not_in_plan",
                "orphan attempt state from run history was not present in resolved plan",
            ));
        }
    }

    Ok(())
}

async fn read_manifest(
    artifacts: &dyn ArtifactStore,
    manifest_id: &ArtifactId,
) -> Result<RunManifest, RunError> {
    let value =
        read_canonical_json_artifact(artifacts, ArtifactKind::Manifest, manifest_id).await?;

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

pub(in crate::runtime) async fn close_orphan_attempt_for_recovery(
    writer: &SharedEventWriter,
    orphan: &OrphanAttempt,
) -> Result<(), RunError> {
    warn!(
        state_id = %orphan.state_id,
        attempt = orphan.attempt,
        entered_seq = orphan.entered_seq,
        "closing orphaned state attempt before retry"
    );
    append_kernel(
        writer,
        KernelEvent::StateFailed {
            state_id: orphan.state_id.clone(),
            error: StateError {
                state_id: Some(orphan.state_id.clone()),
                info: ErrorInfo {
                    code: ErrorCode::must_new("orphan_attempt_recovered"),
                    category: ErrorCategory::Unknown,
                    retryable: true,
                    message: "orphaned state attempt recovered during resume".to_string(),
                    details: None,
                },
            },
            failure_snapshot_id: None,
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[instrument(
    level = "info",
    skip(
        stores,
        plan,
        run_config,
        writer,
        completed_states,
        start_at_state,
        facts,
        live_factory,
        failpoints
    ),
    fields(run_id = %run_id.0, op_id = %plan.op_id)
)]
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
    debug!(execution_mode = ?run_config.execution_mode, "running execution plan");

    let ordered = validate_plan(plan)?;
    debug!(
        state_count = ordered.len(),
        "execution plan resolved to topological order"
    );

    if let Some((state_id, _, _)) = &start_at_state {
        if !ordered.iter().any(|node| &node.id == state_id) {
            return Err(invalid_plan(
                "start_state_not_in_plan",
                "start_at_state was not present in execution plan",
            ));
        }
        if completed_states.contains(state_id) {
            return Err(invalid_plan(
                "start_state_already_completed",
                "start_at_state was already completed in run history",
            ));
        }
    }

    let mut found_start = start_at_state.is_none();
    let mut phase = RunPhase::Running;

    for node in ordered {
        if completed_states.contains(&node.id) {
            debug!(state_id = %node.id, "state already completed, skipping");
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
        info!(state_id = %state_id, attempt, "starting state execution");

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
                    info!(
                        state_id = %state_id,
                        attempt,
                        snapshot_id = %current_snapshot_id,
                        "state execution completed"
                    );
                    break;
                }
                attempt::AttemptExec::StopAfterHandler => {
                    warn!(
                        state_id = %state_id,
                        attempt,
                        "execution stopped after handler due to failpoint"
                    );
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
                        warn!(
                            state_id = %state_id,
                            attempt,
                            next_attempt = next,
                            backoff_ms = d.as_millis() as u64,
                            "state failed and will be retried"
                        );
                        if !d.is_zero() {
                            tokio::time::sleep(d).await;
                        }
                        attempt = next;
                        continue;
                    }

                    phase = RunPhase::Failed;
                    warn!(
                        state_id = %state_id,
                        attempt,
                        retryable,
                        max_attempts = run_config.retry_policy.max_attempts,
                        "state failed and no retries remain"
                    );
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
    info!(
        phase = ?phase,
        final_snapshot_id = final_snapshot_id.as_ref().map(|id| id.as_str()),
        "run completed and finalized"
    );

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
    #[instrument(level = "info", skip(self, stores, run), fields(op_id = %run.plan.op_id))]
    async fn start(&self, stores: Stores, run: StartRun) -> Result<RunResult, RunError> {
        validate_execution_mode(&run.run_config)?;
        validate_start_run_contract(&run)?;
        validate_plan(&run.plan)?;

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
        let stored_manifest = read_manifest(stores.artifacts.as_ref(), &run.manifest_id).await?;
        if stored_manifest != run.manifest {
            return Err(RunError::Storage(storage_corruption(
                "manifest_artifact_mismatch",
                "stored manifest did not match start request manifest",
            )));
        }
        info!(manifest_id = %run.manifest_id, "starting run");

        let run_id = RunId(uuid::Uuid::new_v4());

        // Store initial context snapshot as an artifact.
        let initial_snapshot = run.initial_context.dump().map_err(RunError::Context)?;
        let initial_snapshot_id =
            write_full_snapshot_value(stores.artifacts.as_ref(), initial_snapshot).await?;

        let writer: SharedEventWriter = Arc::new(Mutex::new(
            EventWriter::new(Arc::clone(&stores.streams), run_id)
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
        info!(
            run_id = %run_id.0,
            initial_snapshot_id = %initial_snapshot_id,
            "run started event appended"
        );

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

    #[instrument(level = "info", skip(self, stores), fields(run_id = %run_id.0))]
    async fn resume(&self, stores: Stores, run_id: RunId) -> Result<RunResult, RunError> {
        let head = run_stream_head(&stores, run_id).await?;
        if head == 0 {
            return Err(RunError::Storage(storage_not_found(
                "run_not_found",
                "run stream was not found",
            )));
        }
        debug!(head_seq = head, "resuming run from event stream");

        let stream = read_run_stream(&stores, run_id).await?;

        let facts = FactIndex::from_event_stream(&stream)?;
        let history = read_run_history(run_id, &stream)?;
        debug!(
            completed_state_count = history.completed_states.len(),
            "loaded run history for resume"
        );

        let manifest =
            read_manifest(stores.artifacts.as_ref(), &history.started.manifest_id).await?;
        validate_execution_mode(&manifest.run_config)?;

        if history.started.op_id != manifest.op_id {
            return Err(invalid_plan(
                "run_started_op_id_mismatch",
                "RunStarted.op_id did not match manifest.op_id",
            ));
        }
        debug!(op_id = %manifest.op_id, "manifest loaded for resume");

        let plan = self.resolver.resolve(&manifest)?;
        if plan.op_id != manifest.op_id {
            return Err(invalid_plan(
                "plan_op_id_mismatch",
                "resolved plan.op_id did not match manifest.op_id",
            ));
        }
        let ordered = validate_plan(&plan)?;
        validate_recovery_history(&history, &ordered)?;

        if let Some((status, final_snapshot_id)) = &history.run_completed {
            info!(
                status = ?status,
                final_snapshot_id = final_snapshot_id.as_ref().map(|id| id.as_str()),
                "run already completed; resume returns existing terminal state"
            );
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

        let writer: SharedEventWriter = Arc::new(Mutex::new(
            EventWriter::new(Arc::clone(&stores.streams), run_id)
                .await
                .map_err(RunError::Storage)?,
        ));

        // Orphan attempt handling: retry from base snapshot with attempt+1.
        if let Some(orphan) = &history.orphan_attempt {
            warn!(
                state_id = %orphan.state_id,
                previous_attempt = orphan.attempt,
                "retrying orphan attempt from base snapshot"
            );
            close_orphan_attempt_for_recovery(&writer, orphan).await?;
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
        let next_state = ordered
            .iter()
            .find(|n| !history.completed_states.contains(&n.id))
            .map(|n| n.id.clone());

        let Some(next_state_id) = next_state else {
            info!("all states already completed; finalizing run");
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
                warn!(
                    state_id = %next_state_id,
                    attempt = *attempt,
                    retryable = *retryable,
                    max_attempts = manifest.run_config.retry_policy.max_attempts,
                    "resume cannot retry failed state; finalizing run as failed"
                );
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

            info!(
                state_id = %next_state_id,
                next_attempt = next,
                base_snapshot_id = %base_snapshot,
                "resuming from failed state with retry"
            );
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
        info!(
            state_id = %next_state_id,
            attempt = start.1,
            base_snapshot_id = %history.last_checkpoint,
            "resuming run at next state"
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
#[path = "tests/runtime_tests.rs"]
mod runtime_tests;
