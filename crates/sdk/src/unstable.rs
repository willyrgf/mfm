//! Unstable helper implementations for planning and launching.
//!
//! This module provides batteries-included implementations for the public SDK traits:
//! - [`crate::op::OperationRegistry`] via [`HashMapOperationRegistry`]
//! - [`crate::pipeline::PipelinePlanner`] via [`DefaultPipelinePlanner`]
//! - [`crate::launcher::RunLauncher`] via [`DefaultRunLauncher`]
//!
//! It also exposes convenience helpers such as [`single_op_pipeline`] and runtime-facing child-run
//! IO helpers under [`child_runs`].
//!
//! Source of truth: `docs/redesign.md` (v4).
//! Not part of the stable API contract (Appendix C.2).
//!
//! Typical usage:
//! 1. register operations in [`HashMapOperationRegistry`]
//! 2. flatten a [`crate::pipeline::Pipeline`] via [`DefaultPipelinePlanner`]
//! 3. start or resume runs via [`DefaultRunLauncher`]
//! 4. rebuild plans during resume via [`SdkPlanResolver`]
//! 5. use [`single_op_pipeline`] or [`execute_single_op_report`] from transport layers that expose
//!    one-op convenience APIs
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::ids::OpId;
//! use mfm_sdk::unstable::single_op_pipeline;
//!
//! let pipeline = single_op_pipeline(
//!     OpId::must_new("keystore_list"),
//!     "v1".to_string(),
//!     serde_json::json!({"sort_by": "name"}),
//! )?;
//!
//! assert_eq!(pipeline.machine_id.0, "keystore_list");
//! assert_eq!(pipeline.steps.len(), 1);
//! assert_eq!(pipeline.steps[0].step_id.0, "main");
//! # Ok::<(), mfm_sdk::errors::SdkError>(())
//! ```

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
use mfm_machine::events::{event_envelopes_from_stream_records, Event, KernelEvent};
use mfm_machine::hashing::{
    artifact_id_for_bytes, artifact_id_for_json, canonical_json_bytes, CanonicalJsonError,
};
use mfm_machine::ids::{ContextKey, ErrorCode, OpId, OpPath, RunId, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::{Idempotency, SideEffectKind, StateMeta};
use mfm_machine::plan::{DependencyEdge, ExecutionPlan, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{DynState, State, StateOutcome};
use mfm_machine::stores::{ArtifactKind, StreamId};

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

        // Enforce policy: op_config must be canonical-JSON hashable (floats forbidden).
        canonical_json_bytes(&s.op_config).map_err(|e| match e {
            CanonicalJsonError::FloatNotAllowed => sdk_error(
                "op_config_not_canonical",
                ErrorCategory::ParsingInput,
                "op_config is not canonical-json-hashable (floats are forbidden)",
            ),
            CanonicalJsonError::SecretsNotAllowed => sdk_error(
                "secrets_detected",
                ErrorCategory::ParsingInput,
                "op_config contained secrets (policy forbids persisting secrets)",
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
    let mut it = state_id.as_str().split('.');
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
                            n.id.as_str()
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

/// A simple [`crate::op::OperationRegistry`] implementation backed by a hash map.
///
/// Use this when callers want explicit runtime registration of a known set of operations without
/// adding their own registry abstraction. Re-registering the same `(op_id, op_version)` pair
/// replaces the previous implementation.
#[derive(Default)]
pub struct HashMapOperationRegistry {
    ops: HashMap<(OpId, String), DynOperation>,
}

impl HashMapOperationRegistry {
    /// Registers or replaces an operation implementation by its `(op_id, op_version)` key.
    ///
    /// This is typically performed once during application startup while assembling the operation
    /// plugin or test harness registry.
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

/// Default planner that validates a [`Pipeline`] and flattens it into an [`ExecutionPlan`].
///
/// The planner is responsible for:
/// - validating stable machine/step/state identifier shapes
/// - enforcing import/export wiring between adjacent pipeline steps
/// - wrapping step-local states in a namespaced context view
/// - preserving deterministic step ordering by linking sink states to the next step's sources
///
/// Use this planner for the repository's default flattened-composition contract: each pipeline
/// step expands independently, exports feed later imports, and cross-step ordering is enforced by
/// dependency edges between sink and source states.
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
            op_id: OpId::must_new(pipeline.machine_id.0.clone()),
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

/// Default launcher that persists the manifest and delegates start/resume to an execution engine.
///
/// Use this when callers want the standard MFM manifest layout and artifact-id derivation rules
/// without reimplementing engine orchestration. Pair it with [`SdkPlanResolver`] so resumed runs
/// rebuild the same execution plan from the stored manifest input.
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
            op_id: OpId::must_new(req.pipeline.machine_id.0.clone()),
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
                "run manifest contained secrets (policy forbids persisting secrets)",
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
                "run manifest contained secrets (policy forbids persisting secrets)",
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
/// This is used by `DefaultExecutionEngine` during `resume()` to recover the pipeline definition
/// from `RunManifest.input_params` and rebuild the deterministic execution plan on demand.
///
/// # Examples
///
/// ```rust
/// use std::sync::Arc;
///
/// use mfm_sdk::unstable::{DefaultPipelinePlanner, HashMapOperationRegistry, SdkPlanResolver};
///
/// let registry = Arc::new(HashMapOperationRegistry::default());
/// let planner = Arc::new(DefaultPipelinePlanner);
/// let _resolver = SdkPlanResolver::new(registry, planner);
/// ```
pub struct SdkPlanResolver {
    registry: Arc<dyn OperationRegistry>,
    planner: Arc<dyn PipelinePlanner>,
}

impl SdkPlanResolver {
    /// Builds a resolver from the shared operation registry and pipeline planner.
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
        if OpId::must_new(pipeline.machine_id.0.clone()) != manifest.op_id {
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

/// Builds the standard single-step pipeline wrapper for one operation.
///
/// This is the recommended bridge for transport layers that expose "run one op" ergonomics while
/// still executing through the pipeline-based SDK contract.
pub fn single_op_pipeline(
    op_id: OpId,
    op_version: String,
    op_config: serde_json::Value,
) -> Result<Pipeline, SdkError> {
    let machine_id = MachineId(op_id.as_str().to_string());
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
///
/// This is a transport-facing convenience wrapper for APIs that conceptually execute "one op" but
/// still rely on the pipeline-based launcher and manifest contract under the hood.
///
/// # Examples
///
/// ```rust
/// use mfm_sdk::unstable::SingleOpReportRequest;
///
/// let request = SingleOpReportRequest {
///     op_id: "keystore_list".to_string(),
///     op_version: "v1".to_string(),
///     op_config: serde_json::json!({"sort_by": "name"}),
///     report_context_key: "report".to_string(),
/// };
///
/// assert_eq!(request.op_id, "keystore_list");
/// assert_eq!(request.report_context_key, "report");
/// ```
#[derive(Clone, Debug)]
pub struct SingleOpReportRequest {
    /// Operation identifier to wrap into the single-step pipeline convention.
    pub op_id: String,
    /// Operation version to execute.
    pub op_version: String,
    /// Canonical JSON operation config passed to `Operation::expand`.
    pub op_config: serde_json::Value,
    /// Context key that should contain the final typed report.
    pub report_context_key: String,
}

/// Typed error for single-op report execution.
///
/// The error is intentionally reduced to a stable code and safe display message so transport
/// layers can forward it directly without leaking engine internals or secret-bearing details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingleOpReportError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable message safe to display to callers.
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
    let events = stores
        .streams
        .read_range(&StreamId::run(run.run_id), 1, None)
        .await
        .and_then(|records| event_envelopes_from_stream_records(run.run_id, records));
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
///
/// This helper is best suited for CLI or HTTP adapters that want a typed "run op and return the
/// final report" abstraction without reimplementing pipeline wrapping, launcher setup, manifest
/// persistence, or snapshot decoding.
pub async fn execute_single_op_report<T: serde::de::DeserializeOwned>(
    engine: Arc<dyn ExecutionEngine>,
    stores: Stores,
    registry: Arc<dyn OperationRegistry>,
    planner: Arc<dyn PipelinePlanner>,
    req: SingleOpReportRequest,
) -> Result<T, SingleOpReportError> {
    let op_id = OpId::new(req.op_id).map_err(|_| {
        SingleOpReportError::new("invalid_op_id", "op_id must match ^[a-z][a-z0-9_]{0,62}$")
    })?;
    let pipeline = single_op_pipeline(op_id, req.op_version, req.op_config)
        .map_err(single_op_report_error_from_sdk)?;

    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            engine,
            Stores {
                streams: Arc::clone(&stores.streams),
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

/// Helpers for spawning and awaiting engine-managed child runs through the runtime IO surface.
///
/// These helpers are intentionally `unstable`:
/// - the IO surface is stringly-typed (`IoCall.namespace`)
/// - request/response schemas may evolve
///
/// Typical flow:
/// 1. call [`spawn_child_run_v1`] with a fact key dedicated to the child-run spawn request
/// 2. persist the returned identifiers or emit them in higher-level state output
/// 3. later call [`await_child_run_v1`] with a second fact key to wait for completion
/// 4. decode the returned snapshot or status into parent-state domain output
///
/// The helpers are replay-safe: if the fact key already exists, they avoid emitting duplicate
/// linkage events on resume.
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

    /// Request payload for the `child_run_spawn_v1` helper.
    ///
    /// This is the typed front-end to the `"machine.child_run.spawn"` IO namespace.
    #[derive(Clone, Debug)]
    pub struct SpawnChildRunV1 {
        /// Operation identifier for the child run.
        pub op_id: OpId,
        /// Operation version for the child run.
        pub op_version: String,
        /// Canonical JSON config passed to the child operation.
        pub op_config: serde_json::Value,
        /// Canonical JSON input payload embedded in the child manifest.
        pub input: serde_json::Value,
        /// Effective run configuration for the child run.
        pub run_config: RunConfig,
        /// Optional initial context snapshot for the child run.
        pub initial_context: Option<serde_json::Value>,
    }

    /// Result returned after successfully spawning a child run.
    ///
    /// These identifiers are typically persisted in parent-state context or artifacts so later
    /// states can await or report on the child run deterministically.
    #[derive(Clone, Debug)]
    pub struct SpawnChildRunResult {
        /// Parent run that issued the spawn request.
        pub parent_run_id: RunId,
        /// Newly created child run identifier.
        pub child_run_id: RunId,
        /// Child run manifest artifact identifier.
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

    /// Spawns a child run via the configured IO transport and emits the linkage event once.
    ///
    /// If the supplied fact key already exists, the transport call reuses recorded IO and the
    /// helper suppresses duplicate `ChildRunSpawned` emission during replay or resume.
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
            op_id: req.op_id.to_string(),
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

    /// Request payload for the `child_run_await_v1` helper.
    ///
    /// This is the typed front-end to the `"machine.child_run.await"` IO namespace.
    #[derive(Clone, Debug)]
    pub struct AwaitChildRunV1 {
        /// Child run identifier to wait for.
        pub child_run_id: RunId,
        /// Manifest artifact recorded when the child run was spawned.
        pub child_manifest_id: ArtifactId,
    }

    /// Result returned after waiting for a child run to finish.
    ///
    /// The response includes both the reported run status and the decoded final snapshot payload
    /// returned by the child-run transport.
    #[derive(Clone, Debug)]
    pub struct AwaitChildRunResult {
        /// Child run that finished.
        pub child_run_id: RunId,
        /// Final run status reported by the child.
        pub status: RunStatus,
        /// Final snapshot identifier, if any.
        pub final_snapshot_id: Option<ArtifactId>,
        /// Decoded final snapshot payload returned by the transport.
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

    /// Waits for a previously spawned child run and emits the completion event once.
    ///
    /// If the supplied fact key already exists, the transport call reuses recorded IO and the
    /// helper suppresses duplicate `ChildRunCompleted` emission during replay or resume.
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
#[path = "tests/unstable_tests.rs"]
mod unstable_tests;
