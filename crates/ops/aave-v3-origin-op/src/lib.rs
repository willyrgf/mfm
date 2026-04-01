#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Aave V3 Origin stack config-build and execution planner ops.
//!
//! This crate owns the additive public Aave Origin workflow boundaries:
//!
//! - `aave_v3_origin_stack_config_build`: canonical config -> built config plus explicit config artifacts
//! - `aave_v3_origin_stack_execute`: strict built-config execution root
//! - `aave_v3_origin_stack`: legacy compatibility root that still accepts canonical-or-built config
//!
//! The execute op keeps the current external Nix/Foundry tools as the backend, but the typed
//! authored/canonical/built boundary is now explicit and reusable inside MFM.

use std::sync::Arc;

use async_trait::async_trait;
use mfm_aave_v3_origin_config::{
    decode_aave_v3_origin_stack_built_config, decode_aave_v3_origin_stack_canonical_config,
    AaveV3OriginAdaptStepConfig, AaveV3OriginNixAppStepConfig, AaveV3OriginStackBuiltConfig,
    AaveV3OriginStackCanonicalConfig,
};
use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_op_aave_v3_origin_adapt::{
    AaveV3OriginAdaptDeployOp, AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_ID,
    AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_VERSION,
};
use mfm_op_nix_app::NixAppOp;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::{ChildOpLocalId, PortKey};
use mfm_sdk::op::{
    child_op_path, leaf_state_id, leaf_state_node, AfterEdge, ChildOpInstance, CompositeOpSpec,
    DynOperation, ImportBinding, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
    PlannerPayloadConfigSource, PortSource, ReExportBinding,
};
use mfm_state_aave_v3::manifest::AaveDeployManifest;
use mfm_state_common::ctx as op_ctx;
use mfm_state_common::errors as op_errors;
use mfm_state_common::states::publish::WriteContextValueArtifactState;
use serde::Deserialize;
use serde_json::Value;

mod config_build;
pub use config_build::{
    aave_v3_origin_stack_config_build_built_artifact_id_context_key,
    aave_v3_origin_stack_config_build_built_config_context_key,
    aave_v3_origin_stack_config_build_canonical_artifact_id_context_key,
    aave_v3_origin_stack_config_build_public_ops,
    aave_v3_origin_stack_config_build_report_context_key, AaveV3OriginStackConfigBuildOp,
    AAVE_V3_ORIGIN_STACK_CONFIG_BUILD_OP_ID,
};

/// Legacy public root op id that preserves the canonical-or-built compatibility path.
pub const AAVE_V3_ORIGIN_STACK_OP_ID: &str = "aave_v3_origin_stack";
/// Strict built-config execution root op id.
pub const AAVE_V3_ORIGIN_STACK_EXECUTE_OP_ID: &str = "aave_v3_origin_stack_execute";
/// Shared version for the public Aave Origin roots.
pub const AAVE_V3_ORIGIN_STACK_PUBLIC_OP_VERSION: &str = "v1";

const STACK_BUILD_CHILD_ID: &str = "b";
const STACK_EXECUTE_CHILD_ID: &str = "e";

const FETCH_ORIGIN_CHILD_ID: &str = "fetch_origin";
const COMPILE_ORIGIN_CHILD_ID: &str = "compile_origin";
const DEPLOY_ORIGIN_CHILD_ID: &str = "deploy_origin_stack";
const ADAPT_ORIGIN_CHILD_ID: &str = "adapt_origin_deploy";
const PUBLISH_FETCH_ARTIFACT_CHILD_ID: &str = "publish_fetch_origin_artifact";
const PUBLISH_COMPILE_ARTIFACT_CHILD_ID: &str = "publish_compile_origin_artifact";
const PUBLISH_DEPLOY_ARTIFACT_CHILD_ID: &str = "publish_deploy_origin_artifact";
const PROJECT_REPORT_CHILD_ID: &str = "project_report";

const PUBLISH_ARTIFACT_OP_ID: &str = "aave_v3_origin_publish_artifact";
const PROJECT_REPORT_OP_ID: &str = "aave_v3_origin_project_report";
const INTERNAL_OP_VERSION: &str = "v1-internal";

/// Export key for the raw fetch result.
pub const PORT_FETCH_ORIGIN_RESULT: &str = "fetch_origin_result";
/// Export key for the raw compile manifest.
pub const PORT_COMPILE_ORIGIN_RESULT: &str = "compile_origin_result";
/// Export key for the raw Origin deploy output.
pub const PORT_DEPLOY_ORIGIN_RESULT: &str = "deploy_origin_result";
/// Export key for the adapted deploy manifest.
pub const PORT_DEPLOY_MANIFEST: &str = "deploy_manifest";
/// Export key for the fetch-result artifact id.
pub const PORT_FETCH_ORIGIN_ARTIFACT_ID: &str = "fetch_origin_artifact_id";
/// Export key for the compile-manifest artifact id.
pub const PORT_COMPILE_ORIGIN_ARTIFACT_ID: &str = "compile_origin_artifact_id";
/// Export key for the deploy-output artifact id.
pub const PORT_DEPLOY_ORIGIN_ARTIFACT_ID: &str = "deploy_origin_artifact_id";
/// Export key for the stable execution report.
pub const PORT_REPORT: &str = "report";

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn child_export(child_id: &str, export: &str) -> PortSource {
    PortSource::ChildExport {
        child: ChildOpLocalId(child_id.to_string()),
        export: PortKey(export.to_string()),
    }
}

fn import_binding(to_child: &str, import: &str, source: PortSource) -> ImportBinding {
    ImportBinding {
        to_child: ChildOpLocalId(to_child.to_string()),
        import: PortKey(import.to_string()),
        source,
    }
}

fn re_export_binding(export: &str, source: PortSource) -> ReExportBinding {
    ReExportBinding {
        export: PortKey(export.to_string()),
        source,
    }
}

fn artifact_fact_key(op_path: &OpPath, role: &str) -> FactKey {
    FactKey(format!("aave:origin:execute:{role}|op:{}", op_path.0))
}

/// Stable execution report emitted by `aave_v3_origin_stack_execute`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AaveV3OriginStackExecutionReport {
    /// Schema version for this report surface.
    pub schema_version: u32,
    /// Context export key for the raw fetch result.
    pub fetch_result_export_key: String,
    /// Artifact id for the raw fetch result.
    pub fetch_result_artifact_id: String,
    /// Context export key for the raw compile manifest.
    pub compile_manifest_export_key: String,
    /// Artifact id for the raw compile manifest.
    pub compile_manifest_artifact_id: String,
    /// Context export key for the raw deploy output.
    pub deploy_output_export_key: String,
    /// Artifact id for the raw deploy output.
    pub deploy_output_artifact_id: String,
    /// Context export key for the adapted deploy manifest.
    pub deploy_manifest_export_key: String,
    /// Number of contracts present in the adapted deploy manifest.
    pub deploy_manifest_contract_count: u64,
}

impl AaveV3OriginStackExecutionReport {
    /// Current schema version for the execution report surface.
    pub const SCHEMA_VERSION: u32 = 1;
}

enum CompatibilityInput {
    Canonical(Box<AaveV3OriginStackCanonicalConfig>),
    Built(Box<AaveV3OriginStackBuiltConfig>),
}

#[derive(Clone, Debug, Deserialize)]
struct PublishArtifactConfig {
    input_port: String,
    output_artifact_id_key: String,
    artifact_role: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ProjectReportConfig {
    fetch_result_export_key: String,
    fetch_result_artifact_id_key: String,
    compile_manifest_export_key: String,
    compile_manifest_artifact_id_key: String,
    deploy_output_export_key: String,
    deploy_output_artifact_id_key: String,
    deploy_manifest_key: String,
    report_output_key: String,
}

/// Thin planner op that accepts canonical-or-built config and composes the build/execute workflow.
#[derive(Clone, Default)]
pub struct AaveV3OriginStackOp;

/// Thin planner op that executes pre-built Aave Origin stack config through the compatibility backend.
#[derive(Clone, Default)]
pub struct AaveV3OriginStackExecuteOp;

#[derive(Clone, Default)]
struct PublishArtifactOp;

#[derive(Clone, Default)]
struct ProjectExecutionReportOp;

#[derive(Clone)]
struct ProjectExecutionReportState {
    cfg: ProjectReportConfig,
}

/// Returns the built-in legacy public `aave_v3_origin_stack` root op.
pub fn aave_v3_origin_stack_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(AaveV3OriginStackOp) as DynOperation]
}

/// Returns the built-in public `aave_v3_origin_stack_execute` root op.
pub fn aave_v3_origin_stack_execute_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(AaveV3OriginStackExecuteOp) as DynOperation]
}

/// Returns all built-in public Aave Origin root ops.
pub fn aave_v3_origin_public_ops() -> Vec<DynOperation> {
    let mut ops = aave_v3_origin_stack_config_build_public_ops();
    ops.extend(aave_v3_origin_stack_public_ops());
    ops.extend(aave_v3_origin_stack_execute_public_ops());
    ops
}

/// Returns the planner-internal child ops used to lower Aave Origin execution outputs.
pub fn aave_v3_origin_internal_ops() -> Vec<DynOperation> {
    vec![
        Arc::new(PublishArtifactOp) as DynOperation,
        Arc::new(ProjectExecutionReportOp) as DynOperation,
    ]
}

/// Returns the full Aave Origin operation bundle, including public and planner-internal ops.
pub fn aave_v3_origin_ops() -> Vec<DynOperation> {
    let mut ops = aave_v3_origin_public_ops();
    ops.extend(aave_v3_origin_internal_ops());
    ops
}

/// Returns whether `op_id` refers to a planner-internal Aave Origin helper op.
pub fn is_aave_v3_origin_internal_op_id(op_id: &str) -> bool {
    aave_v3_origin_internal_op_ids().contains(&op_id)
}

/// Returns the canonical list of planner-internal Aave Origin helper op ids.
pub fn aave_v3_origin_internal_op_ids() -> &'static [&'static str] {
    const IDS: &[&str] = &[PUBLISH_ARTIFACT_OP_ID, PROJECT_REPORT_OP_ID];
    IDS
}

fn parse_compatibility_input(op_config: &Value) -> Result<CompatibilityInput, SdkError> {
    if let Ok(cfg) = decode_aave_v3_origin_stack_built_config(op_config) {
        return Ok(CompatibilityInput::Built(Box::new(cfg)));
    }

    decode_aave_v3_origin_stack_canonical_config(op_config)
        .map(Box::new)
        .map(CompatibilityInput::Canonical)
        .map_err(|err| sdk_input_error("invalid_aave_v3_origin_stack_config", err.to_string()))
}

fn parse_built_config(op_config: &Value) -> Result<AaveV3OriginStackBuiltConfig, SdkError> {
    decode_aave_v3_origin_stack_built_config(op_config)
        .map_err(|err| sdk_input_error("invalid_aave_v3_origin_stack_config", err.to_string()))
}

fn validate_nix_step_config(
    op_path: OpPath,
    child_id: &str,
    step: &AaveV3OriginNixAppStepConfig,
    run_config: &RunConfig,
) -> Result<(), SdkError> {
    let op = NixAppOp;
    op.expand(
        child_op_path(&op_path, child_id)?,
        &step.to_op_config(),
        run_config,
    )
    .map(|_| ())
}

fn validate_adapt_step_config(
    op_path: OpPath,
    step: &AaveV3OriginAdaptStepConfig,
    run_config: &RunConfig,
) -> Result<(), SdkError> {
    let op = AaveV3OriginAdaptDeployOp;
    op.expand(
        child_op_path(&op_path, ADAPT_ORIGIN_CHILD_ID)?,
        &step.to_op_config(),
        run_config,
    )
    .map(|_| ())
}

fn validate_execution_config(
    op_path: OpPath,
    cfg: &AaveV3OriginStackBuiltConfig,
    run_config: &RunConfig,
) -> Result<(), SdkError> {
    validate_nix_step_config(
        op_path.clone(),
        FETCH_ORIGIN_CHILD_ID,
        &cfg.execution.fetch_origin,
        run_config,
    )?;
    validate_nix_step_config(
        op_path.clone(),
        COMPILE_ORIGIN_CHILD_ID,
        &cfg.execution.compile_origin,
        run_config,
    )?;
    validate_nix_step_config(
        op_path.clone(),
        DEPLOY_ORIGIN_CHILD_ID,
        &cfg.execution.deploy_origin_stack,
        run_config,
    )?;
    validate_adapt_step_config(op_path, &cfg.execution.adapt_origin_deploy, run_config)?;
    Ok(())
}

fn execute_interface() -> OpInterface {
    OpInterface {
        imports: Vec::new(),
        exports: vec![
            PortKey(PORT_FETCH_ORIGIN_RESULT.to_string()),
            PortKey(PORT_COMPILE_ORIGIN_RESULT.to_string()),
            PortKey(PORT_DEPLOY_ORIGIN_RESULT.to_string()),
            PortKey(PORT_DEPLOY_MANIFEST.to_string()),
            PortKey(PORT_FETCH_ORIGIN_ARTIFACT_ID.to_string()),
            PortKey(PORT_COMPILE_ORIGIN_ARTIFACT_ID.to_string()),
            PortKey(PORT_DEPLOY_ORIGIN_ARTIFACT_ID.to_string()),
            PortKey(PORT_REPORT.to_string()),
        ],
    }
}

fn expand_from_canonical(
    _op_path: OpPath,
    canonical: AaveV3OriginStackCanonicalConfig,
    _run_config: &RunConfig,
) -> Result<PlannedOp, SdkError> {
    let canonical_json = serde_json::to_value(&canonical)
        .map_err(|err| sdk_input_error("invalid_aave_v3_origin_stack_config", err.to_string()))?;

    Ok(PlannedOp {
        interface: execute_interface(),
        kind: PlannedOpKind::Composite(CompositeOpSpec {
            children: vec![
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(STACK_BUILD_CHILD_ID.to_string()),
                    op_id: OpId::must_new(AAVE_V3_ORIGIN_STACK_CONFIG_BUILD_OP_ID.to_string()),
                    op_version: AAVE_V3_ORIGIN_STACK_PUBLIC_OP_VERSION.to_string(),
                    op_config: canonical_json,
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(STACK_EXECUTE_CHILD_ID.to_string()),
                    op_id: OpId::must_new(AAVE_V3_ORIGIN_STACK_EXECUTE_OP_ID.to_string()),
                    op_version: AAVE_V3_ORIGIN_STACK_PUBLIC_OP_VERSION.to_string(),
                    op_config: serde_json::json!({}),
                    op_config_from_planner_payload: Some(PlannerPayloadConfigSource {
                        child: ChildOpLocalId(STACK_BUILD_CHILD_ID.to_string()),
                        pointer: "/built_config".to_string(),
                    }),
                },
            ],
            bindings: Vec::new(),
            order: vec![AfterEdge {
                from_child: ChildOpLocalId(STACK_BUILD_CHILD_ID.to_string()),
                to_child: ChildOpLocalId(STACK_EXECUTE_CHILD_ID.to_string()),
            }],
            re_exports: execute_interface()
                .exports
                .iter()
                .map(|export| ReExportBinding {
                    export: export.clone(),
                    source: PortSource::ChildExport {
                        child: ChildOpLocalId(STACK_EXECUTE_CHILD_ID.to_string()),
                        export: export.clone(),
                    },
                })
                .collect(),
        }),
    })
}

fn expand_execution(
    op_path: OpPath,
    cfg: &AaveV3OriginStackBuiltConfig,
    run_config: &RunConfig,
) -> Result<PlannedOp, SdkError> {
    validate_execution_config(op_path.clone(), cfg, run_config)?;

    let fetch_artifact_role = "fetch_result".to_string();
    let compile_artifact_role = "compile_manifest".to_string();
    let deploy_artifact_role = "deploy_output".to_string();

    Ok(PlannedOp {
        interface: execute_interface(),
        kind: PlannedOpKind::Composite(CompositeOpSpec {
            children: vec![
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(FETCH_ORIGIN_CHILD_ID.to_string()),
                    op_id: OpId::must_new("nix_app".to_string()),
                    op_version: "v1".to_string(),
                    op_config: cfg.execution.fetch_origin.to_op_config(),
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(COMPILE_ORIGIN_CHILD_ID.to_string()),
                    op_id: OpId::must_new("nix_app".to_string()),
                    op_version: "v1".to_string(),
                    op_config: cfg.execution.compile_origin.to_op_config(),
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(DEPLOY_ORIGIN_CHILD_ID.to_string()),
                    op_id: OpId::must_new("nix_app".to_string()),
                    op_version: "v1".to_string(),
                    op_config: cfg.execution.deploy_origin_stack.to_op_config(),
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(ADAPT_ORIGIN_CHILD_ID.to_string()),
                    op_id: OpId::must_new(AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_ID.to_string()),
                    op_version: AAVE_V3_ORIGIN_ADAPT_DEPLOY_OP_VERSION.to_string(),
                    op_config: cfg.execution.adapt_origin_deploy.to_op_config(),
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(PUBLISH_FETCH_ARTIFACT_CHILD_ID.to_string()),
                    op_id: OpId::must_new(PUBLISH_ARTIFACT_OP_ID.to_string()),
                    op_version: INTERNAL_OP_VERSION.to_string(),
                    op_config: serde_json::json!({
                        "input_port": PORT_FETCH_ORIGIN_RESULT,
                        "output_artifact_id_key": PORT_FETCH_ORIGIN_ARTIFACT_ID,
                        "artifact_role": fetch_artifact_role,
                    }),
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(
                        PUBLISH_COMPILE_ARTIFACT_CHILD_ID.to_string(),
                    ),
                    op_id: OpId::must_new(PUBLISH_ARTIFACT_OP_ID.to_string()),
                    op_version: INTERNAL_OP_VERSION.to_string(),
                    op_config: serde_json::json!({
                        "input_port": PORT_COMPILE_ORIGIN_RESULT,
                        "output_artifact_id_key": PORT_COMPILE_ORIGIN_ARTIFACT_ID,
                        "artifact_role": compile_artifact_role,
                    }),
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(PUBLISH_DEPLOY_ARTIFACT_CHILD_ID.to_string()),
                    op_id: OpId::must_new(PUBLISH_ARTIFACT_OP_ID.to_string()),
                    op_version: INTERNAL_OP_VERSION.to_string(),
                    op_config: serde_json::json!({
                        "input_port": PORT_DEPLOY_ORIGIN_RESULT,
                        "output_artifact_id_key": PORT_DEPLOY_ORIGIN_ARTIFACT_ID,
                        "artifact_role": deploy_artifact_role,
                    }),
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(PROJECT_REPORT_CHILD_ID.to_string()),
                    op_id: OpId::must_new(PROJECT_REPORT_OP_ID.to_string()),
                    op_version: INTERNAL_OP_VERSION.to_string(),
                    op_config: serde_json::json!({
                        "fetch_result_export_key": PORT_FETCH_ORIGIN_RESULT,
                        "fetch_result_artifact_id_key": PORT_FETCH_ORIGIN_ARTIFACT_ID,
                        "compile_manifest_export_key": PORT_COMPILE_ORIGIN_RESULT,
                        "compile_manifest_artifact_id_key": PORT_COMPILE_ORIGIN_ARTIFACT_ID,
                        "deploy_output_export_key": PORT_DEPLOY_ORIGIN_RESULT,
                        "deploy_output_artifact_id_key": PORT_DEPLOY_ORIGIN_ARTIFACT_ID,
                        "deploy_manifest_key": PORT_DEPLOY_MANIFEST,
                        "report_output_key": PORT_REPORT,
                    }),
                    op_config_from_planner_payload: None,
                },
            ],
            bindings: vec![
                import_binding(
                    ADAPT_ORIGIN_CHILD_ID,
                    PORT_DEPLOY_ORIGIN_RESULT,
                    child_export(DEPLOY_ORIGIN_CHILD_ID, PORT_DEPLOY_ORIGIN_RESULT),
                ),
                import_binding(
                    PUBLISH_FETCH_ARTIFACT_CHILD_ID,
                    PORT_FETCH_ORIGIN_RESULT,
                    child_export(FETCH_ORIGIN_CHILD_ID, PORT_FETCH_ORIGIN_RESULT),
                ),
                import_binding(
                    PUBLISH_COMPILE_ARTIFACT_CHILD_ID,
                    PORT_COMPILE_ORIGIN_RESULT,
                    child_export(COMPILE_ORIGIN_CHILD_ID, PORT_COMPILE_ORIGIN_RESULT),
                ),
                import_binding(
                    PUBLISH_DEPLOY_ARTIFACT_CHILD_ID,
                    PORT_DEPLOY_ORIGIN_RESULT,
                    child_export(DEPLOY_ORIGIN_CHILD_ID, PORT_DEPLOY_ORIGIN_RESULT),
                ),
                import_binding(
                    PROJECT_REPORT_CHILD_ID,
                    PORT_FETCH_ORIGIN_ARTIFACT_ID,
                    child_export(
                        PUBLISH_FETCH_ARTIFACT_CHILD_ID,
                        PORT_FETCH_ORIGIN_ARTIFACT_ID,
                    ),
                ),
                import_binding(
                    PROJECT_REPORT_CHILD_ID,
                    PORT_COMPILE_ORIGIN_ARTIFACT_ID,
                    child_export(
                        PUBLISH_COMPILE_ARTIFACT_CHILD_ID,
                        PORT_COMPILE_ORIGIN_ARTIFACT_ID,
                    ),
                ),
                import_binding(
                    PROJECT_REPORT_CHILD_ID,
                    PORT_DEPLOY_ORIGIN_ARTIFACT_ID,
                    child_export(
                        PUBLISH_DEPLOY_ARTIFACT_CHILD_ID,
                        PORT_DEPLOY_ORIGIN_ARTIFACT_ID,
                    ),
                ),
                import_binding(
                    PROJECT_REPORT_CHILD_ID,
                    PORT_DEPLOY_MANIFEST,
                    child_export(ADAPT_ORIGIN_CHILD_ID, PORT_DEPLOY_MANIFEST),
                ),
            ],
            order: vec![
                AfterEdge {
                    from_child: ChildOpLocalId(FETCH_ORIGIN_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(COMPILE_ORIGIN_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(COMPILE_ORIGIN_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(DEPLOY_ORIGIN_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(DEPLOY_ORIGIN_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(ADAPT_ORIGIN_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(FETCH_ORIGIN_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(PUBLISH_FETCH_ARTIFACT_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(COMPILE_ORIGIN_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(PUBLISH_COMPILE_ARTIFACT_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(DEPLOY_ORIGIN_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(PUBLISH_DEPLOY_ARTIFACT_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(ADAPT_ORIGIN_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(PROJECT_REPORT_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(PUBLISH_FETCH_ARTIFACT_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(PROJECT_REPORT_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(PUBLISH_COMPILE_ARTIFACT_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(PROJECT_REPORT_CHILD_ID.to_string()),
                },
                AfterEdge {
                    from_child: ChildOpLocalId(PUBLISH_DEPLOY_ARTIFACT_CHILD_ID.to_string()),
                    to_child: ChildOpLocalId(PROJECT_REPORT_CHILD_ID.to_string()),
                },
            ],
            re_exports: vec![
                re_export_binding(
                    PORT_FETCH_ORIGIN_RESULT,
                    child_export(FETCH_ORIGIN_CHILD_ID, PORT_FETCH_ORIGIN_RESULT),
                ),
                re_export_binding(
                    PORT_COMPILE_ORIGIN_RESULT,
                    child_export(COMPILE_ORIGIN_CHILD_ID, PORT_COMPILE_ORIGIN_RESULT),
                ),
                re_export_binding(
                    PORT_DEPLOY_ORIGIN_RESULT,
                    child_export(DEPLOY_ORIGIN_CHILD_ID, PORT_DEPLOY_ORIGIN_RESULT),
                ),
                re_export_binding(
                    PORT_DEPLOY_MANIFEST,
                    child_export(ADAPT_ORIGIN_CHILD_ID, PORT_DEPLOY_MANIFEST),
                ),
                re_export_binding(
                    PORT_FETCH_ORIGIN_ARTIFACT_ID,
                    child_export(
                        PUBLISH_FETCH_ARTIFACT_CHILD_ID,
                        PORT_FETCH_ORIGIN_ARTIFACT_ID,
                    ),
                ),
                re_export_binding(
                    PORT_COMPILE_ORIGIN_ARTIFACT_ID,
                    child_export(
                        PUBLISH_COMPILE_ARTIFACT_CHILD_ID,
                        PORT_COMPILE_ORIGIN_ARTIFACT_ID,
                    ),
                ),
                re_export_binding(
                    PORT_DEPLOY_ORIGIN_ARTIFACT_ID,
                    child_export(
                        PUBLISH_DEPLOY_ARTIFACT_CHILD_ID,
                        PORT_DEPLOY_ORIGIN_ARTIFACT_ID,
                    ),
                ),
                re_export_binding(
                    PORT_REPORT,
                    child_export(PROJECT_REPORT_CHILD_ID, PORT_REPORT),
                ),
            ],
        }),
    })
}

impl Operation for AaveV3OriginStackOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(AAVE_V3_ORIGIN_STACK_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        AAVE_V3_ORIGIN_STACK_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        match parse_compatibility_input(op_config)? {
            CompatibilityInput::Canonical(canonical) => {
                expand_from_canonical(op_path, *canonical, run_config)
            }
            CompatibilityInput::Built(built) => expand_execution(op_path, &built, run_config),
        }
    }
}

impl Operation for AaveV3OriginStackExecuteOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(AAVE_V3_ORIGIN_STACK_EXECUTE_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        AAVE_V3_ORIGIN_STACK_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg = parse_built_config(op_config)?;
        expand_execution(op_path, &cfg, run_config)
    }
}

impl Operation for PublishArtifactOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PUBLISH_ARTIFACT_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: PublishArtifactConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                sdk_input_error(
                    "invalid_aave_v3_origin_internal_config",
                    "invalid publish-artifact op_config",
                )
            })?;
        if cfg.input_port.trim().is_empty() {
            return Err(sdk_input_error(
                "invalid_aave_v3_origin_internal_config",
                "input_port must be non-empty",
            ));
        }
        if cfg.output_artifact_id_key.trim().is_empty() {
            return Err(sdk_input_error(
                "invalid_aave_v3_origin_internal_config",
                "output_artifact_id_key must be non-empty",
            ));
        }
        if cfg.artifact_role.trim().is_empty() {
            return Err(sdk_input_error(
                "invalid_aave_v3_origin_internal_config",
                "artifact_role must be non-empty",
            ));
        }

        let state_id = leaf_state_id(&op_path, "publish_artifact")?;
        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![PortKey(cfg.input_port.clone())],
                exports: vec![PortKey(cfg.output_artifact_id_key.clone())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    "publish_artifact",
                    Arc::new(WriteContextValueArtifactState {
                        state_id,
                        input_key: ContextKey(cfg.input_port.clone()),
                        fact_key: artifact_fact_key(&op_path, &cfg.artifact_role),
                        output_artifact_id_key: ContextKey(cfg.output_artifact_id_key.clone()),
                        missing_input_code: "missing_aave_v3_origin_execution_payload",
                        missing_input_message:
                            "missing Aave V3 Origin execution payload before artifact publication",
                    }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

impl Operation for ProjectExecutionReportOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PROJECT_REPORT_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        INTERNAL_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: ProjectReportConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            sdk_input_error(
                "invalid_aave_v3_origin_internal_config",
                "invalid project-report op_config",
            )
        })?;

        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![
                    PortKey(cfg.fetch_result_artifact_id_key.clone()),
                    PortKey(cfg.compile_manifest_artifact_id_key.clone()),
                    PortKey(cfg.deploy_output_artifact_id_key.clone()),
                    PortKey(cfg.deploy_manifest_key.clone()),
                ],
                exports: vec![PortKey(cfg.report_output_key.clone())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    "project_report",
                    Arc::new(ProjectExecutionReportState { cfg }),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

#[async_trait]
impl State for ProjectExecutionReportState {
    fn meta(&self) -> StateMeta {
        mfm_state_common::states::meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let fetch_result_artifact_id = op_ctx::read_string_required(
            ctx,
            &ContextKey(self.cfg.fetch_result_artifact_id_key.clone()),
            "missing_aave_v3_origin_fetch_artifact_id",
            "missing fetch result artifact id",
            "invalid_aave_v3_origin_fetch_artifact_id",
            "fetch result artifact id must be a string",
        )?;
        let compile_manifest_artifact_id = op_ctx::read_string_required(
            ctx,
            &ContextKey(self.cfg.compile_manifest_artifact_id_key.clone()),
            "missing_aave_v3_origin_compile_artifact_id",
            "missing compile manifest artifact id",
            "invalid_aave_v3_origin_compile_artifact_id",
            "compile manifest artifact id must be a string",
        )?;
        let deploy_output_artifact_id = op_ctx::read_string_required(
            ctx,
            &ContextKey(self.cfg.deploy_output_artifact_id_key.clone()),
            "missing_aave_v3_origin_deploy_artifact_id",
            "missing deploy output artifact id",
            "invalid_aave_v3_origin_deploy_artifact_id",
            "deploy output artifact id must be a string",
        )?;
        let deploy_manifest: AaveDeployManifest = op_ctx::read_typed(
            ctx,
            &ContextKey(self.cfg.deploy_manifest_key.clone()),
            "missing_aave_v3_origin_deploy_manifest",
            "missing deploy manifest",
            "invalid_aave_v3_origin_deploy_manifest",
            "deploy manifest was invalid",
        )?;

        let report = serde_json::to_value(AaveV3OriginStackExecutionReport {
            schema_version: AaveV3OriginStackExecutionReport::SCHEMA_VERSION,
            fetch_result_export_key: self.cfg.fetch_result_export_key.clone(),
            fetch_result_artifact_id,
            compile_manifest_export_key: self.cfg.compile_manifest_export_key.clone(),
            compile_manifest_artifact_id,
            deploy_output_export_key: self.cfg.deploy_output_export_key.clone(),
            deploy_output_artifact_id,
            deploy_manifest_export_key: self.cfg.deploy_manifest_key.clone(),
            deploy_manifest_contract_count: deploy_manifest.contracts.len() as u64,
        })
        .map_err(|_| {
            op_errors::state_unknown(
                "invalid_aave_v3_origin_execution_report",
                "failed to serialize Aave V3 Origin execution report",
            )
        })?;

        op_ctx::write_json(ctx, ContextKey(self.cfg.report_output_key.clone()), report)?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
#[path = "tests/aave_v3_origin_op_tests.rs"]
mod aave_v3_origin_op_tests;
