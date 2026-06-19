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

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{keccak256, Address, B256};
use mfm_adapter_contracts::evm_contract_lifecycle_adapter_binding;
use mfm_artifact_capabilities::{
    ArtifactEvidenceRef as CapabilityArtifactEvidenceRef, ArtifactReadProvider, ArtifactReadRequest,
};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{CapabilitySetDescriptor, CapabilitySpec};
use mfm_events::v1::{self as events, side_effect};
use mfm_evm_capabilities::{
    EvmBlockSelector, EvmCallReadCapability, EvmCallReadProvider, EvmCallReadRequest,
    EvmCapabilityError, EvmChainIdentityCapability, EvmChainIdentityProvider,
    EvmChainIdentityRequest, EvmFeeReadProvider, EvmFeeReadRequest, EvmGasEstimateProvider,
    EvmGasEstimateRequest, EvmLogsReadCapability, EvmLogsReadProvider, EvmLogsReadRequest,
    EvmNonceReadProvider, EvmNonceReadRequest, EvmReceiptReadProvider, EvmReceiptReadRequest,
    EvmSourcePolicyId, EvmSourceRef, EvmTransactionSubmitCapability, EvmTransactionSubmitProvider,
    EvmTransactionSubmitRequest, SignedEvmPayload,
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
    ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest, DescriptorId, DigestAlgorithm,
    NodeId, SchemaId,
};
use mfm_program::{SideEffectState, StateSpec, ValidatedConfig};
use mfm_program_derive::MfmValue;
use mfm_replay::v1 as replay;
use mfm_runtime::{
    CapabilityImplementationId, ErasedNodeRunner, ErasedRunCtx, ErasedRunnerBinding,
    ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry, MaterializedCell,
    MaterializedCellTerminal, MaterializedInputNode, RunnerEventPayload, StagedArtifact,
    StagedRetentionRefs,
};
use mfm_signing::{PublicKeyBytes, SignerRef, SigningProvider};
use mfm_spec::v1 as spec;
use mfm_state_evm_contracts::{
    ConfigureContractInput, ConfigureContractState, ContractConfigureConfirmation,
    ContractConfigureIntent, ContractDeployConfirmation, ContractDeployIntent,
    ContractTransactionIdempotency, ContractTransactionReceipt, ContractTransactionReceipts,
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
        if evidence.transactions.len() != signing_requests.len() {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
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
    /// Creates an adapter from process-local routing and provider bundles.
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

    /// Prepares a deploy invocation with EIP-1559 default and legacy support.
    pub async fn prepare_deploy_invocation(
        &self,
        config: &ValidatedConfig<DeployPhaseConfig>,
        intent: &ContractDeployIntent,
    ) -> Result<PreparedContractMutation> {
        let expected = DeployContractState::new(config.clone())
            .map_err(state_error)?
            .prepare_intent(&())
            .map_err(state_runtime_error)?;
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
        let expected = ConfigureContractState::new(config.clone())
            .map_err(state_error)?
            .prepare_intent(input)
            .map_err(state_runtime_error)?;
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
        let expected = DeployContractState::new(config.clone())
            .map_err(state_error)?
            .prepare_intent(&())
            .map_err(state_runtime_error)?;
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
        let expected = ConfigureContractState::new(config.clone())
            .map_err(state_error)?
            .prepare_intent(input)
            .map_err(state_runtime_error)?;
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
        for signing_request in prepared.signing_requests() {
            let result = self
                .mutation
                .signer
                .sign(signing_request.signing_request())
                .await
                .map_err(EvmContractAdapterError::Signing)?;
            let raw = signing_request
                .materialize_signed_payload(&result)
                .map_err(EvmContractAdapterError::EvmSigning)?;
            let expected_hash = raw
                .transaction_hash()
                .parse::<B256>()
                .map_err(|_| EvmContractAdapterError::TransactionHashMismatch)?;
            let payload =
                SignedEvmPayload::from_verified_bytes(raw.bytes().to_vec(), expected_hash)
                    .map_err(EvmContractAdapterError::EvmCapability)?;
            let response = self
                .mutation
                .submit
                .submit_transaction(&EvmTransactionSubmitRequest {
                    source_ref: self.route().source_ref().clone(),
                    policy_id: self.route().policy_id().clone(),
                    signed_payload: payload,
                })
                .await
                .map_err(EvmContractAdapterError::EvmCapability)?;
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
                .await
                .map_err(EvmContractAdapterError::EvmCapability)?;
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

    /// Projects deploy output from a confirmed deploy receipt and contract address.
    pub fn confirm_deploy(
        &self,
        config: &ValidatedConfig<DeployPhaseConfig>,
        intent: &ContractDeployIntent,
        receipt: ContractTransactionReceipt,
        contract_address: Address,
    ) -> Result<mfm_evm_contract_model::DeployedContract> {
        let state = DeployContractState::new(config.clone()).map_err(state_error)?;
        let confirmation = ContractDeployConfirmation {
            confirmation_version: 1,
            contract_address: normalize_address(&format!("{contract_address:?}"))
                .map_err(EvmContractAdapterError::Model)?,
            receipt,
        };
        state
            .output_from_confirmation(&(), intent, &confirmation)
            .map_err(state_runtime_error)
    }

    /// Projects configure output from confirmed configure receipts.
    pub fn confirm_configure(
        &self,
        config: &ValidatedConfig<ConfigurePhaseConfig>,
        input: &ConfigureContractInput,
        intent: &ContractConfigureIntent,
        receipts: Vec<ContractTransactionReceipt>,
    ) -> Result<ConfiguredContract> {
        let state = ConfigureContractState::new(config.clone()).map_err(state_error)?;
        let confirmation = ContractConfigureConfirmation {
            confirmation_version: 1,
            configured_block_number: receipts.iter().map(|receipt| receipt.block_number).max(),
            receipts,
        };
        state
            .output_from_confirmation(input, intent, &confirmation)
            .map_err(state_runtime_error)
    }

    /// Executes validation reads and projects a validation response.
    pub async fn validate_contract(
        &self,
        config: &ValidatedConfig<ValidatePhaseConfig>,
        input: &ValidateContractInput,
        request: &ContractValidationReadRequest,
    ) -> Result<ContractValidationReadResponse> {
        let expected = ValidateContractState::new(config.clone())
            .map_err(state_error)?
            .read_request(input)
            .map_err(state_runtime_error)?;
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
            .await
            .map_err(EvmContractAdapterError::EvmCapability)?;
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
            .await
            .map_err(EvmContractAdapterError::EvmCapability)?;
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
            .await
            .map_err(EvmContractAdapterError::EvmCapability)?
            .nonce;
        let fees = self
            .mutation
            .fee
            .read_fee(&EvmFeeReadRequest {
                source_ref: self.route().source_ref().clone(),
                policy_id: self.route().policy_id().clone(),
            })
            .await
            .map_err(EvmContractAdapterError::EvmCapability)?;

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
                        .await
                        .map_err(EvmContractAdapterError::EvmCapability)?
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
                        EvmSigningRequest::eip1559(signer_ref.clone(), tx, expected_signer)
                            .map_err(EvmContractAdapterError::EvmSigning)?;
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
                        EvmSigningRequest::legacy(signer_ref.clone(), tx, expected_signer)
                            .map_err(EvmContractAdapterError::EvmSigning)?;
                    PreparedTransaction {
                        style: PreparedContractTransactionStyle::Legacy,
                        request,
                        max_fee_per_gas: None,
                        max_priority_fee_per_gas: None,
                        gas_price: Some(gas_price),
                    }
                }
            };
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
                .await
                .map_err(EvmContractAdapterError::EvmCapability)?;
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
                .await
                .map_err(EvmContractAdapterError::EvmCapability)?;
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
        ensure_schema(
            &input.receipt.receipt.receipt_schema_id,
            &ContractTransactionReceipts::schema_id().map_err(replay_value_error)?,
            "receipt",
        )
    }

    fn verify_confirmation(
        &self,
        input: &replay::SideEffectConfirmationReplayInput,
    ) -> replay::Result<()> {
        self.verify_adapter_binding(&input.intent)?;
        let schema = &input.confirmation.confirmation.confirmation_schema_id;
        let deploy = ContractDeployConfirmation::schema_id().map_err(replay_value_error)?;
        let configure = ContractConfigureConfirmation::schema_id().map_err(replay_value_error)?;
        if schema == &deploy || schema == &configure {
            Ok(())
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

/// Verifies contract lifecycle side-effect replay evidence when present in a broker stream.
///
/// Returns `Ok(false)` when the stream contains no contract lifecycle side-effect intent.
pub fn verify_contract_lifecycle_replay(broker: &replay::ReplayBroker) -> replay::Result<bool> {
    let mut frames =
        BTreeMap::<(events::SideEffectLedgerKey, u32), ContractLifecycleReplayFrames>::new();
    for event in broker.events() {
        match event.payload() {
            events::KernelEventPayload::SideEffectIntentPersisted(payload)
                if is_contract_lifecycle_intent(payload)? =>
            {
                let key = (payload.ledger_key.clone(), payload.invocation_epoch);
                if frames
                    .insert(
                        key,
                        ContractLifecycleReplayFrames {
                            intent: payload.clone(),
                            submission: None,
                            receipt: None,
                            confirmation: None,
                        },
                    )
                    .is_some()
                {
                    return Err(replay::ReplayError::new(
                        replay::ReplayErrorKind::SideEffectMismatch,
                        "duplicate contract lifecycle side-effect intent",
                    ));
                }
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                let key = (payload.ledger_key.clone(), payload.invocation_epoch);
                if let Some(frames) = frames.get_mut(&key) {
                    frames.submission = Some(payload.clone());
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                let key = (payload.ledger_key.clone(), payload.invocation_epoch);
                if let Some(frames) = frames.get_mut(&key) {
                    frames.receipt = Some(payload.clone());
                }
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                let key = (payload.ledger_key.clone(), payload.invocation_epoch);
                if let Some(frames) = frames.get_mut(&key) {
                    frames.confirmation = Some(payload.clone());
                }
            }
            _ => {}
        }
    }
    if frames.is_empty() {
        return Ok(false);
    }
    let verifier =
        EvmContractLifecycleReplayVerifier::new().map_err(replay_contract_adapter_error)?;
    for frames in frames.values() {
        verify_contract_lifecycle_replay_frames(broker, &verifier, frames)?;
    }
    Ok(true)
}

fn verify_contract_lifecycle_replay_frames(
    broker: &replay::ReplayBroker,
    verifier: &EvmContractLifecycleReplayVerifier,
    frames: &ContractLifecycleReplayFrames,
) -> replay::Result<()> {
    let Some(submission) = &frames.submission else {
        return Err(contract_lifecycle_side_effect_missing("submission"));
    };
    let Some(receipt) = &frames.receipt else {
        return Err(contract_lifecycle_side_effect_missing("receipt"));
    };
    let Some(confirmation) = &frames.confirmation else {
        return Err(contract_lifecycle_side_effect_missing("confirmation"));
    };

    broker.verify_side_effect_submission(
        &side_effect_replay_request(
            &frames.intent,
            submission.submission_schema_id.clone(),
            submission.submission_hash.clone(),
            None,
        ),
        verifier,
    )?;
    broker.verify_side_effect_receipt(
        &side_effect_replay_request(
            &frames.intent,
            receipt.receipt_schema_id.clone(),
            receipt.receipt_hash.clone(),
            Some(receipt.replay_verifier_id.clone()),
        ),
        verifier,
    )?;
    broker.verify_side_effect_confirmation(
        &side_effect_replay_request(
            &frames.intent,
            confirmation.confirmation_schema_id.clone(),
            confirmation.confirmation_hash.clone(),
            Some(confirmation.replay_verifier_id.clone()),
        ),
        verifier,
    )?;
    Ok(())
}

/// Reads and decodes prepared invocation evidence through the artifact-read contract.
pub async fn read_prepared_invocation_artifact(
    artifacts: &dyn ArtifactReadProvider,
    evidence: CapabilityArtifactEvidenceRef,
) -> Result<PreparedContractInvocation> {
    let request = ArtifactReadRequest::from_replay_authorized_evidence(evidence);
    let verified = artifacts
        .read_artifact(&request)
        .await
        .map_err(EvmContractAdapterError::ArtifactRead)?;
    let prepared = verified
        .decode_json::<PreparedContractInvocation>()
        .map_err(EvmContractAdapterError::ArtifactRead)?;
    ensure_prepared_invocation_public(&prepared)?;
    Ok(prepared)
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
    for forbidden in forbidden_prepared_terms() {
        if json.contains(&forbidden) {
            return Err(EvmContractAdapterError::PreparedInvocationLeak);
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ContractLifecycleReplayFrames {
    intent: side_effect::IntentPersisted,
    submission: Option<side_effect::SubmissionObserved>,
    receipt: Option<side_effect::ReceiptObserved>,
    confirmation: Option<side_effect::ConfirmationObserved>,
}

fn is_contract_lifecycle_intent(intent: &side_effect::IntentPersisted) -> replay::Result<bool> {
    let binding = evm_contract_lifecycle_adapter_binding().map_err(replay_adapter_error)?;
    Ok(intent.adapter_kind == *binding.adapter_kind()
        && intent.adapter_version == *binding.adapter_version())
}

fn side_effect_replay_request(
    intent: &side_effect::IntentPersisted,
    evidence_schema_id: SchemaId,
    evidence_hash: ContentDigest,
    replay_verifier_id: Option<events::ReplayVerifierId>,
) -> replay::SideEffectEvidenceReplayRequest {
    replay::SideEffectEvidenceReplayRequest {
        ledger_key: intent.ledger_key.clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        invocation_epoch: intent.invocation_epoch,
        intent_schema_id: intent.intent_schema_id.clone(),
        intent_hash: intent.intent_hash.clone(),
        idempotency_input_schema_id: intent.idempotency_input_schema_id.clone(),
        idempotency_input_hash: intent.idempotency_input_hash.clone(),
        capability_kind: intent.capability_kind.clone(),
        capability_version: intent.capability_version.clone(),
        adapter_kind: intent.adapter_kind.clone(),
        adapter_version: intent.adapter_version.clone(),
        evidence_schema_id,
        evidence_hash,
        replay_verifier_id,
    }
}

fn contract_lifecycle_side_effect_missing(phase: &str) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMissing,
        format!("missing contract lifecycle {phase} evidence"),
    )
}

fn replay_contract_adapter_error(error: EvmContractAdapterError) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::SideEffectMismatch,
        error.to_string(),
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
                )
                .map_err(EvmContractAdapterError::EvmSigning)?
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
                )
                .map_err(EvmContractAdapterError::EvmSigning)?
            }
        };
        if transaction.signing_digest != format!("{:?}", signing_request.signing_hash()) {
            return Err(EvmContractAdapterError::InvalidPreparedInvocation);
        }
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

fn state_error(error: mfm_program::PlanError) -> EvmContractAdapterError {
    EvmContractAdapterError::State(error.to_string())
}

fn state_runtime_error(error: mfm_program::StateError) -> EvmContractAdapterError {
    EvmContractAdapterError::State(error.to_string())
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

fn forbidden_prepared_terms() -> Vec<String> {
    vec![
        ["raw", "_transaction"].concat(),
        "signature".to_owned(),
        ["key", "store"].concat(),
        ["private", "_key"].concat(),
        ["pass", "word"].concat(),
        ["rpc", "_url"].concat(),
        ["end", "point"].concat(),
        ["provider", "_kind"].concat(),
        ["author", "ization"].concat(),
    ]
}

const READ_FACTORY: &str = "read_external";
const SIDE_EFFECT_FACTORY: &str = "apply_side_effect";
const CAPABILITY_IMPLEMENTATION_ID: &str = "mfm.evm_contracts.runtime.v1";

/// EVM capability provider set required by contract lifecycle runners.
///
/// Concrete implementations are supplied by app assembly or tests; this trait
/// only bundles the capability contracts the adapter needs.
pub trait EvmContractProvider:
    EvmChainIdentityProvider
    + EvmNonceReadProvider
    + EvmFeeReadProvider
    + EvmGasEstimateProvider
    + EvmTransactionSubmitProvider
    + EvmReceiptReadProvider
    + EvmCallReadProvider
    + EvmLogsReadProvider
{
}

impl<T> EvmContractProvider for T where
    T: EvmChainIdentityProvider
        + EvmNonceReadProvider
        + EvmFeeReadProvider
        + EvmGasEstimateProvider
        + EvmTransactionSubmitProvider
        + EvmReceiptReadProvider
        + EvmCallReadProvider
        + EvmLogsReadProvider
{
}

/// Factory for per-network EVM contract runtime bindings.
pub trait EvmContractRuntimeFactory: Send + Sync {
    /// Returns the artifact reader used to materialize configs, inputs, and side-effect evidence.
    fn artifacts(&self) -> &dyn ArtifactReadProvider;

    /// Returns process-local EVM and signing capabilities for `network_id`.
    fn runtime_for(&self, network_id: &str) -> mfm_runtime::Result<EvmContractRuntime>;
}

/// Process-local runtime capability bundle for one EVM contract lifecycle route.
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
                nonce: evm,
                fee: evm,
                gas: evm,
                signer: self.signer.as_ref(),
                submit: evm,
                receipt: evm,
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
    let deploy = registered_descriptor::<DeployContractState>()?;
    registry.register_capability_set(&deploy.capabilities, implementation_id.clone())?;
    registry.register(binding(
        deploy.descriptor_id,
        SIDE_EFFECT_FACTORY,
        Arc::new(ContractMutationRunner {
            phase: ContractMutationRunnerPhase::Deploy,
            factory: factory.clone(),
        }),
    )?)?;
    let configure = registered_descriptor::<ConfigureContractState>()?;
    registry.register_capability_set(&configure.capabilities, implementation_id.clone())?;
    registry.register(binding(
        configure.descriptor_id,
        SIDE_EFFECT_FACTORY,
        Arc::new(ContractMutationRunner {
            phase: ContractMutationRunnerPhase::Configure,
            factory: factory.clone(),
        }),
    )?)?;
    let validate = registered_descriptor::<ValidateContractState>()?;
    registry.register_capability_set(&validate.capabilities, implementation_id)?;
    registry.register(binding(
        validate.descriptor_id,
        READ_FACTORY,
        Arc::new(ContractValidateRunner { factory }),
    )?)?;
    Ok(())
}

struct RegisteredRuntimeDescriptor {
    descriptor_id: DescriptorId,
    capabilities: CapabilitySetDescriptor,
}

fn registered_descriptor<S>() -> mfm_runtime::Result<RegisteredRuntimeDescriptor>
where
    S: StateSpec,
    S::Effect: mfm_program::EffectRunner<S>,
    S::Caps: mfm_capabilities::CapabilitySetFor<S::Effect>,
{
    let mut states = mfm_program::StateRegistryBuilder::new();
    let registered = states
        .register::<S>()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(RegisteredRuntimeDescriptor {
        descriptor_id: registered.descriptor().descriptor_id().clone(),
        capabilities: registered.descriptor().capabilities().clone(),
    })
}

fn binding(
    descriptor_id: DescriptorId,
    factory: &'static str,
    runner: Arc<dyn ErasedNodeRunner>,
) -> mfm_runtime::Result<ErasedRunnerBinding> {
    let factory_id = events::RunnerFactoryId::new(factory)?;
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        executable(factory_id)?,
        runner,
    )
}

fn executable(
    factory_id: events::RunnerFactoryId,
) -> mfm_runtime::Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-adapters-evm-contracts-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-adapters-evm-contracts")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
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
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { run_validate(ctx, self.factory.as_ref()).await })
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

struct DeployMutationPlan {
    config: ValidatedConfig<DeployPhaseConfig>,
    state: DeployContractState,
    intent: ContractDeployIntent,
    idempotency: ContractTransactionIdempotency,
    ledger_key: events::SideEffectLedgerKey,
}

struct ConfigureMutationPlan {
    config: ValidatedConfig<ConfigurePhaseConfig>,
    state: ConfigureContractState,
    input: ConfigureContractInput,
    intent: ContractConfigureIntent,
    idempotency: ContractTransactionIdempotency,
    ledger_key: events::SideEffectLedgerKey,
}

async fn run_deploy_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let plan = deploy_mutation_plan(&ctx, factory.artifacts()).await?;
    match ctx
        .projections()
        .side_effect(&plan.ledger_key)
        .map(|projection| projection.phase.clone())
    {
        None => {
            let runtime = factory.runtime_for(plan.config.as_ref().network().network_id())?;
            let prepared = runtime
                .adapter()
                .prepare_deploy_invocation(&plan.config, &plan.intent)
                .await
                .map_err(runtime_adapter_error)?;
            side_effect_prepare(
                ctx,
                plan.ledger_key,
                &plan.intent,
                &plan.idempotency,
                prepared.evidence(),
            )
        }
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let stored_prepared = load_prepared_invocation(
                &prepared_projection,
                events::ArtifactRole::PreparedInvocation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let runtime = factory.runtime_for(plan.config.as_ref().network().network_id())?;
            let prepared = runtime
                .adapter()
                .reconstruct_deploy_invocation(&plan.config, &plan.intent, &stored_prepared)
                .map_err(runtime_adapter_error)?;
            side_effect_submission(ctx, &runtime, plan.ledger_key, invocation_epoch, &prepared)
                .await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let submission_projection = projected_submission_artifact(&ctx, &plan.ledger_key)?;
            let runtime = factory.runtime_for(plan.config.as_ref().network().network_id())?;
            side_effect_receipt(
                ctx,
                factory.artifacts(),
                &runtime,
                plan.ledger_key,
                invocation_epoch,
                &prepared_projection,
                &submission_projection,
            )
            .await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let receipt_projection = projected_receipt_artifact(&ctx, &plan.ledger_key)?;
            let prepared = load_prepared_invocation(
                &prepared_projection,
                events::ArtifactRole::PreparedInvocation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let (receipts, receipt_evidence) =
                load_side_effect_artifact::<ContractTransactionReceipts>(
                    &receipt_projection,
                    events::ArtifactRole::Receipt,
                    &ctx,
                    factory.artifacts(),
                )
                .await?;
            let receipts = receipts_with_evidence(receipts, &receipt_evidence);
            let receipt = single_receipt(receipts)?;
            let confirmation = ContractDeployConfirmation {
                confirmation_version: 1,
                contract_address: deploy_contract_address_from_prepared(&prepared)
                    .map_err(runtime_adapter_error)?,
                receipt,
            };
            side_effect_confirmation(ctx, plan.ledger_key, invocation_epoch, &confirmation)
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => {
            let confirmation_projection = projected_confirmation_artifact(&ctx, &plan.ledger_key)?;
            let confirmation = load_side_effect_value::<ContractDeployConfirmation>(
                &confirmation_projection,
                events::ArtifactRole::Confirmation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let output = plan
                .state
                .output_from_confirmation(&(), &plan.intent, &confirmation)
                .map_err(runtime_state_error)?;
            mutation_output(ctx, &output)
        }
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => unsupported_side_effect_phase("deploy", other),
    }
}

async fn run_configure_mutation(
    ctx: ErasedRunCtx<'_>,
    factory: &dyn EvmContractRuntimeFactory,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let plan = configure_mutation_plan(&ctx, factory.artifacts()).await?;
    match ctx
        .projections()
        .side_effect(&plan.ledger_key)
        .map(|projection| projection.phase.clone())
    {
        None => {
            let runtime = factory.runtime_for(plan.config.as_ref().network().network_id())?;
            let prepared = runtime
                .adapter()
                .prepare_configure_invocation(&plan.config, &plan.input, &plan.intent)
                .await
                .map_err(runtime_adapter_error)?;
            side_effect_prepare(
                ctx,
                plan.ledger_key,
                &plan.intent,
                &plan.idempotency,
                prepared.evidence(),
            )
        }
        Some(store::SideEffectPhase::InvocationStarted {
            invocation_epoch, ..
        })
        | Some(store::SideEffectPhase::SubmissionUnknown { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let stored_prepared = load_prepared_invocation(
                &prepared_projection,
                events::ArtifactRole::PreparedInvocation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let runtime = factory.runtime_for(plan.config.as_ref().network().network_id())?;
            let prepared = runtime
                .adapter()
                .reconstruct_configure_invocation(
                    &plan.config,
                    &plan.input,
                    &plan.intent,
                    &stored_prepared,
                )
                .map_err(runtime_adapter_error)?;
            side_effect_submission(ctx, &runtime, plan.ledger_key, invocation_epoch, &prepared)
                .await
        }
        Some(store::SideEffectPhase::SubmissionObserved { invocation_epoch }) => {
            let prepared_projection = projected_prepared_artifact(&ctx, &plan.ledger_key)?;
            let submission_projection = projected_submission_artifact(&ctx, &plan.ledger_key)?;
            let runtime = factory.runtime_for(plan.config.as_ref().network().network_id())?;
            side_effect_receipt(
                ctx,
                factory.artifacts(),
                &runtime,
                plan.ledger_key,
                invocation_epoch,
                &prepared_projection,
                &submission_projection,
            )
            .await
        }
        Some(store::SideEffectPhase::ReceiptObserved { invocation_epoch }) => {
            let receipt_projection = projected_receipt_artifact(&ctx, &plan.ledger_key)?;
            let (receipts, receipt_evidence) =
                load_side_effect_artifact::<ContractTransactionReceipts>(
                    &receipt_projection,
                    events::ArtifactRole::Receipt,
                    &ctx,
                    factory.artifacts(),
                )
                .await?;
            let receipts = receipts_with_evidence(receipts, &receipt_evidence);
            let confirmation = ContractConfigureConfirmation {
                confirmation_version: 1,
                configured_block_number: receipts
                    .transactions
                    .iter()
                    .map(|receipt| receipt.block_number)
                    .max(),
                receipts: receipts.transactions,
            };
            side_effect_confirmation(ctx, plan.ledger_key, invocation_epoch, &confirmation)
        }
        Some(store::SideEffectPhase::ConfirmationObserved { .. }) => {
            let confirmation_projection = projected_confirmation_artifact(&ctx, &plan.ledger_key)?;
            let confirmation = load_side_effect_value::<ContractConfigureConfirmation>(
                &confirmation_projection,
                events::ArtifactRole::Confirmation,
                &ctx,
                factory.artifacts(),
            )
            .await?;
            let output = plan
                .state
                .output_from_confirmation(&plan.input, &plan.intent, &confirmation)
                .map_err(runtime_state_error)?;
            mutation_output(ctx, &output)
        }
        Some(store::SideEffectPhase::Ambiguous { .. }) => Ok(ErasedRunnerOutput::new(Vec::new())),
        Some(other) => unsupported_side_effect_phase("configure", other),
    }
}

async fn deploy_mutation_plan(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<DeployMutationPlan> {
    let config = load_config::<DeployPhaseConfig>(ctx, artifacts).await?;
    let state = DeployContractState::new(config.clone()).map_err(runtime_plan_error)?;
    let intent = state.prepare_intent(&()).map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&(), &intent)
        .map_err(runtime_state_error)?;
    let ledger_key = ledger_key_for_idempotency(&idempotency)?;
    Ok(DeployMutationPlan {
        config,
        state,
        intent,
        idempotency,
        ledger_key,
    })
}

async fn configure_mutation_plan(
    ctx: &ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
) -> mfm_runtime::Result<ConfigureMutationPlan> {
    let config = load_config::<ConfigurePhaseConfig>(ctx, artifacts).await?;
    let deployed =
        load_struct_input_value::<DeployedContract>(ctx.inputs(), "deployed", artifacts).await?;
    let input = ConfigureContractInput { deployed };
    let state = ConfigureContractState::new(config.clone()).map_err(runtime_plan_error)?;
    let intent = state.prepare_intent(&input).map_err(runtime_state_error)?;
    let idempotency = state
        .idempotency_input(&input, &intent)
        .map_err(runtime_state_error)?;
    let ledger_key = ledger_key_for_idempotency(&idempotency)?;
    Ok(ConfigureMutationPlan {
        config,
        state,
        input,
        intent,
        idempotency,
        ledger_key,
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
        .await
        .map_err(runtime_adapter_error)?;
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

fn side_effect_prepare<Intent>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    intent: &Intent,
    idempotency: &ContractTransactionIdempotency,
    prepared: &PreparedContractInvocation,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Intent: MfmValue + Serialize,
{
    let intent_artifact = artifact_for_value(
        intent,
        events::ArtifactRole::SideEffectIntent,
        Some(ctx.node().node_id.clone()),
    )?;
    let prepared_artifact = artifact_for_schema_less_json(
        prepared,
        events::ArtifactRole::PreparedInvocation,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_intent = staged_side_effect_artifact(&ctx, &intent_artifact, ledger_key.clone(), 1)?;
    let staged_prepared =
        staged_side_effect_artifact(&ctx, &prepared_artifact, ledger_key.clone(), 1)?;
    let idempotency_hash = digest_value(idempotency)?;
    let owner = runner_invocation_id(&ctx, &ledger_key)?;
    let token = claim_fencing_token(&ctx, &ledger_key)?;
    let ledger_purpose = events::SideEffectLedgerPurpose::Forward;
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_intent, staged_prepared],
        staged_retention_refs: vec![
            retention(&intent_artifact.evidence),
            retention(&prepared_artifact.evidence),
        ],
        payloads: vec![
            RunnerEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                scope_id: ctx.node().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                invocation_epoch: 1,
                intent_schema_id: Intent::schema_id().map_err(runtime_value_error)?,
                intent_hash: intent_artifact.evidence.digest.clone(),
                intent_artifact_id: intent_artifact.evidence.artifact_id.clone(),
                idempotency_input_schema_id: ContractTransactionIdempotency::schema_id()
                    .map_err(runtime_value_error)?,
                idempotency_input_hash: idempotency_hash,
                idempotency_key: idempotency_key_ref(idempotency)?,
                capability_kind: EvmTransactionSubmitCapability::kind()
                    .map_err(runtime_capability_error)?,
                capability_version: EvmTransactionSubmitCapability::version()
                    .map_err(runtime_capability_error)?,
                adapter_kind: binding.adapter_kind().clone(),
                adapter_version: binding.adapter_version().clone(),
            }),
            RunnerEventPayload::SideEffectClaimed(side_effect::Claimed {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                claim_owner: owner.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            }),
            RunnerEventPayload::SideEffectInvocationPrepared(side_effect::InvocationPrepared {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger_key.clone(),
                ledger_purpose: ledger_purpose.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
                prepared_artifact_id: Some(prepared_artifact.evidence.artifact_id.clone()),
                prepared_hash: Some(prepared_artifact.evidence.digest.clone()),
                resource_key: None,
            }),
            RunnerEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose,
                invocation_epoch: 1,
                claim_owner: owner,
                claim_generation: 1,
                claim_fencing_token: token,
            }),
        ],
    })
}

async fn side_effect_submission(
    ctx: ErasedRunCtx<'_>,
    runtime: &EvmContractRuntime,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    prepared: &PreparedContractMutation,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let submissions = ContractTransactionSubmissions {
        submissions_version: 1,
        transactions: runtime
            .adapter()
            .submit_prepared(prepared)
            .await
            .map_err(runtime_adapter_error)?,
    };
    let artifact = artifact_for_value(
        &submissions,
        events::ArtifactRole::Submission,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                invocation_epoch,
                submission_schema_id: ContractTransactionSubmissions::schema_id()
                    .map_err(runtime_value_error)?,
                submission_hash: artifact.evidence.digest,
                submission_artifact_id: artifact.evidence.artifact_id,
            },
        )],
    })
}

async fn side_effect_receipt(
    ctx: ErasedRunCtx<'_>,
    artifacts: &dyn ArtifactReadProvider,
    runtime: &EvmContractRuntime,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    prepared_projection: &store::SideEffectArtifactProjection,
    submission_projection: &store::SideEffectArtifactProjection,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let prepared = load_prepared_invocation(
        prepared_projection,
        events::ArtifactRole::PreparedInvocation,
        &ctx,
        artifacts,
    )
    .await?;
    let submissions = load_side_effect_value::<ContractTransactionSubmissions>(
        submission_projection,
        events::ArtifactRole::Submission,
        &ctx,
        artifacts,
    )
    .await?;
    let receipts = ContractTransactionReceipts {
        receipts_version: 1,
        transactions: read_receipts_with_poll(runtime, &prepared, &submissions).await?,
    };
    let artifact = artifact_for_value(
        &receipts,
        events::ArtifactRole::Receipt,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                invocation_epoch,
                receipt_schema_id: ContractTransactionReceipts::schema_id()
                    .map_err(runtime_value_error)?,
                receipt_hash: artifact.evidence.digest,
                receipt_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: replay_verifier_id().map_err(runtime_adapter_error)?,
                resource_touched_set: None,
            },
        )],
    })
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
            Err(error) => return Err(runtime_adapter_error(error)),
        }
    }

    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
        "contract lifecycle receipt polling exhausted".to_owned(),
    ))
}

fn side_effect_confirmation<Confirmation>(
    ctx: ErasedRunCtx<'_>,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    confirmation: &Confirmation,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Confirmation: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        confirmation,
        events::ArtifactRole::Confirmation,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged =
        staged_side_effect_artifact(&ctx, &artifact, ledger_key.clone(), invocation_epoch)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![RunnerEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key,
                ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                invocation_epoch,
                confirmation_schema_id: Confirmation::schema_id().map_err(runtime_value_error)?,
                confirmation_hash: artifact.evidence.digest,
                confirmation_artifact_id: artifact.evidence.artifact_id,
                replay_verifier_id: replay_verifier_id().map_err(runtime_adapter_error)?,
                resource_touched_set: None,
            },
        )],
    })
}

fn mutation_output<Output>(
    ctx: ErasedRunCtx<'_>,
    output: &Output,
) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    Output: MfmValue + Serialize,
{
    let artifact = artifact_for_value(
        output,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged = staged_attempt_artifact(&ctx, &artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged],
        staged_retention_refs: vec![retention(&artifact.evidence)],
        payloads: vec![cell_produced(&ctx, &artifact.evidence)],
    })
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
    let request = ArtifactReadRequest::from_certified_config_ref(&ctx.node().config_ref);
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
    let prepared =
        load_side_effect_value::<PreparedContractInvocation>(artifact, role, ctx, artifacts)
            .await?;
    ensure_prepared_invocation_public(&prepared).map_err(runtime_adapter_error)?;
    Ok(prepared)
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
    let (value, _) = load_side_effect_artifact(artifact, role, ctx, artifacts).await?;
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
    let request = ArtifactReadRequest::from_side_effect_projection(
        artifact,
        role,
        ctx.node().node_id.clone(),
    );
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
    let request_hash = digest_value(&request)?;
    let response_artifact = artifact_for_value(
        &response,
        events::ArtifactRole::FactResponse,
        Some(ctx.node().node_id.clone()),
    )?;
    let output_artifact = artifact_for_value(
        &output,
        events::ArtifactRole::StateOutput,
        Some(ctx.node().node_id.clone()),
    )?;
    let staged_response = staged_attempt_artifact(&ctx, &response_artifact)?;
    let staged_output = staged_attempt_artifact(&ctx, &output_artifact)?;
    let binding = evm_contract_lifecycle_adapter_binding()
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
    let mut payloads = Vec::with_capacity(fact_capabilities.len() + 1);
    for fact in fact_capabilities {
        payloads.push(RunnerEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            capability_kind: fact.kind,
            capability_version: fact.version,
            adapter_kind: binding.adapter_kind().clone(),
            adapter_version: binding.adapter_version().clone(),
            request_schema_id: Request::schema_id().map_err(runtime_value_error)?,
            request_hash: request_hash.clone(),
            response_schema_id: Response::schema_id().map_err(runtime_value_error)?,
            response_hash: response_artifact.evidence.digest.clone(),
            fact_key: events::FactKey::new(format!(
                "mfm.evm.contract.fact.{}.{}",
                ctx.node().node_id.as_str(),
                fact.key_suffix
            ))?,
            artifact_id: response_artifact.evidence.artifact_id.clone(),
        }));
    }
    payloads.push(cell_produced(&ctx, &output_artifact.evidence));
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_response, staged_output],
        staged_retention_refs: vec![
            retention(&response_artifact.evidence),
            retention(&output_artifact.evidence),
        ],
        payloads,
    })
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
            artifact_id,
            content_digest,
        } => ArtifactReadRequest::from_materialized_produced_cell(
            artifact_id.clone(),
            content_digest.clone(),
            cell.schema_id.clone(),
            cell.semantic_type_id.clone(),
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

fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &ContractAppArtifact,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_attempt_artifact(ctx, artifact.bytes.clone(), artifact.evidence.clone())
}

fn staged_side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact: &ContractAppArtifact,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> mfm_runtime::Result<StagedArtifact> {
    StagedArtifact::inline_side_effect_artifact(
        ctx,
        artifact.bytes.clone(),
        artifact.evidence.clone(),
        ledger_key,
        invocation_epoch,
    )
}

fn cell_produced(
    ctx: &ErasedRunCtx<'_>,
    artifact: &store::ArtifactEvidenceRef,
) -> RunnerEventPayload {
    RunnerEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.output_cell().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
        schema_id: ctx.output_cell().schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        artifact_id: artifact.artifact_id.clone(),
        content_digest: artifact.digest.clone(),
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    })
}

struct ContractAppArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn artifact_for_value<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<ContractAppArtifact>
where
    T: MfmValue + Serialize,
{
    let bytes = canonical_value(value)?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(T::schema_id().map_err(runtime_value_error)?),
        semantic_type_id: Some(T::semantic_id().map_err(runtime_value_error)?),
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    };
    Ok(ContractAppArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn artifact_for_schema_less_json<T>(
    value: &T,
    role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
) -> mfm_runtime::Result<ContractAppArtifact>
where
    T: Serialize,
{
    let bytes = canonical_value(value)?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id,
        producer_seed_id: None,
        artifact_role: role,
    };
    Ok(ContractAppArtifact {
        bytes: bytes.to_vec(),
        evidence,
    })
}

fn retention(artifact: &store::ArtifactEvidenceRef) -> StagedRetentionRefs {
    StagedRetentionRefs::runtime_evidence(vec![events::RetentionRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        content_digest: artifact.digest.clone(),
    }])
}

fn projected_side_effect<'a>(
    ctx: &'a ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<&'a store::SideEffectProjection> {
    ctx.projections().side_effect(ledger_key).ok_or_else(|| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
            "contract lifecycle side-effect projection missing for ledger {ledger_key}"
        ))
    })
}

fn projected_prepared_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .prepared_invocation
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("prepared invocation"))
}

fn projected_submission_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .submission
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("submission"))
}

fn projected_receipt_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .receipt
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("receipt"))
}

fn projected_confirmation_artifact(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<store::SideEffectArtifactProjection> {
    projected_side_effect(ctx, ledger_key)?
        .confirmation
        .clone()
        .ok_or_else(|| missing_side_effect_artifact("confirmation"))
}

fn missing_side_effect_artifact(label: &str) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "contract lifecycle side-effect {label} artifact is missing"
    ))
}

fn single_receipt(
    receipts: ContractTransactionReceipts,
) -> mfm_runtime::Result<mfm_state_evm_contracts::ContractTransactionReceipt> {
    let mut transactions = receipts.transactions;
    if transactions.len() != 1 {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "contract deploy confirmation requires exactly one receipt".to_owned(),
        ));
    }
    Ok(transactions.remove(0))
}

fn receipts_with_evidence(
    mut receipts: ContractTransactionReceipts,
    evidence: &mfm_artifact_capabilities::ArtifactEvidenceRef,
) -> ContractTransactionReceipts {
    let evidence = lifecycle_evidence_ref(evidence);
    for receipt in &mut receipts.transactions {
        receipt.receipt_evidence = Some(evidence.clone());
    }
    receipts
}

fn ledger_key_for_idempotency(
    idempotency: &ContractTransactionIdempotency,
) -> mfm_runtime::Result<events::SideEffectLedgerKey> {
    events::SideEffectLedgerKey::new(format!(
        "mfm.evm.contract.ledger.{}",
        short_stable_key(&idempotency.key)
    ))
    .map_err(Into::into)
}

fn idempotency_key_ref(
    idempotency: &ContractTransactionIdempotency,
) -> mfm_runtime::Result<events::IdempotencyKeyRef> {
    events::IdempotencyKeyRef::new(format!(
        "mfm.evm.contract.idem.{}",
        short_stable_key(&idempotency.key)
    ))
    .map_err(Into::into)
}

fn runner_invocation_id(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<events::RunnerInvocationId> {
    let digest = digest_json(serde_json::json!({
        "attempt_id": ctx.attempt_id().as_str(),
        "ledger_key": ledger_key.as_str(),
        "node_id": ctx.node().node_id.as_str(),
        "run_id": ctx.run_id().as_str(),
    }))?;
    events::RunnerInvocationId::new(format!("mfm.evm.contract.owner.{}", short_digest(&digest)))
        .map_err(Into::into)
}

fn claim_fencing_token(
    ctx: &ErasedRunCtx<'_>,
    ledger_key: &events::SideEffectLedgerKey,
) -> mfm_runtime::Result<side_effect::ClaimFencingToken> {
    let digest = digest_json(serde_json::json!({
        "attempt_id": ctx.attempt_id().as_str(),
        "ledger_key": ledger_key.as_str(),
        "node_id": ctx.node().node_id.as_str(),
        "token": "contract-lifecycle",
    }))?;
    side_effect::ClaimFencingToken::new(format!("mfm.evm.contract.token.{}", short_digest(&digest)))
        .map_err(Into::into)
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

fn short_digest(digest: &ContentDigest) -> String {
    short_stable_key(digest.as_str())
}

fn unsupported_side_effect_phase(
    phase: &'static str,
    current: store::SideEffectPhase,
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(format!(
        "unsupported contract lifecycle {phase} side-effect phase: {current:?}"
    )))
}

fn canonical_value<T: Serialize>(value: &T) -> mfm_runtime::Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))
}

fn digest_value<T: Serialize>(value: &T) -> mfm_runtime::Result<ContentDigest> {
    Ok(canonical_value(value)?.content_digest())
}

fn digest_json(value: serde_json::Value) -> mfm_runtime::Result<ContentDigest> {
    let json = serde_json::to_string(&value)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?;
    Ok(PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| mfm_runtime::RuntimeError::Canonical(error.to_string()))?
        .content_digest())
}

fn runtime_value_error(error: mfm_values::ValueError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_capability_error(error: mfm_capabilities::CapabilityError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_artifact_read_error(
    error: mfm_artifact_capabilities::ArtifactReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_adapter_error(error: EvmContractAdapterError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_plan_error(error: mfm_program::PlanError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_state_error(error: mfm_program::StateError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_evm_capabilities::{
        EvmCallReadResponse, EvmCapabilityFuture, EvmChainIdentityResponse, EvmFeeReadResponse,
        EvmGasEstimateResponse, EvmLogsReadResponse, EvmNonceReadResponse, EvmReceiptReadResponse,
        RedactedEvmSourceEvidence,
    };
    use mfm_evm_signing::EvmTransactionStyle as SigningTransactionStyle;
    use mfm_replay::v1::SideEffectReplayVerifier;
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
        serde_json::from_value(json!({
            "artifact": artifact_json(),
            "network": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1
            },
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": "0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e"
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

    struct TestEvmProviders;

    impl EvmChainIdentityProvider for TestEvmProviders {
        fn chain_identity<'a>(
            &'a self,
            _request: &'a EvmChainIdentityRequest,
        ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
            Box::pin(async {
                Ok(EvmChainIdentityResponse {
                    evidence: evidence(),
                    chain_id: 1,
                    client_version: Some("test-client".to_owned()),
                })
            })
        }
    }

    impl EvmNonceReadProvider for TestEvmProviders {
        fn read_nonce<'a>(
            &'a self,
            _request: &'a EvmNonceReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
            Box::pin(async {
                Ok(EvmNonceReadResponse {
                    evidence: evidence(),
                    nonce: 7,
                })
            })
        }
    }

    impl EvmFeeReadProvider for TestEvmProviders {
        fn read_fee<'a>(
            &'a self,
            _request: &'a EvmFeeReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
            Box::pin(async {
                Ok(EvmFeeReadResponse {
                    evidence: evidence(),
                    base_fee_per_gas: Some(5),
                    priority_fee_per_gas: Some(3),
                    max_fee_per_gas: Some(11),
                    legacy_gas_price: Some(7),
                })
            })
        }
    }

    impl EvmGasEstimateProvider for TestEvmProviders {
        fn estimate_gas<'a>(
            &'a self,
            _request: &'a EvmGasEstimateRequest,
        ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
            Box::pin(async {
                Ok(EvmGasEstimateResponse {
                    evidence: evidence(),
                    gas_limit: 21_000,
                })
            })
        }
    }

    impl EvmTransactionSubmitProvider for TestEvmProviders {
        fn submit_transaction<'a>(
            &'a self,
            _request: &'a EvmTransactionSubmitRequest,
        ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmTransactionSubmitResponse> {
            Box::pin(async { panic!("not used in preparation tests") })
        }
    }

    impl EvmReceiptReadProvider for TestEvmProviders {
        fn read_receipt<'a>(
            &'a self,
            _request: &'a EvmReceiptReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
            Box::pin(async { panic!("not used in preparation tests") })
        }
    }

    impl EvmCallReadProvider for TestEvmProviders {
        fn read_call<'a>(
            &'a self,
            _request: &'a EvmCallReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
            Box::pin(async { panic!("not used in preparation tests") })
        }
    }

    impl EvmLogsReadProvider for TestEvmProviders {
        fn read_logs<'a>(
            &'a self,
            _request: &'a EvmLogsReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
            Box::pin(async { panic!("not used in preparation tests") })
        }
    }

    impl SigningProvider for TestEvmProviders {
        fn sign<'a>(
            &'a self,
            _request: &'a mfm_signing::SigningRequest,
        ) -> mfm_signing::SigningFuture<'a> {
            Box::pin(async { panic!("not used in preparation tests") })
        }
    }

    fn adapter<'a>(
        route: &'a EvmContractRuntimeRoute,
        providers: &'a TestEvmProviders,
    ) -> EvmContractLifecycleAdapter<'a> {
        EvmContractLifecycleAdapter::new(
            route,
            EvmContractMutationProviders {
                chain_identity: providers,
                nonce: providers,
                fee: providers,
                gas: providers,
                signer: providers,
                submit: providers,
                receipt: providers,
            },
            EvmContractReadProviders {
                chain_identity: providers,
                call: providers,
                logs: providers,
            },
        )
    }

    #[tokio::test]
    async fn deploy_preparation_defaults_to_eip1559_contract_creation() {
        let route = route();
        let providers = TestEvmProviders;
        let adapter = adapter(&route, &providers);
        let config = validated_deploy_config("eip1559");
        let intent = deploy_intent(&config);

        let prepared = adapter
            .prepare_deploy_invocation(&config, &intent)
            .await
            .expect("prepared");

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
        let route = route();
        let providers = TestEvmProviders;
        let adapter = adapter(&route, &providers);
        let config = validated_deploy_config("legacy");
        let intent = deploy_intent(&config);

        let prepared = adapter
            .prepare_deploy_invocation(&config, &intent)
            .await
            .expect("prepared");

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
        let route = route();
        let providers = TestEvmProviders;
        let adapter = adapter(&route, &providers);
        let config = validated_deploy_config("eip1559");
        let intent = deploy_intent(&config);
        let prepared = adapter
            .prepare_deploy_invocation(&config, &intent)
            .await
            .expect("prepared");

        ensure_prepared_invocation_public(prepared.evidence()).expect("public evidence");
        let rendered = serde_json::to_string(prepared.evidence()).expect("json");
        for forbidden in forbidden_prepared_terms() {
            assert!(
                !rendered.contains(&forbidden),
                "{rendered} contains {forbidden}"
            );
        }
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
        let providers = Arc::new(ReceiptFailureProviders {
            reads: Arc::clone(&reads),
        });
        let evm: Arc<dyn EvmContractProvider> = providers.clone();
        let signer: Arc<dyn SigningProvider> = providers;
        let runtime = EvmContractRuntime::new(route(), evm, signer);
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

    struct ReceiptFailureProviders {
        reads: Arc<Mutex<u32>>,
    }

    impl EvmChainIdentityProvider for ReceiptFailureProviders {
        fn chain_identity<'a>(
            &'a self,
            _request: &'a EvmChainIdentityRequest,
        ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
    }

    impl EvmNonceReadProvider for ReceiptFailureProviders {
        fn read_nonce<'a>(
            &'a self,
            _request: &'a EvmNonceReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
    }

    impl EvmFeeReadProvider for ReceiptFailureProviders {
        fn read_fee<'a>(
            &'a self,
            _request: &'a EvmFeeReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
    }

    impl EvmGasEstimateProvider for ReceiptFailureProviders {
        fn estimate_gas<'a>(
            &'a self,
            _request: &'a EvmGasEstimateRequest,
        ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
    }

    impl EvmTransactionSubmitProvider for ReceiptFailureProviders {
        fn submit_transaction<'a>(
            &'a self,
            _request: &'a EvmTransactionSubmitRequest,
        ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmTransactionSubmitResponse> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
    }

    impl EvmReceiptReadProvider for ReceiptFailureProviders {
        fn read_receipt<'a>(
            &'a self,
            _request: &'a EvmReceiptReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
            let reads = Arc::clone(&self.reads);
            Box::pin(async move {
                let mut reads = reads
                    .lock()
                    .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                *reads += 1;
                Err(EvmCapabilityError::redacted_provider_failure("test evm"))
            })
        }
    }

    impl EvmCallReadProvider for ReceiptFailureProviders {
        fn read_call<'a>(
            &'a self,
            _request: &'a EvmCallReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
    }

    impl EvmLogsReadProvider for ReceiptFailureProviders {
        fn read_logs<'a>(
            &'a self,
            _request: &'a EvmLogsReadRequest,
        ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
    }

    impl SigningProvider for ReceiptFailureProviders {
        fn sign<'a>(
            &'a self,
            _request: &'a mfm_signing::SigningRequest,
        ) -> mfm_signing::SigningFuture<'a> {
            Box::pin(async { panic!("not used in receipt polling test") })
        }
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
}
