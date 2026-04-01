#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Deploy/configure/validate config-build and execution planner ops.
//!
//! This crate owns the additive public deploy/configure/validate workflow boundaries:
//!
//! - `evm_deploy_configure_validate_config_build`: canonical config -> built config plus explicit
//!   config artifacts
//! - `evm_deploy_configure_validate_execute`: strict built-config execution root
//! - `evm_deploy_configure_validate`: legacy compatibility root that still accepts
//!   canonical-or-built config
//!
//! All three remain thin planners. Typed authored/canonical/built config lives in
//! `mfm-evm-deploy-configure-validate-config`, and runtime execution stays in the shared EVM
//! state crates.
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
use std::sync::Arc;

use mfm_evm_deploy_configure_validate_config::{
    build_deploy_configure_validate_outcome, decode_deploy_configure_validate_built_config,
    decode_deploy_configure_validate_canonical_config, DeployConfigureValidateBuiltConfig,
    DeployConfigureValidateCanonicalConfig, DeployConfigureValidateExecutionConfig,
};
use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{OpId, OpPath, StateId};
use mfm_machine::plan::DependencyEdge;
use mfm_op_evm_write::{EvmConfigureOp, EvmDeployOp, EvmValidateOp};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::{ChildOpLocalId, PortKey};
use mfm_sdk::op::{
    child_op_path, CompositeOpSpec, DynOperation, LeafOpSpec, LeafStateNode, OpInterface,
    Operation, PlannedOp, PlannedOpKind, PortSource, ReExportBinding,
};
use mfm_state_common::errors as op_errors;
use serde::Serialize;
use serde_json::Value;

mod config_build;
pub use config_build::{
    evm_deploy_configure_validate_config_build_built_artifact_id_context_key,
    evm_deploy_configure_validate_config_build_built_config_context_key,
    evm_deploy_configure_validate_config_build_canonical_artifact_id_context_key,
    evm_deploy_configure_validate_config_build_public_ops,
    evm_deploy_configure_validate_config_build_report_context_key,
    EvmDeployConfigureValidateConfigBuildOp, EVM_DEPLOY_CONFIGURE_VALIDATE_CONFIG_BUILD_OP_ID,
};

/// Legacy public root op id that preserves the canonical-or-built compatibility path.
pub const EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID: &str = "evm_deploy_configure_validate";
/// Strict built-config execution root op id used by thin transport adapters.
pub const EVM_DEPLOY_CONFIGURE_VALIDATE_EXECUTE_OP_ID: &str =
    "evm_deploy_configure_validate_execute";
/// Shared version for the public deploy/configure/validate roots.
pub const EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION: &str = "v1";

#[cfg(test)]
const CONFIGURE_EXPORT: &str = "configure_receipts";
#[cfg(test)]
const CONTRACT_ADDRESS_EXPORT: &str = "contract_address";
#[cfg(test)]
const DEPLOY_TX_HASH_EXPORT: &str = "deploy_tx_hash";
const EXECUTE_CHILD_ID: &str = "execute";
const TRACKER_BUILD_CHILD_ID: &str = "build";
#[cfg(test)]
const VALIDATED_EXPORT: &str = "validated";

/// Compatibility alias for the execution payload shape historically accepted by the legacy root.
pub type EvmDeployConfigureValidateOpConfig = DeployConfigureValidateExecutionConfig;

enum EvmDeployConfigureValidateInput {
    Canonical(DeployConfigureValidateCanonicalConfig),
    Built(DeployConfigureValidateBuiltConfig),
}

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn parse_compatibility_input(
    op_config: &Value,
) -> Result<EvmDeployConfigureValidateInput, SdkError> {
    if let Ok(cfg) = decode_deploy_configure_validate_built_config(op_config) {
        return Ok(EvmDeployConfigureValidateInput::Built(cfg));
    }

    decode_deploy_configure_validate_canonical_config(op_config)
        .map(EvmDeployConfigureValidateInput::Canonical)
        .map_err(|err| {
            sdk_input_error(
                "invalid_evm_deploy_configure_validate_execution_config",
                err.to_string(),
            )
        })
}

fn parse_built_config(op_config: &Value) -> Result<DeployConfigureValidateBuiltConfig, SdkError> {
    decode_deploy_configure_validate_built_config(op_config).map_err(|err| {
        sdk_input_error(
            "invalid_evm_deploy_configure_validate_execution_config",
            err.to_string(),
        )
    })
}

fn re_export_binding(export: &PortKey) -> ReExportBinding {
    ReExportBinding {
        export: export.clone(),
        source: PortSource::ChildExport {
            child: ChildOpLocalId(EXECUTE_CHILD_ID.to_string()),
            export: export.clone(),
        },
    }
}

fn serialize_execution_phase<T: Serialize>(
    phase: &T,
    phase_name: &'static str,
) -> Result<Value, SdkError> {
    serde_json::to_value(phase).map_err(|err| {
        sdk_input_error(
            "invalid_evm_deploy_configure_validate_execution_config",
            format!("failed to serialize {phase_name} config: {err}"),
        )
    })
}

fn expand_from_canonical(
    op_path: OpPath,
    canonical: DeployConfigureValidateCanonicalConfig,
    run_config: &RunConfig,
) -> Result<PlannedOp, SdkError> {
    let canonical_json = serde_json::to_value(&canonical).map_err(|err| {
        sdk_input_error(
            "invalid_evm_deploy_configure_validate_execution_config",
            err.to_string(),
        )
    })?;
    let outcome = build_deploy_configure_validate_outcome(canonical).map_err(|err| {
        sdk_input_error(
            "invalid_evm_deploy_configure_validate_execution_config",
            err.to_string(),
        )
    })?;
    let built_json = serde_json::to_value(&outcome.built).map_err(|err| {
        sdk_input_error(
            "invalid_evm_deploy_configure_validate_execution_config",
            err.to_string(),
        )
    })?;
    let execute_path = child_op_path(&op_path, EXECUTE_CHILD_ID)?;
    let (interface, _) = plan_execution_leaf(execute_path, &outcome.built.execution, run_config)?;

    Ok(PlannedOp {
        interface: OpInterface {
            imports: Vec::new(),
            exports: interface.exports.clone(),
        },
        kind: PlannedOpKind::Composite(CompositeOpSpec {
            children: vec![
                mfm_sdk::op::ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(TRACKER_BUILD_CHILD_ID.to_string()),
                    op_id: OpId::must_new(
                        EVM_DEPLOY_CONFIGURE_VALIDATE_CONFIG_BUILD_OP_ID.to_string(),
                    ),
                    op_version: EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION.to_string(),
                    op_config: canonical_json,
                },
                mfm_sdk::op::ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(EXECUTE_CHILD_ID.to_string()),
                    op_id: OpId::must_new(EVM_DEPLOY_CONFIGURE_VALIDATE_EXECUTE_OP_ID.to_string()),
                    op_version: EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION.to_string(),
                    op_config: built_json,
                },
            ],
            bindings: Vec::new(),
            order: vec![mfm_sdk::op::AfterEdge {
                from_child: ChildOpLocalId(TRACKER_BUILD_CHILD_ID.to_string()),
                to_child: ChildOpLocalId(EXECUTE_CHILD_ID.to_string()),
            }],
            re_exports: interface.exports.iter().map(re_export_binding).collect(),
        }),
    })
}

fn plan_execution_leaf(
    op_path: OpPath,
    cfg: &DeployConfigureValidateExecutionConfig,
    run_config: &RunConfig,
) -> Result<(OpInterface, LeafOpSpec), SdkError> {
    let deploy_config = serialize_execution_phase(&cfg.deploy, "deploy")?;
    let configure_config = serialize_execution_phase(&cfg.configure, "configure")?;
    let validate_config = serialize_execution_phase(&cfg.validate, "validate")?;
    let (deploy_interface, deploy_graph) = into_leaf(EvmDeployOp.expand(
        child_op_path(&op_path, "deploy")?,
        &deploy_config,
        run_config,
    )?)?;
    let (configure_interface, configure_graph) = into_leaf(EvmConfigureOp.expand(
        child_op_path(&op_path, "configure")?,
        &configure_config,
        run_config,
    )?)?;
    let (validate_interface, validate_graph) = into_leaf(EvmValidateOp.expand(
        child_op_path(&op_path, "validate")?,
        &validate_config,
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

    Ok((
        OpInterface {
            imports: deploy_interface.imports,
            exports,
        },
        LeafOpSpec { states, edges },
    ))
}

fn expand_execution(
    op_path: OpPath,
    cfg: &DeployConfigureValidateBuiltConfig,
    run_config: &RunConfig,
) -> Result<PlannedOp, SdkError> {
    let (interface, spec) = plan_execution_leaf(op_path, &cfg.execution, run_config)?;
    Ok(PlannedOp {
        interface,
        kind: PlannedOpKind::Leaf(spec),
    })
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

/// Thin planner op that accepts canonical-or-built config and composes the build/execute
/// workflow.
#[derive(Clone, Default)]
pub struct EvmDeployConfigureValidateOp;

/// Thin planner op that executes pre-built deploy/configure/validate config.
#[derive(Clone, Default)]
pub struct EvmDeployConfigureValidateExecuteOp;

/// Returns the built-in public `evm_deploy_configure_validate` root op.
pub fn evm_deploy_configure_validate_root_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(EvmDeployConfigureValidateOp) as DynOperation]
}

/// Returns the built-in public `evm_deploy_configure_validate_execute` root op.
pub fn evm_deploy_configure_validate_execute_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(EvmDeployConfigureValidateExecuteOp) as DynOperation]
}

/// Returns all built-in public deploy/configure/validate root ops.
pub fn evm_deploy_configure_validate_public_ops() -> Vec<DynOperation> {
    let mut ops = evm_deploy_configure_validate_config_build_public_ops();
    ops.extend(evm_deploy_configure_validate_root_public_ops());
    ops.extend(evm_deploy_configure_validate_execute_public_ops());
    ops
}

impl Operation for EvmDeployConfigureValidateOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        match parse_compatibility_input(op_config)? {
            EvmDeployConfigureValidateInput::Canonical(canonical) => {
                expand_from_canonical(op_path, canonical, run_config)
            }
            EvmDeployConfigureValidateInput::Built(cfg) => {
                expand_execution(op_path, &cfg, run_config)
            }
        }
    }
}

impl Operation for EvmDeployConfigureValidateExecuteOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(EVM_DEPLOY_CONFIGURE_VALIDATE_EXECUTE_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg = parse_built_config(op_config)?;
        expand_execution(op_path, &cfg, run_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_evm_deploy_configure_validate_config::{
        build_deploy_configure_validate_outcome, DeployConfigureValidateBuildReport,
    };
    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::ids::{ArtifactId, ContextKey};
    use mfm_machine::runtime::DefaultExecutionEngine;
    use mfm_sdk::unstable::{
        context_value_with_slot_fallback, single_op_pipeline, SdkPlanResolver,
    };
    use mfm_state_common::test_support as op_test_support;

    fn into_leaf_spec(planned: PlannedOp) -> (OpInterface, LeafOpSpec) {
        let interface = planned.interface;
        let spec = match planned.kind {
            PlannedOpKind::Leaf(spec) => spec,
            PlannedOpKind::Composite(_) => panic!("expected leaf planned op"),
        };
        (interface, spec)
    }

    fn into_composite(planned: PlannedOp) -> CompositeOpSpec {
        match planned.kind {
            PlannedOpKind::Composite(spec) => spec,
            PlannedOpKind::Leaf(_) => panic!("expected composite planned op"),
        }
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

    fn built_config() -> DeployConfigureValidateBuiltConfig {
        build_deploy_configure_validate_outcome(
            decode_deploy_configure_validate_canonical_config(&sample_config()).expect("canonical"),
        )
        .expect("build outcome")
        .built
    }

    async fn load_context_snapshot(stores: &Stores, snapshot_id: &ArtifactId) -> serde_json::Value {
        let bytes = stores
            .artifacts
            .get(snapshot_id)
            .await
            .expect("snapshot bytes");
        serde_json::from_slice(&bytes).expect("snapshot json")
    }

    fn read_required_context_value(
        snapshot: &serde_json::Value,
        key: &ContextKey,
    ) -> serde_json::Value {
        context_value_with_slot_fallback(snapshot, key)
            .unwrap_or_else(|| panic!("missing context key `{}`", key.0))
    }

    #[test]
    fn expand_composes_build_and_execute_children_for_canonical_input() {
        let op = EvmDeployConfigureValidateOp;
        let planned = op
            .expand(
                OpPath("evm_deploy_configure_validate.main".to_string()),
                &sample_config(),
                &op_test_support::run_config_live(),
            )
            .expect("expand canonical config");
        let composite = into_composite(planned);

        assert_eq!(composite.children.len(), 2);
        assert!(composite
            .children
            .iter()
            .any(|child| child.child_op_local_id.0 == TRACKER_BUILD_CHILD_ID
                && child.op_id.as_str() == EVM_DEPLOY_CONFIGURE_VALIDATE_CONFIG_BUILD_OP_ID));
        assert!(composite
            .children
            .iter()
            .any(|child| child.child_op_local_id.0 == EXECUTE_CHILD_ID
                && child.op_id.as_str() == EVM_DEPLOY_CONFIGURE_VALIDATE_EXECUTE_OP_ID));
        assert!(composite.order.iter().any(|edge| {
            edge.from_child.0 == TRACKER_BUILD_CHILD_ID && edge.to_child.0 == EXECUTE_CHILD_ID
        }));
        assert!(composite
            .re_exports
            .iter()
            .any(|binding| binding.export.0 == CONTRACT_ADDRESS_EXPORT));
        assert!(composite
            .re_exports
            .iter()
            .any(|binding| binding.export.0 == VALIDATED_EXPORT));
    }

    #[test]
    fn expand_built_input_skips_build_child_and_keeps_leaf_execution() {
        let op = EvmDeployConfigureValidateOp;
        let (_, graph) = into_leaf_spec(
            op.expand(
                OpPath("evm_deploy_configure_validate.main".to_string()),
                &serde_json::to_value(built_config()).expect("built json"),
                &op_test_support::run_config_live(),
            )
            .expect("expand built config"),
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
    fn execute_root_uses_built_leaf_graph() {
        let op = EvmDeployConfigureValidateExecuteOp;
        assert_eq!(
            op.op_version(),
            EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION
        );

        let (io, graph) = into_leaf_spec(
            op.expand(
                OpPath("evm_deploy_configure_validate_execute.main".to_string()),
                &serde_json::to_value(built_config()).expect("built json"),
                &op_test_support::run_config_live(),
            )
            .expect("expand built config"),
        );

        assert_eq!(graph.states.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert!(io.exports.iter().any(|p| p.0 == CONTRACT_ADDRESS_EXPORT));
        assert!(io.exports.iter().any(|p| p.0 == DEPLOY_TX_HASH_EXPORT));
        assert!(io.exports.iter().any(|p| p.0 == CONFIGURE_EXPORT));
        assert!(io.exports.iter().any(|p| p.0 == VALIDATED_EXPORT));
    }

    #[test]
    fn execute_root_rejects_legacy_canonical_config() {
        let op = EvmDeployConfigureValidateExecuteOp;
        let err = op
            .expand(
                OpPath("evm_deploy_configure_validate_execute.main".to_string()),
                &sample_config(),
                &op_test_support::run_config_live(),
            )
            .err()
            .expect("legacy canonical config should not decode for execute root");

        assert_eq!(
            err.info.code.0,
            "invalid_evm_deploy_configure_validate_execution_config"
        );
    }

    #[tokio::test]
    async fn config_build_emits_built_config_artifacts_and_report() {
        let registry =
            op_test_support::registry_with_ops(evm_deploy_configure_validate_public_ops());
        let planner = op_test_support::default_pipeline_planner();
        let pipeline = single_op_pipeline(
            OpId::must_new(EVM_DEPLOY_CONFIGURE_VALIDATE_CONFIG_BUILD_OP_ID.to_string()),
            EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION.to_string(),
            sample_config(),
        )
        .expect("pipeline");
        let stores = op_test_support::in_memory_stores();
        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(DefaultExecutionEngine::new(resolver));

        let run = op_test_support::start_pipeline_with_defaults(
            engine,
            &stores,
            registry,
            planner,
            pipeline,
            op_test_support::run_config_live(),
        )
        .await
        .expect("start");

        assert_eq!(run.phase, RunPhase::Completed);
        let final_snapshot_id = run.final_snapshot_id.expect("final snapshot");
        let context_snapshot = load_context_snapshot(&stores, &final_snapshot_id).await;

        let built_config: DeployConfigureValidateBuiltConfig =
            serde_json::from_value(read_required_context_value(
                &context_snapshot,
                &evm_deploy_configure_validate_config_build_built_config_context_key(),
            ))
            .expect("built config");
        let report: DeployConfigureValidateBuildReport =
            serde_json::from_value(read_required_context_value(
                &context_snapshot,
                &evm_deploy_configure_validate_config_build_report_context_key(),
            ))
            .expect("report");
        let canonical_artifact_id: String = serde_json::from_value(read_required_context_value(
            &context_snapshot,
            &evm_deploy_configure_validate_config_build_canonical_artifact_id_context_key(),
        ))
        .expect("canonical artifact id");
        let built_artifact_id: String = serde_json::from_value(read_required_context_value(
            &context_snapshot,
            &evm_deploy_configure_validate_config_build_built_artifact_id_context_key(),
        ))
        .expect("built artifact id");

        assert_eq!(report.canonical_config_artifact_id, canonical_artifact_id);
        assert_eq!(report.built_config_artifact_id, built_artifact_id);
        assert_eq!(report.machine_id, built_config.canonical.machine_id);
        assert_eq!(
            report.pipeline_version,
            built_config.canonical.pipeline_version
        );
        assert_eq!(report.phase_count, 3);

        let canonical_bytes = stores
            .artifacts
            .get(&ArtifactId(canonical_artifact_id))
            .await
            .expect("canonical artifact");
        let built_bytes = stores
            .artifacts
            .get(&ArtifactId(built_artifact_id))
            .await
            .expect("built artifact");
        let canonical_from_artifact: DeployConfigureValidateCanonicalConfig =
            serde_json::from_slice(&canonical_bytes).expect("canonical json");
        let built_from_artifact: DeployConfigureValidateBuiltConfig =
            serde_json::from_slice(&built_bytes).expect("built json");

        assert_eq!(canonical_from_artifact, built_config.canonical);
        assert_eq!(built_from_artifact, built_config);
    }
}
