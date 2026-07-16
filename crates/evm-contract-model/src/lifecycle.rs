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
    /// Canonical block hash that confirmed direct deployment.
    pub deployed_block_hash: EvmBlockHash,
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

/// Canonical finalized chain anchor selected for a configured contract instance.
///
/// The configure state selects the last successful configuration receipt in
/// certified transaction order. When configuration submits no transactions, it
/// carries forward the direct deployment receipt anchor instead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configured-contract-anchor",
    schema = "mfm.evm.contract.value.configured_contract_anchor"
)]
pub struct ConfiguredContractAnchor {
    /// Finalized canonical block number.
    pub block_number: u64,
    /// Finalized canonical block hash at `block_number`.
    pub block_hash: EvmBlockHash,
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
    /// Canonical finalized receipt anchor for exact validation reads.
    pub anchor: ConfiguredContractAnchor,
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
    /// Canonical finalized receipt anchor for exact validation reads.
    pub anchor: ConfiguredContractAnchor,
}

impl ConfiguredContractInstanceRef {
    /// Builds a configured instance reference from a configured instance.
    pub fn from_configured(configured: &ConfiguredContractInstance) -> Self {
        Self {
            context_ref: configured.context_ref.clone(),
            address: configured.address.clone(),
            configured_from: configured.configured_from.clone(),
            anchor: configured.anchor.clone(),
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

/// Exact canonical selector used for a deployed-runtime-code identity read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-code-identity-selector",
    schema = "mfm.evm.contract.value.validation_code_identity_selector"
)]
pub struct ValidationCodeIdentitySelector {
    /// Configured contract address read by the adapter.
    pub address: ContractAddress,
    /// Canonical block number retained with the configured anchor.
    pub block_number: u64,
    /// Exact canonical block hash supplied to the EIP-1898 selector.
    pub block_hash: EvmBlockHash,
    /// Whether the code provider was required to reject a non-canonical block.
    pub require_canonical: bool,
}

/// Optional deployed-runtime-code identity assertion requested by validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-code-identity-request",
    schema = "mfm.evm.contract.intent.validation_code_identity_request"
)]
pub struct ValidationCodeIdentityRequest {
    /// Expected Keccak-256 hash from the certified contract profile.
    pub expected_code_hash: EvmCodeHash,
    /// Exact address and canonical block selector for the code read.
    pub selector: ValidationCodeIdentitySelector,
}

/// Retained external evidence for one exact deployed-runtime-code identity read.
///
/// `runtime_bytecode` is replay authority and is retained separately as an
/// external-read evidence artifact. Terminal validation reports intentionally
/// project only its length and Keccak-256 hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-code-identity-evidence",
    schema = "mfm.evm.contract.value.validation_code_identity_evidence"
)]
pub struct ValidationCodeIdentityEvidence {
    /// Evidence contract version.
    pub evidence_version: u64,
    /// Exact address and EIP-1898 block selector used for the read.
    pub selector: ValidationCodeIdentitySelector,
    /// Redacted source evidence for the provider read.
    pub source: ValidationSourceEvidence,
    /// Raw runtime bytecode returned by `eth_getCode`.
    pub runtime_bytecode: Vec<u8>,
    /// Byte length recomputed from `runtime_bytecode` by the adapter.
    pub observed_byte_len: u64,
    /// Keccak-256 hash recomputed from `runtime_bytecode` by the adapter.
    pub observed_code_hash: EvmCodeHash,
}

/// Public projection of an optional deployed-runtime-code identity observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-code-identity-report",
    schema = "mfm.evm.contract.value.validation_code_identity_report"
)]
pub struct ValidationCodeIdentityReport {
    /// Observed runtime bytecode length.
    pub observed_byte_len: u64,
    /// Observed runtime bytecode Keccak-256 hash.
    pub observed_code_hash: EvmCodeHash,
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
    /// Optional deployed-runtime-code identity projection.
    ///
    /// This is absent when the certified contract profile does not require a
    /// deployed-code hash assertion.
    pub code_identity: Option<ValidationCodeIdentityReport>,
    /// Whether all validation checks passed.
    pub valid: bool,
}
