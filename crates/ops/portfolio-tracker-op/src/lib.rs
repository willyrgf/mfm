#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Canonical portfolio snapshot planner op.
//!
//! Source of truth:
//! - `docs/design.md`
//! - `docs/ops-and-states.md`
//!
//! `portfolio_tracker` remains a thin planner that validates canonical config inputs and wires the
//! reusable shared-state runtime for multi-network portfolio execution.
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
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::{ChildOpLocalId, PortKey};
use mfm_sdk::op::{
    AfterEdge, ChildOpInstance, CompositeOpSpec, DynOperation, ImportBinding, OpInterface,
    Operation, PlannedOp, PlannedOpKind, PortSource, ReExportBinding,
};
use mfm_state_aave_v3::portfolio::model::validate_aave_portfolio_config;
use mfm_state_common::errors as op_errors;
use mfm_state_portfolio::model::{
    decode_portfolio_config, validate_portfolio_bundle, PortfolioConfig,
};
use mfm_state_portfolio::semantic::{PortfolioExecutionSpec, PORTFOLIO_EXECUTION_SPEC_KEY};
use mfm_state_symbol::model::{decode_valuation_source_registry, ValuationSourceRegistry};
use serde_json::Value;

mod semantic;
mod semantic_ops;

use semantic::{builtin_semantic_catalog, DefaultPortfolioSemanticCompiler};
use semantic_ops::{
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

const OP_ID: &str = "portfolio_tracker";
const OP_VERSION: &str = "v1";
const MAIN_OP_PATH: &str = "portfolio_tracker.main";

/// Returns the context key that stores the canonical portfolio snapshot JSON.
pub fn portfolio_snapshot_context_key() -> ContextKey {
    ContextKey(format!("{MAIN_OP_PATH}.out.{PORT_SNAPSHOT}"))
}

/// Returns the context key that stores the canonical portfolio snapshot artifact id.
pub fn portfolio_snapshot_artifact_id_context_key() -> ContextKey {
    ContextKey(format!("{MAIN_OP_PATH}.out.{PORT_SNAPSHOT_ARTIFACT_ID}"))
}

/// Returns the context key that stores the canonical portfolio report JSON.
pub fn portfolio_snapshot_report_context_key() -> ContextKey {
    ContextKey(format!("{MAIN_OP_PATH}.out.{PORT_REPORT}"))
}

#[derive(Clone, Debug)]
struct PortfolioTrackerConfig {
    portfolio: PortfolioConfig,
    valuation_source_registry: ValuationSourceRegistry,
}

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn parse_config(op_config: &Value) -> Result<PortfolioTrackerConfig, SdkError> {
    let obj = op_config.as_object().ok_or_else(|| {
        op_errors::sdk_parse_error(
            "invalid_op_config",
            "portfolio_tracker op_config must be a JSON object",
        )
    })?;

    let portfolio_value = obj.get("portfolio").ok_or_else(|| {
        sdk_input_error(
            "missing_portfolio_config",
            "portfolio_tracker op_config must contain `portfolio`",
        )
    })?;
    let valuation_source_registry_value =
        obj.get("valuation_source_registry").ok_or_else(|| {
            sdk_input_error(
                "missing_valuation_source_registry",
                "portfolio_tracker op_config must contain `valuation_source_registry`",
            )
        })?;

    let portfolio = decode_portfolio_config(portfolio_value)
        .map_err(|err| sdk_input_error("invalid_portfolio_config", err.to_string()))?;
    let valuation_source_registry =
        decode_valuation_source_registry(valuation_source_registry_value)
            .map_err(|err| sdk_input_error("invalid_valuation_source_registry", err.to_string()))?;
    validate_portfolio_bundle(&portfolio, &valuation_source_registry)
        .map_err(|err| sdk_input_error("invalid_portfolio_bundle", err.to_string()))?;
    validate_aave_portfolio_config(&portfolio)
        .map_err(|err| sdk_input_error("invalid_aave_portfolio_config", err.to_string()))?;

    Ok(PortfolioTrackerConfig {
        portfolio,
        valuation_source_registry,
    })
}

fn output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:output|op:{}", op_path.0))
}

fn compile_semantic_execution(
    cfg: &PortfolioTrackerConfig,
) -> Result<PortfolioExecutionSpec, SdkError> {
    let semantic_catalog = builtin_semantic_catalog().map_err(|err| {
        sdk_input_error(
            "semantic_catalog_construction_failed",
            format!("failed to construct semantic adapter catalog: {err}"),
        )
    })?;
    let semantic_request = mfm_state_portfolio::semantic::PortfolioRequest {
        portfolio: cfg.portfolio.clone(),
        valuation_source_registry: cfg.valuation_source_registry.clone(),
    };
    mfm_state_portfolio::semantic::PortfolioSemanticCompiler::compile(
        &DefaultPortfolioSemanticCompiler,
        &semantic_request,
        &semantic_catalog,
    )
    .map_err(|err| {
        sdk_input_error(
            "semantic_compilation_failed",
            format!("failed to compile semantic execution spec: {err}"),
        )
    })
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

fn observe_child_local_id(index: usize, batch_id: &str) -> String {
    let prefix = format!("observe_{index}_");
    let sanitized = sanitize_child_local_id(batch_id);
    // The lowered runtime state id appends `__run` to the child segment, so the child-local-id
    // must stay within the remaining identifier budget for the flat `<machine>.<step>.<state>`
    // contract.
    let suffix_len = 58usize.saturating_sub(prefix.len());
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

/// Thin planner op that validates canonical portfolio inputs and wires the shared runtime states.
#[derive(Clone, Default)]
pub struct PortfolioTrackerOp;

/// Returns the built-in public portfolio tracker root op.
pub fn portfolio_tracker_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(PortfolioTrackerOp) as DynOperation]
}

/// Returns the planner-internal semantic child ops used to lower `portfolio_tracker`.
pub fn portfolio_tracker_internal_ops() -> Vec<DynOperation> {
    child_ops()
}

/// Returns the full portfolio tracker operation bundle, including planner-internal child ops.
pub fn portfolio_tracker_ops() -> Vec<DynOperation> {
    let mut ops = portfolio_tracker_public_ops();
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
        OpId::must_new(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg = parse_config(op_config)?;
        let spec = compile_semantic_execution(&cfg)?;
        let mut children = vec![
            ChildOpInstance {
                child_op_local_id: ChildOpLocalId(PREPARE_EXECUTION_SOURCES_CHILD_ID.to_string()),
                op_id: OpId::must_new(PREPARE_EXECUTION_SOURCES_OP_ID.to_string()),
                op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
                op_config: prepare_sources_config(&spec)?,
            },
            ChildOpInstance {
                child_op_local_id: ChildOpLocalId(RESOLVE_SUBJECTS_CHILD_ID.to_string()),
                op_id: OpId::must_new(RESOLVE_SUBJECTS_OP_ID.to_string()),
                op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
                op_config: resolve_subjects_config(&spec)?,
            },
            ChildOpInstance {
                child_op_local_id: ChildOpLocalId(PIN_EXECUTION_VIEWS_CHILD_ID.to_string()),
                op_id: OpId::must_new(PIN_EXECUTION_VIEWS_OP_ID.to_string()),
                op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
                op_config: pin_execution_views_config(&spec)?,
            },
            ChildOpInstance {
                child_op_local_id: ChildOpLocalId(RESOLVE_VALUATION_INPUTS_CHILD_ID.to_string()),
                op_id: OpId::must_new(RESOLVE_VALUATION_INPUTS_OP_ID.to_string()),
                op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
                op_config: resolve_valuations_config(&spec)?,
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
                op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
                op_config: observe_batch_config(batch)?,
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
            op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: merge_observations_config(merge_input_ports)?,
        });
        children.push(ChildOpInstance {
            child_op_local_id: ChildOpLocalId(ASSEMBLE_SNAPSHOT_CHILD_ID.to_string()),
            op_id: OpId::must_new(ASSEMBLE_SNAPSHOT_OP_ID.to_string()),
            op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: assemble_snapshot_config(&cfg.portfolio, output_fact_key(&op_path))?,
        });
        children.push(ChildOpInstance {
            child_op_local_id: ChildOpLocalId(PROJECT_REPORT_CHILD_ID.to_string()),
            op_id: OpId::must_new(PROJECT_REPORT_OP_ID.to_string()),
            op_version: semantic_ops::INTERNAL_OP_VERSION.to_string(),
            op_config: serde_json::json!({}),
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

    fn planner_payload(
        &self,
        _op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<Option<Value>, SdkError> {
        let cfg = parse_config(op_config)?;
        let spec = compile_semantic_execution(&cfg)?;
        Ok(Some(serde_json::json!({
            (PORTFOLIO_EXECUTION_SPEC_KEY): spec
        })))
    }
}

#[cfg(test)]
#[path = "tests/portfolio_tracker_op_tests.rs"]
mod portfolio_tracker_op_tests;
