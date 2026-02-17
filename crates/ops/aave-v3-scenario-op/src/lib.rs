use std::sync::Arc;

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{OpId, OpPath, StateId};
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_op_common::errors as op_errors;
use mfm_op_common::states::aave_v3::{
    validate_scenario_config, AaveScenarioConfig, ApproveErc20State, AssertInvariantsState,
    BorrowAssetState, FundWalletState, LoadDeployManifestState, ReadPositionsState,
    ResolveAccountsState, ResolveChainState, SupplyAssetState, WriteSummaryState,
};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

pub const AAVE_V3_SCENARIO_OP_ID: &str = "aave_v3_scenario";
pub const AAVE_V3_SCENARIO_OP_VERSION: &str = "v1";

#[derive(Clone, Default)]
pub struct AaveV3ScenarioOp;

impl Operation for AaveV3ScenarioOp {
    fn op_id(&self) -> OpId {
        OpId(AAVE_V3_SCENARIO_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        AAVE_V3_SCENARIO_OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: AaveScenarioConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid aave_v3_scenario op_config")
        })?;
        validate_scenario_config(&cfg).map_err(|msg| {
            op_errors::sdk_error("invalid_op_config", ErrorCategory::ParsingInput, false, msg)
        })?;

        Ok(OpIo {
            imports: vec![PortKey(cfg.deploy_manifest_port)],
            exports: vec![
                PortKey(cfg.scenario_report_export_key),
                PortKey(cfg.scenario_report_artifact_key),
            ],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: AaveScenarioConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid aave_v3_scenario op_config")
        })?;
        validate_scenario_config(&cfg).map_err(|msg| {
            op_errors::sdk_error("invalid_op_config", ErrorCategory::ParsingInput, false, msg)
        })?;

        let load_id = StateId(format!("{}.load_deploy_manifest", op_path.0));
        let chain_id = StateId(format!("{}.resolve_chain", op_path.0));
        let accounts_id = StateId(format!("{}.resolve_accounts", op_path.0));
        let fund_id = StateId(format!("{}.fund_wallet", op_path.0));
        let approve_id = StateId(format!("{}.approve_erc20", op_path.0));
        let supply_id = StateId(format!("{}.supply_asset", op_path.0));
        let borrow_id = StateId(format!("{}.borrow_asset", op_path.0));
        let read_id = StateId(format!("{}.read_positions", op_path.0));
        let assert_id = StateId(format!("{}.assert_invariants", op_path.0));
        let write_id = StateId(format!("{}.write_summary", op_path.0));

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
                    id: chain_id.clone(),
                    state: Arc::new(ResolveChainState {
                        state_id: chain_id.clone(),
                    }),
                },
                StateNode {
                    id: accounts_id.clone(),
                    state: Arc::new(ResolveAccountsState {
                        state_id: accounts_id.clone(),
                        cfg: cfg.clone(),
                    }),
                },
                StateNode {
                    id: fund_id.clone(),
                    state: Arc::new(FundWalletState {
                        state_id: fund_id.clone(),
                        cfg: cfg.clone(),
                    }),
                },
                StateNode {
                    id: approve_id.clone(),
                    state: Arc::new(ApproveErc20State {
                        state_id: approve_id.clone(),
                        cfg: cfg.clone(),
                    }),
                },
                StateNode {
                    id: supply_id.clone(),
                    state: Arc::new(SupplyAssetState {
                        state_id: supply_id.clone(),
                        cfg: cfg.clone(),
                    }),
                },
                StateNode {
                    id: borrow_id.clone(),
                    state: Arc::new(BorrowAssetState {
                        state_id: borrow_id.clone(),
                        cfg: cfg.clone(),
                    }),
                },
                StateNode {
                    id: read_id.clone(),
                    state: Arc::new(ReadPositionsState {
                        state_id: read_id.clone(),
                    }),
                },
                StateNode {
                    id: assert_id.clone(),
                    state: Arc::new(AssertInvariantsState {
                        state_id: assert_id.clone(),
                        cfg: cfg.clone(),
                    }),
                },
                StateNode {
                    id: write_id.clone(),
                    state: Arc::new(WriteSummaryState {
                        state_id: write_id.clone(),
                        cfg,
                        op_path: op_path.0,
                    }),
                },
            ],
            edges: vec![
                DependencyEdge {
                    from: load_id.clone(),
                    to: chain_id.clone(),
                },
                DependencyEdge {
                    from: chain_id.clone(),
                    to: accounts_id.clone(),
                },
                DependencyEdge {
                    from: accounts_id.clone(),
                    to: fund_id.clone(),
                },
                DependencyEdge {
                    from: fund_id.clone(),
                    to: approve_id.clone(),
                },
                DependencyEdge {
                    from: approve_id.clone(),
                    to: supply_id.clone(),
                },
                DependencyEdge {
                    from: supply_id.clone(),
                    to: borrow_id.clone(),
                },
                DependencyEdge {
                    from: borrow_id.clone(),
                    to: read_id.clone(),
                },
                DependencyEdge {
                    from: read_id.clone(),
                    to: assert_id.clone(),
                },
                DependencyEdge {
                    from: assert_id,
                    to: write_id,
                },
            ],
        })
    }
}
