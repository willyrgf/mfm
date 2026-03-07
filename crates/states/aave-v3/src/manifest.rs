use std::collections::HashSet;

use mfm_evm_core::abi as common_abi;
use mfm_evm_runtime::dcv as shared_dcv;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_state_common::errors as op_errors;
use serde::{Deserialize, Serialize};

/// Kind string for compile-manifest artifacts consumed by deploy planning states.
pub const COMPILE_MANIFEST_KIND: &str = "aave_v3_origin_compile_manifest_v1";
/// Kind string for deploy-manifest artifacts emitted by deploy states.
pub const DEPLOY_MANIFEST_KIND: &str = "aave_v3_deploy_manifest_v1";
/// Kind string for origin deploy-output artifacts adapted into deploy manifests.
pub const ORIGIN_DEPLOY_OUTPUT_KIND: &str = "aave_v3_origin_deploy_output_v1";
/// Kind string for configuration reports emitted after runtime setup.
pub const CONFIG_REPORT_KIND: &str = "aave_v3_config_report_v1";

/// Canonical manifest identifier for the USDC contract.
pub const CONTRACT_USDC: &str = "usdc";
/// Canonical manifest identifier for the WBTC contract.
pub const CONTRACT_WBTC: &str = "wbtc";
/// Canonical manifest identifier for the pool contract.
pub const CONTRACT_POOL: &str = "pool";
/// Canonical manifest identifier for the USDC aToken contract.
pub const CONTRACT_USDC_A_TOKEN: &str = "usdc_a_token";
/// Canonical manifest identifier for the WBTC aToken contract.
pub const CONTRACT_WBTC_A_TOKEN: &str = "wbtc_a_token";
/// Canonical manifest identifier for the USDC variable-debt-token contract.
pub const CONTRACT_USDC_VARIABLE_DEBT_TOKEN: &str = "usdc_variable_debt_token";

fn default_poll_interval_ms() -> u64 {
    200
}

fn default_max_receipt_polls() -> u64 {
    120
}

fn default_compile_manifest_port() -> String {
    "result".to_string()
}

fn default_deploy_manifest_port() -> String {
    "deploy_manifest".to_string()
}

fn default_deploy_manifest_export_key() -> String {
    "deploy_manifest".to_string()
}

fn default_config_report_export_key() -> String {
    "config_report".to_string()
}

fn default_deployer_account_index() -> usize {
    0
}

fn default_configure_from_account_index() -> usize {
    0
}

/// JSON artifact payload that contains contract ABI and bytecode.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractArtifactJson {
    /// Contract ABI JSON payload.
    pub abi: serde_json::Value,
    /// Contract bytecode JSON payload.
    pub bytecode: serde_json::Value,
}

impl ContractArtifactJson {
    /// Parses the artifact into a validated ABI plus constructor bytecode bytes.
    pub fn parse(&self) -> Result<(common_abi::ParsedAbi, Vec<u8>), StateError> {
        let cfg = serde_json::from_value::<shared_dcv::ContractArtifactConfig>(serde_json::json!({
            "abi": self.abi,
            "bytecode": self.bytecode,
        }))
        .map_err(|_| {
            op_errors::state_unknown("invalid_contract_artifact", "contract artifact was invalid")
        })?;

        shared_dcv::parse_artifact(&cfg).map_err(|_| {
            op_errors::state_unknown("invalid_contract_artifact", "contract artifact was invalid")
        })
    }
}

/// Contract entry inside an Aave compile manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveCompileManifestContract {
    /// Stable contract identifier used by downstream deploy/configure logic.
    pub id: String,
    /// ABI and bytecode payload for the contract.
    pub artifact: ContractArtifactJson,
    #[serde(default)]
    /// Constructor arguments passed during deployment.
    pub constructor_args: Vec<serde_json::Value>,
}

/// Compile-manifest artifact consumed by deployment states.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveCompileManifest {
    /// Manifest kind discriminator.
    pub kind: String,
    /// Contracts available for deployment.
    pub contracts: Vec<AaveCompileManifestContract>,
}

/// Deployed contract entry emitted into the deploy manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployManifestContract {
    /// Stable contract identifier.
    pub id: String,
    /// Deployed contract address.
    pub address: String,
    /// Transaction hash that created the contract.
    pub deploy_tx_hash: String,
    /// Transaction receipt for the deployment.
    pub deploy_receipt: serde_json::Value,
    /// ABI and bytecode payload for the contract.
    pub artifact: ContractArtifactJson,
}

/// Deploy-manifest artifact produced by Aave deployment states.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployManifest {
    /// Manifest kind discriminator.
    pub kind: String,
    /// Deployed contracts included in the manifest.
    pub contracts: Vec<AaveDeployManifestContract>,
}

/// Contract entry in an origin deploy-output artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveOriginDeployOutputContract {
    /// Stable contract identifier.
    pub id: String,
    /// Deployed contract address.
    pub address: String,
    /// ABI and bytecode payload for the contract.
    pub artifact: ContractArtifactJson,
    #[serde(default)]
    /// Optional deployment transaction hash supplied by the origin tool.
    pub deploy_tx_hash: Option<String>,
    #[serde(default)]
    /// Optional deployment receipt supplied by the origin tool.
    pub deploy_receipt: Option<serde_json::Value>,
}

/// Origin deploy-output artifact adapted into an MFM deploy manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveOriginDeployOutput {
    /// Artifact kind discriminator.
    pub kind: String,
    /// Contracts described by the origin tool.
    pub contracts: Vec<AaveOriginDeployOutputContract>,
}

/// Record of a single configuration transaction applied to the deployed runtime.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigCallRecord {
    /// Config function invoked on-chain.
    pub function: String,
    /// Transaction hash that submitted the call.
    pub tx_hash: String,
    /// Receipt returned for the configuration transaction.
    pub receipt: serde_json::Value,
}

/// Report emitted after configuration completes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigReport {
    /// Artifact kind discriminator.
    pub kind: String,
    /// Pool contract address.
    pub pool: String,
    /// USDC contract address.
    pub usdc: String,
    /// WBTC contract address.
    pub wbtc: String,
    /// Configuration transactions applied to the deployment.
    pub calls: Vec<AaveConfigCallRecord>,
}

/// Runtime configuration for the Aave deploy flow.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployRuntimeConfig {
    #[serde(default = "default_compile_manifest_port")]
    /// Context port that contains the compile manifest input.
    pub compile_manifest_port: String,

    #[serde(default = "default_deployer_account_index")]
    /// Account index used when signing through node-managed accounts.
    pub deployer_account_index: usize,

    #[serde(default)]
    /// Optional environment variable name that holds a private key for signed deploys.
    pub signing_key_env: Option<String>,

    #[serde(default = "default_poll_interval_ms")]
    /// Delay between receipt polls in milliseconds.
    pub poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    /// Maximum number of receipt polls before timing out.
    pub max_receipt_polls: u64,

    #[serde(default = "default_deploy_manifest_export_key")]
    /// Context key used to export the deploy manifest.
    pub deploy_manifest_export_key: String,
}

/// Runtime configuration for the Aave configure flow.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigureRuntimeConfig {
    #[serde(default = "default_deploy_manifest_port")]
    /// Context port that contains the deploy manifest input.
    pub deploy_manifest_port: String,

    #[serde(default = "default_configure_from_account_index")]
    /// Account index used to submit configuration transactions.
    pub from_account_index: usize,

    #[serde(default = "default_poll_interval_ms")]
    /// Delay between receipt polls in milliseconds.
    pub poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    /// Maximum number of receipt polls before timing out.
    pub max_receipt_polls: u64,

    #[serde(default = "default_config_report_export_key")]
    /// Context key used to export the configuration report.
    pub config_report_export_key: String,
}

/// Validates deploy runtime configuration before planning or execution.
pub fn validate_deploy_runtime_config(cfg: &AaveDeployRuntimeConfig) -> Result<(), String> {
    if cfg.compile_manifest_port.trim().is_empty() {
        return Err("compile_manifest_port must be non-empty".to_string());
    }
    if cfg.deploy_manifest_export_key.trim().is_empty() {
        return Err("deploy_manifest_export_key must be non-empty".to_string());
    }
    if cfg.max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

/// Validates configure runtime configuration before planning or execution.
pub fn validate_configure_runtime_config(cfg: &AaveConfigureRuntimeConfig) -> Result<(), String> {
    if cfg.deploy_manifest_port.trim().is_empty() {
        return Err("deploy_manifest_port must be non-empty".to_string());
    }
    if cfg.config_report_export_key.trim().is_empty() {
        return Err("config_report_export_key must be non-empty".to_string());
    }
    if cfg.max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

/// Decodes and validates an Aave compile manifest from JSON.
pub fn decode_compile_manifest(
    value: &serde_json::Value,
) -> Result<AaveCompileManifest, StateError> {
    let manifest: AaveCompileManifest = serde_json::from_value(value.clone()).map_err(|_| {
        op_errors::state_error(
            "compile_manifest_invalid",
            ErrorCategory::ParsingInput,
            false,
            "compile manifest was invalid",
        )
    })?;
    validate_compile_manifest(&manifest)?;
    Ok(manifest)
}

/// Decodes and validates an Aave deploy manifest from JSON.
pub fn decode_deploy_manifest(value: &serde_json::Value) -> Result<AaveDeployManifest, StateError> {
    let manifest: AaveDeployManifest = serde_json::from_value(value.clone()).map_err(|_| {
        op_errors::state_error(
            "deploy_manifest_invalid",
            ErrorCategory::ParsingInput,
            false,
            "deploy manifest was invalid",
        )
    })?;
    validate_deploy_manifest(&manifest)?;
    Ok(manifest)
}

/// Decodes and validates an origin deploy-output artifact from JSON.
pub fn decode_origin_deploy_output(
    value: &serde_json::Value,
) -> Result<AaveOriginDeployOutput, StateError> {
    let output: AaveOriginDeployOutput = serde_json::from_value(value.clone()).map_err(|_| {
        op_errors::state_error(
            "origin_deploy_output_invalid",
            ErrorCategory::ParsingInput,
            false,
            "origin deploy output was invalid",
        )
    })?;
    validate_origin_deploy_output(&output)?;
    Ok(output)
}

/// Validates structural and contract-level invariants for a compile manifest.
pub fn validate_compile_manifest(manifest: &AaveCompileManifest) -> Result<(), StateError> {
    if manifest.kind != COMPILE_MANIFEST_KIND {
        return Err(op_errors::state_error(
            "compile_manifest_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "compile manifest kind mismatch",
        ));
    }
    if manifest.contracts.is_empty() {
        return Err(op_errors::state_error(
            "compile_manifest_empty",
            ErrorCategory::ParsingInput,
            false,
            "compile manifest must contain at least one contract",
        ));
    }
    let mut seen: HashSet<String> = HashSet::new();
    for c in &manifest.contracts {
        if c.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "compile_manifest_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "compile manifest contract id must be non-empty",
            ));
        }
        if !seen.insert(c.id.clone()) {
            return Err(op_errors::state_error(
                "compile_manifest_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "compile manifest contract ids must be unique",
            ));
        }
        let _ = c.artifact.parse()?;
    }
    Ok(())
}

/// Validates structural and contract-level invariants for an origin deploy output.
pub fn validate_origin_deploy_output(output: &AaveOriginDeployOutput) -> Result<(), StateError> {
    if output.kind != ORIGIN_DEPLOY_OUTPUT_KIND {
        return Err(op_errors::state_error(
            "origin_deploy_output_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "origin deploy output kind mismatch",
        ));
    }
    if output.contracts.is_empty() {
        return Err(op_errors::state_error(
            "origin_deploy_output_empty",
            ErrorCategory::ParsingInput,
            false,
            "origin deploy output must contain at least one contract",
        ));
    }

    let mut seen: HashSet<String> = HashSet::new();
    for c in &output.contracts {
        if c.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "origin_deploy_output_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "origin deploy output contract id must be non-empty",
            ));
        }
        if !seen.insert(c.id.clone()) {
            return Err(op_errors::state_error(
                "origin_deploy_output_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "origin deploy output contract ids must be unique",
            ));
        }
        let _ = shared_dcv::normalize_address(&c.address).map_err(|_| {
            op_errors::state_error(
                "origin_deploy_output_contract_address_invalid",
                ErrorCategory::ParsingInput,
                false,
                "origin deploy output contract address was invalid",
            )
        })?;
        let _ = c.artifact.parse()?;
    }

    origin_contract_from_output(output, CONTRACT_USDC)?;
    origin_contract_from_output(output, CONTRACT_WBTC)?;
    origin_contract_from_output(output, CONTRACT_POOL)?;
    origin_contract_from_output(output, CONTRACT_USDC_A_TOKEN)?;
    origin_contract_from_output(output, CONTRACT_WBTC_A_TOKEN)?;
    origin_contract_from_output(output, CONTRACT_USDC_VARIABLE_DEBT_TOKEN)?;
    Ok(())
}

/// Validates structural and contract-level invariants for a deploy manifest.
pub fn validate_deploy_manifest(manifest: &AaveDeployManifest) -> Result<(), StateError> {
    if manifest.kind != DEPLOY_MANIFEST_KIND {
        return Err(op_errors::state_error(
            "deploy_manifest_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "deploy manifest kind mismatch",
        ));
    }
    if manifest.contracts.is_empty() {
        return Err(op_errors::state_error(
            "deploy_manifest_empty",
            ErrorCategory::ParsingInput,
            false,
            "deploy manifest must contain at least one contract",
        ));
    }
    let mut seen: HashSet<String> = HashSet::new();
    for c in &manifest.contracts {
        if c.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "deploy_manifest_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "deploy manifest contract id must be non-empty",
            ));
        }
        if !seen.insert(c.id.clone()) {
            return Err(op_errors::state_error(
                "deploy_manifest_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "deploy manifest contract ids must be unique",
            ));
        }
        let _ = shared_dcv::normalize_address(&c.address).map_err(|_| {
            op_errors::state_error(
                "deploy_manifest_contract_address_invalid",
                ErrorCategory::ParsingInput,
                false,
                "deploy manifest contract address was invalid",
            )
        })?;
        let _ = c.artifact.parse()?;
    }
    contract_from_manifest(manifest, CONTRACT_USDC)?;
    contract_from_manifest(manifest, CONTRACT_WBTC)?;
    contract_from_manifest(manifest, CONTRACT_POOL)?;
    Ok(())
}

/// Returns the deploy-manifest entry for `id`.
pub fn contract_from_manifest<'a>(
    manifest: &'a AaveDeployManifest,
    id: &str,
) -> Result<&'a AaveDeployManifestContract, StateError> {
    manifest
        .contracts
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| {
            op_errors::state_error(
                "deploy_manifest_missing_contract",
                ErrorCategory::ParsingInput,
                false,
                format!("deploy manifest missing contract: {id}"),
            )
        })
}

/// Returns the origin deploy-output entry for `id`.
pub fn origin_contract_from_output<'a>(
    output: &'a AaveOriginDeployOutput,
    id: &str,
) -> Result<&'a AaveOriginDeployOutputContract, StateError> {
    output.contracts.iter().find(|c| c.id == id).ok_or_else(|| {
        op_errors::state_error(
            "origin_deploy_output_missing_contract",
            ErrorCategory::ParsingInput,
            false,
            format!("origin deploy output missing contract: {id}"),
        )
    })
}
