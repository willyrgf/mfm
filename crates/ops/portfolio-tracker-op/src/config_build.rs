use std::sync::Arc;

use mfm_canonical::{CanonicalError, PlainCanonicalJsonBytes};
use mfm_ids::{ArtifactId, DigestAlgorithm};
use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_machine::plan::DependencyEdge;
use mfm_portfolio_config::{PortfolioSnapshotBuildReport, PortfolioSnapshotCanonicalConfig};
use mfm_portfolio_model::portfolio::{validate_portfolio_bundle, PortfolioConfigError};
use mfm_portfolio_plan::{
    PlanExecutionSpecError, PlanningError, PortfolioExecutionSpec, PortfolioPlanCompiler,
    PortfolioRequest,
};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    export_context_key, leaf_state_id, leaf_state_node, work_context_key, DynOperation, LeafOpSpec,
    OpInterface, Operation, PlannedOp, PlannedOpKind,
};
use mfm_state_common::errors as op_errors;
use mfm_state_common::states::publish::{WriteContextValueArtifactState, WriteJsonValueState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Public root op id for the canonical-to-built portfolio config workflow.
pub const PORTFOLIO_CONFIG_BUILD_OP_ID: &str = "portfolio_config_build";
const PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH: &str = "portfolio_config_build.main";
const PORT_CONFIG_BUILT: &str = "built_config";
const PORT_CANONICAL_ARTIFACT_ID: &str = "canonical_config_artifact_id";
const PORT_BUILT_ARTIFACT_ID: &str = "built_config_artifact_id";
const PORT_BUILD_REPORT: &str = "report";
const PORT_CANONICAL_CONFIG: &str = "canonical_config";

fn artifact_id_for_json(value: &Value) -> Result<ArtifactId, CanonicalError> {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&value.to_string())?;
    Ok(ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

fn legacy_artifact_id_string_for_json(value: &Value) -> Result<String, CanonicalError> {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&value.to_string())?;
    Ok(canonical.digest_bytes().to_string())
}

/// Built portfolio snapshot config consumed by the legacy execution op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortfolioSnapshotBuiltConfig {
    /// Canonical config from which the built config was derived.
    pub canonical: PortfolioSnapshotCanonicalConfig,
    /// Deterministic compiled semantic execution spec.
    pub execution_spec: PortfolioExecutionSpec,
}

impl PortfolioSnapshotBuiltConfig {
    /// Sorts nested collections into deterministic canonical order.
    pub fn normalize(&mut self) {
        self.canonical.normalize();
        self.execution_spec.normalize();
    }

    /// Returns a normalized clone of the built config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Serializes the built config as a JSON value.
    pub fn to_json_value(&self) -> Result<Value, PortfolioSnapshotBuildError> {
        serde_json::to_value(self).map_err(|source| PortfolioSnapshotBuildError::Serialize {
            stage: "built config",
            source,
        })
    }

    /// Computes the authoritative artifact id for the built config JSON.
    pub fn artifact_id(&self) -> Result<ArtifactId, PortfolioSnapshotBuildError> {
        let value = self.to_json_value()?;
        artifact_id_for_json(&value).map_err(|source| PortfolioSnapshotBuildError::CanonicalJson {
            stage: "built config",
            source,
        })
    }
}

/// Typed outcome produced by the portfolio config-build op.
#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioSnapshotBuildOutcome {
    /// Deterministic built config that the execute op consumes.
    pub built: PortfolioSnapshotBuiltConfig,
    /// Stable build report derived from the canonical and built config artifacts.
    pub report: PortfolioSnapshotBuildReport,
}

/// Errors returned while building or validating built portfolio config.
#[derive(Debug, Error)]
pub enum PortfolioSnapshotBuildError {
    /// Canonical bundle validation failed.
    #[error("invalid canonical portfolio bundle: {0}")]
    InvalidCanonicalConfig(#[from] PortfolioConfigError),
    /// The built-in dispatch catalog could not be constructed.
    #[error("failed to construct the default portfolio dispatch catalog: {source}")]
    DispatchCatalog {
        /// Underlying catalog construction error.
        #[source]
        source: mfm_portfolio_plan::DispatchCatalogError,
    },
    /// Semantic execution compilation failed.
    #[error("failed to compile the portfolio execution spec: {source}")]
    Planning {
        /// Underlying planning error.
        #[source]
        source: PlanningError,
    },
    /// Serializing a typed config to JSON failed.
    #[error("failed to serialize {stage}: {source}")]
    Serialize {
        /// Stage being serialized.
        stage: &'static str,
        /// Underlying serializer error.
        #[source]
        source: serde_json::Error,
    },
    /// Canonical JSON hashing failed.
    #[error("failed to hash {stage} as canonical json: {source}")]
    CanonicalJson {
        /// Stage being hashed.
        stage: &'static str,
        /// Underlying canonical-json error.
        #[source]
        source: CanonicalError,
    },
}

/// Errors returned while decoding execution-op config into built portfolio config.
#[derive(Debug, Error)]
pub enum PortfolioSnapshotExecutionConfigError {
    /// Decoding the op config as built config failed.
    #[error("portfolio execution op_config decode failed: {source}")]
    Decode {
        /// Underlying decode error.
        #[source]
        source: serde_json::Error,
    },
    /// Validating the bundled canonical config failed.
    #[error("invalid built portfolio bundle: {0}")]
    InvalidCanonicalConfig(#[from] PortfolioConfigError),
    /// The bundled execution spec was invalid.
    #[error("invalid built portfolio execution spec: {source}")]
    InvalidExecutionSpec {
        /// Underlying planning validation error.
        #[source]
        source: PlanExecutionSpecError,
    },
}

/// Builds the deterministic execution config consumed by the portfolio execution op.
pub fn build_portfolio_snapshot_config(
    canonical: PortfolioSnapshotCanonicalConfig,
) -> Result<PortfolioSnapshotBuiltConfig, PortfolioSnapshotBuildError> {
    let canonical = canonical.normalized();
    validate_portfolio_bundle(&canonical.portfolio, &canonical.valuation_source_registry)?;

    let catalog = crate::builtin_dispatch_catalog()
        .map_err(|source| PortfolioSnapshotBuildError::DispatchCatalog { source })?;
    let request = PortfolioRequest {
        portfolio: canonical.portfolio.clone(),
        valuation_source_registry: canonical.valuation_source_registry.clone(),
    };
    let execution_spec = crate::DefaultPortfolioPlanCompiler
        .compile(&request, &catalog)
        .map_err(|source| PortfolioSnapshotBuildError::Planning { source })?;

    Ok(PortfolioSnapshotBuiltConfig {
        canonical,
        execution_spec,
    }
    .normalized())
}

/// Builds the deterministic execution config plus the stable build report surface.
pub fn build_portfolio_snapshot_outcome(
    canonical: PortfolioSnapshotCanonicalConfig,
) -> Result<PortfolioSnapshotBuildOutcome, PortfolioSnapshotBuildError> {
    let built = build_portfolio_snapshot_config(canonical)?;
    let canonical_value = serde_json::to_value(&built.canonical).map_err(|source| {
        PortfolioSnapshotBuildError::Serialize {
            stage: "canonical config",
            source,
        }
    })?;
    let canonical_artifact_id =
        legacy_artifact_id_string_for_json(&canonical_value).map_err(|source| {
            PortfolioSnapshotBuildError::CanonicalJson {
                stage: "canonical config",
                source,
            }
        })?;
    let built_value = built.to_json_value()?;
    let built_artifact_id = legacy_artifact_id_string_for_json(&built_value).map_err(|source| {
        PortfolioSnapshotBuildError::CanonicalJson {
            stage: "built config",
            source,
        }
    })?;

    Ok(PortfolioSnapshotBuildOutcome {
        report: PortfolioSnapshotBuildReport {
            schema_version: PortfolioSnapshotBuildReport::SCHEMA_VERSION,
            portfolio_id: built.canonical.portfolio.portfolio_id.clone(),
            canonical_config_artifact_id: canonical_artifact_id,
            built_config_artifact_id: built_artifact_id,
            network_count: built.canonical.portfolio.networks.len() as u64,
            wallet_count: built.canonical.portfolio.wallets.len() as u64,
            observation_batch_count: built.execution_spec.observation_batches.len() as u64,
        },
        built,
    })
}

/// Decodes a built execution op config into the normalized built representation.
///
/// This is the strict execution boundary used by `portfolio_execute/v1`.
pub fn decode_portfolio_snapshot_built_config(
    value: &Value,
) -> Result<PortfolioSnapshotBuiltConfig, PortfolioSnapshotExecutionConfigError> {
    let built = serde_json::from_value::<PortfolioSnapshotBuiltConfig>(value.clone())
        .map_err(|source| PortfolioSnapshotExecutionConfigError::Decode { source })?
        .normalized();
    validate_portfolio_bundle(
        &built.canonical.portfolio,
        &built.canonical.valuation_source_registry,
    )?;
    built
        .execution_spec
        .validate()
        .map_err(|source| PortfolioSnapshotExecutionConfigError::InvalidExecutionSpec { source })?;
    Ok(built)
}

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn canonical_output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:config_build:canonical|op:{}", op_path.0))
}

fn built_output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:config_build:built|op:{}", op_path.0))
}

/// Returns the context key that stores the built portfolio config JSON.
pub fn portfolio_config_build_built_config_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_CONFIG_BUILT}"
    ))
}

/// Returns the context key that stores the canonical config artifact id.
pub fn portfolio_config_build_canonical_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_CANONICAL_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the built config artifact id.
pub fn portfolio_config_build_built_artifact_id_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_BUILT_ARTIFACT_ID}"
    ))
}

/// Returns the context key that stores the stable build report.
pub fn portfolio_config_build_report_context_key() -> ContextKey {
    ContextKey(format!(
        "{PORTFOLIO_CONFIG_BUILD_MAIN_OP_PATH}.out.{PORT_BUILD_REPORT}"
    ))
}

/// Thin planner op that turns canonical portfolio config into built config artifacts and a stable
/// build report.
#[derive(Clone, Default)]
pub struct PortfolioConfigBuildOp;

/// Returns the built-in public `portfolio_config_build` root op.
pub fn portfolio_config_build_public_ops() -> Vec<DynOperation> {
    vec![Arc::new(PortfolioConfigBuildOp) as DynOperation]
}

impl Operation for PortfolioConfigBuildOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(PORTFOLIO_CONFIG_BUILD_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        super::PORTFOLIO_PUBLIC_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let canonical = serde_json::from_value::<PortfolioSnapshotCanonicalConfig>(
            op_config.clone(),
        )
        .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;
        let outcome = build_portfolio_snapshot_outcome(canonical)
            .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;

        let write_built_sid = leaf_state_id(&op_path, "write_built_config")?;
        let write_canonical_sid = leaf_state_id(&op_path, "write_canonical_artifact_input")?;
        let write_canonical_artifact_sid = leaf_state_id(&op_path, "write_canonical_artifact")?;
        let write_built_artifact_sid = leaf_state_id(&op_path, "write_built_artifact")?;
        let write_report_sid = leaf_state_id(&op_path, "write_build_report")?;

        let built_config_key = export_context_key(PORT_CONFIG_BUILT)?;
        let canonical_artifact_id_key = export_context_key(PORT_CANONICAL_ARTIFACT_ID)?;
        let built_artifact_id_key = export_context_key(PORT_BUILT_ARTIFACT_ID)?;
        let report_key = export_context_key(PORT_BUILD_REPORT)?;
        let canonical_config_key = work_context_key(PORT_CANONICAL_CONFIG)?;

        let states = vec![
            leaf_state_node(
                &op_path,
                "write_canonical_artifact_input",
                Arc::new(WriteJsonValueState {
                    state_id: write_canonical_sid.clone(),
                    output_key: canonical_config_key.clone(),
                    value: serde_json::to_value(&outcome.built.canonical).map_err(|err| {
                        sdk_input_error("invalid_portfolio_build_config", err.to_string())
                    })?,
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_canonical_artifact",
                Arc::new(WriteContextValueArtifactState {
                    state_id: write_canonical_artifact_sid.clone(),
                    input_key: canonical_config_key,
                    fact_key: canonical_output_fact_key(&op_path),
                    output_artifact_id_key: canonical_artifact_id_key,
                    missing_input_code: "missing_portfolio_canonical_config",
                    missing_input_message:
                        "missing canonical portfolio config before artifact publication",
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_built_config",
                Arc::new(WriteJsonValueState {
                    state_id: write_built_sid.clone(),
                    output_key: built_config_key.clone(),
                    value: serde_json::to_value(&outcome.built).map_err(|err| {
                        sdk_input_error("invalid_portfolio_build_config", err.to_string())
                    })?,
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_built_artifact",
                Arc::new(WriteContextValueArtifactState {
                    state_id: write_built_artifact_sid.clone(),
                    input_key: built_config_key.clone(),
                    fact_key: built_output_fact_key(&op_path),
                    output_artifact_id_key: built_artifact_id_key,
                    missing_input_code: "missing_portfolio_built_config",
                    missing_input_message:
                        "missing built portfolio config before artifact publication",
                }),
            )?,
            leaf_state_node(
                &op_path,
                "write_build_report",
                Arc::new(WriteJsonValueState {
                    state_id: write_report_sid.clone(),
                    output_key: report_key,
                    value: serde_json::to_value(&outcome.report).map_err(|err| {
                        sdk_input_error("invalid_portfolio_build_config", err.to_string())
                    })?,
                }),
            )?,
        ];

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![
                    PortKey(PORT_CONFIG_BUILT.to_string()),
                    PortKey(PORT_CANONICAL_ARTIFACT_ID.to_string()),
                    PortKey(PORT_BUILT_ARTIFACT_ID.to_string()),
                    PortKey(PORT_BUILD_REPORT.to_string()),
                ],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states,
                edges: vec![
                    DependencyEdge {
                        from: write_canonical_sid,
                        to: write_canonical_artifact_sid.clone(),
                    },
                    DependencyEdge {
                        from: write_built_sid,
                        to: write_built_artifact_sid.clone(),
                    },
                    DependencyEdge {
                        from: write_canonical_artifact_sid,
                        to: write_report_sid.clone(),
                    },
                    DependencyEdge {
                        from: write_built_artifact_sid,
                        to: write_report_sid,
                    },
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
        let canonical = serde_json::from_value::<PortfolioSnapshotCanonicalConfig>(
            op_config.clone(),
        )
        .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;
        let outcome = build_portfolio_snapshot_outcome(canonical)
            .map_err(|err| sdk_input_error("invalid_portfolio_build_config", err.to_string()))?;
        Ok(Some(serde_json::json!({
            "built_config": outcome.built
        })))
    }
}
