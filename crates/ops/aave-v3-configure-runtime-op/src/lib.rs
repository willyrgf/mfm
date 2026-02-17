use std::sync::Arc;

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{OpId, OpPath, StateId};
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_op_common::errors as op_errors;
use mfm_op_common::states::aave_v3::{
    validate_configure_runtime_config, AaveConfigureRuntimeConfig, CollectConfigOutputsState,
    ConfigureRuntimeCallState, LoadDeployManifestState, WaitForConfigReceiptState,
    WriteConfigReportState,
};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

pub const AAVE_V3_CONFIGURE_RUNTIME_OP_ID: &str = "aave_v3_configure_runtime";
pub const AAVE_V3_CONFIGURE_RUNTIME_OP_VERSION: &str = "v1";

#[derive(Clone, Default)]
pub struct AaveV3ConfigureRuntimeOp;

impl Operation for AaveV3ConfigureRuntimeOp {
    fn op_id(&self) -> OpId {
        OpId(AAVE_V3_CONFIGURE_RUNTIME_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        AAVE_V3_CONFIGURE_RUNTIME_OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: AaveConfigureRuntimeConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid aave_v3_configure_runtime op_config",
                )
            })?;
        validate_configure_runtime_config(&cfg).map_err(|msg| {
            op_errors::sdk_error("invalid_op_config", ErrorCategory::ParsingInput, false, msg)
        })?;

        Ok(OpIo {
            imports: vec![PortKey(cfg.deploy_manifest_port)],
            exports: vec![PortKey(cfg.config_report_export_key)],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: AaveConfigureRuntimeConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid aave_v3_configure_runtime op_config",
                )
            })?;
        validate_configure_runtime_config(&cfg).map_err(|msg| {
            op_errors::sdk_error("invalid_op_config", ErrorCategory::ParsingInput, false, msg)
        })?;

        let load_id = StateId(format!("{}.load_deploy_manifest", op_path.0));
        let configure_id = StateId(format!("{}.configure_runtime_call", op_path.0));
        let wait_id = StateId(format!("{}.wait_for_config_receipt", op_path.0));
        let collect_id = StateId(format!("{}.collect_config_outputs", op_path.0));
        let write_id = StateId(format!("{}.write_config_report", op_path.0));

        Ok(StateGraph {
            states: vec![
                StateNode {
                    id: load_id.clone(),
                    state: Arc::new(LoadDeployManifestState {
                        state_id: load_id.clone(),
                        deploy_manifest_port: cfg.deploy_manifest_port.clone(),
                    }),
                },
                StateNode {
                    id: configure_id.clone(),
                    state: Arc::new(ConfigureRuntimeCallState {
                        state_id: configure_id.clone(),
                        cfg: cfg.clone(),
                    }),
                },
                StateNode {
                    id: wait_id.clone(),
                    state: Arc::new(WaitForConfigReceiptState {
                        state_id: wait_id.clone(),
                        poll_interval_ms: cfg.poll_interval_ms,
                        max_receipt_polls: cfg.max_receipt_polls,
                    }),
                },
                StateNode {
                    id: collect_id.clone(),
                    state: Arc::new(CollectConfigOutputsState {
                        state_id: collect_id.clone(),
                    }),
                },
                StateNode {
                    id: write_id.clone(),
                    state: Arc::new(WriteConfigReportState {
                        state_id: write_id.clone(),
                        config_report_export_key: cfg.config_report_export_key.clone(),
                    }),
                },
            ],
            edges: vec![
                DependencyEdge {
                    from: load_id.clone(),
                    to: configure_id.clone(),
                },
                DependencyEdge {
                    from: configure_id.clone(),
                    to: wait_id.clone(),
                },
                DependencyEdge {
                    from: wait_id.clone(),
                    to: collect_id.clone(),
                },
                DependencyEdge {
                    from: collect_id,
                    to: write_id,
                },
            ],
        })
    }
}
