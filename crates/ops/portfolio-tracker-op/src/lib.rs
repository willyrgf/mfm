#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Portfolio config-build and execution planner ops.
//!
//! Source of truth:
//! - `docs/design.md`
//! - `docs/ops-and-states.md`
//!
//! This crate owns the additive public portfolio workflow boundaries:
//!
//! - `portfolio_config_build`: canonical config -> built config plus explicit config artifacts
//! - `portfolio_execute`: built config -> canonical snapshot/runtime report
//! - `portfolio_tracker`: canonical public root that composes build then execute
//!
//! All three remain thin planners. Legacy semantic compilation is isolated in this op crate until
//! the typed portfolio workflow port replaces it, and runtime execution stays in the reusable
//! shared portfolio states.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_portfolio_tracker::PortfolioTrackerOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = PortfolioTrackerOp;
//! assert_eq!(op.op_id().as_str(), "portfolio_tracker");
//! ```

use std::collections::HashSet;
use std::sync::Arc;

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_portfolio_config::{
    decode_portfolio_snapshot_canonical_config, PortfolioSnapshotCanonicalConfig,
};
use mfm_portfolio_plan::{PortfolioExecutionSpec, PORTFOLIO_EXECUTION_SPEC_KEY};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::{ChildOpLocalId, PortKey};
use mfm_sdk::op::{
    AfterEdge, ChildOpInstance, CompositeOpSpec, DynOperation, ImportBinding, OpInterface,
    Operation, PlannedOp, PlannedOpKind, PlannerPayloadConfigSource, PortSource, ReExportBinding,
};
use mfm_state_common::errors as op_errors;
use serde_json::Value;

mod config_build;
mod defaults;
mod plan_ops;
pub use config_build::{
    build_portfolio_snapshot_config, build_portfolio_snapshot_outcome,
    decode_portfolio_snapshot_built_config, portfolio_config_build_built_artifact_id_context_key,
    portfolio_config_build_built_config_context_key,
    portfolio_config_build_canonical_artifact_id_context_key, portfolio_config_build_public_ops,
    portfolio_config_build_report_context_key, PortfolioConfigBuildOp, PortfolioSnapshotBuildError,
    PortfolioSnapshotBuildOutcome, PortfolioSnapshotBuiltConfig,
    PortfolioSnapshotExecutionConfigError, PORTFOLIO_CONFIG_BUILD_OP_ID,
};
pub use defaults::{builtin_dispatch_catalog, DefaultPortfolioPlanCompiler};
use plan_ops::{
    assemble_snapshot_config, child_ops, merge_observations_config, observe_batch_config,
    pin_execution_views_config, prepare_sources_config, resolve_subjects_config,
    resolve_valuations_config, ASSEMBLE_SNAPSHOT_CHILD_ID, ASSEMBLE_SNAPSHOT_OP_ID,
    MERGE_OBSERVATIONS_CHILD_ID, MERGE_OBSERVATIONS_OP_ID, OBSERVE_COMPILED_BATCH_OP_ID,
    PIN_EXECUTION_VIEWS_CHILD_ID, PIN_EXECUTION_VIEWS_OP_ID, PORT_OBSERVATIONS, PORT_PINNED_VIEWS,
    PORT_PREPARED_SOURCES, PORT_REPORT, PORT_RESOLVED_SUBJECTS, PORT_RESOLVED_VALUATIONS,
    PORT_SNAPSHOT, PORT_SNAPSHOT_ARTIFACT_ID, PREPARE_EXECUTION_SOURCES_CHILD_ID,
    PREPARE_EXECUTION_SOURCES_OP_ID, PROJECT_REPORT_CHILD_ID, PROJECT_REPORT_OP_ID,
    RESOLVE_SUBJECTS_CHILD_ID, RESOLVE_SUBJECTS_OP_ID, RESOLVE_VALUATION_INPUTS_CHILD_ID,
    RESOLVE_VALUATION_INPUTS_OP_ID,
};

/// Canonical public root op id that composes build then execute.
pub const PORTFOLIO_TRACKER_OP_ID: &str = "portfolio_tracker";
/// Built-config public root op id used by thin transport adapters.
pub const PORTFOLIO_EXECUTE_OP_ID: &str = "portfolio_execute";
/// Shared version for the public portfolio execution roots.
pub const PORTFOLIO_PUBLIC_OP_VERSION: &str = "v1";

const PORTFOLIO_TRACKER_MAIN_OP_PATH: &str = "portfolio_tracker.main";
const PORTFOLIO_EXECUTE_MAIN_OP_PATH: &str = "portfolio_execute.main";
const PORTFOLIO_TRACKER_BUILD_CHILD_ID: &str = "b";
const PORTFOLIO_TRACKER_EXECUTE_CHILD_ID: &str = "e";

/// Returns the context key that stores the canonical portfolio snapshot JSON.
pub fn portfolio_snapshot_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_TRACKER_MAIN_OP_PATH}.{PORTFOLIO_TRACKER_EXECUTE_CHILD_ID}.out.{PORT_SNAPSHOT}"
    ))
}

/// Returns the context key that stores the canonical portfolio snapshot JSON for
/// `portfolio_execute`.
pub fn portfolio_execute_snapshot_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_EXECUTE_MAIN_OP_PATH}.out.{PORT_SNAPSHOT}"
    ))
}

/// Returns the context key that stores the canonical portfolio snapshot artifact id.
pub fn portfolio_snapshot_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_TRACKER_MAIN_OP_PATH}.{PORTFOLIO_TRACKER_EXECUTE_CHILD_ID}.out.{PORT_SNAPSHOT_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the canonical portfolio snapshot artifact id for
/// `portfolio_execute`.
pub fn portfolio_execute_snapshot_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_EXECUTE_MAIN_OP_PATH}.out.{PORT_SNAPSHOT_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the canonical portfolio report JSON.
pub fn portfolio_snapshot_report_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_TRACKER_MAIN_OP_PATH}.{PORTFOLIO_TRACKER_EXECUTE_CHILD_ID}.out.{PORT_REPORT}"
    ))
}

/// Returns the context key that stores the canonical portfolio report JSON for `portfolio_execute`.
pub fn portfolio_execute_report_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_EXECUTE_MAIN_OP_PATH}.out.{PORT_REPORT}"
    ))
}

type PortfolioTrackerConfig = PortfolioSnapshotBuiltConfig;

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

#[cfg(test)]
fn parse_config(op_config: &Value) -> Result<PortfolioTrackerConfig, SdkError> {
    if let Ok(cfg) = parse_built_config(op_config) {
        return Ok(cfg);
    }

    let canonical = parse_canonical_config(op_config)?;
    build_portfolio_snapshot_outcome(canonical)
        .map(|outcome| outcome.built)
        .map_err(|err| sdk_input_error("invalid_portfolio_execution_config", err.to_string()))
}

fn parse_canonical_config(op_config: &Value) -> Result<PortfolioSnapshotCanonicalConfig, SdkError> {
    decode_portfolio_snapshot_canonical_config(op_config)
        .map_err(|err| sdk_input_error("invalid_portfolio_execution_config", err.to_string()))
}

fn parse_built_config(op_config: &Value) -> Result<PortfolioTrackerConfig, SdkError> {
    decode_portfolio_snapshot_built_config(op_config)
        .map_err(|err| sdk_input_error("invalid_portfolio_execution_config", err.to_string()))
}

fn output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:output|op:{}", op_path.0))
}

fn compile_portfolio_plan(
    cfg: &PortfolioTrackerConfig,
) -> Result<PortfolioExecutionSpec, SdkError> {
    Ok(cfg.execution_spec.clone())
}

fn planner_payload_from_config(
    op_config: &Value,
    parse: fn(&Value) -> Result<PortfolioTrackerConfig, SdkError>,
) -> Result<Option<Value>, SdkError> {
    let cfg = parse(op_config)?;
    let spec = compile_portfolio_plan(&cfg)?;
    Ok(Some(serde_json::json!({
        (PORTFOLIO_EXECUTION_SPEC_KEY): spec
    })))
}

fn planner_payload_from_tracker_input(op_config: &Value) -> Result<Option<Value>, SdkError> {
    let canonical = parse_canonical_config(op_config)?;
    let cfg = build_portfolio_snapshot_outcome(canonical)
        .map_err(|err| sdk_input_error("invalid_portfolio_execution_config", err.to_string()))?
        .built;

    let spec = compile_portfolio_plan(&cfg)?;
    Ok(Some(serde_json::json!({
        (PORTFOLIO_EXECUTION_SPEC_KEY): spec
    })))
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

fn portfolio_execute_interface() -> OpInterface {
    OpInterface {
        imports: Vec::new(),
        exports: vec![
            PortKey(PORT_SNAPSHOT.to_string()),
            PortKey(PORT_SNAPSHOT_ARTIFACT_ID.to_string()),
            PortKey(PORT_REPORT.to_string()),
        ],
    }
}

fn observe_child_local_id(index: usize, batch_id: &str) -> String {
    let prefix = format!("observe_{index}_");
    let sanitized = sanitize_child_local_id(batch_id);
    // The compatibility root now adds one extra wrapper child (`e`) before the semantic runtime.
    // Keep observe child ids short enough that both the built root and the compatibility root
    // still satisfy the flat `<machine>.<step>.<state>` contract after lowering.
    let suffix_len = 55usize.saturating_sub(prefix.len());
    let suffix: String = sanitized.chars().take(suffix_len).collect();
    format!("{prefix}{suffix}")
}

fn sanitize_child_local_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' | '_' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            _ => '_',
        })
        .collect()
}

/// Thin planner op that accepts canonical config and composes the build/execute workflow.
#[derive(Clone, Default)]
pub struct PortfolioTrackerOp;

/// Thin planner op that executes pre-built portfolio config through the shared runtime states.
#[derive(Clone, Default)]
pub struct PortfolioExecuteOp;

/// Returns the built-in legacy public `portfolio_tracker` root op.
pub fn portfolio_tracker_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(PortfolioTrackerOp) as DynOperation]
}

/// Returns the built-in public `portfolio_execute` root op.
pub fn portfolio_execute_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(PortfolioExecuteOp) as DynOperation]
}

/// Returns all built-in public portfolio root ops.
pub fn portfolio_public_ops() -> Vec<DynOperation> {
    let mut ops = portfolio_config_build_public_ops();
    ops.extend(portfolio_tracker_public_ops());
    ops.extend(portfolio_execute_public_ops());
    ops
}

/// Returns the planner-internal semantic child ops used to lower `portfolio_tracker`.
pub fn portfolio_tracker_internal_ops() -> Vec<DynOperation> {
    child_ops()
}

/// Returns the full portfolio operation bundle, including both public roots and planner-internal
/// child ops.
pub fn portfolio_tracker_ops() -> Vec<DynOperation> {
    let mut ops = portfolio_public_ops();
    ops.extend(portfolio_tracker_internal_ops());
    ops
}

/// Returns whether `op_id` refers to a planner-internal semantic child op.
pub fn is_portfolio_tracker_internal_op_id(op_id: &str) -> bool {
    portfolio_tracker_internal_op_ids().contains(&op_id)
}

/// Returns the canonical list of planner-internal semantic child op ids.
pub fn portfolio_tracker_internal_op_ids() -> &'static [&'static str] {
    const IDS: &[&str] = &[
        PREPARE_EXECUTION_SOURCES_OP_ID,
        RESOLVE_SUBJECTS_OP_ID,
        PIN_EXECUTION_VIEWS_OP_ID,
        RESOLVE_VALUATION_INPUTS_OP_ID,
        OBSERVE_COMPILED_BATCH_OP_ID,
        MERGE_OBSERVATIONS_OP_ID,
        ASSEMBLE_SNAPSHOT_OP_ID,
        PROJECT_REPORT_OP_ID,
    ];
    IDS
}

impl Operation for PortfolioTrackerOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PORTFOLIO_TRACKER_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        PORTFOLIO_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let canonical = parse_canonical_config(op_config)?;
        expand_portfolio_tracker_from_canonical(op_path, canonical)
    }

    fn planner_payload(
        &self,
        _op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<Option<Value>, SdkError> {
        planner_payload_from_tracker_input(op_config)
    }
}

impl Operation for PortfolioExecuteOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PORTFOLIO_EXECUTE_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        PORTFOLIO_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg = parse_built_config(op_config)?;
        expand_portfolio_execution(op_path, &cfg)
    }

    fn planner_payload(
        &self,
        _op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<Option<Value>, SdkError> {
        planner_payload_from_config(op_config, parse_built_config)
    }
}

fn expand_portfolio_execution(
    op_path: OpPath,
    cfg: &PortfolioTrackerConfig,
) -> Result<PlannedOp, SdkError> {
    let spec = compile_portfolio_plan(cfg)?;
    let mut children = vec![
        ChildOpInstance {
            child_op_local_id: ChildOpLocalId(PREPARE_EXECUTION_SOURCES_CHILD_ID.to_string()),
            op_id: OpId::must_new(PREPARE_EXECUTION_SOURCES_OP_ID.to_string()),
            op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: prepare_sources_config(&spec)?,
            op_config_from_planner_payload: None,
        },
        ChildOpInstance {
            child_op_local_id: ChildOpLocalId(RESOLVE_SUBJECTS_CHILD_ID.to_string()),
            op_id: OpId::must_new(RESOLVE_SUBJECTS_OP_ID.to_string()),
            op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: resolve_subjects_config(&spec)?,
            op_config_from_planner_payload: None,
        },
        ChildOpInstance {
            child_op_local_id: ChildOpLocalId(PIN_EXECUTION_VIEWS_CHILD_ID.to_string()),
            op_id: OpId::must_new(PIN_EXECUTION_VIEWS_OP_ID.to_string()),
            op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: pin_execution_views_config(&spec)?,
            op_config_from_planner_payload: None,
        },
        ChildOpInstance {
            child_op_local_id: ChildOpLocalId(RESOLVE_VALUATION_INPUTS_CHILD_ID.to_string()),
            op_id: OpId::must_new(RESOLVE_VALUATION_INPUTS_OP_ID.to_string()),
            op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: resolve_valuations_config(&spec)?,
            op_config_from_planner_payload: None,
        },
    ];
    let mut bindings = vec![
        import_binding(
            RESOLVE_SUBJECTS_CHILD_ID,
            PORT_PREPARED_SOURCES,
            child_export(PREPARE_EXECUTION_SOURCES_CHILD_ID, PORT_PREPARED_SOURCES),
        ),
        import_binding(
            PIN_EXECUTION_VIEWS_CHILD_ID,
            PORT_PREPARED_SOURCES,
            child_export(PREPARE_EXECUTION_SOURCES_CHILD_ID, PORT_PREPARED_SOURCES),
        ),
        import_binding(
            RESOLVE_VALUATION_INPUTS_CHILD_ID,
            PORT_PINNED_VIEWS,
            child_export(PIN_EXECUTION_VIEWS_CHILD_ID, PORT_PINNED_VIEWS),
        ),
    ];

    let mut merge_input_ports = Vec::new();
    let mut seen_observe_child_ids = HashSet::new();
    for (index, batch) in spec.observation_batches.iter().enumerate() {
        let child_id = observe_child_local_id(index, batch.batch_id.as_str());
        if !seen_observe_child_ids.insert(child_id.clone()) {
            return Err(sdk_input_error(
                "duplicate_observe_child_id",
                format!(
                    "semantic batch `{}` lowered to duplicate child op id `{child_id}`",
                    batch.batch_id
                ),
            ));
        }
        children.push(ChildOpInstance {
            child_op_local_id: ChildOpLocalId(child_id.clone()),
            op_id: OpId::must_new(OBSERVE_COMPILED_BATCH_OP_ID.to_string()),
            op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: observe_batch_config(batch)?,
            op_config_from_planner_payload: None,
        });
        bindings.push(import_binding(
            child_id.as_str(),
            PORT_RESOLVED_SUBJECTS,
            child_export(RESOLVE_SUBJECTS_CHILD_ID, PORT_RESOLVED_SUBJECTS),
        ));
        bindings.push(import_binding(
            child_id.as_str(),
            PORT_PINNED_VIEWS,
            child_export(PIN_EXECUTION_VIEWS_CHILD_ID, PORT_PINNED_VIEWS),
        ));
        bindings.push(import_binding(
            child_id.as_str(),
            PORT_RESOLVED_VALUATIONS,
            child_export(RESOLVE_VALUATION_INPUTS_CHILD_ID, PORT_RESOLVED_VALUATIONS),
        ));
        let merge_port = format!("batch_{index}");
        merge_input_ports.push(merge_port.clone());
        bindings.push(import_binding(
            MERGE_OBSERVATIONS_CHILD_ID,
            merge_port.as_str(),
            child_export(child_id.as_str(), PORT_OBSERVATIONS),
        ));
    }

    children.push(ChildOpInstance {
        child_op_local_id: ChildOpLocalId(MERGE_OBSERVATIONS_CHILD_ID.to_string()),
        op_id: OpId::must_new(MERGE_OBSERVATIONS_OP_ID.to_string()),
        op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
        op_config: merge_observations_config(merge_input_ports)?,
        op_config_from_planner_payload: None,
    });
    children.push(ChildOpInstance {
        child_op_local_id: ChildOpLocalId(ASSEMBLE_SNAPSHOT_CHILD_ID.to_string()),
        op_id: OpId::must_new(ASSEMBLE_SNAPSHOT_OP_ID.to_string()),
        op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
        op_config: assemble_snapshot_config(&cfg.canonical.portfolio, output_fact_key(&op_path))?,
        op_config_from_planner_payload: None,
    });
    children.push(ChildOpInstance {
        child_op_local_id: ChildOpLocalId(PROJECT_REPORT_CHILD_ID.to_string()),
        op_id: OpId::must_new(PROJECT_REPORT_OP_ID.to_string()),
        op_version: plan_ops::INTERNAL_OP_VERSION.to_string(),
        op_config: serde_json::json!({}),
        op_config_from_planner_payload: None,
    });

    bindings.push(import_binding(
        ASSEMBLE_SNAPSHOT_CHILD_ID,
        PORT_RESOLVED_SUBJECTS,
        child_export(RESOLVE_SUBJECTS_CHILD_ID, PORT_RESOLVED_SUBJECTS),
    ));
    bindings.push(import_binding(
        ASSEMBLE_SNAPSHOT_CHILD_ID,
        PORT_PINNED_VIEWS,
        child_export(PIN_EXECUTION_VIEWS_CHILD_ID, PORT_PINNED_VIEWS),
    ));
    bindings.push(import_binding(
        ASSEMBLE_SNAPSHOT_CHILD_ID,
        PORT_OBSERVATIONS,
        child_export(MERGE_OBSERVATIONS_CHILD_ID, PORT_OBSERVATIONS),
    ));
    bindings.push(import_binding(
        PROJECT_REPORT_CHILD_ID,
        PORT_SNAPSHOT,
        child_export(ASSEMBLE_SNAPSHOT_CHILD_ID, PORT_SNAPSHOT),
    ));

    Ok(PlannedOp {
        interface: OpInterface {
            imports: Vec::new(),
            exports: vec![
                PortKey(PORT_SNAPSHOT.to_string()),
                PortKey(PORT_SNAPSHOT_ARTIFACT_ID.to_string()),
                PortKey(PORT_REPORT.to_string()),
            ],
        },
        kind: PlannedOpKind::Composite(CompositeOpSpec {
            children,
            bindings,
            order: Vec::<AfterEdge>::new(),
            re_exports: vec![
                re_export_binding(
                    PORT_SNAPSHOT,
                    child_export(ASSEMBLE_SNAPSHOT_CHILD_ID, PORT_SNAPSHOT),
                ),
                re_export_binding(
                    PORT_SNAPSHOT_ARTIFACT_ID,
                    child_export(ASSEMBLE_SNAPSHOT_CHILD_ID, PORT_SNAPSHOT_ARTIFACT_ID),
                ),
                re_export_binding(
                    PORT_REPORT,
                    child_export(PROJECT_REPORT_CHILD_ID, PORT_REPORT),
                ),
            ],
        }),
    })
}

fn expand_portfolio_tracker_from_canonical(
    _op_path: OpPath,
    canonical: PortfolioSnapshotCanonicalConfig,
) -> Result<PlannedOp, SdkError> {
    Ok(PlannedOp {
        interface: portfolio_execute_interface(),
        kind: PlannedOpKind::Composite(CompositeOpSpec {
            children: vec![
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(PORTFOLIO_TRACKER_BUILD_CHILD_ID.to_string()),
                    op_id: OpId::must_new(PORTFOLIO_CONFIG_BUILD_OP_ID.to_string()),
                    op_version: PORTFOLIO_PUBLIC_OP_VERSION.to_string(),
                    op_config: serde_json::to_value(&canonical).map_err(|err| {
                        sdk_input_error("invalid_portfolio_execution_config", err.to_string())
                    })?,
                    op_config_from_planner_payload: None,
                },
                ChildOpInstance {
                    child_op_local_id: ChildOpLocalId(
                        PORTFOLIO_TRACKER_EXECUTE_CHILD_ID.to_string(),
                    ),
                    op_id: OpId::must_new(PORTFOLIO_EXECUTE_OP_ID.to_string()),
                    op_version: PORTFOLIO_PUBLIC_OP_VERSION.to_string(),
                    op_config: serde_json::json!({}),
                    op_config_from_planner_payload: Some(PlannerPayloadConfigSource {
                        child: ChildOpLocalId(PORTFOLIO_TRACKER_BUILD_CHILD_ID.to_string()),
                        pointer: "/built_config".to_string(),
                    }),
                },
            ],
            bindings: Vec::new(),
            order: vec![AfterEdge {
                from_child: ChildOpLocalId(PORTFOLIO_TRACKER_BUILD_CHILD_ID.to_string()),
                to_child: ChildOpLocalId(PORTFOLIO_TRACKER_EXECUTE_CHILD_ID.to_string()),
            }],
            re_exports: portfolio_execute_interface()
                .exports
                .iter()
                .map(|export| ReExportBinding {
                    export: export.clone(),
                    source: PortSource::ChildExport {
                        child: ChildOpLocalId(PORTFOLIO_TRACKER_EXECUTE_CHILD_ID.to_string()),
                        export: export.clone(),
                    },
                })
                .collect(),
        }),
    })
}

#[cfg(test)]
#[path = "tests/portfolio_tracker_op_tests.rs"]
mod portfolio_tracker_op_tests;
