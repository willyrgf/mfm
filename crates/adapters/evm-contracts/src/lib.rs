#![warn(missing_docs)]
//! EVM contract lifecycle adapter.
//!
//! This crate binds reusable contract lifecycle states to capability providers
//! and owns the runtime runner bindings for deploy, configure, and validate
//! lifecycle states. It does not own JSON-RPC endpoints, signer-provider
//! resolution, keystore loading, artifact-store implementations, binaries, or
//! operation topology. Runtime source routing and signing are supplied through
//! process-local provider traits.
//!
//! ```rust
//! use mfm_adapters_evm_contracts::replay_verifier_id;
//!
//! assert_eq!(replay_verifier_id()?.as_str(), "mfm.evm.contract.replay.v1");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{keccak256, Address, B256};
use mfm_adapter_contracts::evm_contract_lifecycle_adapter_binding;
use mfm_artifact_capabilities::ArtifactEvidenceRef as CapabilityArtifactEvidenceRef;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1::{self as events, side_effect};
use mfm_evm_capabilities::{
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockSelector, EvmCallReadProvider,
    EvmCallReadRequest, EvmCapabilityError, EvmChainIdentityProvider, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCodeReadProvider, EvmCodeReadRequest, EvmFeeReadProvider,
    EvmFeeReadRequest, EvmGasEstimateProvider, EvmGasEstimateRequest, EvmLogsReadProvider,
    EvmLogsReadRequest, EvmNetworkBinding, EvmNetworkId, EvmNonceOccupancy,
    EvmNonceOccupancyReadProvider, EvmNonceOccupancyReadRequest, EvmNonceReadProvider,
    EvmNonceReadRequest, EvmReceiptReadProvider, EvmReceiptReadRequest, EvmReceiptReadResponse,
    EvmTransactionSubmitCapability, EvmTransactionSubmitProvider, EvmTransactionSubmitRequest,
    RedactedEvmSourceEvidence, SignedEvmPayload,
};
use mfm_evm_contract_model::{
    configured_contract_stage, constructor_data, contract_instance_resource_kind,
    decode_single_output_to_json, deployed_contract_stage, expected_matches, parse_artifact,
    prepare_validate_assertions, resolve_function_call, validation_report_resource_kind,
    validation_report_stage, AcceptedContextPolicy, BlockSelector as ModelBlockSelector, BlockTag,
    ConfigurationClaim, ConfigurationSnapshot, ConfiguredContractInstance,
    ConfiguredContractInstanceRef, ContextBoundValidationReport, ContractArtifactConfig,
    ContractCallConfig, ContractLifecycleStage, ContractProfileDigestRef, DeployProvenance,
    DeployedContractInstance, EventAssertionConfig, EvmCodeHash, EvmContractContext,
    EvmNetworkContext, ExpectedValue, ExternalAdoptionEvidence, ExternalCodeReadEvidence,
    ExternalEventAssertionEvidence, ExternalEvmSourceEvidence, ExternalReadAssertionEvidence,
    ImportFromMfmRun, ImportFromMfmRunEvidence, LifecycleArtifactEvidenceRef, LifecycleNodeIdRef,
    ParsedAbi, ReadAssertionConfig, SourceCellOrOutputRef, SourceRunTerminalEvent,
    ValidationEventResult, ValidationReadResult,
};
use mfm_evm_core::encoding::normalize_address;
use mfm_evm_core::hex::{bytes_to_hex_prefixed, hex_to_bytes};
use mfm_evm_core::rlp::{rlp_encode_list, u64_to_min_be};
use mfm_evm_core::tx::{parse_address, parse_u128_quantity, Eip1559TxToSign, LegacyTxToSign};
use mfm_evm_signing::EvmSigningRequest;
use mfm_ids::{
    short_stable_id_fragment, ContentDigest, DigestAlgorithm, EventId, NodeId, SchemaId,
};
use mfm_program::{SideEffectState, StateSpec, ValidatedConfig};
use mfm_program_derive::MfmValue;
use mfm_replay::v1 as replay;
use mfm_runtime::{
    load_launch_config_for_node, load_materialized_struct_field_value, load_runner_config_for_node,
    load_side_effect_artifact_for_node, preclaim_side_effect_resource_lane,
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedInputs, PreInvocationRunCtx,
    PreInvocationRunnerFuture, RunnerCapabilityBinding, RunnerExecutableIdentityTemplate,
    RunnerIngressContext, RunnerRegistrationBuilder, RuntimeDiagnostic, RuntimeFailure,
    SideEffectDriver, SideEffectDriverCallbacks, SideEffectDriverFuture, SideEffectIntentPlan,
    SideEffectObservedEvidence, SideEffectProtocolAction, SideEffectReplayEvidence,
    SideEffectSubmissionDecision, SideEffectUnknownSubmissionDecision, SideEffectVerifyCallbacks,
    SideEffectVerifyDriver, TypedContextOutputExtractor,
};
use mfm_signing::{PublicKeyBytes, SignerRef, SigningProvider};
use mfm_spec::v1 as spec;
use mfm_state_evm_contracts::{
    account_nonce_resource_key_schema_id, account_nonce_resource_namespace,
    require_validation_event_result_canonical_passed,
    require_validation_read_result_canonical_passed, validation_event_result_passes,
    validation_read_result_passes, ContextBoundConfigureContractState,
    ContextBoundDeployContractState, ContextBoundValidateContractState,
    ContextConfigureContractInput, ContextContractConfigureConfirmation,
    ContextContractConfigureIntent, ContextContractConfigureReceipt, ContextContractDeployIntent,
    ContextContractValidationReadRequest, ContextValidateContractInput, ContractDeployConfirmation,
    ContractDeployReceipt, ContractTransactionIdempotency, ContractTransactionReceipt,
    ContractTransactionSubmission, ContractTransactionSubmissions, ContractValidationReadResponse,
    ImportConfiguredContractState, ImportDeployedContractState,
};
use mfm_state_evm_contracts::{
    ConfigureAction, DeployAction, EvmSignerIntent, EvmTransactionPolicy,
    EvmTransactionStyle as ConfigTransactionStyle, ImportConfiguredSpec, ImportDeployedSpec,
    ReceiptRetryPolicy, ValidateAction,
};
use mfm_store::v1 as store;
use mfm_values::{ContextBoundOutput, MfmConfig, MfmValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

#[path = "mutation_support.rs"]
mod mutation_support;
#[path = "read_validation.rs"]
mod read_validation;
#[path = "replay_adapter.rs"]
mod replay_adapter;
#[path = "runner_bindings.rs"]
mod runner_bindings;
pub(crate) use self::mutation_support::{
    block_selector, configure_action_requires_artifact, configure_action_transaction_inputs,
    contract_lifecycle_side_effect_missing, deploy_action_data,
    deploy_contract_address_from_prepared, digest_bytes, ensure_configured_input_context,
    ensure_deployed_input_context, ensure_replay_confirmation_depth, ensure_schema,
    evm_network_context_ref, lifecycle_evidence_ref, optional_policy_quantity, parse_optional_wei,
    parse_prepared_transaction_hash, prepare_transactions_request, prepared_anchor_submissions,
    prepared_mutation_reconstruction, public_key_hex, reconstruct_prepared_mutation,
    recover_unknown_prepared_contract_submission, replay_adapter_error, replay_contract_mismatch,
    replay_json_error, replay_model_error, replay_value_error, required_prepared_quantity,
    submit_or_recover_contract_submission, verify_prepared_submission_transactions,
    verify_prepared_submissions, verify_receipt_response, ContractMutationActionView,
    ContractMutationNetworkContext, PrepareTransactionsRequest, PreparedSubmissionReconciliation,
    PreparedTransaction, PreparedTransactionInput,
};
pub use self::mutation_support::{
    ensure_prepared_invocation_public, is_contract_lifecycle_replay_intent,
};
use self::read_validation::{
    context_stage_for_lifecycle_stage, decode_source_run_committed_stream, digest_for_value,
    import_configured_requires_artifact, import_configured_with_reads, import_deployed_with_reads,
    lifecycle_artifact_requirement, run_artifact_requirement, source_run_admitted,
    validate_context_contract_with_reads, validate_source_run_authority,
    validate_source_run_import_evidence, validate_source_run_import_policy,
    validation_assertions_required, SourceRunAuthorityEvidence,
};
use self::replay_adapter::contract_side_effect_replay_evidence;
pub use self::replay_adapter::{replay_verifier_id, verify_contract_lifecycle_replay};
#[cfg(test)]
use self::replay_adapter::{
    verify_contract_confirmation_schema, verify_contract_receipt_artifact,
    verify_contract_receipt_schema, verify_prepared_transaction_data_matches_inputs,
    verify_replay_intent_matches_prepared, verify_replayed_external_adoption,
    verify_validation_results_match_action, verify_validation_source_evidence,
    EvmContractLifecycleReplayVerifier,
};
pub(crate) use self::runner_bindings::contract_profile_artifact_requirement;
pub use self::runner_bindings::register_contract_lifecycle_runners_with_factory;
#[cfg(test)]
pub(crate) use self::runner_bindings::{
    account_nonce_resource_key, read_receipts_with_poll, verified_finality_confirmations,
    ContractNonceResourceScope,
};

const REPLAY_VERIFIER_ID: &str = "mfm.evm.contract.replay.v1";

/// Result type for lifecycle adapter operations.
pub type Result<T> = std::result::Result<T, EvmContractAdapterError>;

/// Capability providers needed for mutation phases.
#[derive(Clone, Copy)]
pub struct EvmContractMutationProviders<'a> {
    /// Chain identity provider.
    pub chain_identity: &'a dyn EvmChainIdentityProvider,
    /// Block summary provider.
    pub block: &'a dyn EvmBlockReadProvider,
    /// Nonce provider.
    pub nonce: &'a dyn EvmNonceReadProvider,
    /// Fee provider.
    pub fee: &'a dyn EvmFeeReadProvider,
    /// Gas estimate provider.
    pub gas: &'a dyn EvmGasEstimateProvider,
    /// Signer provider.
    pub signer: &'a dyn SigningProvider,
    /// Transaction submit provider.
    pub submit: &'a dyn EvmTransactionSubmitProvider,
    /// Receipt provider.
    pub receipt: &'a dyn EvmReceiptReadProvider,
    /// Nonce occupancy investigation provider.
    pub nonce_occupancy: &'a dyn EvmNonceOccupancyReadProvider,
}

impl<'a> EvmContractMutationProviders<'a> {
    /// Builds mutation provider bindings from process-local EVM and signer providers.
    pub fn from_evm_and_signer(
        evm: &'a dyn EvmContractProvider,
        signer: &'a dyn SigningProvider,
    ) -> Self {
        Self {
            chain_identity: evm,
            block: evm,
            nonce: evm,
            fee: evm,
            gas: evm,
            signer,
            submit: evm,
            receipt: evm,
            nonce_occupancy: evm,
        }
    }
}

impl fmt::Debug for EvmContractMutationProviders<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContractMutationProviders")
            .finish_non_exhaustive()
    }
}

/// Capability providers needed for validation reads.
#[derive(Clone, Copy)]
pub struct EvmContractReadProviders<'a> {
    /// Chain identity provider.
    pub chain_identity: &'a dyn EvmChainIdentityProvider,
    /// EVM code provider.
    pub code: &'a dyn EvmCodeReadProvider,
    /// EVM call provider.
    pub call: &'a dyn EvmCallReadProvider,
    /// EVM logs provider.
    pub logs: &'a dyn EvmLogsReadProvider,
}

impl<'a> EvmContractReadProviders<'a> {
    /// Builds read provider bindings from a process-local EVM read provider.
    pub fn from_provider(evm: &'a dyn EvmContractReadProvider) -> Self {
        Self {
            chain_identity: evm,
            code: evm,
            call: evm,
            logs: evm,
        }
    }

    /// Builds read provider bindings from a full EVM contract provider.
    pub fn from_contract_provider(evm: &'a dyn EvmContractProvider) -> Self {
        Self {
            chain_identity: evm,
            code: evm,
            call: evm,
            logs: evm,
        }
    }
}

impl fmt::Debug for EvmContractReadProviders<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContractReadProviders")
            .finish_non_exhaustive()
    }
}

/// Prepared invocation evidence retained before a contract mutation is started.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "prepared-invocation",
    schema = "mfm.evm.contract.adapter.prepared_invocation"
)]
pub struct PreparedContractInvocation {
    /// Evidence contract version.
    pub prepared_version: u64,
    /// Mutation phase.
    pub phase: ContractMutationPhase,
    /// Certified contract lifecycle context ref.
    pub context_ref: mfm_values::ContextRefValue,
    /// Canonical content ref string for the certified EVM network context.
    pub evm_network_context_ref: String,
    /// Context resource stage whose nonce lane is being mutated.
    pub resource_stage: ContractLifecycleStage,
    /// Semantic network id from typed config.
    pub network_id: String,
    /// Expected EVM chain id.
    pub expected_chain_id: u64,
    /// Redaction-safe signer reference from typed config.
    pub signer_ref: String,
    /// Expected public signer address.
    pub expected_signer_address: String,
    /// Prepared transaction evidence.
    pub transactions: Vec<PreparedContractTransactionEvidence>,
    /// Receipt poll interval in milliseconds.
    pub poll_interval_ms: u64,
    /// Maximum receipt polls.
    pub max_receipt_polls: u64,
}

/// Contract mutation phase.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "mutation-phase",
    schema = "mfm.evm.contract.adapter.mutation_phase"
)]
pub enum ContractMutationPhase {
    /// Deployment phase.
    Deploy,
    /// Configuration phase.
    Configure,
}

/// Prepared transaction style.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "prepared-transaction-style",
    schema = "mfm.evm.contract.adapter.transaction_style"
)]
pub enum PreparedContractTransactionStyle {
    /// EIP-1559 dynamic fee transaction.
    Eip1559,
    /// Legacy gas-price transaction.
    Legacy,
}

/// Prepared transaction evidence with no raw signed payload or signature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "prepared-transaction-evidence",
    schema = "mfm.evm.contract.adapter.prepared_transaction"
)]
pub struct PreparedContractTransactionEvidence {
    /// Transaction index within the phase.
    pub index: u64,
    /// Prepared transaction style.
    pub style: PreparedContractTransactionStyle,
    /// Chain id used for signing.
    pub chain_id: u64,
    /// Sender nonce.
    pub nonce: u64,
    /// Optional destination address.
    pub to_address: Option<String>,
    /// Native token value in wei.
    pub value_wei: String,
    /// Gas limit.
    pub gas_limit: u64,
    /// EIP-1559 max fee per gas.
    pub max_fee_per_gas: Option<String>,
    /// EIP-1559 priority fee per gas.
    pub max_priority_fee_per_gas: Option<String>,
    /// Legacy gas price.
    pub gas_price: Option<String>,
    /// Unsigned data digest.
    pub data_digest: String,
    /// Unsigned data byte length.
    pub data_len: u64,
    /// Signing digest requested from the signer provider.
    pub signing_digest: String,
    /// Expected signed transaction hash reconstructed from deterministic signing.
    pub expected_transaction_hash: String,
}

/// Defended proof that a prepared anchor was not the transaction occupying its sender nonce.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "not-submitted-proof",
    schema = "mfm.evm.contract.adapter.not_submitted_proof"
)]
pub struct ContractNotSubmittedProof {
    /// Proof schema version.
    pub proof_version: u32,
    /// Expected MFM-authored transaction hash from the prepared invocation.
    pub expected_transaction_hash: String,
    /// Non-anchor transaction observed at the same sender nonce.
    pub occupying_transaction_hash: String,
    /// Mined block number for the occupying transaction, when known.
    pub occupying_block_number: Option<u64>,
    /// Sender address bound by the prepared invocation.
    pub signer_address: String,
    /// Sender nonce that was occupied.
    pub nonce: u64,
    /// EVM chain id reported by the evidence source.
    pub evidence_chain_id: u64,
}

/// Prepared mutation containing public evidence plus transient signing requests.
pub struct PreparedContractMutation {
    evidence: PreparedContractInvocation,
    signing_requests: Vec<EvmSigningRequest>,
}

impl PreparedContractMutation {
    /// Creates prepared mutation evidence from transient signing requests.
    fn new(
        evidence: PreparedContractInvocation,
        signing_requests: Vec<EvmSigningRequest>,
    ) -> Result<Self> {
        ensure_prepared_invocation_public(&evidence)?;
        if evidence.transactions.len() != signing_requests.len() {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }
        for transaction in &evidence.transactions {
            parse_prepared_transaction_hash(&transaction.expected_transaction_hash)?;
        }
        Ok(Self {
            evidence,
            signing_requests,
        })
    }

    /// Returns serializable prepared invocation evidence.
    pub const fn evidence(&self) -> &PreparedContractInvocation {
        &self.evidence
    }

    /// Returns transient signing requests.
    pub fn signing_requests(&self) -> &[EvmSigningRequest] {
        &self.signing_requests
    }
}

impl fmt::Debug for PreparedContractMutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedContractMutation")
            .field("evidence", &self.evidence)
            .field("signing_requests", &self.signing_requests.len())
            .finish()
    }
}

/// EVM contract lifecycle adapter over explicit capability providers.
#[derive(Debug, Clone, Copy)]
pub struct EvmContractLifecycleAdapter<'a> {
    mutation: EvmContractMutationProviders<'a>,
    reads: EvmContractReadProviders<'a>,
}

impl<'a> EvmContractLifecycleAdapter<'a> {
    /// Creates an adapter from process-local provider sets.
    pub fn new(
        mutation: EvmContractMutationProviders<'a>,
        reads: EvmContractReadProviders<'a>,
    ) -> Self {
        Self { mutation, reads }
    }

    /// Reads the latest block number for mutation finality checks.
    pub async fn latest_block_number(&self) -> Result<u64> {
        let response = self
            .mutation
            .block
            .read_block(&EvmBlockReadRequest::new(EvmBlockSelector::Latest))
            .await?;
        Ok(response.block_number)
    }

    /// Prepares a context-bound deploy invocation from certified context.
    pub async fn prepare_context_deploy_invocation(
        &self,
        action: &ValidatedConfig<DeployAction>,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifact: &ContractArtifactConfig,
        intent: &ContextContractDeployIntent,
    ) -> Result<PreparedContractMutation> {
        let expected =
            ContextBoundDeployContractState::new(action.clone())?.prepare_intent(&(), context)?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        let action = action.as_ref();
        let data = deploy_action_data(action, artifact)?;
        self.prepare_transactions(prepare_transactions_request(
            ContractMutationPhase::Deploy,
            ContractMutationNetworkContext::from_context(
                context,
                ContractLifecycleStage::Deployed,
            )?,
            action,
            vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(action.value_wei())?,
                data,
            }],
        )?)
        .await
    }

    /// Prepares context-bound configure invocations from certified context.
    pub async fn prepare_context_configure_invocation(
        &self,
        action: &ValidatedConfig<ConfigureAction>,
        input: &ContextConfigureContractInput,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifact: Option<&ContractArtifactConfig>,
        intent: &ContextContractConfigureIntent,
    ) -> Result<PreparedContractMutation> {
        let expected = ContextBoundConfigureContractState::new(action.clone())?
            .prepare_intent(input, context)?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        ensure_deployed_input_context(input, context)?;
        let action = action.as_ref();
        let tx_inputs =
            configure_action_transaction_inputs(action, artifact, input.deployed.address.as_str())?;
        self.prepare_transactions(prepare_transactions_request(
            ContractMutationPhase::Configure,
            ContractMutationNetworkContext::from_context(
                context,
                ContractLifecycleStage::Configured,
            )?,
            action,
            tx_inputs,
        )?)
        .await
    }

    /// Reconstructs context-bound deploy signing requests from persisted prepared evidence.
    pub fn reconstruct_context_deploy_invocation(
        &self,
        action: &ValidatedConfig<DeployAction>,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifact: &ContractArtifactConfig,
        intent: &ContextContractDeployIntent,
        evidence: &PreparedContractInvocation,
    ) -> Result<PreparedContractMutation> {
        let expected =
            ContextBoundDeployContractState::new(action.clone())?.prepare_intent(&(), context)?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        let action = action.as_ref();
        reconstruct_prepared_mutation(prepared_mutation_reconstruction(
            evidence,
            ContractMutationPhase::Deploy,
            ContractMutationNetworkContext::from_context(
                context,
                ContractLifecycleStage::Deployed,
            )?,
            action,
            vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(action.value_wei())?,
                data: deploy_action_data(action, artifact)?,
            }],
        )?)
    }

    /// Reconstructs context-bound configure signing requests from persisted prepared evidence.
    pub fn reconstruct_context_configure_invocation(
        &self,
        action: &ValidatedConfig<ConfigureAction>,
        input: &ContextConfigureContractInput,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifact: Option<&ContractArtifactConfig>,
        intent: &ContextContractConfigureIntent,
        evidence: &PreparedContractInvocation,
    ) -> Result<PreparedContractMutation> {
        let expected = ContextBoundConfigureContractState::new(action.clone())?
            .prepare_intent(input, context)?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        ensure_deployed_input_context(input, context)?;
        let action = action.as_ref();
        reconstruct_prepared_mutation(prepared_mutation_reconstruction(
            evidence,
            ContractMutationPhase::Configure,
            ContractMutationNetworkContext::from_context(
                context,
                ContractLifecycleStage::Configured,
            )?,
            action,
            configure_action_transaction_inputs(action, artifact, input.deployed.address.as_str())?,
        )?)
    }

    /// Signs and submits all prepared transactions.
    pub async fn submit_prepared(
        &self,
        prepared: &PreparedContractMutation,
    ) -> Result<Vec<ContractTransactionSubmission>> {
        let mut submissions = Vec::with_capacity(prepared.signing_requests.len());
        for (transaction, signing_request) in prepared
            .evidence()
            .transactions
            .iter()
            .zip(prepared.signing_requests())
        {
            let result = self
                .mutation
                .signer
                .sign(signing_request.signing_request())
                .await?;
            let raw = signing_request.materialize_signed_payload(&result)?;
            let expected_hash = raw
                .transaction_hash()
                .parse::<B256>()
                .map_err(|_| EvmContractAdapterError::TransactionHashMismatch)?;
            let prepared_hash =
                parse_prepared_transaction_hash(&transaction.expected_transaction_hash)?;
            if expected_hash != prepared_hash {
                return Err(EvmContractAdapterError::TransactionHashMismatch);
            }
            let payload =
                SignedEvmPayload::from_verified_bytes(raw.bytes().to_vec(), expected_hash)?;
            let response = self
                .mutation
                .submit
                .submit_transaction(&EvmTransactionSubmitRequest::new(payload))
                .await?;
            if response.transaction_hash != expected_hash {
                return Err(EvmContractAdapterError::TransactionHashMismatch);
            }
            submissions.push(ContractTransactionSubmission {
                submission_version: 1,
                context_ref: prepared.evidence().context_ref.clone(),
                evm_network_context_ref: prepared.evidence().evm_network_context_ref.clone(),
                resource_stage: prepared.evidence().resource_stage,
                transaction_hash: format!("{expected_hash:?}"),
                signer_public_key: public_key_hex(result.public_identity().public_key()),
            });
        }
        Ok(submissions)
    }

    /// Reads one confirmed receipt for each submitted transaction.
    pub async fn read_receipts(
        &self,
        prepared: &PreparedContractInvocation,
        submissions: &[ContractTransactionSubmission],
    ) -> Result<Vec<ContractTransactionReceipt>> {
        let transaction_hashes = verify_prepared_submission_transactions(prepared, submissions)?;
        let mut receipts = Vec::with_capacity(submissions.len());
        for transaction_hash in transaction_hashes {
            let response = self
                .mutation
                .receipt
                .read_receipt(&EvmReceiptReadRequest::new(transaction_hash))
                .await?;
            let response = verify_receipt_response(transaction_hash, response)?;
            receipts.push(ContractTransactionReceipt {
                receipt_version: 1,
                context_ref: prepared.context_ref.clone(),
                evm_network_context_ref: prepared.evm_network_context_ref.clone(),
                resource_stage: prepared.resource_stage,
                transaction_hash: format!("{:?}", response.transaction_hash),
                block_number: response.block_number,
                status: response.status,
                receipt_evidence: None,
            });
        }
        Ok(receipts)
    }

    async fn reconcile_prepared_submission(
        &self,
        prepared: &PreparedContractInvocation,
    ) -> Result<PreparedSubmissionReconciliation> {
        let anchor_submissions = prepared_anchor_submissions(prepared)?;
        let mut unlanded_transactions = Vec::new();
        for (transaction, submission) in prepared
            .transactions
            .iter()
            .zip(anchor_submissions.transactions.iter())
        {
            let transaction_hash = submission
                .transaction_hash
                .parse::<B256>()
                .map_err(|_| EvmContractAdapterError::TransactionHashMismatch)?;
            match self
                .mutation
                .receipt
                .read_receipt(&EvmReceiptReadRequest::new(transaction_hash))
                .await
            {
                Ok(response) => {
                    verify_receipt_response(transaction_hash, response)?;
                }
                Err(EvmCapabilityError::ReceiptPending) => {
                    unlanded_transactions.push(transaction);
                }
                Err(EvmCapabilityError::Provider { .. }) => {
                    return Ok(PreparedSubmissionReconciliation::Indeterminate(
                        anchor_submissions,
                    ));
                }
                Err(error) => return Err(error.into()),
            }
        }

        if unlanded_transactions.is_empty() {
            return Ok(PreparedSubmissionReconciliation::Observed(
                anchor_submissions,
            ));
        }

        let expected_signer =
            parse_address(&prepared.expected_signer_address, "expected_signer_address")
                .map_err(|error| EvmContractAdapterError::Model(error.message))?;
        let pending_nonce = match self
            .mutation
            .nonce
            .read_nonce(&EvmNonceReadRequest::new(
                expected_signer,
                EvmBlockSelector::Pending,
            ))
            .await
        {
            Ok(response) => response.nonce,
            Err(EvmCapabilityError::Provider { .. }) => {
                return Ok(PreparedSubmissionReconciliation::Indeterminate(
                    anchor_submissions,
                ));
            }
            Err(error) => return Err(error.into()),
        };
        for transaction in &unlanded_transactions {
            if pending_nonce <= transaction.nonce {
                continue;
            }
            match self
                .mutation
                .nonce_occupancy
                .read_nonce_occupancy(&EvmNonceOccupancyReadRequest::new(
                    expected_signer,
                    transaction.nonce,
                    parse_prepared_transaction_hash(&transaction.expected_transaction_hash)?,
                ))
                .await
            {
                Ok(response) => match response.outcome {
                    EvmNonceOccupancy::Occupied {
                        transaction_hash,
                        block_number,
                    } => {
                        return Ok(PreparedSubmissionReconciliation::NotSubmitted(
                            ContractNotSubmittedProof {
                                proof_version: 1,
                                expected_transaction_hash: transaction
                                    .expected_transaction_hash
                                    .clone(),
                                occupying_transaction_hash: format!("{transaction_hash:?}"),
                                occupying_block_number: block_number,
                                signer_address: prepared.expected_signer_address.clone(),
                                nonce: transaction.nonce,
                                evidence_chain_id: response.evidence.observed_chain_id,
                            },
                        ));
                    }
                    EvmNonceOccupancy::Unknown => {
                        return Ok(PreparedSubmissionReconciliation::Indeterminate(
                            anchor_submissions,
                        ));
                    }
                },
                Err(EvmCapabilityError::Provider { .. }) => {
                    return Ok(PreparedSubmissionReconciliation::Indeterminate(
                        anchor_submissions,
                    ));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(PreparedSubmissionReconciliation::NotObserved(
            anchor_submissions,
        ))
    }

    /// Executes context-bound validation reads against certified context.
    pub async fn validate_context_contract(
        &self,
        action: &ValidatedConfig<ValidateAction>,
        input: &ContextValidateContractInput,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifact: Option<&ContractArtifactConfig>,
        request: &ContextContractValidationReadRequest,
    ) -> Result<ContractValidationReadResponse> {
        validate_context_contract_with_reads(self.reads, action, input, context, artifact, request)
            .await
    }

    async fn prepare_transactions(
        &self,
        request: PrepareTransactionsRequest<'_>,
    ) -> Result<PreparedContractMutation> {
        let PrepareTransactionsRequest {
            phase,
            context_ref,
            evm_network_context_ref,
            resource_stage,
            network_id,
            expected_chain_id,
            signer_ref,
            expected_signer,
            expected_signer_text,
            policy,
            poll_interval_ms,
            max_receipt_polls,
            tx_inputs,
        } = request;

        let nonce_response = self
            .mutation
            .nonce
            .read_nonce(&EvmNonceReadRequest::new(
                expected_signer,
                EvmBlockSelector::Latest,
            ))
            .await?;
        let nonce = nonce_response.nonce;
        let fees = self
            .mutation
            .fee
            .read_fee(&EvmFeeReadRequest::new())
            .await?;

        let mut evidence = Vec::with_capacity(tx_inputs.len());
        let mut signing_requests = Vec::with_capacity(tx_inputs.len());
        for (index, input) in tx_inputs.into_iter().enumerate() {
            let gas_limit = match policy.gas_limit() {
                Some(gas_limit) => gas_limit,
                None => {
                    let gas = self
                        .mutation
                        .gas
                        .estimate_gas(&EvmGasEstimateRequest::new(
                            Some(expected_signer),
                            input.to,
                            input.value_wei,
                            input.data.clone(),
                        ))
                        .await?;
                    gas.gas_limit
                }
            };
            let tx_nonce = nonce
                .checked_add(index as u64)
                .ok_or(EvmContractAdapterError::InvalidPreparedInvocation)?;
            let prepared = match policy.style() {
                ConfigTransactionStyle::Eip1559 => {
                    let max_fee = optional_policy_quantity(policy.max_fee_per_gas(), "max_fee")?
                        .or(fees.max_fee_per_gas)
                        .ok_or(EvmContractAdapterError::FeeUnavailable)?;
                    let priority_fee = optional_policy_quantity(
                        policy.max_priority_fee_per_gas(),
                        "priority_fee",
                    )?
                    .or(fees.priority_fee_per_gas)
                    .ok_or(EvmContractAdapterError::FeeUnavailable)?;
                    let tx = Eip1559TxToSign {
                        to: input.to,
                        value_wei: input.value_wei,
                        chain_id: expected_chain_id,
                        nonce: tx_nonce,
                        max_fee_per_gas: max_fee,
                        max_priority_fee_per_gas: priority_fee,
                        gas_limit,
                        data: input.data.clone(),
                    };
                    let request =
                        EvmSigningRequest::eip1559(signer_ref.clone(), tx, expected_signer)?;
                    PreparedTransaction {
                        style: PreparedContractTransactionStyle::Eip1559,
                        request,
                        max_fee_per_gas: Some(max_fee),
                        max_priority_fee_per_gas: Some(priority_fee),
                        gas_price: None,
                    }
                }
                ConfigTransactionStyle::Legacy => {
                    let gas_price = optional_policy_quantity(policy.gas_price(), "gas_price")?
                        .or(fees.legacy_gas_price)
                        .ok_or(EvmContractAdapterError::FeeUnavailable)?;
                    let tx = LegacyTxToSign {
                        to: input.to,
                        value_wei: input.value_wei,
                        chain_id: expected_chain_id,
                        nonce: tx_nonce,
                        gas_price_wei: gas_price,
                        gas_limit,
                        data: input.data.clone(),
                    };
                    let request =
                        EvmSigningRequest::legacy(signer_ref.clone(), tx, expected_signer)?;
                    PreparedTransaction {
                        style: PreparedContractTransactionStyle::Legacy,
                        request,
                        max_fee_per_gas: None,
                        max_priority_fee_per_gas: None,
                        gas_price: Some(gas_price),
                    }
                }
            };
            let expected_transaction_hash = self
                .expected_transaction_hash_for_request(&prepared.request)
                .await?;
            evidence.push(PreparedContractTransactionEvidence {
                index: index as u64,
                style: prepared.style,
                chain_id: expected_chain_id,
                nonce: tx_nonce,
                to_address: input.to.map(|address| format!("{address:?}")),
                value_wei: input.value_wei.to_string(),
                gas_limit,
                max_fee_per_gas: prepared.max_fee_per_gas.map(|value| value.to_string()),
                max_priority_fee_per_gas: prepared
                    .max_priority_fee_per_gas
                    .map(|value| value.to_string()),
                gas_price: prepared.gas_price.map(|value| value.to_string()),
                data_digest: digest_bytes(&input.data).to_string(),
                data_len: input.data.len() as u64,
                signing_digest: format!("{:?}", prepared.request.signing_hash()),
                expected_transaction_hash: format!("{expected_transaction_hash:?}"),
            });
            signing_requests.push(prepared.request);
        }

        PreparedContractMutation::new(
            PreparedContractInvocation {
                prepared_version: 1,
                phase,
                context_ref,
                evm_network_context_ref,
                resource_stage,
                network_id: network_id.to_owned(),
                expected_chain_id,
                signer_ref: signer_ref.to_string(),
                expected_signer_address: normalize_address(expected_signer_text)
                    .map_err(|error| EvmContractAdapterError::Model(error.message))?,
                transactions: evidence,
                poll_interval_ms,
                max_receipt_polls,
            },
            signing_requests,
        )
    }

    async fn expected_transaction_hash_for_request(
        &self,
        signing_request: &EvmSigningRequest,
    ) -> Result<B256> {
        let result = self
            .mutation
            .signer
            .sign(signing_request.signing_request())
            .await?;
        let raw = signing_request.materialize_signed_payload(&result)?;
        raw.transaction_hash()
            .parse::<B256>()
            .map_err(|_| EvmContractAdapterError::TransactionHashMismatch)
    }
}

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const ADAPTER_FACTORY: &str = "evm_contract_lifecycle_adapter";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.evm_contracts.runtime.v1";

/// EVM capability provider set required by contract lifecycle runners.
///
/// Concrete implementations are supplied by app assembly or tests; this trait
/// only groups the capability contracts the adapter needs.
pub trait EvmContractProvider:
    EvmChainIdentityProvider
    + EvmBlockReadProvider
    + EvmNonceReadProvider
    + EvmFeeReadProvider
    + EvmGasEstimateProvider
    + EvmTransactionSubmitProvider
    + EvmReceiptReadProvider
    + EvmNonceOccupancyReadProvider
    + EvmCodeReadProvider
    + EvmCallReadProvider
    + EvmLogsReadProvider
{
}

impl<T> EvmContractProvider for T where
    T: EvmChainIdentityProvider
        + EvmBlockReadProvider
        + EvmNonceReadProvider
        + EvmFeeReadProvider
        + EvmGasEstimateProvider
        + EvmTransactionSubmitProvider
        + EvmReceiptReadProvider
        + EvmNonceOccupancyReadProvider
        + EvmCodeReadProvider
        + EvmCallReadProvider
        + EvmLogsReadProvider
{
}

/// EVM capability provider set required by contract validation reads.
pub trait EvmContractReadProvider:
    EvmChainIdentityProvider + EvmCodeReadProvider + EvmCallReadProvider + EvmLogsReadProvider
{
}

impl<T> EvmContractReadProvider for T where
    T: EvmChainIdentityProvider + EvmCodeReadProvider + EvmCallReadProvider + EvmLogsReadProvider
{
}

/// Factory for per-network EVM contract runtime bindings.
pub trait EvmContractRuntimeFactory: Send + Sync {
    /// Returns the artifact reader used to materialize configs, inputs, and side-effect evidence.
    fn artifacts(&self) -> &dyn store::RetainedArtifactReadProvider;

    /// Validates process-local runtime bindings for launch ingress before `RunAdmitted`.
    fn validate_runtime_for(
        &self,
        binding: &EvmNetworkBinding,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()>;

    /// Returns process-local EVM read capabilities bound to the certified network.
    fn read_runtime_for(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_runtime::Result<EvmContractReadRuntime>;

    /// Returns process-local EVM and signing capabilities bound to the certified network.
    fn runtime_for(&self, binding: EvmNetworkBinding) -> mfm_runtime::Result<EvmContractRuntime>;
}

/// Process-local read capability set for one EVM contract lifecycle route.
#[derive(Clone)]
pub struct EvmContractReadRuntime {
    evm: Arc<dyn EvmContractReadProvider>,
    source_run_registry: Option<mfm_certify::CertificationRegistry>,
}

impl fmt::Debug for EvmContractReadRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContractReadRuntime")
            .finish_non_exhaustive()
    }
}

impl EvmContractReadRuntime {
    /// Creates an EVM contract read runtime from explicit process-local providers.
    pub fn new(evm: Arc<dyn EvmContractReadProvider>) -> Self {
        Self {
            evm,
            source_run_registry: None,
        }
    }

    /// Creates an EVM contract read runtime with source-run import registry authority.
    pub fn new_with_source_run_import_registry(
        evm: Arc<dyn EvmContractReadProvider>,
        source_run_registry: mfm_certify::CertificationRegistry,
    ) -> Self {
        Self {
            evm,
            source_run_registry: Some(source_run_registry),
        }
    }

    /// Executes context-bound validation reads through this process-local read runtime.
    pub async fn validate_context_contract(
        &self,
        action: &ValidatedConfig<ValidateAction>,
        input: &ContextValidateContractInput,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifact: Option<&ContractArtifactConfig>,
        request: &ContextContractValidationReadRequest,
    ) -> Result<ContractValidationReadResponse> {
        let evm = self.evm.as_ref();
        validate_context_contract_with_reads(
            EvmContractReadProviders::from_provider(evm),
            action,
            input,
            context,
            artifact,
            request,
        )
        .await
    }

    /// Admits a deployed-stage import through this process-local read runtime.
    pub async fn import_deployed(
        &self,
        import: &ImportDeployedSpec,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifacts: &dyn store::RetainedArtifactReadProvider,
    ) -> Result<DeployedContractInstance> {
        let evm = self.evm.as_ref();
        import_deployed_with_reads(
            EvmContractReadProviders::from_provider(evm),
            import,
            context,
            artifacts,
            self.source_run_registry.as_ref(),
        )
        .await
    }

    /// Admits a configured-stage import through this process-local read runtime.
    pub async fn import_configured(
        &self,
        import: &ImportConfiguredSpec,
        context: &mfm_program::CertifiedContext<EvmContractContext>,
        artifact: Option<&ContractArtifactConfig>,
        artifacts: &dyn store::RetainedArtifactReadProvider,
    ) -> Result<ConfiguredContractInstance> {
        let evm = self.evm.as_ref();
        import_configured_with_reads(
            EvmContractReadProviders::from_provider(evm),
            import,
            context,
            artifact,
            artifacts,
            self.source_run_registry.as_ref(),
        )
        .await
    }
}

/// Process-local runtime capability set for one EVM contract lifecycle route.
#[derive(Clone)]
pub struct EvmContractRuntime {
    evm: Arc<dyn EvmContractProvider>,
    signer: Arc<dyn SigningProvider>,
}

impl fmt::Debug for EvmContractRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContractRuntime").finish_non_exhaustive()
    }
}

impl EvmContractRuntime {
    /// Creates an EVM contract runtime from explicit process-local providers.
    pub fn new(evm: Arc<dyn EvmContractProvider>, signer: Arc<dyn SigningProvider>) -> Self {
        Self { evm, signer }
    }

    fn adapter(&self) -> EvmContractLifecycleAdapter<'_> {
        let evm = self.evm.as_ref();
        EvmContractLifecycleAdapter::new(
            EvmContractMutationProviders::from_evm_and_signer(evm, self.signer.as_ref()),
            EvmContractReadProviders::from_contract_provider(evm),
        )
    }
}

/// Redaction-safe contract adapter error.
#[derive(Debug, thiserror::Error)]
pub enum EvmContractAdapterError {
    /// Stable identity construction failed.
    #[error("contract adapter identity failed: {0}")]
    Identity(String),
    /// Launch config extraction failed.
    #[error("contract config failed: {0}")]
    Config(String),
    /// State contract failed.
    #[error("contract state failed: {0}")]
    State(String),
    /// EVM capability provider failed.
    #[error("EVM capability failed: {0}")]
    EvmCapability(#[from] mfm_evm_capabilities::EvmCapabilityError),
    /// Signing provider failed.
    #[error("signing provider failed: {0}")]
    Signing(#[from] mfm_signing::SigningError),
    /// EVM signing bridge failed.
    #[error("EVM signing failed: {0}")]
    EvmSigning(#[from] mfm_evm_signing::EvmSigningError),
    /// Artifact read failed.
    #[error("artifact read failed: {0}")]
    ArtifactRead(mfm_artifact_capabilities::ArtifactReadError),
    /// Pure contract model preparation failed.
    #[error("contract model failed: {0}")]
    Model(String),
    /// Prepared invocation did not match state intent.
    #[error("contract intent did not match state config")]
    IntentMismatch,
    /// Context-bound input or evidence did not match certified context authority.
    #[error("contract context mismatch")]
    ContextMismatch,
    /// Required contract artifact was absent.
    #[error("contract artifact is required for this lifecycle phase")]
    MissingContractArtifact,
    /// Source-run import evidence was missing, unreadable, or mismatched.
    #[error("source-run import evidence failed validation: {0}")]
    SourceRunImportEvidence(String),
    /// Import evidence targeted a different lifecycle stage.
    #[error("contract import stage mismatch")]
    ImportStageMismatch,
    /// External adoption required bytecode but none was observed.
    #[error("external contract code was missing")]
    ExternalCodeMissing,
    /// External adoption observed bytecode did not match the required hash.
    #[error("external contract code hash mismatch")]
    ExternalCodeHashMismatch,
    /// External configured adoption assertions did not all pass.
    #[error("external configured adoption assertions failed")]
    ExternalAdoptionAssertionsFailed,
    /// External configured adoption did not provide required observed evidence.
    #[error("external configured claim is not allowed by policy")]
    ExternalConfiguredClaimNotAllowed,
    /// Fee data was unavailable for the requested transaction style.
    #[error("EVM fee data unavailable for transaction style")]
    FeeUnavailable,
    /// Transaction hash did not match across signing/submission/receipt.
    #[error("EVM transaction hash mismatch")]
    TransactionHashMismatch,
    /// Transaction receipt reported failure.
    #[error("EVM transaction failed")]
    TransactionFailed,
    /// Prepared invocation evidence was malformed.
    #[error("prepared invocation was invalid")]
    InvalidPreparedInvocation,
    /// The configured EVM block tag is not supported by the capability contract.
    #[error("EVM block tag is not supported by this adapter")]
    UnsupportedBlockTag,
}

impl From<mfm_program::PlanError> for EvmContractAdapterError {
    fn from(error: mfm_program::PlanError) -> Self {
        Self::State(error.to_string())
    }
}

impl From<mfm_program::StateError> for EvmContractAdapterError {
    fn from(error: mfm_program::StateError) -> Self {
        Self::State(error.to_string())
    }
}

impl From<EvmContractAdapterError> for mfm_runtime::RuntimeError {
    fn from(error: EvmContractAdapterError) -> Self {
        match error {
            EvmContractAdapterError::EvmCapability(EvmCapabilityError::SourceMismatch {
                diagnostic,
            }) => {
                let failure = RuntimeFailure::new(
                    events::ErrorCode::new("evm_source_mismatch")
                        .expect("EVM source mismatch code is a checked public code"),
                    events::ErrorCategory::Capability,
                    "EVM source evidence did not match provider binding",
                )
                .expect("EVM source mismatch metadata is a checked public contract");
                Self::InvalidRunnerOutputFailure {
                    failure,
                    diagnostic: Some(Box::new(RuntimeDiagnostic::provider(diagnostic))),
                }
            }
            error => Self::InvalidRunnerOutput(error.to_string()),
        }
    }
}

impl From<EvmContractAdapterError> for replay::ReplayError {
    fn from(error: EvmContractAdapterError) -> Self {
        Self::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            error.to_string(),
        )
    }
}

#[cfg(test)]
mod tests;
