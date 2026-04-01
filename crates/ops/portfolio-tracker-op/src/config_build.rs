use std::sync::Arc;

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_machine::plan::DependencyEdge;
use mfm_portfolio_config::{build_portfolio_snapshot_outcome, PortfolioSnapshotCanonicalConfig};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    leaf_state_id, leaf_state_node, DynOperation, LeafOpSpec, OpInterface, Operation, PlannedOp,
    PlannedOpKind,
};
use mfm_state_common::errors as op_errors;
use mfm_state_common::states::publish::{WriteContextValueArtifactState, WriteJsonValueState};
use serde_json::Value;

/// Public root op id for the canonical-to-built portfolio config workflow.
pub const PORTFOLIO_CONFIG_BUILD_OP_ID: &str = "portfolio_config_build";
const PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH: &str = "portfolio_config_build.main";
const PORT_CONFIG_BUILT: &str = "built_config";
const PORT_CANONICAL_ARTIFACT_ID: &str = "canonical_config_artifact_id";
const PORT_BUILT_ARTIFACT_ID: &str = "built_config_artifact_id";
const PORT_BUILD_REPORT: &str = "report";
const PORT_CANONICAL_CONFIG: &str = "canonical_config";

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn canonical_output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:config_build:canonical|op:{}", op_path.0))
}

fn built_output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:config_build:built|op:{}", op_path.0))
}

fn output_context_key(op_path: &OpPath, port: &str) -> ContextKey {
    ContextKey(format!("{}.out.{port}", op_path.0))
}

/// Returns the context key that stores the built portfolio config JSON.
pub fn portfolio_config_build_built_config_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_CONFIG_BUILT}"
    ))
}

/// Returns the context key that stores the canonical config artifact id.
pub fn portfolio_config_build_canonical_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_CANONICAL_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the built config artifact id.
pub fn portfolio_config_build_built_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_BUILT_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the stable build report.
pub fn portfolio_config_build_report_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_BUILD_REPORT}"
    ))
}

/// Thin planner op that turns canonical portfolio config into built config artifacts and a stable
/// build report.
#[derive(Clone, Default)]
pub struct PortfolioConfigBuildOp;

/// Returns the built-in public `portfolio_config_build` root op.
pub fn portfolio_config_build_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(PortfolioConfigBuildOp) as DynOperation]
}

impl Operation for PortfolioConfigBuildOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PORTFOLIO_CONFIG_BUILD_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        super::PORTFOLIO_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let canonical = serde_json::from_value::<PortfolioSnapshotCanonicalConfig>(
            op_config.clone(),
        )
        .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;
        let outcome = build_portfolio_snapshot_outcome(canonical)
            .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;

        let write_built_sid = leaf_state_id(&op_path, "write_built_config")?;
        let write_canonical_sid = leaf_state_id(&op_path, "write_canonical_artifact_input")?;
        let write_canonical_artifact_sid = leaf_state_id(&op_path, "write_canonical_artifact")?;
        let write_built_artifact_sid = leaf_state_id(&op_path, "write_built_artifact")?;
        let write_report_sid = leaf_state_id(&op_path, "write_build_report")?;

        let built_config_key = output_context_key(&op_path, PORT_CONFIG_BUILT);
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
                        sdk_input_error("invalid_portfolio_build_config", err.to_string())
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
                    missing_input_code: "missing_portfolio_canonical_config",
                    missing_input_message:
                        "missing canonical portfolio config before artifact publication",
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_built_config",
                Arc::new(WriteJsonValueState {
                    state_id: write_built_sid.clone(),
                    output_key: built_config_key.clone(),
                    value: serde_json::to_value(&outcome.built).map_err(|err| {
                        sdk_input_error("invalid_portfolio_build_config", err.to_string())
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
                    missing_input_code: "missing_portfolio_built_config",
                    missing_input_message:
                        "missing built portfolio config before artifact publication",
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_build_report",
                Arc::new(WriteJsonValueState {
                    state_id: write_report_sid.clone(),
                    output_key: report_key,
                    value: serde_json::to_value(&outcome.report).map_err(|err| {
                        sdk_input_error("invalid_portfolio_build_config", err.to_string())
                    })?,
                }),
            )?,
        ];

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![
                    PortKey(PORT_CONFIG_BUILT.to_string()),
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

    fn planner_payload(
        &self,
        _op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<Option<Value>, SdkError> {
        let canonical = serde_json::from_value::<PortfolioSnapshotCanonicalConfig>(
            op_config.clone(),
        )
        .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;
        let outcome = build_portfolio_snapshot_outcome(canonical)
            .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;
        Ok(Some(serde_json::json!({
            "built_config": outcome.built
        })))
    }
}
