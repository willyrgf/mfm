//! Unstable helper implementations for planning and launching (Milestone 1).
//!
//! Source of truth: `REDESIGN.md` (v4).
//! Not part of the stable API contract (Appendix C.2).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;

use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig, RunManifest,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase, RunResult, StartRun, Stores};
use mfm_machine::errors::{
    ContextError, ErrorCategory, ErrorInfo, IoError, RunError, StorageError,
};
use mfm_machine::events::{Event, KernelEvent};
use mfm_machine::hashing::{
    artifact_id_for_bytes, artifact_id_for_json, canonical_json_bytes, CanonicalJsonError,
};
use mfm_machine::ids::{ContextKey, ErrorCode, OpId, OpPath, RunId, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::{Idempotency, SideEffectKind, StateMeta};
use mfm_machine::plan::{DependencyEdge, ExecutionPlan, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{DynState, State, StateOutcome};
use mfm_machine::stores::ArtifactKind;

use crate::errors::SdkError;
use crate::ids::{MachineId, PortKey, StepId};
use crate::launcher::{LaunchPipeline, RunLauncher};
use crate::op::{DynOperation, OpIo, OperationRegistry};
use crate::pipeline::{Pipeline, PipelineManifestInput, PipelinePlanner, PipelineStep};

fn info(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: &'static str,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details: None,
    }
}

fn sdk_error(code: &'static str, category: ErrorCategory, message: &'static str) -> SdkError {
    SdkError {
        info: info(code, category, false, message),
    }
}

fn invalid_segment(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 63 {
        return true;
    }
    if !b[0].is_ascii_lowercase() {
        return true;
    }
    for &c in &b[1..] {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_') {
            return true;
        }
    }
    false
}

fn validate_segment(code: &'static str, s: &str, message: &'static str) -> Result<(), SdkError> {
    if invalid_segment(s) {
        return Err(sdk_error(code, ErrorCategory::ParsingInput, message));
    }
    Ok(())
}

fn validate_machine_id(id: &MachineId) -> Result<(), SdkError> {
    validate_segment(
        "invalid_machine_id",
        &id.0,
        "machine_id must match ^[a-z][a-z0-9_]{0,62}$",
    )
}

fn validate_step_id(id: &StepId) -> Result<(), SdkError> {
    validate_segment(
        "invalid_step_id",
        &id.0,
        "step_id must match ^[a-z][a-z0-9_]{0,62}$",
    )
}

fn validate_pipeline(p: &Pipeline) -> Result<(), SdkError> {
    validate_machine_id(&p.machine_id)?;
    if p.steps.is_empty() {
        return Err(sdk_error(
            "empty_pipeline",
            ErrorCategory::ParsingInput,
            "pipeline must contain at least one step",
        ));
    }

    let mut seen = HashSet::new();
    for s in &p.steps {
        validate_step_id(&s.step_id)?;
        if !seen.insert(s.step_id.0.clone()) {
            return Err(sdk_error(
                "duplicate_step_id",
                ErrorCategory::ParsingInput,
                "pipeline step_id must be unique",
            ));
        }

        // Enforce Milestone 1: op_config must be canonical-JSON hashable (floats forbidden).
        canonical_json_bytes(&s.op_config).map_err(|e| match e {
            CanonicalJsonError::FloatNotAllowed => sdk_error(
                "op_config_not_canonical",
                ErrorCategory::ParsingInput,
                "op_config is not canonical-json-hashable (floats are forbidden)",
            ),
            CanonicalJsonError::SecretsNotAllowed => sdk_error(
                "secrets_detected",
                ErrorCategory::ParsingInput,
                "op_config contained secrets (Milestone 1 forbids persisting secrets)",
            ),
        })?;
    }

    Ok(())
}

fn op_path(machine_id: &MachineId, step_id: &StepId) -> OpPath {
    OpPath(format!("{}.{}", machine_id.0, step_id.0))
}

fn validate_state_id_shape(
    state_id: &StateId,
    machine_id: &MachineId,
    step_id: &StepId,
) -> Result<(), SdkError> {
    let mut it = state_id.0.split('.');
    let Some(m) = it.next() else {
        return Err(sdk_error(
            "invalid_state_id",
            ErrorCategory::ParsingInput,
            "state_id must have 3 dot-separated segments",
        ));
    };
    let Some(s) = it.next() else {
        return Err(sdk_error(
            "invalid_state_id",
            ErrorCategory::ParsingInput,
            "state_id must have 3 dot-separated segments",
        ));
    };
    let Some(local) = it.next() else {
        return Err(sdk_error(
            "invalid_state_id",
            ErrorCategory::ParsingInput,
            "state_id must have 3 dot-separated segments",
        ));
    };
    if it.next().is_some() {
        return Err(sdk_error(
            "invalid_state_id",
            ErrorCategory::ParsingInput,
            "state_id must have 3 dot-separated segments",
        ));
    }

    if m != machine_id.0 || s != step_id.0 {
        return Err(sdk_error(
            "invalid_state_id",
            ErrorCategory::ParsingInput,
            "state_id did not match the expected <machine_id>.<step_id> prefix",
        ));
    }

    // Enforce segment rules.
    validate_machine_id(machine_id)?;
    validate_step_id(step_id)?;
    validate_segment(
        "invalid_state_local_id",
        local,
        "state_local_id must match ^[a-z][a-z0-9_]{0,62}$",
    )?;

    Ok(())
}

fn validate_state_graph(
    g: &StateGraph,
    machine_id: &MachineId,
    step_id: &StepId,
) -> Result<(), SdkError> {
    if g.states.is_empty() {
        return Err(sdk_error(
            "empty_state_graph",
            ErrorCategory::ParsingInput,
            "operation expanded to an empty state graph",
        ));
    }

    let mut ids = HashSet::new();
    for n in &g.states {
        validate_state_id_shape(&n.id, machine_id, step_id)?;
        let meta = n.state.meta();
        if meta.side_effects == SideEffectKind::ApplySideEffect {
            let ok = matches!(&meta.idempotency, Idempotency::Key(k) if !k.is_empty());
            if !ok {
                return Err(SdkError {
                    info: ErrorInfo {
                        code: ErrorCode("missing_idempotency_key".to_string()),
                        category: ErrorCategory::ParsingInput,
                        retryable: false,
                        message: format!(
                            "apply_side_effect state must declare Idempotency::Key: {}",
                            n.id.0
                        ),
                        details: None,
                    },
                });
            }
        }
        if !ids.insert(n.id.clone()) {
            return Err(sdk_error(
                "duplicate_state_id",
                ErrorCategory::ParsingInput,
                "duplicate StateId in expanded graph",
            ));
        }
    }

    for DependencyEdge { from, to } in &g.edges {
        if !ids.contains(from) || !ids.contains(to) {
            return Err(sdk_error(
                "missing_state_for_edge",
                ErrorCategory::ParsingInput,
                "edge referenced a missing state id",
            ));
        }
    }

    Ok(())
}

fn sources_and_sinks(g: &StateGraph) -> (Vec<StateId>, Vec<StateId>) {
    let mut indeg: HashMap<StateId, usize> = HashMap::new();
    let mut outdeg: HashMap<StateId, usize> = HashMap::new();

    for s in &g.states {
        indeg.insert(s.id.clone(), 0);
        outdeg.insert(s.id.clone(), 0);
    }
    for DependencyEdge { from, to } in &g.edges {
        *outdeg.get_mut(from).expect("from exists") += 1;
        *indeg.get_mut(to).expect("to exists") += 1;
    }

    let sources: Vec<StateId> = g
        .states
        .iter()
        .filter(|n| indeg.get(&n.id).copied().unwrap_or(0) == 0)
        .map(|n| n.id.clone())
        .collect();
    let sinks: Vec<StateId> = g
        .states
        .iter()
        .filter(|n| outdeg.get(&n.id).copied().unwrap_or(0) == 0)
        .map(|n| n.id.clone())
        .collect();

    (sources, sinks)
}

/// A simple `OperationRegistry` implementation backed by a hash map.
#[derive(Default)]
pub struct HashMapOperationRegistry {
    ops: HashMap<(OpId, String), DynOperation>,
}

impl HashMapOperationRegistry {
    pub fn register(&mut self, op: DynOperation) {
        self.ops
            .insert((op.op_id(), op.op_version().to_string()), op);
    }
}

impl OperationRegistry for HashMapOperationRegistry {
    fn resolve(&self, op_id: &OpId, op_version: &str) -> Result<DynOperation, SdkError> {
        self.ops
            .get(&(op_id.clone(), op_version.to_string()))
            .cloned()
            .ok_or_else(|| {
                sdk_error(
                    "op_not_found",
                    ErrorCategory::ParsingInput,
                    "operation was not found in registry",
                )
            })
    }
}

/// Default Milestone 1 pipeline planner.
#[derive(Clone, Default)]
pub struct DefaultPipelinePlanner;

impl PipelinePlanner for DefaultPipelinePlanner {
    fn build_execution_plan(
        &self,
        registry: Arc<dyn OperationRegistry>,
        pipeline: &Pipeline,
        run_config: &mfm_machine::config::RunConfig,
    ) -> Result<ExecutionPlan, SdkError> {
        validate_pipeline(pipeline)?;

        let mut all_states: Vec<StateNode> = Vec::new();
        let mut all_edges: Vec<DependencyEdge> = Vec::new();
        let mut seen_state_ids: HashSet<StateId> = HashSet::new();

        // For cross-op wiring: PortKey -> exporting OpPath (last writer wins).
        let mut exports_by_port: HashMap<String, String> = HashMap::new();

        // For step-order enforcement:
        // Step i sinks must all happen before step i+1 sources.
        let mut step_sources: Vec<Vec<StateId>> = Vec::new();
        let mut step_sinks: Vec<Vec<StateId>> = Vec::new();

        for PipelineStep {
            step_id,
            op_id,
            op_version,
            op_config,
        } in &pipeline.steps
        {
            let op = registry.resolve(op_id, op_version)?;
            let op_path = op_path(&pipeline.machine_id, step_id);

            // Discover imports/exports for wiring validation.
            let OpIo { imports, exports } = op.io(op_config)?;

            let mut import_sources: HashMap<String, String> = HashMap::new();
            for PortKey(k) in imports {
                let Some(src) = exports_by_port.get(&k) else {
                    return Err(sdk_error(
                        "unsatisfied_import",
                        ErrorCategory::ParsingInput,
                        "pipeline import was not satisfiable by previous exports",
                    ));
                };
                import_sources.insert(k, src.clone());
            }

            let g = op.expand(op_path.clone(), op_config, run_config)?;
            validate_state_graph(&g, &pipeline.machine_id, step_id)?;

            // Wrap states to enforce default context namespacing and import wiring.
            for n in &g.states {
                if !seen_state_ids.insert(n.id.clone()) {
                    return Err(sdk_error(
                        "duplicate_state_id",
                        ErrorCategory::ParsingInput,
                        "duplicate StateId across pipeline steps",
                    ));
                }

                all_states.push(StateNode {
                    id: n.id.clone(),
                    state: Arc::new(NamespacedState {
                        op_path: op_path.clone(),
                        import_sources: import_sources.clone(),
                        inner: Arc::clone(&n.state),
                    }),
                });
            }
            all_edges.extend(g.edges.clone());

            let (sources, sinks) = sources_and_sinks(&g);
            step_sources.push(sources);
            step_sinks.push(sinks);

            // Update exports after expanding to keep "last writer wins" semantics stable.
            for PortKey(k) in exports {
                exports_by_port.insert(k, op_path.0.clone());
            }
        }

        // Enforce step order: sinks(step i) -> sources(step i+1).
        for i in 0..step_sources.len().saturating_sub(1) {
            for from in &step_sinks[i] {
                for to in &step_sources[i + 1] {
                    all_edges.push(DependencyEdge {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
            }
        }

        Ok(ExecutionPlan {
            op_id: OpId(pipeline.machine_id.0.clone()),
            graph: StateGraph {
                states: all_states,
                edges: all_edges,
            },
        })
    }
}

struct NamespacedContext<'a> {
    op_path: &'a OpPath,
    import_sources: &'a HashMap<String, String>,
    inner: &'a mut dyn DynContext,
}

impl NamespacedContext<'_> {
    fn qualify_local(&self, key: &ContextKey) -> ContextKey {
        ContextKey(format!("{}.{}", self.op_path.0, key.0))
    }

    fn qualify_read(&self, key: &ContextKey) -> ContextKey {
        if let Some(src) = self.import_sources.get(&key.0) {
            ContextKey(format!("{src}.{}", key.0))
        } else {
            self.qualify_local(key)
        }
    }
}

impl DynContext for NamespacedContext<'_> {
    fn read(
        &self,
        key: &ContextKey,
    ) -> Result<Option<serde_json::Value>, mfm_machine::errors::ContextError> {
        self.inner.read(&self.qualify_read(key))
    }

    fn write(
        &mut self,
        key: ContextKey,
        value: serde_json::Value,
    ) -> Result<(), mfm_machine::errors::ContextError> {
        self.inner.write(self.qualify_local(&key), value)
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), mfm_machine::errors::ContextError> {
        self.inner.delete(&self.qualify_local(key))
    }

    fn dump(&self) -> Result<serde_json::Value, mfm_machine::errors::ContextError> {
        let v = self.inner.dump()?;
        let serde_json::Value::Object(m) = v else {
            return Ok(v);
        };

        let prefix = format!("{}.", self.op_path.0);
        let mut out = serde_json::Map::new();
        for (k, v) in m {
            if let Some(stripped) = k.strip_prefix(&prefix) {
                out.insert(stripped.to_string(), v);
            }
        }
        Ok(serde_json::Value::Object(out))
    }
}

struct NamespacedState {
    op_path: OpPath,
    import_sources: HashMap<String, String>,
    inner: DynState,
}

#[async_trait]
impl State for NamespacedState {
    fn meta(&self) -> StateMeta {
        self.inner.meta()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, mfm_machine::errors::StateError> {
        let mut ns = NamespacedContext {
            op_path: &self.op_path,
            import_sources: &self.import_sources,
            inner: ctx,
        };
        self.inner.handle(&mut ns, io, rec).await
    }
}

/// Default Milestone 1 run launcher.
#[derive(Clone, Default)]
pub struct DefaultRunLauncher;

#[async_trait]
impl RunLauncher for DefaultRunLauncher {
    async fn start_pipeline(
        &self,
        engine: Arc<dyn ExecutionEngine>,
        stores: Stores,
        registry: Arc<dyn OperationRegistry>,
        planner: Arc<dyn PipelinePlanner>,
        req: LaunchPipeline,
    ) -> Result<RunResult, RunError> {
        let plan = planner
            .build_execution_plan(Arc::clone(&registry), &req.pipeline, &req.run_config)
            .map_err(|e| RunError::InvalidPlan(e.info))?;

        let input_params = serde_json::to_value(PipelineManifestInput {
            pipeline: req.pipeline.clone(),
            input: req.input,
        })
        .map_err(|_| {
            RunError::InvalidPlan(info(
                "manifest_input_serialize_failed",
                ErrorCategory::ParsingInput,
                false,
                "failed to serialize pipeline manifest input",
            ))
        })?;

        let manifest = RunManifest {
            op_id: OpId(req.pipeline.machine_id.0.clone()),
            op_version: req.pipeline.pipeline_version.clone(),
            input_params,
            run_config: req.run_config.clone(),
            build: req.build,
        };

        let value = serde_json::to_value(&manifest).map_err(|_| {
            RunError::InvalidPlan(info(
                "manifest_serialize_failed",
                ErrorCategory::ParsingInput,
                false,
                "failed to serialize run manifest",
            ))
        })?;

        // Store the manifest as canonical JSON bytes so `manifest_id` matches engine validation.
        let bytes = canonical_json_bytes(&value).map_err(|e| match e {
            CanonicalJsonError::FloatNotAllowed => RunError::InvalidPlan(info(
                "manifest_not_canonical",
                ErrorCategory::ParsingInput,
                false,
                "run manifest is not canonical-json-hashable (floats are forbidden)",
            )),
            CanonicalJsonError::SecretsNotAllowed => RunError::InvalidPlan(info(
                "secrets_detected",
                ErrorCategory::ParsingInput,
                false,
                "run manifest contained secrets (Milestone 1 forbids persisting secrets)",
            )),
        })?;
        let computed_id = artifact_id_for_bytes(&bytes);
        let computed_from_value = artifact_id_for_json(&value).map_err(|e| match e {
            CanonicalJsonError::FloatNotAllowed => RunError::InvalidPlan(info(
                "manifest_not_canonical",
                ErrorCategory::ParsingInput,
                false,
                "run manifest is not canonical-json-hashable (floats are forbidden)",
            )),
            CanonicalJsonError::SecretsNotAllowed => RunError::InvalidPlan(info(
                "secrets_detected",
                ErrorCategory::ParsingInput,
                false,
                "run manifest contained secrets (Milestone 1 forbids persisting secrets)",
            )),
        })?;
        debug_assert_eq!(computed_id, computed_from_value);

        let stored_id = stores
            .artifacts
            .put(ArtifactKind::Manifest, bytes)
            .await
            .map_err(RunError::Storage)?;
        if stored_id != computed_id {
            return Err(RunError::Storage(
                mfm_machine::errors::StorageError::Corruption(info(
                    "manifest_id_mismatch",
                    ErrorCategory::Storage,
                    false,
                    "artifact store returned unexpected manifest id",
                )),
            ));
        }

        engine
            .start(
                stores,
                StartRun {
                    manifest,
                    manifest_id: stored_id,
                    plan,
                    run_config: req.run_config,
                    initial_context: req.initial_context,
                },
            )
            .await
    }

    async fn resume(
        &self,
        engine: Arc<dyn ExecutionEngine>,
        stores: Stores,
        _registry: Arc<dyn OperationRegistry>,
        _planner: Arc<dyn PipelinePlanner>,
        run_id: RunId,
    ) -> Result<RunResult, RunError> {
        engine.resume(stores, run_id).await
    }
}

/// A `mfm-machine` runtime plan resolver that rebuilds the execution plan from the stored manifest.
///
/// This is used by `DefaultExecutionEngine` during `resume()`.
pub struct SdkPlanResolver {
    registry: Arc<dyn OperationRegistry>,
    planner: Arc<dyn PipelinePlanner>,
}

impl SdkPlanResolver {
    pub fn new(registry: Arc<dyn OperationRegistry>, planner: Arc<dyn PipelinePlanner>) -> Self {
        Self { registry, planner }
    }
}

impl mfm_machine::runtime::PlanResolver for SdkPlanResolver {
    fn resolve(&self, manifest: &RunManifest) -> Result<ExecutionPlan, RunError> {
        let parsed = serde_json::from_value::<PipelineManifestInput>(manifest.input_params.clone())
            .map_err(|_| {
                RunError::InvalidPlan(info(
                    "manifest_input_deserialize_failed",
                    ErrorCategory::ParsingInput,
                    false,
                    "failed to deserialize PipelineManifestInput from manifest.input_params",
                ))
            })?;

        let PipelineManifestInput { pipeline, .. } = parsed;
        if OpId(pipeline.machine_id.0.clone()) != manifest.op_id {
            return Err(RunError::InvalidPlan(info(
                "manifest_op_id_mismatch",
                ErrorCategory::ParsingInput,
                false,
                "manifest.op_id did not match pipeline.machine_id",
            )));
        }
        if pipeline.pipeline_version != manifest.op_version {
            return Err(RunError::InvalidPlan(info(
                "manifest_op_version_mismatch",
                ErrorCategory::ParsingInput,
                false,
                "manifest.op_version did not match pipeline.pipeline_version",
            )));
        }

        self.planner
            .build_execution_plan(Arc::clone(&self.registry), &pipeline, &manifest.run_config)
            .map_err(|e| RunError::InvalidPlan(e.info))
    }
}

/// Helper for the Milestone 1 single-op run convention.
pub fn single_op_pipeline(
    op_id: OpId,
    op_version: String,
    op_config: serde_json::Value,
) -> Result<Pipeline, SdkError> {
    let machine_id = MachineId(op_id.0.clone());
    validate_machine_id(&machine_id)?;

    let step_id = StepId("main".to_string());
    validate_step_id(&step_id)?;

    Ok(Pipeline {
        machine_id,
        pipeline_version: op_version.clone(),
        steps: vec![PipelineStep {
            step_id,
            op_id,
            op_version,
            op_config,
        }],
    })
}

/// Inputs for single-op run execution with typed report extraction from final snapshot context.
#[derive(Clone, Debug)]
pub struct SingleOpReportRequest {
    pub op_id: String,
    pub op_version: String,
    pub op_config: serde_json::Value,
    pub report_context_key: String,
}

/// Typed error for single-op report execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingleOpReportError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for SingleOpReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SingleOpReportError {}

impl SingleOpReportError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

fn single_op_report_error_from_storage(err: StorageError) -> SingleOpReportError {
    let info = match err {
        StorageError::Concurrency(info)
        | StorageError::NotFound(info)
        | StorageError::Corruption(info)
        | StorageError::Other(info) => info,
    };
    SingleOpReportError::new(info.code.0, info.message)
}

fn single_op_report_error_from_run(err: RunError) -> SingleOpReportError {
    let info = match err {
        RunError::InvalidPlan(info) => info,
        RunError::Storage(err) => match err {
            StorageError::Concurrency(info)
            | StorageError::NotFound(info)
            | StorageError::Corruption(info)
            | StorageError::Other(info) => info,
        },
        RunError::Context(err) => match err {
            ContextError::MissingKey { info, .. }
            | ContextError::Serialization(info)
            | ContextError::Other(info) => info,
        },
        RunError::Io(err) => match err {
            IoError::MissingFactKey(info)
            | IoError::MissingFact { info, .. }
            | IoError::Transport(info)
            | IoError::RateLimited(info)
            | IoError::Other(info) => info,
        },
        RunError::State(err) => err.info,
        RunError::Other(info) => info,
    };

    SingleOpReportError::new(info.code.0, info.message)
}

fn single_op_report_error_from_sdk(err: SdkError) -> SingleOpReportError {
    SingleOpReportError::new(err.info.code.0, err.info.message)
}

fn run_phase_label(phase: &RunPhase) -> &'static str {
    match phase {
        RunPhase::Running => "running",
        RunPhase::Completed => "completed",
        RunPhase::Failed => "failed",
        RunPhase::Cancelled => "cancelled",
    }
}

#[derive(Default)]
struct MapContext {
    inner: HashMap<String, serde_json::Value>,
}

impl DynContext for MapContext {
    fn read(
        &self,
        key: &ContextKey,
    ) -> Result<Option<serde_json::Value>, mfm_machine::errors::ContextError> {
        Ok(self.inner.get(&key.0).cloned())
    }

    fn write(
        &mut self,
        key: ContextKey,
        value: serde_json::Value,
    ) -> Result<(), mfm_machine::errors::ContextError> {
        self.inner.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), mfm_machine::errors::ContextError> {
        self.inner.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, mfm_machine::errors::ContextError> {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.inner {
            m.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(m))
    }
}

fn default_initial_context() -> Box<dyn DynContext> {
    Box::new(MapContext::default())
}

fn default_build_provenance() -> BuildProvenance {
    BuildProvenance {
        git_commit: None,
        cargo_lock_hash: None,
        flake_lock_hash: None,
        rustc_version: None,
        target_triple: None,
        env_allowlist: Vec::new(),
    }
}

fn default_run_config() -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff: BackoffPolicy::Fixed {
                delay: std::time::Duration::from_millis(0),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
        nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
    }
}

async fn single_op_report_error_from_failed_run(
    stores: &Stores,
    run: &RunResult,
) -> SingleOpReportError {
    let events = stores.events.read_range(run.run_id, 1, None).await;
    let Ok(events) = events else {
        return SingleOpReportError::new(
            "RunFailed",
            format!(
                "run {} finished in phase {}",
                run.run_id.0,
                run_phase_label(&run.phase)
            ),
        );
    };

    for envelope in events.iter().rev() {
        let Event::Kernel(kernel) = &envelope.event else {
            continue;
        };

        if let KernelEvent::StateFailed { error, .. } = kernel {
            return SingleOpReportError::new(error.info.code.0.clone(), error.info.message.clone());
        }
    }

    SingleOpReportError::new(
        "RunFailed",
        format!(
            "run {} finished in phase {}",
            run.run_id.0,
            run_phase_label(&run.phase)
        ),
    )
}

/// Executes a single-op run and decodes a typed report from final snapshot context.
pub async fn execute_single_op_report<T: serde::de::DeserializeOwned>(
    engine: Arc<dyn ExecutionEngine>,
    stores: Stores,
    registry: Arc<dyn OperationRegistry>,
    planner: Arc<dyn PipelinePlanner>,
    req: SingleOpReportRequest,
) -> Result<T, SingleOpReportError> {
    let pipeline = single_op_pipeline(OpId(req.op_id), req.op_version, req.op_config)
        .map_err(single_op_report_error_from_sdk)?;

    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            engine,
            Stores {
                events: Arc::clone(&stores.events),
                artifacts: Arc::clone(&stores.artifacts),
            },
            registry,
            planner,
            LaunchPipeline {
                pipeline,
                input: serde_json::json!({}),
                run_config: default_run_config(),
                build: default_build_provenance(),
                initial_context: default_initial_context(),
            },
        )
        .await
        .map_err(single_op_report_error_from_run)?;

    if run.phase != RunPhase::Completed {
        return Err(single_op_report_error_from_failed_run(&stores, &run).await);
    }

    let final_snapshot_id = run.final_snapshot_id.ok_or_else(|| {
        SingleOpReportError::new(
            "MissingFinalSnapshot",
            "run completed without a final snapshot",
        )
    })?;

    let snapshot_bytes = stores
        .artifacts
        .get(&final_snapshot_id)
        .await
        .map_err(single_op_report_error_from_storage)?;

    let snapshot: serde_json::Value = serde_json::from_slice(&snapshot_bytes).map_err(|_| {
        SingleOpReportError::new("InvalidSnapshot", "final snapshot artifact was not JSON")
    })?;

    let report_value = snapshot
        .get(&req.report_context_key)
        .cloned()
        .ok_or_else(|| {
            SingleOpReportError::new(
                "MissingReport",
                "run completed without a report payload in snapshot context",
            )
        })?;

    serde_json::from_value(report_value).map_err(|_| {
        SingleOpReportError::new(
            "InvalidReport",
            "failed to decode report payload from final snapshot",
        )
    })
}

/// Helpers for spawning and awaiting engine-managed child runs (Milestone 5).
///
/// These helpers are intentionally `unstable`:
/// - the IO surface is stringly-typed (`IoCall.namespace`)
/// - request/response schemas may evolve
pub mod child_runs {
    use serde::{Deserialize, Serialize};

    use mfm_machine::config::RunConfig;
    use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError, RunError};
    use mfm_machine::events::{
        ChildRunCompleted, ChildRunSpawned, DomainEvent, RunStatus,
        DOMAIN_EVENT_CHILD_RUN_COMPLETED, DOMAIN_EVENT_CHILD_RUN_SPAWNED,
    };
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey, OpId, RunId};
    use mfm_machine::io::{IoCall, IoProvider};
    use mfm_machine::recorder::EventRecorder;

    const NAMESPACE_CHILD_RUN_SPAWN: &str = "machine.child_run.spawn";
    const NAMESPACE_CHILD_RUN_AWAIT: &str = "machine.child_run.await";

    fn io_other(code: &'static str, category: ErrorCategory, message: &'static str) -> IoError {
        IoError::Other(ErrorInfo {
            code: ErrorCode(code.to_string()),
            category,
            retryable: false,
            message: message.to_string(),
            details: None,
        })
    }

    #[derive(Clone, Debug)]
    pub struct SpawnChildRunV1 {
        pub op_id: OpId,
        pub op_version: String,
        pub op_config: serde_json::Value,
        pub input: serde_json::Value,
        pub run_config: RunConfig,
        pub initial_context: Option<serde_json::Value>,
    }

    #[derive(Clone, Debug)]
    pub struct SpawnChildRunResult {
        pub parent_run_id: RunId,
        pub child_run_id: RunId,
        pub child_manifest_id: ArtifactId,
    }

    #[derive(Clone, Debug, Serialize)]
    struct SpawnRequestV1 {
        kind: &'static str,
        op_id: String,
        op_version: String,
        op_config: serde_json::Value,
        input: serde_json::Value,
        run_config: RunConfig,
        #[serde(default)]
        initial_context: serde_json::Value,
    }

    #[derive(Clone, Debug, Deserialize)]
    struct SpawnResponseV1 {
        parent_run_id: RunId,
        child_run_id: RunId,
        child_manifest_id: ArtifactId,
    }

    pub async fn spawn_child_run_v1(
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
        fact_key: FactKey,
        req: SpawnChildRunV1,
    ) -> Result<SpawnChildRunResult, RunError> {
        // If the fact already exists (resume/replay), do not emit linkage again.
        let existed = io
            .get_recorded_fact(&fact_key)
            .await
            .map_err(RunError::Io)?
            .is_some();

        let request = serde_json::to_value(SpawnRequestV1 {
            kind: "child_run_spawn_v1",
            op_id: req.op_id.0,
            op_version: req.op_version,
            op_config: req.op_config,
            input: req.input,
            run_config: req.run_config,
            initial_context: req.initial_context.unwrap_or(serde_json::Value::Null),
        })
        .map_err(|_| {
            RunError::Io(io_other(
                "child_run_spawn_request_serialize_failed",
                ErrorCategory::ParsingInput,
                "failed to serialize child run spawn request",
            ))
        })?;

        let res = io
            .call(IoCall {
                namespace: NAMESPACE_CHILD_RUN_SPAWN.to_string(),
                request,
                fact_key: Some(fact_key.clone()),
            })
            .await
            .map_err(RunError::Io)?;

        let parsed = serde_json::from_value::<SpawnResponseV1>(res.response).map_err(|_| {
            RunError::Io(io_other(
                "child_run_spawn_response_invalid",
                ErrorCategory::ParsingInput,
                "invalid child run spawn response",
            ))
        })?;

        if !existed {
            let payload = serde_json::to_value(ChildRunSpawned {
                parent_run_id: parsed.parent_run_id,
                child_run_id: parsed.child_run_id,
                child_manifest_id: parsed.child_manifest_id.clone(),
            })
            .map_err(|_| {
                RunError::Io(io_other(
                    "child_run_spawned_payload_serialize_failed",
                    ErrorCategory::Unknown,
                    "failed to serialize ChildRunSpawned payload",
                ))
            })?;

            rec.emit(DomainEvent {
                name: DOMAIN_EVENT_CHILD_RUN_SPAWNED.to_string(),
                payload,
                payload_ref: None,
            })
            .await?;
        }

        Ok(SpawnChildRunResult {
            parent_run_id: parsed.parent_run_id,
            child_run_id: parsed.child_run_id,
            child_manifest_id: parsed.child_manifest_id,
        })
    }

    #[derive(Clone, Debug)]
    pub struct AwaitChildRunV1 {
        pub child_run_id: RunId,
        pub child_manifest_id: ArtifactId,
    }

    #[derive(Clone, Debug)]
    pub struct AwaitChildRunResult {
        pub child_run_id: RunId,
        pub status: RunStatus,
        pub final_snapshot_id: Option<ArtifactId>,
        pub final_snapshot: serde_json::Value,
    }

    #[derive(Clone, Debug, Serialize)]
    struct AwaitRequestV1 {
        kind: &'static str,
        child_run_id: RunId,
        child_manifest_id: ArtifactId,
    }

    #[derive(Clone, Debug, Deserialize)]
    struct AwaitResponseV1 {
        child_run_id: RunId,
        status: RunStatus,
        final_snapshot_id: Option<ArtifactId>,
        #[serde(default)]
        final_snapshot: serde_json::Value,
    }

    pub async fn await_child_run_v1(
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
        fact_key: FactKey,
        req: AwaitChildRunV1,
    ) -> Result<AwaitChildRunResult, RunError> {
        // If the fact already exists (resume/replay), do not emit linkage again.
        let existed = io
            .get_recorded_fact(&fact_key)
            .await
            .map_err(RunError::Io)?
            .is_some();

        let request = serde_json::to_value(AwaitRequestV1 {
            kind: "child_run_await_v1",
            child_run_id: req.child_run_id,
            child_manifest_id: req.child_manifest_id.clone(),
        })
        .map_err(|_| {
            RunError::Io(io_other(
                "child_run_await_request_serialize_failed",
                ErrorCategory::ParsingInput,
                "failed to serialize child run await request",
            ))
        })?;

        let res = io
            .call(IoCall {
                namespace: NAMESPACE_CHILD_RUN_AWAIT.to_string(),
                request,
                fact_key: Some(fact_key.clone()),
            })
            .await
            .map_err(RunError::Io)?;

        let parsed = serde_json::from_value::<AwaitResponseV1>(res.response).map_err(|_| {
            RunError::Io(io_other(
                "child_run_await_response_invalid",
                ErrorCategory::ParsingInput,
                "invalid child run await response",
            ))
        })?;

        if !existed {
            let payload = serde_json::to_value(ChildRunCompleted {
                child_run_id: parsed.child_run_id,
                status: parsed.status.clone(),
                final_snapshot_id: parsed.final_snapshot_id.clone(),
            })
            .map_err(|_| {
                RunError::Io(io_other(
                    "child_run_completed_payload_serialize_failed",
                    ErrorCategory::Unknown,
                    "failed to serialize ChildRunCompleted payload",
                ))
            })?;

            rec.emit(DomainEvent {
                name: DOMAIN_EVENT_CHILD_RUN_COMPLETED.to_string(),
                payload,
                payload_ref: None,
            })
            .await?;
        }

        Ok(AwaitChildRunResult {
            child_run_id: parsed.child_run_id,
            status: parsed.status,
            final_snapshot_id: parsed.final_snapshot_id,
            final_snapshot: parsed.final_snapshot,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_machine::config::{
        BackoffPolicy, ContextCheckpointing, EventProfile, ExecutionMode, IoMode, RetryPolicy,
        RunConfig,
    };
    use mfm_machine::errors::{ContextError, StateError};
    use mfm_machine::events::EventEnvelope;
    use mfm_machine::ids::ArtifactId;
    use mfm_machine::runtime::{DefaultExecutionEngine, PlanResolver};
    use mfm_machine::stores::{ArtifactStore, EventStore};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn run_config_live() -> RunConfig {
        RunConfig {
            io_mode: IoMode::Live,
            retry_policy: RetryPolicy {
                max_attempts: 1,
                backoff: BackoffPolicy::Fixed {
                    delay: std::time::Duration::from_millis(0),
                },
            },
            event_profile: EventProfile::Normal,
            execution_mode: ExecutionMode::Sequential,
            context_checkpointing: ContextCheckpointing::AfterEveryState,
            replay_missing_fact_retryable: false,
            skip_tags: Vec::new(),
            nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
        }
    }

    #[derive(Default)]
    struct MapContext {
        inner: HashMap<String, serde_json::Value>,
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
        inner: Arc<Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
    }

    #[async_trait]
    impl EventStore for MemEventStore {
        async fn head_seq(&self, run_id: RunId) -> Result<u64, mfm_machine::errors::StorageError> {
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
        ) -> Result<u64, mfm_machine::errors::StorageError> {
            let mut inner = self.inner.lock().await;
            let stream = inner.entry(run_id).or_default();
            let head = stream.last().map(|e| e.seq).unwrap_or(0);
            if head != expected_seq {
                return Err(mfm_machine::errors::StorageError::Concurrency(info(
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
        ) -> Result<Vec<EventEnvelope>, mfm_machine::errors::StorageError> {
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
        ) -> Result<ArtifactId, mfm_machine::errors::StorageError> {
            let id = artifact_id_for_bytes(&bytes);
            self.inner.lock().await.insert(id.clone(), bytes);
            Ok(id)
        }

        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, mfm_machine::errors::StorageError> {
            let inner = self.inner.lock().await;
            inner.get(id).cloned().ok_or_else(|| {
                mfm_machine::errors::StorageError::NotFound(info(
                    "artifact_not_found",
                    ErrorCategory::Storage,
                    false,
                    "artifact not found",
                ))
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, mfm_machine::errors::StorageError> {
            Ok(self.inner.lock().await.contains_key(id))
        }
    }

    #[derive(Clone)]
    struct WriteKeyState {
        key: &'static str,
        value: serde_json::Value,
    }

    #[async_trait]
    impl State for WriteKeyState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
                side_effects: mfm_machine::meta::SideEffectKind::Pure,
                idempotency: mfm_machine::meta::Idempotency::None,
            }
        }

        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            ctx.write(ContextKey(self.key.to_string()), self.value.clone())
                .map_err(|_| StateError {
                    state_id: None,
                    info: info(
                        "ctx_write_failed",
                        ErrorCategory::Context,
                        false,
                        "context write failed",
                    ),
                })?;
            Ok(StateOutcome {
                snapshot: mfm_machine::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    #[derive(Clone)]
    struct ReadThenWriteState {
        read_key: &'static str,
        write_key: &'static str,
    }

    #[async_trait]
    impl State for ReadThenWriteState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
                side_effects: mfm_machine::meta::SideEffectKind::Pure,
                idempotency: mfm_machine::meta::Idempotency::None,
            }
        }

        async fn handle(
            &self,
            ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            let v = ctx
                .read(&ContextKey(self.read_key.to_string()))
                .map_err(|_| StateError {
                    state_id: None,
                    info: info(
                        "ctx_read_failed",
                        ErrorCategory::Context,
                        false,
                        "context read failed",
                    ),
                })?
                .unwrap_or(serde_json::Value::Null);

            ctx.write(ContextKey(self.write_key.to_string()), v)
                .map_err(|_| StateError {
                    state_id: None,
                    info: info(
                        "ctx_write_failed",
                        ErrorCategory::Context,
                        false,
                        "context write failed",
                    ),
                })?;

            Ok(StateOutcome {
                snapshot: mfm_machine::state::SnapshotPolicy::OnSuccess,
            })
        }
    }

    struct TestOp {
        op_id: OpId,
        op_version: String,
        io: OpIo,
        graph: StateGraph,
    }

    impl TestOp {
        fn new_write(
            op_id: &str,
            op_version: &str,
            state_id: &str,
            key: &'static str,
            value: serde_json::Value,
            io: OpIo,
        ) -> Self {
            let state: DynState = Arc::new(WriteKeyState { key, value });
            Self {
                op_id: OpId(op_id.to_string()),
                op_version: op_version.to_string(),
                io,
                graph: StateGraph {
                    states: vec![StateNode {
                        id: StateId(state_id.to_string()),
                        state,
                    }],
                    edges: Vec::new(),
                },
            }
        }

        fn new_read_then_write(
            op_id: &str,
            op_version: &str,
            state_id: &str,
            read_key: &'static str,
            write_key: &'static str,
            io: OpIo,
        ) -> Self {
            let state: DynState = Arc::new(ReadThenWriteState {
                read_key,
                write_key,
            });
            Self {
                op_id: OpId(op_id.to_string()),
                op_version: op_version.to_string(),
                io,
                graph: StateGraph {
                    states: vec![StateNode {
                        id: StateId(state_id.to_string()),
                        state,
                    }],
                    edges: Vec::new(),
                },
            }
        }
    }

    impl crate::op::Operation for TestOp {
        fn op_id(&self) -> OpId {
            self.op_id.clone()
        }

        fn op_version(&self) -> String {
            self.op_version.clone()
        }

        fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
            Ok(self.io.clone())
        }

        fn expand(
            &self,
            _op_path: OpPath,
            _op_config: &serde_json::Value,
            _run_config: &RunConfig,
        ) -> Result<StateGraph, SdkError> {
            Ok(self.graph.clone())
        }
    }

    #[test]
    fn planner_adds_step_barrier_edges() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp::new_write(
            "op1",
            "v1",
            "m.step1.s1",
            "k",
            serde_json::json!("v1"),
            OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
        )));
        reg.register(Arc::new(TestOp::new_write(
            "op2",
            "v1",
            "m.step2.s1",
            "k",
            serde_json::json!("v2"),
            OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
        )));

        let pipeline = Pipeline {
            machine_id: MachineId("m".to_string()),
            pipeline_version: "v".to_string(),
            steps: vec![
                PipelineStep {
                    step_id: StepId("step1".to_string()),
                    op_id: OpId("op1".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({}),
                },
                PipelineStep {
                    step_id: StepId("step2".to_string()),
                    op_id: OpId("op2".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({}),
                },
            ],
        };

        let plan = DefaultPipelinePlanner
            .build_execution_plan(Arc::new(reg), &pipeline, &run_config_live())
            .expect("plan should build");

        assert!(
            plan.graph
                .edges
                .iter()
                .any(|e| e.from.0 == "m.step1.s1" && e.to.0 == "m.step2.s1"),
            "expected barrier edge from step1 to step2"
        );
    }

    #[test]
    fn planner_rejects_apply_side_effect_without_idempotency_key() {
        #[derive(Clone)]
        struct ApplyNoIdemState;

        #[async_trait]
        impl State for ApplyNoIdemState {
            fn meta(&self) -> StateMeta {
                StateMeta {
                    tags: Vec::new(),
                    depends_on: Vec::new(),
                    depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
                    side_effects: mfm_machine::meta::SideEffectKind::ApplySideEffect,
                    idempotency: mfm_machine::meta::Idempotency::None,
                }
            }

            async fn handle(
                &self,
                _ctx: &mut dyn DynContext,
                _io: &mut dyn IoProvider,
                _rec: &mut dyn EventRecorder,
            ) -> Result<StateOutcome, StateError> {
                Ok(StateOutcome {
                    snapshot: mfm_machine::state::SnapshotPolicy::OnSuccess,
                })
            }
        }

        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp {
            op_id: OpId("op".to_string()),
            op_version: "v1".to_string(),
            io: OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
            graph: StateGraph {
                states: vec![StateNode {
                    id: StateId("op.main.s1".to_string()),
                    state: Arc::new(ApplyNoIdemState),
                }],
                edges: Vec::new(),
            },
        }));

        let pipeline = single_op_pipeline(
            OpId("op".to_string()),
            "v1".to_string(),
            serde_json::json!({}),
        )
        .expect("pipeline");

        let err = match DefaultPipelinePlanner.build_execution_plan(
            Arc::new(reg),
            &pipeline,
            &run_config_live(),
        ) {
            Ok(_) => panic!("expected planner error"),
            Err(e) => e,
        };
        assert_eq!(err.info.code.0, "missing_idempotency_key");
    }

    #[test]
    fn planner_validates_unsatisfied_import() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp::new_read_then_write(
            "op1",
            "v1",
            "m.step1.s1",
            "x",
            "y",
            OpIo {
                imports: vec![PortKey("x".to_string())],
                exports: Vec::new(),
            },
        )));

        let pipeline = Pipeline {
            machine_id: MachineId("m".to_string()),
            pipeline_version: "v".to_string(),
            steps: vec![PipelineStep {
                step_id: StepId("step1".to_string()),
                op_id: OpId("op1".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            }],
        };

        let err = match DefaultPipelinePlanner.build_execution_plan(
            Arc::new(reg),
            &pipeline,
            &run_config_live(),
        ) {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert_eq!(err.info.code.0, "unsatisfied_import");
    }

    #[test]
    fn planner_rejects_secrets_in_op_config() {
        let reg = HashMapOperationRegistry::default();
        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);

        let pipeline = Pipeline {
            machine_id: MachineId("m".to_string()),
            pipeline_version: "v".to_string(),
            steps: vec![PipelineStep {
                step_id: StepId("step1".to_string()),
                op_id: OpId("op1".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({ "password": "x" }),
            }],
        };

        let err = match planner.build_execution_plan(Arc::new(reg), &pipeline, &run_config_live()) {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert_eq!(err.info.code.0, "secrets_detected");
    }

    #[tokio::test]
    async fn launcher_rejects_secrets_in_manifest_input() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp::new_write(
            "op1",
            "v1",
            "m.step1.s1",
            "k",
            serde_json::json!("v1"),
            OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
        )));

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);

        struct NeverResolver;
        impl mfm_machine::runtime::PlanResolver for NeverResolver {
            fn resolve(&self, _manifest: &RunManifest) -> Result<ExecutionPlan, RunError> {
                Err(RunError::InvalidPlan(info(
                    "resolver_unavailable",
                    ErrorCategory::Unknown,
                    false,
                    "resolver unavailable",
                )))
            }
        }

        let engine: Arc<dyn ExecutionEngine> =
            Arc::new(DefaultExecutionEngine::new(Arc::new(NeverResolver)));
        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let pipeline = Pipeline {
            machine_id: MachineId("m".to_string()),
            pipeline_version: "v".to_string(),
            steps: vec![PipelineStep {
                step_id: StepId("step1".to_string()),
                op_id: OpId("op1".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            }],
        };

        let err = launcher
            .start_pipeline(
                engine,
                stores,
                Arc::new(reg),
                planner,
                LaunchPipeline {
                    pipeline,
                    input: serde_json::json!({ "authorization": "Bearer x" }),
                    run_config: run_config_live(),
                    build: mfm_machine::config::BuildProvenance {
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
            .unwrap_err();

        match err {
            RunError::InvalidPlan(info) => assert_eq!(info.code.0, "secrets_detected"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn namespacing_prevents_context_collisions_and_wires_imports() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp::new_write(
            "op1",
            "v1",
            "m.step1.s1",
            "x",
            serde_json::json!("v1"),
            OpIo {
                imports: Vec::new(),
                exports: vec![PortKey("x".to_string())],
            },
        )));
        reg.register(Arc::new(TestOp::new_read_then_write(
            "op2",
            "v1",
            "m.step2.s1",
            "x",
            "y",
            OpIo {
                imports: vec![PortKey("x".to_string())],
                exports: Vec::new(),
            },
        )));
        reg.register(Arc::new(TestOp::new_write(
            "op3",
            "v1",
            "m.step3.s1",
            "x",
            serde_json::json!("v2"),
            OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
        )));

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);

        // A resolver is required by the engine type but isn't used for start().
        struct NeverResolver;
        impl mfm_machine::runtime::PlanResolver for NeverResolver {
            fn resolve(&self, _manifest: &RunManifest) -> Result<ExecutionPlan, RunError> {
                Err(RunError::InvalidPlan(info(
                    "resolver_unavailable",
                    ErrorCategory::Unknown,
                    false,
                    "resolver unavailable",
                )))
            }
        }

        let engine: Arc<dyn ExecutionEngine> =
            Arc::new(DefaultExecutionEngine::new(Arc::new(NeverResolver)));
        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let pipeline = Pipeline {
            machine_id: MachineId("m".to_string()),
            pipeline_version: "v".to_string(),
            steps: vec![
                PipelineStep {
                    step_id: StepId("step1".to_string()),
                    op_id: OpId("op1".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({}),
                },
                PipelineStep {
                    step_id: StepId("step2".to_string()),
                    op_id: OpId("op2".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({}),
                },
                PipelineStep {
                    step_id: StepId("step3".to_string()),
                    op_id: OpId("op3".to_string()),
                    op_version: "v1".to_string(),
                    op_config: serde_json::json!({}),
                },
            ],
        };

        let result = launcher
            .start_pipeline(
                engine,
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::new(reg),
                planner,
                LaunchPipeline {
                    pipeline,
                    input: serde_json::json!({}),
                    run_config: run_config_live(),
                    build: mfm_machine::config::BuildProvenance {
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
            .expect("run should succeed");

        let snap_id = result.final_snapshot_id.expect("snapshot id");
        let bytes = stores.artifacts.get(&snap_id).await.expect("get snapshot");
        let v = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
        let obj = v.as_object().expect("object");

        // step1 writes x, but it must be namespaced.
        assert_eq!(obj.get("m.step1.x"), Some(&serde_json::json!("v1")));

        // step2 reads imported x (from step1) and writes y in its own namespace.
        assert_eq!(obj.get("m.step2.y"), Some(&serde_json::json!("v1")));

        // step3 writes x too; it must not collide with step1.
        assert_eq!(obj.get("m.step3.x"), Some(&serde_json::json!("v2")));
    }

    #[tokio::test]
    async fn sdk_plan_resolver_rebuilds_plan_from_manifest() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp::new_write(
            "op1",
            "v1",
            "m.step1.s1",
            "k",
            serde_json::json!("v1"),
            OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
        )));

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let resolver = SdkPlanResolver::new(Arc::new(reg), Arc::clone(&planner));

        let pipeline = Pipeline {
            machine_id: MachineId("m".to_string()),
            pipeline_version: "v".to_string(),
            steps: vec![PipelineStep {
                step_id: StepId("step1".to_string()),
                op_id: OpId("op1".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            }],
        };

        let manifest = RunManifest {
            op_id: OpId("m".to_string()),
            op_version: "v".to_string(),
            input_params: serde_json::to_value(PipelineManifestInput {
                pipeline: pipeline.clone(),
                input: serde_json::json!({}),
            })
            .unwrap(),
            run_config: run_config_live(),
            build: mfm_machine::config::BuildProvenance {
                git_commit: None,
                cargo_lock_hash: None,
                flake_lock_hash: None,
                rustc_version: None,
                target_triple: None,
                env_allowlist: Vec::new(),
            },
        };

        let plan = resolver.resolve(&manifest).expect("resolve plan");
        assert_eq!(plan.op_id.0, "m");
        assert_eq!(plan.graph.states.len(), 1);
        assert_eq!(plan.graph.states[0].id.0, "m.step1.s1");
    }

    #[tokio::test]
    async fn single_op_report_returns_typed_report() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp::new_write(
            "report_op",
            "v1",
            "report_op.main.s1",
            "report",
            serde_json::json!({"value": 42}),
            OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
        )));

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let registry: Arc<dyn OperationRegistry> = Arc::new(reg);
        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(DefaultExecutionEngine::new(resolver));
        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let report: serde_json::Value = execute_single_op_report(
            engine,
            stores,
            registry,
            planner,
            SingleOpReportRequest {
                op_id: "report_op".to_string(),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
                report_context_key: "report_op.main.report".to_string(),
            },
        )
        .await
        .expect("single-op report should decode");

        assert_eq!(report, serde_json::json!({"value": 42}));
    }

    #[tokio::test]
    async fn single_op_report_errors_when_report_key_missing() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(TestOp::new_write(
            "report_missing_op",
            "v1",
            "report_missing_op.main.s1",
            "unused",
            serde_json::json!({"value": 1}),
            OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            },
        )));

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let registry: Arc<dyn OperationRegistry> = Arc::new(reg);
        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(DefaultExecutionEngine::new(resolver));
        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let err = execute_single_op_report::<serde_json::Value>(
            engine,
            stores,
            registry,
            planner,
            SingleOpReportRequest {
                op_id: "report_missing_op".to_string(),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
                report_context_key: "report_missing_op.main.report".to_string(),
            },
        )
        .await
        .expect_err("missing report key should fail");

        assert_eq!(err.code, "MissingReport");
    }

    #[derive(Clone)]
    struct FailingState {
        code: &'static str,
        message: &'static str,
    }

    #[async_trait]
    impl State for FailingState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: mfm_machine::meta::DependencyStrategy::Latest,
                side_effects: mfm_machine::meta::SideEffectKind::Pure,
                idempotency: mfm_machine::meta::Idempotency::None,
            }
        }

        async fn handle(
            &self,
            _ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            Err(StateError {
                state_id: Some(StateId("fail_op.main.s1".to_string())),
                info: ErrorInfo {
                    code: ErrorCode(self.code.to_string()),
                    category: ErrorCategory::Unknown,
                    retryable: false,
                    message: self.message.to_string(),
                    details: None,
                },
            })
        }
    }

    struct FailOp;

    impl crate::op::Operation for FailOp {
        fn op_id(&self) -> OpId {
            OpId("fail_op".to_string())
        }

        fn op_version(&self) -> String {
            "v1".to_string()
        }

        fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
            Ok(OpIo {
                imports: Vec::new(),
                exports: Vec::new(),
            })
        }

        fn expand(
            &self,
            _op_path: OpPath,
            _op_config: &serde_json::Value,
            _run_config: &RunConfig,
        ) -> Result<StateGraph, SdkError> {
            Ok(StateGraph {
                states: vec![StateNode {
                    id: StateId("fail_op.main.s1".to_string()),
                    state: Arc::new(FailingState {
                        code: "IntentionalFailure",
                        message: "intentional test failure",
                    }),
                }],
                edges: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn single_op_report_maps_failed_run_state_error() {
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::new(FailOp));

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let registry: Arc<dyn OperationRegistry> = Arc::new(reg);
        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(DefaultExecutionEngine::new(resolver));
        let stores = Stores {
            events: Arc::new(MemEventStore::default()),
            artifacts: Arc::new(MemArtifactStore::default()),
        };

        let err = execute_single_op_report::<serde_json::Value>(
            engine,
            stores,
            registry,
            planner,
            SingleOpReportRequest {
                op_id: "fail_op".to_string(),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
                report_context_key: "fail_op.main.report".to_string(),
            },
        )
        .await
        .expect_err("failed run should surface state failure");

        assert_eq!(err.code, "IntentionalFailure");
        assert_eq!(err.message, "intentional test failure");
    }
}
