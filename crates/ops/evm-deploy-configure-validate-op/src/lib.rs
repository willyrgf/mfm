#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Thin planner op that composes deploy, configure, and validate EVM sub-ops.
//!
//! This op preserves the thin-layer contract by delegating executable behavior to the shared
//! EVM state crates. Its responsibility is limited to validating the composite config, expanding
//! the three sub-ops, and connecting their graphs in sequence.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_deploy_configure_validate::EvmDeployConfigureValidateOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = EvmDeployConfigureValidateOp;
//! assert_eq!(op.op_id().as_str(), "evm_deploy_configure_validate");
//! ```

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{OpId, OpPath, StateId};
use mfm_machine::plan::DependencyEdge;
use mfm_op_evm_write::{EvmConfigureOp, EvmDeployOp, EvmValidateOp};
use mfm_sdk::errors::SdkError;
use mfm_sdk::op::{
    child_op_path, CompositeOpSpec, LeafOpSpec, LeafStateNode, OpInterface, Operation, PlannedOp,
    PlannedOpKind,
};
use mfm_state_common::errors as op_errors;

/// Stable operation identifier for the composite deploy-configure-validate workflow.
pub const EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID: &str = "evm_deploy_configure_validate";
/// Stable operation version for the composite deploy-configure-validate workflow.
pub const EVM_DEPLOY_CONFIGURE_VALIDATE_OP_VERSION: &str = "v1";

/// Config for the composite deploy-configure-validate workflow.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvmDeployConfigureValidateOpConfig {
    /// JSON config forwarded to the deploy sub-op.
    pub deploy: serde_json::Value,
    /// JSON config forwarded to the configure sub-op.
    pub configure: serde_json::Value,
    /// JSON config forwarded to the validate sub-op.
    pub validate: serde_json::Value,
}

/// Planner op that expands deploy, configure, and validate sub-graphs in order.
#[derive(Clone, Default)]
pub struct EvmDeployConfigureValidateOp;

impl Operation for EvmDeployConfigureValidateOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        EVM_DEPLOY_CONFIGURE_VALIDATE_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: EvmDeployConfigureValidateOpConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid evm_deploy_configure_validate op_config",
                )
            })?;

        let (deploy_interface, deploy_graph) = into_leaf(EvmDeployOp.expand(
            child_op_path(&op_path, "deploy")?,
            &cfg.deploy,
            run_config,
        )?)?;
        let (configure_interface, configure_graph) = into_leaf(EvmConfigureOp.expand(
            child_op_path(&op_path, "configure")?,
            &cfg.configure,
            run_config,
        )?)?;
        let (validate_interface, validate_graph) = into_leaf(EvmValidateOp.expand(
            child_op_path(&op_path, "validate")?,
            &cfg.validate,
            run_config,
        )?)?;

        let mut states: Vec<LeafStateNode> = Vec::new();
        let mut edges: Vec<DependencyEdge> = Vec::new();

        connect_graphs(&mut edges, &deploy_graph, &configure_graph);
        connect_graphs(&mut edges, &configure_graph, &validate_graph);

        append_graph(&mut states, &mut edges, deploy_graph);
        append_graph(&mut states, &mut edges, configure_graph);
        append_graph(&mut states, &mut edges, validate_graph);

        let mut seen_exports: HashSet<String> = HashSet::new();
        let mut exports = Vec::new();
        for export in deploy_interface
            .exports
            .iter()
            .chain(configure_interface.exports.iter())
            .chain(validate_interface.exports.iter())
        {
            if seen_exports.insert(export.0.clone()) {
                exports.push(export.clone());
            }
        }

        Ok(PlannedOp {
            interface: OpInterface {
                imports: deploy_interface.imports,
                exports,
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec { states, edges }),
        })
    }
}

fn into_leaf(planned: PlannedOp) -> Result<(OpInterface, LeafOpSpec), SdkError> {
    let interface = planned.interface;
    match planned.kind {
        PlannedOpKind::Leaf(spec) => Ok((interface, spec)),
        PlannedOpKind::Composite(CompositeOpSpec { .. }) => Err(op_errors::sdk_error(
            "unsupported_child_composite_op",
            ErrorCategory::ParsingInput,
            false,
            "child composite planned ops are not supported in this compatibility leaf planner",
        )),
    }
}

fn append_graph(
    states: &mut Vec<LeafStateNode>,
    edges: &mut Vec<DependencyEdge>,
    graph: LeafOpSpec,
) {
    states.extend(graph.states);
    edges.extend(graph.edges);
}

fn connect_graphs(edges: &mut Vec<DependencyEdge>, left: &LeafOpSpec, right: &LeafOpSpec) {
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

fn source_state_ids(graph: &LeafOpSpec) -> Vec<StateId> {
    let mut incoming: HashSet<String> = HashSet::new();
    for edge in &graph.edges {
        incoming.insert(edge.to.as_str().to_string());
    }

    graph
        .states
        .iter()
        .filter(|node| !incoming.contains(node.state_id.as_str()))
        .map(|node| node.state_id.clone())
        .collect()
}

fn sink_state_ids(graph: &LeafOpSpec) -> Vec<StateId> {
    let mut outgoing: HashSet<String> = HashSet::new();
    for edge in &graph.edges {
        outgoing.insert(edge.from.as_str().to_string());
    }

    graph
        .states
        .iter()
        .filter(|node| !outgoing.contains(node.state_id.as_str()))
        .map(|node| node.state_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_machine::ids::OpPath;
    use mfm_state_common::test_support as op_test_support;

    fn into_leaf_spec(planned: PlannedOp) -> (OpInterface, LeafOpSpec) {
        let interface = planned.interface;
        let spec = match planned.kind {
            PlannedOpKind::Leaf(spec) => spec,
            PlannedOpKind::Composite(_) => panic!("expected leaf planned op"),
        };
        (interface, spec)
    }

    fn sample_config() -> serde_json::Value {
        serde_json::json!({
            "deploy": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead"
            },
            "configure": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "calls": [
                    {"function": "noop", "args": []}
                ]
            },
            "validate": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1
            }
        })
    }

    #[test]
    fn expand_composes_deploy_configure_validate_graphs_in_order() {
        let op = EvmDeployConfigureValidateOp;
        let (_, graph) = into_leaf_spec(
            op.expand(
                OpPath("evm_deploy_configure_validate.main".to_string()),
                &sample_config(),
                &op_test_support::run_config_live(),
            )
            .expect("expand"),
        );

        assert_eq!(graph.states.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert!(graph
            .states
            .iter()
            .any(|s| s.state_id.as_str() == "evm_deploy_configure_validate.main.deploy__deploy"));
        assert!(graph
            .states
            .iter()
            .any(|s| s.state_id.as_str()
                == "evm_deploy_configure_validate.main.configure__configure"));
        assert!(graph.states.iter().any(
            |s| s.state_id.as_str() == "evm_deploy_configure_validate.main.validate__validate"
        ));
    }

    #[test]
    fn expand_does_not_surface_internal_contract_address_import() {
        let op = EvmDeployConfigureValidateOp;
        let (io, _) = into_leaf_spec(
            op.expand(
                OpPath("evm_deploy_configure_validate.main".to_string()),
                &sample_config(),
                &op_test_support::run_config_live(),
            )
            .expect("expand"),
        );

        assert!(io.imports.iter().all(|p| p.0 != "contract_address"));
        assert!(io.exports.iter().any(|p| p.0 == "contract_address"));
        assert!(io.exports.iter().any(|p| p.0 == "validated"));
    }
}
