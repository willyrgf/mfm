use super::*;

/// Contract stage produced and consumed by the direct deploy, configure, and validate states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "lifecycle-stage",
    schema = "mfm.evm.contract.value.lifecycle_stage"
)]
pub enum ContractLifecycleStage {
    /// Deployed contract instance stage.
    Deployed,
    /// Configured contract instance stage.
    Configured,
}

/// Context-bound deployed contract instance emitted by the deploy state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deployed-contract-instance",
    schema = "mfm.evm.contract.value.deployed_contract_instance"
)]
pub struct DeployedContractInstance {
    /// Value contract version.
    pub lifecycle_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized contract address.
    pub address: ContractAddress,
    /// Block number that confirmed direct deployment.
    pub deployed_block_number: u64,
}

/// Direct deployed-value lineage consumed by the configure state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-from",
    schema = "mfm.evm.contract.value.configured_from"
)]
pub struct ConfiguredFrom {
    /// Deployed instance context ref.
    pub deployed_context_ref: ContextRefValue,
    /// Deployed instance address.
    pub deployed_address: ContractAddress,
}

/// Context-bound configured contract instance emitted by the configure state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-instance",
    schema = "mfm.evm.contract.value.configured_contract_instance"
)]
pub struct ConfiguredContractInstance {
    /// Value contract version.
    pub lifecycle_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized contract address.
    pub address: ContractAddress,
    /// Direct deployed-value lineage.
    pub configured_from: ConfiguredFrom,
}

/// Configured instance identity consumed by validation reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-instance-ref",
    schema = "mfm.evm.contract.value.configured_contract_instance_ref"
)]
pub struct ConfiguredContractInstanceRef {
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Normalized configured contract address.
    pub address: ContractAddress,
    /// Direct deployed-value lineage.
    pub configured_from: ConfiguredFrom,
}

impl ConfiguredContractInstanceRef {
    /// Builds a configured instance reference from a configured instance.
    pub fn from_configured(configured: &ConfiguredContractInstance) -> Self {
        Self {
            context_ref: configured.context_ref.clone(),
            address: configured.address.clone(),
            configured_from: configured.configured_from.clone(),
        }
    }
}

/// Redacted source identity for a contract validation read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-source-evidence",
    schema = "mfm.evm.contract.value.validation_source_evidence"
)]
pub struct ValidationSourceEvidence {
    /// Semantic network id captured from the bound provider evidence.
    pub network_id: String,
    /// Expected EVM chain id captured from the certified network context.
    pub expected_chain_id: u64,
    /// Observed EVM chain id reported by the provider.
    pub observed_chain_id: u64,
    /// Redacted provider source reference.
    pub source_ref: String,
    /// Redacted provider source policy id.
    pub policy_id: String,
}

/// Replayable evidence for one validation call assertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-read-evidence",
    schema = "mfm.evm.contract.value.validation_read_evidence"
)]
pub struct ValidationReadEvidence {
    /// Redacted source evidence for the provider read.
    pub source: ValidationSourceEvidence,
    /// Decoded assertion result.
    pub result: ValidationReadResult,
}

/// Replayable evidence for one validation log assertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-event-evidence",
    schema = "mfm.evm.contract.value.validation_event_evidence"
)]
pub struct ValidationEventEvidence {
    /// Redacted source evidence for the provider read.
    pub source: ValidationSourceEvidence,
    /// Decoded assertion result.
    pub result: ValidationEventResult,
}

/// Context-bound terminal validation report for a configured contract instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-bound-validation-report",
    schema = "mfm.evm.contract.value.context_bound_validation_report"
)]
pub struct ContextBoundValidationReport {
    /// Validation report contract version.
    pub report_version: u64,
    /// Certified context ref.
    pub context_ref: ContextRefValue,
    /// Configured contract instance that was validated.
    pub configured_instance: ConfiguredContractInstanceRef,
    /// Observed EVM chain id from validation read evidence.
    pub observed_chain_id: u64,
    /// Additional read assertion results from validation action.
    pub read_results: Vec<ValidationReadResult>,
    /// Additional event assertion results from validation action.
    pub event_results: Vec<ValidationEventResult>,
    /// Replayable read assertion evidence from validation reads.
    pub validation_read_evidence: Vec<ValidationReadEvidence>,
    /// Replayable event assertion evidence from validation log reads.
    pub validation_event_evidence: Vec<ValidationEventEvidence>,
    /// Whether all validation checks passed.
    pub valid: bool,
}
