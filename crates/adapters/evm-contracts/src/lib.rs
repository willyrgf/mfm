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
//! use mfm_adapters_evm_contracts::{
//!     EvmContractRuntimeRoute, replay_verifier_id,
//! };
//! use mfm_evm_capabilities::{EvmSourcePolicyId, EvmSourceRef};
//!
//! let route = EvmContractRuntimeRoute::new(
//!     EvmSourceRef::new("local")?,
//!     EvmSourcePolicyId::new("dev")?,
//! );
//! assert_eq!(route.source_ref().as_str(), "local");
//! assert_eq!(replay_verifier_id()?.as_str(), "mfm.evm.contract.replay.v1");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{keccak256, Address, B256};
use mfm_adapter_contracts::evm_contract_lifecycle_adapter_binding;
use mfm_artifact_capabilities::{
    ArtifactEvidenceRef as CapabilityArtifactEvidenceRef, ArtifactReadProvider, ArtifactReadRequest,
};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::CapabilitySpec;
use mfm_events::v1::{self as events, side_effect};
use mfm_evm_capabilities::{
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockSelector, EvmCallReadCapability,
    EvmCallReadProvider, EvmCallReadRequest, EvmCapabilityError, EvmChainIdentityCapability,
    EvmChainIdentityProvider, EvmChainIdentityRequest, EvmFeeReadProvider, EvmFeeReadRequest,
    EvmGasEstimateProvider, EvmGasEstimateRequest, EvmLogsReadCapability, EvmLogsReadProvider,
    EvmLogsReadRequest, EvmNonceOccupancy, EvmNonceOccupancyReadProvider,
    EvmNonceOccupancyReadRequest, EvmNonceReadProvider, EvmNonceReadRequest,
    EvmReceiptReadProvider, EvmReceiptReadRequest, EvmSourcePolicyId, EvmSourceRef,
    EvmTransactionSubmitCapability, EvmTransactionSubmitProvider, EvmTransactionSubmitRequest,
    SignedEvmPayload,
};
use mfm_evm_contract_config::{
    ConfigurePhaseConfig, DeployPhaseConfig, EvmTransactionPolicy,
    EvmTransactionStyle as ConfigTransactionStyle, ReceiptRetryPolicy, ValidatePhaseConfig,
};
use mfm_evm_contract_model::{
    constructor_data, decode_single_output_to_json, expected_matches, hex_to_bytes,
    normalize_address, parse_artifact, prepare_validate_assertions, resolve_function_call,
    ConfiguredContract, ContractCallConfig, DeployedContract, EventAssertionConfig, ExpectedValue,
    LifecycleArtifactEvidenceRef, ReadAssertionConfig, ValidationEventResult, ValidationReadResult,
};
use mfm_evm_core::hex::bytes_to_hex_prefixed;
use mfm_evm_core::rlp::{rlp_encode_list, u64_to_min_be};
use mfm_evm_core::tx::{parse_address, parse_u128_quantity, Eip1559TxToSign, LegacyTxToSign};
use mfm_evm_signing::EvmSigningRequest;
use mfm_ids::{
    CapabilityKind, CapabilityVersion, ContentDigest, DigestAlgorithm, NodeId, SchemaId,
};
use mfm_program::{SideEffectState, StateSpec, ValidatedConfig};
use mfm_program_derive::MfmValue;
use mfm_replay::v1 as replay;
use mfm_runtime::{
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerFuture,
    ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCell, MaterializedCellTerminal,
    MaterializedInputNode, MaterializedInputs, PreInvocationRunCtx, PreInvocationRunnerFuture,
    RunnerArtifactBuilder, RunnerCapabilityBinding, RunnerIngressContext, RunnerOutputBuilder,
    RunnerPayloadBuilder, RunnerRegistrationBuilder, SideEffectDriver, SideEffectDriverCallbacks,
    SideEffectDriverFuture, SideEffectIntentPlan, SideEffectLanePreclaimBuilder,
    SideEffectObservedEvidence, SideEffectPreparedInvocationPlan, SideEffectProtocolAction,
    SideEffectReplayEvidence, SideEffectSubmissionDecision, SideEffectSubmissionDecisionFuture,
    SideEffectVerifyCallbacks, SideEffectVerifyDriver,
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

/// Process-local EVM source route used by adapter runners.
///
/// The route is intentionally not serializable and must be supplied by the
/// process wiring live capability providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmContractRuntimeRoute {
    source_ref: EvmSourceRef,
    policy_id: EvmSourcePolicyId,
}

impl EvmContractRuntimeRoute {
    /// Creates a process-local route.
    pub fn new(source_ref: EvmSourceRef, policy_id: EvmSourcePolicyId) -> Self {
        Self {
            source_ref,
            policy_id,
        }
    }

    /// Returns the selected source reference.
    pub const fn source_ref(&self) -> &EvmSourceRef {
        &self.source_ref
    }

    /// Returns the selected source policy id.
    pub const fn policy_id(&self) -> &EvmSourcePolicyId {
        &self.policy_id
    }
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

impl fmt::Debug for EvmContractReadProviders<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContractReadProviders")
            .finish_non_exhaustive()
    }
}

/// Prepared invocation evidence retained before a contract mutation is started.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
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
    pub fn new(
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
    route: EvmContractRuntimeRouteRef<'a>,
    mutation: EvmContractMutationProviders<'a>,
    reads: EvmContractReadProviders<'a>,
}

#[derive(Debug, Clone, Copy)]
struct EvmContractRuntimeRouteRef<'a> {
    route: &'a EvmContractRuntimeRoute,
}

impl<'a> EvmContractLifecycleAdapter<'a> {
    /// Creates an adapter from process-local routing and provider sets.
    pub fn new(
        route: &'a EvmContractRuntimeRoute,
        mutation: EvmContractMutationProviders<'a>,
        reads: EvmContractReadProviders<'a>,
    ) -> Self {
        Self {
            route: EvmContractRuntimeRouteRef { route },
            mutation,
            reads,
        }
    }

    /// Returns the process-local route used by this adapter.
    pub const fn route(&self) -> &EvmContractRuntimeRoute {
        self.route.route
    }

    /// Reads the latest block number for mutation finality checks.
    pub async fn latest_block_number(&self, expected_chain_id: u64) -> Result<u64> {
        let response = self
            .mutation
            .block
            .read_block(&EvmBlockReadRequest {
                source_ref: self.route.route.source_ref().clone(),
                policy_id: self.route.route.policy_id().clone(),
                block: EvmBlockSelector::Latest,
            })
            .await?;
        if response.evidence.chain_id != expected_chain_id {
            return Err(EvmContractAdapterError::ChainMismatch);
        }
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
        self.prepare_transactions(PrepareTransactionsRequest {
            phase: ContractMutationPhase::Deploy,
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
            policy: config.transaction(),
            poll_interval_ms: config.receipt().poll_interval_ms(),
            max_receipt_polls: config.receipt().max_receipt_polls(),
            tx_inputs: vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(config.value_wei())?,
                data,
            }],
        })
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
        self.prepare_transactions(PrepareTransactionsRequest {
            phase: ContractMutationPhase::Configure,
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
            policy: config.transaction(),
            poll_interval_ms: config.receipt().poll_interval_ms(),
            max_receipt_polls: config.receipt().max_receipt_polls(),
            tx_inputs,
        })
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
        reconstruct_prepared_mutation(PreparedMutationReconstruction {
            evidence,
            phase: ContractMutationPhase::Deploy,
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
            tx_inputs: vec![PreparedTransactionInput {
                to: None,
                value_wei: parse_optional_wei(config.value_wei())?,
                data: deploy_data(config)?,
            }],
        })
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
        reconstruct_prepared_mutation(PreparedMutationReconstruction {
            evidence,
            phase: ContractMutationPhase::Configure,
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
            tx_inputs: configure_transaction_inputs(config, &input.deployed.contract_address)?,
        })
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
                .submit_transaction(&EvmTransactionSubmitRequest {
                    source_ref: self.route().source_ref().clone(),
                    policy_id: self.route().policy_id().clone(),
                    signed_payload: payload,
                })
                .await?;
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
        submissions: &[ContractTransactionSubmission],
    ) -> Result<Vec<ContractTransactionReceipt>> {
        let mut receipts = Vec::with_capacity(submissions.len());
        for submission in submissions {
            let transaction_hash = submission
                .transaction_hash
                .parse::<B256>()
                .map_err(|_| EvmContractAdapterError::TransactionHashMismatch)?;
            let response = self
                .mutation
                .receipt
                .read_receipt(&EvmReceiptReadRequest {
                    source_ref: self.route().source_ref().clone(),
                    policy_id: self.route().policy_id().clone(),
                    transaction_hash,
                })
                .await?;
            if response.transaction_hash != transaction_hash {
                return Err(EvmContractAdapterError::TransactionHashMismatch);
            }
            if !response.status {
                return Err(EvmContractAdapterError::TransactionFailed);
            }
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
                    source_ref: self.route().source_ref().clone(),
                    policy_id: self.route().policy_id().clone(),
                    transaction_hash,
                })
                .await
            {
                Ok(response) => {
                    if response.transaction_hash != transaction_hash {
                        return Err(EvmContractAdapterError::TransactionHashMismatch);
                    }
                    if !response.status {
                        return Err(EvmContractAdapterError::TransactionFailed);
                    }
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
                source_ref: self.route().source_ref().clone(),
                policy_id: self.route().policy_id().clone(),
                account: expected_signer,
                block: EvmBlockSelector::Pending,
            })
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
                .read_nonce_occupancy(&EvmNonceOccupancyReadRequest {
                    source_ref: self.route().source_ref().clone(),
                    policy_id: self.route().policy_id().clone(),
                    account: expected_signer,
                    nonce: transaction.nonce,
                    excluded_transaction_hash: parse_prepared_transaction_hash(
                        &transaction.expected_transaction_hash,
                    )?,
                })
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
                                evidence_chain_id: response.evidence.chain_id,
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

    /// Executes validation reads and projects a validation response.
    pub async fn validate_contract(
        &self,
        config: &ValidatedConfig<ValidatePhaseConfig>,
        input: &ValidateContractInput,
        request: &ContractValidationReadRequest,
    ) -> Result<ContractValidationReadResponse> {
        let expected = ValidateContractState::new(config.clone())?.read_request(input)?;
        if &expected != request {
            return Err(EvmContractAdapterError::IntentMismatch);
        }
        let config = config.as_ref();
        let chain = self
            .reads
            .chain_identity
            .chain_identity(&EvmChainIdentityRequest {
                source_ref: self.route().source_ref().clone(),
                policy_id: self.route().policy_id().clone(),
            })
            .await?;
        if chain.chain_id != request.expected_chain_id {
            return Err(EvmContractAdapterError::ChainMismatch);
        }

        let (configuration_read_results, configuration_event_results) = self
            .evaluate_assertions(
                config.artifact(),
                &input.configured.confirmation_read_assertions,
                &input.configured.confirmation_event_assertions,
                &input.configured.deployed.contract_address,
            )
            .await?;
        let (read_results, event_results) = self
            .evaluate_assertions(
                config.artifact(),
                &request.read_assertions,
                &request.event_assertions,
                &input.configured.deployed.contract_address,
            )
            .await?;

        Ok(ContractValidationReadResponse {
            response_version: 1,
            observed_chain_id: chain.chain_id,
            client_version: chain.client_version.unwrap_or_else(|| "unknown".to_owned()),
            configuration_read_results,
            configuration_event_results,
            read_results,
            event_results,
        })
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
        let chain = self
            .mutation
            .chain_identity
            .chain_identity(&EvmChainIdentityRequest {
                source_ref: self.route().source_ref().clone(),
                policy_id: self.route().policy_id().clone(),
            })
            .await?;
        if chain.chain_id != expected_chain_id {
            return Err(EvmContractAdapterError::ChainMismatch);
        }

        let nonce = self
            .mutation
            .nonce
            .read_nonce(&EvmNonceReadRequest {
                source_ref: self.route().source_ref().clone(),
                policy_id: self.route().policy_id().clone(),
                account: expected_signer,
                block: EvmBlockSelector::Latest,
            })
            .await?
            .nonce;
        let fees = self
            .mutation
            .fee
            .read_fee(&EvmFeeReadRequest {
                source_ref: self.route().source_ref().clone(),
                policy_id: self.route().policy_id().clone(),
            })
            .await?;

        let mut evidence = Vec::with_capacity(tx_inputs.len());
        let mut signing_requests = Vec::with_capacity(tx_inputs.len());
        for (index, input) in tx_inputs.into_iter().enumerate() {
            let gas_limit = match policy.gas_limit() {
                Some(gas_limit) => gas_limit,
                None => {
                    self.mutation
                        .gas
                        .estimate_gas(&EvmGasEstimateRequest {
                            source_ref: self.route().source_ref().clone(),
                            policy_id: self.route().policy_id().clone(),
                            from: Some(expected_signer),
                            to: input.to,
                            value_wei: input.value_wei,
                            data: input.data.clone(),
                        })
                        .await?
                        .gas_limit
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

    async fn evaluate_assertions(
        &self,
        artifact: Option<&mfm_evm_contract_model::ContractArtifactConfig>,
        read_assertions: &[ReadAssertionConfig],
        event_assertions: &[EventAssertionConfig],
        contract_address: &str,
    ) -> Result<(Vec<ValidationReadResult>, Vec<ValidationEventResult>)> {
        if read_assertions.is_empty() && event_assertions.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let artifact = artifact.ok_or(EvmContractAdapterError::MissingContractArtifact)?;
        let (abi, _) = parse_artifact(artifact).map_err(EvmContractAdapterError::Model)?;
        let (prepared_reads, prepared_events) =
            prepare_validate_assertions(&abi, read_assertions, event_assertions)
                .map_err(EvmContractAdapterError::Model)?;
        let address = parse_address(contract_address, "contract_address")
            .map_err(|error| EvmContractAdapterError::Model(error.message))?;

        let mut read_results = Vec::with_capacity(prepared_reads.len());
        for (assertion, prepared) in read_assertions.iter().zip(prepared_reads.iter()) {
            let response = self
                .reads
                .call
                .read_call(&EvmCallReadRequest {
                    source_ref: self.route().source_ref().clone(),
                    policy_id: self.route().policy_id().clone(),
                    to: address,
                    calldata: hex_to_bytes(&prepared.data_hex)
                        .map_err(EvmContractAdapterError::Model)?,
                    block: EvmBlockSelector::Latest,
                })
                .await?;
            let actual_json = decode_single_output_to_json(
                &prepared.outputs,
                &bytes_to_hex_prefixed(&response.return_data),
            )
            .map_err(EvmContractAdapterError::Model)?;
            let actual = ExpectedValue::from_json_value(&actual_json)
                .map_err(EvmContractAdapterError::Model)?;
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
            let logs = self
                .reads
                .logs
                .read_logs(&EvmLogsReadRequest {
                    source_ref: self.route().source_ref().clone(),
                    policy_id: self.route().policy_id().clone(),
                    from_block: block_selector(assertion.from_block.as_ref(), false),
                    to_block: block_selector(assertion.to_block.as_ref(), true),
                    address: Some(address),
                    topics: vec![topic],
                })
                .await?;
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
    let json = serde_json::to_string(prepared)
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)?;
    let json = json.to_ascii_lowercase();
    for forbidden in forbidden_prepared_terms() {
        if json.contains(&forbidden) {
            return Err(EvmContractAdapterError::PreparedInvocationLeak);
        }
    }
    Ok(())
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
    value
        .parse::<B256>()
        .map_err(|_| EvmContractAdapterError::InvalidPreparedInvocation)
}

fn prepared_anchor_submissions(
    prepared: &PreparedContractInvocation,
) -> Result<ContractTransactionSubmissions> {
    ensure_prepared_invocation_public(prepared)?;
    let mut transactions = Vec::with_capacity(prepared.transactions.len());
    for transaction in &prepared.transactions {
        parse_prepared_transaction_hash(&transaction.expected_transaction_hash)?;
        transactions.push(ContractTransactionSubmission {
            submission_version: 1,
            transaction_hash: transaction.expected_transaction_hash.clone(),
            signer_public_key: None,
        });
    }
    Ok(ContractTransactionSubmissions {
        submissions_version: 1,
        transactions,
    })
}

enum PreparedSubmissionReconciliation {
    Observed(ContractTransactionSubmissions),
    NotObserved(ContractTransactionSubmissions),
    NotSubmitted(ContractNotSubmittedProof),
    Indeterminate(ContractTransactionSubmissions),
}

async fn submit_or_recover_contract_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
    action: SideEffectProtocolAction,
) -> Result<
    SideEffectSubmissionDecision<
        ContractTransactionSubmissions,
        ContractTransactionSubmissions,
        ContractNotSubmittedProof,
        ContractTransactionSubmissions,
    >,
> {
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
                return match runtime.adapter().submit_prepared(prepared).await {
                    Ok(transactions) => Ok(SideEffectSubmissionDecision::Observed(
                        ContractTransactionSubmissions {
                            submissions_version: 1,
                            transactions,
                        },
                    )),
                    Err(EvmContractAdapterError::TransactionHashMismatch) => {
                        Ok(SideEffectSubmissionDecision::Unknown(anchor_submissions))
                    }
                    Err(error) => Err(error),
                };
            }
        }
    }

    Ok(SideEffectSubmissionDecision::Observed(
        ContractTransactionSubmissions {
            submissions_version: 1,
            transactions: runtime.adapter().submit_prepared(prepared).await?,
        },
    ))
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
) -> EvmBlockSelector {
    match selector {
        Some(mfm_evm_contract_model::BlockSelector::Number { number }) => {
            EvmBlockSelector::Number(*number)
        }
        Some(mfm_evm_contract_model::BlockSelector::Tag { .. }) if default_latest => {
            EvmBlockSelector::Latest
        }
        Some(mfm_evm_contract_model::BlockSelector::Tag { .. }) | None if default_latest => {
            EvmBlockSelector::Latest
        }
        Some(mfm_evm_contract_model::BlockSelector::Tag { .. }) | None => {
            EvmBlockSelector::Number(0)
        }
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

fn forbidden_prepared_terms() -> Vec<String> {
    vec![
        ["raw", "_transaction"].concat(),
        ["raw", "_tx"].concat(),
        ["signed", "_payload"].concat(),
        "signature".to_owned(),
        ["key", "store"].concat(),
        ["key", "store", "_path"].concat(),
        ["private", "_key"].concat(),
        ["private", " key"].concat(),
        ["pass", "word"].concat(),
        ["mne", "monic"].concat(),
        ["seed", "_phrase"].concat(),
        ["rpc", "_url"].concat(),
        ["rpc", " url"].concat(),
        ["end", "point"].concat(),
        ["provider", "_kind"].concat(),
        ["provider", " kind"].concat(),
        ["author", "ization"].concat(),
    ]
}

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
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

/// Factory for per-network EVM contract runtime bindings.
pub trait EvmContractRuntimeFactory: Send + Sync {
    /// Returns the artifact reader used to materialize configs, inputs, and side-effect evidence.
    fn artifacts(&self) -> &dyn ArtifactReadProvider;

    /// Validates process-local runtime bindings for launch ingress before `RunAdmitted`.
    fn validate_runtime_for(
        &self,
        network_id: &str,
        signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        let _ = signer_ref;
        self.runtime_for(network_id).map(|_| ())
    }

    /// Returns process-local EVM and signing capabilities for `network_id`.
    fn runtime_for(&self, network_id: &str) -> mfm_runtime::Result<EvmContractRuntime>;
}

/// Process-local runtime capability set for one EVM contract lifecycle route.
#[derive(Clone)]
pub struct EvmContractRuntime {
    route: EvmContractRuntimeRoute,
    evm: Arc<dyn EvmContractProvider>,
    signer: Arc<dyn SigningProvider>,
}

impl fmt::Debug for EvmContractRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContractRuntime")
            .field("route", &self.route)
            .finish_non_exhaustive()
    }
}

impl EvmContractRuntime {
    /// Creates an EVM contract runtime from explicit process-local providers.
    pub fn new(
        route: EvmContractRuntimeRoute,
        evm: Arc<dyn EvmContractProvider>,
        signer: Arc<dyn SigningProvider>,
    ) -> Self {
        Self { route, evm, signer }
    }

    fn adapter(&self) -> EvmContractLifecycleAdapter<'_> {
        let evm = self.evm.as_ref();
        EvmContractLifecycleAdapter::new(
            &self.route,
            EvmContractMutationProviders {
                chain_identity: evm,
                block: evm,
                nonce: evm,
                fee: evm,
                gas: evm,
                signer: self.signer.as_ref(),
                submit: evm,
                receipt: evm,
                nonce_occupancy: evm,
            },
            EvmContractReadProviders {
                chain_identity: evm,
                call: evm,
                logs: evm,
            },
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
    let side_effect_factory = events::RunnerFactoryId::new(SIDE_EFFECT_FACTORY)?;
    let read_factory = events::RunnerFactoryId::new(READ_FACTORY)?;
    let deploy = mfm_program::registered_state_descriptor::<DeployContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_descriptor(
        deploy.descriptor_id().clone(),
        deploy.capabilities(),
        side_effect_factory.clone(),
        executable(side_effect_factory.clone())?,
        Arc::new(ContractMutationRunner {
            phase: ContractMutationRunnerPhase::Deploy,
            factory: factory.clone(),
        }),
    )?;
    let configure = mfm_program::registered_state_descriptor::<ConfigureContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_descriptor(
        configure.descriptor_id().clone(),
        configure.capabilities(),
        side_effect_factory.clone(),
        executable(side_effect_factory)?,
        Arc::new(ContractMutationRunner {
            phase: ContractMutationRunnerPhase::Configure,
            factory: factory.clone(),
        }),
    )?;
    let validate = mfm_program::registered_state_descriptor::<ValidateContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    registrations.register_descriptor(
        validate.descriptor_id().clone(),
        validate.capabilities(),
        read_factory.clone(),
        executable(read_factory.clone())?,
        Arc::new(ContractValidateRunner {
            factory: factory.clone(),
        }),
    )?;
    registrations.register_side_effect_verify_runner(
        read_factory.clone(),
        executable(read_factory)?,
        Arc::new(ContractVerifyRunner { factory }),
    )?;
    Ok(())
}

fn executable(
    factory_id: events::RunnerFactoryId,
) -> mfm_runtime::Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id,
        cargo_package_digest: digest_json(serde_json::json!({
            "crate": "mfm-adapters-evm-contracts",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        binary_digest: digest_json(serde_json::json!({
            "crate": "mfm-adapters-evm-contracts",
            "runner": "evm-contract-lifecycle",
            "version": env!("CARGO_PKG_VERSION"),
        }))?,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
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

#[derive(Clone, Copy)]
enum ContractMutationRunnerPhase {
    Deploy,
    Configure,
}

struct ContractMutationRunner {
    phase: ContractMutationRunnerPhase,
    factory: Arc<dyn EvmContractRuntimeFactory>,
}

impl ErasedNodeRunner for ContractMutationRunner {
    fn validate_ingress(&self, ctx: RunnerIngressContext<'_>) -> mfm_runtime::Result<()> {
        validate_mutation_runtime_for_node(&ctx, ctx.node(), self.factory.as_ref())
    }

    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(
            async move { preclaim_mutation_lane(ctx, self.factory.as_ref(), self.phase).await },
        )
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_mutation(ctx, self.factory.as_ref(), self.phase).await })
    }
}

async fn run_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
    phase: ContractMutationRunnerPhase,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    match phase {
        ContractMutationRunnerPhase::Deploy => run_deploy_mutation(ctx, factory).await,
        ContractMutationRunnerPhase::Configure => run_configure_mutation(ctx, factory).await,
    }
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
        ContractMutationRunnerPhase::Deploy => {
            let callbacks = DeploySideEffectVerifyCallbacks { factory };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
        ContractMutationRunnerPhase::Configure => {
            let callbacks = ConfigureSideEffectVerifyCallbacks { factory };
            SideEffectVerifyDriver::drive(ctx, &callbacks).await
        }
    }
}

fn side_effect_verify_submit_node<'a>(
    ctx: &'a ErasedRunCtx<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &ctx.node().framework else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a side-effect verify node",
            ctx.node().node_id
        )));
    };
    ctx.certified_node(&verify.submit_node_id).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "side-effect verify node {} references missing submit node {}",
            ctx.node().node_id,
            verify.submit_node_id
        ))
    })
}

fn side_effect_verify_submit_node_for_ingress<'a>(
    ctx: &'a RunnerIngressContext<'a>,
) -> mfm_runtime::Result<&'a spec::NodeSpec> {
    let Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) = &ctx.node().framework else {
        return Err(mfm_runtime::RuntimeError::RunnerBinding(format!(
            "node {} is not a side-effect verify node",
            ctx.node().node_id
        )));
    };
    ctx.runtime_spec()
        .node(&verify.submit_node_id)
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::RunnerBinding(format!(
                "side-effect verify node {} references missing submit node {}",
                ctx.node().node_id,
                verify.submit_node_id
            ))
        })
}

fn mutation_phase_for_submit_node(
    node: &spec::NodeSpec,
) -> mfm_runtime::Result<ContractMutationRunnerPhase> {
    let deploy = mfm_program::registered_state_descriptor::<DeployContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == deploy.descriptor_id() {
        return Ok(ContractMutationRunnerPhase::Deploy);
    }
    let configure = mfm_program::registered_state_descriptor::<ConfigureContractState>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    if &node.descriptor_id == configure.descriptor_id() {
        return Ok(ContractMutationRunnerPhase::Configure);
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
        ContractMutationRunnerPhase::Deploy => {
            let config = load_launch_config_for_node::<DeployPhaseConfig>(ctx, node)?;
            let signer_ref = config
                .as_ref()
                .signer()
                .signer_ref()
                .map_err(runtime_ingress_model_error)?;
            factory
                .validate_runtime_for(config.as_ref().network().network_id(), Some(&signer_ref))
                .map(|_| ())
        }
        ContractMutationRunnerPhase::Configure => {
            let config = load_launch_config_for_node::<ConfigurePhaseConfig>(ctx, node)?;
            let signer_ref = config
                .as_ref()
                .signer()
                .signer_ref()
                .map_err(runtime_ingress_model_error)?;
            factory
                .validate_runtime_for(config.as_ref().network().network_id(), Some(&signer_ref))
                .map(|_| ())
        }
    }
}

async fn preclaim_mutation_lane(
    ctx: &PreInvocationRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
    phase: ContractMutationRunnerPhase,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    match phase {
        ContractMutationRunnerPhase::Deploy => {
            let plan = deploy_mutation_plan_for_node(ctx.node(), factory.artifacts()).await?;
            let resource_key = mutation_resource_key(
                ctx.node(),
                plan.config.as_ref().network().expected_chain_id(),
                plan.config.as_ref().signer().expected_signer_address_str(),
            )?;
            SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
                &plan.intent,
                &plan.idempotency,
                idempotency_key_ref(&plan.idempotency)?,
                evm_transaction_submit_binding()?,
                resource_key,
            )
        }
        ContractMutationRunnerPhase::Configure => {
            let plan =
                configure_mutation_plan_for_inputs(ctx.node(), ctx.inputs(), factory.artifacts())
                    .await?;
            let resource_key = mutation_resource_key(
                ctx.node(),
                plan.config.as_ref().network().expected_chain_id(),
                plan.config.as_ref().signer().expected_signer_address_str(),
            )?;
            SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
                &plan.intent,
                &plan.idempotency,
                idempotency_key_ref(&plan.idempotency)?,
                evm_transaction_submit_binding()?,
                resource_key,
            )
        }
    }
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

async fn run_deploy_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let plan = deploy_mutation_plan(&ctx, factory.artifacts()).await?;
    let callbacks = ContractMutationSideEffectCallbacks { factory, plan };
    SideEffectDriver::drive(ctx, &callbacks).await
}

struct ContractMutationSideEffectCallbacks<'a, P> {
    factory: &'a dyn EvmContractRuntimeFactory,
    plan: P,
}

trait ContractMutationPlanOps: Send + Sync {
    type Intent: MfmValue + Clone + Send + Sync + 'static;

    fn intent(&self) -> &Self::Intent;

    fn idempotency(&self) -> &ContractTransactionIdempotency;

    fn network_id(&self) -> &str;

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

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn network_id(&self) -> &str {
        self.config.as_ref().network().network_id()
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

    fn intent(&self) -> &Self::Intent {
        &self.intent
    }

    fn idempotency(&self) -> &ContractTransactionIdempotency {
        &self.idempotency
    }

    fn network_id(&self) -> &str {
        self.config.as_ref().network().network_id()
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
        Box::pin(async {
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
        Box::pin(async {
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
        Box::pin(async {
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

async fn run_configure_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let plan = configure_mutation_plan(&ctx, factory.artifacts()).await?;
    let callbacks = ContractMutationSideEffectCallbacks { factory, plan };
    SideEffectDriver::drive(ctx, &callbacks).await
}

struct DeploySideEffectVerifyCallbacks<'a> {
    factory: &'a dyn EvmContractRuntimeFactory,
}

impl SideEffectVerifyCallbacks for DeploySideEffectVerifyCallbacks<'_> {
    type Receipt = ContractDeployReceipt;
    type Confirmation = ContractDeployConfirmation;
    type Output = DeployedContract;

    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        Box::pin(async {
            let plan = deploy_mutation_plan_for_node(submit_node, self.factory.artifacts()).await?;
            let (prepared, submissions) =
                load_verify_prepared_and_submission(ctx, submit_node, submission, self.factory)
                    .await?;
            let runtime = self
                .factory
                .runtime_for(plan.config.as_ref().network().network_id())?;
            let receipts = read_receipts_with_poll(&runtime, &prepared, &submissions).await?;
            Ok(SideEffectObservedEvidence {
                evidence: ContractDeployReceipt {
                    receipt_version: 1,
                    contract_address: deploy_contract_address_from_prepared(&prepared)?,
                    receipt: single_receipt(receipts)?,
                },
                replay: contract_side_effect_replay_evidence()?,
            })
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        Box::pin(async {
            let plan = deploy_mutation_plan_for_node(submit_node, self.factory.artifacts()).await?;
            let runtime = self
                .factory
                .runtime_for(plan.config.as_ref().network().network_id())?;
            let required_depth = finalized_depth_for_submit_node(submit_node)?;
            let (receipt, receipt_evidence) = load_side_effect_artifact::<ContractDeployReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            let receipt = deploy_receipt_with_evidence(receipt, &receipt_evidence);
            let confirmations = verified_finality_confirmations(
                &runtime,
                plan.config.as_ref().network().expected_chain_id(),
                std::slice::from_ref(&receipt.receipt),
                required_depth,
            )
            .await?;
            Ok(SideEffectObservedEvidence {
                evidence: ContractDeployConfirmation {
                    confirmation_version: 1,
                    confirmations,
                    contract_address: receipt.contract_address,
                    receipt: receipt.receipt,
                },
                replay: contract_side_effect_replay_evidence()?,
            })
        })
    }

    fn map_receipt_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = deploy_mutation_plan_for_node(submit_node, self.factory.artifacts()).await?;
            let receipt = load_side_effect_value::<ContractDeployReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            plan.state
                .output_from_receipt(&(), &plan.intent, &receipt)
                .map_err(runtime_state_error)
        })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        Box::pin(async {
            let plan = deploy_mutation_plan_for_node(submit_node, self.factory.artifacts()).await?;
            let confirmation = load_side_effect_value::<ContractDeployConfirmation>(
                confirmation,
                events::ArtifactRole::Confirmation,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            plan.state
                .output_from_confirmation(&(), &plan.intent, &confirmation)
                .map_err(runtime_state_error)
        })
    }
}

struct ConfigureSideEffectVerifyCallbacks<'a> {
    factory: &'a dyn EvmContractRuntimeFactory,
}

impl SideEffectVerifyCallbacks for ConfigureSideEffectVerifyCallbacks<'_> {
    type Receipt = ContractConfigureReceipt;
    type Confirmation = ContractConfigureConfirmation;
    type Output = ConfiguredContract;

    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        submit_node: &'a spec::NodeSpec,
        submit_inputs: &'a MaterializedInputs,
        submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        Box::pin(async {
            let plan = configure_mutation_plan_for_inputs(
                submit_node,
                submit_inputs,
                self.factory.artifacts(),
            )
            .await?;
            let (prepared, submissions) =
                load_verify_prepared_and_submission(ctx, submit_node, submission, self.factory)
                    .await?;
            let runtime = self
                .factory
                .runtime_for(plan.config.as_ref().network().network_id())?;
            let receipts = read_receipts_with_poll(&runtime, &prepared, &submissions).await?;
            Ok(SideEffectObservedEvidence {
                evidence: ContractConfigureReceipt {
                    receipt_version: 1,
                    configured_block_number: receipts
                        .iter()
                        .map(|receipt| receipt.block_number)
                        .max(),
                    receipts,
                },
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
            let plan = configure_mutation_plan_for_inputs(
                submit_node,
                submit_inputs,
                self.factory.artifacts(),
            )
            .await?;
            let runtime = self
                .factory
                .runtime_for(plan.config.as_ref().network().network_id())?;
            let required_depth = finalized_depth_for_submit_node(submit_node)?;
            let (receipt, receipt_evidence) =
                load_side_effect_artifact::<ContractConfigureReceipt>(
                    receipt,
                    events::ArtifactRole::Receipt,
                    ctx,
                    self.factory.artifacts(),
                )
                .await?;
            let receipt = configure_receipt_with_evidence(receipt, &receipt_evidence);
            let confirmations = verified_finality_confirmations(
                &runtime,
                plan.config.as_ref().network().expected_chain_id(),
                &receipt.receipts,
                required_depth,
            )
            .await?;
            Ok(SideEffectObservedEvidence {
                evidence: ContractConfigureConfirmation {
                    confirmation_version: 1,
                    confirmations,
                    configured_block_number: receipt.configured_block_number,
                    receipts: receipt.receipts,
                },
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
            let plan = configure_mutation_plan_for_inputs(
                submit_node,
                submit_inputs,
                self.factory.artifacts(),
            )
            .await?;
            let receipt = load_side_effect_value::<ContractConfigureReceipt>(
                receipt,
                events::ArtifactRole::Receipt,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            plan.state
                .output_from_receipt(&plan.input, &plan.intent, &receipt)
                .map_err(runtime_state_error)
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
            let plan = configure_mutation_plan_for_inputs(
                submit_node,
                submit_inputs,
                self.factory.artifacts(),
            )
            .await?;
            let confirmation = load_side_effect_value::<ContractConfigureConfirmation>(
                confirmation,
                events::ArtifactRole::Confirmation,
                ctx,
                self.factory.artifacts(),
            )
            .await?;
            plan.state
                .output_from_confirmation(&plan.input, &plan.intent, &confirmation)
                .map_err(runtime_state_error)
        })
    }
}

async fn deploy_mutation_plan(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<DeployMutationPlan> {
    deploy_mutation_plan_for_node(ctx.node(), artifacts).await
}

async fn deploy_mutation_plan_for_node(
    node: &spec::NodeSpec,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<DeployMutationPlan> {
    let config = load_config_for_node::<DeployPhaseConfig>(node, artifacts).await?;
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

async fn configure_mutation_plan(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ConfigureMutationPlan> {
    configure_mutation_plan_for_inputs(ctx.node(), ctx.inputs(), artifacts).await
}

async fn configure_mutation_plan_for_inputs(
    node: &spec::NodeSpec,
    inputs: &mfm_runtime::MaterializedInputs,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ConfigureMutationPlan> {
    let config = load_config_for_node::<ConfigurePhaseConfig>(node, artifacts).await?;
    let deployed =
        load_struct_input_value::<DeployedContract>(inputs, "deployed", artifacts).await?;
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
    let config = load_config::<ValidatePhaseConfig>(&ctx, factory.artifacts()).await?;
    let input = load_validate_input(ctx.inputs(), factory.artifacts()).await?;
    let state = ValidateContractState::new(config.clone())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let request = state
        .read_request(&input)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let runtime = factory.runtime_for(config.as_ref().network().network_id())?;
    let response = runtime
        .adapter()
        .validate_contract(&config, &input, &request)
        .await?;
    let report = state
        .report_from_response(&input, response.clone())
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let fact_capabilities = validation_fact_capabilities(&response)?;
    read_output(ctx, request, response, report, fact_capabilities).await
}

struct CapabilityFact {
    key_suffix: &'static str,
    kind: CapabilityKind,
    version: CapabilityVersion,
}

fn validation_fact_capabilities(
    response: &mfm_state_evm_contracts::ContractValidationReadResponse,
) -> mfm_runtime::Result<Vec<CapabilityFact>> {
    let mut facts = vec![CapabilityFact {
        key_suffix: "chain_identity",
        kind: EvmChainIdentityCapability::kind().map_err(runtime_capability_error)?,
        version: EvmChainIdentityCapability::version().map_err(runtime_capability_error)?,
    }];
    if !response.configuration_read_results.is_empty() || !response.read_results.is_empty() {
        facts.push(CapabilityFact {
            key_suffix: "call",
            kind: EvmCallReadCapability::kind().map_err(runtime_capability_error)?,
            version: EvmCallReadCapability::version().map_err(runtime_capability_error)?,
        });
    }
    if !response.configuration_event_results.is_empty() || !response.event_results.is_empty() {
        facts.push(CapabilityFact {
            key_suffix: "logs",
            kind: EvmLogsReadCapability::kind().map_err(runtime_capability_error)?,
            version: EvmLogsReadCapability::version().map_err(runtime_capability_error)?,
        });
    }
    Ok(facts)
}

async fn read_receipts_with_poll(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractInvocation,
    submissions: &ContractTransactionSubmissions,
) -> mfm_runtime::Result<Vec<mfm_state_evm_contracts::ContractTransactionReceipt>> {
    if prepared.transactions.len() != submissions.transactions.len() {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle receipt read transaction count mismatch".to_owned(),
        ));
    }
    let receipt_policy =
        ReceiptRetryPolicy::new(prepared.poll_interval_ms, prepared.max_receipt_polls)
            .map_err(mfm_runtime::RuntimeError::InvalidRunnerOutput)?;

    let max_polls = receipt_policy.max_receipt_polls();
    for attempt in 0..max_polls {
        match runtime
            .adapter()
            .read_receipts(&submissions.transactions)
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
        .latest_block_number(expected_chain_id)
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
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ValidateContractInput> {
    let MaterializedInputNode::Struct(fields) = &inputs.root else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract validate input was not materialized as a struct".to_owned(),
        ));
    };
    let configured = fields
        .iter()
        .find(|field| field.field_path.as_str() == "configured")
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "contract validate input missing configured field".to_owned(),
            )
        })?;
    let configured =
        load_value_from_node::<ConfiguredContract>(&configured.node, artifacts).await?;
    Ok(ValidateContractInput { configured })
}

async fn load_struct_input_value<T>(
    inputs: &mfm_runtime::MaterializedInputs,
    field_path: &str,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Struct(fields) = &inputs.root else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle input was not materialized as a struct".to_owned(),
        ));
    };
    let field = fields
        .iter()
        .find(|field| field.field_path.as_str() == field_path)
        .ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
                "contract lifecycle input missing {field_path} field"
            ))
        })?;
    load_value_from_node::<T>(&field.node, artifacts).await
}

async fn load_config<T>(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    load_config_for_node(ctx.node(), artifacts).await
}

fn load_launch_config<T>(ctx: &RunnerIngressContext<'_>) -> mfm_runtime::Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    load_launch_config_for_node(ctx, ctx.node())
}

fn load_launch_config_for_node<T>(
    ctx: &RunnerIngressContext<'_>,
    node: &spec::NodeSpec,
) -> mfm_runtime::Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let artifact = ctx.config_artifact_for_node(node)?;
    let config = serde_json::from_slice::<T>(&artifact.bytes).map_err(|error| {
        mfm_runtime::RuntimeError::RunnerBinding(format!(
            "launch config for node {} failed to decode: {error}",
            node.node_id
        ))
    })?;
    ValidatedConfig::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))
}

async fn load_config_for_node<T>(
    node: &spec::NodeSpec,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ValidatedConfig<T>>
where
    T: MfmConfig + DeserializeOwned,
{
    let request = ArtifactReadRequest::from_certified_config_ref(&node.config_ref);
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    let config = verified
        .decode_json::<T>()
        .map_err(runtime_artifact_read_error)?;
    ValidatedConfig::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))
}

async fn load_prepared_invocation(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<PreparedContractInvocation> {
    load_prepared_invocation_for_node(artifact, role, ctx, artifacts, &ctx.node().node_id).await
}

async fn load_prepared_invocation_for_node(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
    producer_node_id: &NodeId,
) -> mfm_runtime::Result<PreparedContractInvocation> {
    let prepared = load_side_effect_value_for_node::<PreparedContractInvocation>(
        artifact,
        role,
        ctx,
        artifacts,
        producer_node_id,
    )
    .await?;
    ensure_prepared_invocation_public(&prepared)?;
    Ok(prepared)
}

async fn load_verify_prepared_and_submission(
    ctx: &ErasedRunCtx<'_>,
    submit_node: &spec::NodeSpec,
    submission: &store::SideEffectArtifactProjection,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<(PreparedContractInvocation, ContractTransactionSubmissions)> {
    let artifacts = factory.artifacts();
    let prepared_projection = projected_prepared_artifact_for_submit(ctx, submit_node)?;
    let prepared = load_prepared_invocation_for_node(
        &prepared_projection,
        events::ArtifactRole::PreparedInvocation,
        ctx,
        artifacts,
        &submit_node.node_id,
    )
    .await?;
    let submissions = load_side_effect_value_for_node::<ContractTransactionSubmissions>(
        submission,
        events::ArtifactRole::Submission,
        ctx,
        artifacts,
        &submit_node.node_id,
    )
    .await?;
    Ok((prepared, submissions))
}

async fn load_side_effect_value<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let (value, _) =
        load_side_effect_artifact_for_node(artifact, role, ctx, artifacts, &ctx.node().node_id)
            .await?;
    Ok(value)
}

async fn load_side_effect_value_for_node<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
    producer_node_id: &NodeId,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let (value, _) =
        load_side_effect_artifact_for_node(artifact, role, ctx, artifacts, producer_node_id)
            .await?;
    Ok(value)
}

async fn load_side_effect_artifact<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<(T, mfm_artifact_capabilities::ArtifactEvidenceRef)>
where
    T: MfmValue + DeserializeOwned,
{
    load_side_effect_artifact_for_node(artifact, role, ctx, artifacts, &ctx.node().node_id).await
}

async fn load_side_effect_artifact_for_node<T>(
    artifact: &store::SideEffectArtifactProjection,
    role: events::ArtifactRole,
    _ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
    producer_node_id: &NodeId,
) -> mfm_runtime::Result<(T, mfm_artifact_capabilities::ArtifactEvidenceRef)>
where
    T: MfmValue + DeserializeOwned,
{
    let request =
        ArtifactReadRequest::from_side_effect_projection(artifact, role, producer_node_id.clone());
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    let evidence = verified.evidence().clone();
    let value = verified
        .decode_json()
        .map_err(runtime_artifact_read_error)?;
    Ok((value, evidence))
}

async fn read_output<Request, Response, Output>(
    ctx: ErasedRunCtx<'_>,
    request: Request,
    response: Response,
    output: Output,
    fact_capabilities: Vec<CapabilityFact>,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Request: MfmValue + Serialize,
    Response: MfmValue + Serialize,
    Output: MfmValue + Serialize,
{
    let artifacts = RunnerArtifactBuilder::new(&ctx);
    let payloads = RunnerPayloadBuilder::new(&ctx);
    let response_artifact = artifacts.fact_response(&response)?;
    let output_artifact = artifacts.state_output(&output)?;
    let mut runner_output = RunnerOutputBuilder::new(&ctx);
    runner_output.stage_attempt_artifact(&response_artifact)?;
    runner_output.retain_runtime_evidence(&response_artifact);
    runner_output.stage_attempt_artifact(&output_artifact)?;
    runner_output.retain_runtime_evidence(&output_artifact);
    for fact in fact_capabilities {
        runner_output.payload(payloads.fact_recorded(
            events::FactKey::new(format!(
                "mfm.evm.contract.fact.{}.{}",
                ctx.node().node_id.as_str(),
                fact.key_suffix
            ))?,
            &request,
            &response_artifact,
            evm_capability_binding(fact.kind, fact.version)?,
        )?);
    }
    runner_output.payload(payloads.cell_produced(&output_artifact)?);
    Ok(runner_output.finish())
}

async fn load_value_from_node<T>(
    node: &MaterializedInputNode,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let MaterializedInputNode::Cell(cell) = node else {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract lifecycle input was not a materialized cell".to_owned(),
        ));
    };
    load_value_from_cell(cell, artifacts).await
}

async fn load_value_from_cell<T>(
    cell: &MaterializedCell,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<T>
where
    T: MfmValue + DeserializeOwned,
{
    let request = match &cell.terminal {
        MaterializedCellTerminal::Seed {
            seed_id,
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_seed_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
            seed_id.clone(),
        ),
        MaterializedCellTerminal::Produced {
            producer_node_id,
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_produced_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
            producer_node_id.clone(),
        ),
        MaterializedCellTerminal::Skipped { .. } => {
            return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "contract lifecycle input cell was skipped".to_owned(),
            ));
        }
    };
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(runtime_artifact_read_error)?;
    verified.decode_json().map_err(runtime_artifact_read_error)
}

fn evm_transaction_submit_binding() -> mfm_runtime::Result<RunnerCapabilityBinding> {
    evm_capability_binding(
        EvmTransactionSubmitCapability::kind().map_err(runtime_capability_error)?,
        EvmTransactionSubmitCapability::version().map_err(runtime_capability_error)?,
    )
}

fn evm_capability_binding(
    capability_kind: CapabilityKind,
    capability_version: CapabilityVersion,
) -> mfm_runtime::Result<RunnerCapabilityBinding> {
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(RunnerCapabilityBinding {
        capability_kind,
        capability_version,
        adapter_kind: binding.adapter_kind().clone(),
        adapter_version: binding.adapter_version().clone(),
    })
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
        short_stable_key(&idempotency.key)
    ))?)
}

fn short_stable_key(value: &str) -> String {
    value
        .rsplit(':')
        .next()
        .unwrap_or(value)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(32)
        .collect()
}

fn digest_json(value: serde_json::Value) -> mfm_runtime::Result<ContentDigest> {
    let json = serde_json::to_string(&value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    Ok(PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?
        .content_digest())
}

fn runtime_capability_error(error: mfm_capabilities::CapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_artifact_read_error(
    error: mfm_artifact_capabilities::ArtifactReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
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
    /// State contract failed.
    #[error("contract state failed: {0}")]
    State(String),
    /// EVM capability provider failed.
    #[error("EVM capability failed: {0}")]
    EvmCapability(mfm_evm_capabilities::EvmCapabilityError),
    /// Signing provider failed.
    #[error("signing provider failed: {0}")]
    Signing(mfm_signing::SigningError),
    /// EVM signing bridge failed.
    #[error("EVM signing failed: {0}")]
    EvmSigning(mfm_evm_signing::EvmSigningError),
    /// Artifact read failed.
    #[error("artifact read failed: {0}")]
    ArtifactRead(mfm_artifact_capabilities::ArtifactReadError),
    /// Pure contract model preparation failed.
    #[error("contract model failed: {0}")]
    Model(String),
    /// Prepared invocation did not match state intent.
    #[error("contract intent did not match state config")]
    IntentMismatch,
    /// Observed chain id did not match typed config.
    #[error("contract lifecycle chain id mismatch")]
    ChainMismatch,
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
    /// Prepared invocation evidence carried forbidden live or secret-bearing data.
    #[error("prepared invocation exposed forbidden runtime data")]
    PreparedInvocationLeak,
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

impl From<mfm_evm_capabilities::EvmCapabilityError> for EvmContractAdapterError {
    fn from(error: mfm_evm_capabilities::EvmCapabilityError) -> Self {
        Self::EvmCapability(error)
    }
}

impl From<mfm_signing::SigningError> for EvmContractAdapterError {
    fn from(error: mfm_signing::SigningError) -> Self {
        Self::Signing(error)
    }
}

impl From<mfm_evm_signing::EvmSigningError> for EvmContractAdapterError {
    fn from(error: mfm_evm_signing::EvmSigningError) -> Self {
        Self::EvmSigning(error)
    }
}

impl From<EvmContractAdapterError> for mfm_runtime::RuntimeError {
    fn from(error: EvmContractAdapterError) -> Self {
        Self::InvalidRunnerOutput(error.to_string())
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
mod tests {
    use super::*;
    use mfm_evm_capabilities::{
        EvmBlockReadResponse, EvmCallReadResponse, EvmCapabilityFuture, EvmChainIdentityResponse,
        EvmFeeReadResponse, EvmGasEstimateResponse, EvmLogsReadResponse, EvmNonceReadResponse,
        EvmReceiptReadResponse, RedactedEvmSourceEvidence,
    };
    use mfm_evm_signing::{
        primitive_signature_from_bytes, recover_signing_address,
        EvmTransactionStyle as SigningTransactionStyle,
    };
    use mfm_replay::v1::SideEffectReplayVerifier;
    use mfm_signing::{
        PublicSigningIdentity, SignatureBytes, SigningError, SigningRequest, SigningResult,
    };
    use serde_json::json;
    use std::sync::Mutex;

    fn route() -> EvmContractRuntimeRoute {
        EvmContractRuntimeRoute::new(
            EvmSourceRef::new("local").expect("source"),
            EvmSourcePolicyId::new("test").expect("policy"),
        )
    }

    fn evidence() -> RedactedEvmSourceEvidence {
        RedactedEvmSourceEvidence {
            source_ref: route().source_ref().clone(),
            policy_id: route().policy_id().clone(),
            chain_id: 1,
        }
    }

    fn artifact_json() -> serde_json::Value {
        json!({
            "abi": {
                "json_text": json!([
                    {
                        "type": "constructor",
                        "inputs": []
                    },
                    {
                        "type": "function",
                        "name": "configure",
                        "inputs": [],
                        "outputs": [],
                        "stateMutability": "nonpayable"
                    },
                    {
                        "type": "function",
                        "name": "owner",
                        "inputs": [],
                        "outputs": [{ "name": "", "type": "bool" }],
                        "stateMutability": "view"
                    }
                ]).to_string()
            },
            "bytecode": {
                "json_text": json!({"object": "0x6000"}).to_string()
            }
        })
    }

    fn deploy_config(style: &str) -> DeployPhaseConfig {
        let expected_signer_address = expected_test_signer_address(style);
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1
            },
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": expected_signer_address
            },
            "transaction": match style {
                "legacy" => json!({"style": "legacy", "gas_price": "7"}),
                _ => json!({"style": "eip1559", "max_fee_per_gas": "11", "max_priority_fee_per_gas": "3"}),
            }
        }))
        .expect("deploy config")
    }

    fn validated_deploy_config(style: &str) -> ValidatedConfig<DeployPhaseConfig> {
        ValidatedConfig::new(deploy_config(style)).expect("valid deploy config")
    }

    fn deploy_intent(config: &ValidatedConfig<DeployPhaseConfig>) -> ContractDeployIntent {
        DeployContractState::new(config.clone())
            .expect("state")
            .prepare_intent(&())
            .expect("intent")
    }

    struct DeployPreparationFixture {
        route: EvmContractRuntimeRoute,
        providers: TestEvmProviders,
        config: ValidatedConfig<DeployPhaseConfig>,
        intent: ContractDeployIntent,
    }

    impl DeployPreparationFixture {
        fn new(style: &str) -> Self {
            let route = route();
            let providers = TestEvmProviders::preparation();
            let config = validated_deploy_config(style);
            let intent = deploy_intent(&config);
            Self {
                route,
                providers,
                config,
                intent,
            }
        }

        fn adapter(&self) -> EvmContractLifecycleAdapter<'_> {
            adapter(&self.route, &self.providers)
        }

        async fn prepare(&self) -> PreparedContractMutation {
            self.adapter()
                .prepare_deploy_invocation(&self.config, &self.intent)
                .await
                .expect("prepared")
        }
    }

    struct TestEvmProviders {
        mode: TestEvmProviderMode,
        submit_count: Arc<Mutex<u32>>,
        receipt_failure_reads: Arc<Mutex<u32>>,
    }

    #[derive(Clone, Copy)]
    enum TestEvmProviderMode {
        Preparation,
        Recovery {
            receipt_mode: RecoveryReceiptMode,
            pending_nonce: u64,
            occupancy_mode: RecoveryOccupancyMode,
        },
        ReceiptFailure,
        Finality,
    }

    impl TestEvmProviders {
        fn preparation() -> Self {
            Self::new(TestEvmProviderMode::Preparation)
        }

        fn recovery(
            receipt_mode: RecoveryReceiptMode,
            pending_nonce: u64,
            occupancy_mode: RecoveryOccupancyMode,
            submit_count: Arc<Mutex<u32>>,
        ) -> Self {
            Self {
                mode: TestEvmProviderMode::Recovery {
                    receipt_mode,
                    pending_nonce,
                    occupancy_mode,
                },
                submit_count,
                receipt_failure_reads: Arc::new(Mutex::new(0)),
            }
        }

        fn receipt_failure(reads: Arc<Mutex<u32>>) -> Self {
            Self {
                mode: TestEvmProviderMode::ReceiptFailure,
                submit_count: Arc::new(Mutex::new(0)),
                receipt_failure_reads: reads,
            }
        }

        fn finality() -> Self {
            Self::new(TestEvmProviderMode::Finality)
        }

        fn new(mode: TestEvmProviderMode) -> Self {
            Self {
                mode,
                submit_count: Arc::new(Mutex::new(0)),
                receipt_failure_reads: Arc::new(Mutex::new(0)),
            }
        }
    }

    fn runtime_from_provider(
        route: EvmContractRuntimeRoute,
        provider: TestEvmProviders,
    ) -> EvmContractRuntime {
        let provider = Arc::new(provider);
        let evm: Arc<dyn EvmContractProvider> = provider.clone();
        let signer: Arc<dyn SigningProvider> = provider;
        EvmContractRuntime::new(route, evm, signer)
    }

    fn unexpected_evm_call<'a, T>(capability: &'static str) -> EvmCapabilityFuture<'a, T> {
        Box::pin(async move { panic!("unexpected test EVM capability call: {capability}") })
    }

    fn unexpected_signing_call<'a>() -> mfm_signing::SigningFuture<'a> {
        Box::pin(async { panic!("unexpected test signing provider call") })
    }

    impl EvmChainIdentityProvider for TestEvmProviders {
        fn chain_identity<'a>(
            &'a self,
            _request: &'a EvmChainIdentityRequest,
        ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
            match self.mode {
                TestEvmProviderMode::Preparation => Box::pin(async {
                    Ok(EvmChainIdentityResponse {
                        evidence: evidence(),
                        chain_id: 1,
                        client_version: Some("test-client".to_owned()),
                    })
                }),
                _ => unexpected_evm_call("chain_identity"),
            }
        }
    }

    impl EvmBlockReadProvider for TestEvmProviders {
        fn read_block<'a>(
            &'a self,
            _request: &'a EvmBlockReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
            match self.mode {
                TestEvmProviderMode::Finality => Box::pin(async {
                    Ok(EvmBlockReadResponse {
                        evidence: evidence(),
                        block_number: 64,
                        block_hash: B256::from([0x64; 32]),
                    })
                }),
                _ => unexpected_evm_call("read_block"),
            }
        }
    }

    impl EvmNonceReadProvider for TestEvmProviders {
        fn read_nonce<'a>(
            &'a self,
            request: &'a EvmNonceReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
            match self.mode {
                TestEvmProviderMode::Preparation => Box::pin(async {
                    Ok(EvmNonceReadResponse {
                        evidence: evidence(),
                        nonce: 7,
                    })
                }),
                TestEvmProviderMode::Recovery { pending_nonce, .. } => {
                    assert_eq!(request.block, EvmBlockSelector::Pending);
                    Box::pin(async move {
                        Ok(EvmNonceReadResponse {
                            evidence: evidence(),
                            nonce: pending_nonce,
                        })
                    })
                }
                _ => unexpected_evm_call("read_nonce"),
            }
        }
    }

    impl EvmFeeReadProvider for TestEvmProviders {
        fn read_fee<'a>(
            &'a self,
            _request: &'a EvmFeeReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
            match self.mode {
                TestEvmProviderMode::Preparation => Box::pin(async {
                    Ok(EvmFeeReadResponse {
                        evidence: evidence(),
                        base_fee_per_gas: Some(5),
                        priority_fee_per_gas: Some(3),
                        max_fee_per_gas: Some(11),
                        legacy_gas_price: Some(7),
                    })
                }),
                _ => unexpected_evm_call("read_fee"),
            }
        }
    }

    impl EvmGasEstimateProvider for TestEvmProviders {
        fn estimate_gas<'a>(
            &'a self,
            _request: &'a EvmGasEstimateRequest,
        ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
            match self.mode {
                TestEvmProviderMode::Preparation => Box::pin(async {
                    Ok(EvmGasEstimateResponse {
                        evidence: evidence(),
                        gas_limit: 21_000,
                    })
                }),
                _ => unexpected_evm_call("estimate_gas"),
            }
        }
    }

    impl EvmTransactionSubmitProvider for TestEvmProviders {
        fn submit_transaction<'a>(
            &'a self,
            request: &'a EvmTransactionSubmitRequest,
        ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmTransactionSubmitResponse> {
            match self.mode {
                TestEvmProviderMode::Recovery { .. } => {
                    let submit_count = Arc::clone(&self.submit_count);
                    let transaction_hash = request.signed_payload.transaction_hash();
                    Box::pin(async move {
                        *submit_count.lock().expect("submit count") += 1;
                        Ok(mfm_evm_capabilities::EvmTransactionSubmitResponse {
                            evidence: evidence(),
                            transaction_hash,
                        })
                    })
                }
                _ => unexpected_evm_call("submit_transaction"),
            }
        }
    }

    impl EvmReceiptReadProvider for TestEvmProviders {
        fn read_receipt<'a>(
            &'a self,
            request: &'a EvmReceiptReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
            match self.mode {
                TestEvmProviderMode::Recovery { receipt_mode, .. } => {
                    let transaction_hash = request.transaction_hash;
                    Box::pin(async move {
                        match receipt_mode {
                            RecoveryReceiptMode::Landed => Ok(EvmReceiptReadResponse {
                                evidence: evidence(),
                                transaction_hash,
                                block_number: 42,
                                status: true,
                            }),
                            RecoveryReceiptMode::Pending => Err(EvmCapabilityError::ReceiptPending),
                            RecoveryReceiptMode::ProviderFailure => {
                                Err(EvmCapabilityError::redacted_provider_failure("test rpc"))
                            }
                        }
                    })
                }
                TestEvmProviderMode::ReceiptFailure => {
                    let reads = Arc::clone(&self.receipt_failure_reads);
                    Box::pin(async move {
                        let mut reads = reads.lock().map_err(|_| {
                            EvmCapabilityError::redacted_provider_failure("test evm")
                        })?;
                        *reads += 1;
                        Err(EvmCapabilityError::redacted_provider_failure("test evm"))
                    })
                }
                _ => unexpected_evm_call("read_receipt"),
            }
        }
    }

    impl EvmNonceOccupancyReadProvider for TestEvmProviders {
        fn read_nonce_occupancy<'a>(
            &'a self,
            request: &'a EvmNonceOccupancyReadRequest,
        ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmNonceOccupancyReadResponse> {
            match self.mode {
                TestEvmProviderMode::Recovery { occupancy_mode, .. } => {
                    let nonce = request.nonce;
                    Box::pin(async move {
                        match occupancy_mode {
                            RecoveryOccupancyMode::Unknown => {
                                Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                    evidence: evidence(),
                                    outcome: EvmNonceOccupancy::Unknown,
                                })
                            }
                            RecoveryOccupancyMode::Occupied { transaction_hash } => {
                                Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                    evidence: evidence(),
                                    outcome: EvmNonceOccupancy::Occupied {
                                        transaction_hash,
                                        block_number: Some(43 + nonce),
                                    },
                                })
                            }
                        }
                    })
                }
                _ => unexpected_evm_call("read_nonce_occupancy"),
            }
        }
    }

    impl EvmCallReadProvider for TestEvmProviders {
        fn read_call<'a>(
            &'a self,
            _request: &'a EvmCallReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
            unexpected_evm_call("read_call")
        }
    }

    impl EvmLogsReadProvider for TestEvmProviders {
        fn read_logs<'a>(
            &'a self,
            _request: &'a EvmLogsReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
            unexpected_evm_call("read_logs")
        }
    }

    impl SigningProvider for TestEvmProviders {
        fn sign<'a>(&'a self, request: &'a SigningRequest) -> mfm_signing::SigningFuture<'a> {
            match self.mode {
                TestEvmProviderMode::Preparation | TestEvmProviderMode::Recovery { .. } => {
                    let result = test_signing_result(request);
                    Box::pin(async move { result })
                }
                _ => unexpected_signing_call(),
            }
        }
    }

    fn test_signature_bytes() -> SignatureBytes {
        SignatureBytes::new(
            hex_to_bytes(
                "0x48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353\
                 efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b",
            )
            .expect("signature hex"),
        )
        .expect("signature bytes")
    }

    fn test_signing_result(request: &SigningRequest) -> mfm_signing::Result<SigningResult> {
        let signature = test_signature_bytes();
        let primitive = primitive_signature_from_bytes(&signature)
            .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
        let recovered =
            recover_signing_address(B256::from(*request.digest().as_bytes()), primitive)
                .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
        let identity = PublicSigningIdentity::new(
            request.algorithm().clone(),
            None,
            Some(format!("{recovered:?}")),
        )?;
        SigningResult::for_request(request, identity, signature)
    }

    fn expected_test_signer_address(style: &str) -> String {
        let signer_ref = SignerRef::new("deployer").expect("signer ref");
        let expected_from = Address::from([0_u8; 20]);
        let data = vec![0x60, 0x00];
        let request = match style {
            "legacy" => EvmSigningRequest::legacy(
                signer_ref,
                LegacyTxToSign {
                    to: None,
                    value_wei: 0,
                    chain_id: 1,
                    nonce: 7,
                    gas_price_wei: 7,
                    gas_limit: 21_000,
                    data,
                },
                expected_from,
            ),
            _ => EvmSigningRequest::eip1559(
                signer_ref,
                Eip1559TxToSign {
                    to: None,
                    value_wei: 0,
                    chain_id: 1,
                    nonce: 7,
                    max_fee_per_gas: 11,
                    max_priority_fee_per_gas: 3,
                    gas_limit: 21_000,
                    data,
                },
                expected_from,
            ),
        }
        .expect("signing request");
        let primitive = primitive_signature_from_bytes(&test_signature_bytes()).expect("signature");
        let recovered =
            recover_signing_address(request.signing_hash(), primitive).expect("recovered address");
        format!("{recovered:?}")
    }

    fn adapter<'a>(
        route: &'a EvmContractRuntimeRoute,
        providers: &'a TestEvmProviders,
    ) -> EvmContractLifecycleAdapter<'a> {
        EvmContractLifecycleAdapter::new(
            route,
            EvmContractMutationProviders {
                chain_identity: providers,
                block: providers,
                nonce: providers,
                fee: providers,
                gas: providers,
                signer: providers,
                submit: providers,
                receipt: providers,
                nonce_occupancy: providers,
            },
            EvmContractReadProviders {
                chain_identity: providers,
                call: providers,
                logs: providers,
            },
        )
    }

    #[test]
    fn executable_identity_summary_matches_golden() {
        assert_eq!(
            executable_identity_summary([SIDE_EFFECT_FACTORY, READ_FACTORY]),
            [
                "factory=apply_side_effect;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
                "factory=read_external;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
            ]
        );
    }

    #[tokio::test]
    async fn deploy_preparation_defaults_to_eip1559_contract_creation() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;

        assert_eq!(prepared.evidence().transactions.len(), 1);
        assert_eq!(
            prepared.evidence().transactions[0].style,
            PreparedContractTransactionStyle::Eip1559
        );
        assert_eq!(prepared.evidence().transactions[0].to_address, None);
        assert_eq!(
            prepared.signing_requests()[0].style(),
            SigningTransactionStyle::Eip1559
        );
    }

    #[tokio::test]
    async fn deploy_preparation_supports_legacy_contract_creation() {
        let fixture = DeployPreparationFixture::new("legacy");
        let prepared = fixture.prepare().await;

        assert_eq!(
            prepared.evidence().transactions[0].style,
            PreparedContractTransactionStyle::Legacy
        );
        assert_eq!(
            prepared.signing_requests()[0].style(),
            SigningTransactionStyle::Legacy
        );
    }

    #[tokio::test]
    async fn prepared_invocation_evidence_excludes_live_and_secret_surfaces() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;

        ensure_prepared_invocation_public(prepared.evidence()).expect("public evidence");
        let evidence = prepared.evidence();
        let rendered_value = serde_json::to_value(evidence).expect("json value");
        assert_eq!(
            sorted_json_keys(&rendered_value),
            vec![
                "expected_chain_id",
                "expected_signer_address",
                "max_receipt_polls",
                "network_id",
                "phase",
                "poll_interval_ms",
                "prepared_version",
                "signer_ref",
                "transactions",
            ]
        );
        assert_eq!(evidence.signer_ref, "deployer");
        assert_eq!(
            evidence.expected_signer_address,
            expected_test_signer_address("eip1559")
        );
        let transaction = evidence.transactions.first().expect("transaction");
        let rendered_transaction = rendered_value["transactions"][0].clone();
        assert_eq!(
            sorted_json_keys(&rendered_transaction),
            vec![
                "chain_id",
                "data_digest",
                "data_len",
                "expected_transaction_hash",
                "gas_limit",
                "gas_price",
                "index",
                "max_fee_per_gas",
                "max_priority_fee_per_gas",
                "nonce",
                "signing_digest",
                "style",
                "to_address",
                "value_wei",
            ]
        );
        assert_eq!(transaction.nonce, 7);
        assert_eq!(transaction.gas_limit, 21_000);
        assert_eq!(transaction.max_fee_per_gas.as_deref(), Some("11"));
        assert_eq!(transaction.max_priority_fee_per_gas.as_deref(), Some("3"));
        assert!(transaction
            .data_digest
            .starts_with("content:sha256-jcs-v1:"));
        assert_eq!(transaction.data_len, 2);
        assert!(transaction.signing_digest.starts_with("0x"));
        assert_eq!(transaction.signing_digest.len(), 66);
        assert!(transaction.expected_transaction_hash.starts_with("0x"));
        assert_eq!(transaction.expected_transaction_hash.len(), 66);

        let rendered = serde_json::to_string(prepared.evidence()).expect("json");
        let rendered = rendered.to_ascii_lowercase();
        for forbidden in forbidden_prepared_terms() {
            assert!(
                !rendered.contains(&forbidden),
                "prepared invocation evidence contains forbidden runtime surface"
            );
        }
    }

    #[tokio::test]
    async fn prepared_invocation_reconstruction_preserves_submission_anchor() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let reconstructed = fixture
            .adapter()
            .reconstruct_deploy_invocation(&fixture.config, &fixture.intent, prepared.evidence())
            .expect("reconstructed");

        assert_eq!(reconstructed.evidence(), prepared.evidence());
        assert_eq!(
            reconstructed.evidence().transactions[0].expected_transaction_hash,
            prepared.evidence().transactions[0].expected_transaction_hash
        );
    }

    #[tokio::test]
    async fn submit_rejects_resigned_hash_mismatch_before_broadcast() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let mut evidence = prepared.evidence().clone();
        evidence.transactions[0].expected_transaction_hash =
            "0x1111111111111111111111111111111111111111111111111111111111111111".to_owned();
        let adapter = fixture.adapter();
        let reconstructed = adapter
            .reconstruct_deploy_invocation(&fixture.config, &fixture.intent, &evidence)
            .expect("reconstructed");

        assert!(matches!(
            adapter.submit_prepared(&reconstructed).await,
            Err(EvmContractAdapterError::TransactionHashMismatch)
        ));
    }

    #[tokio::test]
    async fn recovery_observes_landed_anchor_without_resubmitting() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let submit_count = Arc::new(Mutex::new(0_u32));
        let runtime = runtime_from_provider(
            fixture.route,
            TestEvmProviders::recovery(
                RecoveryReceiptMode::Landed,
                7,
                RecoveryOccupancyMode::Unknown,
                Arc::clone(&submit_count),
            ),
        );

        let decision = submit_or_recover_contract_submission(
            &runtime,
            &prepared,
            SideEffectProtocolAction::SubmitOrRecoverSubmission {
                invocation_epoch: 1,
            },
        )
        .await
        .expect("recovered");

        let SideEffectSubmissionDecision::Observed(submissions) = decision else {
            panic!("expected observed anchor recovery");
        };
        assert_eq!(*submit_count.lock().expect("submit count"), 0);
        assert_eq!(
            submissions.transactions[0].transaction_hash,
            prepared.evidence().transactions[0].expected_transaction_hash
        );
    }

    #[tokio::test]
    async fn recovery_rebroadcasts_unlanded_anchor_after_resign_match() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let submit_count = Arc::new(Mutex::new(0_u32));
        let runtime = runtime_from_provider(
            fixture.route,
            TestEvmProviders::recovery(
                RecoveryReceiptMode::Pending,
                7,
                RecoveryOccupancyMode::Unknown,
                Arc::clone(&submit_count),
            ),
        );

        let decision = submit_or_recover_contract_submission(
            &runtime,
            &prepared,
            SideEffectProtocolAction::SubmitOrRecoverSubmission {
                invocation_epoch: 1,
            },
        )
        .await
        .expect("recovered");

        let SideEffectSubmissionDecision::Observed(submissions) = decision else {
            panic!("expected observed rebroadcast");
        };
        assert_eq!(*submit_count.lock().expect("submit count"), 1);
        assert_eq!(
            submissions.transactions[0].transaction_hash,
            prepared.evidence().transactions[0].expected_transaction_hash
        );
    }

    #[tokio::test]
    async fn recovery_does_not_rebroadcast_when_resign_hash_mismatches_anchor() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let mut evidence = prepared.evidence().clone();
        evidence.transactions[0].expected_transaction_hash =
            "0x1111111111111111111111111111111111111111111111111111111111111111".to_owned();
        let reconstructed = fixture
            .adapter()
            .reconstruct_deploy_invocation(&fixture.config, &fixture.intent, &evidence)
            .expect("reconstructed");
        let submit_count = Arc::new(Mutex::new(0_u32));
        let runtime = runtime_from_provider(
            fixture.route,
            TestEvmProviders::recovery(
                RecoveryReceiptMode::Pending,
                7,
                RecoveryOccupancyMode::Unknown,
                Arc::clone(&submit_count),
            ),
        );

        let decision = submit_or_recover_contract_submission(
            &runtime,
            &reconstructed,
            SideEffectProtocolAction::SubmitOrRecoverSubmission {
                invocation_epoch: 1,
            },
        )
        .await
        .expect("recovered");

        let SideEffectSubmissionDecision::Unknown(evidence) = decision else {
            panic!("expected unknown mismatched anchor evidence");
        };
        assert_eq!(*submit_count.lock().expect("submit count"), 0);
        assert_eq!(
            evidence.transactions[0].transaction_hash,
            "0x1111111111111111111111111111111111111111111111111111111111111111"
        );
    }

    #[tokio::test]
    async fn recovery_keeps_advanced_nonce_unknown_without_occupancy_proof() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let submit_count = Arc::new(Mutex::new(0_u32));
        let runtime = runtime_from_provider(
            fixture.route,
            TestEvmProviders::recovery(
                RecoveryReceiptMode::Pending,
                8,
                RecoveryOccupancyMode::Unknown,
                Arc::clone(&submit_count),
            ),
        );

        let decision = submit_or_recover_contract_submission(
            &runtime,
            &prepared,
            SideEffectProtocolAction::SubmitOrRecoverSubmission {
                invocation_epoch: 1,
            },
        )
        .await
        .expect("recovered");

        let SideEffectSubmissionDecision::Unknown(evidence) = decision else {
            panic!("expected unknown occupied nonce evidence");
        };
        assert_eq!(*submit_count.lock().expect("submit count"), 0);
        assert_eq!(
            evidence.transactions[0].transaction_hash,
            prepared.evidence().transactions[0].expected_transaction_hash
        );
    }

    #[tokio::test]
    async fn recovery_proves_not_submitted_when_foreign_transaction_occupies_nonce() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let submit_count = Arc::new(Mutex::new(0_u32));
        let occupying_hash = "0x2222222222222222222222222222222222222222222222222222222222222222"
            .parse::<B256>()
            .expect("occupying hash");
        let runtime = runtime_from_provider(
            fixture.route,
            TestEvmProviders::recovery(
                RecoveryReceiptMode::Pending,
                8,
                RecoveryOccupancyMode::Occupied {
                    transaction_hash: occupying_hash,
                },
                Arc::clone(&submit_count),
            ),
        );

        let decision = submit_or_recover_contract_submission(
            &runtime,
            &prepared,
            SideEffectProtocolAction::SubmitOrRecoverSubmission {
                invocation_epoch: 1,
            },
        )
        .await
        .expect("recovered");

        let SideEffectSubmissionDecision::NotSubmitted(proof) = decision else {
            panic!("expected not-submitted proof");
        };
        assert_eq!(*submit_count.lock().expect("submit count"), 0);
        assert_eq!(
            proof.expected_transaction_hash,
            prepared.evidence().transactions[0].expected_transaction_hash
        );
        assert_eq!(
            proof.occupying_transaction_hash,
            "0x2222222222222222222222222222222222222222222222222222222222222222"
        );
        assert_eq!(proof.nonce, prepared.evidence().transactions[0].nonce);
        assert_eq!(proof.evidence_chain_id, 1);
    }

    #[tokio::test]
    async fn recovery_records_unknown_on_transient_anchor_read_failure() {
        let fixture = DeployPreparationFixture::new("eip1559");
        let prepared = fixture.prepare().await;
        let submit_count = Arc::new(Mutex::new(0_u32));
        let runtime = runtime_from_provider(
            fixture.route,
            TestEvmProviders::recovery(
                RecoveryReceiptMode::ProviderFailure,
                7,
                RecoveryOccupancyMode::Unknown,
                Arc::clone(&submit_count),
            ),
        );

        let decision = submit_or_recover_contract_submission(
            &runtime,
            &prepared,
            SideEffectProtocolAction::SubmitOrRecoverSubmission {
                invocation_epoch: 1,
            },
        )
        .await
        .expect("recovered");

        let SideEffectSubmissionDecision::Unknown(evidence) = decision else {
            panic!("expected unknown recovery evidence");
        };
        assert_eq!(*submit_count.lock().expect("submit count"), 0);
        assert_eq!(
            evidence.transactions[0].transaction_hash,
            prepared.evidence().transactions[0].expected_transaction_hash
        );
    }

    #[test]
    fn prepared_invocation_guard_rejects_forbidden_runtime_terms() {
        for forbidden in forbidden_prepared_terms() {
            let mut prepared = prepared_invocation_fixture();
            prepared.signer_ref = format!("deployer-{forbidden}");

            assert!(
                matches!(
                    ensure_prepared_invocation_public(&prepared),
                    Err(EvmContractAdapterError::PreparedInvocationLeak)
                ),
                "expected forbidden prepared term {forbidden} to be rejected"
            );
        }
    }

    fn prepared_invocation_fixture() -> PreparedContractInvocation {
        PreparedContractInvocation {
            prepared_version: 1,
            phase: ContractMutationPhase::Deploy,
            network_id: "ethereum-mainnet".to_owned(),
            expected_chain_id: 1,
            signer_ref: "deployer".to_owned(),
            expected_signer_address: "0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e".to_owned(),
            transactions: vec![PreparedContractTransactionEvidence {
                index: 0,
                style: PreparedContractTransactionStyle::Eip1559,
                chain_id: 1,
                nonce: 7,
                to_address: None,
                value_wei: "0".to_owned(),
                gas_limit: 21_000,
                max_fee_per_gas: Some("11".to_owned()),
                max_priority_fee_per_gas: Some("3".to_owned()),
                gas_price: None,
                data_digest:
                    "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
                data_len: 2,
                signing_digest:
                    "0x0000000000000000000000000000000000000000000000000000000000000000"
                        .to_owned(),
                expected_transaction_hash:
                    "0x1111111111111111111111111111111111111111111111111111111111111111"
                        .to_owned(),
            }],
            poll_interval_ms: 1_000,
            max_receipt_polls: 10,
        }
    }

    fn sorted_json_keys(value: &serde_json::Value) -> Vec<&str> {
        let mut keys = value
            .as_object()
            .expect("json object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys
    }

    #[derive(Clone, Copy)]
    enum RecoveryReceiptMode {
        Landed,
        Pending,
        ProviderFailure,
    }

    #[derive(Clone, Copy)]
    enum RecoveryOccupancyMode {
        Unknown,
        Occupied { transaction_hash: B256 },
    }

    #[test]
    fn replay_verifier_uses_contract_namespace() {
        let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");

        assert_eq!(verifier.verifier_id().as_str(), REPLAY_VERIFIER_ID);
        assert!(!verifier
            .verifier_id()
            .as_str()
            .contains(&["d", "cv"].concat()));
    }

    #[tokio::test]
    async fn receipt_polling_does_not_retry_permanent_capability_failures() {
        let reads = Arc::new(Mutex::new(0_u32));
        let runtime = runtime_from_provider(
            route(),
            TestEvmProviders::receipt_failure(Arc::clone(&reads)),
        );
        let transaction_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let prepared = PreparedContractInvocation {
            prepared_version: 1,
            phase: ContractMutationPhase::Deploy,
            network_id: "reth-dev".to_owned(),
            expected_chain_id: 31337,
            signer_ref: "deployer".to_owned(),
            expected_signer_address: "0x0000000000000000000000000000000000000001".to_owned(),
            transactions: vec![PreparedContractTransactionEvidence {
                index: 0,
                style: PreparedContractTransactionStyle::Eip1559,
                chain_id: 31337,
                nonce: 7,
                to_address: None,
                value_wei: "0".to_owned(),
                gas_limit: 21_000,
                max_fee_per_gas: Some("11".to_owned()),
                max_priority_fee_per_gas: Some("3".to_owned()),
                gas_price: None,
                data_digest: "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
                data_len: 0,
                signing_digest: transaction_hash.to_owned(),
                expected_transaction_hash: transaction_hash.to_owned(),
            }],
            poll_interval_ms: 25,
            max_receipt_polls: 3,
        };
        let submissions = ContractTransactionSubmissions {
            submissions_version: 1,
            transactions: vec![ContractTransactionSubmission {
                submission_version: 1,
                transaction_hash: transaction_hash.to_owned(),
                signer_public_key: None,
            }],
        };

        let error = read_receipts_with_poll(&runtime, &prepared, &submissions)
            .await
            .expect_err("permanent capability failure");

        assert!(error.to_string().contains("EVM capability failed"));
        assert_eq!(*reads.lock().expect("reads"), 1);
    }

    #[tokio::test]
    async fn finality_confirmation_requires_certified_depth() {
        let runtime = runtime_from_provider(route(), TestEvmProviders::finality());
        let receipt = ContractTransactionReceipt {
            receipt_version: 1,
            transaction_hash: "0x1111111111111111111111111111111111111111111111111111111111111111"
                .to_owned(),
            block_number: 63,
            status: true,
            receipt_evidence: None,
        };

        let confirmations =
            verified_finality_confirmations(&runtime, 1, std::slice::from_ref(&receipt), 2)
                .await
                .expect("sufficient confirmations");
        assert_eq!(confirmations, 2);

        let error = verified_finality_confirmations(&runtime, 1, std::slice::from_ref(&receipt), 3)
            .await
            .expect_err("insufficient confirmations");
        assert!(matches!(error, mfm_runtime::RuntimeError::Blocked(_)));
    }

    #[test]
    fn replay_confirmation_depth_rejects_insufficient_certified_depth() {
        assert!(ensure_replay_confirmation_depth(3, 3).is_ok());
        let error = ensure_replay_confirmation_depth(2, 3).expect_err("insufficient replay depth");
        assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
    }

    #[test]
    fn adapter_source_has_no_concrete_store_or_signer_provider_coupling() {
        let source = include_str!("lib.rs");
        for forbidden in [
            ["artifact", "_store", "_fs"].concat(),
            ["Fs", "Typed", "Artifact", "Store"].concat(),
            ["Key", "store"].concat(),
            ["Runtime", "Secret", "Source"].concat(),
            ["Signer", "Provider", "Runtime", "Config"].concat(),
            ["MFM", "_EVM", "_RPC"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "adapter source contains forbidden coupling {forbidden}"
            );
        }
    }

    fn executable_identity_summary(factories: [&str; 2]) -> Vec<String> {
        factories
            .into_iter()
            .map(|factory| {
                let identity =
                    executable(events::RunnerFactoryId::new(factory).expect("factory id"))
                        .expect("executable identity");
                format!(
                    "factory={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                    identity.factory_id,
                    identity.cargo_package_digest,
                    identity.binary_digest,
                    identity.nix_derivation_hash.is_some(),
                    identity.nix_output_hash.is_some()
                )
            })
            .collect()
    }
}
