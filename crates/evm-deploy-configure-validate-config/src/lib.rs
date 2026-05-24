#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared authored and canonical config pipeline for deploy/configure/validate workflows.
//!
//! This crate establishes the typed ingress boundary for the deploy/configure/validate workflow
//! family. Transport layers parse authored JSON or TOML into
//! [`DeployConfigureValidateAuthoredConfig`], then immediately canonicalize into
//! [`DeployConfigureValidateCanonicalConfig`] before building pipeline wrappers around the result.
//! Internal MFM build workflows then lower the canonical form into
//! [`DeployConfigureValidateBuiltConfig`] for execution ops.
//!
//! The semantic lowering in this workflow family is intentionally small today, but the explicit
//! built-config boundary keeps authored/canonical transport concerns separate from execution
//! concerns and preserves a stable place for future normalization.
//!
//! # Examples
//!
//! ```rust
//! use mfm_authored_config::AuthoredConfigFormat;
//! use mfm_evm_deploy_configure_validate_config::{
//!     build_deploy_configure_validate_outcome, canonicalize_deploy_configure_validate_authored_config,
//!     parse_deploy_configure_validate_authored_config,
//! };
//!
//! let authored = parse_deploy_configure_validate_authored_config(
//!     r#"{
//!         "deploy": {
//!             "network_id": "ethereum-mainnet",
//!             "from": "0x000000000000000000000000000000000000dead",
//!             "signing_key_env": "MFM_DEPLOYER_KEY"
//!         },
//!         "configure": {
//!             "network_id": "ethereum-mainnet",
//!             "from": "0x000000000000000000000000000000000000dead",
//!             "signing_key_env": "MFM_DEPLOYER_KEY",
//!             "calls": []
//!         },
//!         "validate": {
//!             "network_id": "ethereum-mainnet",
//!             "expected_chain_id": 1
//!         }
//!     }"#,
//!     AuthoredConfigFormat::Json,
//! )?;
//! let canonical = canonicalize_deploy_configure_validate_authored_config(authored)?;
//! let built = build_deploy_configure_validate_outcome(canonical)?.built;
//! assert_eq!(built.canonical.machine_id, "evm_deploy_configure_validate");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::Path;

use mfm_authored_config::parse_authored_config_with_hint;
pub use mfm_authored_config::AuthoredConfigFormat;
use mfm_canonical::{CanonicalError, PlainCanonicalJsonBytes};
use mfm_evm_dcv_model::{
    AbiArgumentValue, ConfigureCallConfig, ContractArtifactConfig, EventAssertionConfig,
    ExistingConfiguredContractRef, ReadAssertionConfig,
};
use mfm_ids::{ArtifactId, DigestAlgorithm};
use mfm_program_derive::{MfmConfig, MfmValue};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

fn artifact_id_for_json(value: &Value) -> Result<ArtifactId, CanonicalError> {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&value.to_string())?;
    Ok(ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

fn legacy_artifact_store_id(id: &ArtifactId) -> String {
    id.digest().to_string()
}

fn canonical_json_text_from_str(input: &str) -> Result<String, String> {
    PlainCanonicalJsonBytes::from_json_str(input)
        .map(|canonical| canonical.to_string())
        .map_err(|error| error.to_string())
}

fn canonical_json_text_from_value(value: &Value) -> Result<String, String> {
    canonical_json_text_from_str(&value.to_string())
}

fn legacy_or_typed_json_text(value: Value) -> Result<String, String> {
    match value {
        Value::Object(mut object) if object.len() == 1 && object.contains_key("json_text") => {
            match object.remove("json_text") {
                Some(Value::String(json_text)) => canonical_json_text_from_str(&json_text),
                Some(_) => Err("json_text must be a string".to_string()),
                None => unreachable!("object contains json_text"),
            }
        }
        other => canonical_json_text_from_value(&other),
    }
}

fn deserialize_json_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    legacy_or_typed_json_text(value).map_err(de::Error::custom)
}

fn default_control_scope() -> String {
    "shared".to_string()
}

fn default_poll_interval_ms() -> u64 {
    500
}

fn default_max_receipt_polls() -> u64 {
    120
}

fn default_configure_tx_hashes_export_key() -> String {
    "configure_tx_hashes".to_string()
}

fn default_configure_receipts_export_key() -> String {
    "configure_receipts".to_string()
}

fn default_require_client_substring() -> String {
    "reth".to_string()
}

/// Typed workflow input JSON carried by deploy/configure/validate run manifests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "workflow-input",
    schema = "mfm.evm.dcv.value.workflow_input"
)]
pub struct DeployConfigureValidateInput {
    /// Canonical JSON text for the manifest input payload.
    pub json_text: String,
}

impl DeployConfigureValidateInput {
    /// Builds a typed workflow input from authored JSON text.
    pub fn from_json_text(json_text: impl AsRef<str>) -> Result<Self, String> {
        Ok(Self {
            json_text: canonical_json_text_from_str(json_text.as_ref())?,
        })
    }

    /// Builds a typed workflow input from a JSON value.
    pub fn from_json_value(value: &Value) -> Result<Self, String> {
        Ok(Self {
            json_text: legacy_or_typed_json_text(value.clone())?,
        })
    }

    /// Returns the canonical JSON text.
    pub fn json_text(&self) -> &str {
        &self.json_text
    }

    /// Parses the workflow input into a JSON value for run manifests.
    pub fn to_json_value(&self) -> Result<Value, DeployConfigureValidateConfigError> {
        serde_json::from_str(&self.json_text)
            .map_err(|source| DeployConfigureValidateConfigError::InvalidJson { source })
    }
}

impl<'de> Deserialize<'de> for DeployConfigureValidateInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            json_text: deserialize_json_text(deserializer)?,
        })
    }
}

/// Typed authored and canonical deploy-phase config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "deploy-config",
    schema = "mfm.evm.dcv.config.deploy"
)]
pub struct DeployConfigureValidateDeployConfig {
    /// Inline contract artifact used by the certified typed workflow.
    pub artifact: Option<ContractArtifactConfig>,
    /// Stable network identifier targeted by the managed RPC calls.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Deployer address or sender address.
    pub from: String,
    /// Constructor arguments passed during deployment.
    pub constructor_args: Vec<AbiArgumentValue>,
    /// Optional deployment value expressed in wei.
    pub value_wei: Option<String>,
    /// Optional environment variable name used for local signing.
    pub signing_key_env: Option<String>,
    /// Delay between receipt polls in milliseconds.
    pub poll_interval_ms: u64,
    /// Maximum number of receipt polls before timing out.
    pub max_receipt_polls: u64,
}

impl<'de> Deserialize<'de> for DeployConfigureValidateDeployConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawDeployConfig {
            #[serde(default)]
            artifact: Option<ContractArtifactConfig>,
            network_id: String,
            #[serde(default = "default_control_scope")]
            control_scope: String,
            from: String,
            #[serde(default)]
            constructor_args: Vec<AbiArgumentValue>,
            #[serde(default)]
            value_wei: Option<String>,
            #[serde(default)]
            signing_key_env: Option<String>,
            #[serde(default = "default_poll_interval_ms")]
            poll_interval_ms: u64,
            #[serde(default = "default_max_receipt_polls")]
            max_receipt_polls: u64,
        }

        let raw = RawDeployConfig::deserialize(deserializer)?;
        Ok(Self {
            artifact: raw.artifact,
            network_id: raw.network_id,
            control_scope: raw.control_scope,
            from: raw.from,
            constructor_args: raw.constructor_args,
            value_wei: raw.value_wei,
            signing_key_env: raw.signing_key_env,
            poll_interval_ms: raw.poll_interval_ms,
            max_receipt_polls: raw.max_receipt_polls,
        })
    }
}

/// Typed authored and canonical configure-phase config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-config",
    schema = "mfm.evm.dcv.config.configure"
)]
pub struct DeployConfigureValidateConfigureConfig {
    /// Inline contract artifact used by the certified typed workflow.
    pub artifact: Option<ContractArtifactConfig>,
    /// Stable network identifier targeted by the managed RPC calls.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Sender address used for configuration transactions.
    pub from: String,
    /// Optional environment variable name used for local signing.
    pub signing_key_env: Option<String>,
    /// Calls to execute against the deployed contract.
    pub calls: Vec<ConfigureCallConfig>,
    /// Export key used by the configure phase for transaction hashes.
    pub tx_hashes_export_key: String,
    /// Export key used by the configure phase for transaction receipts.
    pub receipts_export_key: String,
    /// Delay between receipt polls in milliseconds.
    pub poll_interval_ms: u64,
    /// Maximum number of receipt polls before timing out.
    pub max_receipt_polls: u64,
}

impl<'de> Deserialize<'de> for DeployConfigureValidateConfigureConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawConfigureConfig {
            #[serde(default)]
            artifact: Option<ContractArtifactConfig>,
            network_id: String,
            #[serde(default = "default_control_scope")]
            control_scope: String,
            from: String,
            #[serde(default)]
            signing_key_env: Option<String>,
            #[serde(default)]
            calls: Vec<ConfigureCallConfig>,
            #[serde(default = "default_configure_tx_hashes_export_key")]
            tx_hashes_export_key: String,
            #[serde(default = "default_configure_receipts_export_key")]
            receipts_export_key: String,
            #[serde(default = "default_poll_interval_ms")]
            poll_interval_ms: u64,
            #[serde(default = "default_max_receipt_polls")]
            max_receipt_polls: u64,
        }

        let raw = RawConfigureConfig::deserialize(deserializer)?;
        Ok(Self {
            artifact: raw.artifact,
            network_id: raw.network_id,
            control_scope: raw.control_scope,
            from: raw.from,
            signing_key_env: raw.signing_key_env,
            calls: raw.calls,
            tx_hashes_export_key: raw.tx_hashes_export_key,
            receipts_export_key: raw.receipts_export_key,
            poll_interval_ms: raw.poll_interval_ms,
            max_receipt_polls: raw.max_receipt_polls,
        })
    }
}

/// Typed authored and canonical validate-phase config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "validate-config",
    schema = "mfm.evm.dcv.config.validate"
)]
pub struct DeployConfigureValidateValidateConfig {
    /// Inline contract artifact used by the certified typed workflow.
    pub artifact: Option<ContractArtifactConfig>,
    /// Stable network identifier targeted by the managed RPC calls.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Expected chain id for the connected RPC endpoint.
    pub expected_chain_id: u64,
    /// Substring that must appear in `web3_clientVersion`.
    pub require_client_substring: String,
    /// Read assertions evaluated with `eth_call`.
    pub read_assertions: Vec<ReadAssertionConfig>,
    /// Event assertions evaluated with `eth_getLogs`.
    pub event_assertions: Vec<EventAssertionConfig>,
}

impl<'de> Deserialize<'de> for DeployConfigureValidateValidateConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawValidateConfig {
            #[serde(default)]
            artifact: Option<ContractArtifactConfig>,
            network_id: String,
            #[serde(default = "default_control_scope")]
            control_scope: String,
            expected_chain_id: u64,
            #[serde(default = "default_require_client_substring")]
            require_client_substring: String,
            #[serde(default)]
            read_assertions: Vec<ReadAssertionConfig>,
            #[serde(default)]
            event_assertions: Vec<EventAssertionConfig>,
        }

        let raw = RawValidateConfig::deserialize(deserializer)?;
        Ok(Self {
            artifact: raw.artifact,
            network_id: raw.network_id,
            control_scope: raw.control_scope,
            expected_chain_id: raw.expected_chain_id,
            require_client_substring: raw.require_client_substring,
            read_assertions: raw.read_assertions,
            event_assertions: raw.event_assertions,
        })
    }
}

/// Typed validation config for a pre-existing configured deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "existing-configured-contract-validation-config",
    schema = "mfm.evm.dcv.config.existing_configured_contract_validation"
)]
pub struct ExistingConfiguredContractValidationConfig {
    /// Explicit configured-contract reference imported into the validation workflow.
    pub configured_contract: ExistingConfiguredContractRef,
    /// Validation checks to run against the configured contract reference.
    pub validate: DeployConfigureValidateValidateConfig,
}

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
    pub deploy: DeployConfigureValidateDeployConfig,
    /// Operation config for the configure phase.
    pub configure: DeployConfigureValidateConfigureConfig,
    /// Operation config for the validate phase.
    pub validate: DeployConfigureValidateValidateConfig,
}

/// Canonical deploy/configure/validate config used by transport and pipeline glue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "canonical-config",
    schema = "mfm.evm.dcv.config.canonical"
)]
pub struct DeployConfigureValidateCanonicalConfig {
    /// Machine id to use for the generated pipeline.
    pub machine_id: String,
    /// Version string to assign to the generated pipeline.
    pub pipeline_version: String,
    /// Pipeline input payload forwarded into the run manifest.
    pub input: DeployConfigureValidateInput,
    /// Operation config for the deploy phase.
    pub deploy: DeployConfigureValidateDeployConfig,
    /// Operation config for the configure phase.
    pub configure: DeployConfigureValidateConfigureConfig,
    /// Operation config for the validate phase.
    pub validate: DeployConfigureValidateValidateConfig,
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

/// Built execution config for deploy/configure/validate workflows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "built-config",
    schema = "mfm.evm.dcv.config.built"
)]
pub struct DeployConfigureValidateBuiltConfig {
    /// Canonical config that produced this built execution payload.
    pub canonical: DeployConfigureValidateCanonicalConfig,
    /// Execution payload consumed by the strict execution op boundary.
    pub execution: DeployConfigureValidateExecutionConfig,
}

impl DeployConfigureValidateBuiltConfig {
    /// Serializes the built config as a JSON value.
    pub fn to_json_value(&self) -> Result<Value, DeployConfigureValidateConfigError> {
        serde_json::to_value(self).map_err(|source| DeployConfigureValidateConfigError::Serialize {
            stage: "built config",
            source,
        })
    }

    /// Computes the authoritative artifact id for the built config JSON.
    pub fn artifact_id(&self) -> Result<ArtifactId, DeployConfigureValidateConfigError> {
        let value = self.to_json_value()?;
        artifact_id_for_json(&value).map_err(|source| {
            DeployConfigureValidateConfigError::CanonicalJson {
                stage: "built config",
                source,
            }
        })
    }
}

/// Execution payload consumed by the strict deploy/configure/validate execution op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "execution-config",
    schema = "mfm.evm.dcv.config.execution"
)]
pub struct DeployConfigureValidateExecutionConfig {
    /// Deploy sub-op config.
    pub deploy: DeployConfigureValidateDeployConfig,
    /// Configure sub-op config.
    pub configure: DeployConfigureValidateConfigureConfig,
    /// Validate sub-op config.
    pub validate: DeployConfigureValidateValidateConfig,
}

/// Stable build report emitted by deploy/configure/validate config-build workflows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "build-report",
    schema = "mfm.evm.dcv.value.build_report"
)]
pub struct DeployConfigureValidateBuildReport {
    /// Machine id attached to the canonical config.
    pub machine_id: String,
    /// Pipeline version attached to the canonical config.
    pub pipeline_version: String,
    /// Content-addressed artifact id for the canonical config.
    pub canonical_config_artifact_id: String,
    /// Content-addressed artifact id for the built config.
    pub built_config_artifact_id: String,
    /// Number of sequential workflow phases in the lowered execution config.
    pub phase_count: u64,
}

/// Pure build outcome for deploy/configure/validate config workflows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployConfigureValidateBuildOutcome {
    /// Lowered built config payload.
    pub built: DeployConfigureValidateBuiltConfig,
    /// Stable build report derived from the canonical and built payloads.
    pub report: DeployConfigureValidateBuildReport,
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
        source: CanonicalError,
    },
    /// Canonical JSON text validation failed.
    #[error("invalid {stage} JSON text: {message}")]
    InvalidJsonText {
        /// Stage being validated.
        stage: &'static str,
        /// Stable validation message.
        message: String,
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
        input: DeployConfigureValidateInput::from_json_value(
            &authored.input.unwrap_or_else(default_empty_object),
        )
        .map_err(
            |message| DeployConfigureValidateConfigError::InvalidJsonText {
                stage: "workflow input",
                message,
            },
        )?,
        deploy: authored.deploy,
        configure: authored.configure,
        validate: authored.validate,
    })
}

/// Builds canonical deploy/configure/validate config into the explicit execution payload.
pub fn build_deploy_configure_validate_config(
    canonical: DeployConfigureValidateCanonicalConfig,
) -> Result<DeployConfigureValidateBuiltConfig, DeployConfigureValidateConfigError> {
    Ok(DeployConfigureValidateBuiltConfig {
        execution: DeployConfigureValidateExecutionConfig {
            deploy: canonical.deploy.clone(),
            configure: canonical.configure.clone(),
            validate: canonical.validate.clone(),
        },
        canonical,
    })
}

/// Builds canonical deploy/configure/validate config and derives the stable build report.
pub fn build_deploy_configure_validate_outcome(
    canonical: DeployConfigureValidateCanonicalConfig,
) -> Result<DeployConfigureValidateBuildOutcome, DeployConfigureValidateConfigError> {
    let built = build_deploy_configure_validate_config(canonical)?;
    let canonical_config_artifact_id = legacy_artifact_store_id(&built.canonical.artifact_id()?);
    let built_config_artifact_id = legacy_artifact_store_id(&built.artifact_id()?);
    let report = DeployConfigureValidateBuildReport {
        machine_id: built.canonical.machine_id.clone(),
        pipeline_version: built.canonical.pipeline_version.clone(),
        canonical_config_artifact_id,
        built_config_artifact_id,
        phase_count: 3,
    };
    Ok(DeployConfigureValidateBuildOutcome { built, report })
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

/// Decodes JSON transport input into the strict built deploy/configure/validate config.
pub fn decode_deploy_configure_validate_built_config(
    value: &Value,
) -> Result<DeployConfigureValidateBuiltConfig, DeployConfigureValidateConfigError> {
    serde_json::from_value(value.clone())
        .map_err(|source| DeployConfigureValidateConfigError::InvalidJson { source })
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
    use mfm_values::{MfmConfig, MfmValue};

    const JSON_CONFIG: &str = r#"{
        "deploy": {"network_id": "ethereum-mainnet", "from": "0x000000000000000000000000000000000000dead"},
        "configure": {
            "network_id": "ethereum-mainnet",
            "from": "0x000000000000000000000000000000000000dead",
            "calls": [{"function": "noop", "args": []}]
        },
        "validate": {"network_id": "ethereum-mainnet", "expected_chain_id": 1}
    }"#;

    const TOML_CONFIG: &str = r#"
        [deploy]
        network_id = "ethereum-mainnet"
        from = "0x000000000000000000000000000000000000dead"

        [configure]
        network_id = "ethereum-mainnet"
        from = "0x000000000000000000000000000000000000dead"

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
        assert_eq!(
            canonical.input.to_json_value().expect("input"),
            serde_json::json!({})
        );
        assert_eq!(
            canonical.configure.tx_hashes_export_key,
            "configure_tx_hashes"
        );
        assert_eq!(
            canonical.configure.receipts_export_key,
            "configure_receipts"
        );
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
            "deploy": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead"
            },
            "configure": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "calls": []
            },
            "validate": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1
            }
        }))
        .expect("decode");

        assert_eq!(canonical.machine_id, "evm_deploy_configure_validate");
        assert_eq!(canonical.pipeline_version, "v1");
        assert_eq!(
            canonical.input.to_json_value().expect("input"),
            serde_json::json!({})
        );
    }

    #[test]
    fn decode_json_payload_preserves_typed_artifact_and_args() {
        let canonical = decode_deploy_configure_validate_canonical_config(&serde_json::json!({
            "deploy": {
                "artifact": {
                    "abi": [
                        {
                            "type": "constructor",
                            "inputs": [{"name": "owner", "type": "address"}]
                        }
                    ],
                    "bytecode": {"object": "0x60006000"}
                },
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "constructor_args": ["0x0000000000000000000000000000000000000001"]
            },
            "configure": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "calls": [{
                    "function": "setOwner",
                    "args": ["0x0000000000000000000000000000000000000002"]
                }]
            },
            "validate": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1
            }
        }))
        .expect("decode");

        let deploy_artifact = canonical.deploy.artifact.as_ref().expect("deploy artifact");
        assert_eq!(
            deploy_artifact.abi.to_json_value().expect("abi"),
            serde_json::json!([
                {
                    "type": "constructor",
                    "inputs": [{"name": "owner", "type": "address"}]
                }
            ])
        );
        assert_eq!(
            deploy_artifact.bytecode.to_json_value().expect("bytecode"),
            serde_json::json!({"object": "0x60006000"})
        );
        assert_eq!(
            canonical.deploy.constructor_args,
            vec![AbiArgumentValue::from_json_value(&serde_json::json!(
                "0x0000000000000000000000000000000000000001"
            ))
            .expect("constructor arg")]
        );
        assert_eq!(
            canonical.configure.calls[0].args,
            vec![AbiArgumentValue::from_json_value(&serde_json::json!(
                "0x0000000000000000000000000000000000000002"
            ))
            .expect("configure arg")]
        );
    }

    #[test]
    fn decode_json_payload_preserves_typed_validate_assertions() {
        let canonical = decode_deploy_configure_validate_canonical_config(&serde_json::json!({
            "deploy": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead"
            },
            "configure": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "calls": []
            },
            "validate": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1,
                "read_assertions": [{
                    "function": "owner",
                    "args": [],
                    "expected": "0x0000000000000000000000000000000000000001"
                }]
            }
        }))
        .expect("decode");

        assert_eq!(canonical.validate.read_assertions.len(), 1);
        assert_eq!(
            canonical.validate.read_assertions[0].args,
            Vec::<AbiArgumentValue>::new()
        );
        assert_eq!(
            canonical.validate.read_assertions[0]
                .expected
                .to_json_value()
                .expect("expected"),
            serde_json::json!("0x0000000000000000000000000000000000000001")
        );
    }

    #[test]
    fn json_and_toml_authoring_normalize_to_same_built_outcome() {
        let json = build_deploy_configure_validate_outcome(
            canonicalize_deploy_configure_validate_authored_config(
                parse_deploy_configure_validate_authored_config(
                    JSON_CONFIG,
                    AuthoredConfigFormat::Json,
                )
                .expect("json parse"),
            )
            .expect("json canonical"),
        )
        .expect("json build");
        let toml = build_deploy_configure_validate_outcome(
            canonicalize_deploy_configure_validate_authored_config(
                parse_deploy_configure_validate_authored_config_with_hint(
                    TOML_CONFIG,
                    Some(Path::new("config.toml")),
                )
                .expect("toml parse"),
            )
            .expect("toml canonical"),
        )
        .expect("toml build");

        assert_eq!(json, toml);
    }

    #[test]
    fn built_artifact_id_is_deterministic_across_formats() {
        let json = build_deploy_configure_validate_outcome(
            canonicalize_deploy_configure_validate_authored_config(
                parse_deploy_configure_validate_authored_config(
                    JSON_CONFIG,
                    AuthoredConfigFormat::Json,
                )
                .expect("json parse"),
            )
            .expect("json canonical"),
        )
        .expect("json build");
        let toml = build_deploy_configure_validate_outcome(
            canonicalize_deploy_configure_validate_authored_config(
                parse_deploy_configure_validate_authored_config_with_hint(TOML_CONFIG, None)
                    .expect("toml parse"),
            )
            .expect("toml canonical"),
        )
        .expect("toml build");

        assert_eq!(
            json.built.artifact_id().expect("json artifact"),
            toml.built.artifact_id().expect("toml artifact")
        );
        assert_eq!(
            json.report.built_config_artifact_id,
            legacy_artifact_store_id(&json.built.artifact_id().expect("json artifact"))
        );
    }

    #[test]
    fn decode_built_config_rejects_legacy_canonical_shape() {
        let err = decode_deploy_configure_validate_built_config(&serde_json::json!({
            "deploy": {"network_id": "ethereum-mainnet"},
            "configure": {"network_id": "ethereum-mainnet"},
            "validate": {"network_id": "ethereum-mainnet"}
        }))
        .expect_err("legacy canonical shape should not decode as built config");

        assert!(matches!(
            err,
            DeployConfigureValidateConfigError::InvalidJson { .. }
        ));
    }

    #[test]
    fn lifecycle_config_rejects_raw_contract_addresses() {
        let configure_err = decode_deploy_configure_validate_canonical_config(&serde_json::json!({
            "deploy": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead"
            },
            "configure": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "contract_address": "0x0000000000000000000000000000000000000001",
                "calls": []
            },
            "validate": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1
            }
        }))
        .expect_err("configure must not accept raw address");
        let validate_err = decode_deploy_configure_validate_canonical_config(&serde_json::json!({
            "deploy": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead"
            },
            "configure": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "calls": []
            },
            "validate": {
                "network_id": "ethereum-mainnet",
                "contract_address": "0x0000000000000000000000000000000000000001",
                "expected_chain_id": 1
            }
        }))
        .expect_err("validate must not accept raw address");

        assert!(matches!(
            configure_err,
            DeployConfigureValidateConfigError::InvalidJson { .. }
        ));
        assert!(matches!(
            validate_err,
            DeployConfigureValidateConfigError::InvalidJson { .. }
        ));
    }

    #[test]
    fn existing_validation_uses_configured_contract_ref() {
        assert!(ExistingConfiguredContractValidationConfig::schema_id().is_ok());

        let source = include_str!("lib.rs");
        let existing_config = source
            .split("pub struct ExistingConfiguredContractValidationConfig")
            .nth(1)
            .expect("existing validation config")
            .split("/// Human-authored")
            .next()
            .expect("existing validation config body");

        assert!(existing_config.contains("configured_contract: ExistingConfiguredContractRef"));
        assert!(!existing_config.contains("contract_address: String"));
        assert!(!existing_config.contains(&["contract_address", ": Option", "<String>"].concat()));
    }

    #[test]
    fn canonical_and_built_configs_have_typed_schema_ids() {
        assert!(<DeployConfigureValidateInput as MfmValue>::schema_id().is_ok());
        assert!(<DeployConfigureValidateDeployConfig as MfmConfig>::schema_id().is_ok());
        assert!(<DeployConfigureValidateConfigureConfig as MfmConfig>::schema_id().is_ok());
        assert!(<DeployConfigureValidateValidateConfig as MfmConfig>::schema_id().is_ok());
        assert!(<ExistingConfiguredContractValidationConfig as MfmConfig>::schema_id().is_ok());
        assert!(<DeployConfigureValidateCanonicalConfig as MfmConfig>::schema_id().is_ok());
        assert!(<DeployConfigureValidateBuiltConfig as MfmConfig>::schema_id().is_ok());
        assert!(<DeployConfigureValidateExecutionConfig as MfmConfig>::schema_id().is_ok());
        assert!(<DeployConfigureValidateBuildReport as MfmValue>::schema_id().is_ok());
    }
}
