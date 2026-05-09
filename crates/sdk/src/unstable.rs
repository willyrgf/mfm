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
//! Source of truth: `docs/design.md` (v4).
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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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
use mfm_machine::hashing::{canonical_json_bytes, put_artifact_verified, CanonicalJsonError};
use mfm_machine::ids::{
    ArtifactId, ContextKey, ContextSlot, ErrorCode, OpId, OpPath, RunId, StateId,
};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::{Idempotency, SideEffectKind, StateMeta};
use mfm_machine::plan::{DependencyEdge, ExecutionPlan, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{DynState, State, StateOutcome};
use mfm_machine::stores::{ArtifactKind, ArtifactStore, StreamId};

use crate::errors::SdkError;
use crate::ids::{MachineId, PortKey, StepId};
use crate::launcher::{LaunchPipeline, RunLauncher};
use crate::op::{
    child_op_path, leaf_state_id, DynOperation, LeafOpSpec, LeafStateNode, OpInterface,
    OperationRegistry, PlannedOp, PlannedOpKind, PlannerPayloadConfigSource, PortSource,
};
use crate::pipeline::{
    CompiledAfterEdge, CompiledExecutionSpec, CompiledImportBinding, CompiledOpRecord,
    CompiledReExport, CompiledRootExport, CompiledStateLineage, Pipeline, PipelineManifestInput,
    PipelinePlanner, PipelineStep, PlannedPipelineExecution, QualifiedSlotRef,
    COMPILED_EXECUTION_SPEC_SCHEMA_V1,
};

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
        id.0.as_str(),
        "machine_id must match ^[a-z][a-z0-9_]{0,62}$",
    )
}

fn validate_step_id(id: &StepId) -> Result<(), SdkError> {
    validate_segment(
        "invalid_step_id",
        id.0.as_str(),
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
    OpPath::must_new(format!("{}.{}", machine_id.0, step_id.0))
}

fn op_path_is_nested_within(step_root: &OpPath, candidate: &OpPath) -> bool {
    candidate.as_str() == step_root.as_str()
        || candidate
            .as_str()
            .strip_prefix(&format!("{}.", step_root.as_str()))
            .is_some()
}

impl QualifiedSlotRef {
    fn context_key(&self) -> ContextKey {
        ContextKey(format!("{}.{}", self.op_path, self.slot))
    }
}

struct FlattenedFragment {
    states: Vec<StateNode>,
    edges: Vec<DependencyEdge>,
    sources: Vec<StateId>,
    sinks: Vec<StateId>,
    exports: BTreeMap<String, QualifiedSlotRef>,
    compiled_ops: Vec<CompiledOpRecord>,
    state_lineage: Vec<CompiledStateLineage>,
    planner_payloads: BTreeMap<String, serde_json::Value>,
}

fn planned_op_payloads(
    op_path: &OpPath,
    planner_payload: Option<serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    planner_payload
        .map(|payload| BTreeMap::from([(op_path.as_str().to_string(), payload)]))
        .unwrap_or_default()
}

fn planner_payloads_value(payloads: BTreeMap<String, serde_json::Value>) -> serde_json::Value {
    serde_json::Value::Object(payloads.into_iter().collect())
}

fn validate_unique_ports(
    ports: &[PortKey],
    code: &'static str,
    message: &'static str,
) -> Result<(), SdkError> {
    let mut seen = HashSet::new();
    for PortKey(port) in ports {
        if !seen.insert(port.clone()) {
            return Err(sdk_error(code, ErrorCategory::ParsingInput, message));
        }
    }
    Ok(())
}

fn validate_op_interface(interface: &OpInterface) -> Result<(), SdkError> {
    validate_unique_ports(
        &interface.imports,
        "duplicate_import_port",
        "operation interface contained duplicate import ports",
    )?;
    validate_unique_ports(
        &interface.exports,
        "duplicate_export_port",
        "operation interface contained duplicate export ports",
    )?;
    Ok(())
}

fn state_local_id_order_key(node: &LeafStateNode) -> (String, String) {
    (
        node.addr.state_local_id.0.clone(),
        node.state_id.as_str().to_string(),
    )
}

fn ordered_leaf_states(spec: &LeafOpSpec) -> Result<Vec<LeafStateNode>, SdkError> {
    let mut nodes_by_id: HashMap<String, LeafStateNode> = HashMap::new();
    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut edges_from: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for node in &spec.states {
        let state_id = node.state_id.as_str().to_string();
        nodes_by_id.insert(state_id.clone(), node.clone());
        indegree.insert(state_id.clone(), 0);
        edges_from.entry(state_id).or_default();
    }

    for edge in &spec.edges {
        let from = edge.from.as_str().to_string();
        let to = edge.to.as_str().to_string();
        if edges_from.entry(from).or_default().insert(to.clone()) {
            *indegree.entry(to).or_default() += 1;
        }
    }

    let mut ready: BTreeSet<(String, String)> = indegree
        .iter()
        .filter(|(_state_id, deg)| **deg == 0)
        .map(|(state_id, _deg)| {
            let node = nodes_by_id.get(state_id).expect("node exists");
            state_local_id_order_key(node)
        })
        .collect();

    let mut ordered = Vec::with_capacity(spec.states.len());
    while let Some(order_key) = ready.pop_first() {
        let state_id = order_key.1.clone();
        let node = nodes_by_id.get(&state_id).expect("node exists").clone();
        ordered.push(node);

        if let Some(next_ids) = edges_from.get(&state_id) {
            for next in next_ids {
                let deg = indegree.get_mut(next).expect("next exists");
                *deg -= 1;
                if *deg == 0 {
                    let next_node = nodes_by_id.get(next).expect("node exists");
                    ready.insert(state_local_id_order_key(next_node));
                }
            }
        }
    }

    if ordered.len() != spec.states.len() {
        return Err(sdk_error(
            "state_graph_cycle",
            ErrorCategory::ParsingInput,
            "operation expanded to a cyclic state graph",
        ));
    }

    Ok(ordered)
}

fn dedupe_sorted_edges(edges: impl IntoIterator<Item = DependencyEdge>) -> Vec<DependencyEdge> {
    let mut ordered: BTreeSet<(String, String)> = BTreeSet::new();
    for edge in edges {
        ordered.insert((edge.from.as_str().to_string(), edge.to.as_str().to_string()));
    }

    ordered
        .into_iter()
        .map(|(from, to)| DependencyEdge {
            from: StateId::must_new(from),
            to: StateId::must_new(to),
        })
        .collect()
}

fn stable_topological_order(
    node_ids: impl IntoIterator<Item = String>,
    edges: &BTreeSet<(String, String)>,
    cycle_code: &'static str,
    cycle_message: &'static str,
) -> Result<Vec<String>, SdkError> {
    let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
    let mut edges_from: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for node_id in node_ids {
        indegree.entry(node_id.clone()).or_insert(0);
        edges_from.entry(node_id).or_default();
    }

    for (from, to) in edges {
        if edges_from
            .entry(from.clone())
            .or_default()
            .insert(to.clone())
        {
            *indegree.entry(to.clone()).or_insert(0) += 1;
        }
    }

    let mut ready: BTreeSet<String> = indegree
        .iter()
        .filter_map(|(node_id, deg)| (*deg == 0).then_some(node_id.clone()))
        .collect();

    let mut ordered = Vec::with_capacity(indegree.len());
    while let Some(node_id) = ready.pop_first() {
        ordered.push(node_id.clone());
        if let Some(next_ids) = edges_from.get(&node_id) {
            for next in next_ids {
                let deg = indegree.get_mut(next).expect("node exists");
                *deg -= 1;
                if *deg == 0 {
                    ready.insert(next.clone());
                }
            }
        }
    }

    if ordered.len() != indegree.len() {
        return Err(sdk_error(
            cycle_code,
            ErrorCategory::ParsingInput,
            cycle_message,
        ));
    }

    Ok(ordered)
}

fn validate_leaf_op_spec(
    spec: &LeafOpSpec,
    owning_op_path: &OpPath,
    step_root: &OpPath,
) -> Result<(), SdkError> {
    if spec.states.is_empty() {
        return Err(sdk_error(
            "empty_state_graph",
            ErrorCategory::ParsingInput,
            "operation expanded to an empty state graph",
        ));
    }

    let mut ids = HashSet::new();
    for node in &spec.states {
        if node.addr.op_path != *owning_op_path {
            return Err(sdk_error(
                "invalid_state_addr",
                ErrorCategory::ParsingInput,
                "leaf state lineage must match the owning leaf op path exactly",
            ));
        }

        if !op_path_is_nested_within(step_root, &node.addr.op_path) {
            return Err(sdk_error(
                "invalid_state_addr",
                ErrorCategory::ParsingInput,
                "leaf state lineage must stay within the owning pipeline step op path",
            ));
        }

        let expected_state_id =
            leaf_state_id(&node.addr.op_path, node.addr.state_local_id.0.clone())?;
        if expected_state_id != node.state_id {
            return Err(sdk_error(
                "invalid_state_id",
                ErrorCategory::ParsingInput,
                "leaf state id did not match the deterministic lowering rule",
            ));
        }

        let meta = node.state.meta();
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
                            node.state_id.as_str()
                        ),
                        details: None,
                    },
                });
            }
        }
        if !ids.insert(node.state_id.clone()) {
            return Err(sdk_error(
                "duplicate_state_id",
                ErrorCategory::ParsingInput,
                "duplicate StateId in expanded graph",
            ));
        }
    }

    for DependencyEdge { from, to } in &spec.edges {
        if !ids.contains(from) || !ids.contains(to) {
            return Err(sdk_error(
                "missing_state_for_edge",
                ErrorCategory::ParsingInput,
                "edge referenced a missing state id",
            ));
        }
    }

    ordered_leaf_states(spec)?;

    Ok(())
}

fn sources_and_sinks(states: &[StateId], edges: &[DependencyEdge]) -> (Vec<StateId>, Vec<StateId>) {
    let mut indeg: HashMap<StateId, usize> = HashMap::new();
    let mut outdeg: HashMap<StateId, usize> = HashMap::new();

    for state_id in states {
        indeg.insert(state_id.clone(), 0);
        outdeg.insert(state_id.clone(), 0);
    }
    for DependencyEdge { from, to } in edges {
        *outdeg.get_mut(from).expect("from exists") += 1;
        *indeg.get_mut(to).expect("to exists") += 1;
    }

    let mut sources: Vec<StateId> = states
        .iter()
        .filter(|state_id| indeg.get(*state_id).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();
    let mut sinks: Vec<StateId> = states
        .iter()
        .filter(|state_id| outdeg.get(*state_id).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();

    sources.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    sinks.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    (sources, sinks)
}

fn sorted_port_names(ports: &[PortKey]) -> Vec<String> {
    let mut names: Vec<String> = ports.iter().map(|port| port.0.clone()).collect();
    names.sort();
    names
}

fn out_slot_ref(op_path: &OpPath, export: &str) -> QualifiedSlotRef {
    QualifiedSlotRef {
        op_path: op_path.as_str().to_string(),
        slot: format!("out.{export}"),
    }
}

struct FlattenEnv<'a> {
    registry: Arc<dyn OperationRegistry>,
    step_root: &'a OpPath,
    run_config: &'a RunConfig,
}

struct PlannedOpRef<'a> {
    op_id: &'a OpId,
    op_version: &'a str,
    op_path: &'a OpPath,
}

struct FlattenInput<'a> {
    op_id: OpId,
    op_version: String,
    op_path: OpPath,
    planned: PlannedOp,
    planner_payload: Option<serde_json::Value>,
    resolved_imports: &'a BTreeMap<String, QualifiedSlotRef>,
}

fn flatten_leaf_op(
    env: &FlattenEnv<'_>,
    planned_op: PlannedOpRef<'_>,
    interface: OpInterface,
    spec: LeafOpSpec,
    planner_payload: Option<serde_json::Value>,
    resolved_imports: &BTreeMap<String, QualifiedSlotRef>,
) -> Result<FlattenedFragment, SdkError> {
    validate_leaf_op_spec(&spec, planned_op.op_path, env.step_root)?;

    let declared_imports: BTreeSet<String> = interface
        .imports
        .iter()
        .map(|port| port.0.clone())
        .collect();
    let resolved_import_names: BTreeSet<String> = resolved_imports.keys().cloned().collect();
    if declared_imports != resolved_import_names {
        return Err(sdk_error(
            "missing_import_binding",
            ErrorCategory::ParsingInput,
            "leaf operation imports were not fully resolved by the planner",
        ));
    }

    let ordered_states = ordered_leaf_states(&spec)?;
    let edges = dedupe_sorted_edges(spec.edges);
    let state_ids: Vec<StateId> = ordered_states
        .iter()
        .map(|node| node.state_id.clone())
        .collect();
    let (sources, sinks) = sources_and_sinks(&state_ids, &edges);
    let state_lineage: Vec<CompiledStateLineage> = ordered_states
        .iter()
        .map(|node| CompiledStateLineage {
            state_id: node.state_id.as_str().to_string(),
            op_path: node.addr.op_path.as_str().to_string(),
            state_local_id: node.addr.state_local_id.0.clone(),
        })
        .collect();

    let import_sources: HashMap<String, ContextKey> = resolved_imports
        .iter()
        .map(|(port, source)| (port.clone(), source.context_key()))
        .collect();
    let export_ports: BTreeSet<String> = interface
        .exports
        .iter()
        .map(|port| port.0.clone())
        .collect();

    let mut states = Vec::with_capacity(ordered_states.len());
    for node in ordered_states {
        states.push(StateNode {
            id: node.state_id.clone(),
            state: Arc::new(NamespacedState {
                op_path: planned_op.op_path.clone(),
                import_sources: import_sources.clone(),
                export_ports: export_ports.clone(),
                inner: node.state,
            }),
        });
    }

    let mut exports = BTreeMap::new();
    for PortKey(export) in interface.exports {
        exports.insert(export.clone(), out_slot_ref(planned_op.op_path, &export));
    }

    let compiled_import_bindings: Vec<CompiledImportBinding> = resolved_imports
        .iter()
        .map(|(import, source)| CompiledImportBinding {
            to_op_path: planned_op.op_path.as_str().to_string(),
            import: import.clone(),
            source: source.clone(),
        })
        .collect();
    let compiled_exports: Vec<String> = exports.keys().cloned().collect();

    Ok(FlattenedFragment {
        states,
        edges,
        sources,
        sinks,
        exports,
        compiled_ops: vec![CompiledOpRecord {
            op_path: planned_op.op_path.as_str().to_string(),
            op_id: planned_op.op_id.as_str().to_string(),
            op_version: planned_op.op_version.to_string(),
            imports: sorted_port_names(&interface.imports),
            exports: compiled_exports,
            import_bindings: compiled_import_bindings,
            after: Vec::new(),
            re_exports: Vec::new(),
        }],
        state_lineage,
        planner_payloads: planned_op_payloads(planned_op.op_path, planner_payload),
    })
}

fn resolve_port_source(
    source: &PortSource,
    parent_imports: &BTreeMap<String, QualifiedSlotRef>,
    child_fragments: &BTreeMap<String, FlattenedFragment>,
) -> Result<QualifiedSlotRef, SdkError> {
    match source {
        PortSource::ParentImport(PortKey(port)) => {
            parent_imports.get(port).cloned().ok_or_else(|| {
                sdk_error(
                    "unsatisfied_import",
                    ErrorCategory::ParsingInput,
                    "composite op referenced an unsatisfied parent import",
                )
            })
        }
        PortSource::ChildExport { child, export } => child_fragments
            .get(&child.0)
            .and_then(|fragment| fragment.exports.get(&export.0))
            .cloned()
            .ok_or_else(|| {
                sdk_error(
                    "unknown_child_export",
                    ErrorCategory::ParsingInput,
                    "composite op referenced an unknown child export",
                )
            }),
    }
}

fn flatten_composite_op(
    env: &FlattenEnv<'_>,
    planned_op: PlannedOpRef<'_>,
    interface: OpInterface,
    spec: crate::op::CompositeOpSpec,
    planner_payload: Option<serde_json::Value>,
    resolved_imports: &BTreeMap<String, QualifiedSlotRef>,
) -> Result<FlattenedFragment, SdkError> {
    let declared_parent_imports: BTreeSet<String> = interface
        .imports
        .iter()
        .map(|port| port.0.clone())
        .collect();
    let declared_parent_exports: BTreeSet<String> = interface
        .exports
        .iter()
        .map(|port| port.0.clone())
        .collect();

    let mut children_by_id: BTreeMap<String, crate::op::ChildOpInstance> = BTreeMap::new();
    for child in spec.children {
        let child_id = child.child_op_local_id.0.clone();
        if children_by_id.insert(child_id, child).is_some() {
            return Err(sdk_error(
                "duplicate_child_op_local_id",
                ErrorCategory::ParsingInput,
                "composite op declared duplicate child_op_local_id values",
            ));
        }
    }

    let child_ids: BTreeSet<String> = children_by_id.keys().cloned().collect();

    let mut expansion_dependencies: BTreeSet<(String, String)> = BTreeSet::new();
    for child in children_by_id.values() {
        if let Some(PlannerPayloadConfigSource {
            child: source_child,
            pointer: _,
        }) = &child.op_config_from_planner_payload
        {
            let source_id = source_child.0.clone();
            let target_id = child.child_op_local_id.0.clone();
            if !child_ids.contains(&source_id) {
                return Err(sdk_error(
                    "unknown_child_op",
                    ErrorCategory::ParsingInput,
                    "composite op planner-payload config referenced an unknown child",
                ));
            }
            expansion_dependencies.insert((source_id, target_id));
        }
    }

    let mut child_dependencies: BTreeSet<(String, String)> = BTreeSet::new();
    for after in spec.order {
        let from = after.from_child.0.clone();
        let to = after.to_child.0.clone();
        if !child_ids.contains(&from) || !child_ids.contains(&to) {
            return Err(sdk_error(
                "unknown_child_op",
                ErrorCategory::ParsingInput,
                "composite op order edge referenced an unknown child",
            ));
        }
        child_dependencies.insert((from.clone(), to.clone()));
        expansion_dependencies.insert((from, to));
    }

    let expansion_order = stable_topological_order(
        children_by_id.keys().cloned().collect::<Vec<_>>(),
        &expansion_dependencies,
        "composite_cycle",
        "composite op dependency graph contained a cycle",
    )?;

    let mut expanded_children: BTreeMap<
        String,
        (OpPath, OpId, String, PlannedOp, Option<serde_json::Value>),
    > = BTreeMap::new();
    for child_id in &expansion_order {
        let child = children_by_id.get(child_id).expect("child exists");
        let child_op = env.registry.resolve(&child.op_id, &child.op_version)?;
        let child_op_path = child_op_path(planned_op.op_path, child_id.clone())?;
        let resolved_op_config =
            resolve_child_op_config(child, &expanded_children, planned_op.op_path)?;
        canonical_json_bytes(&resolved_op_config).map_err(|e| match e {
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
        let planned =
            child_op.expand(child_op_path.clone(), &resolved_op_config, env.run_config)?;
        let child_planner_payload =
            child_op.planner_payload(child_op_path.clone(), &resolved_op_config, env.run_config)?;
        validate_op_interface(&planned.interface)?;
        expanded_children.insert(
            child_id.clone(),
            (
                child_op_path,
                child.op_id.clone(),
                child.op_version.clone(),
                planned,
                child_planner_payload,
            ),
        );
    }

    let mut bindings_by_child: BTreeMap<String, BTreeMap<String, PortSource>> = BTreeMap::new();
    for binding in spec.bindings {
        let child_id = binding.to_child.0.clone();
        if !child_ids.contains(&child_id) {
            return Err(sdk_error(
                "unknown_child_op",
                ErrorCategory::ParsingInput,
                "composite op binding referenced an unknown child",
            ));
        }

        let import = binding.import.0.clone();
        let child_bindings = bindings_by_child.entry(child_id).or_default();
        if child_bindings.insert(import, binding.source).is_some() {
            return Err(sdk_error(
                "duplicate_child_import_binding",
                ErrorCategory::ParsingInput,
                "composite op bound the same child import more than once",
            ));
        }
    }

    for (child_id, (_child_path, _child_op_id, _child_op_version, planned, _planner_payload)) in
        &expanded_children
    {
        let declared_child_imports: BTreeSet<String> = planned
            .interface
            .imports
            .iter()
            .map(|port| port.0.clone())
            .collect();
        let declared_child_exports: BTreeSet<String> = planned
            .interface
            .exports
            .iter()
            .map(|port| port.0.clone())
            .collect();
        let child_bindings = bindings_by_child.get(child_id);

        for import in &declared_child_imports {
            let Some(source) = child_bindings.and_then(|bindings| bindings.get(import)) else {
                return Err(sdk_error(
                    "missing_child_import_binding",
                    ErrorCategory::ParsingInput,
                    "composite op did not bind every child import exactly once",
                ));
            };

            match source {
                PortSource::ParentImport(PortKey(parent_import)) => {
                    if !declared_parent_imports.contains(parent_import)
                        || !resolved_imports.contains_key(parent_import)
                    {
                        return Err(sdk_error(
                            "unsatisfied_import",
                            ErrorCategory::ParsingInput,
                            "composite op referenced an unsatisfied parent import",
                        ));
                    }
                }
                PortSource::ChildExport { child, export } => {
                    child_dependencies.insert((child.0.clone(), child_id.clone()));

                    let Some((
                        _source_path,
                        _source_op_id,
                        _source_op_version,
                        source_planned,
                        _source_planner_payload,
                    )) = expanded_children.get(&child.0)
                    else {
                        return Err(sdk_error(
                            "unknown_child_op",
                            ErrorCategory::ParsingInput,
                            "composite op binding referenced an unknown child",
                        ));
                    };
                    if !source_planned
                        .interface
                        .exports
                        .iter()
                        .any(|candidate| candidate.0 == export.0)
                    {
                        return Err(sdk_error(
                            "unknown_child_export",
                            ErrorCategory::ParsingInput,
                            "composite op binding referenced an unknown child export",
                        ));
                    }
                }
            }
        }

        if let Some(child_bindings) = child_bindings {
            for import in child_bindings.keys() {
                if !declared_child_imports.contains(import) {
                    return Err(sdk_error(
                        "unknown_child_import",
                        ErrorCategory::ParsingInput,
                        "composite op binding referenced an unknown child import",
                    ));
                }
            }
        }

        let _ = declared_child_exports;
    }

    let mut re_exports: BTreeMap<String, PortSource> = BTreeMap::new();
    for re_export in spec.re_exports {
        let export = re_export.export.0.clone();
        if re_exports
            .insert(export.clone(), re_export.source)
            .is_some()
        {
            return Err(sdk_error(
                "duplicate_re_export",
                ErrorCategory::ParsingInput,
                "composite op declared duplicate re-export names",
            ));
        }
    }

    for export in re_exports.keys() {
        if !declared_parent_exports.contains(export) {
            return Err(sdk_error(
                "undeclared_re_export",
                ErrorCategory::ParsingInput,
                "composite op re-exported a port that is not part of its interface",
            ));
        }
    }
    for export in &declared_parent_exports {
        if !re_exports.contains_key(export) {
            return Err(sdk_error(
                "missing_re_export_binding",
                ErrorCategory::ParsingInput,
                "composite op did not bind every declared export",
            ));
        }
    }

    for source in re_exports.values() {
        match source {
            PortSource::ParentImport(PortKey(parent_import)) => {
                if !declared_parent_imports.contains(parent_import)
                    || !resolved_imports.contains_key(parent_import)
                {
                    return Err(sdk_error(
                        "unsatisfied_import",
                        ErrorCategory::ParsingInput,
                        "composite op referenced an unsatisfied parent import",
                    ));
                }
            }
            PortSource::ChildExport { child, export } => {
                let Some((
                    _child_path,
                    _child_op_id,
                    _child_op_version,
                    planned,
                    _child_planner_payload,
                )) = expanded_children.get(&child.0)
                else {
                    return Err(sdk_error(
                        "unknown_child_op",
                        ErrorCategory::ParsingInput,
                        "composite op re-export referenced an unknown child",
                    ));
                };
                if !planned
                    .interface
                    .exports
                    .iter()
                    .any(|candidate| candidate.0 == export.0)
                {
                    return Err(sdk_error(
                        "unknown_child_export",
                        ErrorCategory::ParsingInput,
                        "composite op re-export referenced an unknown child export",
                    ));
                }
            }
        }
    }

    let ordered_children = stable_topological_order(
        children_by_id.keys().cloned().collect::<Vec<_>>(),
        &child_dependencies,
        "composite_cycle",
        "composite op dependency graph contained a cycle",
    )?;

    let mut child_fragments: BTreeMap<String, FlattenedFragment> = BTreeMap::new();
    let mut all_states = Vec::new();
    let mut all_edges = Vec::new();
    let mut seen_state_ids = HashSet::new();
    let mut compiled_ops = Vec::new();
    let mut state_lineage = Vec::new();
    let mut planner_payloads = planned_op_payloads(planned_op.op_path, planner_payload);

    for child_id in &ordered_children {
        let (child_path, child_op_id, child_op_version, planned, child_planner_payload) =
            expanded_children
                .get(child_id)
                .expect("child exists")
                .clone();
        let child_bindings = bindings_by_child.get(child_id).cloned().unwrap_or_default();
        let mut child_imports: BTreeMap<String, QualifiedSlotRef> = BTreeMap::new();
        for (import, source) in child_bindings {
            let resolved = resolve_port_source(&source, resolved_imports, &child_fragments)?;
            child_imports.insert(import, resolved);
        }

        let fragment = flatten_planned_op(
            env,
            FlattenInput {
                op_id: child_op_id,
                op_version: child_op_version,
                op_path: child_path,
                planned,
                planner_payload: child_planner_payload,
                resolved_imports: &child_imports,
            },
        )?;

        for state in &fragment.states {
            if !seen_state_ids.insert(state.id.clone()) {
                return Err(sdk_error(
                    "duplicate_state_id",
                    ErrorCategory::ParsingInput,
                    "duplicate lowered StateId in recursively flattened plan",
                ));
            }
        }

        all_edges.extend(fragment.edges.iter().cloned());
        all_states.extend(fragment.states.iter().cloned());
        compiled_ops.extend(fragment.compiled_ops.iter().cloned());
        state_lineage.extend(fragment.state_lineage.iter().cloned());
        planner_payloads.extend(
            fragment
                .planner_payloads
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        child_fragments.insert(child_id.clone(), fragment);
    }

    for (from_child, to_child) in &child_dependencies {
        let from_fragment = child_fragments.get(from_child).expect("child exists");
        let to_fragment = child_fragments.get(to_child).expect("child exists");
        for from in &from_fragment.sinks {
            for to in &to_fragment.sources {
                all_edges.push(DependencyEdge {
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
    }

    let edges = dedupe_sorted_edges(all_edges);
    let state_ids: Vec<StateId> = all_states.iter().map(|state| state.id.clone()).collect();
    let (sources, sinks) = sources_and_sinks(&state_ids, &edges);

    let mut exports = BTreeMap::new();
    let mut compiled_re_exports = Vec::new();
    for export in &interface.exports {
        let source = re_exports
            .get(&export.0)
            .expect("validated re-export exists");
        let resolved = resolve_port_source(source, resolved_imports, &child_fragments)?;
        exports.insert(export.0.clone(), resolved);
        compiled_re_exports.push(CompiledReExport {
            export: export.0.clone(),
            source: exports.get(&export.0).expect("export exists").clone(),
        });
    }

    let mut compiled_import_bindings: Vec<CompiledImportBinding> = Vec::new();
    for (child_id, child_bindings) in &bindings_by_child {
        let child_path = child_op_path(planned_op.op_path, child_id.clone())
            .expect("validated child path")
            .as_str()
            .to_string();
        for (import, source) in child_bindings {
            let resolved = resolve_port_source(source, resolved_imports, &child_fragments)
                .expect("validated binding resolves");
            compiled_import_bindings.push(CompiledImportBinding {
                to_op_path: child_path.clone(),
                import: import.clone(),
                source: resolved,
            });
        }
    }
    let compiled_after: Vec<CompiledAfterEdge> = child_dependencies
        .iter()
        .map(|(from_child, to_child)| CompiledAfterEdge {
            from_op_path: format!("{}.{}", planned_op.op_path.as_str(), from_child),
            to_op_path: format!("{}.{}", planned_op.op_path.as_str(), to_child),
        })
        .collect();
    compiled_ops.insert(
        0,
        CompiledOpRecord {
            op_path: planned_op.op_path.as_str().to_string(),
            op_id: planned_op.op_id.as_str().to_string(),
            op_version: planned_op.op_version.to_string(),
            imports: sorted_port_names(&interface.imports),
            exports: sorted_port_names(&interface.exports),
            import_bindings: compiled_import_bindings,
            after: compiled_after,
            re_exports: compiled_re_exports,
        },
    );

    Ok(FlattenedFragment {
        states: all_states,
        edges,
        sources,
        sinks,
        exports,
        compiled_ops,
        state_lineage,
        planner_payloads,
    })
}

fn resolve_child_op_config(
    child: &crate::op::ChildOpInstance,
    expanded_children: &BTreeMap<
        String,
        (OpPath, OpId, String, PlannedOp, Option<serde_json::Value>),
    >,
    _parent_op_path: &OpPath,
) -> Result<serde_json::Value, SdkError> {
    let Some(source) = &child.op_config_from_planner_payload else {
        return Ok(child.op_config.clone());
    };

    let Some((_path, _op_id, _op_version, _planned, planner_payload)) =
        expanded_children.get(&source.child.0)
    else {
        return Err(sdk_error(
            "unsatisfied_child_planner_payload_config",
            ErrorCategory::ParsingInput,
            "composite op child config referenced a planner payload that was not available",
        ));
    };

    let Some(planner_payload) = planner_payload else {
        return Err(sdk_error(
            "missing_child_planner_payload",
            ErrorCategory::ParsingInput,
            "composite op child config referenced a child without planner payload",
        ));
    };

    if source.pointer.is_empty() {
        return Ok(planner_payload.clone());
    }

    planner_payload
        .pointer(&source.pointer)
        .cloned()
        .ok_or_else(|| {
            sdk_error(
                "missing_child_planner_payload_pointer",
                ErrorCategory::ParsingInput,
                "composite op child config pointer was missing in planner payload",
            )
        })
}

fn flatten_planned_op(
    env: &FlattenEnv<'_>,
    input: FlattenInput<'_>,
) -> Result<FlattenedFragment, SdkError> {
    validate_op_interface(&input.planned.interface)?;

    let planned_op = PlannedOpRef {
        op_id: &input.op_id,
        op_version: &input.op_version,
        op_path: &input.op_path,
    };
    let planner_payload = input.planner_payload;
    let PlannedOp { interface, kind } = input.planned;
    match kind {
        PlannedOpKind::Leaf(spec) => flatten_leaf_op(
            env,
            planned_op,
            interface,
            spec,
            planner_payload,
            input.resolved_imports,
        ),
        PlannedOpKind::Composite(spec) => flatten_composite_op(
            env,
            planned_op,
            interface,
            spec,
            planner_payload,
            input.resolved_imports,
        ),
    }
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
/// - recursively flattening composite child ops before runtime starts
/// - enforcing deterministic import/export wiring across child ops and pipeline steps
/// - wrapping leaf states in a namespaced context view
/// - preserving deterministic step ordering by linking sink states to the next step's sources
///
/// Use this planner for the repository's default flatten-before-runtime contract: pipelines and
/// composite ops converge to one final flat execution graph, and runtime executes states only.
#[derive(Clone, Default)]
pub struct DefaultPipelinePlanner;

impl PipelinePlanner for DefaultPipelinePlanner {
    fn build_planned_execution(
        &self,
        registry: Arc<dyn OperationRegistry>,
        pipeline: &Pipeline,
        run_config: &RunConfig,
    ) -> Result<PlannedPipelineExecution, SdkError> {
        validate_pipeline(pipeline)?;

        let mut all_states: Vec<StateNode> = Vec::new();
        let mut all_edges: Vec<DependencyEdge> = Vec::new();
        let mut seen_state_ids: HashSet<StateId> = HashSet::new();

        let mut exports_by_port: BTreeMap<String, QualifiedSlotRef> = BTreeMap::new();
        let mut step_sources: Vec<Vec<StateId>> = Vec::new();
        let mut step_sinks: Vec<Vec<StateId>> = Vec::new();
        let mut compiled_ops = Vec::new();
        let mut state_lineage = Vec::new();
        let mut planner_payloads = BTreeMap::new();

        for PipelineStep {
            step_id,
            op_id,
            op_version,
            op_config,
        } in &pipeline.steps
        {
            let op = registry.resolve(op_id, op_version)?;
            let op_path = op_path(&pipeline.machine_id, step_id);
            let planned = op.expand(op_path.clone(), op_config, run_config)?;
            let step_planner_payload =
                op.planner_payload(op_path.clone(), op_config, run_config)?;
            validate_op_interface(&planned.interface)?;

            let mut import_sources: BTreeMap<String, QualifiedSlotRef> = BTreeMap::new();
            for PortKey(import) in &planned.interface.imports {
                let Some(source) = exports_by_port.get(import) else {
                    return Err(sdk_error(
                        "unsatisfied_import",
                        ErrorCategory::ParsingInput,
                        "pipeline import was not satisfiable by previous exports",
                    ));
                };
                import_sources.insert(import.clone(), source.clone());
            }

            let flatten_env = FlattenEnv {
                registry: Arc::clone(&registry),
                step_root: &op_path,
                run_config,
            };
            let fragment = flatten_planned_op(
                &flatten_env,
                FlattenInput {
                    op_id: op_id.clone(),
                    op_version: op_version.clone(),
                    op_path: op_path.clone(),
                    planned,
                    planner_payload: step_planner_payload,
                    resolved_imports: &import_sources,
                },
            )?;

            for state in &fragment.states {
                if !seen_state_ids.insert(state.id.clone()) {
                    return Err(sdk_error(
                        "duplicate_state_id",
                        ErrorCategory::ParsingInput,
                        "duplicate StateId across pipeline steps",
                    ));
                }
            }

            all_states.extend(fragment.states);
            all_edges.extend(fragment.edges.clone());
            step_sources.push(fragment.sources);
            step_sinks.push(fragment.sinks);
            compiled_ops.extend(fragment.compiled_ops);
            state_lineage.extend(fragment.state_lineage);
            planner_payloads.extend(fragment.planner_payloads);

            for (export, source) in fragment.exports {
                if exports_by_port.insert(export, source).is_some() {
                    return Err(sdk_error(
                        "duplicate_export_port",
                        ErrorCategory::ParsingInput,
                        "pipeline composition produced duplicate exported ports",
                    ));
                }
            }
        }

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

        let root_exports: Vec<CompiledRootExport> = exports_by_port
            .iter()
            .map(|(export, slot)| CompiledRootExport {
                export: export.clone(),
                slot: slot.clone(),
            })
            .collect();

        Ok(PlannedPipelineExecution {
            execution_plan: ExecutionPlan {
                op_id: OpId::must_new(pipeline.machine_id.0.clone()),
                graph: StateGraph {
                    states: all_states,
                    edges: dedupe_sorted_edges(all_edges),
                },
            },
            compiled_execution_spec: Some(CompiledExecutionSpec {
                schema_version: COMPILED_EXECUTION_SPEC_SCHEMA_V1.to_string(),
                root_op_id: pipeline.machine_id.0.clone(),
                root_op_version: pipeline.pipeline_version.clone(),
                root_op_path: pipeline.machine_id.0.clone(),
                ops: compiled_ops,
                root_exports,
                state_lineage,
                planner_payloads: planner_payloads_value(planner_payloads),
            }),
        })
    }
}

struct NamespacedContext<'a> {
    op_path: &'a OpPath,
    import_sources: &'a HashMap<String, ContextKey>,
    export_ports: &'a BTreeSet<String>,
    inner: &'a mut dyn DynContext,
}

impl NamespacedContext<'_> {
    fn qualify_slot(&self, slot: &str) -> ContextKey {
        ContextKey(format!("{}.{}", self.op_path.0, slot))
    }

    fn explicit_slot(key: &ContextKey) -> Option<(ContextSlot, &str)> {
        key.explicit_slot()
    }

    fn qualify_read(&self, key: &ContextKey) -> ContextKey {
        if let Some((prefix, suffix)) = Self::explicit_slot(key) {
            return match prefix {
                ContextSlot::In => self
                    .import_sources
                    .get(suffix)
                    .cloned()
                    .unwrap_or_else(|| self.qualify_slot(&key.0)),
                ContextSlot::Out | ContextSlot::Work => self.qualify_slot(&key.0),
            };
        }

        if self.export_ports.contains(&key.0) {
            self.qualify_slot(&format!("out.{}", key.0))
        } else if let Some(source) = self.import_sources.get(&key.0) {
            source.clone()
        } else {
            self.qualify_slot(&format!("work.{}", key.0))
        }
    }

    fn qualify_write(&self, key: &ContextKey) -> ContextKey {
        if Self::explicit_slot(key).is_some() {
            return self.qualify_slot(&key.0);
        }

        if self.export_ports.contains(&key.0) {
            self.qualify_slot(&format!("out.{}", key.0))
        } else {
            self.qualify_slot(&format!("work.{}", key.0))
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
        self.inner.write(self.qualify_write(&key), value)
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), mfm_machine::errors::ContextError> {
        self.inner.delete(&self.qualify_write(key))
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
    import_sources: HashMap<String, ContextKey>,
    export_ports: BTreeSet<String>,
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
            export_ports: &self.export_ports,
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

const COMPILED_EXECUTION_SPEC_ARTIFACT_KIND: &str = "compiled_execution_spec";
const COMPILED_EXECUTION_SPEC_ARTIFACT_ID_CONTEXT_KEY: &str =
    "mfm.compiled_execution_spec_artifact_id";

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
        let planned_execution = planner
            .build_planned_execution(Arc::clone(&registry), &req.pipeline, &req.run_config)
            .map_err(|e| RunError::InvalidPlan(e.info))?;
        let plan = planned_execution.execution_plan;
        let compiled_execution_spec = planned_execution.compiled_execution_spec;

        let LaunchPipeline {
            pipeline,
            input,
            run_config,
            build,
            mut initial_context,
        } = req;

        let input_params = serde_json::to_value(PipelineManifestInput {
            pipeline: pipeline.clone(),
            input,
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
            op_id: OpId::must_new(pipeline.machine_id.0.clone()),
            op_version: pipeline.pipeline_version.clone(),
            input_params,
            run_config: run_config.clone(),
            build,
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
        let stored_id =
            put_artifact_verified(stores.artifacts.as_ref(), ArtifactKind::Manifest, bytes)
                .await
                .map_err(RunError::Storage)?;

        if let Some(spec) = compiled_execution_spec {
            let spec_value = serde_json::to_value(&spec).map_err(|_| {
                RunError::InvalidPlan(info(
                    "compiled_execution_spec_serialize_failed",
                    ErrorCategory::ParsingInput,
                    false,
                    "failed to serialize compiled execution spec",
                ))
            })?;
            let spec_bytes = canonical_json_bytes(&spec_value).map_err(|e| match e {
                CanonicalJsonError::FloatNotAllowed => RunError::InvalidPlan(info(
                    "compiled_execution_spec_not_canonical",
                    ErrorCategory::ParsingInput,
                    false,
                    "compiled execution spec is not canonical-json-hashable (floats are forbidden)",
                )),
                CanonicalJsonError::SecretsNotAllowed => RunError::InvalidPlan(info(
                    "secrets_detected",
                    ErrorCategory::ParsingInput,
                    false,
                    "compiled execution spec contained secrets (policy forbids persisting secrets)",
                )),
            })?;
            let stored_spec_id = put_artifact_verified(
                stores.artifacts.as_ref(),
                ArtifactKind::Other(COMPILED_EXECUTION_SPEC_ARTIFACT_KIND.to_string()),
                spec_bytes,
            )
            .await
            .map_err(RunError::Storage)?;
            initial_context
                .write(
                    ContextKey(COMPILED_EXECUTION_SPEC_ARTIFACT_ID_CONTEXT_KEY.to_string()),
                    serde_json::json!(stored_spec_id.as_str()),
                )
                .map_err(RunError::Context)?;
        }

        engine
            .start(
                stores,
                StartRun {
                    manifest,
                    manifest_id: stored_id,
                    plan,
                    run_config,
                    initial_context,
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

/// Error returned while loading a persisted context snapshot artifact.
#[derive(Debug)]
pub enum ContextSnapshotLoadError {
    /// Loading the artifact bytes from the content store failed.
    Storage(StorageError),
    /// The artifact existed but was not valid JSON.
    InvalidSnapshot,
}

impl std::fmt::Display for ContextSnapshotLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(err) => {
                let info = match err {
                    StorageError::Concurrency(info)
                    | StorageError::NotFound(info)
                    | StorageError::Corruption(info)
                    | StorageError::Other(info) => info,
                };
                write!(f, "{}: {}", info.code.0, info.message)
            }
            Self::InvalidSnapshot => f.write_str("context snapshot artifact was not JSON"),
        }
    }
}

impl std::error::Error for ContextSnapshotLoadError {}

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

fn context_snapshot_load_error_from_storage(err: StorageError) -> ContextSnapshotLoadError {
    ContextSnapshotLoadError::Storage(err)
}

/// Loads and decodes a persisted context snapshot artifact as JSON.
pub async fn load_context_snapshot_json(
    artifacts: &dyn ArtifactStore,
    snapshot_id: &ArtifactId,
) -> Result<serde_json::Value, ContextSnapshotLoadError> {
    let snapshot_bytes = artifacts
        .get(snapshot_id)
        .await
        .map_err(context_snapshot_load_error_from_storage)?;

    serde_json::from_slice(&snapshot_bytes).map_err(|_| ContextSnapshotLoadError::InvalidSnapshot)
}

/// Resolves a context key by trying direct and slot-prefixed snapshot fallbacks.
///
/// This first checks all direct candidates produced by [`context_key_candidates`].
/// If no direct candidate exists, it falls back to matching nested `.out.` shaped keys.
///
/// Returns a matching value clone when one candidate is found, otherwise `None`.
pub fn context_value_with_slot_fallback(
    snapshot: &serde_json::Value,
    key: &ContextKey,
) -> Option<serde_json::Value> {
    let candidates = context_key_candidates(&key.0);
    if let Some(value) = candidates
        .into_iter()
        .find_map(|candidate| snapshot.get(&candidate).cloned())
    {
        return Some(value);
    }

    let (op_path, leaf) = key.0.rsplit_once(".out.")?;
    let nested_prefix = format!("{op_path}.");
    let nested_suffix = format!(".out.{leaf}");
    let snapshot_obj = snapshot.as_object()?;
    let mut matches: Vec<&serde_json::Value> = snapshot_obj
        .iter()
        .filter(|(candidate, _)| {
            candidate.starts_with(&nested_prefix)
                && candidate.len() > key.0.len()
                && candidate.ends_with(&nested_suffix)
        })
        .map(|(_, value)| value)
        .collect();
    if matches.len() == 1 {
        return matches.pop().cloned();
    }

    None
}

/// Resolves and decodes a typed context value by trying direct and slot-prefixed snapshot
/// fallbacks.
pub fn decode_context_value_with_slot_fallback<T: serde::de::DeserializeOwned>(
    snapshot: &serde_json::Value,
    key: &ContextKey,
) -> Result<Option<T>, serde_json::Error> {
    context_value_with_slot_fallback(snapshot, key)
        .map(serde_json::from_value)
        .transpose()
}

fn context_key_candidates(key: &str) -> Vec<String> {
    let key = ContextKey(key.to_string());
    let mut candidates = vec![key.0.clone()];
    if key.explicit_slot().is_some() {
        return candidates;
    }
    let Some((prefix, leaf)) = key.0.rsplit_once('.') else {
        return candidates;
    };
    candidates.push(format!("{prefix}.out.{leaf}"));
    candidates.push(format!("{prefix}.work.{leaf}"));
    candidates
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

    let snapshot = load_context_snapshot_json(stores.artifacts.as_ref(), &final_snapshot_id)
        .await
        .map_err(|err| match err {
            ContextSnapshotLoadError::Storage(err) => single_op_report_error_from_storage(err),
            ContextSnapshotLoadError::InvalidSnapshot => {
                SingleOpReportError::new("InvalidSnapshot", "final snapshot artifact was not JSON")
            }
        })?;

    decode_context_value_with_slot_fallback(&snapshot, &ContextKey(req.report_context_key))
        .map_err(|_| {
            SingleOpReportError::new(
                "InvalidReport",
                "failed to decode report payload from final snapshot",
            )
        })?
        .ok_or_else(|| {
            SingleOpReportError::new(
                "MissingReport",
                "run completed without a report payload in snapshot context",
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
        /// Child run initial context snapshot artifact identifier.
        pub child_initial_snapshot_id: ArtifactId,
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
        child_initial_snapshot_id: ArtifactId,
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
            child_initial_snapshot_id: parsed.child_initial_snapshot_id,
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
