#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared authored, canonical, and built config pipeline for portfolio snapshot workflows.
//!
//! This crate owns the workflow-family-specific config boundary for portfolio snapshots:
//! authored bytes parse into [`PortfolioSnapshotAuthoredConfig`], canonicalization produces
//! [`PortfolioSnapshotCanonicalConfig`], and the pure build step produces
//! [`PortfolioSnapshotBuiltConfig`].
//!
//! The portfolio execution op consumes built config, while transport layers stay responsible for
//! file reading and output rendering.
//!
//! # Examples
//!
//! ```rust
//! use mfm_portfolio_config::{
//!     build_portfolio_snapshot_config, canonicalize_portfolio_snapshot_authored_config,
//!     parse_portfolio_snapshot_authored_config, AuthoredConfigFormat,
//! };
//!
//! let authored = parse_portfolio_snapshot_authored_config(
//!     r#"{
//!         "portfolio": {
//!             "portfolio_id": "portfolio_main",
//!             "quote_codes": ["USD"],
//!             "networks": [
//!                 {
//!                     "network_id": "ethereum-mainnet",
//!                     "chain_id": 1,
//!                     "metadata": {}
//!                 }
//!             ],
//!             "wallets": [
//!                 {
//!                     "wallet_id": "wallet_main",
//!                     "address": "0x000000000000000000000000000000000000dead",
//!                     "implementation": { "kind": "address_only" },
//!                     "network_id": "ethereum-mainnet",
//!                     "symbol_ids": ["eth.native.ethereum-mainnet"],
//!                     "metadata": {}
//!                 }
//!             ],
//!             "symbol_configs": [
//!                 {
//!                     "symbol_id": "eth.native.ethereum-mainnet",
//!                     "display_symbol": "ETH",
//!                     "kind": "native_balance",
//!                     "role": "native",
//!                     "network_id": "ethereum-mainnet",
//!                     "protocol": null,
//!                     "balance_reader": { "kind": "native_balance" },
//!                     "valuation": {
//!                         "quotes": [
//!                             {
//!                                 "quote": "USD",
//!                                 "priced_symbol_id": "eth.native.ethereum-mainnet",
//!                                 "reader": {
//!                                     "kind": "fixed_unit_price",
//!                                     "unit_price_dec": "1800.00"
//!                                 }
//!                             }
//!                         ]
//!                     },
//!                     "decimals": 18,
//!                     "underlying_symbol_id": null,
//!                     "metadata": {}
//!                 }
//!             ],
//!             "metadata": {}
//!         },
//!         "valuation_source_registry": { "sources": [] }
//!     }"#,
//!     AuthoredConfigFormat::Json,
//! )?;
//!
//! let canonical = canonicalize_portfolio_snapshot_authored_config(authored)?;
//! let built = build_portfolio_snapshot_config(canonical)?;
//! assert_eq!(built.canonical.portfolio.portfolio_id, "portfolio_main");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::Path;

use mfm_authored_config::parse_authored_config_with_hint;
pub use mfm_authored_config::AuthoredConfigFormat;
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::ArtifactId;
use mfm_state_portfolio::model::{
    validate_portfolio_bundle, PortfolioConfig, PortfolioConfigError,
};
use mfm_state_portfolio::plan::{
    PlanExecutionSpecError, PlanningError, PortfolioExecutionSpec, PortfolioPlanCompiler,
    PortfolioRequest,
};
use mfm_state_symbol::model::ValuationSourceRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

mod defaults;

pub use defaults::{builtin_dispatch_catalog, DefaultPortfolioPlanCompiler};

/// Human-authored portfolio snapshot config loaded from JSON or TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortfolioSnapshotAuthoredConfig {
    /// Portfolio-owned config surface.
    pub portfolio: PortfolioConfig,
    /// Sibling valuation source registry surface.
    pub valuation_source_registry: ValuationSourceRegistry,
}

/// Canonical portfolio snapshot config used for deterministic hashing and build.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortfolioSnapshotCanonicalConfig {
    /// Canonical portfolio-owned config surface.
    pub portfolio: PortfolioConfig,
    /// Canonical valuation source registry surface.
    pub valuation_source_registry: ValuationSourceRegistry,
}

impl PortfolioSnapshotCanonicalConfig {
    /// Sorts nested collections into canonical order.
    pub fn normalize(&mut self) {
        self.portfolio.normalize();
        self.valuation_source_registry.normalize();
    }

    /// Returns a normalized clone of the canonical config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
    }

    /// Converts the canonical config into the plan-compiler request shape.
    pub fn as_plan_request(&self) -> PortfolioRequest {
        PortfolioRequest {
            portfolio: self.portfolio.clone(),
            valuation_source_registry: self.valuation_source_registry.clone(),
        }
    }

    /// Serializes the canonical config as a JSON value.
    pub fn to_json_value(&self) -> Result<Value, PortfolioSnapshotConfigError> {
        serde_json::to_value(self).map_err(|source| PortfolioSnapshotConfigError::Serialize {
            stage: "canonical config",
            source,
        })
    }

    /// Computes the authoritative artifact id for the canonical config JSON.
    pub fn artifact_id(&self) -> Result<ArtifactId, PortfolioSnapshotConfigError> {
        let value = self.to_json_value()?;
        artifact_id_for_json(&value).map_err(|source| PortfolioSnapshotConfigError::CanonicalJson {
            stage: "canonical config",
            source,
        })
    }
}

/// Built portfolio snapshot config consumed by the execution op.
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

/// Stable report emitted by the portfolio config-build workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioSnapshotBuildReport {
    /// Schema version for this report surface.
    pub schema_version: u32,
    /// Portfolio identifier carried through from the canonical config.
    pub portfolio_id: String,
    /// Content-addressed canonical config artifact id.
    pub canonical_config_artifact_id: String,
    /// Content-addressed built config artifact id.
    pub built_config_artifact_id: String,
    /// Number of declared networks in the canonical bundle.
    pub network_count: u64,
    /// Number of declared wallets in the canonical bundle.
    pub wallet_count: u64,
    /// Number of compiled observation batches in the built execution spec.
    pub observation_batch_count: u64,
}

impl PortfolioSnapshotBuildReport {
    /// Current schema version for the build report surface.
    pub const SCHEMA_VERSION: u32 = 1;
}

/// Typed outcome produced by the pure portfolio config-build step.
#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioSnapshotBuildOutcome {
    /// Deterministic built config that the execute op consumes.
    pub built: PortfolioSnapshotBuiltConfig,
    /// Stable build report derived from the canonical and built config artifacts.
    pub report: PortfolioSnapshotBuildReport,
}

/// Errors returned while parsing or canonicalizing portfolio snapshot config.
#[derive(Debug, Error)]
pub enum PortfolioSnapshotConfigError {
    /// JSON authored config parsing failed.
    #[error("failed to parse portfolio authored config as json: {source}")]
    InvalidJson {
        /// Underlying parser error.
        #[source]
        source: serde_json::Error,
    },
    /// TOML authored config parsing failed.
    #[error("failed to parse portfolio authored config as toml: {source}")]
    InvalidToml {
        /// Underlying parser error.
        #[source]
        source: toml::de::Error,
    },
    /// Canonical bundle validation failed.
    #[error("invalid portfolio bundle: {0}")]
    InvalidBundle(#[from] PortfolioConfigError),
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
        source: CanonicalJsonError,
    },
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
        source: mfm_state_portfolio::plan::DispatchCatalogError,
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
        source: CanonicalJsonError,
    },
}

/// Errors returned while decoding execution-op config into built portfolio config.
#[derive(Debug, Error)]
pub enum PortfolioSnapshotExecutionConfigError {
    /// Decoding the op config as a canonical or built config failed.
    #[error("portfolio execution op_config decode failed: {source}")]
    Decode {
        /// Underlying decode error.
        #[source]
        source: serde_json::Error,
    },
    /// Building from a canonical config failed.
    #[error(transparent)]
    Build(#[from] PortfolioSnapshotBuildError),
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

/// Parses authored portfolio snapshot config using the supplied format.
pub fn parse_portfolio_snapshot_authored_config(
    raw: &str,
    format: AuthoredConfigFormat,
) -> Result<PortfolioSnapshotAuthoredConfig, PortfolioSnapshotConfigError> {
    match format {
        AuthoredConfigFormat::Json => serde_json::from_str(raw)
            .map_err(|source| PortfolioSnapshotConfigError::InvalidJson { source }),
        AuthoredConfigFormat::Toml => toml::from_str(raw)
            .map_err(|source| PortfolioSnapshotConfigError::InvalidToml { source }),
    }
}

/// Parses authored portfolio snapshot config using an optional path hint.
///
/// When no recognized extension is present, the parser first sniffs the leading non-whitespace
/// character and falls back to the other format if the first parse attempt fails.
pub fn parse_portfolio_snapshot_authored_config_with_hint(
    raw: &str,
    path_hint: Option<&Path>,
) -> Result<PortfolioSnapshotAuthoredConfig, PortfolioSnapshotConfigError> {
    parse_authored_config_with_hint(
        raw,
        path_hint,
        |value| parse_portfolio_snapshot_authored_config(value, AuthoredConfigFormat::Json),
        |value| parse_portfolio_snapshot_authored_config(value, AuthoredConfigFormat::Toml),
    )
}

/// Canonicalizes authored portfolio config into the normalized canonical representation.
pub fn canonicalize_portfolio_snapshot_authored_config(
    authored: PortfolioSnapshotAuthoredConfig,
) -> Result<PortfolioSnapshotCanonicalConfig, PortfolioSnapshotConfigError> {
    let canonical = PortfolioSnapshotCanonicalConfig {
        portfolio: authored.portfolio,
        valuation_source_registry: authored.valuation_source_registry,
    }
    .normalized();
    validate_portfolio_bundle(&canonical.portfolio, &canonical.valuation_source_registry)?;
    Ok(canonical)
}

/// Builds the deterministic execution config consumed by the portfolio execution op.
pub fn build_portfolio_snapshot_config(
    canonical: PortfolioSnapshotCanonicalConfig,
) -> Result<PortfolioSnapshotBuiltConfig, PortfolioSnapshotBuildError> {
    let canonical = canonical.normalized();
    validate_portfolio_bundle(&canonical.portfolio, &canonical.valuation_source_registry)?;

    let catalog = builtin_dispatch_catalog()
        .map_err(|source| PortfolioSnapshotBuildError::DispatchCatalog { source })?;
    let execution_spec = DefaultPortfolioPlanCompiler
        .compile(&canonical.as_plan_request(), &catalog)
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
    let canonical_artifact_id = artifact_id_for_json(&canonical_value).map_err(|source| {
        PortfolioSnapshotBuildError::CanonicalJson {
            stage: "canonical config",
            source,
        }
    })?;
    let built_artifact_id = built.artifact_id()?;

    Ok(PortfolioSnapshotBuildOutcome {
        report: PortfolioSnapshotBuildReport {
            schema_version: PortfolioSnapshotBuildReport::SCHEMA_VERSION,
            portfolio_id: built.canonical.portfolio.portfolio_id.clone(),
            canonical_config_artifact_id: canonical_artifact_id.0,
            built_config_artifact_id: built_artifact_id.0,
            network_count: built.canonical.portfolio.networks.len() as u64,
            wallet_count: built.canonical.portfolio.wallets.len() as u64,
            observation_batch_count: built.execution_spec.observation_batches.len() as u64,
        },
        built,
    })
}

/// Decodes canonical portfolio snapshot config.
pub fn decode_portfolio_snapshot_canonical_config(
    value: &Value,
) -> Result<PortfolioSnapshotCanonicalConfig, PortfolioSnapshotExecutionConfigError> {
    serde_json::from_value::<PortfolioSnapshotCanonicalConfig>(value.clone())
        .map_err(|source| PortfolioSnapshotExecutionConfigError::Decode { source })
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use serde_json::json;

    fn sample_authored_config() -> PortfolioSnapshotAuthoredConfig {
        serde_json::from_value(sample_request_json()).expect("sample authored config")
    }

    fn sample_request_json() -> Value {
        json!({
            "portfolio": {
                "portfolio_id": "portfolio_main",
                "quote_codes": ["USD"],
                "networks": [
                    {
                        "network_id": "ethereum-mainnet",
                        "chain_id": 1,
                        "metadata": {}
                    }
                ],
                "wallets": [
                    {
                        "wallet_id": "wallet_main",
                        "address": "0x000000000000000000000000000000000000dead",
                        "implementation": {
                            "kind": "address_only"
                        },
                        "network_id": "ethereum-mainnet",
                        "symbol_ids": ["eth.native.ethereum-mainnet"],
                        "metadata": {}
                    }
                ],
                "symbol_configs": [
                    {
                        "symbol_id": "eth.native.ethereum-mainnet",
                        "display_symbol": "ETH",
                        "kind": "native_balance",
                        "role": "native",
                        "network_id": "ethereum-mainnet",
                        "protocol": null,
                        "balance_reader": {
                            "kind": "native_balance"
                        },
                        "valuation": {
                            "quotes": [
                                {
                                    "quote": "USD",
                                    "priced_symbol_id": "eth.native.ethereum-mainnet",
                                    "reader": {
                                        "kind": "fixed_unit_price",
                                        "unit_price_dec": "1800.00"
                                    }
                                }
                            ]
                        },
                        "decimals": 18,
                        "underlying_symbol_id": null,
                        "metadata": {}
                    }
                ],
                "metadata": {}
            },
            "valuation_source_registry": {
                "sources": []
            }
        })
    }

    #[test]
    fn authored_json_and_toml_canonicalize_to_same_config_and_hash() {
        let json_authored = parse_portfolio_snapshot_authored_config(
            &sample_request_json().to_string(),
            AuthoredConfigFormat::Json,
        )
        .expect("json authored config");
        let toml_authored = parse_portfolio_snapshot_authored_config_with_hint(
            &toml::to_string(&sample_authored_config()).expect("toml"),
            Some(Path::new("config.toml")),
        )
        .expect("toml authored config");

        let json_canonical =
            canonicalize_portfolio_snapshot_authored_config(json_authored).expect("json canonical");
        let toml_canonical =
            canonicalize_portfolio_snapshot_authored_config(toml_authored).expect("toml canonical");

        assert_eq!(json_canonical, toml_canonical);
        assert_eq!(
            json_canonical.artifact_id().expect("json canonical hash"),
            toml_canonical.artifact_id().expect("toml canonical hash")
        );
    }

    #[test]
    fn build_is_deterministic_across_authoring_order() {
        let mut authored = sample_authored_config();
        authored.portfolio.wallets.reverse();
        authored.portfolio.symbol_configs.reverse();
        authored.portfolio.quote_codes.reverse();

        let left = build_portfolio_snapshot_config(
            canonicalize_portfolio_snapshot_authored_config(sample_authored_config())
                .expect("left canonical"),
        )
        .expect("left build");
        let right = build_portfolio_snapshot_config(
            canonicalize_portfolio_snapshot_authored_config(authored).expect("right canonical"),
        )
        .expect("right build");

        assert_eq!(left, right);
        assert_eq!(
            left.artifact_id().expect("left built hash"),
            right.artifact_id().expect("right built hash")
        );
    }

    #[test]
    fn decode_canonical_config_accepts_canonical_shape() {
        let canonical = canonicalize_portfolio_snapshot_authored_config(sample_authored_config())
            .expect("canonical");
        let decoded = decode_portfolio_snapshot_canonical_config(
            &serde_json::to_value(&canonical).expect("canonical json"),
        )
        .expect("decode canonical");

        assert_eq!(decoded, canonical);
    }

    #[test]
    fn decode_built_config_rejects_legacy_canonical_shape() {
        let canonical = canonicalize_portfolio_snapshot_authored_config(sample_authored_config())
            .expect("canonical");

        let err = decode_portfolio_snapshot_built_config(
            &serde_json::to_value(&canonical).expect("canonical json"),
        )
        .expect_err("legacy canonical shape should not decode as built config");

        assert!(matches!(
            err,
            PortfolioSnapshotExecutionConfigError::Decode { .. }
        ));
    }

    #[test]
    fn build_outcome_report_is_deterministic_and_matches_artifact_ids() {
        let canonical = canonicalize_portfolio_snapshot_authored_config(sample_authored_config())
            .expect("canonical");

        let left = build_portfolio_snapshot_outcome(canonical.clone()).expect("left outcome");
        let right = build_portfolio_snapshot_outcome(canonical).expect("right outcome");

        assert_eq!(left, right);
        assert_eq!(
            left.report.canonical_config_artifact_id,
            left.built
                .canonical
                .artifact_id()
                .expect("canonical artifact id")
                .0
        );
        assert_eq!(
            left.report.built_config_artifact_id,
            left.built.artifact_id().expect("built artifact id").0
        );
    }
}
