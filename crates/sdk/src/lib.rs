#![warn(missing_docs)]
//! Orchestration SDK for planning and launching MFM runs.
//!
//! `mfm-sdk` sits above `mfm-machine` and below transport layers such as the CLI and REST API.
//! It provides stable identifiers, operation and pipeline traits, and launcher contracts for
//! turning deterministic op descriptions into executable `mfm-machine` plans.
//!
//! Source of truth: `docs/redesign.md` (v4) Appendix C.2.
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
//! use mfm_machine::ids::{OpId, OpPath, StateId};
//! use mfm_machine::io::IoProvider;
//! use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta};
//! use mfm_machine::plan::{StateGraph, StateNode};
//! use mfm_machine::recorder::EventRecorder;
//! use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
//! use mfm_sdk::errors::SdkError;
//! use mfm_sdk::op::{OpIo, Operation};
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
//!     fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
//!         Ok(OpIo {
//!             imports: Vec::new(),
//!             exports: Vec::new(),
//!         })
//!     }
//!
//!     fn expand(
//!         &self,
//!         _op_path: OpPath,
//!         _op_config: &serde_json::Value,
//!         _run_config: &RunConfig,
//!     ) -> Result<StateGraph, SdkError> {
//!         Ok(StateGraph {
//!             states: vec![StateNode {
//!                 id: StateId::must_new("example.main.report"),
//!                 state: Arc::new(ExampleState),
//!             }],
//!             edges: Vec::new(),
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
use mfm_machine::plan::{ExecutionPlan, StateGraph};

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

    /// Logical import/export key for cross-op wiring (not a `ContextKey`).
    ///
    /// Recommended: dot-separated segments like `prices.latest_eth`.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct PortKey(pub String);
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
    use crate::ids::PortKey;

    /// Declared op IO surface for pipeline validation.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct OpIo {
        /// Ports this operation expects earlier pipeline steps to provide.
        pub imports: Vec<PortKey>,
        /// Ports this operation makes available to later pipeline steps.
        pub exports: Vec<PortKey>,
    }

    /// A reusable operation definition.
    ///
    /// Contract:
    /// - `op_id` + `op_version` MUST be stable across environments.
    /// - `expand()` MUST be deterministic and MUST NOT perform IO.
    /// - `expand()` MUST assign StateIds of the form: "<op_path>.<state_local_id>" (3 segments).
    pub trait Operation: Send + Sync {
        /// Returns the stable operation identifier used in manifests and registries.
        fn op_id(&self) -> OpId;
        /// Returns the stable operation version string.
        fn op_version(&self) -> String;

        /// Declares the import/export surface for the provided config.
        fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError>;

        /// Expands this operation into an executable state graph.
        fn expand(
            &self,
            op_path: OpPath,
            op_config: &serde_json::Value,
            run_config: &RunConfig,
        ) -> Result<StateGraph, SdkError>;
    }

    /// Shared trait-object form for storing heterogeneous operations in registries.
    pub type DynOperation = Arc<dyn Operation>;

    /// Registry used to resolve operations (by id + version) at plan/launch/resume time.
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

    /// Pipeline planning contract (flattened composition).
    pub trait PipelinePlanner: Send + Sync {
        /// Implementations MUST:
        /// - resolve ops via `OperationRegistry`
        /// - ensure all `StateId`s are unique and match "<machine_id>.<step_id>.<state_local_id>"
        /// - enforce step order by adding dependency edges between step graphs (flattened composition)
        ///
        /// Returns the executable plan for the supplied pipeline and run config.
        fn build_execution_plan(
            &self,
            registry: Arc<dyn OperationRegistry>,
            pipeline: &Pipeline,
            run_config: &RunConfig,
        ) -> Result<ExecutionPlan, SdkError>;
    }
}

/// Contracts for launching and resuming pipeline runs.
pub mod launcher {
    use super::*;
    use crate::op::OperationRegistry;
    use crate::pipeline::{Pipeline, PipelinePlanner};

    /// Start inputs for launching a pipeline run.
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
