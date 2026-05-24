#![warn(missing_docs)]
//! Typed EVM deploy/configure/validate state contracts.
//!
//! This crate owns the reusable typed state, value, and capability contracts for the EVM
//! deploy/configure/validate lifecycle. The workflow is expressed as typestate values:
//! deployment produces [`DeployedContract`], configuration consumes it and produces
//! [`ConfiguredContract`], and validation consumes only [`ConfiguredContract`].
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_evm_dcv::{ConfigureContractState, DeployContractState, ValidateContractState};
//! use mfm_program::StateSpec;
//!
//! assert_eq!(DeployContractState::name(), "mfm.evm.dcv.deploy_contract");
//! assert_eq!(ConfigureContractState::name(), "mfm.evm.dcv.configure_contract");
//! assert_eq!(ValidateContractState::name(), "mfm.evm.dcv.validate_contract");
//! ```

use std::future::{self, Future};
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ExternalMutationAuthorityRole, ReadExternalRole, SupportRole,
};
use mfm_effects::{ApplySideEffect, ReadExternal};
pub use mfm_evm_dcv_model::{
    bytes_to_hex_prefixed, decode_single_output_to_json, expected_matches, normalize_address,
    parse_artifact, prepare_validate_assertions, AbiArgumentValue, ConfiguredContract,
    ConfiguredContractRef, ContractArtifactConfig, DeployedContract, ExpectedValue,
    ValidationEventResult, ValidationReadResult, ValidationReport,
};
use mfm_evm_dcv_model::{constructor_data, parse_value_wei_to_hex, resolve_function_call};
use mfm_evm_deploy_configure_validate_config::{
    DeployConfigureValidateConfigureConfig, DeployConfigureValidateDeployConfig,
    DeployConfigureValidateValidateConfig,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, CapabilityKind, CapabilityVersion, DigestAlgorithm, StateKind,
    StateVersion,
};
use mfm_program::{
    AdapterBindingSpec, IdempotencyKey, ReadState, SideEffectState, StateError, StateResult,
    StateSpec,
};
use mfm_program_derive::{MfmValue, OperationOutput, PublicOutputs};
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "mfm.evm.dcv";
const ADAPTER_NAME: &str = "typed-evm-dcv";
const ADAPTER_VERSION: &str = "mfm.evm.dcv.adapter.typed.v1";

/// Returns the typed EVM deploy/configure/validate adapter kind.
pub fn evm_dcv_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        NAMESPACE,
        ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.evm.dcv.adapter:typed-evm-dcv"),
    )
}

/// Returns the typed EVM deploy/configure/validate adapter version.
pub fn evm_dcv_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(ADAPTER_VERSION)
}

fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: evm_dcv_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("evm dcv adapter kind invalid: {error}"))
        })?,
        adapter_version: evm_dcv_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!("evm dcv adapter version invalid: {error}"))
        })?,
    }])
}

fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.evm.dcv.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.evm.dcv.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn capability_kind(name: &'static str) -> mfm_capabilities::Result<CapabilityKind> {
    CapabilityKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.evm.dcv.capability:{name}").as_bytes()),
    )
    .map_err(|error| CapabilityError::Identity(error.to_string()))
}

/// EVM JSON-RPC read capability.
pub struct EvmDcvReadCapability;

impl CapabilitySpec for EvmDcvReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        capability_kind("read")
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.evm.dcv.capability.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.read"
    }
}

/// Local signer or keystore support capability.
pub struct EvmDcvSignerCapability;

impl CapabilitySpec for EvmDcvSignerCapability {
    type Role = SupportRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        capability_kind("signer")
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.evm.dcv.capability.signer.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.signer"
    }
}

/// EVM transaction submission authority.
pub struct EvmDcvTransactionSubmitCapability;

impl CapabilitySpec for EvmDcvTransactionSubmitCapability {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        capability_kind("transaction_submit")
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.evm.dcv.capability.transaction_submit.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.transaction_submit"
    }
}

/// Redaction-safe EVM read error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmDcvReadError {
    /// Stable error code.
    pub code: String,
    /// Safe error message.
    pub message: String,
}

impl EvmDcvReadError {
    /// Creates a redaction-safe EVM read error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EvmDcvReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EvmDcvReadError {}

/// Future returned by typed EVM read backends.
pub type EvmDcvReadFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, EvmDcvReadError>> + Send + 'a>>;

/// Runtime backend used by validation states to read EVM data.
pub trait EvmDcvReadBackend: Send + Sync {
    /// Reads the current chain id.
    fn chain_id<'a>(&'a self, network_id: &'a str) -> EvmDcvReadFuture<'a, u64>;

    /// Reads the redaction-safe client version.
    fn client_version<'a>(&'a self, network_id: &'a str) -> EvmDcvReadFuture<'a, String>;

    /// Executes an `eth_call` at `latest`.
    fn eth_call<'a>(
        &'a self,
        network_id: &'a str,
        to: &'a str,
        data_hex: &'a str,
    ) -> EvmDcvReadFuture<'a, String>;

    /// Counts matching logs for one prepared event assertion.
    fn log_count<'a>(
        &'a self,
        network_id: &'a str,
        address: &'a str,
        topic0_hex: &'a str,
        from_block: &'a serde_json::Value,
        to_block: &'a serde_json::Value,
    ) -> EvmDcvReadFuture<'a, u64>;
}

struct UnavailableEvmDcvReadBackend;

impl EvmDcvReadBackend for UnavailableEvmDcvReadBackend {
    fn chain_id<'a>(&'a self, network_id: &'a str) -> EvmDcvReadFuture<'a, u64> {
        Box::pin(async move {
            Err(EvmDcvReadError::new(
                "external_read_required",
                format!("typed EVM DCV read backend unavailable for network `{network_id}`"),
            ))
        })
    }

    fn client_version<'a>(&'a self, network_id: &'a str) -> EvmDcvReadFuture<'a, String> {
        Box::pin(async move {
            Err(EvmDcvReadError::new(
                "external_read_required",
                format!("typed EVM DCV read backend unavailable for network `{network_id}`"),
            ))
        })
    }

    fn eth_call<'a>(
        &'a self,
        network_id: &'a str,
        _to: &'a str,
        _data_hex: &'a str,
    ) -> EvmDcvReadFuture<'a, String> {
        Box::pin(async move {
            Err(EvmDcvReadError::new(
                "external_read_required",
                format!("typed EVM DCV read backend unavailable for network `{network_id}`"),
            ))
        })
    }

    fn log_count<'a>(
        &'a self,
        network_id: &'a str,
        _address: &'a str,
        _topic0_hex: &'a str,
        _from_block: &'a serde_json::Value,
        _to_block: &'a serde_json::Value,
    ) -> EvmDcvReadFuture<'a, u64> {
        Box::pin(async move {
            Err(EvmDcvReadError::new(
                "external_read_required",
                format!("typed EVM DCV read backend unavailable for network `{network_id}`"),
            ))
        })
    }
}

/// One unsigned EVM transaction intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "transaction-intent",
    version = "1",
    schema = "mfm.evm.dcv.value.transaction_intent"
)]
pub struct EvmDcvTransactionIntent {
    /// Workflow phase that owns the transaction.
    pub phase: String,
    /// Transaction index within the phase.
    pub transaction_index: u64,
    /// Stable network id.
    pub network_id: String,
    /// Stable control scope.
    pub control_scope: String,
    /// Normalized sender address.
    pub from: String,
    /// Normalized recipient address, or absent for contract creation.
    pub to: Option<String>,
    /// Value in wei represented as a hex JSON-RPC quantity.
    pub value_hex: String,
    /// Calldata or initcode as lowercase `0x`-prefixed hex.
    pub data_hex: String,
}

/// Deployment mutation intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "deploy-intent",
    version = "1",
    schema = "mfm.evm.dcv.value.deploy_intent"
)]
pub struct EvmDcvDeployIntent {
    /// Contract creation transaction.
    pub transaction: EvmDcvTransactionIntent,
}

/// Stable idempotency input for deployment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "deploy-idempotency-input",
    version = "1",
    schema = "mfm.evm.dcv.value.deploy_idempotency_input"
)]
pub struct EvmDcvDeployIdempotencyInput {
    /// Contract creation transaction.
    pub transaction: EvmDcvTransactionIntent,
}

/// Deployment submission evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "deploy-submission",
    version = "1",
    schema = "mfm.evm.dcv.value.deploy_submission"
)]
pub struct EvmDcvDeploySubmission {
    /// Transaction hash accepted by the RPC endpoint.
    pub transaction_hash: String,
    /// Idempotency digest used for submission.
    pub idempotency_digest: String,
    /// Managed protected raw transaction artifact id.
    pub protected_raw_transaction_artifact_id: String,
}

/// Deployment receipt evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "deploy-receipt",
    version = "1",
    schema = "mfm.evm.dcv.value.deploy_receipt"
)]
pub struct EvmDcvDeployReceipt {
    /// Transaction hash.
    pub transaction_hash: String,
    /// Normalized deployed contract address.
    pub contract_address: String,
    /// Block number that included the deployment.
    pub block_number: u64,
    /// Whether the receipt status succeeded.
    pub status: bool,
}

/// Deployment confirmation evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "deploy-confirmation",
    version = "1",
    schema = "mfm.evm.dcv.value.deploy_confirmation"
)]
pub struct EvmDcvDeployConfirmation {
    /// Transaction hash.
    pub transaction_hash: String,
    /// Normalized deployed contract address.
    pub contract_address: String,
    /// Receipt artifact id.
    pub receipt_artifact_id: String,
    /// Block number that included the deployment.
    pub block_number: u64,
    /// Number of confirmations checked by the runner.
    pub confirmations: u64,
}

/// Configuration mutation intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-intent",
    version = "1",
    schema = "mfm.evm.dcv.value.configure_intent"
)]
pub struct EvmDcvConfigureIntent {
    /// Deployment being configured.
    pub deployed: DeployedContract,
    /// Sequential configuration transactions.
    pub transactions: Vec<EvmDcvTransactionIntent>,
}

/// Stable idempotency input for configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-idempotency-input",
    version = "1",
    schema = "mfm.evm.dcv.value.configure_idempotency_input"
)]
pub struct EvmDcvConfigureIdempotencyInput {
    /// Deployment being configured.
    pub deployed: DeployedContract,
    /// Sequential configuration transactions.
    pub transactions: Vec<EvmDcvTransactionIntent>,
}

/// Configuration submission evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-submission",
    version = "1",
    schema = "mfm.evm.dcv.value.configure_submission"
)]
pub struct EvmDcvConfigureSubmission {
    /// Transaction hashes accepted by the RPC endpoint.
    pub transaction_hashes: Vec<String>,
    /// Idempotency digest used for submission.
    pub idempotency_digest: String,
    /// Managed protected raw transaction artifact ids.
    pub protected_raw_transaction_artifact_ids: Vec<String>,
}

/// Durable evidence that transaction submission may have reached the remote EVM node but could not
/// be confirmed by an immediate receipt lookup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "submission-unknown-evidence",
    version = "1",
    schema = "mfm.evm.dcv.value.submission_unknown_evidence"
)]
pub struct EvmDcvSubmissionUnknownEvidence {
    /// Stable network identifier targeted by the submission.
    pub network_id: String,
    /// Transaction hashes whose submission status is ambiguous.
    pub transaction_hashes: Vec<String>,
    /// Transaction hash that first produced the ambiguous submission result.
    pub unknown_transaction_hash: String,
    /// Redacted reason code recorded by the runner.
    pub reason_code: String,
}

/// One configuration receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-receipt-entry",
    version = "1",
    schema = "mfm.evm.dcv.value.configure_receipt_entry"
)]
pub struct EvmDcvConfigureReceiptEntry {
    /// Transaction hash.
    pub transaction_hash: String,
    /// Block number that included the transaction.
    pub block_number: u64,
    /// Whether the receipt status succeeded.
    pub status: bool,
}

/// Configuration receipt evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-receipt",
    version = "1",
    schema = "mfm.evm.dcv.value.configure_receipt"
)]
pub struct EvmDcvConfigureReceipt {
    /// Per-transaction receipt evidence.
    pub receipts: Vec<EvmDcvConfigureReceiptEntry>,
}

/// Configuration confirmation evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.dcv",
    name = "configure-confirmation",
    version = "1",
    schema = "mfm.evm.dcv.value.configure_confirmation"
)]
pub struct EvmDcvConfigureConfirmation {
    /// Transaction hashes.
    pub transaction_hashes: Vec<String>,
    /// Receipt artifact ids.
    pub receipt_artifact_ids: Vec<String>,
    /// Highest block number that included a configuration transaction.
    pub configured_block_number: Option<u64>,
    /// Number of confirmations checked by the runner.
    pub confirmations: u64,
}

/// Contract for protected raw transaction artifacts.
///
/// This type intentionally does not implement `MfmValue`, `PublicOutputs`, or any public-output
/// wrapper trait. Runners may store it as a managed prepared-invocation artifact, but certified
/// state programs cannot expose it as a typed semantic value.
#[derive(Clone, PartialEq, Eq)]
pub struct ProtectedRawTransaction {
    raw_transaction_hex: String,
}

impl std::fmt::Debug for ProtectedRawTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtectedRawTransaction")
            .field("raw_transaction_hex", &"<redacted>")
            .finish()
    }
}

impl ProtectedRawTransaction {
    /// Builds a protected raw transaction wrapper.
    pub fn new(raw_transaction_hex: impl Into<String>) -> Self {
        Self {
            raw_transaction_hex: raw_transaction_hex.into(),
        }
    }

    /// Returns the raw transaction hex for mutation-capable transports.
    pub fn raw_transaction_hex(&self) -> &str {
        &self.raw_transaction_hex
    }
}

/// Public output contract for typed EVM DCV workflows.
#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.dcv.public_outputs")]
pub struct DcvPublicOutputs<'program, 'scope> {
    /// Terminal validation report.
    pub validation_report: mfm_program::Handle<'program, 'scope, ValidationReport>,
}

/// Operation output handles produced by the typed EVM DCV workflow.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.evm.dcv.operation_outputs")]
pub struct DcvOperationOutputs<'program, 'scope> {
    /// Deployment lifecycle output.
    pub deployed: mfm_program::Handle<'program, 'scope, DeployedContract>,
    /// Configuration lifecycle output.
    pub configured: mfm_program::Handle<'program, 'scope, ConfiguredContract>,
    /// Terminal validation report.
    pub validation_report: mfm_program::Handle<'program, 'scope, ValidationReport>,
}

/// State that deploys a contract and emits a typed lifecycle value.
pub struct DeployContractState {
    config: DeployConfigureValidateDeployConfig,
}

impl StateSpec for DeployContractState {
    type Config = DeployConfigureValidateDeployConfig;
    type Input = ();
    type Output = DeployedContract;
    type Effect = ApplySideEffect;
    type Caps = (
        EvmDcvReadCapability,
        EvmDcvSignerCapability,
        EvmDcvTransactionSubmitCapability,
    );

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("deploy_contract")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("deploy_contract")
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.deploy_contract"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        validate_deploy_config(&config)?;
        Ok(Self { config })
    }
}

impl SideEffectState for DeployContractState {
    type Intent = EvmDcvDeployIntent;
    type IdempotencyInput = EvmDcvDeployIdempotencyInput;
    type Submission = EvmDcvDeploySubmission;
    type Receipt = EvmDcvDeployReceipt;
    type Confirmation = EvmDcvDeployConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(&self, _input: &Self::Input) -> StateResult<Self::Intent> {
        deploy_intent_from_config(&self.config).map_err(state_error)
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
    ) -> StateResult<Self::IdempotencyInput> {
        Ok(EvmDcvDeployIdempotencyInput {
            transaction: intent.transaction.clone(),
        })
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(StateError::Message(
            "typed EVM deployment submission is provided by the certified transport runner"
                .to_owned(),
        )))
    }

    fn output_from_confirmation(
        &self,
        _input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
    ) -> StateResult<Self::Output> {
        Ok(DeployedContract {
            lifecycle_version: 1,
            network_id: self.config.network_id.clone(),
            control_scope: self.config.control_scope.clone(),
            contract_address: confirmation.contract_address.clone(),
            deploy_tx_hash: confirmation.transaction_hash.clone(),
            deploy_receipt_artifact_id: Some(confirmation.receipt_artifact_id.clone()),
            deployed_block_number: Some(confirmation.block_number),
        })
    }
}

/// State that configures a deployed contract and emits a configured lifecycle value.
pub struct ConfigureContractState {
    config: DeployConfigureValidateConfigureConfig,
}

impl StateSpec for ConfigureContractState {
    type Config = DeployConfigureValidateConfigureConfig;
    type Input = DeployedContract;
    type Output = ConfiguredContract;
    type Effect = ApplySideEffect;
    type Caps = (
        EvmDcvReadCapability,
        EvmDcvSignerCapability,
        EvmDcvTransactionSubmitCapability,
    );

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("configure_contract")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("configure_contract")
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.configure_contract"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        validate_configure_config(&config)?;
        Ok(Self { config })
    }
}

impl SideEffectState for ConfigureContractState {
    type Intent = EvmDcvConfigureIntent;
    type IdempotencyInput = EvmDcvConfigureIdempotencyInput;
    type Submission = EvmDcvConfigureSubmission;
    type Receipt = EvmDcvConfigureReceipt;
    type Confirmation = EvmDcvConfigureConfirmation;
    type SubmitFuture<'a> = future::Ready<StateResult<Self::Submission>>;

    fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent> {
        configure_intent_from_config(&self.config, input).map_err(state_error)
    }

    fn idempotency_input(
        &self,
        _input: &Self::Input,
        intent: &Self::Intent,
    ) -> StateResult<Self::IdempotencyInput> {
        Ok(EvmDcvConfigureIdempotencyInput {
            deployed: intent.deployed.clone(),
            transactions: intent.transactions.clone(),
        })
    }

    fn submit<'a>(
        &'a self,
        _intent: &'a Self::Intent,
        _key: &'a IdempotencyKey<Self::IdempotencyInput>,
        _caps: &'a Self::Caps,
    ) -> Self::SubmitFuture<'a> {
        future::ready(Err(StateError::Message(
            "typed EVM configuration submission is provided by the certified transport runner"
                .to_owned(),
        )))
    }

    fn output_from_confirmation(
        &self,
        input: &Self::Input,
        _intent: &Self::Intent,
        confirmation: &Self::Confirmation,
    ) -> StateResult<Self::Output> {
        ensure_deployed_scope_matches_config(
            &self.config.network_id,
            &self.config.control_scope,
            input,
            "configure",
        )
        .map_err(state_error)?;
        Ok(ConfiguredContract {
            lifecycle_version: 1,
            deployed: input.clone(),
            configure_tx_hashes: confirmation.transaction_hashes.clone(),
            configure_receipt_artifact_ids: confirmation.receipt_artifact_ids.clone(),
            configured_block_number: confirmation.configured_block_number,
        })
    }
}

/// State that validates a configured contract through read-only EVM capabilities.
pub struct ValidateContractState {
    config: DeployConfigureValidateValidateConfig,
}

impl StateSpec for ValidateContractState {
    type Config = DeployConfigureValidateValidateConfig;
    type Input = ConfiguredContract;
    type Output = ValidationReport;
    type Effect = ReadExternal;
    type Caps = (EvmDcvReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("validate_contract")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("validate_contract")
    }

    fn name() -> &'static str {
        "mfm.evm.dcv.validate_contract"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: Self::Config) -> mfm_program::Result<Self> {
        validate_validate_config(&config)?;
        Ok(Self { config })
    }
}

impl ReadState for ValidateContractState {
    type RunFuture<'a> = Pin<Box<dyn Future<Output = StateResult<Self::Output>> + Send + 'a>>;

    fn run<'a>(&'a self, input: Self::Input, _caps: &'a Self::Caps) -> Self::RunFuture<'a> {
        Box::pin(async move {
            validate_configured_contract_with_backend(
                &self.config,
                &input,
                &UnavailableEvmDcvReadBackend,
            )
            .await
            .map_err(|error| StateError::Message(error.to_string()))
        })
    }
}

/// Builds a deploy intent from a typed deploy config.
pub fn deploy_intent_from_config(
    config: &DeployConfigureValidateDeployConfig,
) -> Result<EvmDcvDeployIntent, String> {
    let artifact = required_artifact(config.artifact.as_ref(), "deploy")?;
    let (abi, bytecode) = parse_artifact(artifact)?;
    let data = constructor_data(&abi, &bytecode, &config.constructor_args)?;
    Ok(EvmDcvDeployIntent {
        transaction: EvmDcvTransactionIntent {
            phase: "deploy".to_owned(),
            transaction_index: 0,
            network_id: config.network_id.clone(),
            control_scope: config.control_scope.clone(),
            from: normalize_address(&config.from)?,
            to: None,
            value_hex: parse_value_wei_to_hex(&config.value_wei)?
                .unwrap_or_else(|| "0x0".to_owned()),
            data_hex: bytes_to_hex_prefixed(&data),
        },
    })
}

/// Builds a configure intent from a typed configure config and deployed-contract value.
pub fn configure_intent_from_config(
    config: &DeployConfigureValidateConfigureConfig,
    deployed: &DeployedContract,
) -> Result<EvmDcvConfigureIntent, String> {
    ensure_deployed_scope_matches_config(
        &config.network_id,
        &config.control_scope,
        deployed,
        "configure",
    )?;
    let artifact = required_artifact(config.artifact.as_ref(), "configure")?;
    let (abi, _) = parse_artifact(artifact)?;
    let mut transactions = Vec::with_capacity(config.calls.len());
    for (idx, call) in config.calls.iter().enumerate() {
        let (data, _) = resolve_function_call(&abi, &call.function, &call.args)?;
        transactions.push(EvmDcvTransactionIntent {
            phase: "configure".to_owned(),
            transaction_index: idx as u64,
            network_id: config.network_id.clone(),
            control_scope: config.control_scope.clone(),
            from: normalize_address(&config.from)?,
            to: Some(normalize_address(&deployed.contract_address)?),
            value_hex: parse_value_wei_to_hex(&call.value_wei)?.unwrap_or_else(|| "0x0".to_owned()),
            data_hex: bytes_to_hex_prefixed(&data),
        });
    }
    Ok(EvmDcvConfigureIntent {
        deployed: deployed.clone(),
        transactions,
    })
}

/// Validates a configured contract through an explicit typed read backend.
pub async fn validate_configured_contract_with_backend(
    config: &DeployConfigureValidateValidateConfig,
    configured: &ConfiguredContract,
    backend: &dyn EvmDcvReadBackend,
) -> Result<ValidationReport, EvmDcvReadError> {
    ensure_deployed_scope_matches_config(
        &config.network_id,
        &config.control_scope,
        &configured.deployed,
        "validate",
    )
    .map_err(|message| EvmDcvReadError::new("invalid_configured_contract", message))?;
    let artifact = required_artifact(config.artifact.as_ref(), "validate")
        .map_err(|message| EvmDcvReadError::new("invalid_validate_config", message))?;
    let (abi, _) = parse_artifact(artifact)
        .map_err(|message| EvmDcvReadError::new("invalid_validate_config", message))?;
    let (reads, events) =
        prepare_validate_assertions(&abi, &config.read_assertions, &config.event_assertions)
            .map_err(|message| EvmDcvReadError::new("invalid_validate_config", message))?;

    let observed_chain_id = backend.chain_id(&config.network_id).await?;
    let client_version = backend.client_version(&config.network_id).await?;
    let mut valid = observed_chain_id == config.expected_chain_id
        && client_version.contains(&config.require_client_substring);

    let contract_address = normalize_address(&configured.deployed.contract_address)
        .map_err(|message| EvmDcvReadError::new("invalid_configured_contract", message))?;

    let mut read_results = Vec::with_capacity(reads.len());
    for (assertion, cfg) in reads.iter().zip(config.read_assertions.iter()) {
        let raw = backend
            .eth_call(&config.network_id, &contract_address, &assertion.data_hex)
            .await?;
        let actual_json = decode_single_output_to_json(&assertion.outputs, &raw)
            .map_err(|message| EvmDcvReadError::new("invalid_read_response", message))?;
        let actual = ExpectedValue::from_json_value(&actual_json)
            .map_err(|message| EvmDcvReadError::new("invalid_read_response", message))?;
        let passed = expected_matches(&actual, &assertion.expected);
        valid &= passed;
        read_results.push(ValidationReadResult {
            function: cfg.function.clone(),
            args: cfg.args.clone(),
            expected: assertion.expected.clone(),
            actual,
            passed,
        });
    }

    let mut event_results = Vec::with_capacity(events.len());
    for assertion in events {
        let observed_count = backend
            .log_count(
                &config.network_id,
                &contract_address,
                &assertion.topic0_hex,
                &assertion.from_block,
                &assertion.to_block,
            )
            .await?;
        let passed = observed_count >= assertion.min_count;
        valid &= passed;
        event_results.push(ValidationEventResult {
            event: assertion.event.clone(),
            min_count: assertion.min_count,
            observed_count,
            passed,
        });
    }

    Ok(ValidationReport {
        report_version: 1,
        configured_contract: ConfiguredContractRef::from_configured(configured),
        expected_chain_id: config.expected_chain_id,
        observed_chain_id,
        client_version,
        read_results,
        event_results,
        valid,
    })
}

fn ensure_deployed_scope_matches_config(
    config_network_id: &str,
    config_control_scope: &str,
    deployed: &DeployedContract,
    phase: &str,
) -> Result<(), String> {
    if deployed.network_id != config_network_id {
        return Err(format!(
            "{phase} config network_id {} does not match deployed contract network_id {}",
            config_network_id, deployed.network_id
        ));
    }
    if deployed.control_scope != config_control_scope {
        return Err(format!(
            "{phase} config control_scope {} does not match deployed contract control_scope {}",
            config_control_scope, deployed.control_scope
        ));
    }
    Ok(())
}

fn validate_deploy_config(config: &DeployConfigureValidateDeployConfig) -> mfm_program::Result<()> {
    required_artifact(config.artifact.as_ref(), "deploy")
        .and_then(|artifact| parse_artifact(artifact).map(|_| ()))
        .map_err(mfm_plan_error)?;
    normalize_address(&config.from).map_err(mfm_plan_error)?;
    mfm_evm_dcv_model::ensure_nonzero_polls(config.max_receipt_polls).map_err(mfm_plan_error)?;
    Ok(())
}

fn validate_configure_config(
    config: &DeployConfigureValidateConfigureConfig,
) -> mfm_program::Result<()> {
    required_artifact(config.artifact.as_ref(), "configure")
        .and_then(|artifact| parse_artifact(artifact).map(|_| ()))
        .map_err(mfm_plan_error)?;
    normalize_address(&config.from).map_err(mfm_plan_error)?;
    mfm_evm_dcv_model::ensure_nonzero_polls(config.max_receipt_polls).map_err(mfm_plan_error)?;
    Ok(())
}

fn validate_validate_config(
    config: &DeployConfigureValidateValidateConfig,
) -> mfm_program::Result<()> {
    required_artifact(config.artifact.as_ref(), "validate")
        .and_then(|artifact| parse_artifact(artifact).map(|_| ()))
        .map_err(mfm_plan_error)?;
    if config.require_client_substring.trim().is_empty() {
        return Err(mfm_plan_error(
            "require_client_substring must be non-empty".to_owned(),
        ));
    }
    Ok(())
}

fn required_artifact<'a>(
    artifact: Option<&'a ContractArtifactConfig>,
    phase: &'static str,
) -> Result<&'a ContractArtifactConfig, String> {
    artifact.ok_or_else(|| {
        format!("{phase} config must include an inline typed contract artifact; dynamic artifact ports are not part of certified typed EVM DCV execution")
    })
}

fn mfm_plan_error(message: String) -> mfm_program::PlanError {
    mfm_program::PlanError::Key(message)
}

fn state_error(message: String) -> StateError {
    StateError::Message(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_values::MfmValue;

    #[test]
    fn lifecycle_state_contracts_are_typed() {
        let deploy = DeployContractState::new(sample_deploy_config()).expect("deploy state");
        let intent = deploy.prepare_intent(&()).expect("deploy intent");
        assert!(intent.transaction.to.is_none());
        assert_eq!(intent.transaction.phase, "deploy");

        let deployed = DeployedContract {
            lifecycle_version: 1,
            network_id: "local".to_owned(),
            control_scope: "shared".to_owned(),
            contract_address: "0x0000000000000000000000000000000000000001".to_owned(),
            deploy_tx_hash: "0x1".to_owned(),
            deploy_receipt_artifact_id: Some("artifact".to_owned()),
            deployed_block_number: Some(1),
        };
        let configure =
            ConfigureContractState::new(sample_configure_config()).expect("configure state");
        let configure_intent = configure
            .prepare_intent(&deployed)
            .expect("configure intent");
        assert_eq!(configure_intent.transactions.len(), 1);
        assert_eq!(
            configure_intent.transactions[0].to.as_deref(),
            Some("0x0000000000000000000000000000000000000001")
        );

        assert!(ValidationReport::schema_id().is_ok());
    }

    #[test]
    fn configure_rejects_deployed_contract_scope_mismatch() {
        let mut deployed = sample_deployed_contract();
        deployed.network_id = "other".to_owned();
        let configure =
            ConfigureContractState::new(sample_configure_config()).expect("configure state");

        let error = configure
            .prepare_intent(&deployed)
            .expect_err("network mismatch rejected");
        assert!(error.to_string().contains("network_id"));
    }

    #[tokio::test]
    async fn validate_rejects_configured_contract_scope_mismatch_before_reads() {
        let mut deployed = sample_deployed_contract();
        deployed.control_scope = "other".to_owned();
        let configured = ConfiguredContract {
            lifecycle_version: 1,
            deployed,
            configure_tx_hashes: Vec::new(),
            configure_receipt_artifact_ids: Vec::new(),
            configured_block_number: None,
        };

        let error = validate_configured_contract_with_backend(
            &sample_validate_config(),
            &configured,
            &UnavailableEvmDcvReadBackend,
        )
        .await
        .expect_err("control scope mismatch rejected");
        assert_eq!(error.code, "invalid_configured_contract");
        assert!(error.to_string().contains("control_scope"));
    }

    fn sample_deployed_contract() -> DeployedContract {
        DeployedContract {
            lifecycle_version: 1,
            network_id: "local".to_owned(),
            control_scope: "shared".to_owned(),
            contract_address: "0x0000000000000000000000000000000000000001".to_owned(),
            deploy_tx_hash: "0x1".to_owned(),
            deploy_receipt_artifact_id: Some("artifact".to_owned()),
            deployed_block_number: Some(1),
        }
    }

    fn sample_artifact() -> ContractArtifactConfig {
        ContractArtifactConfig {
            abi: mfm_evm_dcv_model::AbiJson::from_json_value(&serde_json::json!([
                {
                    "type": "constructor",
                    "inputs": []
                },
                {
                    "type": "function",
                    "name": "configure",
                    "inputs": [],
                    "outputs": []
                }
            ]))
            .expect("abi"),
            bytecode: mfm_evm_dcv_model::BytecodeJson::from_json_value(&serde_json::json!(
                "0x6000"
            ))
            .expect("bytecode"),
        }
    }

    fn sample_deploy_config() -> DeployConfigureValidateDeployConfig {
        DeployConfigureValidateDeployConfig {
            artifact: Some(sample_artifact()),
            network_id: "local".to_owned(),
            control_scope: "shared".to_owned(),
            from: "0x0000000000000000000000000000000000000000".to_owned(),
            constructor_args: Vec::new(),
            value_wei: None,
            signing_key_env: Some("MFM_TEST_KEY".to_owned()),
            poll_interval_ms: 1,
            max_receipt_polls: 1,
        }
    }

    fn sample_configure_config() -> DeployConfigureValidateConfigureConfig {
        DeployConfigureValidateConfigureConfig {
            artifact: Some(sample_artifact()),
            network_id: "local".to_owned(),
            control_scope: "shared".to_owned(),
            from: "0x0000000000000000000000000000000000000000".to_owned(),
            signing_key_env: Some("MFM_TEST_KEY".to_owned()),
            calls: vec![mfm_evm_dcv_model::ConfigureCallConfig {
                function: "configure".to_owned(),
                args: Vec::new(),
                value_wei: None,
            }],
            tx_hashes_export_key: "tx_hashes".to_owned(),
            receipts_export_key: "receipts".to_owned(),
            poll_interval_ms: 1,
            max_receipt_polls: 1,
        }
    }

    fn sample_validate_config() -> DeployConfigureValidateValidateConfig {
        DeployConfigureValidateValidateConfig {
            artifact: Some(sample_artifact()),
            network_id: "local".to_owned(),
            control_scope: "shared".to_owned(),
            expected_chain_id: 1,
            require_client_substring: "reth".to_owned(),
            read_assertions: Vec::new(),
            event_assertions: Vec::new(),
        }
    }
}
