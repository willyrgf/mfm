use std::sync::Arc;

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{OpId, OpPath, StateId};
use mfm_machine::plan::{StateGraph, StateNode};
use mfm_op_aave_v3_common::states::AdaptOriginDeployOutputState;
use mfm_op_common::errors as op_errors;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};
use serde::Deserialize;

pub const AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_ID: &str = "aave_v3_origin_adapt_deploy";
pub const AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_VERSION: &str = "v1";

fn default_origin_deploy_port() -> String {
    "result".to_string()
}

fn default_deploy_manifest_export_key() -> String {
    "deploy_manifest".to_string()
}

#[derive(Clone, Debug, Deserialize)]
pub struct AaveV3OriginAdaptDeployConfig {
    #[serde(default = "default_origin_deploy_port")]
    pub origin_deploy_port: String,
    #[serde(default = "default_deploy_manifest_export_key")]
    pub deploy_manifest_export_key: String,
}

fn validate_config(cfg: &AaveV3OriginAdaptDeployConfig) -> Result<(), String> {
    if cfg.origin_deploy_port.trim().is_empty() {
        return Err("origin_deploy_port must be non-empty".to_string());
    }
    if cfg.deploy_manifest_export_key.trim().is_empty() {
        return Err("deploy_manifest_export_key must be non-empty".to_string());
    }
    Ok(())
}

#[derive(Clone, Default)]
pub struct AaveV3OriginAdaptDeployOp;

impl Operation for AaveV3OriginAdaptDeployOp {
    fn op_id(&self) -> OpId {
        OpId(AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: AaveV3OriginAdaptDeployConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid aave_v3_origin_adapt_deploy op_config",
                )
            })?;
        validate_config(&cfg).map_err(|msg| {
            op_errors::sdk_error("invalid_op_config", ErrorCategory::ParsingInput, false, msg)
        })?;

        Ok(OpIo {
            imports: vec![PortKey(cfg.origin_deploy_port)],
            exports: vec![PortKey(cfg.deploy_manifest_export_key)],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: AaveV3OriginAdaptDeployConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid aave_v3_origin_adapt_deploy op_config",
                )
            })?;
        validate_config(&cfg).map_err(|msg| {
            op_errors::sdk_error("invalid_op_config", ErrorCategory::ParsingInput, false, msg)
        })?;

        let state_id = StateId(format!("{}.adapt_origin_deploy", op_path.0));
        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id.clone(),
                state: Arc::new(AdaptOriginDeployOutputState {
                    state_id,
                    origin_deploy_port: cfg.origin_deploy_port,
                    deploy_manifest_export_key: cfg.deploy_manifest_export_key,
                }),
            }],
            edges: Vec::new(),
        })
    }
}
