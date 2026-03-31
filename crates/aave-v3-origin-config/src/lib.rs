#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared authored, canonical, and built config pipeline for Aave V3 Origin stack workflows.
//!
//! This crate owns the workflow-family-specific config boundary for Aave Origin phase-A flows:
//! authored bytes parse into [`AaveV3OriginStackAuthoredConfig`], canonicalization produces
//! [`AaveV3OriginStackCanonicalConfig`], and the pure build step produces
//! [`AaveV3OriginStackBuiltConfig`].
//!
//! The current execution backend remains the existing Nix/Foundry wrapper tools. The canonical
//! source pin is therefore validated against the currently supported backend pin so typed authored
//! config remains truthful about what the compatibility backend can execute.

use std::collections::BTreeMap;
use std::path::Path;

use mfm_authored_config::parse_authored_config_with_hint;
pub use mfm_authored_config::AuthoredConfigFormat;
use mfm_evm_core::encoding::normalize_address;
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::ArtifactId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Current backend-supported Aave Origin repository URL.
pub const AAVE_V3_ORIGIN_BACKEND_REPO_URL: &str = "https://github.com/aave-dao/aave-v3-origin";
/// Current backend-supported Aave Origin pinned commit.
pub const AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA: &str = "1e3d70c4151a94166ebc59e2eaa4aff6e6ba6978";
/// Flake app ref for the fetch backend.
pub const AAVE_V3_ORIGIN_FETCH_APP_REF: &str = "path:.#aave-v3-origin-fetch";
/// Flake app ref for the compile backend.
pub const AAVE_V3_ORIGIN_COMPILE_APP_REF: &str = "path:.#aave-v3-origin-compile";
/// Flake app ref for the deploy backend.
pub const AAVE_V3_ORIGIN_DEPLOY_APP_REF: &str = "path:.#aave-v3-origin-deploy";

/// Non-secret env key that tells the compatibility backend which env var contains the deploy
/// signing key.
pub const AAVE_V3_ORIGIN_DEPLOY_SIGNING_KEY_ENV_SELECTOR: &str =
    "MFM_AAVE_V3_ORIGIN_DEPLOY_SIGNING_KEY_ENV";
/// Non-secret env key that tells the compatibility backend which env var contains the RPC URL.
pub const AAVE_V3_ORIGIN_RPC_URL_ENV_SELECTOR: &str = "MFM_AAVE_V3_ORIGIN_RPC_URL_ENV";
/// Internal runtime bridge env key that carries the resolved deploy signer material into the Nix
/// task wrapper without persisting the secret value.
pub const AAVE_V3_ORIGIN_DEPLOY_SIGNER_VALUE_BRIDGE_ENV: &str =
    "MFM_AAVE_V3_ORIGIN_DEPLOY_SIGNER_VALUE";
/// Internal runtime bridge env key that carries the resolved RPC URL into the Nix task wrapper.
pub const AAVE_V3_ORIGIN_RPC_URL_BRIDGE_ENV: &str = "MFM_AAVE_V3_ORIGIN_RPC_URL";
/// Non-secret env key that carries the supplier address for compatibility deploys.
pub const AAVE_V3_ORIGIN_SUPPLIER_ENV: &str = "MFM_AAVE_V3_ORIGIN_SUPPLIER";
/// Non-secret env key that carries the borrower address for compatibility deploys.
pub const AAVE_V3_ORIGIN_BORROWER_ENV: &str = "MFM_AAVE_V3_ORIGIN_BORROWER";
/// Non-secret env key that carries the USDC supply amount for compatibility deploys.
pub const AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT_ENV: &str = "MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT";
/// Non-secret env key that carries the WBTC collateral amount for compatibility deploys.
pub const AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT_ENV: &str =
    "MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT";
/// Non-secret env key that forces the compatibility backend to validate the expected repo URL.
pub const AAVE_V3_ORIGIN_EXPECTED_REPO_URL_ENV: &str = "MFM_AAVE_V3_ORIGIN_EXPECTED_REPO_URL";
/// Non-secret env key that forces the compatibility backend to validate the expected commit SHA.
pub const AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA_ENV: &str = "MFM_AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA";

const FETCH_RESULT_PORT: &str = "fetch_origin_result";
const COMPILE_RESULT_PORT: &str = "compile_origin_result";
const DEPLOY_RESULT_PORT: &str = "deploy_origin_result";
const DEPLOY_MANIFEST_PORT: &str = "deploy_manifest";

fn default_control_scope() -> String {
    "shared".to_string()
}

fn default_fetch_timeout_ms() -> u64 {
    300_000
}

fn default_compile_timeout_ms() -> u64 {
    600_000
}

fn default_deploy_timeout_ms() -> u64 {
    600_000
}

fn default_empty_object() -> Value {
    serde_json::json!({})
}

fn default_empty_vec() -> Vec<String> {
    Vec::new()
}

/// Human-authored Aave Origin stack config loaded from JSON or TOML.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginStackAuthoredConfig {
    /// Source pin the caller intends to deploy.
    pub source: AaveV3OriginSourceConfig,
    /// Managed network id used by downstream phase-B flows.
    pub network_id: String,
    /// Managed RPC control scope to use during runtime interactions.
    #[serde(default = "default_control_scope")]
    pub control_scope: String,
    /// Env var name that holds the deploy signing key at execution time.
    pub deploy_signing_key_env: String,
    /// Env var name that holds the RPC URL at execution time.
    pub rpc_url_env: String,
    /// Supplier actor address.
    pub supplier: String,
    /// Borrower actor address.
    pub borrower: String,
    /// USDC supply amount in base units.
    pub usdc_supply_amount: u64,
    /// WBTC collateral amount in base units.
    pub wbtc_collateral_amount: u64,
    /// Timeout for the fetch step.
    #[serde(default = "default_fetch_timeout_ms")]
    pub fetch_timeout_ms: u64,
    /// Timeout for the compile step.
    #[serde(default = "default_compile_timeout_ms")]
    pub compile_timeout_ms: u64,
    /// Timeout for the deploy step.
    #[serde(default = "default_deploy_timeout_ms")]
    pub deploy_timeout_ms: u64,
}

/// Source pin carried by authored and canonical config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginSourceConfig {
    /// Repository URL.
    pub repo_url: String,
    /// Commit SHA.
    pub commit_sha: String,
}

/// Canonical Aave Origin stack config used for deterministic hashing and build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginStackCanonicalConfig {
    /// Canonical source pin supported by the current backend.
    pub source: AaveV3OriginSourceConfig,
    /// Managed network id used by downstream phase-B flows.
    pub network_id: String,
    /// Managed RPC control scope used for runtime interactions.
    pub control_scope: String,
    /// Env var name that holds the deploy signing key at execution time.
    pub deploy_signing_key_env: String,
    /// Env var name that holds the RPC URL at execution time.
    pub rpc_url_env: String,
    /// Canonical supplier address.
    pub supplier: String,
    /// Canonical borrower address.
    pub borrower: String,
    /// USDC supply amount in base units.
    pub usdc_supply_amount: u64,
    /// WBTC collateral amount in base units.
    pub wbtc_collateral_amount: u64,
    /// Timeout for the fetch step.
    pub fetch_timeout_ms: u64,
    /// Timeout for the compile step.
    pub compile_timeout_ms: u64,
    /// Timeout for the deploy step.
    pub deploy_timeout_ms: u64,
}

impl AaveV3OriginStackCanonicalConfig {
    /// Serializes the canonical config as JSON.
    pub fn to_json_value(&self) -> Result<Value, AaveV3OriginStackConfigError> {
        serde_json::to_value(self).map_err(|source| AaveV3OriginStackConfigError::Serialize {
            stage: "canonical config",
            source,
        })
    }

    /// Computes the authoritative artifact id for the canonical config.
    pub fn artifact_id(&self) -> Result<ArtifactId, AaveV3OriginStackConfigError> {
        let value = self.to_json_value()?;
        artifact_id_for_json(&value).map_err(|source| AaveV3OriginStackConfigError::CanonicalJson {
            stage: "canonical config",
            source,
        })
    }
}

/// Built Aave Origin stack config consumed by the strict execute op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginStackBuiltConfig {
    /// Canonical config from which this built config was derived.
    pub canonical: AaveV3OriginStackCanonicalConfig,
    /// Execution payload consumed by the strict execute op.
    pub execution: AaveV3OriginStackExecutionConfig,
}

impl AaveV3OriginStackBuiltConfig {
    /// Serializes the built config as JSON.
    pub fn to_json_value(&self) -> Result<Value, AaveV3OriginStackConfigError> {
        serde_json::to_value(self).map_err(|source| AaveV3OriginStackConfigError::Serialize {
            stage: "built config",
            source,
        })
    }

    /// Computes the authoritative artifact id for the built config.
    pub fn artifact_id(&self) -> Result<ArtifactId, AaveV3OriginStackConfigError> {
        let value = self.to_json_value()?;
        artifact_id_for_json(&value).map_err(|source| AaveV3OriginStackConfigError::CanonicalJson {
            stage: "built config",
            source,
        })
    }
}

/// Execution payload consumed by the strict Aave Origin execute op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginStackExecutionConfig {
    /// Fetch step config for the external compatibility backend.
    pub fetch_origin: AaveV3OriginNixAppStepConfig,
    /// Compile step config for the external compatibility backend.
    pub compile_origin: AaveV3OriginNixAppStepConfig,
    /// Deploy step config for the external compatibility backend.
    pub deploy_origin_stack: AaveV3OriginNixAppStepConfig,
    /// Adapt step config that turns raw deploy output into the standard deploy manifest.
    pub adapt_origin_deploy: AaveV3OriginAdaptStepConfig,
}

/// Execution-ready nix-app child-op config carried in the built config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginNixAppStepConfig {
    /// Flake app ref resolved by `nix_app`.
    pub app: String,
    /// Extra command-line arguments.
    #[serde(default = "default_empty_vec")]
    pub argv: Vec<String>,
    /// JSON payload written to stdin.
    #[serde(default = "default_empty_object")]
    pub stdin_json: Value,
    /// Extra non-secret environment variables forwarded to the child process.
    #[serde(default = "default_empty_object")]
    pub env: Value,
    /// Runtime host env bindings resolved by the Nix transport when the step runs through
    /// `nix_app`.
    #[serde(default)]
    pub host_env_bindings: BTreeMap<String, String>,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
    /// Export key written by the child op.
    pub write_result_to: String,
}

impl AaveV3OriginNixAppStepConfig {
    /// Converts the step config into a `nix_app` op config value.
    pub fn to_op_config(&self) -> Value {
        serde_json::json!({
            "app": self.app,
            "argv": self.argv,
            "stdin_json": self.stdin_json,
            "env": self.env,
            "host_env_bindings": self.host_env_bindings,
            "timeout_ms": self.timeout_ms,
            "write_result_to": self.write_result_to,
        })
    }
}

/// Execution-ready adapt-op config carried in the built config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginAdaptStepConfig {
    /// Imported port that contains the raw Origin deploy output payload.
    pub origin_deploy_port: String,
    /// Exported port that receives the adapted deploy manifest.
    pub deploy_manifest_export_key: String,
}

impl AaveV3OriginAdaptStepConfig {
    /// Converts the step config into an `aave_v3_origin_adapt_deploy` op config value.
    pub fn to_op_config(&self) -> Value {
        serde_json::json!({
            "origin_deploy_port": self.origin_deploy_port,
            "deploy_manifest_export_key": self.deploy_manifest_export_key,
        })
    }
}

/// Stable report emitted by the Aave Origin config-build workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AaveV3OriginStackBuildReport {
    /// Schema version for this report surface.
    pub schema_version: u32,
    /// Canonical source repo URL.
    pub source_repo_url: String,
    /// Canonical source commit SHA.
    pub source_commit_sha: String,
    /// Canonical network id.
    pub network_id: String,
    /// Canonical control scope.
    pub control_scope: String,
    /// Content-addressed canonical config artifact id.
    pub canonical_config_artifact_id: String,
    /// Content-addressed built config artifact id.
    pub built_config_artifact_id: String,
    /// Number of phase-A steps in the lowered execution config.
    pub step_count: u64,
}

impl AaveV3OriginStackBuildReport {
    /// Current schema version for the build report surface.
    pub const SCHEMA_VERSION: u32 = 1;
}

/// Pure build outcome for Aave Origin config workflows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AaveV3OriginStackBuildOutcome {
    /// Deterministic built config consumed by the execute op.
    pub built: AaveV3OriginStackBuiltConfig,
    /// Stable build report derived from the canonical and built config artifacts.
    pub report: AaveV3OriginStackBuildReport,
}

/// Errors returned while parsing, canonicalizing, or building Aave Origin stack config.
#[derive(Debug, Error)]
pub enum AaveV3OriginStackConfigError {
    /// JSON authored config parsing failed.
    #[error("failed to parse Aave Origin authored config as json: {source}")]
    InvalidJson {
        /// Underlying parser error.
        #[source]
        source: serde_json::Error,
    },
    /// TOML authored config parsing failed.
    #[error("failed to parse Aave Origin authored config as toml: {source}")]
    InvalidToml {
        /// Underlying parser error.
        #[source]
        source: toml::de::Error,
    },
    /// Validation failed.
    #[error("{0}")]
    Invalid(String),
    /// The current compatibility backend only supports one pinned source.
    #[error(
        "Aave Origin compatibility backend only supports {supported_repo_url}@{supported_commit_sha}, got {repo_url}@{commit_sha}"
    )]
    UnsupportedSource {
        /// Repo URL requested by the caller.
        repo_url: String,
        /// Commit SHA requested by the caller.
        commit_sha: String,
        /// Repo URL supported by the current backend.
        supported_repo_url: &'static str,
        /// Commit SHA supported by the current backend.
        supported_commit_sha: &'static str,
    },
    /// Address normalization failed.
    #[error("invalid {field}: {value}")]
    InvalidAddress {
        /// Field name that failed normalization.
        field: &'static str,
        /// Raw value supplied by the caller.
        value: String,
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

/// Parses authored Aave Origin stack config using the supplied format.
pub fn parse_aave_v3_origin_stack_authored_config(
    raw: &str,
    format: AuthoredConfigFormat,
) -> Result<AaveV3OriginStackAuthoredConfig, AaveV3OriginStackConfigError> {
    match format {
        AuthoredConfigFormat::Json => serde_json::from_str(raw)
            .map_err(|source| AaveV3OriginStackConfigError::InvalidJson { source }),
        AuthoredConfigFormat::Toml => toml::from_str(raw)
            .map_err(|source| AaveV3OriginStackConfigError::InvalidToml { source }),
    }
}

/// Parses authored Aave Origin stack config using an optional path hint.
pub fn parse_aave_v3_origin_stack_authored_config_with_hint(
    raw: &str,
    path_hint: Option<&Path>,
) -> Result<AaveV3OriginStackAuthoredConfig, AaveV3OriginStackConfigError> {
    parse_authored_config_with_hint(
        raw,
        path_hint,
        |value| parse_aave_v3_origin_stack_authored_config(value, AuthoredConfigFormat::Json),
        |value| parse_aave_v3_origin_stack_authored_config(value, AuthoredConfigFormat::Toml),
    )
}

/// Canonicalizes authored Aave Origin config by normalizing addresses, materializing defaults, and
/// validating that the current compatibility backend can execute the requested source pin.
pub fn canonicalize_aave_v3_origin_stack_authored_config(
    authored: AaveV3OriginStackAuthoredConfig,
) -> Result<AaveV3OriginStackCanonicalConfig, AaveV3OriginStackConfigError> {
    validate_non_empty("source.repo_url", &authored.source.repo_url)?;
    validate_non_empty("source.commit_sha", &authored.source.commit_sha)?;
    validate_non_empty("network_id", &authored.network_id)?;
    validate_non_empty("control_scope", &authored.control_scope)?;
    validate_non_empty("deploy_signing_key_env", &authored.deploy_signing_key_env)?;
    validate_non_empty("rpc_url_env", &authored.rpc_url_env)?;
    validate_positive_timeout("fetch_timeout_ms", authored.fetch_timeout_ms)?;
    validate_positive_timeout("compile_timeout_ms", authored.compile_timeout_ms)?;
    validate_positive_timeout("deploy_timeout_ms", authored.deploy_timeout_ms)?;

    if authored.source.repo_url != AAVE_V3_ORIGIN_BACKEND_REPO_URL
        || authored.source.commit_sha != AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA
    {
        return Err(AaveV3OriginStackConfigError::UnsupportedSource {
            repo_url: authored.source.repo_url,
            commit_sha: authored.source.commit_sha,
            supported_repo_url: AAVE_V3_ORIGIN_BACKEND_REPO_URL,
            supported_commit_sha: AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA,
        });
    }

    Ok(AaveV3OriginStackCanonicalConfig {
        source: AaveV3OriginSourceConfig {
            repo_url: AAVE_V3_ORIGIN_BACKEND_REPO_URL.to_string(),
            commit_sha: AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA.to_string(),
        },
        network_id: authored.network_id,
        control_scope: authored.control_scope,
        deploy_signing_key_env: authored.deploy_signing_key_env,
        rpc_url_env: authored.rpc_url_env,
        supplier: canonical_address("supplier", &authored.supplier)?,
        borrower: canonical_address("borrower", &authored.borrower)?,
        usdc_supply_amount: authored.usdc_supply_amount,
        wbtc_collateral_amount: authored.wbtc_collateral_amount,
        fetch_timeout_ms: authored.fetch_timeout_ms,
        compile_timeout_ms: authored.compile_timeout_ms,
        deploy_timeout_ms: authored.deploy_timeout_ms,
    })
}

/// Builds canonical Aave Origin config into the explicit execution payload.
pub fn build_aave_v3_origin_stack_config(
    canonical: AaveV3OriginStackCanonicalConfig,
) -> Result<AaveV3OriginStackBuiltConfig, AaveV3OriginStackConfigError> {
    Ok(AaveV3OriginStackBuiltConfig {
        execution: AaveV3OriginStackExecutionConfig {
            fetch_origin: AaveV3OriginNixAppStepConfig {
                app: AAVE_V3_ORIGIN_FETCH_APP_REF.to_string(),
                argv: Vec::new(),
                stdin_json: default_empty_object(),
                env: expected_source_env(&canonical),
                host_env_bindings: BTreeMap::new(),
                timeout_ms: canonical.fetch_timeout_ms,
                write_result_to: FETCH_RESULT_PORT.to_string(),
            },
            compile_origin: AaveV3OriginNixAppStepConfig {
                app: AAVE_V3_ORIGIN_COMPILE_APP_REF.to_string(),
                argv: Vec::new(),
                stdin_json: default_empty_object(),
                env: expected_source_env(&canonical),
                host_env_bindings: BTreeMap::new(),
                timeout_ms: canonical.compile_timeout_ms,
                write_result_to: COMPILE_RESULT_PORT.to_string(),
            },
            deploy_origin_stack: AaveV3OriginNixAppStepConfig {
                app: AAVE_V3_ORIGIN_DEPLOY_APP_REF.to_string(),
                argv: Vec::new(),
                stdin_json: default_empty_object(),
                env: deploy_env(&canonical),
                host_env_bindings: deploy_host_env_bindings(&canonical),
                timeout_ms: canonical.deploy_timeout_ms,
                write_result_to: DEPLOY_RESULT_PORT.to_string(),
            },
            adapt_origin_deploy: AaveV3OriginAdaptStepConfig {
                origin_deploy_port: DEPLOY_RESULT_PORT.to_string(),
                deploy_manifest_export_key: DEPLOY_MANIFEST_PORT.to_string(),
            },
        },
        canonical,
    })
}

/// Builds canonical Aave Origin config and returns the typed build outcome.
pub fn build_aave_v3_origin_stack_outcome(
    canonical: AaveV3OriginStackCanonicalConfig,
) -> Result<AaveV3OriginStackBuildOutcome, AaveV3OriginStackConfigError> {
    let built = build_aave_v3_origin_stack_config(canonical)?;
    let canonical_artifact_id = built.canonical.artifact_id()?;
    let built_artifact_id = built.artifact_id()?;
    let report = AaveV3OriginStackBuildReport {
        schema_version: AaveV3OriginStackBuildReport::SCHEMA_VERSION,
        source_repo_url: built.canonical.source.repo_url.clone(),
        source_commit_sha: built.canonical.source.commit_sha.clone(),
        network_id: built.canonical.network_id.clone(),
        control_scope: built.canonical.control_scope.clone(),
        canonical_config_artifact_id: canonical_artifact_id.0,
        built_config_artifact_id: built_artifact_id.0,
        step_count: 4,
    };
    Ok(AaveV3OriginStackBuildOutcome { built, report })
}

/// Decodes canonical Aave Origin config from JSON.
pub fn decode_aave_v3_origin_stack_canonical_config(
    value: &Value,
) -> Result<AaveV3OriginStackCanonicalConfig, AaveV3OriginStackConfigError> {
    serde_json::from_value(value.clone())
        .map_err(|err| AaveV3OriginStackConfigError::Invalid(err.to_string()))
}

/// Decodes built Aave Origin config from JSON.
pub fn decode_aave_v3_origin_stack_built_config(
    value: &Value,
) -> Result<AaveV3OriginStackBuiltConfig, AaveV3OriginStackConfigError> {
    serde_json::from_value(value.clone())
        .map_err(|err| AaveV3OriginStackConfigError::Invalid(err.to_string()))
}

fn validate_non_empty(
    field: &'static str,
    value: &str,
) -> Result<(), AaveV3OriginStackConfigError> {
    if value.trim().is_empty() {
        return Err(AaveV3OriginStackConfigError::Invalid(format!(
            "{field} must be non-empty"
        )));
    }
    Ok(())
}

fn validate_positive_timeout(
    field: &'static str,
    value: u64,
) -> Result<(), AaveV3OriginStackConfigError> {
    if value == 0 {
        return Err(AaveV3OriginStackConfigError::Invalid(format!(
            "{field} must be > 0"
        )));
    }
    Ok(())
}

fn canonical_address(
    field: &'static str,
    raw: &str,
) -> Result<String, AaveV3OriginStackConfigError> {
    normalize_address(raw).map_err(|_| AaveV3OriginStackConfigError::InvalidAddress {
        field,
        value: raw.to_string(),
    })
}

fn expected_source_env(canonical: &AaveV3OriginStackCanonicalConfig) -> Value {
    serde_json::json!({
        AAVE_V3_ORIGIN_EXPECTED_REPO_URL_ENV: canonical.source.repo_url,
        AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA_ENV: canonical.source.commit_sha,
    })
}

fn deploy_env(canonical: &AaveV3OriginStackCanonicalConfig) -> Value {
    let mut env = expected_source_env(canonical);
    let object = env.as_object_mut().expect("expected source env object");
    object.insert(
        AAVE_V3_ORIGIN_DEPLOY_SIGNING_KEY_ENV_SELECTOR.to_string(),
        Value::String(canonical.deploy_signing_key_env.clone()),
    );
    object.insert(
        AAVE_V3_ORIGIN_RPC_URL_ENV_SELECTOR.to_string(),
        Value::String(canonical.rpc_url_env.clone()),
    );
    object.insert(
        AAVE_V3_ORIGIN_SUPPLIER_ENV.to_string(),
        Value::String(canonical.supplier.clone()),
    );
    object.insert(
        AAVE_V3_ORIGIN_BORROWER_ENV.to_string(),
        Value::String(canonical.borrower.clone()),
    );
    object.insert(
        AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT_ENV.to_string(),
        Value::String(canonical.usdc_supply_amount.to_string()),
    );
    object.insert(
        AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT_ENV.to_string(),
        Value::String(canonical.wbtc_collateral_amount.to_string()),
    );
    env
}

fn deploy_host_env_bindings(
    canonical: &AaveV3OriginStackCanonicalConfig,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            AAVE_V3_ORIGIN_DEPLOY_SIGNER_VALUE_BRIDGE_ENV.to_string(),
            canonical.deploy_signing_key_env.clone(),
        ),
        (
            AAVE_V3_ORIGIN_RPC_URL_BRIDGE_ENV.to_string(),
            canonical.rpc_url_env.clone(),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_CONFIG: &str = r#"{
        "source": {
            "repo_url": "https://github.com/aave-dao/aave-v3-origin",
            "commit_sha": "1e3d70c4151a94166ebc59e2eaa4aff6e6ba6978"
        },
        "network_id": "reth-local",
        "deploy_signing_key_env": "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY",
        "rpc_url_env": "MFM_EVM_RPC_URL",
        "supplier": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
        "borrower": "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC",
        "usdc_supply_amount": 1000000000000,
        "wbtc_collateral_amount": 1000000000
    }"#;

    const TOML_CONFIG: &str = r#"
network_id = "reth-local"
deploy_signing_key_env = "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY"
rpc_url_env = "MFM_EVM_RPC_URL"
supplier = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
borrower = "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
usdc_supply_amount = 1000000000000
wbtc_collateral_amount = 1000000000

[source]
repo_url = "https://github.com/aave-dao/aave-v3-origin"
commit_sha = "1e3d70c4151a94166ebc59e2eaa4aff6e6ba6978"
"#;

    #[test]
    fn canonicalization_equates_json_and_toml_inputs() {
        let json =
            parse_aave_v3_origin_stack_authored_config(JSON_CONFIG, AuthoredConfigFormat::Json)
                .expect("json parse");
        let toml =
            parse_aave_v3_origin_stack_authored_config(TOML_CONFIG, AuthoredConfigFormat::Toml)
                .expect("toml parse");

        let json_canonical =
            canonicalize_aave_v3_origin_stack_authored_config(json).expect("json canonical");
        let toml_canonical =
            canonicalize_aave_v3_origin_stack_authored_config(toml).expect("toml canonical");

        assert_eq!(json_canonical, toml_canonical);
    }

    #[test]
    fn canonicalization_rejects_unsupported_source_pin() {
        let mut authored =
            parse_aave_v3_origin_stack_authored_config(JSON_CONFIG, AuthoredConfigFormat::Json)
                .expect("parse");
        authored.source.commit_sha = "deadbeef".to_string();
        let err = canonicalize_aave_v3_origin_stack_authored_config(authored)
            .expect_err("unsupported source must fail");
        assert!(matches!(
            err,
            AaveV3OriginStackConfigError::UnsupportedSource { .. }
        ));
    }

    #[test]
    fn build_outcome_is_deterministic() {
        let authored =
            parse_aave_v3_origin_stack_authored_config(JSON_CONFIG, AuthoredConfigFormat::Json)
                .expect("parse");
        let canonical =
            canonicalize_aave_v3_origin_stack_authored_config(authored).expect("canonical");

        let first = build_aave_v3_origin_stack_outcome(canonical.clone()).expect("build");
        let second = build_aave_v3_origin_stack_outcome(canonical).expect("build");

        assert_eq!(first.built, second.built);
        assert_eq!(first.report, second.report);
    }

    #[test]
    fn built_config_carries_selector_envs_and_expected_source() {
        let authored =
            parse_aave_v3_origin_stack_authored_config(JSON_CONFIG, AuthoredConfigFormat::Json)
                .expect("parse");
        let canonical =
            canonicalize_aave_v3_origin_stack_authored_config(authored).expect("canonical");
        let built = build_aave_v3_origin_stack_config(canonical).expect("build");
        let env = built
            .execution
            .deploy_origin_stack
            .env
            .as_object()
            .expect("deploy env object");

        assert_eq!(
            env.get(AAVE_V3_ORIGIN_DEPLOY_SIGNING_KEY_ENV_SELECTOR),
            Some(&Value::String(
                "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY".to_string()
            ))
        );
        assert_eq!(
            env.get(AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA_ENV),
            Some(&Value::String(
                AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA.to_string()
            ))
        );
        assert_eq!(
            built
                .execution
                .deploy_origin_stack
                .host_env_bindings
                .get(AAVE_V3_ORIGIN_DEPLOY_SIGNER_VALUE_BRIDGE_ENV),
            Some(&"MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY".to_string())
        );
    }

    #[test]
    fn canonicalization_rejects_invalid_addresses() {
        let mut authored =
            parse_aave_v3_origin_stack_authored_config(JSON_CONFIG, AuthoredConfigFormat::Json)
                .expect("parse");
        authored.supplier = "not-an-address".to_string();
        let err = canonicalize_aave_v3_origin_stack_authored_config(authored)
            .expect_err("invalid address must fail");
        assert!(matches!(
            err,
            AaveV3OriginStackConfigError::InvalidAddress {
                field: "supplier",
                ..
            }
        ));
    }
}
