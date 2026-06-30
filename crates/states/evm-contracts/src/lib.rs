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
use mfm_capabilities::NoCaps;
use mfm_effects::{ApplySideEffect, Pure, ReadExternal};
use mfm_evm_capabilities::{
    EvmCallReadCapability, EvmChainIdentityCapability, EvmFeeReadCapability,
    EvmGasEstimateCapability, EvmLogsReadCapability, EvmNonceOccupancyReadCapability,
    EvmNonceReadCapability, EvmReceiptReadCapability, EvmTransactionSubmitCapability,
};
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, EvmNetworkIntent, EvmTransactionPolicy,
    ValidatePhaseConfig,
};
use mfm_evm_contract_model::{
    ConfiguredContract, ConfiguredContractRef, ContractCallConfig, DeployedContract,
    EventAssertionConfig, LifecycleArtifactEvidenceRef, ReadAssertionConfig, ValidationEventResult,
    ValidationReadResult, ValidationReport,
};
use mfm_ids::{DigestAlgorithm, SchemaId, StateKind, StateVersion};
use mfm_program::{
    AdapterBindingSpec, IdempotencyKey, PureState, ReadState, ResourceClaim, ResourceNamespace,
    SideEffectState, StateError, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs, StateInput};
use mfm_signing::SigningCapability;
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

    fn prepare_intent(&self, _input: &Self::Input) -> StateResult<Self::Intent> {
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
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
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

    fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent> {
        ensure_network_matches_deployed(self.config.network(), &input.deployed)?;
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
    ) -> StateResult<Self::IdempotencyInput> {
        idempotency_from_intent(intent)
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }

    fn output_from_receipt(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        receipt: &Self::Receipt,
    ) -> StateResult<Self::Output> {
        ensure_network_matches_deployed(self.config.network(), &input.deployed)?;
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
    ) -> StateResult<Self::Output> {
        ensure_network_matches_deployed(self.config.network(), &input.deployed)?;
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
        ensure_network_matches_configured(self.config.network(), &input.configured)?;
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
        ensure_network_matches_configured(self.config.network(), &input.configured)?;
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

    fn run<'a>(&'a self, _input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
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
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(ConfiguredContractRef::from_configured(&input))
    }
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

fn ensure_network_matches_deployed(
    network: &EvmNetworkIntent,
    deployed: &DeployedContract,
) -> StateResult<()> {
    if network.network_id() != deployed.network_id {
        return Err(StateError::Message(
            "contract lifecycle network id mismatch".to_owned(),
        ));
    }
    if network.expected_chain_id() != deployed.expected_chain_id {
        return Err(StateError::Message(
            "contract lifecycle expected chain id mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn ensure_network_matches_configured(
    network: &EvmNetworkIntent,
    configured: &ConfiguredContract,
) -> StateResult<()> {
    ensure_network_matches_deployed(network, &configured.deployed)
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
