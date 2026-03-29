use std::sync::Arc;

use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_portfolio_config::builtin_dispatch_catalog;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    leaf_state_id, leaf_state_node, DynOperation, LeafOpSpec, OpInterface, Operation, PlannedOp,
    PlannedOpKind,
};
use mfm_state_common::errors as op_errors;
use mfm_state_portfolio::execution_states::{
    AssembleSnapshotState, MergeObservationsState, ObserveCompiledBatchState,
    PinExecutionViewsState, PrepareExecutionSourcesState, ProjectReportState, ResolveSubjectsState,
    ResolveValuationInputsState,
};
use mfm_state_portfolio::model::PortfolioConfig;
use mfm_state_portfolio::plan::{
    CompiledObservationBatch, DispatchCatalog, PortfolioExecutionSpec, SourcePreparationTask,
    SubjectResolutionTask, ValuationTask, ViewPinTask,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PREPARE_EXECUTION_SOURCES_CHILD_ID: &str = "prepare_execution_sources";
pub const RESOLVE_SUBJECTS_CHILD_ID: &str = "resolve_subjects";
pub const PIN_EXECUTION_VIEWS_CHILD_ID: &str = "pin_execution_views";
pub const RESOLVE_VALUATION_INPUTS_CHILD_ID: &str = "resolve_valuation_inputs";
pub const MERGE_OBSERVATIONS_CHILD_ID: &str = "merge_observations";
pub const ASSEMBLE_SNAPSHOT_CHILD_ID: &str = "assemble_snapshot";
pub const PROJECT_REPORT_CHILD_ID: &str = "project_report";

pub const PREPARE_EXECUTION_SOURCES_OP_ID: &str = "portfolio_prepare_execution_sources";
pub const RESOLVE_SUBJECTS_OP_ID: &str = "portfolio_resolve_subjects";
pub const PIN_EXECUTION_VIEWS_OP_ID: &str = "portfolio_pin_execution_views";
pub const RESOLVE_VALUATION_INPUTS_OP_ID: &str = "portfolio_resolve_valuation_inputs";
pub const OBSERVE_COMPILED_BATCH_OP_ID: &str = "portfolio_observe_compiled_batch";
pub const MERGE_OBSERVATIONS_OP_ID: &str = "portfolio_merge_observations";
pub const ASSEMBLE_SNAPSHOT_OP_ID: &str = "portfolio_assemble_snapshot";
pub const PROJECT_REPORT_OP_ID: &str = "portfolio_project_report";
pub const INTERNAL_OP_VERSION: &str = "v1";

pub const PORT_PREPARED_SOURCES: &str = "prepared_sources";
pub const PORT_RESOLVED_SUBJECTS: &str = "resolved_subjects";
pub const PORT_PINNED_VIEWS: &str = "pinned_views";
pub const PORT_RESOLVED_VALUATIONS: &str = "resolved_valuations";
pub const PORT_OBSERVATIONS: &str = "observations";
pub const PORT_SNAPSHOT: &str = "snapshot";
pub const PORT_SNAPSHOT_ARTIFACT_ID: &str = "snapshot_artifact_id";
pub const PORT_REPORT: &str = "report";

const STATE_LOCAL_ID: &str = "run";

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(
        code,
        mfm_machine::errors::ErrorCategory::ParsingInput,
        false,
        message,
    )
}

fn local_slot(prefix: &str, port: &str) -> ContextKey {
    ContextKey(format!("{prefix}.{port}"))
}

fn semantic_catalog() -> Result<Arc<DispatchCatalog>, SdkError> {
    builtin_dispatch_catalog().map(Arc::new).map_err(|err| {
        sdk_input_error(
            "semantic_catalog_construction_failed",
            format!("failed to construct semantic adapter catalog: {err}"),
        )
    })
}

fn decode_config<T: for<'de> Deserialize<'de>>(
    op_id: &'static str,
    op_config: &Value,
) -> Result<T, SdkError> {
    serde_json::from_value(op_config.clone()).map_err(|err| {
        sdk_input_error(
            "invalid_op_config",
            format!("{op_id} op_config decode failed: {err}"),
        )
    })
}

fn encode_config<T: Serialize>(value: &T) -> Result<Value, SdkError> {
    serde_json::to_value(value).map_err(|err| {
        sdk_input_error(
            "semantic_op_config_serialize_failed",
            format!("failed to serialize semantic child op config: {err}"),
        )
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrepareExecutionSourcesOpConfig {
    pub tasks: Vec<SourcePreparationTask>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolveSubjectsOpConfig {
    pub tasks: Vec<SubjectResolutionTask>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PinExecutionViewsOpConfig {
    pub tasks: Vec<ViewPinTask>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolveValuationInputsOpConfig {
    pub tasks: Vec<ValuationTask>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObserveCompiledBatchOpConfig {
    pub batch: CompiledObservationBatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeObservationsOpConfig {
    pub input_ports: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssembleSnapshotOpConfig {
    pub portfolio: PortfolioConfig,
    pub fact_key: FactKey,
}

#[derive(Clone, Default)]
pub struct PrepareExecutionSourcesOp;

impl Operation for PrepareExecutionSourcesOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PREPARE_EXECUTION_SOURCES_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: PrepareExecutionSourcesOpConfig =
            decode_config(PREPARE_EXECUTION_SOURCES_OP_ID, op_config)?;
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![PortKey(PORT_PREPARED_SOURCES.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(PrepareExecutionSourcesState {
                        state_id: state_id.clone(),
                        tasks: cfg.tasks,
                        output_key: local_slot("out", PORT_PREPARED_SOURCES),
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[derive(Clone, Default)]
pub struct ResolveSubjectsOp;

impl Operation for ResolveSubjectsOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(RESOLVE_SUBJECTS_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: ResolveSubjectsOpConfig = decode_config(RESOLVE_SUBJECTS_OP_ID, op_config)?;
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![PortKey(PORT_PREPARED_SOURCES.to_string())],
                exports: vec![PortKey(PORT_RESOLVED_SUBJECTS.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(ResolveSubjectsState {
                        state_id: state_id.clone(),
                        tasks: cfg.tasks,
                        catalog: semantic_catalog()?,
                        prepared_sources_key: local_slot("in", PORT_PREPARED_SOURCES),
                        output_key: local_slot("out", PORT_RESOLVED_SUBJECTS),
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[derive(Clone, Default)]
pub struct PinExecutionViewsOp;

impl Operation for PinExecutionViewsOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PIN_EXECUTION_VIEWS_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: PinExecutionViewsOpConfig = decode_config(PIN_EXECUTION_VIEWS_OP_ID, op_config)?;
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![PortKey(PORT_PREPARED_SOURCES.to_string())],
                exports: vec![PortKey(PORT_PINNED_VIEWS.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(PinExecutionViewsState {
                        state_id: state_id.clone(),
                        tasks: cfg.tasks,
                        catalog: semantic_catalog()?,
                        prepared_sources_key: local_slot("in", PORT_PREPARED_SOURCES),
                        output_key: local_slot("out", PORT_PINNED_VIEWS),
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[derive(Clone, Default)]
pub struct ResolveValuationInputsOp;

impl Operation for ResolveValuationInputsOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(RESOLVE_VALUATION_INPUTS_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: ResolveValuationInputsOpConfig =
            decode_config(RESOLVE_VALUATION_INPUTS_OP_ID, op_config)?;
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![PortKey(PORT_PINNED_VIEWS.to_string())],
                exports: vec![PortKey(PORT_RESOLVED_VALUATIONS.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(ResolveValuationInputsState {
                        state_id: state_id.clone(),
                        tasks: cfg.tasks,
                        catalog: semantic_catalog()?,
                        pinned_views_key: local_slot("in", PORT_PINNED_VIEWS),
                        output_key: local_slot("out", PORT_RESOLVED_VALUATIONS),
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[derive(Clone, Default)]
pub struct ObserveCompiledBatchOp;

impl Operation for ObserveCompiledBatchOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OBSERVE_COMPILED_BATCH_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: ObserveCompiledBatchOpConfig =
            decode_config(OBSERVE_COMPILED_BATCH_OP_ID, op_config)?;
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![
                    PortKey(PORT_RESOLVED_SUBJECTS.to_string()),
                    PortKey(PORT_PINNED_VIEWS.to_string()),
                    PortKey(PORT_RESOLVED_VALUATIONS.to_string()),
                ],
                exports: vec![PortKey(PORT_OBSERVATIONS.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(ObserveCompiledBatchState {
                        state_id: state_id.clone(),
                        batch: cfg.batch,
                        catalog: semantic_catalog()?,
                        resolved_subjects_key: local_slot("in", PORT_RESOLVED_SUBJECTS),
                        pinned_views_key: local_slot("in", PORT_PINNED_VIEWS),
                        resolved_valuations_key: local_slot("in", PORT_RESOLVED_VALUATIONS),
                        output_key: local_slot("out", PORT_OBSERVATIONS),
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[derive(Clone, Default)]
pub struct MergeObservationsOp;

impl Operation for MergeObservationsOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(MERGE_OBSERVATIONS_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: MergeObservationsOpConfig = decode_config(MERGE_OBSERVATIONS_OP_ID, op_config)?;
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        let imports = cfg
            .input_ports
            .iter()
            .cloned()
            .map(PortKey)
            .collect::<Vec<_>>();
        let input_keys = cfg
            .input_ports
            .iter()
            .map(|port| local_slot("in", port))
            .collect::<Vec<_>>();
        Ok(PlannedOp {
            interface: OpInterface {
                imports,
                exports: vec![PortKey(PORT_OBSERVATIONS.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(MergeObservationsState {
                        state_id: state_id.clone(),
                        input_keys,
                        output_key: local_slot("out", PORT_OBSERVATIONS),
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[derive(Clone, Default)]
pub struct AssembleSnapshotOp;

impl Operation for AssembleSnapshotOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(ASSEMBLE_SNAPSHOT_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: AssembleSnapshotOpConfig = decode_config(ASSEMBLE_SNAPSHOT_OP_ID, op_config)?;
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![
                    PortKey(PORT_RESOLVED_SUBJECTS.to_string()),
                    PortKey(PORT_PINNED_VIEWS.to_string()),
                    PortKey(PORT_OBSERVATIONS.to_string()),
                ],
                exports: vec![
                    PortKey(PORT_SNAPSHOT_ARTIFACT_ID.to_string()),
                    PortKey(PORT_SNAPSHOT.to_string()),
                ],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(AssembleSnapshotState {
                        state_id: state_id.clone(),
                        portfolio: cfg.portfolio,
                        resolved_subjects_key: local_slot("in", PORT_RESOLVED_SUBJECTS),
                        pinned_views_key: local_slot("in", PORT_PINNED_VIEWS),
                        observations_key: local_slot("in", PORT_OBSERVATIONS),
                        fact_key: cfg.fact_key,
                        artifact_id_output_key: local_slot("out", PORT_SNAPSHOT_ARTIFACT_ID),
                        snapshot_output_key: local_slot("out", PORT_SNAPSHOT),
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[derive(Clone, Default)]
pub struct ProjectReportOp;

impl Operation for ProjectReportOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PROJECT_REPORT_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        _op_config: &Value,
        _run_config: &mfm_machine::config::RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let state_id = leaf_state_id(&op_path, STATE_LOCAL_ID)?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![PortKey(PORT_SNAPSHOT.to_string())],
                exports: vec![PortKey(PORT_REPORT.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    STATE_LOCAL_ID,
                    Arc::new(ProjectReportState {
                        state_id: state_id.clone(),
                        snapshot_key: local_slot("in", PORT_SNAPSHOT),
                        output_key: local_slot("out", PORT_REPORT),
                        event_name: "portfolio_tracker.completed",
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

pub fn child_ops() -> Vec<DynOperation> {
    vec![
        Arc::new(PrepareExecutionSourcesOp),
        Arc::new(ResolveSubjectsOp),
        Arc::new(PinExecutionViewsOp),
        Arc::new(ResolveValuationInputsOp),
        Arc::new(ObserveCompiledBatchOp),
        Arc::new(MergeObservationsOp),
        Arc::new(AssembleSnapshotOp),
        Arc::new(ProjectReportOp),
    ]
}

pub fn prepare_sources_config(spec: &PortfolioExecutionSpec) -> Result<Value, SdkError> {
    encode_config(&PrepareExecutionSourcesOpConfig {
        tasks: spec.source_tasks.clone(),
    })
}

pub fn resolve_subjects_config(spec: &PortfolioExecutionSpec) -> Result<Value, SdkError> {
    encode_config(&ResolveSubjectsOpConfig {
        tasks: spec.subject_tasks.clone(),
    })
}

pub fn pin_execution_views_config(spec: &PortfolioExecutionSpec) -> Result<Value, SdkError> {
    encode_config(&PinExecutionViewsOpConfig {
        tasks: spec.view_tasks.clone(),
    })
}

pub fn resolve_valuations_config(spec: &PortfolioExecutionSpec) -> Result<Value, SdkError> {
    encode_config(&ResolveValuationInputsOpConfig {
        tasks: spec.valuation_tasks.clone(),
    })
}

pub fn observe_batch_config(batch: &CompiledObservationBatch) -> Result<Value, SdkError> {
    encode_config(&ObserveCompiledBatchOpConfig {
        batch: batch.clone(),
    })
}

pub fn merge_observations_config(input_ports: Vec<String>) -> Result<Value, SdkError> {
    encode_config(&MergeObservationsOpConfig { input_ports })
}

pub fn assemble_snapshot_config(
    portfolio: &PortfolioConfig,
    fact_key: FactKey,
) -> Result<Value, SdkError> {
    encode_config(&AssembleSnapshotOpConfig {
        portfolio: portfolio.clone(),
        fact_key,
    })
}
