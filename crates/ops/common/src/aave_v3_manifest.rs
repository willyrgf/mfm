use std::collections::HashSet;

use mfm_machine::errors::{ErrorCategory, StateError};
use serde::{Deserialize, Serialize};

use crate::abi as common_abi;
use crate::errors as op_errors;
use crate::evm_dcv as shared_dcv;

pub const COMPILE_MANIFEST_KIND: &str = "aave_v3_origin_compile_manifest_v1";
pub const DEPLOY_MANIFEST_KIND: &str = "aave_v3_deploy_manifest_v1";
pub const ORIGIN_DEPLOY_OUTPUT_KIND: &str = "aave_v3_origin_deploy_output_v1";
pub const CONFIG_REPORT_KIND: &str = "aave_v3_config_report_v1";

pub const CONTRACT_USDC: &str = "usdc";
pub const CONTRACT_WBTC: &str = "wbtc";
pub const CONTRACT_POOL: &str = "pool";
pub const CONTRACT_USDC_A_TOKEN: &str = "usdc_a_token";
pub const CONTRACT_WBTC_A_TOKEN: &str = "wbtc_a_token";
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractArtifactJson {
    pub abi: serde_json::Value,
    pub bytecode: serde_json::Value,
}

impl ContractArtifactJson {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveCompileManifestContract {
    pub id: String,
    pub artifact: ContractArtifactJson,
    #[serde(default)]
    pub constructor_args: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveCompileManifest {
    pub kind: String,
    pub contracts: Vec<AaveCompileManifestContract>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployManifestContract {
    pub id: String,
    pub address: String,
    pub deploy_tx_hash: String,
    pub deploy_receipt: serde_json::Value,
    pub artifact: ContractArtifactJson,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployManifest {
    pub kind: String,
    pub contracts: Vec<AaveDeployManifestContract>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveOriginDeployOutputContract {
    pub id: String,
    pub address: String,
    pub artifact: ContractArtifactJson,
    #[serde(default)]
    pub deploy_tx_hash: Option<String>,
    #[serde(default)]
    pub deploy_receipt: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveOriginDeployOutput {
    pub kind: String,
    pub contracts: Vec<AaveOriginDeployOutputContract>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigCallRecord {
    pub function: String,
    pub tx_hash: String,
    pub receipt: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigReport {
    pub kind: String,
    pub pool: String,
    pub usdc: String,
    pub wbtc: String,
    pub calls: Vec<AaveConfigCallRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployRuntimeConfig {
    #[serde(default = "default_compile_manifest_port")]
    pub compile_manifest_port: String,

    #[serde(default = "default_deployer_account_index")]
    pub deployer_account_index: usize,

    #[serde(default)]
    pub signing_key_env: Option<String>,

    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    pub max_receipt_polls: u64,

    #[serde(default = "default_deploy_manifest_export_key")]
    pub deploy_manifest_export_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigureRuntimeConfig {
    #[serde(default = "default_deploy_manifest_port")]
    pub deploy_manifest_port: String,

    #[serde(default = "default_configure_from_account_index")]
    pub from_account_index: usize,

    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    pub max_receipt_polls: u64,

    #[serde(default = "default_config_report_export_key")]
    pub config_report_export_key: String,
}

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
