use std::sync::Arc;

use mfm_aave_v3_origin_config::{
    build_aave_v3_origin_stack_outcome, decode_aave_v3_origin_stack_canonical_config,
};
use mfm_machine::config::RunConfig;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_machine::plan::DependencyEdge;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    child_op_path, leaf_state_id, leaf_state_node, DynOperation, LeafOpSpec, OpInterface,
    Operation, PlannedOp, PlannedOpKind,
};
use mfm_state_common::states::publish::{WriteContextValueArtifactState, WriteJsonValueState};

/// Public root op id for the canonical-to-built Aave Origin config workflow.
pub const AAVE_V3_ORIGIN_STACK_CONFIG_BUILD_OP_ID: &str = "aave_v3_origin_stack_config_build";

const CONFIG_BUILD_MAIN_OP_PATH: &str = "aave_v3_origin_stack_config_build.main";
const PORT_BUILT_CONFIG: &str = "built_config";
const PORT_CANONICAL_ARTIFACT_ID: &str = "canonical_config_artifact_id";
const PORT_BUILT_ARTIFACT_ID: &str = "built_config_artifact_id";
const PORT_BUILD_REPORT: &str = "report";
const PORT_CANONICAL_CONFIG: &str = "canonical_config";

fn canonical_output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!(
        "aave:origin:config_build:canonical|op:{}",
        op_path.0
    ))
}

fn built_output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("aave:origin:config_build:built|op:{}", op_path.0))
}

fn output_context_key(op_path: &OpPath, port: &str) -> ContextKey {
    ContextKey(format!("{}.out.{port}", op_path.0))
}

/// Returns the context key that stores the built config JSON.
pub fn aave_v3_origin_stack_config_build_built_config_context_key() -> ContextKey {
    ContextKey(format!(
        "{CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_BUILT_CONFIG}"
    ))
}

/// Returns the context key that stores the canonical config artifact id.
pub fn aave_v3_origin_stack_config_build_canonical_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_CANONICAL_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the built config artifact id.
pub fn aave_v3_origin_stack_config_build_built_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_BUILT_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the stable build report.
pub fn aave_v3_origin_stack_config_build_report_context_key() -> ContextKey {
    ContextKey(format!(
        "{CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_BUILD_REPORT}"
    ))
}

/// Thin planner op that turns canonical Aave Origin config into built config artifacts and a
/// stable build report.
#[derive(Clone, Default)]
pub struct AaveV3OriginStackConfigBuildOp;

/// Returns the built-in public `aave_v3_origin_stack_config_build` root op.
pub fn aave_v3_origin_stack_config_build_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(AaveV3OriginStackConfigBuildOp) as DynOperation]
}

impl Operation for AaveV3OriginStackConfigBuildOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(AAVE_V3_ORIGIN_STACK_CONFIG_BUILD_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        super::AAVE_V3_ORIGIN_STACK_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let canonical = decode_aave_v3_origin_stack_canonical_config(op_config).map_err(|err| {
            super::sdk_input_error("invalid_aave_v3_origin_stack_build_config", err.to_string())
        })?;
        let outcome = build_aave_v3_origin_stack_outcome(canonical).map_err(|err| {
            super::sdk_input_error("invalid_aave_v3_origin_stack_build_config", err.to_string())
        })?;

        super::validate_execution_config(
            child_op_path(&op_path, "validate_built")?,
            &outcome.built,
            run_config,
        )?;

        let write_built_sid = leaf_state_id(&op_path, "write_built_config")?;
        let write_canonical_sid = leaf_state_id(&op_path, "write_canonical_artifact_input")?;
        let write_canonical_artifact_sid = leaf_state_id(&op_path, "write_canonical_artifact")?;
        let write_built_artifact_sid = leaf_state_id(&op_path, "write_built_artifact")?;
        let write_report_sid = leaf_state_id(&op_path, "write_build_report")?;

        let built_config_key = output_context_key(&op_path, PORT_BUILT_CONFIG);
        let canonical_artifact_id_key = output_context_key(&op_path, PORT_CANONICAL_ARTIFACT_ID);
        let built_artifact_id_key = output_context_key(&op_path, PORT_BUILT_ARTIFACT_ID);
        let report_key = output_context_key(&op_path, PORT_BUILD_REPORT);
        let canonical_config_key = output_context_key(&op_path, PORT_CANONICAL_CONFIG);

        let states = vec![
            leaf_state_node(
                &op_path,
                "write_canonical_artifact_input",
                Arc::new(WriteJsonValueState {
                    state_id: write_canonical_sid.clone(),
                    output_key: canonical_config_key.clone(),
                    value: serde_json::to_value(&outcome.built.canonical).map_err(|err| {
                        super::sdk_input_error(
                            "invalid_aave_v3_origin_stack_build_config",
                            err.to_string(),
                        )
                    })?,
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_canonical_artifact",
                Arc::new(WriteContextValueArtifactState {
                    state_id: write_canonical_artifact_sid.clone(),
                    input_key: canonical_config_key,
                    fact_key: canonical_output_fact_key(&op_path),
                    output_artifact_id_key: canonical_artifact_id_key,
                    missing_input_code: "missing_aave_v3_origin_canonical_config",
                    missing_input_message:
                        "missing canonical Aave V3 Origin config before artifact publication",
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_built_config",
                Arc::new(WriteJsonValueState {
                    state_id: write_built_sid.clone(),
                    output_key: built_config_key.clone(),
                    value: serde_json::to_value(&outcome.built).map_err(|err| {
                        super::sdk_input_error(
                            "invalid_aave_v3_origin_stack_build_config",
                            err.to_string(),
                        )
                    })?,
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_built_artifact",
                Arc::new(WriteContextValueArtifactState {
                    state_id: write_built_artifact_sid.clone(),
                    input_key: built_config_key.clone(),
                    fact_key: built_output_fact_key(&op_path),
                    output_artifact_id_key: built_artifact_id_key,
                    missing_input_code: "missing_aave_v3_origin_built_config",
                    missing_input_message:
                        "missing built Aave V3 Origin config before artifact publication",
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_build_report",
                Arc::new(WriteJsonValueState {
                    state_id: write_report_sid.clone(),
                    output_key: report_key,
                    value: serde_json::to_value(&outcome.report).map_err(|err| {
                        super::sdk_input_error(
                            "invalid_aave_v3_origin_stack_build_config",
                            err.to_string(),
                        )
                    })?,
                }),
            )?,
        ];

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![
                    PortKey(PORT_BUILT_CONFIG.to_string()),
                    PortKey(PORT_CANONICAL_ARTIFACT_ID.to_string()),
                    PortKey(PORT_BUILT_ARTIFACT_ID.to_string()),
                    PortKey(PORT_BUILD_REPORT.to_string()),
                ],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states,
                edges: vec![
                    DependencyEdge {
                        from: write_canonical_sid,
                        to: write_canonical_artifact_sid.clone(),
                    },
                    DependencyEdge {
                        from: write_built_sid,
                        to: write_built_artifact_sid.clone(),
                    },
                    DependencyEdge {
                        from: write_canonical_artifact_sid,
                        to: write_report_sid.clone(),
                    },
                    DependencyEdge {
                        from: write_built_artifact_sid,
                        to: write_report_sid,
                    },
                ],
            }),
        })
    }
}
