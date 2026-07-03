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
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1::{self as events, side_effect};
use mfm_evm_capabilities::{
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockSelector, EvmCallReadProvider,
    EvmCallReadRequest, EvmCapabilityError, EvmChainGuard, EvmChainIdentityProvider,
    EvmChainIdentityRequest, EvmChainIdentityResponse, EvmFeeReadProvider, EvmFeeReadRequest,
    EvmGasEstimateProvider, EvmGasEstimateRequest, EvmLogsReadProvider, EvmLogsReadRequest,
    EvmNetworkId, EvmNonceOccupancy, EvmNonceOccupancyReadProvider, EvmNonceOccupancyReadRequest,
    EvmNonceReadProvider, EvmNonceReadRequest, EvmReceiptReadProvider, EvmReceiptReadRequest,
    EvmReceiptReadResponse, EvmTransactionSubmitCapability, EvmTransactionSubmitProvider,
    EvmTransactionSubmitRequest, SignedEvmPayload,
};
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, EvmNetworkIntent, EvmSignerIntent,
    EvmTransactionPolicy, EvmTransactionStyle as ConfigTransactionStyle, ReceiptRetryPolicy,
    ValidatePhaseConfig,
};
use mfm_evm_contract_model::{
    constructor_data, decode_single_output_to_json, expected_matches, hex_to_bytes,
    normalize_address, parse_artifact, prepare_validate_assertions, resolve_function_call,
    ConfiguredContract, ContractArtifactConfig, ContractCallConfig, DeployedContract,
    EventAssertionConfig, ExpectedValue, LifecycleArtifactEvidenceRef, ParsedAbi,
    ReadAssertionConfig, ValidationEventResult, ValidationReadResult,
};
use mfm_evm_core::hex::bytes_to_hex_prefixed;
use mfm_evm_core::rlp::{rlp_encode_list, u64_to_min_be};
use mfm_evm_core::tx::{parse_address, parse_u128_quantity, Eip1559TxToSign, LegacyTxToSign};
use mfm_evm_signing::EvmSigningRequest;
use mfm_ids::{short_stable_id_fragment, ContentDigest, DigestAlgorithm, NodeId, SchemaId};
use mfm_program::{SideEffectState, StateSpec, ValidatedConfig};
use mfm_program_derive::MfmValue;
use mfm_replay::v1 as replay;
use mfm_runtime::{
    load_launch_config, load_launch_config_for_node, load_materialized_struct_field_value,
    load_runner_config, load_runner_config_for_node, load_side_effect_artifact,
    load_side_effect_value, load_side_effect_value_for_node, CapabilityImplementationId,
    ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry,
    MaterializedInputs, PreInvocationRunCtx, PreInvocationRunnerFuture, RunnerCapabilityBinding,
    RunnerExecutableIdentityTemplate, RunnerIngressContext, RunnerRegistrationBuilder,
    SideEffectDriver, SideEffectDriverCallbacks, SideEffectDriverFuture, SideEffectIntentPlan,
    SideEffectLanePreclaimBuilder, SideEffectObservedEvidence, SideEffectPreparedInvocationPlan,
    SideEffectProtocolAction, SideEffectReplayEvidence, SideEffectSubmissionDecision,
    SideEffectSubmissionDecisionFuture, SideEffectUnknownSubmissionDecision,
    SideEffectUnknownSubmissionDecisionFuture, SideEffectVerifyCallbacks, SideEffectVerifyDriver,
};
use mfm_signing::{PublicKeyBytes, SignerRef, SigningProvider};
use mfm_spec::v1 as spec;
use mfm_state_evm_contracts::{
    account_nonce_resource_key_schema_id, account_nonce_resource_namespace, ConfigureContractInput,
    ConfigureContractState, ContractConfigureConfirmation, ContractConfigureIntent,
    ContractConfigureReceipt, ContractDeployConfirmation, ContractDeployIntent,
    ContractDeployReceipt, ContractTransactionIdempotency, ContractTransactionReceipt,
    ContractTransactionSubmission, ContractTransactionSubmissions, ContractValidationReadRequest,
    ContractValidationReadResponse, DeployContractState, ValidateContractInput,
    ValidateContractState,
};
use mfm_store::v1 as store;
use mfm_values::{MfmConfig, MfmValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

const REPLAY_VERIFIER_ID: &str = "mfm.evm.contract.replay.v1";

/// Result type for lifecycle adapter operations.
pub type Result<T> = std::result::Result<T, EvmContractAdapterError>;

/// Extracts certified EVM guards from a contract lifecycle launch config artifact.
pub fn evm_chain_guards_from_launch_config(
    schema_id: &SchemaId,
    bytes: &[u8],
) -> Result<Vec<EvmChainGuard>> {
    if schema_id == &config_schema::<ValidatePhaseConfig>()? {
        let config = decode_replay_config::<ValidatePhaseConfig>(bytes)?;
        return Ok(vec![evm_chain_guard_for_network(
            config.as_ref().network(),
        )?]);
    }
    if schema_id == &config_schema::<DeployPhaseConfig>()? {
        let config = decode_replay_config::<DeployPhaseConfig>(bytes)?;
        return Ok(vec![evm_chain_guard_for_network(
            config.as_ref().network(),
        )?]);
    }
    if schema_id == &config_schema::<ConfigurePhaseConfig>()? {
        let config = decode_replay_config::<ConfigurePhaseConfig>(bytes)?;
        return Ok(vec![evm_chain_guard_for_network(
            config.as_ref().network(),
        )?]);
    }
    Ok(Vec::new())
}

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
            call: evm,
            logs: evm,
        }
    }

    /// Builds read provider bindings from a full EVM contract provider.
    pub fn from_contract_provider(evm: &'a dyn EvmContractProvider) -> Self {
        Self {
            chain_identity: evm,
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

    /// Prepares a deploy invocation with EIP-1559 default and legacy support.
    pub async fn prepare_deploy_invocation(
        &self,
        config: &ValidatedConfig<DeployPhaseConfig>,
        intent: &ContractDeployIntent,
    ) -> Result<PreparedContractMutation> {
        let expected = DeployContractState::new(config.clone())?.prepare_intent(&())?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        let config = config.as_ref();
        let data = deploy_data(config)?;
        self.prepare_transactions(prepare_transactions_request(
            ContractMutationPhase::Deploy,
            config,
            vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(config.value_wei())?,
                data,
            }],
        )?)
        .await
    }

    /// Prepares configure invocations with EIP-1559 default and legacy support.
    pub async fn prepare_configure_invocation(
        &self,
        config: &ValidatedConfig<ConfigurePhaseConfig>,
        input: &ConfigureContractInput,
        intent: &ContractConfigureIntent,
    ) -> Result<PreparedContractMutation> {
        let expected = ConfigureContractState::new(config.clone())?.prepare_intent(input)?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        let config = config.as_ref();
        let tx_inputs = configure_transaction_inputs(config, &input.deployed.contract_address)?;
        self.prepare_transactions(prepare_transactions_request(
            ContractMutationPhase::Configure,
            config,
            tx_inputs,
        )?)
        .await
    }

    /// Reconstructs deploy signing requests from persisted prepared evidence without live reads.
    pub fn reconstruct_deploy_invocation(
        &self,
        config: &ValidatedConfig<DeployPhaseConfig>,
        intent: &ContractDeployIntent,
        evidence: &PreparedContractInvocation,
    ) -> Result<PreparedContractMutation> {
        let expected = DeployContractState::new(config.clone())?.prepare_intent(&())?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        let config = config.as_ref();
        reconstruct_prepared_mutation(prepared_mutation_reconstruction(
            evidence,
            ContractMutationPhase::Deploy,
            config,
            vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(config.value_wei())?,
                data: deploy_data(config)?,
            }],
        )?)
    }

    /// Reconstructs configure signing requests from persisted prepared evidence without live reads.
    pub fn reconstruct_configure_invocation(
        &self,
        config: &ValidatedConfig<ConfigurePhaseConfig>,
        input: &ConfigureContractInput,
        intent: &ContractConfigureIntent,
        evidence: &PreparedContractInvocation,
    ) -> Result<PreparedContractMutation> {
        let expected = ConfigureContractState::new(config.clone())?.prepare_intent(input)?;
        if &expected != intent {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        let config = config.as_ref();
        reconstruct_prepared_mutation(prepared_mutation_reconstruction(
            evidence,
            ContractMutationPhase::Configure,
            config,
            configure_transaction_inputs(config, &input.deployed.contract_address)?,
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

    /// Executes validation reads and projects a validation response.
    pub async fn validate_contract(
        &self,
        config: &ValidatedConfig<ValidatePhaseConfig>,
        input: &ValidateContractInput,
        request: &ContractValidationReadRequest,
    ) -> Result<ContractValidationReadResponse> {
        validate_contract_with_reads(self.reads, config, input, request).await
    }

    async fn prepare_transactions(
        &self,
        request: PrepareTransactionsRequest<'_>,
    ) -> Result<PreparedContractMutation> {
        let PrepareTransactionsRequest {
            phase,
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
                network_id: network_id.to_owned(),
                expected_chain_id,
                signer_ref: signer_ref.to_string(),
                expected_signer_address: normalize_address(expected_signer_text)
                    .map_err(EvmContractAdapterError::Model)?,
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

async fn validate_contract_with_reads(
    reads: EvmContractReadProviders<'_>,
    config: &ValidatedConfig<ValidatePhaseConfig>,
    input: &ValidateContractInput,
    request: &ContractValidationReadRequest,
) -> Result<ContractValidationReadResponse> {
    let expected = ValidateContractState::new(config.clone())?.read_request(input)?;
    if &expected != request {
        return Err(EvmContractAdapterError::IntentMismatch);
    }
    let config = config.as_ref();
    let chain = verified_chain_identity(
        reads.chain_identity,
        config.network().network_id(),
        request.expected_chain_id,
    )
    .await?;

    let assertion_context = prepare_validation_assertion_context(
        config.artifact(),
        &input.configured.deployed.contract_address,
        validation_assertions_required(
            &input.configured.confirmation_read_assertions,
            &input.configured.confirmation_event_assertions,
        ) || validation_assertions_required(&request.read_assertions, &request.event_assertions),
    )?;
    let (configuration_read_results, configuration_event_results) = evaluate_assertions(
        reads,
        assertion_context.as_ref(),
        &chain.guard,
        &input.configured.confirmation_read_assertions,
        &input.configured.confirmation_event_assertions,
    )
    .await?;
    let (read_results, event_results) = evaluate_assertions(
        reads,
        assertion_context.as_ref(),
        &chain.guard,
        &request.read_assertions,
        &request.event_assertions,
    )
    .await?;

    Ok(ContractValidationReadResponse {
        response_version: 1,
        observed_chain_id: chain.response.chain_id,
        client_version: chain
            .response
            .client_version
            .unwrap_or_else(|| "unknown".to_owned()),
        configuration_read_results,
        configuration_event_results,
        read_results,
        event_results,
    })
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
) -> Result<(Vec<ValidationReadResult>, Vec<ValidationEventResult>)> {
    if !validation_assertions_required(read_assertions, event_assertions) {
        return Ok((Vec::new(), Vec::new()));
    }
    let context = context.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (prepared_reads, prepared_events) =
        prepare_validate_assertions(&context.abi, read_assertions, event_assertions)
            .map_err(EvmContractAdapterError::Model)?;

    let mut read_results = Vec::with_capacity(prepared_reads.len());
    for (assertion, prepared) in read_assertions.iter().zip(prepared_reads.iter()) {
        let response = reads
            .call
            .read_call(&EvmCallReadRequest {
                guard: guard.clone(),
                to: context.address,
                calldata: hex_to_bytes(&prepared.data_hex)
                    .map_err(EvmContractAdapterError::Model)?,
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
        read_results.push(ValidationReadResult {
            function: assertion.function.to_string(),
            args: assertion.args.clone(),
            expected: assertion.expected.clone(),
            passed: expected_matches(&actual, &assertion.expected),
            actual,
        });
    }

    let mut event_results = Vec::with_capacity(prepared_events.len());
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
        event_results.push(ValidationEventResult {
            event: prepared.event.clone(),
            min_count: prepared.min_count,
            observed_count,
            passed: observed_count >= prepared.min_count,
        });
    }

    Ok((read_results, event_results))
}

struct PreparedTransactionInput {
    to: Option<Address>,
    value_wei: u128,
    data: Vec<u8>,
}

struct PrepareTransactionsRequest<'a> {
    phase: ContractMutationPhase,
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

trait ContractMutationConfigView {
    fn network(&self) -> &EvmNetworkIntent;

    fn signer(&self) -> &EvmSignerIntent;

    fn transaction(&self) -> &EvmTransactionPolicy;

    fn receipt(&self) -> &ReceiptRetryPolicy;
}

impl ContractMutationConfigView for DeployPhaseConfig {
    fn network(&self) -> &EvmNetworkIntent {
        self.network()
    }

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

impl ContractMutationConfigView for ConfigurePhaseConfig {
    fn network(&self) -> &EvmNetworkIntent {
        self.network()
    }

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

struct ResolvedContractMutationConfig<'a> {
    network_id: &'a str,
    expected_chain_id: u64,
    signer_ref: SignerRef,
    expected_signer: Address,
    expected_signer_text: &'a str,
}

impl<'a> ResolvedContractMutationConfig<'a> {
    fn from_config(config: &'a impl ContractMutationConfigView) -> Result<Self> {
        Ok(Self {
            network_id: config.network().network_id(),
            expected_chain_id: config.network().expected_chain_id(),
            signer_ref: config
                .signer()
                .signer_ref()
                .map_err(EvmContractAdapterError::Model)?,
            expected_signer: config
                .signer()
                .expected_signer_address()
                .map_err(EvmContractAdapterError::Model)?,
            expected_signer_text: config.signer().expected_signer_address_str(),
        })
    }
}

fn prepare_transactions_request<'a>(
    phase: ContractMutationPhase,
    config: &'a impl ContractMutationConfigView,
    tx_inputs: Vec<PreparedTransactionInput>,
) -> Result<PrepareTransactionsRequest<'a>> {
    let resolved = ResolvedContractMutationConfig::from_config(config)?;
    Ok(PrepareTransactionsRequest {
        phase,
        network_id: resolved.network_id,
        expected_chain_id: resolved.expected_chain_id,
        signer_ref: resolved.signer_ref,
        expected_signer: resolved.expected_signer,
        expected_signer_text: resolved.expected_signer_text,
        policy: config.transaction(),
        poll_interval_ms: config.receipt().poll_interval_ms(),
        max_receipt_polls: config.receipt().max_receipt_polls(),
        tx_inputs,
    })
}

fn prepared_mutation_reconstruction<'a>(
    evidence: &'a PreparedContractInvocation,
    phase: ContractMutationPhase,
    config: &'a impl ContractMutationConfigView,
    tx_inputs: Vec<PreparedTransactionInput>,
) -> Result<PreparedMutationReconstruction<'a>> {
    let resolved = ResolvedContractMutationConfig::from_config(config)?;
    Ok(PreparedMutationReconstruction {
        evidence,
        phase,
        network_id: resolved.network_id,
        expected_chain_id: resolved.expected_chain_id,
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

/// Evidence-only replay verifier for contract lifecycle side effects.
pub struct EvmContractLifecycleReplayVerifier {
    verifier_id: events::ReplayVerifierId,
}

impl EvmContractLifecycleReplayVerifier {
    /// Creates a lifecycle replay verifier.
    pub fn new() -> Result<Self> {
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
        )
    }

    fn verify_receipt(&self, input: &replay::SideEffectReceiptReplayInput) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        let schema = &input.receipt.receipt.receipt_schema_id;
        let deploy = ContractDeployReceipt::schema_id().map_err(replay_value_error)?;
        let configure = ContractConfigureReceipt::schema_id().map_err(replay_value_error)?;
        if schema == &deploy || schema == &configure {
            Ok(())
        } else {
            Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "receipt schema did not match contract lifecycle schemas",
            ))
        }
    }

    fn verify_confirmation(
        &self,
        input: &replay::SideEffectConfirmationReplayInput,
    ) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        let schema = &input.confirmation.confirmation.confirmation_schema_id;
        let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
        let configure = ContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
        let required_depth = match &input.verification {
            spec::SideEffectVerificationSpec::Finalized { depth } => *depth,
            spec::SideEffectVerificationSpec::Receipt => {
                return Err(replay::ReplayError::new(
                    replay::ReplayErrorKind::SideEffectMismatch,
                    "receipt-only contract lifecycle side effect recorded confirmation evidence",
                ));
            }
        };
        if schema == &deploy {
            let confirmation: ContractDeployConfirmation =
                serde_json::from_slice(&input.confirmation.artifact_bytes)
                    .map_err(replay_json_error)?;
            ensure_replay_confirmation_depth(confirmation.confirmations, required_depth)
        } else if schema == &configure {
            let confirmation: ContractConfigureConfirmation =
                serde_json::from_slice(&input.confirmation.artifact_bytes)
                    .map_err(replay_json_error)?;
            ensure_replay_confirmation_depth(confirmation.confirmations, required_depth)
        } else {
            Err(replay::ReplayError::new(
                replay::ReplayErrorKind::SideEffectMismatch,
                "confirmation schema did not match contract lifecycle schemas",
            ))
        }
    }
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

/// Verifies contract lifecycle side-effect replay evidence when present in a broker stream.
///
/// Returns `Ok(false)` when the stream contains no contract lifecycle side-effect intent.
pub fn verify_contract_lifecycle_replay(broker: &replay::ReplayBroker) -> replay::Result<bool> {
    let frames = broker.side_effect_replay_frames_matching(is_contract_lifecycle_intent)?;
    if frames.is_empty() {
        return Ok(false);
    }
    let verifier = EvmContractLifecycleReplayVerifier::new()?;
    for frame in &frames {
        verify_contract_lifecycle_replay_frame(broker, &verifier, frame)?;
    }
    Ok(true)
}

fn verify_contract_lifecycle_replay_frame(
    broker: &replay::ReplayBroker,
    verifier: &EvmContractLifecycleReplayVerifier,
    frame: &replay::SideEffectReplayFrame<'_>,
) -> replay::Result<()> {
    let Some(submission_request) = frame.submission_request() else {
        return Err(contract_lifecycle_side_effect_missing("submission"));
    };
    let Some(receipt_request) = frame.receipt_request() else {
        return Err(contract_lifecycle_side_effect_missing("receipt"));
    };

    broker.verify_side_effect_submission(&submission_request, verifier)?;
    broker.verify_side_effect_receipt(&receipt_request, verifier)?;
    if let Some(confirmation_request) = frame.confirmation_request() {
        broker.verify_side_effect_confirmation(&confirmation_request, verifier)?;
    }
    Ok(())
}

/// Converts artifact capability evidence into lifecycle value evidence.
pub fn lifecycle_evidence_ref(
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

/// Derives the deployed contract address from public prepared deploy evidence.
pub fn deploy_contract_address_from_prepared(
    prepared: &PreparedContractInvocation,
) -> Result<String> {
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
    .map_err(EvmContractAdapterError::Model)
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

fn deploy_data(config: &DeployPhaseConfig) -> Result<Vec<u8>> {
    let artifact = config
        .artifact()
        .ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (abi, bytecode) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    constructor_data(&abi, &bytecode, config.constructor_args())
        .map_err(EvmContractAdapterError::Model)
}

fn configure_transaction_inputs(
    config: &ConfigurePhaseConfig,
    contract_address: &str,
) -> Result<Vec<PreparedTransactionInput>> {
    if config.calls().is_empty() {
        return Ok(Vec::new());
    }
    let artifact = config
        .artifact()
        .ok_or(EvmContractAdapterError::MissingContractArtifact)?;
    let (abi, _) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
    let to = parse_address(contract_address, "contract_address")
        .map_err(|error| EvmContractAdapterError::Model(error.message))?;
    config
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
        signer_ref,
        expected_signer,
        expected_signer_text,
        tx_inputs,
    } = request;
    ensure_prepared_invocation_public(evidence)?;
    let expected_signer_address =
        normalize_address(expected_signer_text).map_err(EvmContractAdapterError::Model)?;
    if evidence.phase != phase
        || evidence.network_id != network_id
        || evidence.expected_chain_id != expected_chain_id
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
    Ok(transaction_hashes_to_submissions(transaction_hashes))
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
    if submissions.submissions_version != 1 {
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
        if submission.submission_version != 1 {
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
    transaction_hashes: Vec<B256>,
) -> ContractTransactionSubmissions {
    ContractTransactionSubmissions {
        submissions_version: 1,
        transactions: transaction_hashes
            .into_iter()
            .map(|transaction_hash| ContractTransactionSubmission {
                submission_version: 1,
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

fn replay_adapter_error(error: mfm_adapter_contracts::AdapterContractError) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
    )
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
        + EvmCallReadProvider
        + EvmLogsReadProvider
{
}

/// EVM capability provider set required by contract validation reads.
pub trait EvmContractReadProvider:
    EvmChainIdentityProvider + EvmCallReadProvider + EvmLogsReadProvider
{
}

impl<T> EvmContractReadProvider for T where
    T: EvmChainIdentityProvider + EvmCallReadProvider + EvmLogsReadProvider
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
        Self { evm }
    }

    async fn validate_contract(
        &self,
        config: &ValidatedConfig<ValidatePhaseConfig>,
        input: &ValidateContractInput,
        request: &ContractValidationReadRequest,
    ) -> Result<ContractValidationReadResponse> {
        let evm = self.evm.as_ref();
        validate_contract_with_reads(
            EvmContractReadProviders::from_provider(evm),
            config,
            input,
            request,
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

/// Registers contract lifecycle runners with the supplied runtime factory.
pub fn register_contract_lifecycle_runners_with_factory(
    registry: &mut ErasedRunnerRegistry,
    factory: Arc<dyn EvmContractRuntimeFactory>,
) -> mfm_runtime::Result<()> {
    let implementation_id = CapabilityImplementationId::new(CAPABILITY_IMPLEMENTATION_ID)?;
    let mut registrations = RunnerRegistrationBuilder::new(registry, implementation_id);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-evm-contracts",
        "evm-contract-lifecycle",
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
    let deploy = registrations.register_state_descriptor_with_factory::<DeployContractState>(
        &side_effect_factory,
        Arc::new(ContractMutationRunner::<DeployMutationPlan> {
            factory: factory.clone(),
            _phase: PhantomData,
        }),
    )?;
    let configure = registrations
        .register_state_descriptor_with_factory::<ConfigureContractState>(
            &side_effect_factory,
            Arc::new(ContractMutationRunner::<ConfigureMutationPlan> {
                factory: factory.clone(),
                _phase: PhantomData,
            }),
        )?;
    registrations.register_state_descriptor_with_factory::<ValidateContractState>(
        &read_factory,
        Arc::new(ContractValidateRunner {
            factory: factory.clone(),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        deploy.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContractVerifyRunner {
            factory: factory.clone(),
        }),
    )?;
    registrations.register_side_effect_verify_runner_with_factory(
        configure.descriptor_id().clone(),
        &read_factory,
        Arc::new(ContractVerifyRunner { factory }),
    )?;
    Ok(())
}

struct ContractValidateRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
}

impl ErasedNodeRunner for ContractValidateRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let config = load_launch_config::<ValidatePhaseConfig>(&ctx)?;
        self.factory
            .validate_runtime_for(config.as_ref().network().network_id(), None)
            .map(|_| ())
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_validate(ctx, self.factory.as_ref()).await })
    }
}

struct ContractVerifyRunner {
    factory: Arc<dyn EvmContractRuntimeFactory>,
}

impl ErasedNodeRunner for ContractVerifyRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        let submit_node = side_effect_verify_submit_node_for_ingress(&ctx)?;
        validate_mutation_runtime_for_node(&ctx, submit_node, self.factory.as_ref())
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_verify(ctx, self.factory.as_ref()).await })
    }
}

struct ContractMutationRunner<P> {
    factory: Arc<dyn EvmContractRuntimeFactory>,
    _phase: PhantomData<P>,
}

impl<P> ErasedNodeRunner for ContractMutationRunner<P>
where
    P: ContractMutationPlanOps + 'static,
{
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        validate_mutation_runtime_for_node(&ctx, ctx.node(), self.factory.as_ref())
    }

    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let plan = P::load_plan(ctx.node(), ctx.inputs(), self.factory.artifacts()).await?;
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
    let plan = P::load_plan(ctx.node(), ctx.inputs(), factory.artifacts()).await?;
    let callbacks = ContractMutationSideEffectCallbacks { factory, plan };
    SideEffectDriver::drive(ctx, &callbacks).await
}

async fn run_verify(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let phase = {
        let submit_node = side_effect_verify_submit_node(&ctx)?;
        mutation_phase_for_submit_node(submit_node)?
    };
    match phase {
        ContractMutationPhase::Deploy => {
            let callbacks = ContractVerifyCallbacks::<DeployMutationPlan> {
                factory,
                _phase: PhantomData,
            };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
        ContractMutationPhase::Configure => {
            let callbacks = ContractVerifyCallbacks::<ConfigureMutationPlan> {
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

fn mutation_phase_for_submit_node(
    node: &spec::NodeSpec,
) -> mfm_runtime::Result<ContractMutationPhase> {
    let deploy = mfm_program::registered_state_descriptor::<DeployContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == deploy.descriptor_id() {
        return Ok(ContractMutationPhase::Deploy);
    }
    let configure = mfm_program::registered_state_descriptor::<ConfigureContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == configure.descriptor_id() {
        return Ok(ContractMutationPhase::Configure);
    }
    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "side-effect verify submit node {} is not an EVM contract mutation",
        node.node_id
    )))
}

fn validate_mutation_runtime_for_node(
    ctx: &RunnerIngressContext<'_>,
    node: &spec::NodeSpec,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<()> {
    match mutation_phase_for_submit_node(node)? {
        ContractMutationPhase::Deploy => {
            let config = load_launch_config_for_node::<DeployPhaseConfig>(ctx, node)?;
            validate_mutation_runtime_for_config(factory, config.as_ref())
        }
        ContractMutationPhase::Configure => {
            let config = load_launch_config_for_node::<ConfigurePhaseConfig>(ctx, node)?;
            validate_mutation_runtime_for_config(factory, config.as_ref())
        }
    }
}

fn validate_mutation_runtime_for_config(
    factory: &dyn EvmContractRuntimeFactory,
    config: &impl ContractMutationConfigView,
) -> mfm_runtime::Result<()> {
    let signer_ref = config
        .signer()
        .signer_ref()
        .map_err(runtime_ingress_model_error)?;
    factory
        .validate_runtime_for(config.network().network_id(), Some(&signer_ref))
        .map(|_| ())
}

fn claim_mutation_resource_lane<P>(
    ctx: &PreInvocationRunCtx<'_>,
    plan: &P,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    P: ContractMutationPlanOps,
{
    let resource_key = mutation_resource_key(
        ctx.node(),
        plan.expected_chain_id(),
        plan.expected_signer_address(),
    )?;
    SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
        plan.intent(),
        plan.idempotency(),
        idempotency_key_ref(plan.idempotency())?,
        evm_transaction_submit_binding()?,
        resource_key,
    )
}

struct DeployMutationPlan {
    config: ValidatedConfig<DeployPhaseConfig>,
    state: DeployContractState,
    intent: ContractDeployIntent,
    idempotency: ContractTransactionIdempotency,
}

struct ConfigureMutationPlan {
    config: ValidatedConfig<ConfigurePhaseConfig>,
    state: ConfigureContractState,
    input: ConfigureContractInput,
    intent: ContractConfigureIntent,
    idempotency: ContractTransactionIdempotency,
}

fn account_nonce_resource_key_for_node(
    node: &spec::NodeSpec,
    expected_chain_id: u64,
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

    let account =
        normalize_address(expected_signer_address).map_err(EvmContractAdapterError::Model)?;
    let key = events::ResourceKey::new(format!(
        r#"{{"account":"{}","chain_id":{}}}"#,
        account, expected_chain_id
    ))
    .map_err(|error| EvmContractAdapterError::Model(error.to_string()))?;

    Ok(Some(events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema.clone(),
        key,
    }))
}

fn mutation_resource_key(
    node: &spec::NodeSpec,
    expected_chain_id: u64,
    expected_signer_address: &str,
) -> mfm_runtime::Result<events::ResourceKeyEvidence> {
    account_nonce_resource_key_for_node(node, expected_chain_id, expected_signer_address)
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

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        inputs: &'a MaterializedInputs,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self>
    where
        Self: Sized;

    fn config(&self) -> &dyn ContractMutationConfigView;

    fn intent(&self) -> &Self::Intent;

    fn idempotency(&self) -> &ContractTransactionIdempotency;

    fn network_id(&self) -> &str {
        self.config().network().network_id()
    }

    fn expected_chain_id(&self) -> u64 {
        self.config().network().expected_chain_id()
    }

    fn expected_signer_address(&self) -> &str {
        self.config().signer().expected_signer_address_str()
    }

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

impl ContractMutationPlanOps for DeployMutationPlan {
    type Intent = ContractDeployIntent;

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        _inputs: &'a MaterializedInputs,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self> {
        Box::pin(async move { deploy_mutation_plan_for_node(node, artifacts).await })
    }

    fn config(&self) -> &dyn ContractMutationConfigView {
        self.config.as_ref()
    }

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation> {
        Box::pin(async move {
            runtime
                .adapter()
                .prepare_deploy_invocation(&self.config, &self.intent)
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
            .reconstruct_deploy_invocation(&self.config, &self.intent, prepared)
            .map_err(mfm_runtime::RuntimeError::from)
    }
}

impl ContractMutationPlanOps for ConfigureMutationPlan {
    type Intent = ContractConfigureIntent;

    fn load_plan<'a>(
        node: &'a spec::NodeSpec,
        inputs: &'a MaterializedInputs,
        artifacts: &'a dyn store::RetainedArtifactReadProvider,
    ) -> SideEffectDriverFuture<'a, Self> {
        Box::pin(async move { configure_mutation_plan_for_inputs(node, inputs, artifacts).await })
    }

    fn config(&self) -> &dyn ContractMutationConfigView {
        self.config.as_ref()
    }

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn prepare_invocation<'a>(
        &'a self,
        runtime: &'a EvmContractRuntime,
    ) -> SideEffectDriverFuture<'a, PreparedContractMutation> {
        Box::pin(async move {
            runtime
                .adapter()
                .prepare_configure_invocation(&self.config, &self.input, &self.intent)
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
            .reconstruct_configure_invocation(&self.config, &self.input, &self.intent, prepared)
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
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<P> {
        P::load_plan(submit_node, submit_inputs, self.factory.artifacts()).await
    }

    async fn load_plan_and_runtime(
        &self,
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
    ) -> mfm_runtime::Result<(P, EvmContractRuntime)> {
        let plan = self.load_plan(submit_node, submit_inputs).await?;
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
        submit_node: &spec::NodeSpec,
        submit_inputs: &MaterializedInputs,
        projection: &store::SideEffectArtifactProjection,
    ) -> mfm_runtime::Result<(EvmContractRuntime, PreparedContractMutation)> {
        let stored_prepared = self
            .load_prepared_invocation(submit_node, projection)
            .await?;
        let (plan, runtime) = self
            .load_plan_and_runtime(submit_node, submit_inputs)
            .await?;
        let prepared = plan.reconstruct_prepared_invocation(&runtime, &stored_prepared)?;
        Ok((runtime, prepared))
    }
}

impl<P> SideEffectVerifyCallbacks for ContractVerifyCallbacks<'_, P>
where
    P: ContractVerifyPhase,
{
    type Submission = ContractTransactionSubmissions;
    type Receipt = P::Receipt;
    type Confirmation = P::Confirmation;
    type Output = P::Output;
    type NotSubmittedProof = ContractNotSubmittedProof;
    type AmbiguityEvidence = ContractTransactionSubmissions;

    fn recover_unknown_submission<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
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
            let (runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
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
            let (runtime, prepared) = self
                .load_reconstructed_prepared_invocation(
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
                evidence: P::receipt_from_observed(prepared.evidence(), receipts)?,
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
                .load_plan_and_runtime(submit_node, submit_inputs)
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
            let plan = self.load_plan(submit_node, submit_inputs).await?;
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
            let plan = self.load_plan(submit_node, submit_inputs).await?;
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
    type Output: MfmValue + Send + Sync + 'static;

    fn receipt_from_observed(
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt>;

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt;

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt];

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation;

    fn output_from_receipt(&self, receipt: &Self::Receipt) -> mfm_runtime::Result<Self::Output>;

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<Self::Output>;
}

impl ContractVerifyPhase for DeployMutationPlan {
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;
    type Output = DeployedContract;

    fn receipt_from_observed(
        prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContractDeployReceipt {
            receipt_version: 1,
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
            confirmations,
            contract_address: receipt.contract_address,
            receipt: receipt.receipt,
        }
    }

    fn output_from_receipt(&self, receipt: &Self::Receipt) -> mfm_runtime::Result<Self::Output> {
        self.state
            .output_from_receipt(&(), &self.intent, receipt)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<Self::Output> {
        self.state
            .output_from_confirmation(&(), &self.intent, confirmation)
            .map_err(runtime_state_error)
    }
}

impl ContractVerifyPhase for ConfigureMutationPlan {
    type Receipt = ContractConfigureReceipt;
    type Confirmation = ContractConfigureConfirmation;
    type Output = ConfiguredContract;

    fn receipt_from_observed(
        _prepared: &PreparedContractInvocation,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> mfm_runtime::Result<Self::Receipt> {
        Ok(ContractConfigureReceipt {
            receipt_version: 1,
            configured_block_number: receipts.iter().map(|receipt| receipt.block_number).max(),
            receipts,
        })
    }

    fn receipt_with_evidence(
        receipt: Self::Receipt,
        evidence: &CapabilityArtifactEvidenceRef,
    ) -> Self::Receipt {
        configure_receipt_with_evidence(receipt, evidence)
    }

    fn receipt_transactions(receipt: &Self::Receipt) -> &[ContractTransactionReceipt] {
        &receipt.receipts
    }

    fn confirmation_from_receipt(receipt: Self::Receipt, confirmations: u64) -> Self::Confirmation {
        ContractConfigureConfirmation {
            confirmation_version: 1,
            confirmations,
            configured_block_number: receipt.configured_block_number,
            receipts: receipt.receipts,
        }
    }

    fn output_from_receipt(&self, receipt: &Self::Receipt) -> mfm_runtime::Result<Self::Output> {
        self.state
            .output_from_receipt(&self.input, &self.intent, receipt)
            .map_err(runtime_state_error)
    }

    fn output_from_confirmation(
        &self,
        confirmation: &Self::Confirmation,
    ) -> mfm_runtime::Result<Self::Output> {
        self.state
            .output_from_confirmation(&self.input, &self.intent, confirmation)
            .map_err(runtime_state_error)
    }
}

async fn deploy_mutation_plan_for_node(
    node: &spec::NodeSpec,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<DeployMutationPlan> {
    let config = load_runner_config_for_node::<DeployPhaseConfig>(node, artifacts).await?;
    let state = DeployContractState::new(config.clone()).map_err(runtime_plan_error)?;
    let intent = state.prepare_intent(&()).map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&(), &intent)
        .map_err(runtime_state_error)?;
    Ok(DeployMutationPlan {
        config,
        state,
        intent,
        idempotency,
    })
}

async fn configure_mutation_plan_for_inputs(
    node: &spec::NodeSpec,
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ConfigureMutationPlan> {
    let config = load_runner_config_for_node::<ConfigurePhaseConfig>(node, artifacts).await?;
    let deployed =
        load_materialized_struct_field_value::<DeployedContract>(inputs, "deployed", artifacts)
            .await?;
    let input = ConfigureContractInput { deployed };
    let state = ConfigureContractState::new(config.clone()).map_err(runtime_plan_error)?;
    let intent = state.prepare_intent(&input).map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&input, &intent)
        .map_err(runtime_state_error)?;
    Ok(ConfigureMutationPlan {
        config,
        state,
        input,
        intent,
        idempotency,
    })
}

async fn run_validate(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let config = load_runner_config::<ValidatePhaseConfig>(&ctx, factory.artifacts()).await?;
    let input = load_validate_input(ctx.inputs(), factory.artifacts()).await?;
    let state = ValidateContractState::new(config.clone())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let request = state
        .read_request(&input)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let runtime = factory.read_runtime_for(config.as_ref().network().network_id())?;
    let response = runtime.validate_contract(&config, &input, &request).await?;
    let report = state
        .report_from_response(&input, response)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    ErasedRunnerOutput::state_output(&ctx, &report)
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

async fn load_validate_input(
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<ValidateContractInput> {
    let configured =
        load_materialized_struct_field_value::<ConfiguredContract>(inputs, "configured", artifacts)
            .await?;
    Ok(ValidateContractInput { configured })
}

fn decode_replay_config<T>(bytes: &[u8]) -> Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let config = serde_json::from_slice::<T>(bytes)
        .map_err(|error| EvmContractAdapterError::Config(error.to_string()))?;
    ValidatedConfig::new(config).map_err(|error| EvmContractAdapterError::Config(error.to_string()))
}

fn config_schema<T>() -> Result<SchemaId>
where
    T: MfmConfig,
{
    T::schema_id().map_err(|error| EvmContractAdapterError::Config(error.to_string()))
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

fn configure_receipt_with_evidence(
    mut receipt: ContractConfigureReceipt,
    evidence: &mfm_artifact_capabilities::ArtifactEvidenceRef,
) -> ContractConfigureReceipt {
    let evidence = lifecycle_evidence_ref(evidence);
    for transaction_receipt in &mut receipt.receipts {
        transaction_receipt.receipt_evidence = Some(evidence.clone());
    }
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

fn evm_chain_guard_for_network(network: &EvmNetworkIntent) -> Result<EvmChainGuard> {
    evm_chain_guard(network.network_id(), network.expected_chain_id())
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
    /// Required contract artifact was absent.
    #[error("contract artifact is required for this lifecycle phase")]
    MissingContractArtifact,
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
