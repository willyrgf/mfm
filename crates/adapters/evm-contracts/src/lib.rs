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
//! use mfm_evm_capabilities::{EvmChainGuard, EvmNetworkId};
//!
//! let guard = EvmChainGuard::new(EvmNetworkId::new("reth-dev")?, 31337)?;
//! assert_eq!(guard.expected_chain_id(), 31337);
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
    EvmCallReadRequest, EvmCapabilityError, EvmChainGuard, EvmChainIdentityProvider,
    EvmChainIdentityRequest, EvmChainIdentityResponse, EvmCodeReadProvider, EvmCodeReadRequest,
    EvmFeeReadProvider, EvmFeeReadRequest, EvmGasEstimateProvider, EvmGasEstimateRequest,
    EvmLogsReadProvider, EvmLogsReadRequest, EvmNetworkId, EvmNonceOccupancy,
    EvmNonceOccupancyReadProvider, EvmNonceOccupancyReadRequest, EvmNonceReadProvider,
    EvmNonceReadRequest, EvmReceiptReadProvider, EvmReceiptReadRequest, EvmReceiptReadResponse,
    EvmTransactionSubmitCapability, EvmTransactionSubmitProvider, EvmTransactionSubmitRequest,
    RedactedEvmSourceEvidence, SignedEvmPayload,
};
use mfm_evm_contract_config::{
    ConfigureAction, DeployAction, EvmSignerIntent, EvmTransactionPolicy,
    EvmTransactionStyle as ConfigTransactionStyle, ImportConfiguredSpec, ImportDeployedSpec,
    ReceiptRetryPolicy, ValidateAction,
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
    load_launch_config_for_node, load_materialized_struct_field_value, load_runner_config,
    load_runner_config_for_node, load_side_effect_artifact, load_side_effect_value,
    load_side_effect_value_for_node, CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedInputs,
    PreInvocationRunCtx, PreInvocationRunnerFuture, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerRegistrationBuilder,
    SideEffectDriver, SideEffectDriverCallbacks, SideEffectDriverFuture, SideEffectIntentPlan,
    SideEffectLanePreclaimBuilder, SideEffectObservedEvidence, SideEffectPreparedInvocationPlan,
    SideEffectProtocolAction, SideEffectReplayEvidence, SideEffectSubmissionDecision,
    SideEffectSubmissionDecisionFuture, SideEffectUnknownSubmissionDecision,
    SideEffectUnknownSubmissionDecisionFuture, SideEffectVerifyCallbacks, SideEffectVerifyDriver,
    TypedContextOutputExtractor,
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
use mfm_store::v1 as store;
use mfm_values::{ContextBoundOutput, MfmConfig, MfmValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

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
    pub async fn latest_block_number(
        &self,
        network_id: &str,
        expected_chain_id: u64,
    ) -> Result<u64> {
        let guard = evm_chain_guard(network_id, expected_chain_id)?;
        let response = self
            .mutation
            .block
            .read_block(&EvmBlockReadRequest {
                guard: guard.clone(),
                block: EvmBlockSelector::Latest,
            })
            .await?;
        response.evidence.verify_guard(&guard)?;
        Ok(response.block_number)
    }

    /// Prepares a context-bound deploy invocation from certified context authority.
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
            ContractMutationNetworkAuthority::from_context(
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

    /// Prepares context-bound configure invocations from certified context authority.
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
            ContractMutationNetworkAuthority::from_context(
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
            ContractMutationNetworkAuthority::from_context(
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
            ContractMutationNetworkAuthority::from_context(
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
        let verified_chain = verified_chain_identity(
            self.mutation.chain_identity,
            &prepared.evidence().network_id,
            prepared.evidence().expected_chain_id,
        )
        .await?;
        let guard = verified_chain.guard;
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
                .submit_transaction(&EvmTransactionSubmitRequest {
                    guard: guard.clone(),
                    signed_payload: payload,
                })
                .await?;
            response.evidence.verify_guard(&guard)?;
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
        let guard = evm_chain_guard(&prepared.network_id, prepared.expected_chain_id)?;
        let mut receipts = Vec::with_capacity(submissions.len());
        for transaction_hash in transaction_hashes {
            let response = self
                .mutation
                .receipt
                .read_receipt(&EvmReceiptReadRequest {
                    guard: guard.clone(),
                    transaction_hash,
                })
                .await?;
            let response = verify_receipt_response(&guard, transaction_hash, response)?;
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
        let guard = evm_chain_guard(&prepared.network_id, prepared.expected_chain_id)?;
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
                .read_receipt(&EvmReceiptReadRequest {
                    guard: guard.clone(),
                    transaction_hash,
                })
                .await
            {
                Ok(response) => {
                    verify_receipt_response(&guard, transaction_hash, response)?;
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
            .read_nonce(&EvmNonceReadRequest {
                guard: guard.clone(),
                account: expected_signer,
                block: EvmBlockSelector::Pending,
            })
            .await
        {
            Ok(response) => {
                response.evidence.verify_guard(&guard)?;
                response.nonce
            }
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
                .read_nonce_occupancy(&EvmNonceOccupancyReadRequest {
                    guard: guard.clone(),
                    account: expected_signer,
                    nonce: transaction.nonce,
                    excluded_transaction_hash: parse_prepared_transaction_hash(
                        &transaction.expected_transaction_hash,
                    )?,
                })
                .await
            {
                Ok(response) => {
                    response.evidence.verify_guard(&guard)?;
                    match response.outcome {
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
                    }
                }
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

    /// Executes context-bound validation reads against certified context authority.
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
        let verified_chain =
            verified_chain_identity(self.mutation.chain_identity, network_id, expected_chain_id)
                .await?;
        let guard = verified_chain.guard;

        let nonce_response = self
            .mutation
            .nonce
            .read_nonce(&EvmNonceReadRequest {
                guard: guard.clone(),
                account: expected_signer,
                block: EvmBlockSelector::Latest,
            })
            .await?;
        nonce_response.evidence.verify_guard(&guard)?;
        let nonce = nonce_response.nonce;
        let fees = self
            .mutation
            .fee
            .read_fee(&EvmFeeReadRequest {
                guard: guard.clone(),
            })
            .await?;
        fees.evidence.verify_guard(&guard)?;

        let mut evidence = Vec::with_capacity(tx_inputs.len());
        let mut signing_requests = Vec::with_capacity(tx_inputs.len());
        for (index, input) in tx_inputs.into_iter().enumerate() {
            let gas_limit = match policy.gas_limit() {
                Some(gas_limit) => gas_limit,
                None => {
                    let gas = self
                        .mutation
                        .gas
                        .estimate_gas(&EvmGasEstimateRequest {
                            guard: guard.clone(),
                            from: Some(expected_signer),
                            to: input.to,
                            value_wei: input.value_wei,
                            data: input.data.clone(),
                        })
                        .await?;
                    gas.evidence.verify_guard(&guard)?;
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

async fn validate_context_contract_with_reads(
    reads: EvmContractReadProviders<'_>,
    action: &ValidatedConfig<ValidateAction>,
    input: &ContextValidateContractInput,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifact: Option<&ContractArtifactConfig>,
    request: &ContextContractValidationReadRequest,
) -> Result<ContractValidationReadResponse> {
    let expected =
        ContextBoundValidateContractState::new(action.clone())?.read_request(input, context)?;
    if &expected != request {
        return Err(EvmContractAdapterError::IntentMismatch);
    }
    ensure_configured_input_context(input, context)?;

    let chain = verified_chain_identity(
        reads.chain_identity,
        context.value().network.network_id.as_str(),
        context.value().network.expected_chain_id(),
    )
    .await?;

    let assertion_context = prepare_validation_assertion_context(
        artifact,
        input.configured.address.as_str(),
        validation_assertions_required(&request.read_assertions, &request.event_assertions),
    )?;
    let evaluated = evaluate_assertions(
        reads,
        assertion_context.as_ref(),
        &chain.guard,
        &request.read_assertions,
        &request.event_assertions,
    )
    .await?;
    let (configuration_read_results, configuration_event_results) = input
        .configured
        .asserted_configuration_snapshot
        .as_ref()
        .map(|snapshot| {
            (
                snapshot.read_results.clone(),
                snapshot.event_results.clone(),
            )
        })
        .unwrap_or_default();

    Ok(ContractValidationReadResponse {
        response_version: 1,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        configured_input_digest: request.configured_input_digest.clone(),
        evm_network_context_ref: evm_network_context_ref(&context.value().network)?,
        resource_stage: ContractLifecycleStage::Configured,
        observed_chain_id: chain.response.chain_id,
        client_version: chain
            .response
            .client_version
            .unwrap_or_else(|| "unknown".to_owned()),
        configuration_read_results,
        configuration_event_results,
        read_results: evaluated.read_results,
        event_results: evaluated.event_results,
        validation_read_evidence: evaluated.read_evidence,
        validation_event_evidence: evaluated.event_evidence,
        evidence_refs: input.configured.configure_or_import_evidence.clone(),
    })
}

async fn import_deployed_with_reads(
    reads: EvmContractReadProviders<'_>,
    import: &ImportDeployedSpec,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    source_run_registry: Option<&mfm_certify::CertificationRegistry>,
) -> Result<DeployedContractInstance> {
    match import {
        ImportDeployedSpec::FromMfmRun { source, evidence } => {
            validate_source_run_import_policy(source, context, ContractLifecycleStage::Deployed)?;
            let imported = import_source_run_value::<DeployedContractInstance>(
                artifacts,
                source,
                evidence,
                context,
                ContractLifecycleStage::Deployed,
                source_run_registry,
            )
            .await?;
            ImportDeployedContractState::admit_verified_mfm_run_import(import, imported, context)
                .map_err(Into::into)
        }
        ImportDeployedSpec::AdoptExternalAddress { adoption } => {
            let verified = verify_external_adoption(
                reads,
                adoption,
                context,
                ContractLifecycleStage::Deployed,
            )
            .await?;
            ImportDeployedContractState::admit_verified_external_adoption(
                import,
                context,
                verified.evidence,
                None,
            )
            .map_err(Into::into)
        }
    }
}

async fn import_configured_with_reads(
    reads: EvmContractReadProviders<'_>,
    import: &ImportConfiguredSpec,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifact: Option<&ContractArtifactConfig>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    source_run_registry: Option<&mfm_certify::CertificationRegistry>,
) -> Result<ConfiguredContractInstance> {
    match import {
        ImportConfiguredSpec::FromMfmRun { source, evidence } => {
            validate_source_run_import_policy(source, context, ContractLifecycleStage::Configured)?;
            let imported = import_source_run_value::<ConfiguredContractInstance>(
                artifacts,
                source,
                evidence,
                context,
                ContractLifecycleStage::Configured,
                source_run_registry,
            )
            .await?;
            ImportConfiguredContractState::admit_verified_mfm_run_import(import, imported, context)
                .map_err(Into::into)
        }
        ImportConfiguredSpec::AdoptExternalAddress { adoption } => {
            let mut verified = verify_external_adoption(
                reads,
                adoption,
                context,
                ContractLifecycleStage::Configured,
            )
            .await?;
            let snapshot = if validation_assertions_required(
                &adoption.evidence_policy.initial_read_assertions,
                &adoption.evidence_policy.initial_event_assertions,
            ) {
                let assertion_context = prepare_validation_assertion_context(
                    artifact,
                    adoption.address.as_str(),
                    true,
                )?;
                let evaluated = evaluate_assertions(
                    reads,
                    assertion_context.as_ref(),
                    &verified.guard,
                    &adoption.evidence_policy.initial_read_assertions,
                    &adoption.evidence_policy.initial_event_assertions,
                )
                .await?;
                let all_passed = evaluated.read_results.iter().all(|result| result.passed)
                    && evaluated.event_results.iter().all(|result| result.passed);
                if !all_passed {
                    return Err(EvmContractAdapterError::ExternalAdoptionAssertionsFailed);
                }
                verified.evidence.read_assertion_evidence = evaluated.read_evidence;
                verified.evidence.event_assertion_evidence = evaluated.event_evidence;
                Some(ConfigurationSnapshot {
                    read_results: evaluated.read_results,
                    event_results: evaluated.event_results,
                })
            } else if adoption.evidence_policy.allow_external_claimed_configured {
                None
            } else {
                return Err(EvmContractAdapterError::ExternalConfiguredClaimNotAllowed);
            };
            ImportConfiguredContractState::admit_verified_external_adoption(
                import,
                context,
                verified.evidence,
                snapshot,
                None,
            )
            .map_err(Into::into)
        }
    }
}

struct VerifiedExternalAdoption {
    guard: EvmChainGuard,
    evidence: ExternalAdoptionEvidence,
}

async fn verify_external_adoption(
    reads: EvmContractReadProviders<'_>,
    adoption: &mfm_evm_contract_model::AdoptExternalAddress,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    resource_stage: ContractLifecycleStage,
) -> Result<VerifiedExternalAdoption> {
    let chain = verified_chain_identity(
        reads.chain_identity,
        context.value().network.network_id.as_str(),
        context.value().network.expected_chain_id(),
    )
    .await?;
    let guard = chain.guard;
    let mut code_read_evidence = None;
    if adoption.evidence_policy.require_code
        || adoption.evidence_policy.expected_code_hash.is_some()
    {
        let address = parse_address(adoption.address.as_str(), "address")
            .map_err(|error| EvmContractAdapterError::Model(error.message))?;
        let block =
            adoption
                .evidence_policy
                .block_anchor
                .clone()
                .unwrap_or(ModelBlockSelector::Tag {
                    tag: BlockTag::Latest,
                });
        let response = reads
            .code
            .read_code(&EvmCodeReadRequest {
                guard: guard.clone(),
                address,
                block: block_selector(Some(&block), true)?,
            })
            .await?;
        response.evidence.verify_guard(&guard)?;
        if adoption.evidence_policy.require_code && response.code.is_empty() {
            return Err(EvmContractAdapterError::ExternalCodeMissing);
        }
        let observed = EvmCodeHash::new(format!("{:?}", response.code_hash))
            .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
        if let Some(expected) = &adoption.evidence_policy.expected_code_hash {
            if &observed != expected {
                return Err(EvmContractAdapterError::ExternalCodeHashMismatch);
            }
        }
        code_read_evidence = Some(ExternalCodeReadEvidence {
            address: adoption.address.clone(),
            block,
            source: external_evm_source_evidence(&response.evidence),
            observed_code_hash: observed,
            observed_code_byte_len: response.code.len() as u64,
        });
    }
    let evidence = ExternalAdoptionEvidence {
        evidence_policy_digest: digest_for_value(&adoption.evidence_policy)?,
        context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
        evm_network_context_ref: evm_network_context_ref(&context.value().network)?,
        resource_stage,
        observed_chain_id: chain.response.chain_id,
        code_read_evidence,
        read_assertion_evidence: Vec::new(),
        event_assertion_evidence: Vec::new(),
    };
    Ok(VerifiedExternalAdoption { guard, evidence })
}

fn external_evm_source_evidence(evidence: &RedactedEvmSourceEvidence) -> ExternalEvmSourceEvidence {
    ExternalEvmSourceEvidence {
        network_id: evidence.network_id.to_string(),
        expected_chain_id: evidence.expected_chain_id,
        observed_chain_id: evidence.observed_chain_id,
        source_ref: evidence.source_ref.to_string(),
        policy_id: evidence.policy_id.to_string(),
    }
}

fn import_configured_requires_artifact(import: &ImportConfiguredSpec) -> bool {
    match import {
        ImportConfiguredSpec::FromMfmRun { .. } => false,
        ImportConfiguredSpec::AdoptExternalAddress { adoption } => validation_assertions_required(
            &adoption.evidence_policy.initial_read_assertions,
            &adoption.evidence_policy.initial_event_assertions,
        ),
    }
}

async fn import_source_run_value<T>(
    artifacts: &dyn store::RetainedArtifactReadProvider,
    source: &ImportFromMfmRun,
    evidence: &ImportFromMfmRunEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
    source_run_registry: Option<&mfm_certify::CertificationRegistry>,
) -> Result<T>
where
    T: ContextBoundOutput + DeserializeOwned,
{
    let source_run_registry = source_run_registry.ok_or_else(|| {
        EvmContractAdapterError::SourceRunImportEvidence(
            "source-run import requires trusted certification registry authority".to_owned(),
        )
    })?;
    validate_source_run_import_evidence::<T>(source, evidence, context, required_stage)?;
    let source_spec =
        read_lifecycle_evidence_artifact(artifacts, &evidence.source_spec_artifact_ref).await?;
    let certificate =
        read_lifecycle_evidence_artifact(artifacts, &evidence.source_spec_certificate_ref).await?;
    let stream =
        read_lifecycle_evidence_artifact(artifacts, &evidence.source_run_stream_ref).await?;
    let source_value = read_lifecycle_evidence_artifact(
        artifacts,
        &evidence.source_value_artifact_ref_or_inline_canonical_value,
    )
    .await?;
    let value = serde_json::from_slice::<T>(source_value.bytes())
        .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))?;
    if value.context_ref() != evidence.source_context_ref.as_context_ref()
        || value.context_resource_kind()
            != mfm_evm_contract_model::contract_instance_resource_kind()
        || value.context_stage() != context_stage_for_lifecycle_stage(required_stage)
        || digest_for_value(&value)? != evidence.source_value_digest
    {
        return Err(EvmContractAdapterError::ContextMismatch);
    }
    let committed = decode_source_run_committed_stream(
        source,
        stream.bytes(),
        source_value.evidence(),
        source_value.bytes(),
    )?;
    validate_source_run_authority(SourceRunAuthorityEvidence {
        source,
        evidence,
        source_spec_bytes: source_spec.bytes(),
        certificate_bytes: certificate.bytes(),
        committed: &committed,
        source_value_artifact: source_value.evidence(),
        required_stage,
        source_run_registry,
    })?;
    Ok(value)
}

fn validate_source_run_import_evidence<T>(
    source: &ImportFromMfmRun,
    evidence: &ImportFromMfmRunEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> Result<()>
where
    T: ContextBoundOutput,
{
    if evidence.source_spec_hash != source.source_spec_hash
        || evidence.source_cell_or_output_id != source.source_cell_or_output_id
        || evidence.source_stage != required_stage
        || evidence.source_context_ref != source.source_context_ref
        || evidence.source_value_digest != source.source_value_digest
        || evidence.import_policy_digest != digest_for_value(source)?
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run import evidence does not match certified import policy".to_owned(),
        ));
    }
    if evidence.source_context_ref.as_context_ref() == context.context_ref()
        && evidence
            .source_context_descriptor_id
            .typed()
            .map_err(EvmContractAdapterError::Model)?
            != *context.context_descriptor_id()
    {
        return Err(EvmContractAdapterError::ContextMismatch);
    }
    let value_ref = &evidence.source_value_artifact_ref_or_inline_canonical_value;
    if value_ref
        .content_digest()
        .map_err(EvmContractAdapterError::Model)?
        != evidence
            .source_value_digest
            .typed()
            .map_err(EvmContractAdapterError::Model)?
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source value artifact digest does not match import evidence".to_owned(),
        ));
    }
    if evidence
        .source_cell_schema_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?
        != T::schema_id().map_err(|error| EvmContractAdapterError::Model(error.to_string()))?
        || evidence
            .source_cell_semantic_type_id
            .typed()
            .map_err(EvmContractAdapterError::Model)?
            != T::semantic_id()
                .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run import evidence value type does not match requested lifecycle stage"
                .to_owned(),
        ));
    }
    Ok(())
}

struct SourceRunAuthorityEvidence<'a> {
    source: &'a ImportFromMfmRun,
    evidence: &'a ImportFromMfmRunEvidence,
    source_spec_bytes: &'a [u8],
    certificate_bytes: &'a [u8],
    committed: &'a store::CommittedRunStream,
    source_value_artifact: &'a store::ArtifactEvidenceRef,
    required_stage: ContractLifecycleStage,
    source_run_registry: &'a mfm_certify::CertificationRegistry,
}

fn validate_source_run_authority(authority: SourceRunAuthorityEvidence<'_>) -> Result<()> {
    let SourceRunAuthorityEvidence {
        source,
        evidence,
        source_spec_bytes,
        certificate_bytes,
        committed,
        source_value_artifact,
        required_stage,
        source_run_registry,
    } = authority;
    let certified_source_spec =
        mfm_certify::verify_persisted_spec_certificate_with_trusted_registry(
            source_spec_bytes,
            certificate_bytes,
            source_run_registry,
        )
        .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))?;
    let run_admitted = source_run_admitted(committed)?;
    let source_run_id = source
        .source_run_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    let source_spec_hash = source
        .source_spec_hash
        .typed()
        .map_err(EvmContractAdapterError::Model)?;

    if committed.run_id() != &source_run_id
        || run_admitted.run_id != source_run_id
        || run_admitted.spec_hash != source_spec_hash
        || certified_source_spec.spec_hash() != &source_spec_hash
        || !lifecycle_ref_matches_run_artifact(
            &evidence.source_spec_artifact_ref,
            &run_admitted.spec_artifact,
        )?
        || !lifecycle_ref_matches_run_artifact(
            &evidence.source_spec_certificate_ref,
            &run_admitted.certificate_artifact,
        )?
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream does not match trusted source spec authority".to_owned(),
        ));
    }

    let terminal = source_run_terminal_event_from_committed_stream(
        committed,
        evidence,
        source_value_artifact,
        &certified_source_spec,
        required_stage,
    )?;

    verify_source_terminal_event_against_certified_spec(
        &certified_source_spec,
        &terminal,
        required_stage,
    )?;

    Ok(())
}

fn decode_source_run_committed_stream(
    source: &ImportFromMfmRun,
    stream_bytes: &[u8],
    source_value_artifact: &store::ArtifactEvidenceRef,
    source_value_bytes: &[u8],
) -> Result<store::CommittedRunStream> {
    let source_run_id = source
        .source_run_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    let mut artifact_bytes = store::ArtifactByteAuthorityMap::new();
    artifact_bytes.insert(
        (
            source_value_artifact.artifact_id.clone(),
            source_value_artifact.evidence_hash().map_err(|error| {
                EvmContractAdapterError::SourceRunImportEvidence(error.to_string())
            })?,
        ),
        (source_value_bytes.to_vec(), source_value_artifact.clone()),
    );
    store::committed_run_stream_from_canonical_json_slice(
        &source_run_id,
        stream_bytes,
        &artifact_bytes,
    )
    .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))
}

fn source_run_admitted(committed: &store::CommittedRunStream) -> Result<&events::RunAdmitted> {
    let mut admitted = committed.events().iter().filter_map(|event| {
        if let events::KernelEventPayload::RunAdmitted(payload) = event.payload() {
            Some(payload.as_ref())
        } else {
            None
        }
    });
    let Some(first) = admitted.next() else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream is missing RunAdmitted".to_owned(),
        ));
    };
    if admitted.next().is_some() {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream contains multiple RunAdmitted events".to_owned(),
        ));
    }
    Ok(first)
}

fn lifecycle_ref_matches_run_artifact(
    evidence: &LifecycleArtifactEvidenceRef,
    artifact: &events::RunArtifactEvidenceRef,
) -> Result<bool> {
    Ok(evidence
        .artifact_id()
        .map_err(EvmContractAdapterError::Model)?
        == artifact.artifact_id
        && evidence
            .content_digest()
            .map_err(EvmContractAdapterError::Model)?
            == artifact.content_digest
        && evidence.byte_len() == artifact.byte_len
        && evidence
            .schema_id()
            .map_err(EvmContractAdapterError::Model)?
            == artifact.schema_id
        && evidence
            .semantic_type_id()
            .map_err(EvmContractAdapterError::Model)?
            == artifact.semantic_type_id)
}

fn source_run_terminal_event_from_committed_stream(
    committed: &store::CommittedRunStream,
    evidence: &ImportFromMfmRunEvidence,
    source_value_artifact: &store::ArtifactEvidenceRef,
    certified: &mfm_certify::CertifiedTypedSpec,
    required_stage: ContractLifecycleStage,
) -> Result<SourceRunTerminalEvent> {
    let event_id = evidence
        .source_terminal_cell_or_output_event_ref
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    let event = committed
        .events()
        .iter()
        .find(|event| event.event_id() == &event_id)
        .ok_or_else(|| {
            EvmContractAdapterError::SourceRunImportEvidence(
                "source-run committed stream does not contain claimed terminal event".to_owned(),
            )
        })?;

    let terminal = match (event.payload(), &evidence.source_cell_or_output_id) {
        (
            events::KernelEventPayload::CellProduced(payload),
            SourceCellOrOutputRef::Cell { cell_id: _ },
        ) => source_run_terminal_event_from_cell(
            event.event_id(),
            payload,
            evidence,
            source_value_artifact,
            required_stage,
        )?,
        (
            events::KernelEventPayload::PublicOutputProduced(payload),
            SourceCellOrOutputRef::PublicOutput { output_key },
        ) => source_run_terminal_event_from_public_output(
            event.event_id(),
            payload,
            output_key,
            evidence,
            source_value_artifact,
            certified,
            required_stage,
        )?,
        _ => {
            return Err(EvmContractAdapterError::SourceRunImportEvidence(
                "source-run committed stream terminal event kind does not match import evidence"
                    .to_owned(),
            ));
        }
    };
    if source_run_terminal_event_matches(&terminal, evidence, required_stage) {
        Ok(terminal)
    } else {
        Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run committed stream terminal event does not match import evidence".to_owned(),
        ))
    }
}

fn source_run_terminal_event_from_cell(
    event_id: &EventId,
    payload: &events::CellProduced,
    evidence: &ImportFromMfmRunEvidence,
    source_value_artifact: &store::ArtifactEvidenceRef,
    required_stage: ContractLifecycleStage,
) -> Result<SourceRunTerminalEvent> {
    if payload.artifact_id != source_value_artifact.artifact_id
        || payload.content_digest != source_value_artifact.digest
        || Some(&payload.schema_id) != source_value_artifact.schema_id.as_ref()
        || Some(&payload.semantic_type_id) != source_value_artifact.semantic_type_id.as_ref()
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell artifact does not match retained source value".to_owned(),
        ));
    }
    let spec::CellContextSpec::Bound { context_ref, .. } = &payload.context else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell is not context-bound".to_owned(),
        ));
    };
    Ok(SourceRunTerminalEvent {
        source_terminal_cell_or_output_event_ref: mfm_evm_contract_model::LifecycleEventIdRef::from(
            event_id.clone(),
        ),
        source_cell_or_output_id: SourceCellOrOutputRef::Cell {
            cell_id: mfm_evm_contract_model::LifecycleCellIdRef::from(payload.cell_id.clone()),
        },
        source_cell_schema_id: mfm_evm_contract_model::ArtifactEvidenceSchemaId::from(
            payload.schema_id.clone(),
        ),
        source_cell_semantic_type_id: mfm_evm_contract_model::ArtifactEvidenceSemanticTypeId::from(
            payload.semantic_type_id.clone(),
        ),
        source_producer_descriptor_id: evidence.source_producer_descriptor_id.clone(),
        source_stage: required_stage,
        source_context_ref: mfm_values::ContextRefValue::from(context_ref.clone()),
        source_context_descriptor_id: evidence.source_context_descriptor_id.clone(),
        source_value_digest: ContractProfileDigestRef::from(payload.content_digest.clone()),
    })
}

fn source_run_terminal_event_from_public_output(
    event_id: &EventId,
    payload: &events::PublicOutputProduced,
    output_key: &mfm_evm_contract_model::ArtifactPort,
    evidence: &ImportFromMfmRunEvidence,
    source_value_artifact: &store::ArtifactEvidenceRef,
    certified: &mfm_certify::CertifiedTypedSpec,
    required_stage: ContractLifecycleStage,
) -> Result<SourceRunTerminalEvent> {
    let cell = payload
        .cells
        .iter()
        .find(|cell| cell.public_field_path.as_str() == output_key.as_str())
        .ok_or_else(|| {
            EvmContractAdapterError::SourceRunImportEvidence(
                "source-run public output event does not contain claimed output key".to_owned(),
            )
        })?;
    if cell.artifact_id != source_value_artifact.artifact_id
        || cell.content_digest != source_value_artifact.digest
        || Some(&cell.schema_id) != source_value_artifact.schema_id.as_ref()
        || Some(&cell.semantic_type_id) != source_value_artifact.semantic_type_id.as_ref()
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run public output artifact does not match retained source value".to_owned(),
        ));
    }
    let certified_cell = certified
        .validated_spec()
        .graph()
        .cell(&cell.cell_id)
        .ok_or_else(|| {
            EvmContractAdapterError::SourceRunImportEvidence(
                "source-run public output cell is not declared by source spec".to_owned(),
            )
        })?;
    let spec::CellContextSpec::Bound { context_ref, .. } = &certified_cell.context else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run public output cell is not context-bound".to_owned(),
        ));
    };
    Ok(SourceRunTerminalEvent {
        source_terminal_cell_or_output_event_ref: mfm_evm_contract_model::LifecycleEventIdRef::from(
            event_id.clone(),
        ),
        source_cell_or_output_id: SourceCellOrOutputRef::PublicOutput {
            output_key: output_key.clone(),
        },
        source_cell_schema_id: mfm_evm_contract_model::ArtifactEvidenceSchemaId::from(
            cell.schema_id.clone(),
        ),
        source_cell_semantic_type_id: mfm_evm_contract_model::ArtifactEvidenceSemanticTypeId::from(
            cell.semantic_type_id.clone(),
        ),
        source_producer_descriptor_id: evidence.source_producer_descriptor_id.clone(),
        source_stage: required_stage,
        source_context_ref: mfm_values::ContextRefValue::from(context_ref.clone()),
        source_context_descriptor_id: evidence.source_context_descriptor_id.clone(),
        source_value_digest: ContractProfileDigestRef::from(cell.content_digest.clone()),
    })
}

fn verify_source_terminal_event_against_certified_spec(
    certified: &mfm_certify::CertifiedTypedSpec,
    terminal: &SourceRunTerminalEvent,
    required_stage: ContractLifecycleStage,
) -> Result<()> {
    let cell_id = match &terminal.source_cell_or_output_id {
        SourceCellOrOutputRef::Cell { cell_id } => {
            cell_id.typed().map_err(EvmContractAdapterError::Model)?
        }
        SourceCellOrOutputRef::PublicOutput { output_key } => certified
            .envelope()
            .spec
            .public_outputs
            .outputs
            .iter()
            .find(|output| output.public_field_path.as_str() == output_key.as_str())
            .map(|output| output.cell_id.clone())
            .ok_or_else(|| {
                EvmContractAdapterError::SourceRunImportEvidence(
                    "source-run public output is not declared by the certified source spec"
                        .to_owned(),
                )
            })?,
    };
    let graph = certified.validated_spec().graph();
    let cell = graph.cell(&cell_id).ok_or_else(|| {
        EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell is not declared by the certified source spec".to_owned(),
        )
    })?;

    if terminal
        .source_cell_schema_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?
        != cell.schema_id
        || terminal
            .source_cell_semantic_type_id
            .typed()
            .map_err(EvmContractAdapterError::Model)?
            != cell.semantic_type_id
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal type evidence does not match certified source cell".to_owned(),
        ));
    }

    let spec::CellContextSpec::Bound {
        context_ref,
        resource_kind,
        stage,
        producer,
    } = &cell.context
    else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell is not context-bound".to_owned(),
        ));
    };
    if context_ref != terminal.source_context_ref.as_context_ref()
        || resource_kind != mfm_evm_contract_model::contract_instance_resource_kind()
        || stage != context_stage_for_lifecycle_stage(required_stage)
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal context does not match certified source cell".to_owned(),
        ));
    }
    let source_context_descriptor_id = terminal
        .source_context_descriptor_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    if !certified.envelope().spec.contexts.iter().any(|context| {
        &context.context_ref == context_ref
            && context.context_descriptor_id == source_context_descriptor_id
    }) {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal context descriptor is not certified by source spec".to_owned(),
        ));
    }

    let spec::CellProducer::Node(node_id) = &cell.producer else {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run terminal cell was not produced by a certified lifecycle node".to_owned(),
        ));
    };
    let node = graph.forward_node(node_id).ok_or_else(|| {
        EvmContractAdapterError::SourceRunImportEvidence(
            "source-run producer node is not certified by source spec".to_owned(),
        )
    })?;
    let source_producer_descriptor_id = terminal
        .source_producer_descriptor_id
        .typed()
        .map_err(EvmContractAdapterError::Model)?;
    if node.output_cell != cell.cell_id
        || !producer
            .producer_descriptor_ids
            .iter()
            .any(|descriptor_id| descriptor_id == &source_producer_descriptor_id)
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run producer descriptor is not certified for the terminal cell".to_owned(),
        ));
    }
    if node.context
        != (spec::NodeContextSpec::Required {
            context_ref: context_ref.clone(),
        })
    {
        return Err(EvmContractAdapterError::SourceRunImportEvidence(
            "source-run producer node context does not match terminal cell context".to_owned(),
        ));
    }
    Ok(())
}

fn source_run_terminal_event_matches(
    event: &SourceRunTerminalEvent,
    evidence: &ImportFromMfmRunEvidence,
    required_stage: ContractLifecycleStage,
) -> bool {
    event.source_terminal_cell_or_output_event_ref
        == evidence.source_terminal_cell_or_output_event_ref
        && event.source_cell_or_output_id == evidence.source_cell_or_output_id
        && event.source_cell_schema_id == evidence.source_cell_schema_id
        && event.source_cell_semantic_type_id == evidence.source_cell_semantic_type_id
        && event.source_producer_descriptor_id == evidence.source_producer_descriptor_id
        && event.source_stage == required_stage
        && event.source_stage == evidence.source_stage
        && event.source_context_ref == evidence.source_context_ref
        && event.source_context_descriptor_id == evidence.source_context_descriptor_id
        && event.source_value_digest == evidence.source_value_digest
}

async fn read_lifecycle_evidence_artifact(
    artifacts: &dyn store::RetainedArtifactReadProvider,
    evidence: &LifecycleArtifactEvidenceRef,
) -> Result<store::VerifiedRunArtifactBytes> {
    let requirement = lifecycle_artifact_requirement(evidence)?;
    artifacts
        .read_retained_artifact(&requirement)
        .await
        .map_err(|error| EvmContractAdapterError::SourceRunImportEvidence(error.to_string()))
}

fn lifecycle_artifact_requirement(
    evidence: &LifecycleArtifactEvidenceRef,
) -> Result<events::EventArtifactRequirement> {
    Ok(events::EventArtifactRequirement {
        source: events::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: evidence
            .artifact_id()
            .map_err(EvmContractAdapterError::Model)?,
        digest: Some(
            evidence
                .content_digest()
                .map_err(EvmContractAdapterError::Model)?,
        ),
        byte_len: Some(evidence.byte_len()),
        media_type: None,
        schema_id: evidence
            .schema_id()
            .map_err(EvmContractAdapterError::Model)?,
        semantic_type_id: evidence
            .semantic_type_id()
            .map_err(EvmContractAdapterError::Model)?,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: None,
    })
}

fn run_artifact_requirement(
    source: events::EventArtifactReferenceSource,
    artifact: &events::RunArtifactEvidenceRef,
) -> events::EventArtifactRequirement {
    events::EventArtifactRequirement {
        source,
        artifact_id: artifact.artifact_id.clone(),
        digest: Some(artifact.content_digest.clone()),
        byte_len: Some(artifact.byte_len),
        media_type: Some(artifact.media_type.clone()),
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(artifact.role),
    }
}

fn context_stage_for_lifecycle_stage(
    stage: ContractLifecycleStage,
) -> &'static mfm_ids::ContextStage {
    match stage {
        ContractLifecycleStage::Deployed => deployed_contract_stage(),
        ContractLifecycleStage::Configured => configured_contract_stage(),
    }
}

fn validate_source_run_import_policy(
    source: &ImportFromMfmRun,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> Result<()> {
    if source.required_stage != required_stage {
        return Err(EvmContractAdapterError::ImportStageMismatch);
    }
    match &source.accepted_context_policy {
        AcceptedContextPolicy::ExactContext {} => {
            if source.source_context_ref.as_context_ref() != context.context_ref() {
                return Err(EvmContractAdapterError::ContextMismatch);
            }
        }
        AcceptedContextPolicy::AcceptedContextRefs { context_refs } => {
            if !context_refs
                .iter()
                .any(|context_ref| context_ref == &source.source_context_ref)
            {
                return Err(EvmContractAdapterError::ContextMismatch);
            }
        }
    }
    Ok(())
}

fn digest_for_value<T>(value: &T) -> Result<ContractProfileDigestRef>
where
    T: Serialize,
{
    let json = serde_json::to_string(value)
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| ContractProfileDigestRef::from(canonical.content_digest()))
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))
}

struct VerifiedChainIdentity {
    guard: EvmChainGuard,
    response: EvmChainIdentityResponse,
}

async fn verified_chain_identity(
    provider: &dyn EvmChainIdentityProvider,
    network_id: &str,
    expected_chain_id: u64,
) -> Result<VerifiedChainIdentity> {
    let guard = evm_chain_guard(network_id, expected_chain_id)?;
    let chain = provider
        .chain_identity(&EvmChainIdentityRequest {
            guard: guard.clone(),
        })
        .await?;
    chain.evidence.verify_guard(&guard)?;
    if chain.chain_id != expected_chain_id {
        return Err(EvmCapabilityError::ChainMismatch {
            evidence: chain.evidence.with_observed_chain_id(chain.chain_id),
        }
        .into());
    }
    Ok(VerifiedChainIdentity {
        guard,
        response: chain,
    })
}

struct ValidationAssertionContext {
    abi: ParsedAbi,
    address: Address,
}

struct EvaluatedAssertions {
    read_results: Vec<ValidationReadResult>,
    event_results: Vec<ValidationEventResult>,
    read_evidence: Vec<ExternalReadAssertionEvidence>,
    event_evidence: Vec<ExternalEventAssertionEvidence>,
}

fn prepare_validation_assertion_context(
    artifact: Option<&ContractArtifactConfig>,
    contract_address: &str,
    required: bool,
) -> Result<Option<ValidationAssertionContext>> {
    if !required {
        return Ok(None);
    }
    let artifact = artifact.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (abi, _) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    let address = parse_address(contract_address, "contract_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    Ok(Some(ValidationAssertionContext { abi, address }))
}

fn validation_assertions_required(
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> bool {
    !read_assertions.is_empty() || !event_assertions.is_empty()
}

async fn evaluate_assertions(
    reads: EvmContractReadProviders<'_>,
    context: Option<&ValidationAssertionContext>,
    guard: &EvmChainGuard,
    read_assertions: &[ReadAssertionConfig],
    event_assertions: &[EventAssertionConfig],
) -> Result<EvaluatedAssertions> {
    if !validation_assertions_required(read_assertions, event_assertions) {
        return Ok(EvaluatedAssertions {
            read_results: Vec::new(),
            event_results: Vec::new(),
            read_evidence: Vec::new(),
            event_evidence: Vec::new(),
        });
    }
    let context = context.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (prepared_reads, prepared_events) =
        prepare_validate_assertions(&context.abi, read_assertions, event_assertions)
            .map_err(EvmContractAdapterError::Model)?;

    let mut read_results = Vec::with_capacity(prepared_reads.len());
    let mut read_evidence = Vec::with_capacity(prepared_reads.len());
    for (assertion, prepared) in read_assertions.iter().zip(prepared_reads.iter()) {
        let response = reads
            .call
            .read_call(&EvmCallReadRequest {
                guard: guard.clone(),
                to: context.address,
                calldata: hex_to_bytes(&prepared.data_hex)
                    .map_err(|error| EvmContractAdapterError::Model(error.message))?,
                block: EvmBlockSelector::Latest,
            })
            .await?;
        response.evidence.verify_guard(guard)?;
        let actual_json = decode_single_output_to_json(
            &prepared.outputs,
            &bytes_to_hex_prefixed(&response.return_data),
        )
        .map_err(EvmContractAdapterError::Model)?;
        let actual =
            ExpectedValue::from_json_value(&actual_json).map_err(EvmContractAdapterError::Model)?;
        let result = ValidationReadResult {
            function: assertion.function.to_string(),
            args: assertion.args.clone(),
            expected: assertion.expected.clone(),
            passed: expected_matches(&actual, &assertion.expected),
            actual,
        };
        read_evidence.push(ExternalReadAssertionEvidence {
            source: external_evm_source_evidence(&response.evidence),
            result: result.clone(),
        });
        read_results.push(result);
    }

    let mut event_results = Vec::with_capacity(prepared_events.len());
    let mut event_evidence = Vec::with_capacity(prepared_events.len());
    for (assertion, prepared) in event_assertions.iter().zip(prepared_events.iter()) {
        let topic = prepared
            .topic0_hex
            .parse::<B256>()
            .map_err(|_| EvmContractAdapterError::Model("invalid event topic".to_owned()))?;
        let logs = reads
            .logs
            .read_logs(&EvmLogsReadRequest {
                guard: guard.clone(),
                from_block: block_selector(assertion.from_block.as_ref(), false)?,
                to_block: block_selector(assertion.to_block.as_ref(), true)?,
                address: Some(context.address),
                topics: vec![topic],
            })
            .await?;
        logs.evidence.verify_guard(guard)?;
        let observed_count = logs.logs.len() as u64;
        let result = ValidationEventResult {
            event: prepared.event.clone(),
            min_count: prepared.min_count,
            observed_count,
            passed: observed_count >= prepared.min_count,
        };
        event_evidence.push(ExternalEventAssertionEvidence {
            source: external_evm_source_evidence(&logs.evidence),
            result: result.clone(),
        });
        event_results.push(result);
    }

    Ok(EvaluatedAssertions {
        read_results,
        event_results,
        read_evidence,
        event_evidence,
    })
}

struct PreparedTransactionInput {
    to: Option<Address>,
    value_wei: u128,
    data: Vec<u8>,
}

struct PrepareTransactionsRequest<'a> {
    phase: ContractMutationPhase,
    context_ref: mfm_values::ContextRefValue,
    evm_network_context_ref: String,
    resource_stage: ContractLifecycleStage,
    network_id: &'a str,
    expected_chain_id: u64,
    signer_ref: SignerRef,
    expected_signer: Address,
    expected_signer_text: &'a str,
    policy: &'a EvmTransactionPolicy,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
    tx_inputs: Vec<PreparedTransactionInput>,
}

trait ContractMutationActionView {
    fn signer(&self) -> &EvmSignerIntent;

    fn transaction(&self) -> &EvmTransactionPolicy;

    fn receipt(&self) -> &ReceiptRetryPolicy;
}

impl ContractMutationActionView for DeployAction {
    fn signer(&self) -> &EvmSignerIntent {
        self.signer()
    }

    fn transaction(&self) -> &EvmTransactionPolicy {
        self.transaction()
    }

    fn receipt(&self) -> &ReceiptRetryPolicy {
        self.receipt()
    }
}

impl ContractMutationActionView for ConfigureAction {
    fn signer(&self) -> &EvmSignerIntent {
        self.signer()
    }

    fn transaction(&self) -> &EvmTransactionPolicy {
        self.transaction()
    }

    fn receipt(&self) -> &ReceiptRetryPolicy {
        self.receipt()
    }
}

#[derive(Clone)]
struct ContractMutationNetworkAuthority<'a> {
    context_ref: mfm_values::ContextRefValue,
    evm_network_context_ref: String,
    resource_stage: ContractLifecycleStage,
    network_id: &'a str,
    expected_chain_id: u64,
}

impl<'a> ContractMutationNetworkAuthority<'a> {
    fn from_context(
        context: &'a mfm_program::CertifiedContext<EvmContractContext>,
        resource_stage: ContractLifecycleStage,
    ) -> Result<Self> {
        Ok(Self {
            context_ref: mfm_values::ContextRefValue::from(context.context_ref().clone()),
            evm_network_context_ref: evm_network_context_ref(&context.value().network)?,
            resource_stage,
            network_id: context.value().network.network_id.as_str(),
            expected_chain_id: context.value().network.expected_chain_id(),
        })
    }
}

struct ResolvedContractMutationAction<'a> {
    network_id: &'a str,
    expected_chain_id: u64,
    context_ref: mfm_values::ContextRefValue,
    evm_network_context_ref: String,
    resource_stage: ContractLifecycleStage,
    signer_ref: SignerRef,
    expected_signer: Address,
    expected_signer_text: &'a str,
}

impl<'a> ResolvedContractMutationAction<'a> {
    fn from_authority_and_action(
        authority: ContractMutationNetworkAuthority<'a>,
        action: &'a impl ContractMutationActionView,
    ) -> Result<Self> {
        Ok(Self {
            network_id: authority.network_id,
            expected_chain_id: authority.expected_chain_id,
            context_ref: authority.context_ref,
            evm_network_context_ref: authority.evm_network_context_ref,
            resource_stage: authority.resource_stage,
            signer_ref: action
                .signer()
                .signer_ref()
                .map_err(EvmContractAdapterError::Model)?,
            expected_signer: action
                .signer()
                .expected_signer_address()
                .map_err(EvmContractAdapterError::Model)?,
            expected_signer_text: action.signer().expected_signer_address_str(),
        })
    }
}

fn prepare_transactions_request<'a>(
    phase: ContractMutationPhase,
    authority: ContractMutationNetworkAuthority<'a>,
    action: &'a impl ContractMutationActionView,
    tx_inputs: Vec<PreparedTransactionInput>,
) -> Result<PrepareTransactionsRequest<'a>> {
    let resolved = ResolvedContractMutationAction::from_authority_and_action(authority, action)?;
    Ok(PrepareTransactionsRequest {
        phase,
        context_ref: resolved.context_ref,
        evm_network_context_ref: resolved.evm_network_context_ref,
        resource_stage: resolved.resource_stage,
        network_id: resolved.network_id,
        expected_chain_id: resolved.expected_chain_id,
        signer_ref: resolved.signer_ref,
        expected_signer: resolved.expected_signer,
        expected_signer_text: resolved.expected_signer_text,
        policy: action.transaction(),
        poll_interval_ms: action.receipt().poll_interval_ms(),
        max_receipt_polls: action.receipt().max_receipt_polls(),
        tx_inputs,
    })
}

fn prepared_mutation_reconstruction<'a>(
    evidence: &'a PreparedContractInvocation,
    phase: ContractMutationPhase,
    authority: ContractMutationNetworkAuthority<'a>,
    action: &'a impl ContractMutationActionView,
    tx_inputs: Vec<PreparedTransactionInput>,
) -> Result<PreparedMutationReconstruction<'a>> {
    let resolved = ResolvedContractMutationAction::from_authority_and_action(authority, action)?;
    Ok(PreparedMutationReconstruction {
        evidence,
        phase,
        network_id: resolved.network_id,
        expected_chain_id: resolved.expected_chain_id,
        context_ref: resolved.context_ref,
        evm_network_context_ref: resolved.evm_network_context_ref,
        resource_stage: resolved.resource_stage,
        signer_ref: resolved.signer_ref,
        expected_signer: resolved.expected_signer,
        expected_signer_text: resolved.expected_signer_text,
        tx_inputs,
    })
}

struct PreparedTransaction {
    style: PreparedContractTransactionStyle,
    request: EvmSigningRequest,
    max_fee_per_gas: Option<u128>,
    max_priority_fee_per_gas: Option<u128>,
    gas_price: Option<u128>,
}

struct EvmContractLifecycleReplayVerifier {
    verifier_id: events::ReplayVerifierId,
}

impl EvmContractLifecycleReplayVerifier {
    fn new() -> Result<Self> {
        Ok(Self {
            verifier_id: replay_verifier_id()?,
        })
    }

    fn verify_adapter_binding(
        &self,
        intent: &replay::SideEffectIntentReplayEvidence,
    ) -> replay::Result<()> {
        let binding = evm_contract_lifecycle_adapter_binding().map_err(replay_adapter_error)?;
        if intent.intent.adapter_kind != *binding.adapter_kind()
            || intent.intent.adapter_version != *binding.adapter_version()
        {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "contract lifecycle adapter identity mismatch",
            ));
        }
        Ok(())
    }
}

impl Default for EvmContractLifecycleReplayVerifier {
    fn default() -> Self {
        Self::new().expect("valid verifier id")
    }
}

impl replay::SideEffectReplayVerifier for EvmContractLifecycleReplayVerifier {
    fn verifier_id(&self) -> &events::ReplayVerifierId {
        &self.verifier_id
    }

    fn verify_submission(
        &self,
        input: &replay::SideEffectSubmissionReplayInput,
    ) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        ensure_schema(
            &input.submission.submission.submission_schema_id,
            &ContractTransactionSubmissions::schema_id().map_err(replay_value_error)?,
            "submission",
        )?;
        let prepared = replay_prepared_invocation(input.prepared_invocation.as_ref())?;
        verify_prepared_matches_certified_side_effect_context(&input.certified_context, &prepared)?;
        verify_replay_intent_matches_prepared(&input.intent, &prepared)?;
        let submissions: ContractTransactionSubmissions =
            serde_json::from_slice(&input.submission.artifact_bytes).map_err(replay_json_error)?;
        verify_prepared_submissions(&prepared, &submissions).map_err(replay_adapter_error)?;
        Ok(())
    }

    fn verify_receipt(&self, input: &replay::SideEffectReceiptReplayInput) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        let prepared = replay_prepared_invocation(input.prepared_invocation.as_ref())?;
        verify_prepared_matches_certified_side_effect_context(&input.certified_context, &prepared)?;
        verify_replay_intent_matches_prepared(&input.intent, &prepared)?;
        if let Some(submission) = &input.submission {
            let submissions: ContractTransactionSubmissions =
                serde_json::from_slice(&submission.artifact_bytes).map_err(replay_json_error)?;
            verify_prepared_submissions(&prepared, &submissions).map_err(replay_adapter_error)?;
        }
        verify_contract_receipt_schema(&input.receipt.receipt.receipt_schema_id)?;
        verify_contract_receipt_artifact(
            &input.receipt.receipt.receipt_schema_id,
            &input.receipt.artifact_bytes,
            &prepared,
        )
    }

    fn verify_confirmation(
        &self,
        input: &replay::SideEffectConfirmationReplayInput,
    ) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        let required_depth = match &input.verification {
            spec::SideEffectVerificationSpec::Finalized { depth } => *depth,
            spec::SideEffectVerificationSpec::Receipt => {
                return Err(replay::ReplayError::new(
                    replay::ReplayErrorKind::SideEffectMismatch,
                    "receipt-only contract lifecycle side effect recorded confirmation evidence",
                ));
            }
        };
        let prepared = replay_prepared_invocation(input.prepared_invocation.as_ref())?;
        verify_prepared_matches_certified_side_effect_context(&input.certified_context, &prepared)?;
        verify_replay_intent_matches_prepared(&input.intent, &prepared)?;
        if let Some(submission) = &input.submission {
            let submissions: ContractTransactionSubmissions =
                serde_json::from_slice(&submission.artifact_bytes).map_err(replay_json_error)?;
            verify_prepared_submissions(&prepared, &submissions).map_err(replay_adapter_error)?;
        }
        if let Some(receipt) = &input.receipt {
            verify_contract_receipt_artifact(
                &receipt.receipt.receipt_schema_id,
                &receipt.artifact_bytes,
                &prepared,
            )?;
        }
        verify_contract_confirmation_schema(
            &input.confirmation.confirmation.confirmation_schema_id,
            &input.confirmation.artifact_bytes,
            required_depth,
        )?;
        verify_contract_confirmation_artifact(
            &input.confirmation.confirmation.confirmation_schema_id,
            &input.confirmation.artifact_bytes,
            &prepared,
        )
    }
}

fn replay_prepared_invocation(
    prepared: Option<&replay::PreparedInvocationReplayEvidence>,
) -> replay::Result<PreparedContractInvocation> {
    let prepared = prepared.ok_or_else(|| contract_lifecycle_side_effect_missing("prepared"))?;
    let evidence: PreparedContractInvocation =
        serde_json::from_slice(&prepared.artifact_bytes).map_err(replay_json_error)?;
    ensure_prepared_invocation_public(&evidence).map_err(replay_adapter_error)?;
    Ok(evidence)
}

fn verify_prepared_matches_certified_side_effect_context(
    certified: &replay::CertifiedSideEffectContext,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    ensure_prepared_invocation_public(prepared).map_err(replay_adapter_error)?;
    let expected_stage = context_stage_for_lifecycle_stage(prepared.resource_stage);
    let phase_stage_matches = matches!(
        (prepared.phase, prepared.resource_stage),
        (
            ContractMutationPhase::Deploy,
            ContractLifecycleStage::Deployed
        ) | (
            ContractMutationPhase::Configure,
            ContractLifecycleStage::Configured
        )
    );
    let (
        spec::NodeContextSpec::Required { context_ref },
        spec::CellContextSpec::Bound {
            context_ref: output_context_ref,
            resource_kind,
            stage,
            ..
        },
    ) = (&certified.node_context, &certified.output_context)
    else {
        return Err(replay_contract_mismatch(
            "contract lifecycle side effect is missing certified context authority",
        ));
    };
    if context_ref != output_context_ref
        || prepared.context_ref.as_context_ref() != context_ref
        || resource_kind != mfm_evm_contract_model::contract_instance_resource_kind()
        || stage != expected_stage
        || !phase_stage_matches
    {
        return Err(replay_contract_mismatch(
            "prepared invocation context does not match certified node context",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum VerifiedContractSideEffectIntent {
    Deploy(Box<ContextContractDeployIntent>),
    Configure(Box<ContextContractConfigureIntent>),
}

fn verify_replay_intent_matches_prepared(
    intent: &replay::SideEffectIntentReplayEvidence,
    prepared: &PreparedContractInvocation,
) -> replay::Result<VerifiedContractSideEffectIntent> {
    if intent.intent.intent_schema_id
        == ContextContractDeployIntent::schema_id().map_err(replay_value_error)?
    {
        let deploy: ContextContractDeployIntent =
            serde_json::from_slice(&intent.artifact_bytes).map_err(replay_json_error)?;
        if prepared.transactions.len() != 1 {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "deploy intent transaction count does not match prepared invocation",
            ));
        }
        verify_transaction_intent_matches_prepared(
            &deploy.transaction,
            &prepared.transactions[0],
            prepared,
        )?;
        if prepared.phase != ContractMutationPhase::Deploy
            || prepared.resource_stage != ContractLifecycleStage::Deployed
            || deploy.transaction.to_address.is_some()
        {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "deploy intent does not match prepared invocation context",
            ));
        }
        return Ok(VerifiedContractSideEffectIntent::Deploy(Box::new(deploy)));
    }
    if intent.intent.intent_schema_id
        == ContextContractConfigureIntent::schema_id().map_err(replay_value_error)?
    {
        let configure: ContextContractConfigureIntent =
            serde_json::from_slice(&intent.artifact_bytes).map_err(replay_json_error)?;
        if prepared.phase != ContractMutationPhase::Configure
            || prepared.resource_stage != ContractLifecycleStage::Configured
            || configure.deployed.context_ref != prepared.context_ref
        {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "configure intent does not match prepared invocation context",
            ));
        }
        if configure.transactions.len() != prepared.transactions.len() {
            return Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "configure intent transaction count does not match prepared invocation",
            ));
        }
        for (intent_transaction, prepared_transaction) in configure
            .transactions
            .iter()
            .zip(prepared.transactions.iter())
        {
            verify_transaction_intent_matches_prepared(
                intent_transaction,
                prepared_transaction,
                prepared,
            )?;
        }
        return Ok(VerifiedContractSideEffectIntent::Configure(Box::new(
            configure,
        )));
    }
    Err(replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        "contract lifecycle side-effect intent schema did not match deploy or configure intent",
    ))
}

fn verify_replay_prepared_transaction_data_matches_certified_inputs(
    broker: &replay::ReplayBroker,
    submit_node: &spec::NodeSpec,
    intent: &VerifiedContractSideEffectIntent,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    let context = certified_evm_context_for_ref(broker, prepared.context_ref.as_context_ref())?;
    match intent {
        VerifiedContractSideEffectIntent::Deploy(intent) => {
            let action: DeployAction = replay_node_config(broker, submit_node)?;
            let state = ContextBoundDeployContractState::new(
                ValidatedConfig::new(action.clone()).map_err(replay_adapter_error)?,
            )
            .map_err(replay_adapter_error)?;
            let expected_intent = state
                .prepare_intent(&(), &context)
                .map_err(replay_adapter_error)?;
            if &expected_intent != intent.as_ref() {
                return Err(replay_contract_mismatch(
                    "deploy intent does not match certified node config",
                ));
            }
            let artifact = replay_context_profile_artifact(broker, &context)?;
            let tx_inputs = vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(action.value_wei()).map_err(replay_adapter_error)?,
                data: deploy_action_data(&action, &artifact).map_err(replay_adapter_error)?,
            }];
            verify_prepared_transaction_data_matches_inputs(prepared, &tx_inputs)
        }
        VerifiedContractSideEffectIntent::Configure(intent) => {
            let action: ConfigureAction = replay_node_config(broker, submit_node)?;
            let deployed = configured_deployed_input_for_node(broker, submit_node, &context)?;
            let input = ContextConfigureContractInput { deployed };
            let state = ContextBoundConfigureContractState::new(
                ValidatedConfig::new(action.clone()).map_err(replay_adapter_error)?,
            )
            .map_err(replay_adapter_error)?;
            let expected_intent = state
                .prepare_intent(&input, &context)
                .map_err(replay_adapter_error)?;
            if &expected_intent != intent.as_ref() {
                return Err(replay_contract_mismatch(
                    "configure intent does not match certified node config and inputs",
                ));
            }
            let artifact = if configure_action_requires_artifact(&action) {
                Some(replay_context_profile_artifact(broker, &context)?)
            } else {
                None
            };
            let tx_inputs = configure_action_transaction_inputs(
                &action,
                artifact.as_ref(),
                input.deployed.address.as_str(),
            )
            .map_err(replay_adapter_error)?;
            verify_prepared_transaction_data_matches_inputs(prepared, &tx_inputs)
        }
    }
}

fn verify_prepared_transaction_data_matches_inputs(
    prepared: &PreparedContractInvocation,
    tx_inputs: &[PreparedTransactionInput],
) -> replay::Result<()> {
    if prepared.transactions.len() != tx_inputs.len() {
        return Err(replay_contract_mismatch(
            "prepared transaction count does not match certified transaction inputs",
        ));
    }
    for (index, (transaction, input)) in prepared.transactions.iter().zip(tx_inputs).enumerate() {
        let expected_to = input.to.as_ref().map(|address| format!("{address:?}"));
        if transaction.index != index as u64
            || transaction.chain_id != prepared.expected_chain_id
            || transaction.to_address != expected_to
            || transaction.value_wei != input.value_wei.to_string()
            || transaction.data_digest != digest_bytes(&input.data).to_string()
            || transaction.data_len != input.data.len() as u64
        {
            return Err(replay_contract_mismatch(
                "prepared transaction data does not match certified transaction inputs",
            ));
        }
    }
    Ok(())
}

fn verify_transaction_intent_matches_prepared(
    intent: &mfm_state_evm_contracts::ContextContractTransactionIntent,
    transaction: &PreparedContractTransactionEvidence,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    if intent.context_ref != prepared.context_ref
        || intent.network_id != prepared.network_id
        || intent.expected_chain_id != prepared.expected_chain_id
        || intent.signer_ref != prepared.signer_ref
        || normalize_address(&intent.expected_signer_address)
            .map_err(|error| replay_model_error(error.message))?
            != prepared.expected_signer_address
    {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "contract lifecycle intent context does not match prepared invocation",
        ));
    }
    let expected_to = intent
        .to_address
        .as_deref()
        .map(normalize_address)
        .transpose()
        .map_err(|error| replay_model_error(error.message))?;
    if transaction.to_address != expected_to
        || transaction.chain_id != intent.expected_chain_id
        || transaction.value_wei
            != parse_optional_wei(intent.value_wei.as_deref())
                .map_err(replay_adapter_error)?
                .to_string()
    {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "contract lifecycle transaction intent does not match prepared transaction",
        ));
    }
    verify_transaction_policy_matches_prepared(&intent.transaction, transaction)
}

fn verify_transaction_policy_matches_prepared(
    policy: &EvmTransactionPolicy,
    transaction: &PreparedContractTransactionEvidence,
) -> replay::Result<()> {
    let gas_limit = policy.gas_limit();
    if gas_limit.is_some_and(|gas_limit| gas_limit != transaction.gas_limit) {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "prepared transaction gas limit does not match certified policy",
        ));
    }
    match policy.style() {
        ConfigTransactionStyle::Eip1559 => {
            if transaction.style != PreparedContractTransactionStyle::Eip1559
                || transaction.gas_price.is_some()
            {
                return Err(replay::ReplayError::new(
                    replay::ReplayErrorKind::SideEffectMismatch,
                    "prepared transaction style does not match certified EIP-1559 policy",
                ));
            }
            verify_optional_policy_quantity_matches_prepared(
                policy.max_fee_per_gas(),
                transaction.max_fee_per_gas.as_deref(),
                "max_fee_per_gas",
            )?;
            verify_optional_policy_quantity_matches_prepared(
                policy.max_priority_fee_per_gas(),
                transaction.max_priority_fee_per_gas.as_deref(),
                "max_priority_fee_per_gas",
            )
        }
        ConfigTransactionStyle::Legacy => {
            if transaction.style != PreparedContractTransactionStyle::Legacy
                || transaction.max_fee_per_gas.is_some()
                || transaction.max_priority_fee_per_gas.is_some()
            {
                return Err(replay::ReplayError::new(
                    replay::ReplayErrorKind::SideEffectMismatch,
                    "prepared transaction style does not match certified legacy policy",
                ));
            }
            verify_optional_policy_quantity_matches_prepared(
                policy.gas_price(),
                transaction.gas_price.as_deref(),
                "gas_price",
            )
        }
    }
}

fn verify_optional_policy_quantity_matches_prepared(
    policy_value: Option<&str>,
    prepared_value: Option<&str>,
    field: &'static str,
) -> replay::Result<()> {
    let Some(policy_value) =
        optional_policy_quantity(policy_value, field).map_err(replay_adapter_error)?
    else {
        return Ok(());
    };
    let prepared_value =
        required_prepared_quantity(prepared_value, field).map_err(replay_adapter_error)?;
    if policy_value != prepared_value {
        return Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!("prepared transaction {field} does not match certified policy"),
        ));
    }
    Ok(())
}

fn verify_contract_receipt_schema(schema: &SchemaId) -> replay::Result<()> {
    let deploy = ContractDeployReceipt::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureReceipt::schema_id().map_err(replay_value_error)?;
    if schema == &deploy || schema == &context_configure {
        Ok(())
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "receipt schema did not match contract lifecycle schemas",
        ))
    }
}

fn verify_contract_receipt_artifact(
    schema: &SchemaId,
    artifact_bytes: &[u8],
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    let deploy = ContractDeployReceipt::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureReceipt::schema_id().map_err(replay_value_error)?;
    if schema == &deploy {
        let receipt: ContractDeployReceipt =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_deploy_receipt_matches_prepared(&receipt, prepared)
    } else if schema == &context_configure {
        let receipt: ContextContractConfigureReceipt =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_configure_receipt_matches_prepared(&receipt, prepared)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "receipt schema did not match contract lifecycle schemas",
        ))
    }
}

fn verify_contract_confirmation_schema(
    schema: &SchemaId,
    artifact_bytes: &[u8],
    required_depth: u64,
) -> replay::Result<()> {
    let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
    if schema == &deploy {
        let confirmation: ContractDeployConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        ensure_replay_confirmation_depth(confirmation.confirmations, required_depth)
    } else if schema == &context_configure {
        let confirmation: ContextContractConfigureConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        ensure_replay_confirmation_depth(confirmation.confirmations, required_depth)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "confirmation schema did not match contract lifecycle schemas",
        ))
    }
}

fn verify_contract_confirmation_artifact(
    schema: &SchemaId,
    artifact_bytes: &[u8],
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
    let context_configure =
        ContextContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
    if schema == &deploy {
        let confirmation: ContractDeployConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_deploy_confirmation_matches_prepared(&confirmation, prepared)
    } else if schema == &context_configure {
        let confirmation: ContextContractConfigureConfirmation =
            serde_json::from_slice(artifact_bytes).map_err(replay_json_error)?;
        verify_contract_configure_confirmation_matches_prepared(&confirmation, prepared)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "confirmation schema did not match contract lifecycle schemas",
        ))
    }
}

fn verify_contract_deploy_receipt_matches_prepared(
    receipt: &ContractDeployReceipt,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Deploy,
        &receipt.context_ref,
        &receipt.evm_network_context_ref,
        receipt.resource_stage,
        ContractLifecycleStage::Deployed,
        "deploy receipt context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(prepared, std::slice::from_ref(&receipt.receipt))
}

fn verify_contract_configure_receipt_matches_prepared(
    receipt: &ContextContractConfigureReceipt,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Configure,
        &receipt.context_ref,
        &receipt.evm_network_context_ref,
        receipt.resource_stage,
        ContractLifecycleStage::Configured,
        "configure receipt context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(prepared, &receipt.receipts)
}

fn verify_contract_deploy_confirmation_matches_prepared(
    confirmation: &ContractDeployConfirmation,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Deploy,
        &confirmation.context_ref,
        &confirmation.evm_network_context_ref,
        confirmation.resource_stage,
        ContractLifecycleStage::Deployed,
        "deploy confirmation context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(
        prepared,
        std::slice::from_ref(&confirmation.receipt),
    )
}

fn verify_contract_configure_confirmation_matches_prepared(
    confirmation: &ContextContractConfigureConfirmation,
    prepared: &PreparedContractInvocation,
) -> replay::Result<()> {
    verify_prepared_context(
        prepared,
        ContractMutationPhase::Configure,
        &confirmation.context_ref,
        &confirmation.evm_network_context_ref,
        confirmation.resource_stage,
        ContractLifecycleStage::Configured,
        "configure confirmation context does not match prepared invocation",
    )?;
    verify_transaction_receipts_match_prepared(prepared, &confirmation.receipts)
}

fn verify_prepared_context(
    prepared: &PreparedContractInvocation,
    expected_phase: ContractMutationPhase,
    actual_context_ref: &mfm_values::ContextRefValue,
    actual_evm_network_context_ref: &str,
    actual_resource_stage: ContractLifecycleStage,
    expected_resource_stage: ContractLifecycleStage,
    mismatch: &'static str,
) -> replay::Result<()> {
    if prepared.phase != expected_phase
        || actual_context_ref != &prepared.context_ref
        || actual_evm_network_context_ref != prepared.evm_network_context_ref
        || actual_resource_stage != expected_resource_stage
    {
        return Err(replay_contract_mismatch(mismatch));
    }
    Ok(())
}

fn verify_transaction_receipts_match_prepared(
    prepared: &PreparedContractInvocation,
    receipts: &[ContractTransactionReceipt],
) -> replay::Result<()> {
    if receipts.len() != prepared.transactions.len() {
        return Err(replay_contract_mismatch(
            "contract receipt count does not match prepared transactions",
        ));
    }
    for (prepared_transaction, receipt) in prepared.transactions.iter().zip(receipts) {
        if receipt.receipt_version != 1
            || receipt.context_ref != prepared.context_ref
            || receipt.evm_network_context_ref != prepared.evm_network_context_ref
            || receipt.resource_stage != prepared.resource_stage
            || !receipt.status
            || receipt.transaction_hash != prepared_transaction.expected_transaction_hash
        {
            return Err(replay_contract_mismatch(
                "contract receipt transaction evidence does not match prepared invocation",
            ));
        }
    }
    Ok(())
}

/// Returns the stable replay verifier id.
pub fn replay_verifier_id() -> Result<events::ReplayVerifierId> {
    events::ReplayVerifierId::new(REPLAY_VERIFIER_ID)
        .map_err(|error| EvmContractAdapterError::Identity(error.to_string()))
}

fn contract_side_effect_replay_evidence() -> mfm_runtime::Result<SideEffectReplayEvidence> {
    Ok(SideEffectReplayEvidence {
        replay_verifier_id: replay_verifier_id()?,
        resource_touched_set: None,
    })
}

/// Verifies contract lifecycle replay evidence when present in a broker stream.
///
/// Returns `Ok(false)` when the stream contains no contract lifecycle evidence.
pub fn verify_contract_lifecycle_replay(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
) -> replay::Result<bool> {
    let frames = broker.side_effect_replay_frames_matching(is_contract_lifecycle_intent)?;
    let verifier = EvmContractLifecycleReplayVerifier::new()?;
    let mut verified_frames = Vec::with_capacity(frames.len());
    for frame in &frames {
        verified_frames.push(verify_contract_lifecycle_replay_frame(
            broker, &verifier, frame,
        )?);
    }
    let output_count =
        verify_contract_lifecycle_state_outputs(broker, source_run_registry, &verified_frames)?;
    Ok(output_count > 0 || !frames.is_empty())
}

#[derive(Debug, Clone)]
struct VerifiedContractSideEffectFrame {
    pair_id: mfm_ids::SideEffectPairId,
    node_id: NodeId,
    intent: VerifiedContractSideEffectIntent,
    prepared: PreparedContractInvocation,
    terminal: VerifiedContractSideEffectTerminal,
}

#[derive(Debug, Clone)]
enum VerifiedContractSideEffectTerminal {
    DeployReceipt(ContractDeployReceipt),
    DeployConfirmation(ContractDeployConfirmation),
    ConfigureReceipt(ContextContractConfigureReceipt),
    ConfigureConfirmation(ContextContractConfigureConfirmation),
}

fn verify_contract_lifecycle_replay_frame(
    broker: &replay::ReplayBroker,
    verifier: &EvmContractLifecycleReplayVerifier,
    frame: &replay::SideEffectReplayFrame<'_>,
) -> replay::Result<VerifiedContractSideEffectFrame> {
    let pair = broker
        .certified_spec()
        .spec
        .side_effect_verify_pair_for_pair_id(&frame.intent.pair_id)
        .map_err(replay_adapter_error)?;
    let Some(submission_request) = frame.submission_request() else {
        return Err(contract_lifecycle_side_effect_missing("submission"));
    };
    let Some(receipt_request) = frame.receipt_request() else {
        return Err(contract_lifecycle_side_effect_missing("receipt"));
    };

    broker.verify_side_effect_submission(&submission_request, verifier)?;
    let receipt = broker.verify_side_effect_receipt(&receipt_request, verifier)?;
    let receipt_terminal = decode_verified_contract_receipt(&receipt)?;
    let terminal = if let Some(confirmation_request) = frame.confirmation_request() {
        let confirmation =
            broker.verify_side_effect_confirmation(&confirmation_request, verifier)?;
        decode_verified_contract_confirmation(&confirmation)?
    } else {
        receipt_terminal
    };
    let prepared = broker.side_effect_prepared_invocation(&receipt_request)?;
    let prepared = replay_prepared_invocation(Some(&prepared))?;
    let intent_evidence = replay_side_effect_intent(broker, frame.intent)?;
    let intent = verify_replay_intent_matches_prepared(&intent_evidence, &prepared)?;
    verify_replay_prepared_transaction_data_matches_certified_inputs(
        broker,
        pair.submit_node,
        &intent,
        &prepared,
    )?;
    Ok(VerifiedContractSideEffectFrame {
        pair_id: frame.intent.pair_id.clone(),
        node_id: frame.intent.node_id.clone(),
        intent,
        prepared,
        terminal,
    })
}

fn replay_side_effect_intent(
    broker: &replay::ReplayBroker,
    intent: &events::side_effect::IntentPersisted,
) -> replay::Result<replay::SideEffectIntentReplayEvidence> {
    let artifact = broker.retained_artifact(&store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::SideEffectIntent,
        artifact_id: intent.intent_artifact_id.clone(),
        digest: Some(intent.intent_hash.clone()),
        byte_len: None,
        media_type: None,
        schema_id: Some(intent.intent_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(intent.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::SideEffectIntent),
    })?;
    Ok(replay::SideEffectIntentReplayEvidence {
        intent: intent.clone(),
        artifact: artifact.artifact,
        artifact_bytes: artifact.artifact_bytes,
    })
}

fn decode_verified_contract_receipt(
    receipt: &replay::ReceiptReplayEvidence,
) -> replay::Result<VerifiedContractSideEffectTerminal> {
    let deploy = ContractDeployReceipt::schema_id().map_err(replay_value_error)?;
    let configure = ContextContractConfigureReceipt::schema_id().map_err(replay_value_error)?;
    if receipt.receipt.receipt_schema_id == deploy {
        serde_json::from_slice(&receipt.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::DeployReceipt)
            .map_err(replay_json_error)
    } else if receipt.receipt.receipt_schema_id == configure {
        serde_json::from_slice(&receipt.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::ConfigureReceipt)
            .map_err(replay_json_error)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "receipt schema did not match contract lifecycle schemas",
        ))
    }
}

fn decode_verified_contract_confirmation(
    confirmation: &replay::ConfirmationReplayEvidence,
) -> replay::Result<VerifiedContractSideEffectTerminal> {
    let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
    let configure =
        ContextContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
    if confirmation.confirmation.confirmation_schema_id == deploy {
        serde_json::from_slice(&confirmation.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::DeployConfirmation)
            .map_err(replay_json_error)
    } else if confirmation.confirmation.confirmation_schema_id == configure {
        serde_json::from_slice(&confirmation.artifact_bytes)
            .map(VerifiedContractSideEffectTerminal::ConfigureConfirmation)
            .map_err(replay_json_error)
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            "confirmation schema did not match contract lifecycle schemas",
        ))
    }
}

fn verify_contract_lifecycle_state_outputs(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
) -> replay::Result<usize> {
    let frames = broker.produced_cell_frames_matching(is_contract_lifecycle_output_cell)?;
    for frame in &frames {
        verify_contract_lifecycle_state_output(
            broker,
            source_run_registry,
            verified_frames,
            frame,
        )?;
    }
    Ok(frames.len())
}

fn is_contract_lifecycle_output_cell(
    _node: &spec::NodeSpec,
    cell: &spec::CellSpec,
    produced: &events::CellProduced,
) -> replay::Result<bool> {
    let deployed_schema = DeployedContractInstance::schema_id().map_err(replay_value_error)?;
    let configured_schema = ConfiguredContractInstance::schema_id().map_err(replay_value_error)?;
    let report_schema = ContextBoundValidationReport::schema_id().map_err(replay_value_error)?;
    if produced.schema_id == deployed_schema
        || produced.schema_id == configured_schema
        || produced.schema_id == report_schema
    {
        return Ok(true);
    }
    Ok(matches!(
        &cell.context,
        spec::CellContextSpec::Bound { resource_kind, .. }
            if resource_kind == contract_instance_resource_kind()
                || resource_kind == validation_report_resource_kind()
    ))
}

fn verify_contract_lifecycle_state_output(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<()> {
    let deployed_schema = DeployedContractInstance::schema_id().map_err(replay_value_error)?;
    let configured_schema = ConfiguredContractInstance::schema_id().map_err(replay_value_error)?;
    let report_schema = ContextBoundValidationReport::schema_id().map_err(replay_value_error)?;
    if frame.produced.schema_id == deployed_schema {
        let output: DeployedContractInstance =
            serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
        verify_deployed_replay_output(broker, source_run_registry, verified_frames, frame, &output)
    } else if frame.produced.schema_id == configured_schema {
        let output: ConfiguredContractInstance =
            serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
        verify_configured_replay_output(
            broker,
            source_run_registry,
            verified_frames,
            frame,
            &output,
        )
    } else if frame.produced.schema_id == report_schema {
        let output: ContextBoundValidationReport =
            serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
        verify_validation_report_replay_output(broker, frame, &output)
    } else {
        Err(replay_contract_mismatch(
            "contract lifecycle output schema is not a registered lifecycle value",
        ))
    }
}

fn verify_deployed_replay_output(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
    frame: &replay::ProducedCellReplayFrame,
    output: &DeployedContractInstance,
) -> replay::Result<()> {
    let context = verify_context_bound_replay_output::<DeployedContractInstance>(
        broker,
        frame,
        output,
        contract_instance_resource_kind(),
        deployed_contract_stage(),
    )?;
    match &output.deploy_provenance {
        DeployProvenance::MfmDeploy { .. } => {
            let expected = expected_deployed_output_from_verified_side_effect(
                broker,
                frame,
                verified_frames,
                &context,
            )?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "mfm deploy output does not match replayed side-effect evidence",
                ));
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "mfm deploy output carried external adoption evidence",
                ));
            }
            Ok(())
        }
        DeployProvenance::ImportedMfmRun {
            source_context_ref,
            source_value_digest,
            import_policy_digest,
        } => {
            let import: ImportDeployedSpec = replay_node_config(broker, &frame.node)?;
            let ImportDeployedSpec::FromMfmRun { source, evidence } = &import else {
                return Err(replay_contract_mismatch(
                    "deployed import output was not produced from source-run import config",
                ));
            };
            if source_context_ref != &evidence.source_context_ref
                || source_value_digest != &evidence.source_value_digest
                || import_policy_digest != &evidence.import_policy_digest
                || output.deploy_evidence != source_run_import_evidence_refs(evidence)
            {
                return Err(replay_contract_mismatch(
                    "deployed source-run import output does not match retained import evidence",
                ));
            }
            let imported = verify_replayed_source_run_import::<DeployedContractInstance>(
                broker,
                source_run_registry,
                source,
                evidence,
                &context,
                ContractLifecycleStage::Deployed,
            )?;
            let expected = ImportDeployedContractState::admit_verified_mfm_run_import(
                &import, imported, &context,
            )
            .map_err(replay_adapter_error)?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "deployed source-run import output does not match replayed projection",
                ));
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "source-run deployed import carried external adoption evidence",
                ));
            }
            Ok(())
        }
        DeployProvenance::ExternalAdoption {
            evidence_policy_digest,
            ..
        } => {
            let import: ImportDeployedSpec = replay_node_config(broker, &frame.node)?;
            let ImportDeployedSpec::AdoptExternalAddress { adoption } = &import else {
                return Err(replay_contract_mismatch(
                    "deployed external adoption output was not produced from adoption config",
                ));
            };
            let evidence = output.external_adoption_evidence.as_ref().ok_or_else(|| {
                replay_contract_mismatch("deployed external adoption evidence is missing")
            })?;
            if evidence_policy_digest != &evidence.evidence_policy_digest
                || !output.deploy_evidence.is_empty()
            {
                return Err(replay_contract_mismatch(
                    "deployed external adoption output evidence does not match provenance",
                ));
            }
            verify_replayed_external_adoption(
                evidence,
                &context,
                ContractLifecycleStage::Deployed,
                adoption,
            )
        }
    }
}

fn verify_configured_replay_output(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    verified_frames: &[VerifiedContractSideEffectFrame],
    frame: &replay::ProducedCellReplayFrame,
    output: &ConfiguredContractInstance,
) -> replay::Result<()> {
    let context = verify_context_bound_replay_output::<ConfiguredContractInstance>(
        broker,
        frame,
        output,
        contract_instance_resource_kind(),
        configured_contract_stage(),
    )?;
    if output.configured_from.deployed_context_ref.as_context_ref() != context.context_ref() {
        return Err(replay_contract_mismatch(
            "configured output source deployed context does not match certified context",
        ));
    }
    match &output.configuration_claim {
        ConfigurationClaim::MfmConfigured {
            call_evidence_refs,
            confirmation_evidence_refs,
            ..
        } => {
            let expected = expected_configured_output_from_verified_side_effect(
                broker,
                frame,
                verified_frames,
                &context,
            )?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "mfm configured output does not match replayed side-effect evidence",
                ));
            }
            if let ConfigurationClaim::MfmConfigured {
                call_evidence_refs: expected_call_refs,
                confirmation_evidence_refs: expected_confirmation_refs,
                ..
            } = &expected.configuration_claim
            {
                if call_evidence_refs != expected_call_refs
                    || confirmation_evidence_refs != expected_confirmation_refs
                {
                    return Err(replay_contract_mismatch(
                        "configured output evidence refs do not match replayed side-effect evidence",
                    ));
                }
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "mfm configured output carried external adoption evidence",
                ));
            }
        }
        ConfigurationClaim::ImportedMfmConfigured {
            source_run_id,
            source_spec_hash,
            source_cell_or_output_id,
            source_value_digest,
            source_context_ref,
        } => {
            let import: ImportConfiguredSpec = replay_node_config(broker, &frame.node)?;
            let ImportConfiguredSpec::FromMfmRun { source, evidence } = &import else {
                return Err(replay_contract_mismatch(
                    "configured import output was not produced from source-run import config",
                ));
            };
            if source_run_id != &source.source_run_id
                || source_spec_hash != &evidence.source_spec_hash
                || source_cell_or_output_id != &evidence.source_cell_or_output_id
                || source_value_digest != &evidence.source_value_digest
                || source_context_ref != &evidence.source_context_ref
                || output.configure_or_import_evidence != source_run_import_evidence_refs(evidence)
            {
                return Err(replay_contract_mismatch(
                    "configured source-run import output does not match retained import evidence",
                ));
            }
            let imported = verify_replayed_source_run_import::<ConfiguredContractInstance>(
                broker,
                source_run_registry,
                source,
                evidence,
                &context,
                ContractLifecycleStage::Configured,
            )?;
            let expected = ImportConfiguredContractState::admit_verified_mfm_run_import(
                &import, imported, &context,
            )
            .map_err(replay_adapter_error)?;
            if &expected != output {
                return Err(replay_contract_mismatch(
                    "configured source-run import output does not match replayed projection",
                ));
            }
            if output.external_adoption_evidence.is_some() {
                return Err(replay_contract_mismatch(
                    "source-run configured import carried external adoption evidence",
                ));
            }
        }
        ConfigurationClaim::ExternalObservedConfigured {
            evidence_policy_digest,
            external_adoption_evidence_digest,
            ..
        } => {
            let import: ImportConfiguredSpec = replay_node_config(broker, &frame.node)?;
            let ImportConfiguredSpec::AdoptExternalAddress { adoption } = &import else {
                return Err(replay_contract_mismatch(
                    "configured external adoption output was not produced from adoption config",
                ));
            };
            let evidence = output.external_adoption_evidence.as_ref().ok_or_else(|| {
                replay_contract_mismatch("configured external adoption evidence is missing")
            })?;
            if evidence_policy_digest != &evidence.evidence_policy_digest
                || !output.configure_or_import_evidence.is_empty()
            {
                return Err(replay_contract_mismatch(
                    "configured external adoption output evidence does not match provenance",
                ));
            }
            if digest_for_value(evidence).map_err(replay_adapter_error)?
                != *external_adoption_evidence_digest
            {
                return Err(replay_contract_mismatch(
                    "configured external adoption evidence digest does not match claim",
                ));
            }
            verify_replayed_external_adoption(
                evidence,
                &context,
                ContractLifecycleStage::Configured,
                adoption,
            )?;
        }
        ConfigurationClaim::ExternalClaimedConfigured {
            evidence_policy_digest,
            ..
        } => {
            let import: ImportConfiguredSpec = replay_node_config(broker, &frame.node)?;
            let ImportConfiguredSpec::AdoptExternalAddress { adoption } = &import else {
                return Err(replay_contract_mismatch(
                    "configured external adoption output was not produced from adoption config",
                ));
            };
            let evidence = output.external_adoption_evidence.as_ref().ok_or_else(|| {
                replay_contract_mismatch("configured external adoption evidence is missing")
            })?;
            if evidence_policy_digest != &evidence.evidence_policy_digest
                || !output.configure_or_import_evidence.is_empty()
            {
                return Err(replay_contract_mismatch(
                    "configured external adoption output evidence does not match provenance",
                ));
            }
            verify_replayed_external_adoption(
                evidence,
                &context,
                ContractLifecycleStage::Configured,
                adoption,
            )?;
        }
    }
    Ok(())
}

fn expected_deployed_output_from_verified_side_effect(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    verified_frames: &[VerifiedContractSideEffectFrame],
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<DeployedContractInstance> {
    let pair = side_effect_pair_for_output(broker, frame)?;
    let side_effect = verified_side_effect_for_pair(&pair, verified_frames)?;
    let VerifiedContractSideEffectIntent::Deploy(intent) = &side_effect.intent else {
        return Err(replay_contract_mismatch(
            "deploy output was bound to non-deploy side-effect intent",
        ));
    };
    if side_effect.prepared.phase != ContractMutationPhase::Deploy {
        return Err(replay_contract_mismatch(
            "deploy output was bound to non-deploy prepared invocation",
        ));
    }
    let action: DeployAction = replay_node_config(broker, pair.submit_node)?;
    let state = ContextBoundDeployContractState::new(
        ValidatedConfig::new(action).map_err(replay_adapter_error)?,
    )
    .map_err(replay_adapter_error)?;
    match &side_effect.terminal {
        VerifiedContractSideEffectTerminal::DeployReceipt(receipt) => state
            .output_from_receipt(&(), intent, receipt, context)
            .map_err(replay_adapter_error),
        VerifiedContractSideEffectTerminal::DeployConfirmation(confirmation) => state
            .output_from_confirmation(&(), intent, confirmation, context)
            .map_err(replay_adapter_error),
        _ => Err(replay_contract_mismatch(
            "deploy output was bound to non-deploy terminal side-effect evidence",
        )),
    }
}

fn expected_configured_output_from_verified_side_effect(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    verified_frames: &[VerifiedContractSideEffectFrame],
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<ConfiguredContractInstance> {
    let pair = side_effect_pair_for_output(broker, frame)?;
    let side_effect = verified_side_effect_for_pair(&pair, verified_frames)?;
    let VerifiedContractSideEffectIntent::Configure(intent) = &side_effect.intent else {
        return Err(replay_contract_mismatch(
            "configured output was bound to non-configure side-effect intent",
        ));
    };
    if side_effect.prepared.phase != ContractMutationPhase::Configure {
        return Err(replay_contract_mismatch(
            "configured output was bound to non-configure prepared invocation",
        ));
    }
    let deployed = configured_deployed_input_for_node(broker, pair.submit_node, context)?;
    let action: ConfigureAction = replay_node_config(broker, pair.submit_node)?;
    let state = ContextBoundConfigureContractState::new(
        ValidatedConfig::new(action).map_err(replay_adapter_error)?,
    )
    .map_err(replay_adapter_error)?;
    let input = ContextConfigureContractInput { deployed };
    match &side_effect.terminal {
        VerifiedContractSideEffectTerminal::ConfigureReceipt(receipt) => state
            .output_from_receipt(&input, intent, receipt, context)
            .map_err(replay_adapter_error),
        VerifiedContractSideEffectTerminal::ConfigureConfirmation(confirmation) => state
            .output_from_confirmation(&input, intent, confirmation, context)
            .map_err(replay_adapter_error),
        _ => Err(replay_contract_mismatch(
            "configured output was bound to non-configure terminal side-effect evidence",
        )),
    }
}

fn side_effect_pair_for_output<'a>(
    broker: &'a replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
) -> replay::Result<spec::SideEffectVerifyPairRef<'a>> {
    broker
        .certified_spec()
        .spec
        .side_effect_verify_pair_for_verify_node(&frame.node.node_id)
        .map_err(replay_adapter_error)
}

fn verified_side_effect_for_pair<'a>(
    pair: &spec::SideEffectVerifyPairRef<'_>,
    verified_frames: &'a [VerifiedContractSideEffectFrame],
) -> replay::Result<&'a VerifiedContractSideEffectFrame> {
    verified_frames
        .iter()
        .find(|verified| {
            verified.pair_id == *pair.pair_id && verified.node_id == pair.submit_node.node_id
        })
        .ok_or_else(|| {
            replay_contract_mismatch(
                "contract lifecycle output lacks verified side-effect evidence for its certified pair",
            )
        })
}

fn configured_deployed_input_for_node(
    broker: &replay::ReplayBroker,
    node: &spec::NodeSpec,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<DeployedContractInstance> {
    let deployed = replay_input_cell_value::<DeployedContractInstance>(
        broker,
        node,
        contract_instance_resource_kind(),
        deployed_contract_stage(),
    )?;
    if deployed.context_ref.as_context_ref() != context.context_ref() {
        return Err(replay_contract_mismatch(
            "configured output deployed input context does not match certified context",
        ));
    }
    Ok(deployed)
}

fn replay_input_cell_value<T>(
    broker: &replay::ReplayBroker,
    node: &spec::NodeSpec,
    expected_resource_kind: &mfm_ids::ContextResourceKind,
    expected_stage: &mfm_ids::ContextStage,
) -> replay::Result<T>
where
    T: ContextBoundOutput + MfmValue + DeserializeOwned,
{
    let schema = T::schema_id().map_err(replay_value_error)?;
    let semantic = T::semantic_id().map_err(replay_value_error)?;
    let cell = unique_input_cell(&node.input_bindings.root, &schema, &semantic)?;
    let frames = broker.produced_cell_frames_matching(|_node, _cell, produced| {
        Ok(produced.cell_id == cell.cell_id)
    })?;
    let mut frames = frames.into_iter();
    let frame = frames.next().ok_or_else(|| {
        replay_contract_mismatch("certified input cell was not produced in replay stream")
    })?;
    if frames.next().is_some() {
        return Err(replay_contract_mismatch(
            "certified input cell has multiple produced replay frames",
        ));
    }
    let value: T = serde_json::from_slice(&frame.artifact_bytes).map_err(replay_json_error)?;
    verify_context_bound_replay_output(
        broker,
        &frame,
        &value,
        expected_resource_kind,
        expected_stage,
    )?;
    Ok(value)
}

fn unique_input_cell<'a>(
    root: &'a spec::InputBindingNodeSpec,
    schema: &SchemaId,
    semantic: &mfm_ids::SemanticTypeId,
) -> replay::Result<&'a spec::InputBindingCellSpec> {
    let mut matches = Vec::new();
    collect_input_cells(root, schema, semantic, &mut matches);
    if matches.len() != 1 {
        return Err(replay_contract_mismatch(
            "certified node input bindings do not contain exactly one required lifecycle input cell",
        ));
    }
    Ok(matches.remove(0))
}

fn collect_input_cells<'a>(
    node: &'a spec::InputBindingNodeSpec,
    schema: &SchemaId,
    semantic: &mfm_ids::SemanticTypeId,
    matches: &mut Vec<&'a spec::InputBindingCellSpec>,
) {
    match node {
        spec::InputBindingNodeSpec::Unit => {}
        spec::InputBindingNodeSpec::Cell(cell) => {
            if cell.schema_id == *schema && cell.semantic_type_id == *semantic {
                matches.push(cell);
            }
        }
        spec::InputBindingNodeSpec::Tuple(elements)
        | spec::InputBindingNodeSpec::Vec { elements, .. }
        | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            for element in elements {
                collect_input_cells(element, schema, semantic, matches);
            }
        }
        spec::InputBindingNodeSpec::Struct(fields) => {
            for field in fields {
                collect_input_cells(&field.node, schema, semantic, matches);
            }
        }
    }
}

fn verify_validation_report_replay_output(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    output: &ContextBoundValidationReport,
) -> replay::Result<()> {
    let context = verify_context_bound_replay_output::<ContextBoundValidationReport>(
        broker,
        frame,
        output,
        validation_report_resource_kind(),
        validation_report_stage(),
    )?;
    if output.configured_instance.context_ref.as_context_ref() != context.context_ref() {
        return Err(replay_contract_mismatch(
            "validation report configured instance context does not match certified context",
        ));
    }
    let configured_input = replay_input_cell_value::<ConfiguredContractInstance>(
        broker,
        &frame.node,
        contract_instance_resource_kind(),
        configured_contract_stage(),
    )?;
    if output.configured_instance
        != ConfiguredContractInstanceRef::from_configured(&configured_input)
    {
        return Err(replay_contract_mismatch(
            "validation report configured instance does not match certified input cell",
        ));
    }
    if output.evidence_refs != configured_input.configure_or_import_evidence {
        return Err(replay_contract_mismatch(
            "validation report evidence refs do not match certified configured input",
        ));
    }
    verify_lifecycle_evidence_refs_retained(broker, &output.evidence_refs)?;
    let action: ValidateAction = replay_node_config(broker, &frame.node)?;
    verify_validation_results_match_action(output, &action)?;
    verify_validation_source_evidence(
        &context,
        &output.read_results,
        &output.validation_read_evidence,
        &output.event_results,
        &output.validation_event_evidence,
    )?;
    for result in output
        .configuration_read_results
        .iter()
        .chain(output.read_results.iter())
    {
        require_validation_read_result_canonical_passed(result).map_err(replay_adapter_error)?;
    }
    for result in output
        .configuration_event_results
        .iter()
        .chain(output.event_results.iter())
    {
        require_validation_event_result_canonical_passed(result).map_err(replay_adapter_error)?;
    }
    let valid = output.observed_chain_id == context.value().network.expected_chain_id()
        && output
            .configuration_read_results
            .iter()
            .all(validation_read_result_passes)
        && output
            .configuration_event_results
            .iter()
            .all(validation_event_result_passes)
        && output
            .read_results
            .iter()
            .all(validation_read_result_passes)
        && output
            .event_results
            .iter()
            .all(validation_event_result_passes);
    if output.valid != valid {
        return Err(replay_contract_mismatch(
            "validation report validity does not match retained evidence",
        ));
    }
    Ok(())
}

fn verify_context_bound_replay_output<T>(
    broker: &replay::ReplayBroker,
    frame: &replay::ProducedCellReplayFrame,
    output: &T,
    expected_resource_kind: &mfm_ids::ContextResourceKind,
    expected_stage: &mfm_ids::ContextStage,
) -> replay::Result<mfm_program::CertifiedContext<EvmContractContext>>
where
    T: ContextBoundOutput + MfmValue,
{
    let expected_schema = T::schema_id().map_err(replay_value_error)?;
    let expected_semantic = T::semantic_id().map_err(replay_value_error)?;
    let (
        spec::NodeContextSpec::Required { context_ref },
        spec::CellContextSpec::Bound {
            context_ref: output_context_ref,
            resource_kind,
            stage,
            ..
        },
    ) = (&frame.node.context, &frame.cell.context)
    else {
        return Err(replay_contract_mismatch(
            "contract lifecycle output is missing certified context authority",
        ));
    };
    if context_ref != output_context_ref
        || resource_kind != expected_resource_kind
        || stage != expected_stage
        || output.context_ref() != context_ref
        || output.context_resource_kind() != expected_resource_kind
        || output.context_stage() != expected_stage
        || frame.produced.context != frame.cell.context
        || frame.produced.schema_id != expected_schema
        || frame.produced.semantic_type_id != expected_semantic
    {
        return Err(replay_contract_mismatch(
            "contract lifecycle output context does not match certified cell authority",
        ));
    }
    certified_evm_context_for_ref(broker, context_ref)
}

fn certified_evm_context_for_ref(
    broker: &replay::ReplayBroker,
    context_ref: &mfm_ids::ContextRef,
) -> replay::Result<mfm_program::CertifiedContext<EvmContractContext>> {
    let context = broker
        .certified_spec()
        .spec
        .contexts
        .iter()
        .find(|context| &context.context_ref == context_ref)
        .ok_or_else(|| replay_contract_mismatch("certified context table entry is missing"))?;
    mfm_program::CertifiedContext::<EvmContractContext>::from_certified_spec(context)
        .map_err(replay_adapter_error)
}

fn replay_node_config<T>(broker: &replay::ReplayBroker, node: &spec::NodeSpec) -> replay::Result<T>
where
    T: MfmConfig + DeserializeOwned,
{
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: Some(node.config_ref.digest.clone()),
        byte_len: Some(node.config_ref.byte_len),
        media_type: Some(node.config_ref.media_type.clone()),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    let config: T = serde_json::from_slice(&artifact.artifact_bytes).map_err(replay_json_error)?;
    ValidatedConfig::new(config)
        .map(ValidatedConfig::into_inner)
        .map_err(replay_adapter_error)
}

fn verify_replayed_source_run_import<T>(
    broker: &replay::ReplayBroker,
    source_run_registry: &mfm_certify::CertificationRegistry,
    source: &ImportFromMfmRun,
    evidence: &ImportFromMfmRunEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
) -> replay::Result<T>
where
    T: ContextBoundOutput + DeserializeOwned + Serialize,
{
    validate_source_run_import_policy(source, context, required_stage)
        .map_err(replay_adapter_error)?;
    validate_source_run_import_evidence::<T>(source, evidence, context, required_stage)
        .map_err(replay_adapter_error)?;
    let certificate = replay_lifecycle_artifact(broker, &evidence.source_spec_certificate_ref)?;
    let bundle_artifact = replay_lifecycle_artifact(broker, &evidence.source_run_stream_ref)?;
    let source_value_artifact = replay_lifecycle_artifact(
        broker,
        &evidence.source_value_artifact_ref_or_inline_canonical_value,
    )?;
    let source_value: T =
        serde_json::from_slice(&source_value_artifact.artifact_bytes).map_err(replay_json_error)?;
    if source_value.context_ref() != evidence.source_context_ref.as_context_ref()
        || source_value.context_resource_kind() != contract_instance_resource_kind()
        || source_value.context_stage() != context_stage_for_lifecycle_stage(required_stage)
        || digest_for_value(&source_value).map_err(replay_adapter_error)?
            != evidence.source_value_digest
    {
        return Err(replay_contract_mismatch(
            "source-run import source value does not match retained evidence",
        ));
    }
    let committed = decode_source_run_committed_stream(
        source,
        &bundle_artifact.artifact_bytes,
        &source_value_artifact.artifact,
        &source_value_artifact.artifact_bytes,
    )
    .map_err(replay_adapter_error)?;
    let run_admitted = source_run_admitted(&committed).map_err(replay_adapter_error)?;
    let source_spec = replay_source_run_artifact(
        broker,
        events::EventArtifactReferenceSource::RunSpec,
        &run_admitted.spec_artifact,
    )?;
    validate_source_run_authority(SourceRunAuthorityEvidence {
        source,
        evidence,
        source_spec_bytes: &source_spec.artifact_bytes,
        certificate_bytes: &certificate.artifact_bytes,
        committed: &committed,
        source_value_artifact: &source_value_artifact.artifact,
        required_stage,
        source_run_registry,
    })
    .map_err(replay_adapter_error)?;
    Ok(source_value)
}

fn replay_lifecycle_artifact(
    broker: &replay::ReplayBroker,
    evidence: &LifecycleArtifactEvidenceRef,
) -> replay::Result<replay::ArtifactReplayEvidence> {
    let requirement = lifecycle_artifact_requirement(evidence).map_err(replay_adapter_error)?;
    broker.retained_artifact(&requirement)
}

fn replay_context_profile_artifact(
    broker: &replay::ReplayBroker,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> replay::Result<ContractArtifactConfig> {
    let reference = context
        .value()
        .contract_profile
        .artifact_ref
        .as_ref()
        .ok_or_else(|| replay_adapter_error(EvmContractAdapterError::MissingContractArtifact))?;
    let requirement =
        contract_profile_artifact_requirement(reference).map_err(replay_adapter_error)?;
    let artifact = broker.retained_artifact(&requirement)?;
    serde_json::from_slice::<ContractArtifactConfig>(&artifact.artifact_bytes)
        .map_err(replay_json_error)
}

fn replay_source_run_artifact(
    broker: &replay::ReplayBroker,
    source: events::EventArtifactReferenceSource,
    artifact: &events::RunArtifactEvidenceRef,
) -> replay::Result<replay::ArtifactReplayEvidence> {
    broker.retained_artifact(&run_artifact_requirement(source, artifact))
}

fn verify_lifecycle_evidence_refs_retained(
    broker: &replay::ReplayBroker,
    refs: &[LifecycleArtifactEvidenceRef],
) -> replay::Result<()> {
    for evidence in refs {
        verify_lifecycle_evidence_ref_retained(broker, evidence)?;
    }
    Ok(())
}

fn verify_lifecycle_evidence_ref_retained(
    broker: &replay::ReplayBroker,
    evidence: &LifecycleArtifactEvidenceRef,
) -> replay::Result<()> {
    replay_lifecycle_artifact(broker, evidence).map(|_| ())
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

fn verify_replayed_external_adoption(
    evidence: &ExternalAdoptionEvidence,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    required_stage: ContractLifecycleStage,
    adoption: &mfm_evm_contract_model::AdoptExternalAddress,
) -> replay::Result<()> {
    let policy = &adoption.evidence_policy;
    if evidence.context_ref.as_context_ref() != context.context_ref()
        || evidence.evm_network_context_ref
            != evm_network_context_ref(&context.value().network).map_err(replay_adapter_error)?
        || evidence.resource_stage != required_stage
        || evidence.observed_chain_id != context.value().network.expected_chain_id()
        || evidence.evidence_policy_digest
            != digest_for_value(policy).map_err(replay_adapter_error)?
    {
        return Err(replay_contract_mismatch(
            "external adoption evidence context does not match certified import authority",
        ));
    }
    if policy.require_code || policy.expected_code_hash.is_some() {
        let code = evidence.code_read_evidence.as_ref().ok_or_else(|| {
            replay_contract_mismatch("external adoption code-read evidence is missing")
        })?;
        verify_external_source_evidence(context, &code.source)?;
        let expected_block = policy
            .block_anchor
            .clone()
            .unwrap_or(ModelBlockSelector::Tag {
                tag: BlockTag::Latest,
            });
        if code.address != adoption.address
            || code.block != expected_block
            || (policy.require_code && code.observed_code_byte_len == 0)
            || policy
                .expected_code_hash
                .as_ref()
                .is_some_and(|expected| expected != &code.observed_code_hash)
        {
            return Err(replay_contract_mismatch(
                "external adoption code-read evidence does not match certified policy",
            ));
        }
    } else if evidence.code_read_evidence.is_some() {
        return Err(replay_contract_mismatch(
            "external adoption carried code-read evidence outside certified policy",
        ));
    }
    if evidence.read_assertion_evidence.len() != policy.initial_read_assertions.len()
        || evidence.event_assertion_evidence.len() != policy.initial_event_assertions.len()
    {
        return Err(replay_contract_mismatch(
            "external adoption assertion evidence count does not match certified policy",
        ));
    }
    for (record, assertion) in evidence
        .read_assertion_evidence
        .iter()
        .zip(policy.initial_read_assertions.iter())
    {
        verify_external_source_evidence(context, &record.source)?;
        if record.result.function.as_str() != assertion.function.as_str()
            || record.result.args != assertion.args
            || record.result.expected != assertion.expected
        {
            return Err(replay_contract_mismatch(
                "external adoption read assertion evidence does not match certified policy",
            ));
        }
        require_validation_read_result_canonical_passed(&record.result)
            .map_err(replay_adapter_error)?;
        if !validation_read_result_passes(&record.result) {
            return Err(replay_contract_mismatch(
                "external adoption read assertion evidence failed",
            ));
        }
    }
    for (record, assertion) in evidence
        .event_assertion_evidence
        .iter()
        .zip(policy.initial_event_assertions.iter())
    {
        if assertion.from_block.is_some() || assertion.to_block.is_some() {
            return Err(replay_contract_mismatch(
                "external adoption event assertion block bounds are not retained in evidence",
            ));
        }
        verify_external_source_evidence(context, &record.source)?;
        if record.result.event.as_str() != assertion.event.as_str()
            || record.result.min_count != assertion.min_count
        {
            return Err(replay_contract_mismatch(
                "external adoption event assertion evidence does not match certified policy",
            ));
        }
        require_validation_event_result_canonical_passed(&record.result)
            .map_err(replay_adapter_error)?;
        if !validation_event_result_passes(&record.result) {
            return Err(replay_contract_mismatch(
                "external adoption event assertion evidence failed",
            ));
        }
    }
    if required_stage == ContractLifecycleStage::Configured
        && evidence.read_assertion_evidence.is_empty()
        && evidence.event_assertion_evidence.is_empty()
        && !policy.allow_external_claimed_configured
    {
        return Err(replay_contract_mismatch(
            "configured external adoption lacks required assertion authority",
        ));
    }
    Ok(())
}

fn verify_validation_results_match_action(
    output: &ContextBoundValidationReport,
    action: &ValidateAction,
) -> replay::Result<()> {
    if output.read_results.len() != action.read_assertions().len()
        || output.event_results.len() != action.event_assertions().len()
    {
        return Err(replay_contract_mismatch(
            "validation report result count does not match certified validate action",
        ));
    }
    for (result, assertion) in output.read_results.iter().zip(action.read_assertions()) {
        if result.function != assertion.function.as_str()
            || result.args != assertion.args
            || result.expected != assertion.expected
        {
            return Err(replay_contract_mismatch(
                "validation read result does not match certified validate action",
            ));
        }
    }
    for (result, assertion) in output.event_results.iter().zip(action.event_assertions()) {
        if result.event != assertion.event.as_str() || result.min_count != assertion.min_count {
            return Err(replay_contract_mismatch(
                "validation event result does not match certified validate action",
            ));
        }
    }
    Ok(())
}

fn verify_validation_source_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    read_results: &[ValidationReadResult],
    read_evidence: &[ExternalReadAssertionEvidence],
    event_results: &[ValidationEventResult],
    event_evidence: &[ExternalEventAssertionEvidence],
) -> replay::Result<()> {
    if read_results.len() != read_evidence.len() || event_results.len() != event_evidence.len() {
        return Err(replay_contract_mismatch(
            "validation report evidence count does not match assertion results",
        ));
    }
    for (result, evidence) in read_results.iter().zip(read_evidence) {
        verify_external_source_evidence(context, &evidence.source)?;
        if result != &evidence.result {
            return Err(replay_contract_mismatch(
                "validation read evidence does not match report result",
            ));
        }
    }
    for (result, evidence) in event_results.iter().zip(event_evidence) {
        verify_external_source_evidence(context, &evidence.source)?;
        if result != &evidence.result {
            return Err(replay_contract_mismatch(
                "validation event evidence does not match report result",
            ));
        }
    }
    Ok(())
}

fn verify_external_source_evidence(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    evidence: &ExternalEvmSourceEvidence,
) -> replay::Result<()> {
    if evidence.network_id != context.value().network.network_id.to_string()
        || evidence.expected_chain_id != context.value().network.expected_chain_id()
        || evidence.observed_chain_id != context.value().network.expected_chain_id()
    {
        return Err(replay_contract_mismatch(
            "external EVM source evidence does not match certified context",
        ));
    }
    EvmNetworkId::new(evidence.network_id.as_str()).map_err(replay_adapter_error)?;
    Ok(())
}

fn lifecycle_evidence_ref(
    evidence: &CapabilityArtifactEvidenceRef,
) -> LifecycleArtifactEvidenceRef {
    LifecycleArtifactEvidenceRef::new(
        evidence.artifact_id.clone(),
        evidence.digest.clone(),
        evidence.byte_len,
        evidence.schema_id.clone(),
        evidence.semantic_type_id.clone(),
    )
}

fn deploy_contract_address_from_prepared(prepared: &PreparedContractInvocation) -> Result<String> {
    ensure_prepared_invocation_public(prepared)?;
    if prepared.phase != ContractMutationPhase::Deploy || prepared.transactions.len() != 1 {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    let transaction = &prepared.transactions[0];
    if transaction.to_address.is_some() {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    let signer = parse_address(&prepared.expected_signer_address, "expected_signer_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    normalize_address(&format!(
        "{:?}",
        created_contract_address(signer, transaction.nonce)
    ))
    .map_err(|error| EvmContractAdapterError::Model(error.message))
}

fn created_contract_address(sender: Address, nonce: u64) -> Address {
    let encoded = rlp_encode_list(&[sender.as_slice().to_vec(), u64_to_min_be(nonce)]);
    let hash = keccak256(encoded);
    Address::from_slice(&hash.as_slice()[12..])
}

/// Validates that prepared invocation evidence has no live or secret-bearing surface.
pub fn ensure_prepared_invocation_public(prepared: &PreparedContractInvocation) -> Result<()> {
    if prepared.prepared_version != 1 {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    EvmNetworkId::new(&prepared.network_id)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    SignerRef::new(&prepared.signer_ref)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    ensure_canonical_address(&prepared.expected_signer_address)?;
    ReceiptRetryPolicy::new(prepared.poll_interval_ms, prepared.max_receipt_polls)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    ContentDigest::parse(&prepared.evm_network_context_ref)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;

    for (index, transaction) in prepared.transactions.iter().enumerate() {
        ensure_prepared_transaction_public(prepared.expected_chain_id, index, transaction)?;
    }
    Ok(())
}

fn ensure_prepared_transaction_public(
    expected_chain_id: u64,
    index: usize,
    transaction: &PreparedContractTransactionEvidence,
) -> Result<()> {
    if transaction.index != index as u64 || transaction.chain_id != expected_chain_id {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    if let Some(to_address) = transaction.to_address.as_deref() {
        ensure_canonical_address(to_address)?;
    }
    ensure_canonical_quantity(&transaction.value_wei, "value_wei")?;
    ensure_canonical_quantity_optional(transaction.max_fee_per_gas.as_deref(), "max_fee_per_gas")?;
    ensure_canonical_quantity_optional(
        transaction.max_priority_fee_per_gas.as_deref(),
        "max_priority_fee_per_gas",
    )?;
    ensure_canonical_quantity_optional(transaction.gas_price.as_deref(), "gas_price")?;
    match transaction.style {
        PreparedContractTransactionStyle::Eip1559 => {
            if transaction.max_fee_per_gas.is_none()
                || transaction.max_priority_fee_per_gas.is_none()
                || transaction.gas_price.is_some()
            {
                return Err(EvmContractAdapterError::InvalidPreparedInvocation);
            }
        }
        PreparedContractTransactionStyle::Legacy => {
            if transaction.gas_price.is_none()
                || transaction.max_fee_per_gas.is_some()
                || transaction.max_priority_fee_per_gas.is_some()
            {
                return Err(EvmContractAdapterError::InvalidPreparedInvocation);
            }
        }
    }
    ContentDigest::parse(&transaction.data_digest)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    parse_b256_hex(&transaction.signing_digest)?;
    parse_b256_hex(&transaction.expected_transaction_hash)?;
    Ok(())
}

fn ensure_canonical_address(value: &str) -> Result<()> {
    let normalized =
        normalize_address(value).map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    if normalized == value {
        Ok(())
    } else {
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    }
}

fn ensure_canonical_quantity(value: &str, field: &'static str) -> Result<()> {
    let parsed = parse_u128_quantity(value, field)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    if parsed.to_string() == value {
        Ok(())
    } else {
        Err(EvmContractAdapterError::InvalidPreparedInvocation)
    }
}

fn ensure_canonical_quantity_optional(value: Option<&str>, field: &'static str) -> Result<()> {
    if let Some(value) = value {
        ensure_canonical_quantity(value, field)?;
    }
    Ok(())
}

fn parse_b256_hex(value: &str) -> Result<B256> {
    value
        .parse::<B256>()
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)
}

fn is_contract_lifecycle_intent(intent: &side_effect::IntentPersisted) -> replay::Result<bool> {
    let binding = evm_contract_lifecycle_adapter_binding().map_err(replay_adapter_error)?;
    Ok(intent.adapter_kind == *binding.adapter_kind()
        && intent.adapter_version == *binding.adapter_version())
}

fn contract_lifecycle_side_effect_missing(phase: &str) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMissing,
        format!("missing contract lifecycle {phase} evidence"),
    )
}

fn deploy_action_data(action: &DeployAction, artifact: &ContractArtifactConfig) -> Result<Vec<u8>> {
    let (abi, bytecode) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    constructor_data(&abi, &bytecode, action.constructor_args())
        .map_err(EvmContractAdapterError::Model)
}

fn configure_action_requires_artifact(action: &ConfigureAction) -> bool {
    !action.calls().is_empty()
}

fn configure_action_transaction_inputs(
    action: &ConfigureAction,
    artifact: Option<&ContractArtifactConfig>,
    contract_address: &str,
) -> Result<Vec<PreparedTransactionInput>> {
    if !configure_action_requires_artifact(action) {
        return Ok(Vec::new());
    }
    let artifact = artifact.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (abi, _) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    let to = parse_address(contract_address, "contract_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    action
        .calls()
        .iter()
        .map(|call| configure_transaction_input(&abi, to, call))
        .collect()
}

fn configure_transaction_input(
    abi: &mfm_evm_contract_model::ParsedAbi,
    to: Address,
    call: &ContractCallConfig,
) -> Result<PreparedTransactionInput> {
    let (data, _) = resolve_function_call(abi, call.function.as_str(), &call.args)
        .map_err(EvmContractAdapterError::Model)?;
    Ok(PreparedTransactionInput {
        to: Some(to),
        value_wei: parse_optional_wei(call.value_wei.as_ref().map(|value| value.as_str()))?,
        data,
    })
}

fn reconstruct_prepared_mutation(
    request: PreparedMutationReconstruction<'_>,
) -> Result<PreparedContractMutation> {
    let PreparedMutationReconstruction {
        evidence,
        phase,
        network_id,
        expected_chain_id,
        context_ref,
        evm_network_context_ref,
        resource_stage,
        signer_ref,
        expected_signer,
        expected_signer_text,
        tx_inputs,
    } = request;
    ensure_prepared_invocation_public(evidence)?;
    let expected_signer_address = normalize_address(expected_signer_text)
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    if evidence.phase != phase
        || evidence.network_id != network_id
        || evidence.expected_chain_id != expected_chain_id
        || evidence.context_ref != context_ref
        || evidence.evm_network_context_ref != evm_network_context_ref
        || evidence.resource_stage != resource_stage
        || evidence.signer_ref != signer_ref.to_string()
        || evidence.expected_signer_address != expected_signer_address
        || evidence.transactions.len() != tx_inputs.len()
    {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }

    let mut signing_requests = Vec::with_capacity(tx_inputs.len());
    for (index, (input, transaction)) in tx_inputs
        .into_iter()
        .zip(evidence.transactions.iter())
        .enumerate()
    {
        let expected_to = input.to.map(|address| format!("{address:?}"));
        if transaction.index != index as u64
            || transaction.chain_id != expected_chain_id
            || transaction.to_address != expected_to
            || transaction.value_wei != input.value_wei.to_string()
            || transaction.data_digest != digest_bytes(&input.data).to_string()
            || transaction.data_len != input.data.len() as u64
        {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }

        let signing_request = match transaction.style {
            PreparedContractTransactionStyle::Eip1559 => {
                let max_fee_per_gas = required_prepared_quantity(
                    transaction.max_fee_per_gas.as_deref(),
                    "max_fee_per_gas",
                )?;
                let max_priority_fee_per_gas = required_prepared_quantity(
                    transaction.max_priority_fee_per_gas.as_deref(),
                    "max_priority_fee_per_gas",
                )?;
                if transaction.gas_price.is_some() {
                    return Err(EvmContractAdapterError::InvalidPreparedInvocation);
                }
                EvmSigningRequest::eip1559(
                    signer_ref.clone(),
                    Eip1559TxToSign {
                        to: input.to,
                        value_wei: input.value_wei,
                        chain_id: expected_chain_id,
                        nonce: transaction.nonce,
                        max_fee_per_gas,
                        max_priority_fee_per_gas,
                        gas_limit: transaction.gas_limit,
                        data: input.data,
                    },
                    expected_signer,
                )?
            }
            PreparedContractTransactionStyle::Legacy => {
                let gas_price_wei =
                    required_prepared_quantity(transaction.gas_price.as_deref(), "gas_price")?;
                if transaction.max_fee_per_gas.is_some()
                    || transaction.max_priority_fee_per_gas.is_some()
                {
                    return Err(EvmContractAdapterError::InvalidPreparedInvocation);
                }
                EvmSigningRequest::legacy(
                    signer_ref.clone(),
                    LegacyTxToSign {
                        to: input.to,
                        value_wei: input.value_wei,
                        chain_id: expected_chain_id,
                        nonce: transaction.nonce,
                        gas_price_wei,
                        gas_limit: transaction.gas_limit,
                        data: input.data,
                    },
                    expected_signer,
                )?
            }
        };
        if transaction.signing_digest != format!("{:?}", signing_request.signing_hash()) {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }
        parse_prepared_transaction_hash(&transaction.expected_transaction_hash)?;
        signing_requests.push(signing_request);
    }

    PreparedContractMutation::new(evidence.clone(), signing_requests)
}

struct PreparedMutationReconstruction<'a> {
    evidence: &'a PreparedContractInvocation,
    phase: ContractMutationPhase,
    network_id: &'a str,
    expected_chain_id: u64,
    context_ref: mfm_values::ContextRefValue,
    evm_network_context_ref: String,
    resource_stage: ContractLifecycleStage,
    signer_ref: SignerRef,
    expected_signer: Address,
    expected_signer_text: &'a str,
    tx_inputs: Vec<PreparedTransactionInput>,
}

fn required_prepared_quantity(value: Option<&str>, field: &'static str) -> Result<u128> {
    optional_policy_quantity(value, field)?
        .ok_or(EvmContractAdapterError::InvalidPreparedInvocation)
}

fn parse_prepared_transaction_hash(value: &str) -> Result<B256> {
    parse_b256_hex(value)
}

fn verify_receipt_response(
    guard: &EvmChainGuard,
    expected_hash: B256,
    response: EvmReceiptReadResponse,
) -> Result<EvmReceiptReadResponse> {
    response.evidence.verify_guard(guard)?;
    if response.transaction_hash != expected_hash {
        return Err(EvmContractAdapterError::TransactionHashMismatch);
    }
    if !response.status {
        return Err(EvmContractAdapterError::TransactionFailed);
    }
    Ok(response)
}

fn prepared_anchor_submissions(
    prepared: &PreparedContractInvocation,
) -> Result<ContractTransactionSubmissions> {
    let transaction_hashes = prepared_transaction_hashes(prepared)?;
    Ok(transaction_hashes_to_submissions(
        prepared,
        transaction_hashes,
    ))
}

fn prepared_transaction_hashes(prepared: &PreparedContractInvocation) -> Result<Vec<B256>> {
    ensure_prepared_invocation_public(prepared)?;
    prepared
        .transactions
        .iter()
        .map(|transaction| parse_prepared_transaction_hash(&transaction.expected_transaction_hash))
        .collect::<Result<Vec<_>>>()
}

fn verify_prepared_submissions(
    prepared: &PreparedContractInvocation,
    submissions: &ContractTransactionSubmissions,
) -> Result<Vec<B256>> {
    if submissions.submissions_version != 1
        || submissions.context_ref != prepared.context_ref
        || submissions.evm_network_context_ref != prepared.evm_network_context_ref
        || submissions.resource_stage != prepared.resource_stage
    {
        return Err(EvmContractAdapterError::InvalidPreparedInvocation);
    }
    verify_prepared_submission_transactions(prepared, &submissions.transactions)
}

fn verify_prepared_submission_transactions(
    prepared: &PreparedContractInvocation,
    submissions: &[ContractTransactionSubmission],
) -> Result<Vec<B256>> {
    ensure_prepared_invocation_public(prepared)?;
    if prepared.transactions.len() != submissions.len() {
        return Err(EvmContractAdapterError::TransactionHashMismatch);
    }
    let mut transaction_hashes = Vec::with_capacity(submissions.len());
    for (prepared_transaction, submission) in prepared.transactions.iter().zip(submissions) {
        if submission.submission_version != 1
            || submission.context_ref != prepared.context_ref
            || submission.evm_network_context_ref != prepared.evm_network_context_ref
            || submission.resource_stage != prepared.resource_stage
        {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }
        let expected_hash =
            parse_prepared_transaction_hash(&prepared_transaction.expected_transaction_hash)?;
        let submitted_hash = parse_b256_hex(&submission.transaction_hash)
            .map_err(|_| EvmContractAdapterError::TransactionHashMismatch)?;
        if submitted_hash != expected_hash {
            return Err(EvmContractAdapterError::TransactionHashMismatch);
        }
        transaction_hashes.push(expected_hash);
    }
    Ok(transaction_hashes)
}

fn transaction_hashes_to_submissions(
    prepared: &PreparedContractInvocation,
    transaction_hashes: Vec<B256>,
) -> ContractTransactionSubmissions {
    ContractTransactionSubmissions {
        submissions_version: 1,
        context_ref: prepared.context_ref.clone(),
        evm_network_context_ref: prepared.evm_network_context_ref.clone(),
        resource_stage: prepared.resource_stage,
        transactions: transaction_hashes
            .into_iter()
            .map(|transaction_hash| ContractTransactionSubmission {
                submission_version: 1,
                context_ref: prepared.context_ref.clone(),
                evm_network_context_ref: prepared.evm_network_context_ref.clone(),
                resource_stage: prepared.resource_stage,
                transaction_hash: format!("{transaction_hash:?}"),
                signer_public_key: None,
            })
            .collect(),
    }
}

enum PreparedSubmissionReconciliation {
    Observed(ContractTransactionSubmissions),
    NotObserved(ContractTransactionSubmissions),
    NotSubmitted(ContractNotSubmittedProof),
    Indeterminate(ContractTransactionSubmissions),
}

type ContractSubmissionDecision = SideEffectSubmissionDecision<
    ContractTransactionSubmissions,
    ContractTransactionSubmissions,
    ContractNotSubmittedProof,
    ContractTransactionSubmissions,
>;

fn transaction_hash_mismatch_ambiguity(
    evidence: ContractTransactionSubmissions,
) -> Result<ContractSubmissionDecision> {
    Ok(SideEffectSubmissionDecision::Ambiguous {
        ambiguity_code: events::AmbiguityCode::new("mfm.evm.transaction_hash_mismatch")
            .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?,
        evidence,
    })
}

async fn submit_or_recover_contract_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
    action: SideEffectProtocolAction,
) -> Result<ContractSubmissionDecision> {
    if matches!(
        action,
        SideEffectProtocolAction::SubmitOrRecoverSubmission { .. }
    ) {
        match runtime
            .adapter()
            .reconcile_prepared_submission(prepared.evidence())
            .await?
        {
            PreparedSubmissionReconciliation::Observed(submissions) => {
                return Ok(SideEffectSubmissionDecision::Observed(submissions));
            }
            PreparedSubmissionReconciliation::Indeterminate(anchor_submissions) => {
                return Ok(SideEffectSubmissionDecision::Unknown(anchor_submissions));
            }
            PreparedSubmissionReconciliation::NotSubmitted(proof) => {
                return Ok(SideEffectSubmissionDecision::NotSubmitted(proof));
            }
            PreparedSubmissionReconciliation::NotObserved(anchor_submissions) => {
                return submit_prepared_contract_submission(runtime, prepared, anchor_submissions)
                    .await;
            }
        }
    }

    submit_prepared_contract_submission(
        runtime,
        prepared,
        prepared_anchor_submissions(prepared.evidence())?,
    )
    .await
}

async fn submit_prepared_contract_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
    ambiguity_evidence: ContractTransactionSubmissions,
) -> Result<ContractSubmissionDecision> {
    match runtime.adapter().submit_prepared(prepared).await {
        Ok(transactions) => Ok(SideEffectSubmissionDecision::Observed(
            ContractTransactionSubmissions {
                submissions_version: 1,
                context_ref: prepared.evidence().context_ref.clone(),
                evm_network_context_ref: prepared.evidence().evm_network_context_ref.clone(),
                resource_stage: prepared.evidence().resource_stage,
                transactions,
            },
        )),
        Err(EvmContractAdapterError::TransactionHashMismatch) => {
            transaction_hash_mismatch_ambiguity(ambiguity_evidence)
        }
        Err(error) => Err(error),
    }
}

async fn recover_unknown_prepared_contract_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
) -> mfm_runtime::Result<
    SideEffectUnknownSubmissionDecision<
        ContractTransactionSubmissions,
        ContractNotSubmittedProof,
        ContractTransactionSubmissions,
    >,
> {
    match runtime
        .adapter()
        .reconcile_prepared_submission(prepared.evidence())
        .await?
    {
        PreparedSubmissionReconciliation::Observed(submissions) => {
            Ok(SideEffectUnknownSubmissionDecision::Observed(submissions))
        }
        PreparedSubmissionReconciliation::NotSubmitted(proof) => {
            Ok(SideEffectUnknownSubmissionDecision::NotSubmitted(proof))
        }
        PreparedSubmissionReconciliation::Indeterminate(_)
        | PreparedSubmissionReconciliation::NotObserved(_) => {
            Ok(SideEffectUnknownSubmissionDecision::StillUnknown)
        }
    }
}

fn parse_optional_wei(value: Option<&str>) -> Result<u128> {
    value
        .map(|raw| parse_u128_quantity(raw, "value_wei").map_err(|error| error.message))
        .transpose()
        .map_err(EvmContractAdapterError::Model)
        .map(|value| value.unwrap_or(0))
}

fn optional_policy_quantity(value: Option<&str>, field: &'static str) -> Result<Option<u128>> {
    value
        .map(|raw| parse_u128_quantity(raw, field).map_err(|error| error.message))
        .transpose()
        .map_err(EvmContractAdapterError::Model)
}

fn block_selector(
    selector: Option<&mfm_evm_contract_model::BlockSelector>,
    default_latest: bool,
) -> Result<EvmBlockSelector> {
    use mfm_evm_contract_model::{BlockSelector, BlockTag};

    match selector {
        None if default_latest => Ok(EvmBlockSelector::Latest),
        None => Ok(EvmBlockSelector::Number(0)),
        Some(BlockSelector::Number { number }) => Ok(EvmBlockSelector::Number(*number)),
        Some(BlockSelector::Tag {
            tag: BlockTag::Earliest,
        }) => Ok(EvmBlockSelector::Number(0)),
        Some(BlockSelector::Tag {
            tag: BlockTag::Latest,
        }) => Ok(EvmBlockSelector::Latest),
        Some(BlockSelector::Tag {
            tag: BlockTag::Pending,
        }) => Ok(EvmBlockSelector::Pending),
        Some(BlockSelector::Tag {
            tag: BlockTag::Safe | BlockTag::Finalized,
        }) => Err(EvmContractAdapterError::UnsupportedBlockTag),
    }
}

fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn evm_network_context_ref(network: &EvmNetworkContext) -> Result<String> {
    let json = serde_json::to_string(network)
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.content_digest().to_string())
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))
}

fn ensure_deployed_input_context(
    input: &ContextConfigureContractInput,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> Result<()> {
    if input.deployed.context_ref.as_context_ref() == context.context_ref() {
        Ok(())
    } else {
        Err(EvmContractAdapterError::ContextMismatch)
    }
}

fn ensure_configured_input_context(
    input: &ContextValidateContractInput,
    context: &mfm_program::CertifiedContext<EvmContractContext>,
) -> Result<()> {
    if input.configured.context_ref.as_context_ref() == context.context_ref()
        && input
            .configured
            .configured_from
            .deployed_context_ref
            .as_context_ref()
            == context.context_ref()
    {
        Ok(())
    } else {
        Err(EvmContractAdapterError::ContextMismatch)
    }
}

fn public_key_hex(public_key: Option<&PublicKeyBytes>) -> Option<String> {
    public_key.map(|key| bytes_to_hex_prefixed(key.as_bytes()))
}

fn ensure_schema(
    actual: &SchemaId,
    expected: &SchemaId,
    label: &'static str,
) -> replay::Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!("{label} schema did not match contract lifecycle schema"),
        ))
    }
}

fn replay_adapter_error(error: impl fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn replay_model_error(error: impl fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn replay_contract_mismatch(message: &'static str) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::SideEffectMismatch, message)
}

fn replay_value_error(error: mfm_values::ValueError) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn replay_json_error(error: serde_json::Error) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
}

fn ensure_replay_confirmation_depth(confirmations: u64, required_depth: u64) -> replay::Result<()> {
    if confirmations >= required_depth {
        Ok(())
    } else {
        Err(replay::ReplayError::new(
            replay::ReplayErrorKind::SideEffectMismatch,
            format!(
                "contract lifecycle confirmation depth {confirmations} is below certified depth {required_depth}"
            ),
        ))
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
        network_id: &str,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()>;

    /// Returns process-local EVM read capabilities for validation on `network_id`.
    fn read_runtime_for(&self, network_id: &str) -> mfm_runtime::Result<EvmContractReadRuntime>;

    /// Returns process-local EVM and signing capabilities for `network_id`.
    fn runtime_for(&self, network_id: &str) -> mfm_runtime::Result<EvmContractRuntime>;
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

/// Registers context-bound contract lifecycle runners with the supplied runtime factory.
pub fn register_contract_lifecycle_runners_with_factory(
    registry: &mut ErasedRunnerRegistry,
    factory: Arc<dyn EvmContractRuntimeFactory>,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm-contracts",
        "evm-contract-lifecycle-context",
        env!("CARGO_PKG_VERSION"),
    )?;
    let side_effect_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(SIDE_EFFECT_FACTORY)?);
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    let adapter_binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_adapter_executable_with_factory(
        adapter_binding.adapter_kind().clone(),
        adapter_binding.adapter_version().clone(),
        &adapter_factory,
    )?;
    let deploy = registrations
        .register_state_descriptor_with_factory::<ContextBoundDeployContractState>(
            &side_effect_factory,
            Arc::new(ContractMutationRunner::<ContextDeployMutationPlan> {
                factory: factory.clone(),
                extractor: TypedContextOutputExtractor::new(),
                _phase: PhantomData,
            }),
        )?;
    let configure = registrations
        .register_state_descriptor_with_factory::<ContextBoundConfigureContractState>(
            &side_effect_factory,
            Arc::new(ContractMutationRunner::<ContextConfigureMutationPlan> {
                factory: factory.clone(),
                extractor: TypedContextOutputExtractor::new(),
                _phase: PhantomData,
            }),
        )?;
    registrations.register_state_descriptor_with_factory::<ContextBoundValidateContractState>(
        &read_factory,
        Arc::new(ContextContractValidateRunner {
            factory: factory.clone(),
            extractor: TypedContextOutputExtractor::new(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<mfm_state_evm_contracts::ImportDeployedContractState>(
        &read_factory,
        Arc::new(ImportDeployedRunner {
            factory: factory.clone(),
            extractor: TypedContextOutputExtractor::new(),
        }),
    )?;
    registrations.register_state_descriptor_with_factory::<mfm_state_evm_contracts::ImportConfiguredContractState>(
        &read_factory,
        Arc::new(ImportConfiguredRunner {
            factory: factory.clone(),
            extractor: TypedContextOutputExtractor::new(),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        deploy.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContextContractVerifyRunner {
            factory: factory.clone(),
            extractor: ContractLifecycleContextOutputExtractor::new(),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        configure.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContextContractVerifyRunner {
            factory,
            extractor: ContractLifecycleContextOutputExtractor::new(),
        }),
    )?;
    Ok(())
}

struct ContextContractValidateRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<ContextBoundValidationReport>,
}

impl ErasedNodeRunner for ContextContractValidateRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let context = ctx.context()?.certified_context::<EvmContractContext>()?;
        self.factory
            .validate_runtime_for(context.value().network.network_id.as_str(), None)
            .map(|_| ())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_context_validate(ctx, self.factory.as_ref()).await })
    }
}

struct ImportDeployedRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<DeployedContractInstance>,
}

impl ErasedNodeRunner for ImportDeployedRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let context = ctx.context()?.certified_context::<EvmContractContext>()?;
        self.factory
            .validate_runtime_for(context.value().network.network_id.as_str(), None)
            .map(|_| ())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_import_deployed(ctx, self.factory.as_ref()).await })
    }
}

struct ImportConfiguredRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<ConfiguredContractInstance>,
}

impl ErasedNodeRunner for ImportConfiguredRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let context = ctx.context()?.certified_context::<EvmContractContext>()?;
        self.factory
            .validate_runtime_for(context.value().network.network_id.as_str(), None)
            .map(|_| ())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_import_configured(ctx, self.factory.as_ref()).await })
    }
}

struct ContextContractVerifyRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: ContractLifecycleContextOutputExtractor,
}

impl ErasedNodeRunner for ContextContractVerifyRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let submit_node = side_effect_verify_submit_node_for_ingress(&ctx)?;
        validate_context_mutation_runtime_for_node(&ctx, submit_node, self.factory.as_ref())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_context_verify(ctx, self.factory.as_ref()).await })
    }
}

struct ContractLifecycleContextOutputExtractor {
    deployed: TypedContextOutputExtractor<DeployedContractInstance>,
    configured: TypedContextOutputExtractor<ConfiguredContractInstance>,
    validation_report: TypedContextOutputExtractor<ContextBoundValidationReport>,
}

impl ContractLifecycleContextOutputExtractor {
    const fn new() -> Self {
        Self {
            deployed: TypedContextOutputExtractor::new(),
            configured: TypedContextOutputExtractor::new(),
            validation_report: TypedContextOutputExtractor::new(),
        }
    }
}

impl mfm_runtime::ContextOutputExtractor for ContractLifecycleContextOutputExtractor {
    fn validate_context_output(
        &self,
        cell_context: &spec::CellContextSpec,
        artifact: &store::ArtifactEvidenceRef,
        bytes: &[u8],
    ) -> mfm_runtime::Result<()> {
        let spec::CellContextSpec::Bound { stage, .. } = cell_context else {
            return Ok(());
        };
        if stage == deployed_contract_stage() {
            return self
                .deployed
                .validate_context_output(cell_context, artifact, bytes);
        }
        if stage == configured_contract_stage() {
            return self
                .configured
                .validate_context_output(cell_context, artifact, bytes);
        }
        if stage == validation_report_stage() {
            return self
                .validation_report
                .validate_context_output(cell_context, artifact, bytes);
        }
        Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "unsupported EVM contract context output stage {}",
            stage
        )))
    }
}

struct ContractMutationRunner<P: ContractMutationPlanOps> {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    extractor: TypedContextOutputExtractor<<P as ContractMutationPlanOps>::Output>,
    _phase: PhantomData<P>,
}

impl<P> ErasedNodeRunner for ContractMutationRunner<P>
where
    P: ContractMutationPlanOps + 'static,
{
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        P::validate_ingress(&ctx, self.factory.as_ref())
    }

    fn context_output_extractor(&self) -> Option<&dyn mfm_runtime::ContextOutputExtractor> {
        Some(&self.extractor)
    }

    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let plan = P::load_plan(
                ctx.node(),
                ctx.inputs(),
                ctx.context(),
                self.factory.artifacts(),
            )
            .await?;
            claim_mutation_resource_lane(ctx, &plan)
        })
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_contract_mutation::<P>(ctx, self.factory.as_ref()).await })
    }
}

async fn run_contract_mutation<P>(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    P: ContractMutationPlanOps,
{
    let plan = P::load_plan(ctx.node(), ctx.inputs(), ctx.context(), factory.artifacts()).await?;
    let callbacks = ContractMutationSideEffectCallbacks { factory, plan };
    SideEffectDriver::drive(ctx, &callbacks).await
}

async fn run_context_verify(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let phase = {
        let submit_node = side_effect_verify_submit_node(&ctx)?;
        context_mutation_phase_for_submit_node(submit_node)?
    };
    match phase {
        ContractMutationPhase::Deploy => {
            let callbacks = ContractVerifyCallbacks::<ContextDeployMutationPlan> {
                factory,
                _phase: PhantomData,
            };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
        ContractMutationPhase::Configure => {
            let callbacks = ContractVerifyCallbacks::<ContextConfigureMutationPlan> {
                factory,
                _phase: PhantomData,
            };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
    }
}

fn side_effect_verify_submit_node<'a>(
    ctx: &'a ErasedRunCtx<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let submit_node_id = side_effect_verify_submit_node_id(ctx.node()).map_err(|_| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a side-effect verify node",
            ctx.node().node_id
        ))
    })?;
    ctx.certified_node(submit_node_id).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} references missing submit node {}",
            ctx.node().node_id,
            submit_node_id
        ))
    })
}

fn side_effect_verify_submit_node_for_ingress<'a>(
    ctx: &'a RunnerIngressContext<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let submit_node_id = side_effect_verify_submit_node_id(ctx.node()).map_err(|_| {
        mfm_runtime::RuntimeError::RunnerBinding(format!(
            "node {} is not a side-effect verify node",
            ctx.node().node_id
        ))
    })?;
    ctx.runtime_spec().node(submit_node_id).ok_or_else(|| {
        mfm_runtime::RuntimeError::RunnerBinding(format!(
            "side-effect verify node {} references missing submit node {}",
            ctx.node().node_id,
            submit_node_id
        ))
    })
}

fn side_effect_verify_submit_node_id(node: &spec::NodeSpec) -> std::result::Result<&NodeId, ()> {
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &node.framework else {
        return Err(());
    };
    Ok(&verify.submit_node_id)
}

fn context_mutation_phase_for_submit_node(
    node: &spec::NodeSpec,
) -> mfm_runtime::Result<ContractMutationPhase> {
    let deploy = mfm_program::state_descriptor::<ContextBoundDeployContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == deploy.descriptor_id() {
        return Ok(ContractMutationPhase::Deploy);
    }
    let configure = mfm_program::state_descriptor::<ContextBoundConfigureContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == configure.descriptor_id() {
        return Ok(ContractMutationPhase::Configure);
    }
    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "side-effect verify submit node {} is not a context-bound EVM contract mutation",
        node.node_id
    )))
}

fn validate_context_mutation_runtime_for_node(
    ctx: &RunnerIngressContext<'_>,
    node: &spec::NodeSpec,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<()> {
    let context = ctx
        .runtime_spec()
        .invocation_context_for_node(node)?
        .certified_context::<EvmContractContext>()?;
    let signer_ref = match context_mutation_phase_for_submit_node(node)? {
        ContractMutationPhase::Deploy => {
            let action = load_launch_config_for_node::<DeployAction>(ctx, node)?;
            action
                .as_ref()
                .signer()
                .signer_ref()
                .map_err(runtime_ingress_model_error)?
        }
        ContractMutationPhase::Configure => {
            let action = load_launch_config_for_node::<ConfigureAction>(ctx, node)?;
            action
                .as_ref()
                .signer()
                .signer_ref()
                .map_err(runtime_ingress_model_error)?
        }
    };
    factory
        .validate_runtime_for(
            context.value().network.network_id.as_str(),
            Some(&signer_ref),
        )
        .map(|_| ())
}

fn claim_mutation_resource_lane<P>(
    ctx: &PreInvocationRunCtx<'_>,
    plan: &P,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    P: ContractMutationPlanOps,
{
    let scope = plan.nonce_resource_scope()?;
    let resource_key = mutation_resource_key(ctx.node(), &scope, plan.expected_signer_address())?;
    SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
        plan.intent(),
        plan.idempotency(),
        idempotency_key_ref(plan.idempotency())?,
        evm_transaction_submit_binding()?,
        resource_key,
    )
}

struct ContextDeployMutationPlan {
    action: ValidatedConfig<DeployAction>,
    state: ContextBoundDeployContractState,
    context: mfm_program::CertifiedContext<EvmContractContext>,
    artifact: ContractArtifactConfig,
    intent: ContextContractDeployIntent,
    idempotency: ContractTransactionIdempotency,
}

struct ContextConfigureMutationPlan {
    node_id: NodeId,
    action: ValidatedConfig<ConfigureAction>,
    state: ContextBoundConfigureContractState,
    input: ContextConfigureContractInput,
    context: mfm_program::CertifiedContext<EvmContractContext>,
    artifact: Option<ContractArtifactConfig>,
    intent: ContextContractConfigureIntent,
    idempotency: ContractTransactionIdempotency,
}

struct ContractNonceResourceScope {
    evm_network_context_ref: String,
}

fn account_nonce_resource_key_for_node(
    node: &spec::NodeSpec,
    scope: &ContractNonceResourceScope,
    expected_signer_address: &str,
) -> Result<Option<events::ResourceKeyEvidence>> {
    let Some(side_effect) = &node.side_effect else {
        return Ok(None);
    };
    let spec::ResourceClaimSpec::Exclusive {
        namespace,
        key_schema,
    } = &side_effect.resource_claim
    else {
        return Ok(None);
    };

    let expected_namespace = account_nonce_resource_namespace()
        .map_err(|error| EvmContractAdapterError::State(error.to_string()))?;
    let expected_key_schema = account_nonce_resource_key_schema_id()
        .map_err(|error| EvmContractAdapterError::State(error.to_string()))?;
    if namespace != &expected_namespace || key_schema != &expected_key_schema {
        return Err(EvmContractAdapterError::State(
            "EVM mutation side effect must declare the account nonce resource claim".to_owned(),
        ));
    }

    let account = normalize_address(expected_signer_address)
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    let key = account_nonce_resource_key(scope, &account)?;

    Ok(Some(events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema.clone(),
        key,
    }))
}

fn account_nonce_resource_key(
    scope: &ContractNonceResourceScope,
    account: &str,
) -> Result<events::ResourceKey> {
    events::ResourceKey::new(format!(
        r#"{{"account":"{}","evm_network_context_ref":"{}"}}"#,
        account, scope.evm_network_context_ref
    ))
    .map_err(|error| EvmContractAdapterError::Model(error.to_string()))
}

fn mutation_resource_key(
    node: &spec::NodeSpec,
    scope: &ContractNonceResourceScope,
    expected_signer_address: &str,
) -> mfm_runtime::Result<events::ResourceKeyEvidence> {
    account_nonce_resource_key_for_node(node, scope, expected_signer_address)
        .map_err(mfm_runtime::RuntimeError::from)?
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "EVM mutation node {} requires an exclusive account nonce resource claim",
                node.node_id
            ))
        })
}

struct ContractMutationSideEffectCallbacks<'a, P> {
    factory: &'a dyn EvmContractRuntimeFactory,
    plan: P,
}

trait ContractMutationPlanOps: Send + Sync {
    type Intent: MfmValue + Clone + Send + Sync + 'static;
    type Output: mfm_values::ContextBoundOutput + 'static;

    fn validate_ingress(
        ctx: &RunnerIngressContext<'_>,
        factory: &dyn EvmContractRuntimeFactory,
    ) -> mfm_runtime::Result<()>;

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        inputs: &'a MaterializedInputs,
        context: &'a mfm_runtime::CertifiedInvocationContext,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self>
    where
        Self: Sized;

    fn action(&self) -> &dyn ContractMutationActionView;

    fn intent(&self) -> &Self::Intent;

    fn idempotency(&self) -> &ContractTransactionIdempotency;

    fn network_id(&self) -> &str;

    fn expected_chain_id(&self) -> u64;

    fn expected_signer_address(&self) -> &str {
        self.action().signer().expected_signer_address_str()
    }

    fn nonce_resource_scope(&self) -> mfm_runtime::Result<ContractNonceResourceScope>;

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation>;

    fn reconstruct_prepared_invocation(
        &self,
        runtime: &EvmContractRuntime,
        prepared: &PreparedContractInvocation,
    ) -> mfm_runtime::Result<PreparedContractMutation>;
}

impl ContractMutationPlanOps for ContextDeployMutationPlan {
    type Intent = ContextContractDeployIntent;
    type Output = DeployedContractInstance;

    fn validate_ingress(
        ctx: &RunnerIngressContext<'_>,
        factory: &dyn EvmContractRuntimeFactory,
    ) -> mfm_runtime::Result<()> {
        validate_context_mutation_runtime_for_node(ctx, ctx.node(), factory)
    }

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        _inputs: &'a MaterializedInputs,
        context: &'a mfm_runtime::CertifiedInvocationContext,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self> {
        Box::pin(
            async move { context_deploy_mutation_plan_for_node(node, context, artifacts).await },
        )
    }

    fn action(&self) -> &dyn ContractMutationActionView {
        self.action.as_ref()
    }

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn network_id(&self) -> &str {
        self.context.value().network.network_id.as_str()
    }

    fn expected_chain_id(&self) -> u64 {
        self.context.value().network.expected_chain_id()
    }

    fn nonce_resource_scope(&self) -> mfm_runtime::Result<ContractNonceResourceScope> {
        Ok(ContractNonceResourceScope {
            evm_network_context_ref: evm_network_context_ref(&self.context.value().network)
                .map_err(mfm_runtime::RuntimeError::from)?,
        })
    }

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation> {
        Box::pin(async move {
            runtime
                .adapter()
                .prepare_context_deploy_invocation(
                    &self.action,
                    &self.context,
                    &self.artifact,
                    &self.intent,
                )
                .await
                .map_err(mfm_runtime::RuntimeError::from)
        })
    }

    fn reconstruct_prepared_invocation(
        &self,
        runtime: &EvmContractRuntime,
        prepared: &PreparedContractInvocation,
    ) -> mfm_runtime::Result<PreparedContractMutation> {
        runtime
            .adapter()
            .reconstruct_context_deploy_invocation(
                &self.action,
                &self.context,
                &self.artifact,
                &self.intent,
                prepared,
            )
            .map_err(mfm_runtime::RuntimeError::from)
    }
}

impl ContractMutationPlanOps for ContextConfigureMutationPlan {
    type Intent = ContextContractConfigureIntent;
    type Output = ConfiguredContractInstance;

    fn validate_ingress(
        ctx: &RunnerIngressContext<'_>,
        factory: &dyn EvmContractRuntimeFactory,
    ) -> mfm_runtime::Result<()> {
        validate_context_mutation_runtime_for_node(ctx, ctx.node(), factory)
    }

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        inputs: &'a MaterializedInputs,
        context: &'a mfm_runtime::CertifiedInvocationContext,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self> {
        Box::pin(async move {
            context_configure_mutation_plan_for_inputs(node, inputs, context, artifacts).await
        })
    }

    fn action(&self) -> &dyn ContractMutationActionView {
        self.action.as_ref()
    }

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn network_id(&self) -> &str {
        self.context.value().network.network_id.as_str()
    }

    fn expected_chain_id(&self) -> u64 {
        self.context.value().network.expected_chain_id()
    }

    fn nonce_resource_scope(&self) -> mfm_runtime::Result<ContractNonceResourceScope> {
        Ok(ContractNonceResourceScope {
            evm_network_context_ref: evm_network_context_ref(&self.context.value().network)
                .map_err(mfm_runtime::RuntimeError::from)?,
        })
    }

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation> {
        Box::pin(async move {
            runtime
                .adapter()
                .prepare_context_configure_invocation(
                    &self.action,
                    &self.input,
                    &self.context,
                    self.artifact.as_ref(),
                    &self.intent,
                )
                .await
                .map_err(mfm_runtime::RuntimeError::from)
        })
    }

    fn reconstruct_prepared_invocation(
        &self,
        runtime: &EvmContractRuntime,
        prepared: &PreparedContractInvocation,
    ) -> mfm_runtime::Result<PreparedContractMutation> {
        runtime
            .adapter()
            .reconstruct_context_configure_invocation(
                &self.action,
                &self.input,
                &self.context,
                self.artifact.as_ref(),
                &self.intent,
                prepared,
            )
            .map_err(mfm_runtime::RuntimeError::from)
    }
}

impl<P> SideEffectDriverCallbacks for ContractMutationSideEffectCallbacks<'_, P>
where
    P: ContractMutationPlanOps,
{
    type Intent = P::Intent;
    type Idempotency = ContractTransactionIdempotency;
    type PreparedInvocation = PreparedContractInvocation;
    type Submission = ContractTransactionSubmissions;
    type SubmissionUnknownEvidence = ContractTransactionSubmissions;
    type NotSubmittedProof = ContractNotSubmittedProof;
    type AmbiguityEvidence = ContractTransactionSubmissions;

    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>> {
        Box::pin(async move {
            Ok(SideEffectIntentPlan {
                intent: self.plan.intent().clone(),
                idempotency: self.plan.idempotency().clone(),
                idempotency_key: idempotency_key_ref(self.plan.idempotency())?,
                capability_binding: evm_transaction_submit_binding()?,
            })
        })
    }

    fn prepare_invocation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        _plan: &'a SideEffectIntentPlan<Self::Intent, Self::Idempotency>,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocationPlan<Self::PreparedInvocation>>
    {
        Box::pin(async move {
            let runtime = self.factory.runtime_for(self.plan.network_id())?;
            let prepared = self.plan.prepare_invocation(&runtime).await?;
            Ok(SideEffectPreparedInvocationPlan::with_prepared_invocation(
                prepared.evidence().clone(),
            ))
        })
    }

    fn reconstruct_prepared_invocation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        Box::pin(async move {
            load_prepared_invocation(
                prepared,
                events::ArtifactRole::PreparedInvocation,
                ctx,
                self.factory.artifacts(),
            )
            .await
        })
    }

    fn submit_or_recover_submission<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        action: SideEffectProtocolAction,
        prepared: Option<Self::PreparedInvocation>,
    ) -> SideEffectSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::SubmissionUnknownEvidence,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    > {
        Box::pin(async move {
            let stored_prepared =
                prepared.ok_or_else(|| missing_side_effect_artifact("prepared invocation"))?;
            let runtime = self.factory.runtime_for(self.plan.network_id())?;
            let prepared = self
                .plan
                .reconstruct_prepared_invocation(&runtime, &stored_prepared)?;
            submit_or_recover_contract_submission(&runtime, &prepared, action)
                .await
                .map_err(mfm_runtime::RuntimeError::from)
        })
    }
}

struct ContractVerifyCallbacks<'a, P> {
    factory: &'a dyn EvmContractRuntimeFactory,
    _phase: PhantomData<P>,
}

impl<P> ContractVerifyCallbacks<'_, P>
where
    P: ContractVerifyPhase,
{
    async fn load_plan(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<P> {
        let submit_context = ctx.invocation_context_for_node(submit_node)?;
        P::load_plan(
            submit_node,
            submit_inputs,
            &submit_context,
            self.factory.artifacts(),
        )
        .await
    }

    async fn load_plan_and_runtime(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<(P, EvmContractRuntime)> {
        let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
        let runtime = self.factory.runtime_for(plan.network_id())?;
        Ok((plan, runtime))
    }

    async fn load_prepared_invocation(
        &self,
        submit_node: &spec::NodeSpec,
        projection: &store::SideEffectArtifactProjection,
    ) -> mfm_runtime::Result<PreparedContractInvocation> {
        load_prepared_invocation_for_node(
            projection,
            events::ArtifactRole::PreparedInvocation,
            self.factory.artifacts(),
            &submit_node.node_id,
        )
        .await
    }

    async fn load_reconstructed_prepared_invocation(
        &self,
        ctx: &ErasedRunCtx<'_>,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
        projection: &store::SideEffectArtifactProjection,
    ) -> mfm_runtime::Result<(P, EvmContractRuntime, PreparedContractMutation)> {
        let stored_prepared = self
            .load_prepared_invocation(submit_node, projection)
            .await?;
        let (plan, runtime) = self
            .load_plan_and_runtime(ctx, submit_node, submit_inputs)
            .await?;
        let prepared = plan.reconstruct_prepared_invocation(&runtime, &stored_prepared)?;
        Ok((plan, runtime, prepared))
    }
}

impl<P> SideEffectVerifyCallbacks for ContractVerifyCallbacks<'_, P>
where
    P: ContractVerifyPhase,
{
    type Submission = ContractTransactionSubmissions;
    type Receipt = P::Receipt;
    type Confirmation = P::Confirmation;
    type Output = <P as ContractMutationPlanOps>::Output;
    type NotSubmittedProof = ContractNotSubmittedProof;
    type AmbiguityEvidence = ContractTransactionSubmissions;

    fn recover_unknown_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        prepared_invocation: Option<&'a store::SideEffectArtifactProjection>,
    ) -> SideEffectUnknownSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    > {
        Box::pin(async move {
            let prepared_projection = prepared_invocation
                .ok_or_else(|| missing_side_effect_artifact("prepared invocation"))?;
            let (_plan, runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
                    ctx,
                    submit_node,
                    submit_inputs,
                    prepared_projection,
                )
                .await?;
            recover_unknown_prepared_contract_submission(&runtime, &prepared).await
        })
    }

    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        Box::pin(async {
            let prepared_projection = projected_prepared_artifact_for_submit(ctx, submit_node)?;
            let (plan, runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
                    ctx,
                    submit_node,
                    submit_inputs,
                    &prepared_projection,
                )
                .await?;
            let submissions = load_side_effect_value_for_node::<ContractTransactionSubmissions>(
                submission,
                events::ArtifactRole::Submission,
                &submit_node.node_id,
                self.factory.artifacts(),
            )
            .await?;
            let receipts =
                read_receipts_with_poll(&runtime, prepared.evidence(), &submissions).await?;
            Ok(SideEffectObservedEvidence {
                evidence: plan.receipt_from_observed(prepared.evidence(), receipts)?,
                replay: contract_side_effect_replay_evidence()?,
            })
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        Box::pin(async {
            let (plan, runtime) = self
                .load_plan_and_runtime(ctx, submit_node, submit_inputs)
                .await?;
            let required_depth = finalized_depth_for_submit_node(submit_node)?;
            let (receipt, receipt_evidence) = load_side_effect_artifact::<P::Receipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            let receipt_evidence = CapabilityArtifactEvidenceRef::from(receipt_evidence);
            let receipt = P::receipt_with_evidence(receipt, &receipt_evidence);
            let confirmations = verified_finality_confirmations(
                &runtime,
                plan.network_id(),
                plan.expected_chain_id(),
                P::receipt_transactions(&receipt),
                required_depth,
            )
            .await?;
            Ok(SideEffectObservedEvidence {
                evidence: P::confirmation_from_receipt(receipt, confirmations),
                replay: contract_side_effect_replay_evidence()?,
            })
        })
    }

    fn map_receipt_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
            let receipt = load_side_effect_value::<P::Receipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            plan.output_from_receipt(&receipt)
        })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = self.load_plan(ctx, submit_node, submit_inputs).await?;
            let confirmation = load_side_effect_value::<P::Confirmation>(
                confirmation,
                events::ArtifactRole::Confirmation,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            plan.output_from_confirmation(&confirmation)
        })
    }
}

trait ContractVerifyPhase: ContractMutationPlanOps + Sized + Send + Sync + 'static {
    type Receipt: MfmValue + Send + Sync + 'static;
    type Confirmation: MfmValue + Send + Sync + 'static;

    fn receipt_from_observed(
        &self,
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt>;

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt;

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt];

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation;

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output>;

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output>;
}

impl ContractVerifyPhase for ContextDeployMutationPlan {
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;

    fn receipt_from_observed(
        &self,
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContractDeployReceipt {
            receipt_version: 1,
            context_ref: prepared.context_ref.clone(),
            evm_network_context_ref: prepared.evm_network_context_ref.clone(),
            resource_stage: prepared.resource_stage,
            contract_address: deploy_contract_address_from_prepared(prepared)?,
            receipt: single_receipt(receipts)?,
        })
    }

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt {
        deploy_receipt_with_evidence(receipt, evidence)
    }

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt] {
        std::slice::from_ref(&receipt.receipt)
    }

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation {
        ContractDeployConfirmation {
            confirmation_version: 1,
            context_ref: receipt.context_ref,
            evm_network_context_ref: receipt.evm_network_context_ref,
            resource_stage: receipt.resource_stage,
            confirmations,
            contract_address: receipt.contract_address,
            receipt: receipt.receipt,
        }
    }

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_receipt(&(), &self.intent, receipt, &self.context)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_confirmation(&(), &self.intent, confirmation, &self.context)
            .map_err(runtime_state_error)
    }
}

impl ContractVerifyPhase for ContextConfigureMutationPlan {
    type Receipt = ContextContractConfigureReceipt;
    type Confirmation = ContextContractConfigureConfirmation;

    fn receipt_from_observed(
        &self,
        _prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContextContractConfigureReceipt {
            receipt_version: 1,
            context_ref: _prepared.context_ref.clone(),
            evm_network_context_ref: _prepared.evm_network_context_ref.clone(),
            resource_stage: _prepared.resource_stage,
            configure_node: LifecycleNodeIdRef::from(self.node_id.clone()),
            configured_block_number: receipts.iter().map(|receipt| receipt.block_number).max(),
            receipts,
            call_evidence_refs: Vec::new(),
            confirmation_evidence_refs: Vec::new(),
        })
    }

    fn receipt_with_evidence(
        mut receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt {
        let evidence = lifecycle_evidence_ref(evidence);
        for transaction_receipt in &mut receipt.receipts {
            transaction_receipt.receipt_evidence = Some(evidence.clone());
        }
        receipt
    }

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt] {
        &receipt.receipts
    }

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation {
        ContextContractConfigureConfirmation {
            confirmation_version: 1,
            context_ref: receipt.context_ref,
            evm_network_context_ref: receipt.evm_network_context_ref,
            resource_stage: receipt.resource_stage,
            confirmations,
            configure_node: receipt.configure_node,
            receipts: receipt.receipts,
            call_evidence_refs: receipt.call_evidence_refs,
            confirmation_evidence_refs: receipt.confirmation_evidence_refs,
            configured_block_number: receipt.configured_block_number,
        }
    }

    fn output_from_receipt(
        &self,
        receipt: &Self::Receipt,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_receipt(&self.input, &self.intent, receipt, &self.context)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<<Self as ContractMutationPlanOps>::Output> {
        self.state
            .output_from_confirmation(&self.input, &self.intent, confirmation, &self.context)
            .map_err(runtime_state_error)
    }
}

async fn context_deploy_mutation_plan_for_node(
    node: &spec::NodeSpec,
    invocation_context: &mfm_runtime::CertifiedInvocationContext,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContextDeployMutationPlan> {
    let action = load_runner_config_for_node::<DeployAction>(node, artifacts).await?;
    let context = invocation_context.certified_context::<EvmContractContext>()?;
    let artifact = load_context_profile_artifact(&context, artifacts).await?;
    let state = ContextBoundDeployContractState::new(action.clone()).map_err(runtime_plan_error)?;
    let intent = state
        .prepare_intent(&(), &context)
        .map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&(), &intent, &context)
        .map_err(runtime_state_error)?;
    Ok(ContextDeployMutationPlan {
        action,
        state,
        context,
        artifact,
        intent,
        idempotency,
    })
}

async fn context_configure_mutation_plan_for_inputs(
    node: &spec::NodeSpec,
    inputs: &mfm_runtime::MaterializedInputs,
    invocation_context: &mfm_runtime::CertifiedInvocationContext,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContextConfigureMutationPlan> {
    let action = load_runner_config_for_node::<ConfigureAction>(node, artifacts).await?;
    let deployed = load_materialized_struct_field_value::<DeployedContractInstance>(
        inputs, "deployed", artifacts,
    )
    .await?;
    let input = ContextConfigureContractInput { deployed };
    let context = invocation_context.certified_context::<EvmContractContext>()?;
    ensure_deployed_input_context(&input, &context).map_err(mfm_runtime::RuntimeError::from)?;
    let artifact = if configure_action_requires_artifact(action.as_ref()) {
        Some(load_context_profile_artifact(&context, artifacts).await?)
    } else {
        None
    };
    let state =
        ContextBoundConfigureContractState::new(action.clone()).map_err(runtime_plan_error)?;
    let intent = state
        .prepare_intent(&input, &context)
        .map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&input, &intent, &context)
        .map_err(runtime_state_error)?;
    Ok(ContextConfigureMutationPlan {
        node_id: node.node_id.clone(),
        action,
        state,
        input,
        context,
        artifact,
        intent,
        idempotency,
    })
}

async fn run_context_validate(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let action = load_runner_config::<ValidateAction>(&ctx, factory.artifacts()).await?;
    let input = load_context_validate_input(ctx.inputs(), factory.artifacts()).await?;
    let context = ctx.certified_context::<EvmContractContext>()?;
    ensure_configured_input_context(&input, &context).map_err(mfm_runtime::RuntimeError::from)?;
    let state = ContextBoundValidateContractState::new(action.clone())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let request = state
        .read_request(&input, &context)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let artifact = if validation_assertions_required(
        action.as_ref().read_assertions(),
        action.as_ref().event_assertions(),
    ) {
        Some(load_context_profile_artifact(&context, factory.artifacts()).await?)
    } else {
        None
    };
    let runtime = factory.read_runtime_for(context.value().network.network_id.as_str())?;
    let response = runtime
        .validate_context_contract(&action, &input, &context, artifact.as_ref(), &request)
        .await?;
    let report = state
        .report_from_response(&input, response, &context)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    ErasedRunnerOutput::state_output(&ctx, &report)
}

async fn run_import_deployed(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let import = load_runner_config::<ImportDeployedSpec>(&ctx, factory.artifacts()).await?;
    let context = ctx.certified_context::<EvmContractContext>()?;
    let runtime = factory.read_runtime_for(context.value().network.network_id.as_str())?;
    let deployed = runtime
        .import_deployed(import.as_ref(), &context, factory.artifacts())
        .await?;
    ErasedRunnerOutput::state_output(&ctx, &deployed)
}

async fn run_import_configured(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let import = load_runner_config::<ImportConfiguredSpec>(&ctx, factory.artifacts()).await?;
    let context = ctx.certified_context::<EvmContractContext>()?;
    let artifact = if import_configured_requires_artifact(import.as_ref()) {
        Some(load_context_profile_artifact(&context, factory.artifacts()).await?)
    } else {
        None
    };
    let runtime = factory.read_runtime_for(context.value().network.network_id.as_str())?;
    let configured = runtime
        .import_configured(
            import.as_ref(),
            &context,
            artifact.as_ref(),
            factory.artifacts(),
        )
        .await?;
    ErasedRunnerOutput::state_output(&ctx, &configured)
}

async fn read_receipts_with_poll(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractInvocation,
    submissions: &ContractTransactionSubmissions,
) -> mfm_runtime::Result<Vec<mfm_state_evm_contracts::ContractTransactionReceipt>> {
    verify_prepared_submissions(prepared, submissions).map_err(mfm_runtime::RuntimeError::from)?;
    let receipt_policy =
        ReceiptRetryPolicy::new(prepared.poll_interval_ms, prepared.max_receipt_polls)
            .map_err(mfm_runtime::RuntimeError::InvalidRunnerOutput)?;

    let max_polls = receipt_policy.max_receipt_polls();
    for attempt in 0..max_polls {
        match runtime
            .adapter()
            .read_receipts(prepared, &submissions.transactions)
            .await
        {
            Ok(receipts) => return Ok(receipts),
            Err(EvmContractAdapterError::EvmCapability(EvmCapabilityError::ReceiptPending)) => {
                if attempt + 1 == max_polls {
                    return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                        "contract lifecycle receipt polling exhausted".to_owned(),
                    ));
                }
                sleep(Duration::from_millis(receipt_policy.poll_interval_ms())).await;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
        "contract lifecycle receipt polling exhausted".to_owned(),
    ))
}

fn finalized_depth_for_submit_node(submit_node: &spec::NodeSpec) -> mfm_runtime::Result<u64> {
    match submit_node
        .side_effect
        .as_ref()
        .map(|contract| &contract.verification)
    {
        Some(spec::SideEffectVerificationSpec::Finalized { depth }) => Ok(*depth),
        Some(spec::SideEffectVerificationSpec::Receipt) => {
            Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "contract lifecycle submit node {} has receipt-only verification",
                submit_node.node_id
            )))
        }
        None => Err(mfm_runtime::RuntimeError::InvalidSpec(format!(
            "contract lifecycle submit node {} lacks side-effect contract",
            submit_node.node_id
        ))),
    }
}

async fn verified_finality_confirmations(
    runtime: &EvmContractRuntime,
    network_id: &str,
    expected_chain_id: u64,
    receipts: &[ContractTransactionReceipt],
    required_depth: u64,
) -> mfm_runtime::Result<u64> {
    let Some(highest_receipt_block) = receipts.iter().map(|receipt| receipt.block_number).max()
    else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle finality requires at least one receipt".to_owned(),
        ));
    };
    let latest_block = runtime
        .adapter()
        .latest_block_number(network_id, expected_chain_id)
        .await
        .map_err(mfm_runtime::RuntimeError::from)?;
    let confirmations = latest_block
        .checked_sub(highest_receipt_block)
        .map(|distance| distance.saturating_add(1))
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::Blocked(format!(
                "contract lifecycle finality latest block {latest_block} is behind receipt block {highest_receipt_block}"
            ))
        })?;
    if confirmations < required_depth {
        return Err(mfm_runtime::RuntimeError::Blocked(format!(
            "contract lifecycle finality depth {required_depth} not reached; observed {confirmations}"
        )));
    }
    Ok(confirmations)
}

async fn load_context_validate_input(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContextValidateContractInput> {
    let configured = load_materialized_struct_field_value::<ConfiguredContractInstance>(
        inputs,
        "configured",
        artifacts,
    )
    .await?;
    Ok(ContextValidateContractInput { configured })
}

async fn load_context_profile_artifact(
    context: &mfm_program::CertifiedContext<EvmContractContext>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ContractArtifactConfig> {
    let reference = context
        .value()
        .contract_profile
        .artifact_ref
        .as_ref()
        .ok_or(EvmContractAdapterError::MissingContractArtifact)
        .map_err(mfm_runtime::RuntimeError::from)?;
    let requirement = contract_profile_artifact_requirement(reference)
        .map_err(mfm_runtime::RuntimeError::from)?;
    let verified = artifacts
        .read_retained_artifact(&requirement)
        .await
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    serde_json::from_slice::<ContractArtifactConfig>(verified.bytes())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

fn contract_profile_artifact_requirement(
    reference: &LifecycleArtifactEvidenceRef,
) -> Result<store::EventArtifactRequirement> {
    let schema_id = reference
        .schema_id()
        .map_err(EvmContractAdapterError::Model)?
        .ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let expected_schema = <ContractArtifactConfig as MfmConfig>::schema_id()
        .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;
    if schema_id != expected_schema
        || reference
            .semantic_type_id()
            .map_err(EvmContractAdapterError::Model)?
            .is_some()
    {
        return Err(EvmContractAdapterError::MissingContractArtifact);
    }
    Ok(store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: reference
            .artifact_id()
            .map_err(EvmContractAdapterError::Model)?,
        digest: Some(
            reference
                .content_digest()
                .map_err(EvmContractAdapterError::Model)?,
        ),
        byte_len: Some(reference.byte_len()),
        media_type: Some(
            spec::MediaType::new("application/json")
                .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?,
        ),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    })
}

async fn load_prepared_invocation(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<PreparedContractInvocation> {
    load_prepared_invocation_for_node(artifact, role, artifacts, &ctx.node().node_id).await
}

async fn load_prepared_invocation_for_node(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    producer_node_id: &NodeId,
) -> mfm_runtime::Result<PreparedContractInvocation> {
    let prepared = load_side_effect_value_for_node::<PreparedContractInvocation>(
        artifact,
        role,
        producer_node_id,
        artifacts,
    )
    .await?;
    ensure_prepared_invocation_public(&prepared)?;
    Ok(prepared)
}

fn evm_transaction_submit_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    RunnerCapabilityBinding::for_capability::<EvmTransactionSubmitCapability>(
        binding.adapter_kind().clone(),
        binding.adapter_version().clone(),
    )
}

fn projected_side_effect_for_submit<'a>(
    ctx: &'a ErasedRunCtx<'_>,
    submit_node: &spec::NodeSpec,
) -> mfm_runtime::Result<&'a store::SideEffectProjection> {
    let contract = submit_node.side_effect.as_ref().ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidSpec(format!(
            "contract lifecycle submit node {} lacks side-effect contract",
            submit_node.node_id
        ))
    })?;
    let pair_id =
        spec::side_effect_pair_id(&submit_node.node_id, &submit_node.output_cell, contract)
            .map_err(|error| {
                mfm_runtime::RuntimeError::InvalidSpec(format!(
                    "contract lifecycle submit node {} pair id is invalid: {error}",
                    submit_node.node_id
                ))
            })?;
    ctx.projections()
        .side_effect_for_pair(ctx.run_id(), &pair_id)
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "contract lifecycle side-effect projection missing for submit node {}",
                submit_node.node_id
            ))
        })
}

fn projected_prepared_artifact_for_submit(
    ctx: &ErasedRunCtx<'_>,
    submit_node: &spec::NodeSpec,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect_for_submit(ctx, submit_node)?
        .prepared_invocation
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("prepared invocation"))
}

fn missing_side_effect_artifact(label: &str) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "contract lifecycle side-effect {label} artifact is missing"
    ))
}

fn single_receipt(
    mut receipts: Vec<ContractTransactionReceipt>,
) -> mfm_runtime::Result<ContractTransactionReceipt> {
    if receipts.len() != 1 {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract deploy confirmation requires exactly one receipt".to_owned(),
        ));
    }
    Ok(receipts.remove(0))
}

fn deploy_receipt_with_evidence(
    mut receipt: ContractDeployReceipt,
    evidence: &mfm_artifact_capabilities::ArtifactEvidenceRef,
) -> ContractDeployReceipt {
    let evidence = lifecycle_evidence_ref(evidence);
    receipt.receipt.receipt_evidence = Some(evidence);
    receipt
}

fn idempotency_key_ref(
    idempotency: &ContractTransactionIdempotency,
) -> mfm_runtime::Result<events::IdempotencyKeyRef> {
    Ok(events::IdempotencyKeyRef::new(format!(
        "mfm.evm.contract.idem.{}",
        short_stable_id_fragment(&idempotency.key, 32)
    ))?)
}

fn evm_chain_guard(network_id: &str, expected_chain_id: u64) -> Result<EvmChainGuard> {
    EvmChainGuard::new(EvmNetworkId::new(network_id)?, expected_chain_id).map_err(Into::into)
}

fn runtime_plan_error(error: mfm_program::PlanError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_state_error(error: mfm_program::StateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_ingress_model_error(error: String) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error)
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
            EvmContractAdapterError::EvmCapability(EvmCapabilityError::ChainMismatch {
                evidence,
            }) => {
                let message =
                    EvmContractAdapterError::EvmCapability(EvmCapabilityError::ChainMismatch {
                        evidence: evidence.clone(),
                    })
                    .to_string();
                Self::InvalidRunnerOutputDiagnostic {
                    message,
                    details: mfm_runtime::RuntimeDiagnosticDetails::from_json(
                        evidence.chain_mismatch_diagnostic_details(),
                    ),
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
