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
//! - `evm_deploy_configure_validate`: canonical public root that composes build then execute
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

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use mfm_evm_deploy_configure_validate_config::{
    build_deploy_configure_validate_outcome, decode_deploy_configure_validate_built_config,
    decode_deploy_configure_validate_canonical_config, DeployConfigureValidateBuiltConfig,
    DeployConfigureValidateCanonicalConfig, DeployConfigureValidateExecutionConfig,
};
use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::OpId;
use mfm_machine::ids::OpPath;
use mfm_op_evm_write::{EvmConfigureOp, EvmDeployOp, EvmValidateOp};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::{ChildOpLocalId, PortKey};
use mfm_sdk::op::{
    child_op_path, AfterEdge, ChildOpInstance, CompositeOpSpec, DynOperation, ImportBinding,
    OpInterface, Operation, PlannedOp, PlannedOpKind, PlannerPayloadConfigSource, PortSource,
    ReExportBinding,
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

/// Canonical public root op id that composes build then execute.
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
const CONFIGURE_CHILD_ID: &str = "configure";
const DEPLOY_CHILD_ID: &str = "deploy";
const EXECUTE_CHILD_ID: &str = "e";
const TRACKER_BUILD_CHILD_ID: &str = "b";
#[cfg(test)]
const VALIDATED_EXPORT: &str = "validated";
const VALIDATE_CHILD_ID: &str = "validate";

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
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

fn child_import_binding(import: &PortKey) -> ImportBinding {
    ImportBinding {
        to_child: ChildOpLocalId(EXECUTE_CHILD_ID.to_string()),
        import: import.clone(),
        source: PortSource::ParentImport(import.clone()),
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

struct ExecutionChildPlan {
    child_op_local_id: ChildOpLocalId,
    op_id: OpId,
    op_version: String,
    op_config: Value,
    interface: OpInterface,
}

fn plan_execution_child(
    op_path: &OpPath,
    child_id: &'static str,
    op_id: OpId,
    op_version: String,
    op_config: Value,
    run_config: &RunConfig,
    op: impl Operation,
) -> Result<ExecutionChildPlan, SdkError> {
    let interface = op
        .expand(child_op_path(op_path, child_id)?, &op_config, run_config)?
        .interface;

    Ok(ExecutionChildPlan {
        child_op_local_id: ChildOpLocalId(child_id.to_string()),
        op_id,
        op_version,
        op_config,
        interface,
    })
}

fn plan_execution_children(
    op_path: &OpPath,
    cfg: &DeployConfigureValidateExecutionConfig,
    run_config: &RunConfig,
) -> Result<Vec<ExecutionChildPlan>, SdkError> {
    let deploy_config = serialize_execution_phase(&cfg.deploy, "deploy")?;
    let configure_config = serialize_execution_phase(&cfg.configure, "configure")?;
    let validate_config = serialize_execution_phase(&cfg.validate, "validate")?;

    let deploy_op = EvmDeployOp;
    let configure_op = EvmConfigureOp;
    let validate_op = EvmValidateOp;

    Ok(vec![
        plan_execution_child(
            op_path,
            DEPLOY_CHILD_ID,
            deploy_op.op_id(),
            deploy_op.op_version(),
            deploy_config,
            run_config,
            deploy_op,
        )?,
        plan_execution_child(
            op_path,
            CONFIGURE_CHILD_ID,
            configure_op.op_id(),
            configure_op.op_version(),
            configure_config,
            run_config,
            configure_op,
        )?,
        plan_execution_child(
            op_path,
            VALIDATE_CHILD_ID,
            validate_op.op_id(),
            validate_op.op_version(),
            validate_config,
            run_config,
            validate_op,
        )?,
    ])
}

fn execution_composite(
    op_path: &OpPath,
    cfg: &DeployConfigureValidateExecutionConfig,
    run_config: &RunConfig,
) -> Result<(OpInterface, CompositeOpSpec), SdkError> {
    let children = plan_execution_children(op_path, cfg, run_config)?;
    let mut export_producers: HashMap<String, ChildOpLocalId> = HashMap::new();
    let mut seen_parent_imports: HashSet<String> = HashSet::new();
    let mut parent_imports = Vec::new();
    let mut bindings = Vec::new();

    for child in &children {
        for import in &child.interface.imports {
            let source = match export_producers.get(&import.0) {
                Some(source_child) => PortSource::ChildExport {
                    child: source_child.clone(),
                    export: import.clone(),
                },
                None => {
                    if seen_parent_imports.insert(import.0.clone()) {
                        parent_imports.push(import.clone());
                    }
                    PortSource::ParentImport(import.clone())
                }
            };
            bindings.push(ImportBinding {
                to_child: child.child_op_local_id.clone(),
                import: import.clone(),
                source,
            });
        }

        for export in &child.interface.exports {
            export_producers
                .entry(export.0.clone())
                .or_insert_with(|| child.child_op_local_id.clone());
        }
    }

    let mut seen_parent_exports: HashSet<String> = HashSet::new();
    let mut parent_exports = Vec::new();
    let mut re_exports = Vec::new();
    for child in &children {
        for export in &child.interface.exports {
            if seen_parent_exports.insert(export.0.clone()) {
                parent_exports.push(export.clone());
                re_exports.push(ReExportBinding {
                    export: export.clone(),
                    source: PortSource::ChildExport {
                        child: child.child_op_local_id.clone(),
                        export: export.clone(),
                    },
                });
            }
        }
    }

    let child_instances = children
        .iter()
        .map(|child| ChildOpInstance {
            child_op_local_id: child.child_op_local_id.clone(),
            op_id: child.op_id.clone(),
            op_version: child.op_version.clone(),
            op_config: child.op_config.clone(),
            op_config_from_planner_payload: None,
        })
        .collect();
    let order = children
        .windows(2)
        .map(|pair| AfterEdge {
            from_child: pair[0].child_op_local_id.clone(),
            to_child: pair[1].child_op_local_id.clone(),
        })
        .collect();

    Ok((
        OpInterface {
            imports: parent_imports,
            exports: parent_exports,
        },
        CompositeOpSpec {
            children: child_instances,
            bindings,
            order,
            re_exports,
        },
    ))
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

    let execute_interface = build_deploy_configure_validate_outcome(canonical.clone())
        .map_err(|err| {
            sdk_input_error(
                "invalid_evm_deploy_configure_validate_execution_config",
                err.to_string(),
            )
        })
        .and_then(|outcome| {
            expand_execution(
                child_op_path(&op_path, EXECUTE_CHILD_ID)?,
                &outcome.built,
                run_config,
            )
            .map(|planned| planned.interface)
        })?;

    Ok(PlannedOp {
        interface: execute_interface.clone(),
        kind: PlannedOpKind::Composite(CompositeOpSpec {
            children: vec![
                mfm_sdk::op::ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(TRACKER_BUILD_CHILD_ID.to_string()),
                    op_id: OpId::must_new(
                        EVM_DEPLOY_CONFIGURE_VALIDATE_CONFIG_BUILD_OP_ID.to_string(),
                    ),
                    op_version: EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION.to_string(),
                    op_config: canonical_json,
                    op_config_from_planner_payload: None,
                },
                mfm_sdk::op::ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(EXECUTE_CHILD_ID.to_string()),
                    op_id: OpId::must_new(EVM_DEPLOY_CONFIGURE_VALIDATE_EXECUTE_OP_ID.to_string()),
                    op_version: EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION.to_string(),
                    op_config: serde_json::json!({}),
                    op_config_from_planner_payload: Some(PlannerPayloadConfigSource {
                        child: ChildOpLocalId(TRACKER_BUILD_CHILD_ID.to_string()),
                        pointer: "/built_config".to_string(),
                    }),
                },
            ],
            bindings: execute_interface
                .imports
                .iter()
                .map(child_import_binding)
                .collect(),
            order: vec![mfm_sdk::op::AfterEdge {
                from_child: ChildOpLocalId(TRACKER_BUILD_CHILD_ID.to_string()),
                to_child: ChildOpLocalId(EXECUTE_CHILD_ID.to_string()),
            }],
            re_exports: execute_interface
                .exports
                .iter()
                .map(re_export_binding)
                .collect(),
        }),
    })
}

fn expand_execution(
    op_path: OpPath,
    cfg: &DeployConfigureValidateBuiltConfig,
    run_config: &RunConfig,
) -> Result<PlannedOp, SdkError> {
    let (interface, spec) = execution_composite(&op_path, &cfg.execution, run_config)?;
    Ok(PlannedOp {
        interface,
        kind: PlannedOpKind::Composite(spec),
    })
}

/// Thin planner op that accepts canonical config and composes the build/execute workflow.
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
        let canonical =
            decode_deploy_configure_validate_canonical_config(op_config).map_err(|err| {
                sdk_input_error(
                    "invalid_evm_deploy_configure_validate_execution_config",
                    err.to_string(),
                )
            })?;
        expand_from_canonical(op_path, canonical, run_config)
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
        assert!(planned
            .interface
            .imports
            .iter()
            .any(|port| port.0 == "contract_artifact"));
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
        assert!(composite.bindings.iter().any(|binding| {
            binding.to_child.0 == EXECUTE_CHILD_ID
                && binding.import.0 == "contract_artifact"
                && matches!(binding.source, PortSource::ParentImport(ref import) if import.0 == "contract_artifact")
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
    fn root_rejects_built_input() {
        let op = EvmDeployConfigureValidateOp;
        let err = op
            .expand(
                OpPath("evm_deploy_configure_validate.main".to_string()),
                &serde_json::to_value(built_config()).expect("built json"),
                &op_test_support::run_config_live(),
            )
            .err()
            .expect("built config must not decode for public root");

        assert_eq!(
            err.info.code.0,
            "invalid_evm_deploy_configure_validate_execution_config"
        );
    }

    #[test]
    fn execute_root_uses_built_execute_graph() {
        let op = EvmDeployConfigureValidateExecuteOp;
        assert_eq!(
            op.op_version(),
            EVM_DEPLOY_CONFIGURE_VALIDATE_PUBLIC_OP_VERSION
        );

        let planned = op
            .expand(
                OpPath("evm_deploy_configure_validate_execute.main".to_string()),
                &serde_json::to_value(built_config()).expect("built json"),
                &op_test_support::run_config_live(),
            )
            .expect("expand built config");
        let io = planned.interface.clone();
        let graph = into_composite(planned);

        assert_eq!(graph.children.len(), 3);
        assert_eq!(graph.order.len(), 2);
        assert!(graph
            .bindings
            .iter()
            .any(|binding| binding.to_child.0 == DEPLOY_CHILD_ID
                && binding.import.0 == "contract_artifact"
                && matches!(binding.source, PortSource::ParentImport(ref import) if import.0 == "contract_artifact")));
        assert!(graph
            .bindings
            .iter()
            .any(|binding| binding.to_child.0 == CONFIGURE_CHILD_ID
                && binding.import.0 == "contract_address"
                && matches!(binding.source, PortSource::ChildExport { ref child, ref export } if child.0 == DEPLOY_CHILD_ID && export.0 == "contract_address")));
        assert!(graph
            .bindings
            .iter()
            .any(|binding| binding.to_child.0 == VALIDATE_CHILD_ID
                && binding.import.0 == "contract_address"
                && matches!(binding.source, PortSource::ChildExport { ref child, ref export } if child.0 == DEPLOY_CHILD_ID && export.0 == "contract_address")));
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
            .get(&ArtifactId::must_new(canonical_artifact_id.as_str()))
            .await
            .expect("canonical artifact");
        let built_bytes = stores
            .artifacts
            .get(&ArtifactId::must_new(built_artifact_id.as_str()))
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
