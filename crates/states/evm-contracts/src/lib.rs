#![warn(missing_docs)]
//! Typed EVM contract lifecycle state contracts.
//!
//! This crate owns reusable deploy, configure, and validate state contracts for EVM
//! contracts. The state definitions expose semantic intents, typestate values, adapter
//! bindings, and capability sets only. Live RPC, signer-provider resolution, artifact-store
//! access, and transaction submission are adapter responsibilities.
//!
//! ```rust
//! use mfm_program::StateSpec;
//! use mfm_state_evm_contracts::ContextBoundDeployContractState;
//!
//! assert_eq!(
//!     ContextBoundDeployContractState::name(),
//!     "mfm.evm.contract.context_deploy"
//! );
//! ```

use std::future;

use mfm_adapter_contracts::evm_contract_lifecycle_adapter_binding;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::CapabilitySetFor;
use mfm_effects::{ApplySideEffect, ReadExternal};
use mfm_evm_capabilities::{
    EvmCallReadCapability, EvmChainIdentityCapability, EvmCodeReadCapability, EvmFeeReadCapability,
    EvmGasEstimateCapability, EvmLogsReadCapability, EvmNonceOccupancyReadCapability,
    EvmNonceReadCapability, EvmReceiptReadCapability, EvmTransactionSubmitCapability,
};
use mfm_evm_contract_config::{
    ConfigureAction, DeployAction, EvmTransactionPolicy, ImportConfiguredSpec, ImportDeployedSpec,
    ValidateAction,
};
use mfm_evm_contract_model::{
    configured_contract_stage, contract_instance_resource_kind, deployed_contract_stage,
    expected_matches, validation_report_resource_kind, validation_report_stage,
    AdoptExternalAddress, ConfigurationClaim, ConfigurationSnapshot, ConfiguredContractInstance,
    ConfiguredContractInstanceRef, ConfiguredFrom, ContextBoundValidationReport, ContractAddress,
    ContractCallConfig, ContractLifecycleStage, ContractProfileDigestRef, DeployProvenance,
    DeployedContractInstance, EventAssertionConfig, EvmContractContext, ExternalAdoptionEvidence,
    ExternalEventAssertionEvidence, ExternalEvmSourceEvidence, ExternalReadAssertionEvidence,
    ImportFromMfmRun, ImportFromMfmRunEvidence, LifecycleArtifactEvidenceRef, LifecycleNodeIdRef,
    ReadAssertionConfig, ValidationEventResult, ValidationReadResult,
};
use mfm_ids::{
    ContextResourceKind, ContextStage, DescriptorId, DigestAlgorithm, SchemaId, StateKind,
    StateVersion,
};
use mfm_program::{
    AdapterBindingSpec, IdempotencyKey, ReadState, ResourceClaim, ResourceNamespace,
    SideEffectState, StateError, StateResult, StateSpec,
};
use mfm_program_derive::{MfmValue, StateInput};
use mfm_signing::SigningCapability;
use mfm_values::{ContextBoundOutput, ContextRefValue};
use serde::{Deserialize, Serialize};

#[path = "mutations.rs"]
mod mutations;
pub use self::mutations::{ContextBoundConfigureContractState, ContextBoundDeployContractState};

#[path = "validation.rs"]
mod validation;
pub use self::validation::ContextBoundValidateContractState;

#[path = "imports.rs"]
mod imports;
pub use self::imports::{ImportConfiguredContractState, ImportDeployedContractState};

const NAMESPACE: &str = "mfm.evm.contract";
const ACCOUNT_NONCE_RESOURCE_NAMESPACE: &str = "mfm.evm.contract.account_nonce";
const ACCOUNT_NONCE_RESOURCE_KEY_SCHEMA: &str = "mfm.evm.contract.resource_key.account_nonce";

type ContractMutationCaps = (
    EvmChainIdentityCapability,
    EvmNonceReadCapability,
    EvmFeeReadCapability,
    EvmGasEstimateCapability,
    SigningCapability,
    EvmTransactionSubmitCapability,
    EvmReceiptReadCapability,
    EvmNonceOccupancyReadCapability,
);

type ContractValidationReadCaps = (
    EvmChainIdentityCapability,
    EvmCallReadCapability,
    EvmLogsReadCapability,
);

type ContractImportReadCaps = (
    EvmChainIdentityCapability,
    EvmCodeReadCapability,
    EvmCallReadCapability,
    EvmLogsReadCapability,
);

fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
    Ok(vec![AdapterBindingSpec {
        adapter_kind: binding.adapter_kind().clone(),
        adapter_version: binding.adapter_version().clone(),
    }])
}

fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.evm.contract.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.evm.contract.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

/// Returns the exclusive resource namespace for signer account nonce mutation lanes.
pub fn account_nonce_resource_namespace() -> mfm_program::Result<ResourceNamespace> {
    ResourceNamespace::new(ACCOUNT_NONCE_RESOURCE_NAMESPACE)
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

/// Returns the schema id for signer account nonce resource-key evidence.
pub fn account_nonce_resource_key_schema_id() -> mfm_program::Result<SchemaId> {
    SchemaId::new(
        ACCOUNT_NONCE_RESOURCE_KEY_SCHEMA,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(ACCOUNT_NONCE_RESOURCE_KEY_SCHEMA.as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

/// Returns the exclusive side-effect claim used by deploy/configure mutations.
pub fn account_nonce_resource_claim() -> mfm_program::Result<ResourceClaim> {
    Ok(ResourceClaim::exclusive(
        account_nonce_resource_namespace()?,
        account_nonce_resource_key_schema_id()?,
    ))
}

fn adapter_required_error(state_name: &'static str) -> StateError {
    StateError::Message(format!(
        "{state_name} requires an EVM contract lifecycle adapter runner"
    ))
}

fn requires_context(
    resource_kind: &ContextResourceKind,
    stage: &ContextStage,
    producer_descriptor_ids: Vec<DescriptorId>,
) -> mfm_program::Result<mfm_program::StateInputContextContractSpec> {
    Ok(mfm_program::StateInputContextContractSpec::Required {
        resource_kind: resource_kind.clone(),
        stage: stage.clone(),
        producer: Box::new(mfm_program::ContextProducerSpec {
            producer_descriptor_ids: sorted_descriptor_ids(producer_descriptor_ids),
            seed_producers_allowed: false,
        }),
    })
}

fn produces_context(
    resource_kind: &ContextResourceKind,
    stage: &ContextStage,
) -> mfm_program::Result<mfm_program::StateOutputContextContractSpec> {
    Ok(mfm_program::StateOutputContextContractSpec::Produces {
        resource_kind: resource_kind.clone(),
        stage: stage.clone(),
    })
}

/// Input consumed by context-bound contract configuration states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.contract.input.context_configure")]
pub struct ContextConfigureContractInput {
    /// Context-bound deployed contract emitted by deploy or import-deployed states.
    pub deployed: DeployedContractInstance,
}

/// Input consumed by context-bound contract validation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.contract.input.context_validate")]
pub struct ContextValidateContractInput {
    /// Context-bound configured contract emitted by configure or import-configured states.
    pub configured: ConfiguredContractInstance,
}

/// Deterministic EVM transaction intent bound to a certified contract context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-transaction-intent",
    schema = "mfm.evm.contract.intent.context_transaction"
)]
pub struct ContextContractTransactionIntent {
    /// Intent contract version.
    pub intent_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Stable semantic network id from the certified context.
    pub network_id: String,
    /// Expected EVM chain id from the certified context.
    pub expected_chain_id: u64,
    /// Process-local signer reference from the action config.
    pub signer_ref: String,
    /// Expected signer EVM address from the action config.
    pub expected_signer_address: String,
    /// Optional destination contract address; absent for contract creation.
    pub to_address: Option<String>,
    /// Native token value expressed in wei.
    pub value_wei: Option<String>,
    /// Optional ABI or initcode payload reference, never a signed transaction.
    pub data_ref: Option<String>,
    /// Fee and gas policy.
    pub transaction: EvmTransactionPolicy,
}

/// Deploy mutation intent prepared by [`ContextBoundDeployContractState`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-deploy-intent",
    schema = "mfm.evm.contract.intent.context_deploy"
)]
pub struct ContextContractDeployIntent {
    /// Intent contract version.
    pub intent_version: u64,
    /// Generic certified-context transaction intent.
    pub transaction: ContextContractTransactionIntent,
    /// Constructor arguments retained for adapter-side ABI encoding.
    pub constructor_args_len: u64,
    /// Contract profile id from the certified context.
    pub contract_profile_id: String,
}

/// Configure mutation intent prepared by [`ContextBoundConfigureContractState`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-configure-intent",
    schema = "mfm.evm.contract.intent.context_configure"
)]
pub struct ContextContractConfigureIntent {
    /// Intent contract version.
    pub intent_version: u64,
    /// Context-bound deployed contract being configured.
    pub deployed: DeployedContractInstance,
    /// Generic transaction intents, one for each configured call.
    pub transactions: Vec<ContextContractTransactionIntent>,
}

/// Idempotency input for contract lifecycle mutation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-idempotency",
    schema = "mfm.evm.contract.intent.transaction_idempotency"
)]
pub struct ContractTransactionIdempotency {
    /// Stable intent digest or transaction-domain key.
    pub key: String,
}

/// Redaction-safe transaction submission result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-submission",
    schema = "mfm.evm.contract.value.transaction_submission"
)]
pub struct ContractTransactionSubmission {
    /// Submission contract version.
    pub submission_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being mutated.
    pub resource_stage: ContractLifecycleStage,
    /// Submitted transaction hash.
    pub transaction_hash: String,
    /// Public signer metadata, when supplied by the signer provider.
    pub signer_public_key: Option<String>,
}

/// Redaction-safe aggregate transaction submission result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-submissions",
    schema = "mfm.evm.contract.value.transaction_submissions"
)]
pub struct ContractTransactionSubmissions {
    /// Aggregate contract version.
    pub submissions_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being mutated.
    pub resource_stage: ContractLifecycleStage,
    /// Submitted transaction evidence in deterministic transaction order.
    pub transactions: Vec<ContractTransactionSubmission>,
}

/// Redaction-safe transaction receipt evidence summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-receipt",
    schema = "mfm.evm.contract.value.transaction_receipt"
)]
pub struct ContractTransactionReceipt {
    /// Receipt contract version.
    pub receipt_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being mutated.
    pub resource_stage: ContractLifecycleStage,
    /// Transaction hash.
    pub transaction_hash: String,
    /// Block number that included the transaction.
    pub block_number: u64,
    /// Whether the transaction succeeded.
    pub status: bool,
    /// Typed artifact evidence reference for the retained receipt.
    pub receipt_evidence: Option<LifecycleArtifactEvidenceRef>,
}

/// Deployment receipt-level terminal evidence consumed by context-bound deploy states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-receipt",
    schema = "mfm.evm.contract.value.deploy_receipt"
)]
pub struct ContractDeployReceipt {
    /// Receipt-level contract version.
    pub receipt_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being produced.
    pub resource_stage: ContractLifecycleStage,
    /// Deployed contract address derived from prepared invocation evidence.
    pub contract_address: String,
    /// Observed transaction receipt.
    pub receipt: ContractTransactionReceipt,
}

/// Deployment confirmation consumed by context-bound deploy states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-confirmation",
    schema = "mfm.evm.contract.value.deploy_confirmation"
)]
pub struct ContractDeployConfirmation {
    /// Confirmation contract version.
    pub confirmation_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being produced.
    pub resource_stage: ContractLifecycleStage,
    /// Confirmations proven by the confirmation adapter when this artifact was recorded.
    pub confirmations: u64,
    /// Deployed contract address.
    pub contract_address: String,
    /// Confirmed transaction receipt.
    pub receipt: ContractTransactionReceipt,
}

/// Configuration receipt-level terminal evidence for context-bound configure states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-configure-receipt",
    schema = "mfm.evm.contract.value.context_configure_receipt"
)]
pub struct ContextContractConfigureReceipt {
    /// Receipt-level contract version.
    pub receipt_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being produced.
    pub resource_stage: ContractLifecycleStage,
    /// Configure node id captured by certified invocation evidence.
    pub configure_node: LifecycleNodeIdRef,
    /// Observed transaction receipts.
    pub receipts: Vec<ContractTransactionReceipt>,
    /// Configuration call evidence refs.
    pub call_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    /// Confirmation read/event evidence refs.
    pub confirmation_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    /// Highest observed configuration block, when known.
    pub configured_block_number: Option<u64>,
}

/// Configuration confirmation for context-bound configure states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-configure-confirmation",
    schema = "mfm.evm.contract.value.context_configure_confirmation"
)]
pub struct ContextContractConfigureConfirmation {
    /// Confirmation contract version.
    pub confirmation_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being produced.
    pub resource_stage: ContractLifecycleStage,
    /// Confirmations proven by the confirmation adapter when this artifact was recorded.
    pub confirmations: u64,
    /// Configure node id captured by certified invocation evidence.
    pub configure_node: LifecycleNodeIdRef,
    /// Confirmed transaction receipts.
    pub receipts: Vec<ContractTransactionReceipt>,
    /// Configuration call evidence refs.
    pub call_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    /// Confirmation read/event evidence refs.
    pub confirmation_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    /// Highest observed configuration block, when known.
    pub configured_block_number: Option<u64>,
}

/// Read request used by context-bound contract validation adapters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "context-validation-read-request",
    schema = "mfm.evm.contract.intent.context_validation_read_request"
)]
pub struct ContextContractValidationReadRequest {
    /// Request contract version.
    pub request_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Digest of the configured input value this read request was derived from.
    pub configured_input_digest: ContractProfileDigestRef,
    /// Configured contract instance being validated.
    pub configured_instance: ConfiguredContractInstanceRef,
    /// Stable semantic network id from the certified context.
    pub network_id: String,
    /// Expected EVM chain id from the certified context.
    pub expected_chain_id: u64,
    /// Read assertions evaluated by the adapter.
    pub read_assertions: Vec<ReadAssertionConfig>,
    /// Event assertions evaluated by the adapter.
    pub event_assertions: Vec<EventAssertionConfig>,
}

/// Read response used to project a validation report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-read-response",
    schema = "mfm.evm.contract.value.validation_read_response"
)]
pub struct ContractValidationReadResponse {
    /// Response contract version.
    pub response_version: u64,
    /// Certified contract lifecycle context ref.
    pub context_ref: ContextRefValue,
    /// Digest of the configured input value this response is bound to.
    pub configured_input_digest: ContractProfileDigestRef,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being validated.
    pub resource_stage: ContractLifecycleStage,
    /// Observed EVM chain id.
    pub observed_chain_id: u64,
    /// Redaction-safe client version label.
    pub client_version: String,
    /// Results for configured-intent read confirmations.
    pub configuration_read_results: Vec<ValidationReadResult>,
    /// Results for configured-intent event confirmations.
    pub configuration_event_results: Vec<ValidationEventResult>,
    /// Results for validation read assertions.
    pub read_results: Vec<ValidationReadResult>,
    /// Results for validation event assertions.
    pub event_results: Vec<ValidationEventResult>,
    /// Replayable read assertion evidence.
    pub validation_read_evidence: Vec<ExternalReadAssertionEvidence>,
    /// Replayable event assertion evidence.
    pub validation_event_evidence: Vec<ExternalEventAssertionEvidence>,
    /// Retained lifecycle evidence refs available to the terminal report.
    pub evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
}

/// Recomputes whether a read validation result passes.
pub fn validation_read_result_passes(result: &ValidationReadResult) -> bool {
    expected_matches(&result.actual, &result.expected)
}

/// Recomputes whether an event validation result passes.
pub fn validation_event_result_passes(result: &ValidationEventResult) -> bool {
    result.observed_count >= result.min_count
}

/// Requires a read validation result's retained `passed` field to match recomputation.
pub fn require_validation_read_result_canonical_passed(
    result: &ValidationReadResult,
) -> StateResult<()> {
    if result.passed == validation_read_result_passes(result) {
        Ok(())
    } else {
        Err(StateError::Message(
            "validation read result passed flag does not match recomputed value".to_owned(),
        ))
    }
}

/// Requires an event validation result's retained `passed` field to match recomputation.
pub fn require_validation_event_result_canonical_passed(
    result: &ValidationEventResult,
) -> StateResult<()> {
    if result.passed == validation_event_result_passes(result) {
        Ok(())
    } else {
        Err(StateError::Message(
            "validation event result passed flag does not match recomputed value".to_owned(),
        ))
    }
}

fn require_validation_read_results_canonical_passed(
    results: &[ValidationReadResult],
) -> StateResult<()> {
    for result in results {
        require_validation_read_result_canonical_passed(result)?;
    }
    Ok(())
}

fn require_validation_event_results_canonical_passed(
    results: &[ValidationEventResult],
) -> StateResult<()> {
    for result in results {
        require_validation_event_result_canonical_passed(result)?;
    }
    Ok(())
}

fn descriptor_id_for_state<S>() -> mfm_program::Result<DescriptorId>
where
    S: StateSpec,
    S::Effect: mfm_program::EffectRunner<S>,
    S::Caps: CapabilitySetFor<S::Effect>,
{
    mfm_program::state_descriptor::<S>()
        .map(|descriptor| descriptor.descriptor_id().clone())
        .map_err(|error| mfm_program::PlanError::Registry(error.to_string()))
}

fn sorted_descriptor_ids(mut ids: Vec<DescriptorId>) -> Vec<DescriptorId> {
    ids.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    ids.dedup_by(|left, right| left.as_str() == right.as_str());
    ids
}

fn context_transaction_intent_from_deploy_action(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    action: &DeployAction,
    data_ref: Option<String>,
) -> ContextContractTransactionIntent {
    ContextContractTransactionIntent {
        intent_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        network_id: context.value().network.network_id.as_str().to_owned(),
        expected_chain_id: context.value().network.expected_chain_id(),
        signer_ref: action.signer().signer_ref_str().to_owned(),
        expected_signer_address: action.signer().expected_signer_address_str().to_owned(),
        to_address: None,
        value_wei: action.value_wei().map(ToOwned::to_owned),
        data_ref,
        transaction: action.transaction().clone(),
    }
}

fn context_transaction_intent_from_configure_call(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    action: &ConfigureAction,
    deployed: &DeployedContractInstance,
    call: &ContractCallConfig,
) -> ContextContractTransactionIntent {
    ContextContractTransactionIntent {
        intent_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        network_id: context.value().network.network_id.as_str().to_owned(),
        expected_chain_id: context.value().network.expected_chain_id(),
        signer_ref: action.signer().signer_ref_str().to_owned(),
        expected_signer_address: action.signer().expected_signer_address_str().to_owned(),
        to_address: Some(deployed.address.as_str().to_owned()),
        value_wei: call.value_wei.as_ref().map(ToString::to_string),
        data_ref: Some(call.function.to_string()),
        transaction: action.transaction().clone(),
    }
}

fn deployed_instance_from_receipt(
    contract_address: &str,
    receipt: &ContractTransactionReceipt,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> StateResult<DeployedContractInstance> {
    let address = ContractAddress::new(contract_address)
        .map_err(|error| StateError::Message(error.to_string()))?;
    let deploy_evidence = receipt
        .receipt_evidence
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(DeployedContractInstance {
        lifecycle_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        address,
        deploy_provenance: DeployProvenance::MfmDeploy {
            deploy_tx_hash: receipt.transaction_hash.clone(),
        },
        deploy_evidence,
        external_adoption_evidence: None,
        deployed_block_number: Some(receipt.block_number),
    })
}

struct ContextConfigureOutputEvidence<'a> {
    receipts: &'a [ContractTransactionReceipt],
    configured_block_number: Option<u64>,
    configure_node: LifecycleNodeIdRef,
    call_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
    confirmation_evidence_refs: Vec<LifecycleArtifactEvidenceRef>,
}

fn configured_instance_from_evidence(
    action: &ConfigureAction,
    input: &ContextConfigureContractInput,
    evidence: ContextConfigureOutputEvidence<'_>,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> StateResult<ConfiguredContractInstance> {
    let mut retained_evidence = evidence
        .receipts
        .iter()
        .filter_map(|receipt| receipt.receipt_evidence.clone())
        .collect::<Vec<_>>();
    retained_evidence.extend(evidence.call_evidence_refs.iter().cloned());
    retained_evidence.extend(evidence.confirmation_evidence_refs.iter().cloned());

    Ok(ConfiguredContractInstance {
        lifecycle_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        address: input.deployed.address.clone(),
        configured_from: ConfiguredFrom {
            deployed_context_ref: ContextRefValue::from(context.context_ref().clone()),
            deployed_address: input.deployed.address.clone(),
        },
        configuration_claim: ConfigurationClaim::MfmConfigured {
            configure_node: evidence.configure_node,
            configure_action_digest: digest_for_config(action)?,
            call_evidence_refs: evidence.call_evidence_refs,
            confirmation_evidence_refs: evidence.confirmation_evidence_refs,
        },
        configure_or_import_evidence: retained_evidence,
        external_adoption_evidence: None,
        configured_block_number: evidence.configured_block_number.or_else(|| {
            evidence
                .receipts
                .iter()
                .map(|receipt| receipt.block_number)
                .max()
        }),
        asserted_configuration_snapshot: None,
    })
}

fn digest_for_config<T>(value: &T) -> StateResult<ContractProfileDigestRef>
where
    T: Serialize,
{
    let json = serde_json::to_string(value)
        .map_err(|error| StateError::Message(format!("config serialization failed: {error}")))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| StateError::Message(format!("config canonicalization failed: {error}")))?;
    Ok(ContractProfileDigestRef::from(canonical.content_digest()))
}

fn ensure_verified_source_run_import<T>(
    source: &ImportFromMfmRun,
    evidence: &ImportFromMfmRunEvidence,
    value: &T,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> StateResult<()>
where
    T: ContextBoundOutput + mfm_values::MfmValue + Serialize,
{
    validate_source_run_import_policy(source, context, required_stage)?;
    if evidence.source_spec_hash != source.source_spec_hash
        || evidence.source_cell_or_output_id != source.source_cell_or_output_id
        || evidence.source_stage != required_stage
        || evidence.source_context_ref != source.source_context_ref
        || evidence.source_value_digest != source.source_value_digest
        || evidence.import_policy_digest != digest_for_config(source)?
    {
        return Err(import_admission_error("source-run import"));
    }
    if value.context_ref() != evidence.source_context_ref.as_context_ref()
        || value.context_resource_kind() != contract_instance_resource_kind()
        || value.context_stage() != context_stage_for_lifecycle_stage(required_stage)
        || digest_for_config(value)? != evidence.source_value_digest
    {
        return Err(import_admission_error("source-run import"));
    }
    if evidence.source_context_ref.as_context_ref() == context.context_ref()
        && evidence
            .source_context_descriptor_id
            .typed()
            .map_err(StateError::Message)?
            != *context.context_descriptor_id()
    {
        return Err(import_admission_error("source-run import"));
    }
    if evidence
        .source_value_artifact_ref_or_inline_canonical_value
        .content_digest()
        .map_err(StateError::Message)?
        != evidence
            .source_value_digest
            .typed()
            .map_err(StateError::Message)?
        || evidence
            .source_cell_schema_id
            .typed()
            .map_err(StateError::Message)?
            != T::schema_id().map_err(|error| StateError::Message(error.to_string()))?
        || evidence
            .source_cell_semantic_type_id
            .typed()
            .map_err(StateError::Message)?
            != T::semantic_id().map_err(|error| StateError::Message(error.to_string()))?
    {
        return Err(import_admission_error("source-run import"));
    }
    Ok(())
}

fn validate_source_run_import_policy(
    source: &ImportFromMfmRun,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> StateResult<()> {
    if source.required_stage != required_stage {
        return Err(import_admission_error("source-run import"));
    }
    match &source.accepted_context_policy {
        mfm_evm_contract_model::AcceptedContextPolicy::ExactContext {} => {
            if source.source_context_ref.as_context_ref() != context.context_ref() {
                return Err(import_admission_error("source-run import"));
            }
        }
        mfm_evm_contract_model::AcceptedContextPolicy::AcceptedContextRefs { context_refs } => {
            if !context_refs
                .iter()
                .any(|context_ref| context_ref == &source.source_context_ref)
            {
                return Err(import_admission_error("source-run import"));
            }
        }
    }
    Ok(())
}

fn source_run_import_evidence_refs(
    evidence: &ImportFromMfmRunEvidence,
) -> Vec<LifecycleArtifactEvidenceRef> {
    vec![
        evidence.source_spec_artifact_ref.clone(),
        evidence.source_spec_certificate_ref.clone(),
        evidence.source_run_stream_ref.clone(),
        evidence
            .source_value_artifact_ref_or_inline_canonical_value
            .clone(),
    ]
}

fn external_source_matches_context(
    source: &ExternalEvmSourceEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> bool {
    source.network_id.as_str() == context.value().network.network_id.as_str()
        && source.expected_chain_id == context.value().network.expected_chain_id()
        && source.observed_chain_id == context.value().network.expected_chain_id()
}

fn ensure_external_adoption_evidence(
    evidence: &ExternalAdoptionEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
    adoption: &AdoptExternalAddress,
) -> StateResult<()> {
    let policy = &adoption.evidence_policy;
    let expected_network_context_ref = digest_for_config(&context.value().network)?.to_string();
    if evidence.context_ref.as_context_ref() != context.context_ref()
        || evidence.evm_network_context_ref != expected_network_context_ref
        || evidence.resource_stage != required_stage
        || evidence.observed_chain_id != context.value().network.expected_chain_id()
        || evidence.evidence_policy_digest != digest_for_config(policy)?
    {
        return Err(import_admission_error("external adoption"));
    }
    if policy.require_code || policy.expected_code_hash.is_some() {
        let Some(code) = evidence.code_read_evidence.as_ref() else {
            return Err(import_admission_error("external adoption"));
        };
        if code.address != adoption.address
            || !external_source_matches_context(&code.source, context)
            || (policy.require_code && code.observed_code_byte_len == 0)
        {
            return Err(import_admission_error("external adoption"));
        }
        if let Some(expected_code_hash) = &policy.expected_code_hash {
            if &code.observed_code_hash != expected_code_hash {
                return Err(import_admission_error("external adoption"));
            }
        }
    }
    if evidence.read_assertion_evidence.len() != policy.initial_read_assertions.len()
        || evidence.event_assertion_evidence.len() != policy.initial_event_assertions.len()
    {
        return Err(import_admission_error("external adoption"));
    }
    for (record, assertion) in evidence
        .read_assertion_evidence
        .iter()
        .zip(policy.initial_read_assertions.iter())
    {
        if !external_source_matches_context(&record.source, context)
            || record.result.function.as_str() != assertion.function.as_str()
            || record.result.args != assertion.args
            || record.result.expected != assertion.expected
        {
            return Err(import_admission_error("external adoption"));
        }
        require_validation_read_result_canonical_passed(&record.result)?;
        if !validation_read_result_passes(&record.result) {
            return Err(import_admission_error("external adoption"));
        }
    }
    for (record, assertion) in evidence
        .event_assertion_evidence
        .iter()
        .zip(policy.initial_event_assertions.iter())
    {
        if assertion.from_block.is_some() || assertion.to_block.is_some() {
            return Err(import_admission_error("external adoption"));
        }
        if !external_source_matches_context(&record.source, context)
            || record.result.event.as_str() != assertion.event.as_str()
            || record.result.min_count != assertion.min_count
        {
            return Err(import_admission_error("external adoption"));
        }
        require_validation_event_result_canonical_passed(&record.result)?;
        if !validation_event_result_passes(&record.result) {
            return Err(import_admission_error("external adoption"));
        }
    }
    Ok(())
}

fn deployed_instance_from_verified_external_adoption(
    adoption: &AdoptExternalAddress,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    evidence: ExternalAdoptionEvidence,
    deployed_block_number: Option<u64>,
) -> StateResult<DeployedContractInstance> {
    ensure_external_adoption_evidence(
        &evidence,
        context,
        ContractLifecycleStage::Deployed,
        adoption,
    )?;
    Ok(DeployedContractInstance {
        lifecycle_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        address: adoption.address.clone(),
        deploy_provenance: DeployProvenance::ExternalAdoption {
            provenance_label: adoption.provenance_label.clone(),
            evidence_policy_digest: evidence.evidence_policy_digest.clone(),
        },
        deploy_evidence: Vec::new(),
        external_adoption_evidence: Some(evidence),
        deployed_block_number,
    })
}

fn context_stage_for_lifecycle_stage(
    stage: ContractLifecycleStage,
) -> &'static mfm_ids::ContextStage {
    match stage {
        ContractLifecycleStage::Deployed => deployed_contract_stage(),
        ContractLifecycleStage::Configured => configured_contract_stage(),
    }
}

fn import_admission_error(state_name: &'static str) -> StateError {
    StateError::Message(format!(
        "{state_name} requires certified import evidence before producing a context-bound resource"
    ))
}

fn idempotency_from_intent<T>(intent: &T) -> StateResult<ContractTransactionIdempotency>
where
    T: Serialize,
{
    let json = serde_json::to_string(intent)
        .map_err(|error| StateError::Message(format!("intent serialization failed: {error}")))?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| StateError::Message(format!("intent canonicalization failed: {error}")))?;
    Ok(ContractTransactionIdempotency {
        key: canonical.digest_bytes().to_string(),
    })
}

#[cfg(test)]
mod tests;
