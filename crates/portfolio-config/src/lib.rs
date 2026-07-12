#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared authored and canonical config pipeline for portfolio snapshot workflows.
//!
//! This crate owns the workflow-family-specific config boundary for portfolio snapshots:
//! authored bytes parse into [`PortfolioSnapshotAuthoredConfig`], canonicalization produces
//! [`PortfolioSnapshotCanonicalConfig`].
//!
//! Planning and execution-specific built config live outside this crate so authored/canonical
//! config remains independent from runtime and scheduler APIs.
//!
//! # Examples
//!
//! ```rust
//! use mfm_portfolio_config::{
//!     canonicalize_portfolio_snapshot_authored_config, parse_portfolio_snapshot_authored_config,
//!     AuthoredConfigFormat,
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
//!                     "family": "evm",
//!                     "chain_id": 1,
//!                     "metadata": {}
//!                 }
//!             ],
//!             "wallets": [
//!                 {
//!                     "wallet_id": "wallet_main",
//!                     "subject": {
//!                         "kind": "evm_address",
//!                         "address": "0x000000000000000000000000000000000000dead"
//!                     },
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
//!                                 "unit_price_dec": "1800.00"
//!                             }
//!                         ]
//!                     },
//!                     "underlying_symbol_id": null,
//!                     "metadata": {}
//!                 }
//!             ],
//!             "metadata": {}
//!         }
//!     }"#,
//!     AuthoredConfigFormat::Json,
//! )?;
//!
//! let canonical = canonicalize_portfolio_snapshot_authored_config(authored)?;
//! assert_eq!(canonical.portfolio.portfolio_id, "portfolio_main");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::Path;

use mfm_authored_config::parse_authored_config_with_hint;
pub use mfm_authored_config::AuthoredConfigFormat;
use mfm_canonical::{CanonicalError, PlainCanonicalJsonBytes};
use mfm_ids::{ArtifactId, DigestAlgorithm};
use mfm_portfolio_model::portfolio::{
    PortfolioConfig, PortfolioConfigError, ValidatedPortfolioConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn artifact_id_for_json(value: &Value) -> Result<ArtifactId, CanonicalError> {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&value.to_string())?;
    Ok(ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

/// Human-authored portfolio snapshot config loaded from JSON or TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmConfig)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-authored-config",
    schema = "mfm.portfolio.config.snapshot_authored"
)]
pub struct PortfolioSnapshotAuthoredConfig {
    /// Portfolio-owned config surface.
    pub portfolio: PortfolioConfig,
}

/// Canonical portfolio snapshot config used for deterministic hashing and build.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, MfmConfig)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-canonical-config",
    schema = "mfm.portfolio.config.snapshot_canonical"
)]
pub struct PortfolioSnapshotCanonicalConfig {
    /// Canonical portfolio-owned config surface.
    pub portfolio: PortfolioConfig,
}

impl PortfolioSnapshotCanonicalConfig {
    /// Sorts nested collections into canonical order.
    pub fn normalize(&mut self) {
        self.portfolio.normalize();
    }

    /// Returns a normalized clone of the canonical config.
    pub fn normalized(mut self) -> Self {
        self.normalize();
        self
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

/// Stable report emitted by the portfolio config-build workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, PublicOutputs)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-build-report",
    schema = "mfm.portfolio.snapshot_build_report"
)]
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

/// Errors returned while parsing or canonicalizing portfolio snapshot config.
#[derive(Debug, thiserror::Error)]
pub enum PortfolioSnapshotConfigError {
    /// JSON authored config parsing failed.
    #[error("failed to parse portfolio authored config as json: {source}")]
    InvalidJson {
        /// Underlying parser error.
        source: serde_json::Error,
    },
    /// TOML authored config parsing failed.
    #[error("failed to parse portfolio authored config as toml: {source}")]
    InvalidToml {
        /// Underlying parser error.
        source: toml::de::Error,
    },
    /// Canonical bundle validation failed.
    #[error("invalid portfolio bundle: {0}")]
    InvalidBundle(#[from] PortfolioConfigError),
    /// Decoding an already-parsed JSON value into a typed config failed.
    #[error("failed to decode {stage}: {source}")]
    Decode {
        /// Stage being decoded.
        stage: &'static str,
        /// Underlying decode error.
        source: serde_json::Error,
    },
    /// Serializing a typed config to JSON failed.
    #[error("failed to serialize {stage}: {source}")]
    Serialize {
        /// Stage being serialized.
        stage: &'static str,
        /// Underlying serializer error.
        source: serde_json::Error,
    },
    /// Canonical JSON hashing failed.
    #[error("failed to hash {stage} as canonical json: {source}")]
    CanonicalJson {
        /// Stage being hashed.
        stage: &'static str,
        /// Underlying canonical-json error.
        source: CanonicalError,
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
    let portfolio = ValidatedPortfolioConfig::new(authored.portfolio)?.into_config();
    Ok(PortfolioSnapshotCanonicalConfig { portfolio })
}

/// Decodes canonical portfolio snapshot config.
pub fn decode_portfolio_snapshot_canonical_config(
    value: &Value,
) -> Result<PortfolioSnapshotCanonicalConfig, PortfolioSnapshotConfigError> {
    let canonical = serde_json::from_value::<PortfolioSnapshotCanonicalConfig>(value.clone())
        .map_err(|source| PortfolioSnapshotConfigError::Decode {
            stage: "canonical config",
            source,
        })?;
    let portfolio = ValidatedPortfolioConfig::new(canonical.portfolio)?.into_config();
    Ok(PortfolioSnapshotCanonicalConfig { portfolio })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
