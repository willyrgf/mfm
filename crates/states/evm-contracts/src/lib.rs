#![warn(missing_docs)]
//! Typed EVM contract state contracts.
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

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::CapabilitySetFor;
use mfm_effects::{ApplySideEffect, ReadExternal};
use mfm_evm_capabilities::{
    EvmBlockReadCapability, EvmCallReadCapability, EvmChainIdentityCapability,
    EvmCodeReadCapability, EvmFeeReadCapability, EvmGasEstimateCapability, EvmLogsReadCapability,
    EvmNonceOccupancyReadCapability, EvmNonceReadCapability, EvmReceiptReadCapability,
    EvmTransactionSubmitCapability,
};
pub mod config;
pub use config::{
    ConfigureAction, DeployAction, EvmSignerIntent, EvmTransactionPolicy, EvmTransactionStyle,
    ReceiptRetryPolicy, ValidateAction, MAX_RECEIPT_POLLS, MAX_RECEIPT_POLL_INTERVAL_MS,
    MAX_RECEIPT_TOTAL_WAIT_MS,
};
use mfm_evm_contract_model::{
    configured_contract_stage, contract_instance_resource_kind, deployed_contract_stage,
    expected_matches, validation_report_resource_kind, validation_report_stage,
    ConfiguredContractAnchor, ConfiguredContractInstance, ConfiguredContractInstanceRef,
    ConfiguredFrom, ContextBoundValidationReport, ContractAddress, ContractCallConfig,
    ContractLifecycleStage, ContractProfileDigestRef, DeployedContractInstance,
    EventAssertionConfig, EvmBlockHash, EvmContractContext, LifecycleArtifactEvidenceRef,
    ReadAssertionConfig, ValidationCodeIdentityEvidence, ValidationCodeIdentityRequest,
    ValidationEventEvidence, ValidationEventResult, ValidationReadEvidence, ValidationReadResult,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ContextResourceKind, ContextStage, DescriptorId, DigestAlgorithm,
    SchemaId, StateKind, StateVersion,
};
use mfm_program::{
    AdapterBindingSpec, IdempotencyKey, ReadState, ResourceClaim, ResourceNamespace,
    SideEffectState, StateError, StateResult, StateSpec,
};
use mfm_program_derive::{MfmValue, StateInput};
use mfm_signing::SigningCapability;
use mfm_values::ContextRefValue;
use serde::{Deserialize, Serialize};

#[path = "mutations.rs"]
mod mutations;
pub use self::mutations::{ContextBoundConfigureContractState, ContextBoundDeployContractState};

#[path = "validation.rs"]
mod validation;
pub use self::validation::ContextBoundValidateContractState;

const NAMESPACE: &str = "mfm.evm.contract";
const CONTRACT_STATES_ADAPTER_NAME: &str = "contract_states";
const CONTRACT_STATES_ADAPTER_VERSION: &str = "mfm.evm.contract_states.adapter.v1";
const ACCOUNT_NONCE_RESOURCE_NAMESPACE: &str = "mfm.evm.contract.account_nonce";
const ACCOUNT_NONCE_RESOURCE_KEY_SCHEMA: &str = "mfm.evm.contract.resource_key.account_nonce";

type ContractMutationCaps = (
    EvmBlockReadCapability,
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
    EvmCodeReadCapability,
    EvmCallReadCapability,
    EvmLogsReadCapability,
);

/// Returns the stable adapter kind for the reusable EVM contract states.
pub fn contract_states_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        NAMESPACE,
        CONTRACT_STATES_ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.evm.contract.adapter:contract_states"),
    )
}

/// Returns the stable adapter version for the reusable EVM contract states.
pub fn contract_states_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(CONTRACT_STATES_ADAPTER_VERSION)
}

fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: contract_states_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("EVM contract state adapter kind invalid: {error}"))
        })?,
        adapter_version: contract_states_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!(
                "EVM contract state adapter version invalid: {error}"
            ))
        })?,
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
        "{state_name} requires an EVM contract state adapter runner"
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
    /// Context-bound deployed contract emitted by the deploy state.
    pub deployed: DeployedContractInstance,
}

/// Input consumed by context-bound contract validation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.contract.input.context_validate")]
pub struct ContextValidateContractInput {
    /// Context-bound configured contract emitted by the configure state.
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
    /// Certified contract context ref.
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

/// Idempotency input for contract mutation states.
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
    /// Certified contract context ref.
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
    /// Certified contract context ref.
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
    /// Certified contract context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being mutated.
    pub resource_stage: ContractLifecycleStage,
    /// Transaction hash.
    pub transaction_hash: String,
    /// Block number that included the transaction.
    pub block_number: u64,
    /// Canonical block hash that included the transaction.
    pub block_hash: EvmBlockHash,
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
    /// Certified contract context ref.
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
    /// Certified contract context ref.
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
    /// Certified contract context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being produced.
    pub resource_stage: ContractLifecycleStage,
    /// Observed transaction receipts.
    pub receipts: Vec<ContractTransactionReceipt>,
    /// Exact configured-chain anchor selected from successful receipts or deployment.
    pub anchor: ConfiguredContractAnchor,
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
    /// Certified contract context ref.
    pub context_ref: ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage being produced.
    pub resource_stage: ContractLifecycleStage,
    /// Confirmations proven by the confirmation adapter when this artifact was recorded.
    pub confirmations: u64,
    /// Confirmed transaction receipts.
    pub receipts: Vec<ContractTransactionReceipt>,
    /// Canonical finalized configured-chain anchor.
    pub anchor: ConfiguredContractAnchor,
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
    /// Certified contract context ref.
    pub context_ref: ContextRefValue,
    /// Digest of the configured input value this read request was derived from.
    pub configured_input_digest: ContractProfileDigestRef,
    /// Configured contract instance being validated.
    pub configured_instance: ConfiguredContractInstanceRef,
    /// Stable semantic network id from the certified context.
    pub network_id: String,
    /// Expected EVM chain id from the certified context.
    pub expected_chain_id: u64,
    /// Optional exact deployed-runtime-code identity assertion.
    pub code_identity: Option<ValidationCodeIdentityRequest>,
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
    /// Certified contract context ref.
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
    /// Optional raw deployed-runtime-code identity evidence.
    pub code_identity_evidence: Option<ValidationCodeIdentityEvidence>,
    /// Results for validation read assertions.
    pub read_results: Vec<ValidationReadResult>,
    /// Results for validation event assertions.
    pub event_results: Vec<ValidationEventResult>,
    /// Replayable read assertion evidence.
    pub validation_read_evidence: Vec<ValidationReadEvidence>,
    /// Replayable event assertion evidence.
    pub validation_event_evidence: Vec<ValidationEventEvidence>,
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
    Ok(DeployedContractInstance {
        lifecycle_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        address,
        deployed_block_number: receipt.block_number,
        deployed_block_hash: receipt.block_hash.clone(),
    })
}

fn configured_instance_from_evidence(
    input: &ContextConfigureContractInput,
    anchor: &ConfiguredContractAnchor,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> StateResult<ConfiguredContractInstance> {
    Ok(ConfiguredContractInstance {
        lifecycle_version: 1,
        context_ref: ContextRefValue::from(context.context_ref().clone()),
        address: input.deployed.address.clone(),
        configured_from: ConfiguredFrom {
            deployed_context_ref: ContextRefValue::from(context.context_ref().clone()),
            deployed_address: input.deployed.address.clone(),
        },
        anchor: anchor.clone(),
    })
}

/// Selects the configured-contract anchor from certified configuration receipt order.
///
/// An empty configuration carries forward the direct deployment anchor. Otherwise
/// the last successful configuration receipt is authoritative.
pub fn configured_contract_anchor_from_receipts(
    input: &ContextConfigureContractInput,
    receipts: &[ContractTransactionReceipt],
) -> StateResult<ConfiguredContractAnchor> {
    if receipts.is_empty() {
        return Ok(ConfiguredContractAnchor {
            block_number: input.deployed.deployed_block_number,
            block_hash: input.deployed.deployed_block_hash.clone(),
        });
    }

    let receipt = receipts
        .iter()
        .rev()
        .find(|receipt| receipt.status)
        .ok_or_else(|| {
            StateError::Message(
                "configuration receipt evidence has no successful transaction".to_owned(),
            )
        })?;
    Ok(ConfiguredContractAnchor {
        block_number: receipt.block_number,
        block_hash: receipt.block_hash.clone(),
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
