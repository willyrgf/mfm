//! Generic compiled and deployed EVM contract-set manifests.
//!
//! These manifests are the reusable boundary between external build backends and MFM-native
//! deployment/runtime flows. Domain-specific wrappers may emit them, but the shapes themselves are
//! intentionally protocol-agnostic.

use std::collections::HashSet;

use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_state_common::errors as op_errors;
use serde::{Deserialize, Serialize};

use mfm_evm_dcv_model::{self as shared_dcv, ContractArtifactConfig};

/// Kind string for compiled contract-set manifests.
pub const COMPILED_CONTRACT_SET_KIND: &str = "evm_contract_set_compile_manifest_v1";
/// Kind string for deployed contract-set manifests.
pub const DEPLOYED_CONTRACT_SET_KIND: &str = "evm_contract_set_deploy_manifest_v1";

/// Single contract entry inside a compiled contract-set manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledContractSetEntry {
    /// Stable contract identifier preserved into the deployed manifest.
    pub id: String,
    /// ABI and bytecode payload used for deployment.
    pub artifact: ContractArtifactConfig,
    /// Constructor arguments passed to the deployment call.
    #[serde(default)]
    pub constructor_args: Vec<serde_json::Value>,
}

/// Compiled contract-set manifest consumed by generic deployment flows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledContractSetManifest {
    /// Manifest kind discriminator.
    pub kind: String,
    /// Contracts available for deployment.
    pub contracts: Vec<CompiledContractSetEntry>,
}

/// Single deployed contract entry emitted after generic contract-set deployment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeployedContractSetEntry {
    /// Stable contract identifier preserved from the compiled manifest.
    pub id: String,
    /// Deployed contract address.
    pub address: String,
    /// Deployment transaction hash.
    pub deploy_tx_hash: String,
    /// Deployment receipt.
    pub deploy_receipt: serde_json::Value,
    /// ABI and bytecode payload for downstream configure/validate steps.
    pub artifact: ContractArtifactConfig,
}

/// Deployed contract-set manifest produced by generic deployment flows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeployedContractSetManifest {
    /// Manifest kind discriminator.
    pub kind: String,
    /// Deployed contracts in this manifest.
    pub contracts: Vec<DeployedContractSetEntry>,
}

/// Decodes and validates a compiled contract-set manifest from JSON.
pub fn decode_compiled_contract_set_manifest(
    value: &serde_json::Value,
) -> Result<CompiledContractSetManifest, StateError> {
    let manifest: CompiledContractSetManifest =
        serde_json::from_value(value.clone()).map_err(|_| {
            op_errors::state_error(
                "compiled_contract_set_invalid",
                ErrorCategory::ParsingInput,
                false,
                "compiled contract-set manifest was invalid",
            )
        })?;
    validate_compiled_contract_set_manifest(&manifest)?;
    Ok(manifest)
}

/// Decodes and validates a deployed contract-set manifest from JSON.
pub fn decode_deployed_contract_set_manifest(
    value: &serde_json::Value,
) -> Result<DeployedContractSetManifest, StateError> {
    let manifest: DeployedContractSetManifest =
        serde_json::from_value(value.clone()).map_err(|_| {
            op_errors::state_error(
                "deployed_contract_set_invalid",
                ErrorCategory::ParsingInput,
                false,
                "deployed contract-set manifest was invalid",
            )
        })?;
    validate_deployed_contract_set_manifest(&manifest)?;
    Ok(manifest)
}

/// Validates structural and contract-level invariants for compiled contract-set manifests.
pub fn validate_compiled_contract_set_manifest(
    manifest: &CompiledContractSetManifest,
) -> Result<(), StateError> {
    if manifest.kind != COMPILED_CONTRACT_SET_KIND {
        return Err(op_errors::state_error(
            "compiled_contract_set_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "compiled contract-set manifest kind mismatch",
        ));
    }
    if manifest.contracts.is_empty() {
        return Err(op_errors::state_error(
            "compiled_contract_set_empty",
            ErrorCategory::ParsingInput,
            false,
            "compiled contract-set manifest must contain at least one contract",
        ));
    }

    let mut seen: HashSet<String> = HashSet::new();
    for contract in &manifest.contracts {
        if contract.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "compiled_contract_set_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "compiled contract-set contract id must be non-empty",
            ));
        }
        if !seen.insert(contract.id.clone()) {
            return Err(op_errors::state_error(
                "compiled_contract_set_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "compiled contract-set contract ids must be unique",
            ));
        }
        shared_dcv::parse_artifact(&contract.artifact).map_err(|_| {
            op_errors::state_unknown(
                "invalid_contract_artifact",
                "compiled contract-set artifact was invalid",
            )
        })?;
    }
    Ok(())
}

/// Validates structural and contract-level invariants for deployed contract-set manifests.
pub fn validate_deployed_contract_set_manifest(
    manifest: &DeployedContractSetManifest,
) -> Result<(), StateError> {
    if manifest.kind != DEPLOYED_CONTRACT_SET_KIND {
        return Err(op_errors::state_error(
            "deployed_contract_set_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "deployed contract-set manifest kind mismatch",
        ));
    }
    if manifest.contracts.is_empty() {
        return Err(op_errors::state_error(
            "deployed_contract_set_empty",
            ErrorCategory::ParsingInput,
            false,
            "deployed contract-set manifest must contain at least one contract",
        ));
    }

    let mut seen: HashSet<String> = HashSet::new();
    for contract in &manifest.contracts {
        if contract.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "deployed_contract_set_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "deployed contract-set contract id must be non-empty",
            ));
        }
        if !seen.insert(contract.id.clone()) {
            return Err(op_errors::state_error(
                "deployed_contract_set_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "deployed contract-set contract ids must be unique",
            ));
        }
        shared_dcv::normalize_address(&contract.address).map_err(|_| {
            op_errors::state_error(
                "deployed_contract_set_contract_address_invalid",
                ErrorCategory::ParsingInput,
                false,
                "deployed contract-set contract address was invalid",
            )
        })?;
        shared_dcv::parse_artifact(&contract.artifact).map_err(|_| {
            op_errors::state_unknown(
                "invalid_contract_artifact",
                "deployed contract-set artifact was invalid",
            )
        })?;
    }
    Ok(())
}
