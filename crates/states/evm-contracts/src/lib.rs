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
//! use mfm_state_evm_contracts::DeployContractState;
//!
//! assert_eq!(DeployContractState::name(), "mfm.evm.contract.deploy");
//! ```

use std::future;

use mfm_adapter_contracts::evm_contract_lifecycle_adapter_binding;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySetFor, NoCaps};
use mfm_effects::{ApplySideEffect, Pure, ReadExternal};
use mfm_evm_capabilities::{
    EvmCallReadCapability, EvmChainIdentityCapability, EvmCodeReadCapability, EvmFeeReadCapability,
    EvmGasEstimateCapability, EvmLogsReadCapability, EvmNonceOccupancyReadCapability,
    EvmNonceReadCapability, EvmReceiptReadCapability, EvmTransactionSubmitCapability,
};
use mfm_evm_contract_config::{
    ConfigureAction, ConfigurePhaseConfig, DeployAction, DeployPhaseConfig, EvmNetworkIntent,
    EvmTransactionPolicy, ImportConfiguredSpec, ImportDeployedSpec, ValidateAction,
    ValidatePhaseConfig,
};
use mfm_evm_contract_model::{
    configured_contract_stage, contract_instance_resource_kind, deployed_contract_stage,
    validation_report_resource_kind, validation_report_stage, ConfigurationClaim,
    ConfiguredContract, ConfiguredContractInstance, ConfiguredContractInstanceRef,
    ConfiguredContractRef, ConfiguredFrom, ContextBoundValidationReport, ContractAddress,
    ContractCallConfig, ContractProfileDigestRef, DeployProvenance, DeployedContract,
    DeployedContractInstance, EventAssertionConfig, EvmContractContext,
    LifecycleArtifactEvidenceRef, LifecycleNodeIdRef, ReadAssertionConfig, ValidationEventResult,
    ValidationReadResult, ValidationReport,
};
use mfm_ids::{DescriptorId, DigestAlgorithm, SchemaId, StateKind, StateVersion};
use mfm_program::{
    AdapterBindingSpec, IdempotencyKey, NoContext, PureState, ReadState, ResourceClaim,
    ResourceNamespace, SideEffectState, StateError, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs, StateInput};
use mfm_signing::SigningCapability;
use mfm_values::ContextRefValue;
use serde::{Deserialize, Serialize};

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

/// Input consumed by contract configuration states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.contract.input.configure")]
pub struct ConfigureContractInput {
    /// Contract emitted by a deploy state.
    pub deployed: DeployedContract,
}

/// Input consumed by contract validation states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.contract.input.validate")]
pub struct ValidateContractInput {
    /// Configured contract emitted by a configure state.
    pub configured: ConfiguredContract,
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

/// Generic deterministic EVM transaction intent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "transaction-intent",
    schema = "mfm.evm.contract.intent.transaction"
)]
pub struct ContractTransactionIntent {
    /// Intent contract version.
    pub intent_version: u64,
    /// Stable semantic network id.
    pub network_id: String,
    /// Expected EVM chain id.
    pub expected_chain_id: u64,
    /// Process-local signer reference.
    pub signer_ref: String,
    /// Expected signer EVM address.
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

/// Deploy mutation intent prepared by [`DeployContractState`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-intent",
    schema = "mfm.evm.contract.intent.deploy"
)]
pub struct ContractDeployIntent {
    /// Intent contract version.
    pub intent_version: u64,
    /// Generic transaction intent.
    pub transaction: ContractTransactionIntent,
    /// Constructor arguments retained for adapter-side ABI encoding.
    pub constructor_args_len: u64,
    /// Whether an inline contract artifact was provided.
    pub has_inline_artifact: bool,
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

/// Configure mutation intent prepared by [`ConfigureContractState`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configure-intent",
    schema = "mfm.evm.contract.intent.configure"
)]
pub struct ContractConfigureIntent {
    /// Intent contract version.
    pub intent_version: u64,
    /// Deployed contract being configured.
    pub deployed: DeployedContract,
    /// Generic transaction intents, one for each configured call.
    pub transactions: Vec<ContractTransactionIntent>,
    /// Whether an inline contract artifact was provided.
    pub has_inline_artifact: bool,
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
    /// Transaction hash.
    pub transaction_hash: String,
    /// Block number that included the transaction.
    pub block_number: u64,
    /// Whether the transaction succeeded.
    pub status: bool,
    /// Typed artifact evidence reference for the retained receipt.
    pub receipt_evidence: Option<LifecycleArtifactEvidenceRef>,
}

/// Deployment receipt-level terminal evidence consumed by [`DeployContractState::output_from_receipt`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-receipt",
    schema = "mfm.evm.contract.value.deploy_receipt"
)]
pub struct ContractDeployReceipt {
    /// Receipt-level contract version.
    pub receipt_version: u64,
    /// Deployed contract address derived from prepared invocation evidence.
    pub contract_address: String,
    /// Observed transaction receipt.
    pub receipt: ContractTransactionReceipt,
}

/// Deployment confirmation consumed by [`DeployContractState::output_from_confirmation`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "deploy-confirmation",
    schema = "mfm.evm.contract.value.deploy_confirmation"
)]
pub struct ContractDeployConfirmation {
    /// Confirmation contract version.
    pub confirmation_version: u64,
    /// Confirmations proven by the confirmation adapter when this artifact was recorded.
    pub confirmations: u64,
    /// Deployed contract address.
    pub contract_address: String,
    /// Confirmed transaction receipt.
    pub receipt: ContractTransactionReceipt,
}

/// Configuration receipt-level terminal evidence consumed by [`ConfigureContractState::output_from_receipt`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configure-receipt",
    schema = "mfm.evm.contract.value.configure_receipt"
)]
pub struct ContractConfigureReceipt {
    /// Receipt-level contract version.
    pub receipt_version: u64,
    /// Observed transaction receipts.
    pub receipts: Vec<ContractTransactionReceipt>,
    /// Highest observed configuration block, when known.
    pub configured_block_number: Option<u64>,
}

/// Configuration confirmation consumed by [`ConfigureContractState::output_from_confirmation`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "configure-confirmation",
    schema = "mfm.evm.contract.value.configure_confirmation"
)]
pub struct ContractConfigureConfirmation {
    /// Confirmation contract version.
    pub confirmation_version: u64,
    /// Confirmations proven by the confirmation adapter when this artifact was recorded.
    pub confirmations: u64,
    /// Confirmed transaction receipts.
    pub receipts: Vec<ContractTransactionReceipt>,
    /// Highest observed configuration block, when known.
    pub configured_block_number: Option<u64>,
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

/// Read request used by contract validation adapters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "validation-read-request",
    schema = "mfm.evm.contract.intent.validation_read_request"
)]
pub struct ContractValidationReadRequest {
    /// Request contract version.
    pub request_version: u64,
    /// Configured contract being validated.
    pub configured_contract: ConfiguredContractRef,
    /// Expected EVM chain id.
    pub expected_chain_id: u64,
    /// Read assertions evaluated by the adapter.
    pub read_assertions: Vec<ReadAssertionConfig>,
    /// Event assertions evaluated by the adapter.
    pub event_assertions: Vec<EventAssertionConfig>,
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
}

/// Public output handles for contract lifecycle workflows.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.public_outputs.lifecycle")]
pub struct ContractLifecyclePublicOutputs<'program, 'scope> {
    /// Deployed contract typestate.
    pub deployed: mfm_program::Handle<'program, 'scope, DeployedContract>,
    /// Configured contract typestate.
    pub configured: mfm_program::Handle<'program, 'scope, ConfiguredContract>,
    /// Validation report.
    pub validation_report: mfm_program::Handle<'program, 'scope, ValidationReport>,
}

/// Operation output handles produced by contract lifecycle operation helpers.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.contract.operation_outputs.lifecycle")]
pub struct ContractLifecycleOperationOutputs<'program, 'scope> {
    /// Deployed contract typestate.
    pub deployed: mfm_program::Handle<'program, 'scope, DeployedContract>,
    /// Configured contract typestate.
    pub configured: mfm_program::Handle<'program, 'scope, ConfiguredContract>,
    /// Validation report.
    pub validation_report: mfm_program::Handle<'program, 'scope, ValidationReport>,
}

/// Context-bound state that prepares and submits contract deployment transactions.
pub struct ContextBoundDeployContractState {
    action: DeployAction,
}

impl ContextBoundDeployContractState {
    /// Returns the deploy action config.
    pub const fn action(&self) -> &DeployAction {
        &self.action
    }
}

impl StateSpec for ContextBoundDeployContractState {
    type Config = DeployAction;
    type Context = EvmContractContext;
    type Input = ();
    type Output = DeployedContractInstance;
    type Effect = ApplySideEffect;
    type Caps = ContractMutationCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("context_deploy")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("context_deploy")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_deploy"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        Ok(mfm_program::StateOutputContextContractSpec::Produces {
            resource_kind: contract_instance_resource_kind().clone(),
            stage: deployed_contract_stage().clone(),
        })
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            action: config.into_inner(),
        })
    }
}

impl SideEffectState for ContextBoundDeployContractState {
    type Intent = ContextContractDeployIntent;
    type IdempotencyInput = ContractTransactionIdempotency;
    type Submission = ContractTransactionSubmissions;
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        _input: &Self::Input,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        Ok(ContextContractDeployIntent {
            intent_version: 1,
            transaction: context_transaction_intent_from_deploy_action(context, &self.action, None),
            constructor_args_len: self.action.constructor_args().len() as u64,
            contract_profile_id: context
                .value()
                .contract_profile
                .profile_id
                .as_str()
                .to_owned(),
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        deployed_instance_from_receipt(receipt.contract_address.as_str(), &receipt.receipt, context)
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        deployed_instance_from_receipt(
            confirmation.contract_address.as_str(),
            &confirmation.receipt,
            context,
        )
    }
}

/// Context-bound state that prepares and submits contract configuration transactions.
pub struct ContextBoundConfigureContractState {
    action: ConfigureAction,
}

impl ContextBoundConfigureContractState {
    /// Returns the configure action config.
    pub const fn action(&self) -> &ConfigureAction {
        &self.action
    }
}

impl StateSpec for ContextBoundConfigureContractState {
    type Config = ConfigureAction;
    type Context = EvmContractContext;
    type Input = ContextConfigureContractInput;
    type Output = ConfiguredContractInstance;
    type Effect = ApplySideEffect;
    type Caps = ContractMutationCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("context_configure")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("context_configure")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_configure"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn input_context_contract() -> mfm_program::Result<mfm_program::StateInputContextContractSpec> {
        Ok(mfm_program::StateInputContextContractSpec::Required {
            resource_kind: contract_instance_resource_kind().clone(),
            stage: deployed_contract_stage().clone(),
            producer: Box::new(mfm_program::ContextProducerSpec {
                producer_descriptor_ids: sorted_descriptor_ids(vec![
                    descriptor_id_for_state::<ContextBoundDeployContractState>()?,
                    descriptor_id_for_state::<ImportDeployedContractState>()?,
                ]),
                seed_producers_allowed: false,
            }),
        })
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        Ok(mfm_program::StateOutputContextContractSpec::Produces {
            resource_kind: contract_instance_resource_kind().clone(),
            stage: configured_contract_stage().clone(),
        })
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            action: config.into_inner(),
        })
    }
}

impl SideEffectState for ContextBoundConfigureContractState {
    type Intent = ContextContractConfigureIntent;
    type IdempotencyInput = ContractTransactionIdempotency;
    type Submission = ContractTransactionSubmissions;
    type Receipt = ContextContractConfigureReceipt;
    type Confirmation = ContextContractConfigureConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        input: &Self::Input,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        let transactions = self
            .action
            .calls()
            .iter()
            .map(|call| {
                context_transaction_intent_from_configure_call(
                    context,
                    &self.action,
                    &input.deployed,
                    call,
                )
            })
            .collect();
        Ok(ContextContractConfigureIntent {
            intent_version: 1,
            deployed: input.deployed.clone(),
            transactions,
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        configured_instance_from_evidence(
            &self.action,
            input,
            ContextConfigureOutputEvidence {
                receipts: &receipt.receipts,
                configured_block_number: receipt.configured_block_number,
                configure_node: receipt.configure_node.clone(),
                call_evidence_refs: receipt.call_evidence_refs.clone(),
                confirmation_evidence_refs: receipt.confirmation_evidence_refs.clone(),
            },
            context,
        )
    }

    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        configured_instance_from_evidence(
            &self.action,
            input,
            ContextConfigureOutputEvidence {
                receipts: &confirmation.receipts,
                configured_block_number: confirmation.configured_block_number,
                configure_node: confirmation.configure_node.clone(),
                call_evidence_refs: confirmation.call_evidence_refs.clone(),
                confirmation_evidence_refs: confirmation.confirmation_evidence_refs.clone(),
            },
            context,
        )
    }
}

/// Context-bound state that validates a configured contract through read-only EVM capabilities.
pub struct ContextBoundValidateContractState {
    action: ValidateAction,
}

impl ContextBoundValidateContractState {
    /// Returns the validate action config.
    pub const fn action(&self) -> &ValidateAction {
        &self.action
    }

    /// Builds the deterministic read request an adapter must execute.
    pub fn read_request(
        &self,
        input: &ContextValidateContractInput,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
    ) -> StateResult<ContextContractValidationReadRequest> {
        Ok(ContextContractValidationReadRequest {
            request_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            configured_instance: ConfiguredContractInstanceRef::from_configured(&input.configured),
            network_id: context.value().network.network_id.as_str().to_owned(),
            expected_chain_id: context.value().network.expected_chain_id(),
            read_assertions: self.action.read_assertions().to_vec(),
            event_assertions: self.action.event_assertions().to_vec(),
        })
    }

    /// Projects a context-bound validation report from an adapter-provided read response.
    pub fn report_from_response(
        &self,
        input: &ContextValidateContractInput,
        response: ContractValidationReadResponse,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
    ) -> StateResult<ContextBoundValidationReport> {
        let valid = response.observed_chain_id == context.value().network.expected_chain_id()
            && response
                .configuration_read_results
                .iter()
                .all(|result| result.passed)
            && response
                .configuration_event_results
                .iter()
                .all(|result| result.passed)
            && response.read_results.iter().all(|result| result.passed)
            && response.event_results.iter().all(|result| result.passed);

        Ok(ContextBoundValidationReport {
            report_version: 1,
            context_ref: ContextRefValue::from(context.context_ref().clone()),
            configured_instance: ConfiguredContractInstanceRef::from_configured(&input.configured),
            configuration_read_results: response.configuration_read_results,
            configuration_event_results: response.configuration_event_results,
            read_results: response.read_results,
            event_results: response.event_results,
            evidence_refs: Vec::new(),
            valid,
        })
    }
}

impl StateSpec for ContextBoundValidateContractState {
    type Config = ValidateAction;
    type Context = EvmContractContext;
    type Input = ContextValidateContractInput;
    type Output = ContextBoundValidationReport;
    type Effect = ReadExternal;
    type Caps = ContractValidationReadCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("context_validate")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("context_validate")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.context_validate"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn input_context_contract() -> mfm_program::Result<mfm_program::StateInputContextContractSpec> {
        Ok(mfm_program::StateInputContextContractSpec::Required {
            resource_kind: contract_instance_resource_kind().clone(),
            stage: configured_contract_stage().clone(),
            producer: Box::new(mfm_program::ContextProducerSpec {
                producer_descriptor_ids: sorted_descriptor_ids(vec![
                    descriptor_id_for_state::<ContextBoundConfigureContractState>()?,
                    descriptor_id_for_state::<ImportConfiguredContractState>()?,
                ]),
                seed_producers_allowed: false,
            }),
        })
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        Ok(mfm_program::StateOutputContextContractSpec::Produces {
            resource_kind: validation_report_resource_kind().clone(),
            stage: validation_report_stage().clone(),
        })
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            action: config.into_inner(),
        })
    }
}

impl ReadState for ContextBoundValidateContractState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Context-bound state that imports a deployed contract instance from certified evidence.
pub struct ImportDeployedContractState {
    import: ImportDeployedSpec,
}

impl ImportDeployedContractState {
    /// Returns the deployed import spec.
    pub const fn import(&self) -> &ImportDeployedSpec {
        &self.import
    }

    /// Fails closed until an adapter supplies verified import evidence.
    pub fn reject_without_verified_evidence(&self) -> StateResult<DeployedContractInstance> {
        Err(import_admission_error(Self::name()))
    }
}

impl StateSpec for ImportDeployedContractState {
    type Config = ImportDeployedSpec;
    type Context = EvmContractContext;
    type Input = ();
    type Output = DeployedContractInstance;
    type Effect = ReadExternal;
    type Caps = ContractImportReadCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("import_deployed")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("import_deployed")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.import_deployed"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        Ok(mfm_program::StateOutputContextContractSpec::Produces {
            resource_kind: contract_instance_resource_kind().clone(),
            stage: deployed_contract_stage().clone(),
        })
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            import: config.into_inner(),
        })
    }
}

impl ReadState for ImportDeployedContractState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(import_admission_error(Self::name())))
    }
}

/// Context-bound state that imports a configured contract instance from certified evidence.
pub struct ImportConfiguredContractState {
    import: ImportConfiguredSpec,
}

impl ImportConfiguredContractState {
    /// Returns the configured import spec.
    pub const fn import(&self) -> &ImportConfiguredSpec {
        &self.import
    }

    /// Fails closed until an adapter supplies verified import evidence.
    pub fn reject_without_verified_evidence(&self) -> StateResult<ConfiguredContractInstance> {
        Err(import_admission_error(Self::name()))
    }
}

impl StateSpec for ImportConfiguredContractState {
    type Config = ImportConfiguredSpec;
    type Context = EvmContractContext;
    type Input = ();
    type Output = ConfiguredContractInstance;
    type Effect = ReadExternal;
    type Caps = ContractImportReadCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("import_configured")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("import_configured")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.import_configured"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn output_context_contract() -> mfm_program::Result<mfm_program::StateOutputContextContractSpec>
    {
        Ok(mfm_program::StateOutputContextContractSpec::Produces {
            resource_kind: contract_instance_resource_kind().clone(),
            stage: configured_contract_stage().clone(),
        })
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            import: config.into_inner(),
        })
    }
}

impl ReadState for ImportConfiguredContractState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(import_admission_error(Self::name())))
    }
}

/// State that prepares and submits contract deployment transactions.
pub struct DeployContractState {
    config: DeployPhaseConfig,
}

impl DeployContractState {
    /// Returns the deploy phase config.
    pub const fn config(&self) -> &DeployPhaseConfig {
        &self.config
    }
}

impl StateSpec for DeployContractState {
    type Config = DeployPhaseConfig;
    type Context = NoContext;
    type Input = ();
    type Output = DeployedContract;
    type Effect = ApplySideEffect;
    type Caps = ContractMutationCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("deploy")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("deploy")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.deploy"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for DeployContractState {
    type Intent = ContractDeployIntent;
    type IdempotencyInput = ContractTransactionIdempotency;
    type Submission = ContractTransactionSubmissions;
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        _input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        Ok(ContractDeployIntent {
            intent_version: 1,
            transaction: transaction_intent_from_deploy_config(&self.config, None),
            constructor_args_len: self.config.constructor_args().len() as u64,
            has_inline_artifact: self.config.artifact().is_some(),
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(DeployedContract {
            lifecycle_version: 1,
            network_id: self.config.network().network_id().to_owned(),
            expected_chain_id: self.config.network().expected_chain_id(),
            contract_address: receipt.contract_address.clone(),
            deploy_tx_hash: receipt.receipt.transaction_hash.clone(),
            deploy_receipt_evidence: receipt.receipt.receipt_evidence.clone(),
            deployed_block_number: Some(receipt.receipt.block_number),
        })
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(DeployedContract {
            lifecycle_version: 1,
            network_id: self.config.network().network_id().to_owned(),
            expected_chain_id: self.config.network().expected_chain_id(),
            contract_address: confirmation.contract_address.clone(),
            deploy_tx_hash: confirmation.receipt.transaction_hash.clone(),
            deploy_receipt_evidence: confirmation.receipt.receipt_evidence.clone(),
            deployed_block_number: Some(confirmation.receipt.block_number),
        })
    }
}

/// State that prepares and submits contract configuration transactions.
pub struct ConfigureContractState {
    config: ConfigurePhaseConfig,
}

impl ConfigureContractState {
    /// Returns the configure phase config.
    pub const fn config(&self) -> &ConfigurePhaseConfig {
        &self.config
    }
}

impl StateSpec for ConfigureContractState {
    type Config = ConfigurePhaseConfig;
    type Context = NoContext;
    type Input = ConfigureContractInput;
    type Output = ConfiguredContract;
    type Effect = ApplySideEffect;
    type Caps = ContractMutationCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("configure")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("configure")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.configure"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl SideEffectState for ConfigureContractState {
    type Intent = ContractConfigureIntent;
    type IdempotencyInput = ContractTransactionIdempotency;
    type Submission = ContractTransactionSubmissions;
    type Receipt = ContractConfigureReceipt;
    type Confirmation = ContractConfigureConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Intent> {
        ensure_network_matches(
            self.config.network(),
            &input.deployed.network_id,
            input.deployed.expected_chain_id,
        )?;
        let transactions = self
            .config
            .calls()
            .iter()
            .map(|call| transaction_intent_from_configure_call(&self.config, &input.deployed, call))
            .collect();
        Ok(ContractConfigureIntent {
            intent_version: 1,
            deployed: input.deployed.clone(),
            transactions,
            has_inline_artifact: self.config.artifact().is_some(),
        })
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        ensure_network_matches(
            self.config.network(),
            &input.deployed.network_id,
            input.deployed.expected_chain_id,
        )?;
        Ok(ConfiguredContract {
            lifecycle_version: 1,
            deployed: input.deployed.clone(),
            configure_calls: self.config.calls().to_vec(),
            confirmation_read_assertions: self.config.confirmation_read_assertions().to_vec(),
            confirmation_event_assertions: self.config.confirmation_event_assertions().to_vec(),
            configure_tx_hashes: receipt
                .receipts
                .iter()
                .map(|receipt| receipt.transaction_hash.clone())
                .collect(),
            configure_receipt_evidence: receipt
                .receipts
                .iter()
                .filter_map(|receipt| receipt.receipt_evidence.clone())
                .collect(),
            configured_block_number: receipt.configured_block_number.or_else(|| {
                receipt
                    .receipts
                    .iter()
                    .map(|receipt| receipt.block_number)
                    .max()
            }),
        })
    }

    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        ensure_network_matches(
            self.config.network(),
            &input.deployed.network_id,
            input.deployed.expected_chain_id,
        )?;
        Ok(ConfiguredContract {
            lifecycle_version: 1,
            deployed: input.deployed.clone(),
            configure_calls: self.config.calls().to_vec(),
            confirmation_read_assertions: self.config.confirmation_read_assertions().to_vec(),
            confirmation_event_assertions: self.config.confirmation_event_assertions().to_vec(),
            configure_tx_hashes: confirmation
                .receipts
                .iter()
                .map(|receipt| receipt.transaction_hash.clone())
                .collect(),
            configure_receipt_evidence: confirmation
                .receipts
                .iter()
                .filter_map(|receipt| receipt.receipt_evidence.clone())
                .collect(),
            configured_block_number: confirmation.configured_block_number.or_else(|| {
                confirmation
                    .receipts
                    .iter()
                    .map(|receipt| receipt.block_number)
                    .max()
            }),
        })
    }
}

/// State that validates a configured contract through read-only EVM capabilities.
pub struct ValidateContractState {
    config: ValidatePhaseConfig,
}

impl ValidateContractState {
    /// Returns the validate phase config.
    pub const fn config(&self) -> &ValidatePhaseConfig {
        &self.config
    }

    /// Builds the deterministic read request an adapter must execute.
    pub fn read_request(
        &self,
        input: &ValidateContractInput,
    ) -> StateResult<ContractValidationReadRequest> {
        ensure_network_matches(
            self.config.network(),
            &input.configured.deployed.network_id,
            input.configured.deployed.expected_chain_id,
        )?;
        Ok(ContractValidationReadRequest {
            request_version: 1,
            configured_contract: ConfiguredContractRef::from_configured(&input.configured),
            expected_chain_id: self.config.network().expected_chain_id(),
            read_assertions: self.config.validation().read_assertions().to_vec(),
            event_assertions: self.config.validation().event_assertions().to_vec(),
        })
    }

    /// Projects a validation report from an adapter-provided read response.
    pub fn report_from_response(
        &self,
        input: &ValidateContractInput,
        response: ContractValidationReadResponse,
    ) -> StateResult<ValidationReport> {
        ensure_network_matches(
            self.config.network(),
            &input.configured.deployed.network_id,
            input.configured.deployed.expected_chain_id,
        )?;
        let valid = response.observed_chain_id == self.config.network().expected_chain_id()
            && response
                .configuration_read_results
                .iter()
                .all(|result| result.passed)
            && response
                .configuration_event_results
                .iter()
                .all(|result| result.passed)
            && response.read_results.iter().all(|result| result.passed)
            && response.event_results.iter().all(|result| result.passed);

        Ok(ValidationReport {
            report_version: 1,
            configured_contract: ConfiguredContractRef::from_configured(&input.configured),
            expected_chain_id: self.config.network().expected_chain_id(),
            observed_chain_id: response.observed_chain_id,
            client_version: response.client_version,
            configuration_read_results: response.configuration_read_results,
            configuration_event_results: response.configuration_event_results,
            read_results: response.read_results,
            event_results: response.event_results,
            valid,
        })
    }
}

impl StateSpec for ValidateContractState {
    type Config = ValidatePhaseConfig;
    type Context = NoContext;
    type Input = ValidateContractInput;
    type Output = ValidationReport;
    type Effect = ReadExternal;
    type Caps = ContractValidationReadCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("validate")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("validate")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.validate"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for ValidateContractState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Pure state that projects a configured contract reference for public outputs.
pub struct ProjectConfiguredContractRefState;

/// Config for projecting a configured contract reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.evm.contract.config.project_configured_ref")]
pub struct ProjectConfiguredContractRefConfig {}

impl StateSpec for ProjectConfiguredContractRefState {
    type Config = ProjectConfiguredContractRefConfig;
    type Context = NoContext;
    type Input = ConfiguredContract;
    type Output = ConfiguredContractRef;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("project_configured_ref")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("project_configured_ref")
    }

    fn name() -> &'static str {
        "mfm.evm.contract.project_configured_ref"
    }

    fn new(_config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for ProjectConfiguredContractRefState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(ConfiguredContractRef::from_configured(&input))
    }
}

fn descriptor_id_for_state<S>() -> mfm_program::Result<DescriptorId>
where
    S: StateSpec,
    S::Effect: mfm_program::EffectRunner<S>,
    S::Caps: CapabilitySetFor<S::Effect>,
{
    mfm_program::registered_state_descriptor::<S>()
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

fn import_admission_error(state_name: &'static str) -> StateError {
    StateError::Message(format!(
        "{state_name} requires certified import evidence before producing a context-bound resource"
    ))
}

fn transaction_intent_from_deploy_config(
    config: &DeployPhaseConfig,
    data_ref: Option<String>,
) -> ContractTransactionIntent {
    ContractTransactionIntent {
        intent_version: 1,
        network_id: config.network().network_id().to_owned(),
        expected_chain_id: config.network().expected_chain_id(),
        signer_ref: config.signer().signer_ref_str().to_owned(),
        expected_signer_address: config.signer().expected_signer_address_str().to_owned(),
        to_address: None,
        value_wei: config.value_wei().map(ToOwned::to_owned),
        data_ref,
        transaction: config.transaction().clone(),
    }
}

fn transaction_intent_from_configure_call(
    config: &ConfigurePhaseConfig,
    deployed: &DeployedContract,
    call: &ContractCallConfig,
) -> ContractTransactionIntent {
    ContractTransactionIntent {
        intent_version: 1,
        network_id: config.network().network_id().to_owned(),
        expected_chain_id: config.network().expected_chain_id(),
        signer_ref: config.signer().signer_ref_str().to_owned(),
        expected_signer_address: config.signer().expected_signer_address_str().to_owned(),
        to_address: Some(deployed.contract_address.clone()),
        value_wei: call.value_wei.as_ref().map(ToString::to_string),
        data_ref: Some(call.function.to_string()),
        transaction: config.transaction().clone(),
    }
}

fn ensure_network_matches(
    network: &EvmNetworkIntent,
    network_id: &str,
    expected_chain_id: u64,
) -> StateResult<()> {
    if network.network_id() != network_id {
        return Err(StateError::Message(
            "contract lifecycle network id mismatch".to_owned(),
        ));
    }
    if network.expected_chain_id() != expected_chain_id {
        return Err(StateError::Message(
            "contract lifecycle expected chain id mismatch".to_owned(),
        ));
    }
    Ok(())
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
