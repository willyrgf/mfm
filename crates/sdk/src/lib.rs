#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![warn(missing_docs)]
//! Orchestration SDK for planning and launching MFM runs.
//!
//! `mfm-sdk` sits above `mfm-machine` and below transport layers such as the CLI and REST API.
//! It provides stable identifiers, operation and pipeline traits, and launcher contracts for
//! turning deterministic op descriptions into executable `mfm-machine` plans.
//!
//! Source of truth: `docs/design.md` (v4) Appendix C.2.
//!
//! # Examples
//!
//! ```rust
//! use async_trait::async_trait;
//! use std::sync::Arc;
//!
//! use mfm_machine::config::RunConfig;
//! use mfm_machine::context::DynContext;
//! use mfm_machine::errors::StateError;
//! use mfm_machine::ids::{OpId, OpPath};
//! use mfm_machine::io::IoProvider;
//! use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta};
//! use mfm_machine::recorder::EventRecorder;
//! use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
//! use mfm_sdk::errors::SdkError;
//! use mfm_sdk::op::{
//!     leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
//! };
//! use mfm_sdk::unstable::HashMapOperationRegistry;
//!
//! struct ExampleState;
//!
//! #[async_trait]
//! impl State for ExampleState {
//!     fn meta(&self) -> StateMeta {
//!         StateMeta {
//!             tags: Vec::new(),
//!             depends_on: Vec::new(),
//!             depends_on_strategy: DependencyStrategy::Latest,
//!             side_effects: SideEffectKind::Pure,
//!             idempotency: Idempotency::None,
//!         }
//!     }
//!
//!     async fn handle(
//!         &self,
//!         _ctx: &mut dyn DynContext,
//!         _io: &mut dyn IoProvider,
//!         _rec: &mut dyn EventRecorder,
//!     ) -> Result<StateOutcome, StateError> {
//!         Ok(StateOutcome {
//!             snapshot: SnapshotPolicy::OnSuccess,
//!         })
//!     }
//! }
//!
//! struct ExampleOp;
//!
//! impl Operation for ExampleOp {
//!     fn op_id(&self) -> OpId {
//!         OpId::must_new("example")
//!     }
//!
//!     fn op_version(&self) -> String {
//!         "v1".to_string()
//!     }
//!
//!     fn expand(
//!         &self,
//!         op_path: OpPath,
//!         _op_config: &serde_json::Value,
//!         _run_config: &RunConfig,
//!     ) -> Result<PlannedOp, SdkError> {
//!         Ok(PlannedOp {
//!             interface: OpInterface {
//!                 imports: Vec::new(),
//!                 exports: Vec::new(),
//!             },
//!             kind: PlannedOpKind::Leaf(LeafOpSpec {
//!                 states: vec![leaf_state_node(&op_path, "report", Arc::new(ExampleState))?],
//!                 edges: Vec::new(),
//!             }),
//!         })
//!     }
//! }
//!
//! let mut registry = HashMapOperationRegistry::default();
//! registry.register(Arc::new(ExampleOp));
//! ```
//!
//! Anything not listed in Appendix C.2 is internal or unstable, even if temporarily public.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use mfm_machine::config::{BuildProvenance, RunConfig};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunResult, Stores};
use mfm_machine::errors::{ErrorInfo, RunError};
use mfm_machine::ids::{OpId, OpPath, RunId};
use mfm_machine::plan::{DependencyEdge, ExecutionPlan};

/// Stable identifiers used by pipeline planners and operation definitions.
pub mod ids {
    use super::*;

    /// Enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct MachineId(pub String);

    /// Enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StepId(pub String);

    /// Local state id within an operation.
    ///
    /// Enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StateLocalId(pub String);

    /// Parent-local identity for a child operation instance.
    ///
    /// Enforced: `^[a-z][a-z0-9_]{0,62}$`
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ChildOpLocalId(pub String);

    /// Logical import/export key for cross-op wiring (not a `ContextKey`).
    ///
    /// Recommended: dot-separated segments like `prices.latest_eth`.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct PortKey(pub String);

    /// Planner-visible lineage for one flattened state.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct StateAddr {
        /// Full hierarchical operation path that owns the state.
        pub op_path: OpPath,
        /// Local state identifier within that operation path.
        pub state_local_id: StateLocalId,
    }
}

/// Typed SDK errors returned by planners and launch helpers.
pub mod errors {
    use super::*;

    /// SDK planning/launch errors (no secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct SdkError {
        /// Structured error payload safe to serialize and persist.
        pub info: ErrorInfo,
    }
}

/// Traits and types for deterministic operation definitions.
pub mod op {
    use super::*;
    use crate::errors::SdkError;
    use crate::ids::{ChildOpLocalId, PortKey, StateAddr, StateLocalId};
    use mfm_machine::errors::{ErrorCategory, ErrorInfo};
    use mfm_machine::ids::{ErrorCode, StateId};
    use mfm_machine::plan::StateNode;
    use mfm_machine::state::DynState;

    fn sdk_error(code: &'static str, message: &'static str) -> SdkError {
        SdkError {
            info: ErrorInfo {
                code: ErrorCode(code.to_string()),
                category: ErrorCategory::ParsingInput,
                retryable: false,
                message: message.to_string(),
                details: None,
            },
        }
    }

    fn validate_state_local_id(state_local_id: &str) -> Result<(), SdkError> {
        OpId::new(state_local_id).map(|_| ()).map_err(|_| {
            sdk_error(
                "invalid_state_local_id",
                "state_local_id must match ^[a-z][a-z0-9_]{0,62}$",
            )
        })
    }

    fn validate_child_op_local_id(child_op_local_id: &str) -> Result<(), SdkError> {
        OpId::new(child_op_local_id).map(|_| ()).map_err(|_| {
            sdk_error(
                "invalid_child_op_local_id",
                "child_op_local_id must match ^[a-z][a-z0-9_]{0,62}$",
            )
        })
    }

    /// Declared operation interface for planner validation and parent-owned bindings.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct OpInterface {
        /// Ports this operation expects a parent to bind before execution.
        pub imports: Vec<PortKey>,
        /// Ports this operation makes available to a parent for re-binding or re-export.
        pub exports: Vec<PortKey>,
    }

    /// One leaf state plus its planner-visible lineage.
    #[derive(Clone)]
    pub struct LeafStateNode {
        /// Planner-visible lineage for the lowered runtime state.
        pub addr: StateAddr,
        /// Lowered runtime state identifier assigned at planning time.
        pub state_id: StateId,
        /// Runtime implementation invoked for this node.
        pub state: DynState,
    }

    impl LeafStateNode {
        /// Converts this planned leaf node into the runtime node executed by `mfm-machine`.
        pub fn into_state_node(self) -> StateNode {
            StateNode {
                id: self.state_id,
                state: self.state,
            }
        }
    }

    /// Executable leaf operation payload.
    #[derive(Clone)]
    pub struct LeafOpSpec {
        /// Lowered runtime states with explicit planner lineage.
        pub states: Vec<LeafStateNode>,
        /// Dependency edges between lowered runtime state identifiers.
        pub edges: Vec<DependencyEdge>,
    }

    /// Child operation instance declared by a composite parent.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ChildOpInstance {
        /// Unique parent-local child identity.
        pub child_op_local_id: ChildOpLocalId,
        /// Operation implementation identifier for this child.
        pub op_id: OpId,
        /// Operation version for this child.
        pub op_version: String,
        /// Canonical JSON configuration for this child.
        pub op_config: serde_json::Value,
    }

    /// Source for a child import or parent re-export.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum PortSource {
        /// Reuse one of the parent's imports.
        ParentImport(PortKey),
        /// Bind to an export produced by a named child.
        ChildExport {
            /// Parent-local child identity.
            child: ChildOpLocalId,
            /// Child export name.
            export: PortKey,
        },
    }

    /// Parent-owned binding that wires one child import to a parent import or sibling export.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ImportBinding {
        /// Child receiving the bound import.
        pub to_child: ChildOpLocalId,
        /// Import port on that child.
        pub import: PortKey,
        /// Planner-owned source for the bound value.
        pub source: PortSource,
    }

    /// Explicit pure-ordering dependency between two children.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct AfterEdge {
        /// Upstream child that must finish first.
        pub from_child: ChildOpLocalId,
        /// Downstream child gated by `from_child`.
        pub to_child: ChildOpLocalId,
    }

    /// Explicit parent re-export of a child export or parent import.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ReExportBinding {
        /// Export name exposed by the parent.
        pub export: PortKey,
        /// Bound source for that parent export.
        pub source: PortSource,
    }

    /// Composite operation payload with explicit child instances and bindings.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompositeOpSpec {
        /// Immediate child operations declared by the parent.
        pub children: Vec<ChildOpInstance>,
        /// Parent-owned bindings for child imports.
        pub bindings: Vec<ImportBinding>,
        /// Explicit pure-ordering edges between child instances.
        pub order: Vec<AfterEdge>,
        /// Parent exports re-bound explicitly from imports or child exports.
        pub re_exports: Vec<ReExportBinding>,
    }

    /// Planned operation payload returned by `Operation::expand`.
    #[derive(Clone)]
    pub struct PlannedOp {
        /// Declared interface for this operation instance.
        pub interface: OpInterface,
        /// Executable shape for the operation instance.
        pub kind: PlannedOpKind,
    }

    /// Planned operation shape.
    #[derive(Clone)]
    pub enum PlannedOpKind {
        /// Executable leaf op that already owns runtime states.
        Leaf(LeafOpSpec),
        /// Composite op that expands through explicit child instances and bindings.
        Composite(CompositeOpSpec),
    }

    /// Derives the hierarchical child path `<parent>.<child_op_local_id>`.
    pub fn child_op_path(
        parent_op_path: &OpPath,
        child_op_local_id: impl Into<String>,
    ) -> Result<OpPath, SdkError> {
        let child_op_local_id = child_op_local_id.into();
        validate_child_op_local_id(&child_op_local_id)?;
        OpPath::new(format!("{}.{}", parent_op_path.as_str(), child_op_local_id))
            .map_err(|_| sdk_error("invalid_op_path", "invalid hierarchical child op path"))
    }

    /// Returns the planner-visible lineage for one leaf state.
    pub fn state_addr(
        op_path: &OpPath,
        state_local_id: impl Into<String>,
    ) -> Result<StateAddr, SdkError> {
        let state_local_id = state_local_id.into();
        validate_state_local_id(&state_local_id)?;
        Ok(StateAddr {
            op_path: op_path.clone(),
            state_local_id: StateLocalId(state_local_id),
        })
    }

    /// Lowers one planner-visible state lineage into the v1 flat runtime `StateId`.
    pub fn leaf_state_id(
        op_path: &OpPath,
        state_local_id: impl Into<String>,
    ) -> Result<StateId, SdkError> {
        let addr = state_addr(op_path, state_local_id)?;
        let (machine_id, step_id) = addr.op_path.machine_and_step();
        let mut flattened = addr.op_path.flattened_child_path().unwrap_or_default();
        if !flattened.is_empty() {
            flattened.push_str("__");
        }
        flattened.push_str(&addr.state_local_id.0);

        StateId::new(format!("{machine_id}.{step_id}.{flattened}")).map_err(|_| {
            sdk_error(
                "invalid_state_id",
                "lowered state id did not satisfy the runtime naming contract",
            )
        })
    }

    /// Builds one planned leaf state node from a hierarchical op path and local id.
    pub fn leaf_state_node(
        op_path: &OpPath,
        state_local_id: impl Into<String>,
        state: DynState,
    ) -> Result<LeafStateNode, SdkError> {
        let state_local_id = state_local_id.into();
        let addr = state_addr(op_path, state_local_id.clone())?;
        let state_id = leaf_state_id(op_path, state_local_id)?;
        Ok(LeafStateNode {
            addr,
            state_id,
            state,
        })
    }

    /// A reusable operation definition.
    ///
    /// Contract:
    /// - `op_id` + `op_version` MUST be stable across environments.
    /// - `expand()` MUST be deterministic and MUST NOT perform IO.
    /// - `expand()` returns one planner-owned operation instance with explicit interface +
    ///   leaf/composite shape.
    /// - `planner_payload()` MAY emit canonical-json-safe audit/debug payloads derived from the
    ///   same deterministic planning inputs; runtime MUST NOT depend on them.
    pub trait Operation: Send + Sync {
        /// Returns the stable operation identifier used in manifests and registries.
        fn op_id(&self) -> OpId;
        /// Returns the stable operation version string.
        fn op_version(&self) -> String;

        /// Expands this operation into an executable state graph.
        fn expand(
            &self,
            op_path: OpPath,
            op_config: &serde_json::Value,
            run_config: &RunConfig,
        ) -> Result<PlannedOp, SdkError>;

        /// Returns optional planner-owned audit/debug payload for compiled execution specs.
        fn planner_payload(
            &self,
            _op_path: OpPath,
            _op_config: &serde_json::Value,
            _run_config: &RunConfig,
        ) -> Result<Option<serde_json::Value>, SdkError> {
            Ok(None)
        }
    }

    /// Shared trait-object form for storing heterogeneous operations in registries.
    pub type DynOperation = Arc<dyn Operation>;

    /// Registry used to resolve operations (by id + version) at plan/launch/resume time.
    ///
    /// For the default in-memory implementation used by most binaries and tests, see
    /// [`crate::unstable::HashMapOperationRegistry`].
    pub trait OperationRegistry: Send + Sync {
        /// Resolves an operation implementation for the requested id and version.
        fn resolve(&self, op_id: &OpId, op_version: &str) -> Result<DynOperation, SdkError>;
    }
}

/// Pipeline composition types used to flatten multi-op workflows into execution plans.
pub mod pipeline {
    use super::*;
    use crate::errors::SdkError;
    use crate::ids::{MachineId, StepId};
    use crate::op::OperationRegistry;

    /// Schema version used by the v1 compiled execution spec artifact.
    pub const COMPILED_EXECUTION_SPEC_SCHEMA_V1: &str = "compiled_execution_spec/v1";

    /// One pipeline step.
    ///
    /// Contract:
    /// - `step_id` MUST be unique within the pipeline.
    /// - `op_config` MUST be canonical-JSON hashable and MUST NOT contain secrets.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct PipelineStep {
        /// Unique step identifier within the pipeline.
        pub step_id: StepId,
        /// Operation identifier executed by this step.
        pub op_id: OpId,
        /// Operation version executed by this step.
        pub op_version: String,
        /// Canonical JSON configuration passed to the operation.
        pub op_config: serde_json::Value,
    }

    /// A flattened machine definition (ordered steps).
    ///
    /// Contract:
    /// - `machine_id` + `pipeline_version` map to `RunManifest.{op_id, op_version}`.
    /// - Step `OpPath` is "<machine_id>.<step_id>".
    /// - Single-op convention: wrap a single op as a 1-step pipeline with:
    ///   - `machine_id = <op_id>`
    ///   - `steps[0].step_id = "main"`
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Pipeline {
        /// Stable machine identifier used as the outer `OpId`.
        pub machine_id: MachineId,
        /// Version string for the overall pipeline template.
        pub pipeline_version: String,
        /// Ordered steps that make up the pipeline.
        pub steps: Vec<PipelineStep>,
    }

    /// Recommended `RunManifest.input_params` shape for pipeline runs (canonical JSON; no secrets).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct PipelineManifestInput {
        /// Pipeline definition embedded in the run manifest.
        pub pipeline: Pipeline,
        /// Additional pipeline input payload.
        pub input: serde_json::Value,
    }

    /// Fully qualified runtime slot reference emitted in compiled execution specs.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct QualifiedSlotRef {
        /// Fully qualified leaf op path that owns the slot.
        pub op_path: String,
        /// Slot name relative to that op path, for example `out.report`.
        pub slot: String,
    }

    /// One resolved import binding recorded for audit/debug compiled specs.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompiledImportBinding {
        /// Fully qualified child or leaf op path receiving the import.
        pub to_op_path: String,
        /// Import name on the op being recorded.
        pub import: String,
        /// Resolved slot supplying that import.
        pub source: QualifiedSlotRef,
    }

    /// One explicit ordering edge between child ops in the compiled execution spec.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompiledAfterEdge {
        /// Upstream child op path.
        pub from_op_path: String,
        /// Downstream child op path.
        pub to_op_path: String,
    }

    /// One explicit re-export alias in the compiled execution spec.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompiledReExport {
        /// Export name exposed by the recorded op.
        pub export: String,
        /// Resolved slot backing that export.
        pub source: QualifiedSlotRef,
    }

    /// One op instance recorded in the compiled execution spec artifact.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompiledOpRecord {
        /// Hierarchical op path for this planned op instance.
        pub op_path: String,
        /// Stable operation identifier.
        pub op_id: String,
        /// Stable operation version.
        pub op_version: String,
        /// Declared interface imports in deterministic planner order.
        pub imports: Vec<String>,
        /// Declared interface exports in deterministic planner order.
        pub exports: Vec<String>,
        /// Resolved import bindings for this op instance.
        pub import_bindings: Vec<CompiledImportBinding>,
        /// Explicit child ordering edges owned by this op instance.
        pub after: Vec<CompiledAfterEdge>,
        /// Explicit parent re-exports owned by this op instance.
        pub re_exports: Vec<CompiledReExport>,
    }

    /// One root export resolution entry in the compiled execution spec artifact.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompiledRootExport {
        /// Export name visible at the pipeline root.
        pub export: String,
        /// Resolved slot that backs the root export.
        pub slot: QualifiedSlotRef,
    }

    /// Planner-visible lineage for one lowered runtime state id.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompiledStateLineage {
        /// Lowered runtime state identifier.
        pub state_id: String,
        /// Hierarchical op path that owns the state.
        pub op_path: String,
        /// Local state identifier within that op path.
        pub state_local_id: String,
    }

    /// Optional audit/debug artifact emitted by planners in v1.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CompiledExecutionSpec {
        /// Schema version for this artifact.
        pub schema_version: String,
        /// Root pipeline or machine identifier.
        pub root_op_id: String,
        /// Root pipeline or machine version.
        pub root_op_version: String,
        /// Root planning scope for this artifact.
        pub root_op_path: String,
        /// Flattened planned op records in deterministic planner order.
        pub ops: Vec<CompiledOpRecord>,
        /// Root export resolution table.
        pub root_exports: Vec<CompiledRootExport>,
        /// Planner-visible state lineage for every lowered runtime state.
        pub state_lineage: Vec<CompiledStateLineage>,
        /// Planner-owned semantic payloads. v1 keeps this as free-form canonical JSON.
        pub planner_payloads: serde_json::Value,
    }

    /// Planner result used by launchers to persist audit/debug artifacts alongside the runtime plan.
    pub struct PlannedPipelineExecution {
        /// Final runtime plan handed to `mfm-machine`.
        pub execution_plan: ExecutionPlan,
        /// Optional compiled execution spec artifact for audit/debug use.
        pub compiled_execution_spec: Option<CompiledExecutionSpec>,
    }

    /// Pipeline planning contract (flattened composition).
    ///
    /// For the standard implementation that validates pipeline wiring and produces the flattened
    /// execution graph used by `mfm-machine`, see [`crate::unstable::DefaultPipelinePlanner`].
    pub trait PipelinePlanner: Send + Sync {
        /// Builds the runtime plan plus any optional audit/debug compiled-spec payload.
        fn build_planned_execution(
            &self,
            registry: Arc<dyn OperationRegistry>,
            pipeline: &Pipeline,
            run_config: &RunConfig,
        ) -> Result<PlannedPipelineExecution, SdkError>;

        /// Implementations MUST:
        /// - resolve ops via `OperationRegistry`
        /// - support recursive child-op expansion before runtime starts
        /// - assign deterministic hierarchical child `OpPath`s during flattening
        /// - ensure all flattened `StateId`s are unique and match the
        ///   "<machine_id>.<step_id>.<flattened_state_local_id>" convention
        /// - reject duplicate child-op paths and duplicate exported ports
        /// - enforce step order by adding dependency edges between step graphs (flattened composition)
        ///
        /// Returns the executable plan for the supplied pipeline and run config.
        fn build_execution_plan(
            &self,
            registry: Arc<dyn OperationRegistry>,
            pipeline: &Pipeline,
            run_config: &RunConfig,
        ) -> Result<ExecutionPlan, SdkError> {
            self.build_planned_execution(registry, pipeline, run_config)
                .map(|planned| planned.execution_plan)
        }
    }
}

/// Contracts for launching and resuming pipeline runs.
pub mod launcher {
    use super::*;
    use crate::op::OperationRegistry;
    use crate::pipeline::{Pipeline, PipelinePlanner};

    /// Start inputs for launching a pipeline run.
    ///
    /// This request bundles everything a launcher needs to compute the plan, persist the
    /// manifest, and hand the run to an execution engine.
    pub struct LaunchPipeline {
        /// Pipeline template to execute.
        pub pipeline: Pipeline,
        /// Canonical JSON input passed through to the manifest.
        pub input: serde_json::Value,
        /// Effective run configuration for this launch.
        pub run_config: RunConfig,
        /// Build provenance stored in the manifest.
        pub build: BuildProvenance,
        /// Initial context snapshot used as the run starting point.
        pub initial_context: Box<dyn DynContext>,
    }

    /// Run launcher contract:
    /// - plan the pipeline via `PipelinePlanner`
    /// - compute + store the `RunManifest` artifact (content-addressed)
    ///   - `RunManifest.input_params` SHOULD embed `PipelineManifestInput { pipeline, input }` (no secrets)
    /// - call `ExecutionEngine::{start,resume}`
    ///
    /// For the standard implementation that follows the repository manifest layout and resume
    /// contract, see [`crate::unstable::DefaultRunLauncher`].
    #[async_trait]
    pub trait RunLauncher: Send + Sync {
        /// Plans the supplied pipeline, persists its manifest, and starts a new run.
        async fn start_pipeline(
            &self,
            engine: Arc<dyn ExecutionEngine>,
            stores: Stores,
            registry: Arc<dyn OperationRegistry>,
            planner: Arc<dyn PipelinePlanner>,
            req: LaunchPipeline,
        ) -> Result<RunResult, RunError>;

        /// Rebuilds the execution plan and resumes a previously started run.
        async fn resume(
            &self,
            engine: Arc<dyn ExecutionEngine>,
            stores: Stores,
            registry: Arc<dyn OperationRegistry>,
            planner: Arc<dyn PipelinePlanner>,
            run_id: RunId,
        ) -> Result<RunResult, RunError>;
    }
}

/// Unstable helper implementations for planning and launching.
///
/// Not part of the stable API contract (Appendix C.2).
pub mod unstable;
