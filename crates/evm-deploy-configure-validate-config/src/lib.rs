#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared authored and canonical config pipeline for deploy/configure/validate workflows.
//!
//! This crate establishes the typed ingress boundary for the deploy/configure/validate workflow
//! family. Transport layers parse authored JSON or TOML into
//! [`DeployConfigureValidateAuthoredConfig`], then immediately canonicalize into
//! [`DeployConfigureValidateCanonicalConfig`] before building pipeline wrappers around the result.
//!
//! The execution op boundary stays unchanged in this phase; this crate only removes ad hoc
//! transport parsing and default expansion from app/CLI glue.
//!
//! # Examples
//!
//! ```rust
//! use mfm_authored_config::AuthoredConfigFormat;
//! use mfm_evm_deploy_configure_validate_config::{
//!     canonicalize_deploy_configure_validate_authored_config,
//!     parse_deploy_configure_validate_authored_config,
//! };
//!
//! let authored = parse_deploy_configure_validate_authored_config(
//!     r#"{
//!         "deploy": {"network_id": "ethereum-mainnet"},
//!         "configure": {"network_id": "ethereum-mainnet"},
//!         "validate": {"network_id": "ethereum-mainnet"}
//!     }"#,
//!     AuthoredConfigFormat::Json,
//! )?;
//! let canonical = canonicalize_deploy_configure_validate_authored_config(authored)?;
//! assert_eq!(canonical.machine_id, "evm_deploy_configure_validate");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::Path;

use mfm_authored_config::parse_authored_config_with_hint;
pub use mfm_authored_config::AuthoredConfigFormat;
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::ArtifactId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Human-authored deploy/configure/validate config loaded from JSON or TOML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeployConfigureValidateAuthoredConfig {
    /// Optional machine id to use for the generated pipeline.
    #[serde(default)]
    pub machine_id: Option<String>,
    /// Optional version string to assign to the generated pipeline.
    #[serde(default)]
    pub pipeline_version: Option<String>,
    /// Optional pipeline input payload forwarded into the run manifest.
    #[serde(default)]
    pub input: Option<Value>,
    /// Operation config for the deploy phase.
    pub deploy: Value,
    /// Operation config for the configure phase.
    pub configure: Value,
    /// Operation config for the validate phase.
    pub validate: Value,
}

/// Canonical deploy/configure/validate config used by transport and pipeline glue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeployConfigureValidateCanonicalConfig {
    /// Machine id to use for the generated pipeline.
    pub machine_id: String,
    /// Version string to assign to the generated pipeline.
    pub pipeline_version: String,
    /// Pipeline input payload forwarded into the run manifest.
    pub input: Value,
    /// Operation config for the deploy phase.
    pub deploy: Value,
    /// Operation config for the configure phase.
    pub configure: Value,
    /// Operation config for the validate phase.
    pub validate: Value,
}

impl DeployConfigureValidateCanonicalConfig {
    /// Serializes the canonical config as a JSON value.
    pub fn to_json_value(&self) -> Result<Value, DeployConfigureValidateConfigError> {
        serde_json::to_value(self).map_err(|source| DeployConfigureValidateConfigError::Serialize {
            stage: "canonical config",
            source,
        })
    }

    /// Computes the authoritative artifact id for the canonical config JSON.
    pub fn artifact_id(&self) -> Result<ArtifactId, DeployConfigureValidateConfigError> {
        let value = self.to_json_value()?;
        artifact_id_for_json(&value).map_err(|source| {
            DeployConfigureValidateConfigError::CanonicalJson {
                stage: "canonical config",
                source,
            }
        })
    }
}

/// Errors returned while parsing or canonicalizing deploy/configure/validate config.
#[derive(Debug, Error)]
pub enum DeployConfigureValidateConfigError {
    /// JSON authored config parsing failed.
    #[error("failed to parse deploy/configure/validate authored config as json: {source}")]
    InvalidJson {
        /// Underlying parser error.
        #[source]
        source: serde_json::Error,
    },
    /// TOML authored config parsing failed.
    #[error("failed to parse deploy/configure/validate authored config as toml: {source}")]
    InvalidToml {
        /// Underlying parser error.
        #[source]
        source: toml::de::Error,
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

/// Parses authored deploy/configure/validate config using the supplied format.
pub fn parse_deploy_configure_validate_authored_config(
    raw: &str,
    format: AuthoredConfigFormat,
) -> Result<DeployConfigureValidateAuthoredConfig, DeployConfigureValidateConfigError> {
    match format {
        AuthoredConfigFormat::Json => serde_json::from_str(raw)
            .map_err(|source| DeployConfigureValidateConfigError::InvalidJson { source }),
        AuthoredConfigFormat::Toml => toml::from_str(raw)
            .map_err(|source| DeployConfigureValidateConfigError::InvalidToml { source }),
    }
}

/// Parses authored deploy/configure/validate config using an optional path hint.
pub fn parse_deploy_configure_validate_authored_config_with_hint(
    raw: &str,
    path_hint: Option<&Path>,
) -> Result<DeployConfigureValidateAuthoredConfig, DeployConfigureValidateConfigError> {
    parse_authored_config_with_hint(
        raw,
        path_hint,
        |value| parse_deploy_configure_validate_authored_config(value, AuthoredConfigFormat::Json),
        |value| parse_deploy_configure_validate_authored_config(value, AuthoredConfigFormat::Toml),
    )
}

/// Canonicalizes authored deploy/configure/validate config by materializing defaults.
pub fn canonicalize_deploy_configure_validate_authored_config(
    authored: DeployConfigureValidateAuthoredConfig,
) -> Result<DeployConfigureValidateCanonicalConfig, DeployConfigureValidateConfigError> {
    Ok(DeployConfigureValidateCanonicalConfig {
        machine_id: authored.machine_id.unwrap_or_else(default_machine_id),
        pipeline_version: authored
            .pipeline_version
            .unwrap_or_else(default_pipeline_version),
        input: authored.input.unwrap_or_else(default_empty_object),
        deploy: authored.deploy,
        configure: authored.configure,
        validate: authored.validate,
    })
}

/// Decodes JSON transport input into the canonical deploy/configure/validate config.
///
/// This preserves backward-compatible default materialization for `machine_id`,
/// `pipeline_version`, and `input` when transport payloads omit them.
pub fn decode_deploy_configure_validate_canonical_config(
    value: &Value,
) -> Result<DeployConfigureValidateCanonicalConfig, DeployConfigureValidateConfigError> {
    let authored = serde_json::from_value::<DeployConfigureValidateAuthoredConfig>(value.clone())
        .map_err(|source| DeployConfigureValidateConfigError::InvalidJson { source })?;
    canonicalize_deploy_configure_validate_authored_config(authored)
}

fn default_machine_id() -> String {
    "evm_deploy_configure_validate".to_string()
}

fn default_pipeline_version() -> String {
    "v1".to_string()
}

fn default_empty_object() -> Value {
    serde_json::json!({})
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_CONFIG: &str = r#"{
        "deploy": {"network_id": "ethereum-mainnet", "from": "0x000000000000000000000000000000000000dead"},
        "configure": {"network_id": "ethereum-mainnet", "calls": [{"function": "noop", "args": []}]},
        "validate": {"network_id": "ethereum-mainnet", "expected_chain_id": 1}
    }"#;

    const TOML_CONFIG: &str = r#"
        [deploy]
        network_id = "ethereum-mainnet"
        from = "0x000000000000000000000000000000000000dead"

        [configure]
        network_id = "ethereum-mainnet"

        [[configure.calls]]
        function = "noop"
        args = []

        [validate]
        network_id = "ethereum-mainnet"
        expected_chain_id = 1
    "#;

    #[test]
    fn json_and_toml_authoring_normalize_to_same_canonical_config() {
        let json = canonicalize_deploy_configure_validate_authored_config(
            parse_deploy_configure_validate_authored_config(
                JSON_CONFIG,
                AuthoredConfigFormat::Json,
            )
            .expect("json parse"),
        )
        .expect("json canonical");
        let toml = canonicalize_deploy_configure_validate_authored_config(
            parse_deploy_configure_validate_authored_config_with_hint(
                TOML_CONFIG,
                Some(Path::new("config.toml")),
            )
            .expect("toml parse"),
        )
        .expect("toml canonical");

        assert_eq!(json, toml);
    }

    #[test]
    fn canonicalization_materializes_pipeline_defaults() {
        let canonical = canonicalize_deploy_configure_validate_authored_config(
            parse_deploy_configure_validate_authored_config(
                JSON_CONFIG,
                AuthoredConfigFormat::Json,
            )
            .expect("json parse"),
        )
        .expect("canonical");

        assert_eq!(canonical.machine_id, "evm_deploy_configure_validate");
        assert_eq!(canonical.pipeline_version, "v1");
        assert_eq!(canonical.input, serde_json::json!({}));
    }

    #[test]
    fn canonical_artifact_id_is_deterministic_across_formats() {
        let json = canonicalize_deploy_configure_validate_authored_config(
            parse_deploy_configure_validate_authored_config(
                JSON_CONFIG,
                AuthoredConfigFormat::Json,
            )
            .expect("json parse"),
        )
        .expect("json canonical");
        let toml = canonicalize_deploy_configure_validate_authored_config(
            parse_deploy_configure_validate_authored_config_with_hint(TOML_CONFIG, None)
                .expect("toml parse"),
        )
        .expect("toml canonical");

        assert_eq!(
            json.artifact_id().expect("json artifact"),
            toml.artifact_id().expect("toml artifact")
        );
    }

    #[test]
    fn decode_json_payload_materializes_missing_defaults() {
        let canonical = decode_deploy_configure_validate_canonical_config(&serde_json::json!({
            "deploy": {"network_id": "ethereum-mainnet"},
            "configure": {"network_id": "ethereum-mainnet"},
            "validate": {"network_id": "ethereum-mainnet"}
        }))
        .expect("decode");

        assert_eq!(canonical.machine_id, "evm_deploy_configure_validate");
        assert_eq!(canonical.pipeline_version, "v1");
        assert_eq!(canonical.input, serde_json::json!({}));
    }
}
