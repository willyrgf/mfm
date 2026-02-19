use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use mfm_machine::config::RunConfig;
use mfm_machine::ids::{OpId, OpPath, StateId};
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_state_common::errors as op_errors;
use mfm_op_evm_write::{EvmConfigureOp, EvmDeployOp, EvmValidateOp};
use mfm_sdk::errors::SdkError;
use mfm_sdk::op::{OpIo, Operation};

pub const EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID: &str = "evm_deploy_configure_validate";
pub const EVM_DEPLOY_CONFIGURE_VALIDATE_OP_VERSION: &str = "v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvmDeployConfigureValidateOpConfig {
    pub deploy: serde_json::Value,
    pub configure: serde_json::Value,
    pub validate: serde_json::Value,
}

#[derive(Clone, Default)]
pub struct EvmDeployConfigureValidateOp;

impl Operation for EvmDeployConfigureValidateOp {
    fn op_id(&self) -> OpId {
        OpId(EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        EVM_DEPLOY_CONFIGURE_VALIDATE_OP_VERSION.to_string()
    }

    fn io(&self, op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        let cfg: EvmDeployConfigureValidateOpConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid evm_deploy_configure_validate op_config",
                )
            })?;

        let deploy_io = EvmDeployOp.io(&cfg.deploy)?;
        let configure_io = EvmConfigureOp.io(&cfg.configure)?;
        let validate_io = EvmValidateOp.io(&cfg.validate)?;

        let mut seen_exports: HashSet<String> = HashSet::new();
        let mut exports = Vec::new();
        for export in deploy_io
            .exports
            .iter()
            .chain(configure_io.exports.iter())
            .chain(validate_io.exports.iter())
        {
            if seen_exports.insert(export.0.clone()) {
                exports.push(export.clone());
            }
        }

        // Deploy imports capture the external preconditions for this fixed sequence.
        Ok(OpIo {
            imports: deploy_io.imports,
            exports,
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: EvmDeployConfigureValidateOpConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid evm_deploy_configure_validate op_config",
                )
            })?;

        let deploy_graph = EvmDeployOp.expand(
            OpPath(format!("{}.deploy", op_path.0)),
            &cfg.deploy,
            run_config,
        )?;
        let configure_graph = EvmConfigureOp.expand(
            OpPath(format!("{}.configure", op_path.0)),
            &cfg.configure,
            run_config,
        )?;
        let validate_graph = EvmValidateOp.expand(
            OpPath(format!("{}.validate", op_path.0)),
            &cfg.validate,
            run_config,
        )?;

        let mut states: Vec<StateNode> = Vec::new();
        let mut edges: Vec<DependencyEdge> = Vec::new();

        connect_graphs(&mut edges, &deploy_graph, &configure_graph);
        connect_graphs(&mut edges, &configure_graph, &validate_graph);

        append_graph(&mut states, &mut edges, deploy_graph);
        append_graph(&mut states, &mut edges, configure_graph);
        append_graph(&mut states, &mut edges, validate_graph);

        Ok(StateGraph { states, edges })
    }
}

fn append_graph(states: &mut Vec<StateNode>, edges: &mut Vec<DependencyEdge>, graph: StateGraph) {
    states.extend(graph.states);
    edges.extend(graph.edges);
}

fn connect_graphs(edges: &mut Vec<DependencyEdge>, left: &StateGraph, right: &StateGraph) {
    let left_sinks = sink_state_ids(left);
    let right_sources = source_state_ids(right);

    for from in &left_sinks {
        for to in &right_sources {
            edges.push(DependencyEdge {
                from: from.clone(),
                to: to.clone(),
            });
        }
    }
}

fn source_state_ids(graph: &StateGraph) -> Vec<StateId> {
    let mut incoming: HashSet<String> = HashSet::new();
    for edge in &graph.edges {
        incoming.insert(edge.to.0.clone());
    }

    graph
        .states
        .iter()
        .filter(|node| !incoming.contains(&node.id.0))
        .map(|node| node.id.clone())
        .collect()
}

fn sink_state_ids(graph: &StateGraph) -> Vec<StateId> {
    let mut outgoing: HashSet<String> = HashSet::new();
    for edge in &graph.edges {
        outgoing.insert(edge.from.0.clone());
    }

    graph
        .states
        .iter()
        .filter(|node| !outgoing.contains(&node.id.0))
        .map(|node| node.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_machine::ids::OpPath;
    use mfm_state_common::test_support as op_test_support;

    fn sample_config() -> serde_json::Value {
        serde_json::json!({
            "deploy": {
                "from": "0x000000000000000000000000000000000000dead"
            },
            "configure": {
                "from": "0x000000000000000000000000000000000000dead",
                "calls": [
                    {"function": "noop", "args": []}
                ]
            },
            "validate": {
                "expected_chain_id": 1
            }
        })
    }

    #[test]
    fn expand_composes_deploy_configure_validate_graphs_in_order() {
        let op = EvmDeployConfigureValidateOp;
        let graph = op
            .expand(
                OpPath("evm_deploy_configure_validate.main".to_string()),
                &sample_config(),
                &op_test_support::run_config_live(),
            )
            .expect("expand");

        assert_eq!(graph.states.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert!(graph
            .states
            .iter()
            .any(|s| s.id.0 == "evm_deploy_configure_validate.main.deploy.deploy"));
        assert!(graph
            .states
            .iter()
            .any(|s| s.id.0 == "evm_deploy_configure_validate.main.configure.configure"));
        assert!(graph
            .states
            .iter()
            .any(|s| s.id.0 == "evm_deploy_configure_validate.main.validate.validate"));
    }

    #[test]
    fn io_does_not_surface_internal_contract_address_import() {
        let op = EvmDeployConfigureValidateOp;
        let io = op.io(&sample_config()).expect("io");

        assert!(io.imports.iter().all(|p| p.0 != "contract_address"));
        assert!(io.exports.iter().any(|p| p.0 == "contract_address"));
        assert!(io.exports.iter().any(|p| p.0 == "validated"));
    }
}
