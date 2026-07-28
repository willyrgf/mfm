//! Stateless, one-exchange EVM wallet target operations.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

use alloy_primitives::{Address, TxKind, B256, U256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{
    sign_eip1559_guarded, EvmBlockAnchor, EvmSubmitTransactionRequest, EvmWalletAccessListEntry,
    EvmWalletAttemptResult, EvmWalletBroadcastStatus, EvmWalletObservedTransaction,
    EvmWalletReceipt, EvmWalletReceiptLog, EvmWalletReceiptStatus, EvmWalletReference,
    EvmWalletTerminalEvidence, EvmWalletTransactionCandidate, EvmWalletTransactionPlacement,
    TransientSignedEip1559Envelope, EVM_WALLET_BROADCAST_OPERATION_ID,
    EVM_WALLET_FINALIZED_HEAD_OPERATION_ID, EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
    EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID, EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
    EVM_WALLET_TRANSACTION_TYPE,
};
use mfm_executor::{
    reference_safe_failure, BoundaryStage, CanonicalExecutorRequest, DeliveryAttemptOutcome,
    FailureClass, ReferenceFailureCode, SchemaQualifiedCanonicalValue, TargetEntryAuthority,
    TargetOperationReceipt,
};
use mfm_ids::{ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_program::{boundary_content_ref, encode_boundary};
use mfm_signing::GenerationGuardedDeterministicSigningProviderBinder;
use mfm_values::{MfmValue, RetainedValueContract};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use zeroize::Zeroizing;

use crate::transport::EvmJsonRpcTransport;

/// Exact already-known classifier descriptor version.
pub const EVM_ALREADY_KNOWN_CLASSIFIER_VERSION: &str = "mfm.evm-live.already-known-classifier.v1";
/// Exact five-method wallet target callback-surface version.
pub const EVM_WALLET_TARGET_CALLBACK_SURFACE_VERSION: &str =
    "mfm.evm-live.wallet-target-callback-surface.v1";
/// Exact durable target-entry descriptor version.
pub const EVM_WALLET_TARGET_ENTRY_DESCRIPTOR_VERSION: &str = "mfm.evm-live.wallet-target-entry.v1";

const EXACT_ALREADY_KNOWN_CODE: i64 = -32_000;
const EXACT_ALREADY_KNOWN_MESSAGE: &str = "already known";

/// Future returned by one raw wallet JSON-RPC exchange.
pub type EvmWalletRpcFuture<'a> =
    Pin<Box<dyn Future<Output = Result<EvmWalletRpcResponse, EvmWalletRpcFailure>> + Send + 'a>>;

/// Raw, transient result of one JSON-RPC exchange.
#[derive(Debug)]
pub enum EvmWalletRpcResponse {
    /// The envelope contained exactly one result.
    Result(Value),
    /// The envelope contained exactly one JSON-RPC error.
    Error(EvmWalletRpcError),
}

/// Transient JSON-RPC error fields used only for exact classification.
#[derive(Debug)]
pub struct EvmWalletRpcError {
    code: i64,
    message: String,
    exact_shape: bool,
}

impl EvmWalletRpcError {
    pub(crate) fn new(code: i64, message: String, exact_shape: bool) -> Self {
        Self {
            code,
            message,
            exact_shape,
        }
    }

    fn is_exact_already_known(&self) -> bool {
        self.exact_shape
            && self.code == EXACT_ALREADY_KNOWN_CODE
            && self.message == EXACT_ALREADY_KNOWN_MESSAGE
    }
}

/// Closed redaction-safe failure of one raw exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmWalletRpcFailure {
    /// The immutable route generation is absent or mismatched.
    GenerationFenced,
    /// Local access was cancelled before target entry.
    AccessCancelled,
    /// The destination was unavailable before target entry.
    UnavailableBeforeEntry,
    /// Target entry may have occurred but its response was lost.
    ResponseLost,
    /// The destination returned a non-classified rejection.
    DestinationRejected,
    /// The response could not satisfy the exact typed schema.
    InvalidResponse,
}

/// Minimal client contract: every call performs exactly one exchange.
pub trait EvmWalletRpcClient: Clone + Send + Sync + 'static {
    /// Performs one exchange against the exact immutable route generation.
    fn exchange<'a>(
        &'a self,
        route_generation_ref: &'a ContentRef,
        chain_id: u64,
        method: &'static str,
        params: Value,
    ) -> EvmWalletRpcFuture<'a>;
}

/// Qualified process-local guarded signer binding.
#[derive(Clone)]
pub struct EvmWalletSignerBinding {
    binding_ref: EvmWalletReference,
    binder: GenerationGuardedDeterministicSigningProviderBinder,
}

impl EvmWalletSignerBinding {
    /// Binds the opaque admitted signer identity to its guarded provider binder.
    pub fn new(
        binding_ref: EvmWalletReference,
        binder: GenerationGuardedDeterministicSigningProviderBinder,
    ) -> Result<Self, EvmWalletLiveError> {
        binding_ref
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        Ok(Self {
            binding_ref,
            binder,
        })
    }

    /// Returns the admitted signer-binding identity.
    pub const fn binding_ref(&self) -> &EvmWalletReference {
        &self.binding_ref
    }

    /// Returns the one durable generation accepted by guarded signing and target entry.
    pub fn durable_generation_ref(&self) -> &ContentRef {
        self.binder.binding().durable_generation_ref()
    }

    /// Returns the deployment fence checked by every guarded signing call.
    pub fn fence_attestation_ref(&self) -> &ContentRef {
        self.binder.binding().fence_attestation_ref()
    }
}

impl fmt::Debug for EvmWalletSignerBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EvmWalletSignerBinding")
            .field("binding_ref", &self.binding_ref)
            .field("binder", &"<guarded>")
            .finish()
    }
}

/// Redaction-safe local wallet target failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvmWalletLiveError {
    /// A domain, signer, operation, or retained-value contract is inconsistent.
    #[error("EVM wallet live contract is invalid")]
    InvalidContract,
    /// Canonical safe-result construction failed.
    #[error("EVM wallet safe result could not be retained")]
    ResultEncoding,
    /// The generation-guarded signer could not prepare transient bytes.
    #[error("EVM wallet signer is unavailable")]
    SignerUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EvmWalletRpcInputCommitment {
    RawTransaction {
        transaction_hash: String,
    },
    TransactionHash {
        transaction_hash: String,
    },
    FinalizedTag {
        block_tag: String,
        full_transactions: bool,
    },
    InclusionNumber {
        block_number: String,
        full_transactions: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvmWalletTargetEntryWire {
    version: String,
    operation_id: String,
    rpc_method: String,
    request_ref: EvmWalletReference,
    candidate_ref: EvmWalletReference,
    transaction_hash: String,
    allocated_nonce: String,
    fee_ordinal: u16,
    rpc_input: EvmWalletRpcInputCommitment,
}

/// Durable, candidate-specific commitment minted before one target entry.
///
/// This value contains the public candidate hash and exact non-secret RPC
/// input commitment, but never a signature, signed envelope, provider body,
/// endpoint, credential, or signer path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmWalletTargetEntryDescriptor {
    wire: EvmWalletTargetEntryWire,
    canonical: SchemaQualifiedCanonicalValue,
}

impl EvmWalletTargetEntryDescriptor {
    /// Builds the exact raw-transaction entry descriptor.
    pub fn broadcast(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_BROADCAST_OPERATION_ID,
            "eth_sendRawTransaction",
            EvmWalletRpcInputCommitment::RawTransaction {
                transaction_hash: candidate.transaction_hash().to_owned(),
            },
        )
    }

    /// Builds the exact transaction-by-hash entry descriptor.
    pub fn transaction_lookup(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
            "eth_getTransactionByHash",
            EvmWalletRpcInputCommitment::TransactionHash {
                transaction_hash: candidate.transaction_hash().to_owned(),
            },
        )
    }

    /// Builds the exact receipt-by-hash entry descriptor.
    pub fn receipt_lookup(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
            "eth_getTransactionReceipt",
            EvmWalletRpcInputCommitment::TransactionHash {
                transaction_hash: candidate.transaction_hash().to_owned(),
            },
        )
    }

    /// Builds the exact finalized-tag entry descriptor.
    pub fn finalized_head(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
            "eth_getBlockByNumber",
            EvmWalletRpcInputCommitment::FinalizedTag {
                block_tag: "finalized".to_owned(),
                full_transactions: false,
            },
        )
    }

    /// Builds the exact inclusion-number entry descriptor.
    pub fn canonical_inclusion(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
        inclusion_number: U256,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
            "eth_getBlockByNumber",
            EvmWalletRpcInputCommitment::InclusionNumber {
                block_number: encode_quantity(inclusion_number),
                full_transactions: false,
            },
        )
    }

    fn new(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
        operation_id: &'static str,
        rpc_method: &'static str,
        rpc_input: EvmWalletRpcInputCommitment,
    ) -> Result<Self, EvmWalletLiveError> {
        require_candidate_request(request, candidate)?;
        let request_ref = request
            .canonical_request()
            .reference()
            .map(EvmWalletReference::from_content_ref)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let wire = EvmWalletTargetEntryWire {
            version: EVM_WALLET_TARGET_ENTRY_DESCRIPTOR_VERSION.to_owned(),
            operation_id: operation_id.to_owned(),
            rpc_method: rpc_method.to_owned(),
            request_ref,
            candidate_ref: candidate
                .reference()
                .map_err(|_| EvmWalletLiveError::InvalidContract)?,
            transaction_hash: candidate.transaction_hash().to_owned(),
            allocated_nonce: candidate
                .allocated_nonce()
                .map_err(|_| EvmWalletLiveError::InvalidContract)?
                .to_string(),
            fee_ordinal: candidate.fee_ordinal(),
            rpc_input,
        };
        let canonical = canonical_json(&wire)?;
        let canonical = SchemaQualifiedCanonicalValue::new(
            descriptor_schema_id("mfm.evm-live.wallet-target-entry")?,
            canonical.as_bytes(),
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let descriptor = Self { wire, canonical };
        descriptor.validate_for_request(request)?;
        if descriptor.candidate_ref()
            != &candidate
                .reference()
                .map_err(|_| EvmWalletLiveError::InvalidContract)?
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        Ok(descriptor)
    }

    /// Strictly reconstructs a retained candidate-specific descriptor.
    pub fn strict_decode(
        value: &SchemaQualifiedCanonicalValue,
    ) -> Result<Self, EvmWalletLiveError> {
        if value.schema_id() != &descriptor_schema_id("mfm.evm-live.wallet-target-entry")? {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        let wire = serde_json::from_slice::<EvmWalletTargetEntryWire>(value.as_bytes())
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let canonical = canonical_json(&wire)?;
        if canonical.as_bytes() != value.as_bytes() {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        let descriptor = Self {
            wire,
            canonical: value.clone(),
        };
        descriptor.validate_structure()?;
        Ok(descriptor)
    }

    /// Returns the exact schema-qualified descriptor retained by authorization.
    pub const fn canonical(&self) -> &SchemaQualifiedCanonicalValue {
        &self.canonical
    }

    /// Returns its exact content identity.
    pub fn content_ref(&self) -> Result<ContentRef, EvmWalletLiveError> {
        self.canonical
            .reference()
            .map_err(|_| EvmWalletLiveError::InvalidContract)
    }

    /// Returns the reviewed target-operation family.
    pub fn operation_id(&self) -> &str {
        &self.wire.operation_id
    }

    /// Returns the exact candidate content identity.
    pub const fn candidate_ref(&self) -> &EvmWalletReference {
        &self.wire.candidate_ref
    }

    /// Returns the exact signed transaction hash.
    pub fn transaction_hash(&self) -> &str {
        &self.wire.transaction_hash
    }

    /// Returns the permanently allocated account nonce.
    pub fn allocated_nonce(&self) -> Result<u64, EvmWalletLiveError> {
        self.wire
            .allocated_nonce
            .parse()
            .map_err(|_| EvmWalletLiveError::InvalidContract)
    }

    /// Returns the immutable fee-schedule ordinal.
    pub const fn fee_ordinal(&self) -> u16 {
        self.wire.fee_ordinal
    }

    /// Returns the exact inclusion number for an inclusion lookup.
    pub fn inclusion_number(&self) -> Result<Option<U256>, EvmWalletLiveError> {
        match &self.wire.rpc_input {
            EvmWalletRpcInputCommitment::InclusionNumber { block_number, .. } => Ok(Some(
                parse_quantity(block_number).ok_or(EvmWalletLiveError::InvalidContract)?,
            )),
            EvmWalletRpcInputCommitment::RawTransaction { .. }
            | EvmWalletRpcInputCommitment::TransactionHash { .. }
            | EvmWalletRpcInputCommitment::FinalizedTag { .. } => Ok(None),
        }
    }

    /// Reconstructs and revalidates the public candidate without signer IO.
    pub fn candidate(
        &self,
        request: &EvmSubmitTransactionRequest,
    ) -> Result<EvmWalletTransactionCandidate, EvmWalletLiveError> {
        self.validate_for_request(request)?;
        let candidate = EvmWalletTransactionCandidate::new(
            request.clone(),
            self.allocated_nonce()?,
            self.fee_ordinal(),
            parse_hash(self.transaction_hash()).ok_or(EvmWalletLiveError::InvalidContract)?,
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if candidate.reference().ok().as_ref() != Some(self.candidate_ref()) {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        Ok(candidate)
    }

    fn validate_for_request(
        &self,
        request: &EvmSubmitTransactionRequest,
    ) -> Result<(), EvmWalletLiveError> {
        self.validate_structure()?;
        if self.wire.request_ref.to_content_ref().ok().as_ref()
            != request.canonical_request().reference().ok().as_ref()
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), EvmWalletLiveError> {
        let allocated_nonce = self
            .wire
            .allocated_nonce
            .parse::<u64>()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if self.wire.version != EVM_WALLET_TARGET_ENTRY_DESCRIPTOR_VERSION
            || self.wire.request_ref.to_content_ref().is_err()
            || self.wire.candidate_ref.to_content_ref().is_err()
            || self.wire.allocated_nonce != allocated_nonce.to_string()
            || parse_hash(&self.wire.transaction_hash).is_none()
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        let expected = match (
            self.wire.operation_id.as_str(),
            self.wire.rpc_method.as_str(),
            &self.wire.rpc_input,
        ) {
            (
                EVM_WALLET_BROADCAST_OPERATION_ID,
                "eth_sendRawTransaction",
                EvmWalletRpcInputCommitment::RawTransaction { transaction_hash },
            )
            | (
                EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
                "eth_getTransactionByHash",
                EvmWalletRpcInputCommitment::TransactionHash { transaction_hash },
            )
            | (
                EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
                "eth_getTransactionReceipt",
                EvmWalletRpcInputCommitment::TransactionHash { transaction_hash },
            ) => transaction_hash == &self.wire.transaction_hash,
            (
                EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
                "eth_getBlockByNumber",
                EvmWalletRpcInputCommitment::FinalizedTag {
                    block_tag,
                    full_transactions,
                },
            ) => block_tag == "finalized" && !full_transactions,
            (
                EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
                "eth_getBlockByNumber",
                EvmWalletRpcInputCommitment::InclusionNumber {
                    block_number,
                    full_transactions,
                },
            ) => parse_quantity(block_number).is_some() && !full_transactions,
            _ => false,
        };
        if !expected {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        Ok(())
    }
}

/// Affine transient signed payload prepared before durable authorization.
///
/// The value is neither cloneable nor serializable. Dropping it zeroizes the
/// raw signed envelope bytes.
pub struct PreparedEvmWalletBroadcast {
    descriptor: EvmWalletTargetEntryDescriptor,
    candidate: EvmWalletTransactionCandidate,
    signed: TransientSignedEip1559Envelope,
}

impl PreparedEvmWalletBroadcast {
    /// Returns the public candidate committed by authorization.
    pub const fn candidate(&self) -> &EvmWalletTransactionCandidate {
        &self.candidate
    }

    /// Returns the durable candidate-specific entry descriptor.
    pub const fn descriptor(&self) -> &EvmWalletTargetEntryDescriptor {
        &self.descriptor
    }
}

impl fmt::Debug for PreparedEvmWalletBroadcast {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedEvmWalletBroadcast")
            .field("descriptor", &self.descriptor)
            .field("candidate", &self.candidate)
            .field("signed", &"<redacted-zeroizing>")
            .finish()
    }
}

/// Result of a broadcast target operation, including its public candidate.
pub struct EvmWalletBroadcastReturn {
    receipt: TargetOperationReceipt,
    candidate: EvmWalletTransactionCandidate,
}

impl EvmWalletBroadcastReturn {
    /// Returns the affine executor receipt.
    pub const fn receipt(&self) -> &TargetOperationReceipt {
        &self.receipt
    }

    /// Returns the candidate whose signed bytes were transiently submitted.
    pub const fn candidate(&self) -> &EvmWalletTransactionCandidate {
        &self.candidate
    }

    /// Splits the receipt and public candidate.
    pub fn into_parts(self) -> (TargetOperationReceipt, EvmWalletTransactionCandidate) {
        (self.receipt, self.candidate)
    }
}

/// Stateless target operations over one exact RPC client and guarded signer.
#[derive(Clone)]
pub struct EvmWalletJsonRpcTarget<Client = EvmJsonRpcTransport>
where
    Client: EvmWalletRpcClient,
{
    client: Client,
    signer: EvmWalletSignerBinding,
    safe_failure_contract_ref: ContentRef,
}

impl<Client> EvmWalletJsonRpcTarget<Client>
where
    Client: EvmWalletRpcClient,
{
    /// Constructs a target without performing signer or provider IO.
    pub fn new(
        client: Client,
        signer: EvmWalletSignerBinding,
        safe_failure_contract_ref: ContentRef,
    ) -> Self {
        Self {
            client,
            signer,
            safe_failure_contract_ref,
        }
    }

    /// Returns the generation required by both guarded signing and target entry.
    pub fn durable_generation_ref(&self) -> &ContentRef {
        self.signer.durable_generation_ref()
    }

    /// Guardedly prepares one exact candidate before durable authorization.
    pub async fn prepare_broadcast(
        &self,
        request: &EvmSubmitTransactionRequest,
        allocated_nonce: u64,
        fee_ordinal: u16,
    ) -> Result<PreparedEvmWalletBroadcast, EvmWalletLiveError> {
        validate_request_binding(request, &self.signer)?;
        let envelope = request
            .unsigned_envelope(allocated_nonce, fee_ordinal)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let provider = self
            .signer
            .binder
            .bind()
            .await
            .map_err(|_| EvmWalletLiveError::SignerUnavailable)?;
        let expected_sender = request
            .policy()
            .sender_address()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let generation_ref = provider.binding().durable_generation_ref().clone();
        let signed = sign_eip1559_guarded(
            &envelope,
            provider.binding().signer_ref().clone(),
            expected_sender,
            &generation_ref,
            provider.as_ref(),
        )
        .await
        .map_err(|_| EvmWalletLiveError::SignerUnavailable)?;
        let candidate = EvmWalletTransactionCandidate::new(
            request.clone(),
            allocated_nonce,
            fee_ordinal,
            signed.transaction_hash(),
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let descriptor = EvmWalletTargetEntryDescriptor::broadcast(request, &candidate)?;
        Ok(PreparedEvmWalletBroadcast {
            descriptor,
            candidate,
            signed,
        })
    }

    /// Performs one authorized `eth_sendRawTransaction` exchange.
    pub async fn broadcast(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        prepared: PreparedEvmWalletBroadcast,
    ) -> Result<EvmWalletBroadcastReturn, EvmWalletLiveError> {
        let PreparedEvmWalletBroadcast {
            descriptor,
            candidate,
            signed,
        } = prepared;
        require_target_entry(
            &authority,
            &descriptor,
            self.signer.durable_generation_ref(),
        )?;
        require_candidate_request(request, &candidate)?;
        if descriptor.operation_id() != EVM_WALLET_BROADCAST_OPERATION_ID
            || format!("{:#x}", signed.transaction_hash()) != candidate.transaction_hash()
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        let route = request
            .policy()
            .route_generation_ref()
            .to_content_ref()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let raw_transaction = Zeroizing::new(format!("0x{}", hex::encode(signed.bytes())));
        let response = self
            .client
            .exchange(
                &route,
                request.policy().chain_id(),
                "eth_sendRawTransaction",
                json!([raw_transaction.as_str()]),
            )
            .await;
        let outcome = match response {
            Ok(EvmWalletRpcResponse::Result(value)) => {
                let acknowledged = value.as_str().and_then(parse_hash);
                if acknowledged != Some(signed.transaction_hash()) {
                    self.indeterminate_unclassified()?
                } else {
                    self.returned(
                        request,
                        EvmWalletAttemptResult::Broadcast {
                            candidate_ref: candidate
                                .reference()
                                .map_err(|_| EvmWalletLiveError::InvalidContract)?,
                            transaction_hash: candidate.transaction_hash().to_owned(),
                            status: EvmWalletBroadcastStatus::Accepted,
                            classifier_ref: None,
                        },
                    )?
                }
            }
            Ok(EvmWalletRpcResponse::Error(error))
                if error.is_exact_already_known()
                    && request.policy().already_known_classifier_ref()
                        == &evm_already_known_classifier_ref()? =>
            {
                self.returned(
                    request,
                    EvmWalletAttemptResult::Broadcast {
                        candidate_ref: candidate
                            .reference()
                            .map_err(|_| EvmWalletLiveError::InvalidContract)?,
                        transaction_hash: candidate.transaction_hash().to_owned(),
                        status: EvmWalletBroadcastStatus::AlreadyKnown,
                        classifier_ref: Some(
                            request.policy().already_known_classifier_ref().clone(),
                        ),
                    },
                )?
            }
            Ok(EvmWalletRpcResponse::Error(_)) => self.indeterminate_conflict()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(EvmWalletBroadcastReturn {
            receipt: authority.complete(outcome),
            candidate,
        })
    }

    /// Performs one `eth_getTransactionByHash` exchange.
    pub async fn transaction_lookup(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::transaction_lookup(request, candidate)?;
        require_target_entry(
            &authority,
            &descriptor,
            self.signer.durable_generation_ref(),
        )?;
        let response = self
            .exchange_hash(request, "eth_getTransactionByHash", candidate)
            .await;
        let outcome = match response {
            Ok(EvmWalletRpcResponse::Result(Value::Null)) => self.returned(
                request,
                EvmWalletAttemptResult::TransactionLookup {
                    transaction_hash: candidate.transaction_hash().to_owned(),
                    transaction: None,
                },
            )?,
            Ok(EvmWalletRpcResponse::Result(value)) => match parse_transaction(&value) {
                Ok(transaction) if transaction.matches_candidate(candidate).unwrap_or(false) => {
                    self.returned(
                        request,
                        EvmWalletAttemptResult::TransactionLookup {
                            transaction_hash: candidate.transaction_hash().to_owned(),
                            transaction: Some(transaction),
                        },
                    )?
                }
                _ => self.indeterminate_unclassified()?,
            },
            Ok(EvmWalletRpcResponse::Error(_)) => self.indeterminate_conflict()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
    }

    /// Performs one `eth_getTransactionReceipt` exchange.
    pub async fn receipt_lookup(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::receipt_lookup(request, candidate)?;
        require_target_entry(
            &authority,
            &descriptor,
            self.signer.durable_generation_ref(),
        )?;
        let response = self
            .exchange_hash(request, "eth_getTransactionReceipt", candidate)
            .await;
        let outcome = match response {
            Ok(EvmWalletRpcResponse::Result(Value::Null)) => self.returned(
                request,
                EvmWalletAttemptResult::ReceiptLookup {
                    transaction_hash: candidate.transaction_hash().to_owned(),
                    receipt: None,
                },
            )?,
            Ok(EvmWalletRpcResponse::Result(value)) => match parse_receipt(&value) {
                Ok(receipt) if receipt.transaction_hash() == candidate.transaction_hash() => self
                    .returned(
                    request,
                    EvmWalletAttemptResult::ReceiptLookup {
                        transaction_hash: candidate.transaction_hash().to_owned(),
                        receipt: Some(receipt),
                    },
                )?,
                _ => self.indeterminate_unclassified()?,
            },
            Ok(EvmWalletRpcResponse::Error(_)) => self.indeterminate_conflict()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
    }

    /// Performs one `eth_getBlockByNumber("finalized", false)` exchange.
    pub async fn finalized_head(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::finalized_head(request, candidate)?;
        require_target_entry(
            &authority,
            &descriptor,
            self.signer.durable_generation_ref(),
        )?;
        let response = self
            .exchange(request, "eth_getBlockByNumber", json!(["finalized", false]))
            .await;
        let outcome = match response {
            Ok(EvmWalletRpcResponse::Result(value)) => match parse_block(&value) {
                Ok(block) => {
                    self.returned(request, EvmWalletAttemptResult::FinalizedHead { block })?
                }
                Err(()) => self.indeterminate_unclassified()?,
            },
            Ok(EvmWalletRpcResponse::Error(_)) => self.indeterminate_conflict()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
    }

    /// Performs one inclusion-number block lookup and admits terminal evidence only on equality.
    pub async fn canonical_inclusion(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
        inclusion_number: U256,
        terminal_candidate: Option<EvmWalletTerminalEvidence>,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::canonical_inclusion(
            request,
            candidate,
            inclusion_number,
        )?;
        require_target_entry(
            &authority,
            &descriptor,
            self.signer.durable_generation_ref(),
        )?;
        let response = self
            .exchange(
                request,
                "eth_getBlockByNumber",
                json!([encode_quantity(inclusion_number), false]),
            )
            .await;
        let outcome = match response {
            Ok(EvmWalletRpcResponse::Result(Value::Null)) => self.returned(
                request,
                EvmWalletAttemptResult::CanonicalInclusion {
                    block: None,
                    terminal: None,
                },
            )?,
            Ok(EvmWalletRpcResponse::Result(value)) => match parse_block(&value) {
                Ok(block) => {
                    let terminal = terminal_candidate
                        .filter(|terminal| terminal.inclusion_block() == &block)
                        .filter(|terminal| terminal.outcome().is_ok());
                    self.returned(
                        request,
                        EvmWalletAttemptResult::CanonicalInclusion {
                            block: Some(block),
                            terminal,
                        },
                    )?
                }
                Err(()) => self.indeterminate_unclassified()?,
            },
            Ok(EvmWalletRpcResponse::Error(_)) => self.indeterminate_conflict()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
    }

    async fn exchange_hash(
        &self,
        request: &EvmSubmitTransactionRequest,
        method: &'static str,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<EvmWalletRpcResponse, EvmWalletRpcFailure> {
        self.exchange(request, method, json!([candidate.transaction_hash()]))
            .await
    }

    async fn exchange(
        &self,
        request: &EvmSubmitTransactionRequest,
        method: &'static str,
        params: Value,
    ) -> Result<EvmWalletRpcResponse, EvmWalletRpcFailure> {
        let route = request
            .policy()
            .route_generation_ref()
            .to_content_ref()
            .map_err(|_| EvmWalletRpcFailure::GenerationFenced)?;
        self.client
            .exchange(&route, request.policy().chain_id(), method, params)
            .await
    }

    fn returned(
        &self,
        request: &EvmSubmitTransactionRequest,
        result: EvmWalletAttemptResult,
    ) -> Result<DeliveryAttemptOutcome, EvmWalletLiveError> {
        result
            .validate()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let canonical = encode_boundary(&result).map_err(|_| EvmWalletLiveError::ResultEncoding)?;
        if canonical.as_bytes().len()
            > usize::try_from(request.policy().convergence().max_attempt_result_bytes())
                .map_err(|_| EvmWalletLiveError::InvalidContract)?
        {
            return Err(EvmWalletLiveError::ResultEncoding);
        }
        let schema =
            EvmWalletAttemptResult::schema_id().map_err(|_| EvmWalletLiveError::ResultEncoding)?;
        let result = SchemaQualifiedCanonicalValue::new(schema, canonical.as_bytes())
            .map_err(|_| EvmWalletLiveError::ResultEncoding)?;
        DeliveryAttemptOutcome::returned(result).map_err(|_| EvmWalletLiveError::ResultEncoding)
    }

    fn failure_outcome(
        &self,
        failure: EvmWalletRpcFailure,
    ) -> Result<DeliveryAttemptOutcome, EvmWalletLiveError> {
        let (code, class, stage, did_not_enter) = match failure {
            EvmWalletRpcFailure::GenerationFenced => (
                ReferenceFailureCode::GenerationFenced,
                FailureClass::Authorization,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            ),
            EvmWalletRpcFailure::AccessCancelled => (
                ReferenceFailureCode::AccessCancelled,
                FailureClass::Cancellation,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            ),
            EvmWalletRpcFailure::UnavailableBeforeEntry => (
                ReferenceFailureCode::DestinationUnavailable,
                FailureClass::Transport,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            ),
            EvmWalletRpcFailure::ResponseLost => (
                ReferenceFailureCode::DestinationUnavailable,
                FailureClass::Transport,
                BoundaryStage::BoundaryEntry,
                false,
            ),
            EvmWalletRpcFailure::DestinationRejected => (
                ReferenceFailureCode::RequestConflict,
                FailureClass::Destination,
                BoundaryStage::BoundaryObservation,
                false,
            ),
            EvmWalletRpcFailure::InvalidResponse => (
                ReferenceFailureCode::UnclassifiedFailure,
                FailureClass::Unclassified,
                BoundaryStage::BoundaryObservation,
                false,
            ),
        };
        let failure =
            reference_safe_failure(self.safe_failure_contract_ref.clone(), code, class, stage)
                .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if did_not_enter {
            DeliveryAttemptOutcome::did_not_enter(failure)
        } else {
            DeliveryAttemptOutcome::indeterminate(failure)
        }
        .map_err(|_| EvmWalletLiveError::InvalidContract)
    }

    fn indeterminate_conflict(&self) -> Result<DeliveryAttemptOutcome, EvmWalletLiveError> {
        self.failure_outcome(EvmWalletRpcFailure::DestinationRejected)
    }

    fn indeterminate_unclassified(&self) -> Result<DeliveryAttemptOutcome, EvmWalletLiveError> {
        self.failure_outcome(EvmWalletRpcFailure::InvalidResponse)
    }
}

/// Returns the exact canonical classifier for one provider error shape.
pub fn evm_already_known_classifier_canonical(
) -> Result<PlainCanonicalJsonBytes, EvmWalletLiveError> {
    canonical_json(&json!({
        "exact_error": {
            "code": EXACT_ALREADY_KNOWN_CODE,
            "message": EXACT_ALREADY_KNOWN_MESSAGE,
            "other_fields": false,
        },
        "provider_text_persisted": false,
        "version": EVM_ALREADY_KNOWN_CLASSIFIER_VERSION,
    }))
}

/// Returns the exact classifier identity stored in immutable wallet policy.
pub fn evm_already_known_classifier_ref() -> Result<EvmWalletReference, EvmWalletLiveError> {
    let reference = boundary_content_ref(
        descriptor_schema_id("mfm.evm-live.already-known-classifier")?,
        &evm_already_known_classifier_canonical()?,
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)?;
    Ok(EvmWalletReference::from_content_ref(reference))
}

/// Returns the sealed five-method target callback surface.
pub fn evm_wallet_target_callback_surface_canonical(
) -> Result<PlainCanonicalJsonBytes, EvmWalletLiveError> {
    canonical_json(&json!({
        "callbacks": [
            EVM_WALLET_BROADCAST_OPERATION_ID,
            EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
            EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
            EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
            EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
        ],
        "failover": false,
        "redirects": false,
        "reselection": false,
        "retries": false,
        "rpc_exchanges_per_callback": 1,
        "version": EVM_WALLET_TARGET_CALLBACK_SURFACE_VERSION,
    }))
}

/// Returns the sealed wallet target callback-surface identity.
pub fn evm_wallet_target_callback_surface_ref() -> Result<ContentRef, EvmWalletLiveError> {
    boundary_content_ref(
        descriptor_schema_id("mfm.evm-live.wallet-target-callback-surface")?,
        &evm_wallet_target_callback_surface_canonical()?,
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)
}

/// Builds retained metadata for the wallet target callback surface.
pub fn evm_wallet_target_callback_surface_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract, EvmWalletLiveError> {
    RetainedValueContract::new(
        descriptor_schema_id("mfm.evm-live.wallet-target-callback-surface")?,
        SemanticTypeId::new(
            "mfm.evm-live",
            "wallet-target-callback-surface",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"semantic:mfm.evm-live:wallet-target-callback-surface:1"),
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)
}

fn require_target_entry(
    authority: &TargetEntryAuthority,
    descriptor: &EvmWalletTargetEntryDescriptor,
    guarded_generation_ref: &ContentRef,
) -> Result<(), EvmWalletLiveError> {
    if authority.target_operation_ref() != &descriptor.content_ref()?
        || authority.durable_ledger_generation_ref() != guarded_generation_ref
    {
        return Err(EvmWalletLiveError::InvalidContract);
    }
    Ok(())
}

fn validate_request_binding(
    request: &EvmSubmitTransactionRequest,
    signer: &EvmWalletSignerBinding,
) -> Result<(), EvmWalletLiveError> {
    if request.policy().signer_binding_ref() != signer.binding_ref()
        || signer
            .binder
            .binding()
            .expected_public_identity()
            .account_id()
            != Some(request.policy().sender())
    {
        return Err(EvmWalletLiveError::InvalidContract);
    }
    Ok(())
}

fn require_candidate_request(
    request: &EvmSubmitTransactionRequest,
    candidate: &EvmWalletTransactionCandidate,
) -> Result<(), EvmWalletLiveError> {
    if candidate.request() != request || candidate.validate().is_err() {
        return Err(EvmWalletLiveError::InvalidContract);
    }
    Ok(())
}

fn parse_transaction(value: &Value) -> Result<EvmWalletObservedTransaction, ()> {
    let object = value.as_object().ok_or(())?;
    if quantity_field(object, "type")? != U256::from(EVM_WALLET_TRANSACTION_TYPE) {
        return Err(());
    }
    let to = match object.get("to") {
        Some(Value::Null) => TxKind::Create,
        Some(Value::String(value)) => TxKind::Call(parse_address(value)?),
        _ => return Err(()),
    };
    let placement = parse_optional_placement(object)?;
    EvmWalletObservedTransaction::new(
        hash_field(object, "hash")?,
        quantity_field(object, "chainId")?,
        quantity_field(object, "nonce")?,
        address_field(object, "from")?,
        to,
        quantity_field(object, "value")?,
        bytes_field(object, "input", mfm_evm::EVM_WALLET_DATA_MAX_BYTES)?,
        quantity_field(object, "gas")?,
        quantity_field(object, "maxFeePerGas")?,
        quantity_field(object, "maxPriorityFeePerGas")?,
        parse_access_list(object.get("accessList").ok_or(())?)?,
        placement,
    )
    .map_err(|_| ())
}

fn parse_receipt(value: &Value) -> Result<EvmWalletReceipt, ()> {
    let object = value.as_object().ok_or(())?;
    if let Some(value) = object.get("type") {
        let kind = value.as_str().and_then(parse_quantity).ok_or(())?;
        if kind != U256::from(EVM_WALLET_TRANSACTION_TYPE) {
            return Err(());
        }
    }
    let block = EvmBlockAnchor::new(
        quantity_field(object, "blockNumber")?,
        hash_field(object, "blockHash")?,
    );
    let status = match quantity_field(object, "status")? {
        value if value == U256::from(1) => EvmWalletReceiptStatus::Success,
        value if value == U256::ZERO => EvmWalletReceiptStatus::Reverted,
        _ => return Err(()),
    };
    let transaction_hash = hash_field(object, "transactionHash")?;
    let transaction_index = quantity_field(object, "transactionIndex")?;
    let logs = object
        .get("logs")
        .and_then(Value::as_array)
        .ok_or(())?
        .iter()
        .map(|value| parse_receipt_log(value, &block, transaction_hash, transaction_index))
        .collect::<Result<Vec<_>, _>>()?;
    EvmWalletReceipt::new(
        transaction_hash,
        transaction_index,
        block,
        address_field(object, "from")?,
        optional_address_field(object, "to")?,
        optional_address_field(object, "contractAddress")?,
        status,
        quantity_field(object, "gasUsed")?,
        quantity_field(object, "cumulativeGasUsed")?,
        logs,
    )
    .map_err(|_| ())
}

fn parse_receipt_log(
    value: &Value,
    expected_block: &EvmBlockAnchor,
    expected_transaction_hash: B256,
    expected_transaction_index: U256,
) -> Result<EvmWalletReceiptLog, ()> {
    let object = value.as_object().ok_or(())?;
    let block = EvmBlockAnchor::new(
        quantity_field(object, "blockNumber")?,
        hash_field(object, "blockHash")?,
    );
    let transaction_hash = hash_field(object, "transactionHash")?;
    let transaction_index = quantity_field(object, "transactionIndex")?;
    if &block != expected_block
        || transaction_hash != expected_transaction_hash
        || transaction_index != expected_transaction_index
    {
        return Err(());
    }
    let topics = object
        .get("topics")
        .and_then(Value::as_array)
        .ok_or(())?
        .iter()
        .map(|value| value.as_str().and_then(parse_hash).ok_or(()))
        .collect::<Result<Vec<_>, _>>()?;
    EvmWalletReceiptLog::new(
        address_field(object, "address")?,
        topics,
        bytes_field(
            object,
            "data",
            mfm_evm::EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES,
        )?,
        block,
        transaction_hash,
        transaction_index,
        quantity_field(object, "logIndex")?,
        object.get("removed").and_then(Value::as_bool).ok_or(())?,
    )
    .map_err(|_| ())
}

fn parse_optional_placement(
    object: &Map<String, Value>,
) -> Result<Option<EvmWalletTransactionPlacement>, ()> {
    match (
        object.get("blockNumber"),
        object.get("blockHash"),
        object.get("transactionIndex"),
    ) {
        (Some(Value::Null), Some(Value::Null), Some(Value::Null)) => Ok(None),
        (Some(Value::String(number)), Some(Value::String(hash)), Some(Value::String(index))) => {
            EvmWalletTransactionPlacement::new(
                EvmBlockAnchor::new(
                    parse_quantity(number).ok_or(())?,
                    parse_hash(hash).ok_or(())?,
                ),
                parse_quantity(index).ok_or(())?,
            )
            .map(Some)
            .map_err(|_| ())
        }
        _ => Err(()),
    }
}

fn parse_access_list(value: &Value) -> Result<Vec<EvmWalletAccessListEntry>, ()> {
    value
        .as_array()
        .ok_or(())?
        .iter()
        .map(|value| {
            let object = value.as_object().ok_or(())?;
            let keys = object
                .get("storageKeys")
                .and_then(Value::as_array)
                .ok_or(())?
                .iter()
                .map(|value| value.as_str().and_then(parse_hash).ok_or(()))
                .collect::<Result<Vec<_>, _>>()?;
            EvmWalletAccessListEntry::new(address_field(object, "address")?, keys).map_err(|_| ())
        })
        .collect()
}

fn parse_block(value: &Value) -> Result<EvmBlockAnchor, ()> {
    let object = value.as_object().ok_or(())?;
    let block = EvmBlockAnchor::new(
        quantity_field(object, "number")?,
        hash_field(object, "hash")?,
    );
    block.validate().map_err(|_| ())?;
    Ok(block)
}

fn encode_quantity(value: U256) -> String {
    format!("0x{value:x}")
}

fn parse_quantity(value: &str) -> Option<U256> {
    let digits = value.strip_prefix("0x")?;
    if digits.is_empty()
        || digits.len() > 64
        || (digits.len() > 1 && digits.starts_with('0'))
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let parsed = U256::from_str_radix(digits, 16).ok()?;
    (encode_quantity(parsed) == value).then_some(parsed)
}

fn parse_hash(value: &str) -> Option<B256> {
    let parsed = B256::from_str(value).ok()?;
    (format!("{parsed:#x}") == value).then_some(parsed)
}

fn parse_address(value: &str) -> Result<Address, ()> {
    let parsed = Address::from_str(value).map_err(|_| ())?;
    (format!("{parsed:#x}") == value)
        .then_some(parsed)
        .ok_or(())
}

fn quantity_field(object: &Map<String, Value>, field: &str) -> Result<U256, ()> {
    object
        .get(field)
        .and_then(Value::as_str)
        .and_then(parse_quantity)
        .ok_or(())
}

fn hash_field(object: &Map<String, Value>, field: &str) -> Result<B256, ()> {
    object
        .get(field)
        .and_then(Value::as_str)
        .and_then(parse_hash)
        .ok_or(())
}

fn address_field(object: &Map<String, Value>, field: &str) -> Result<Address, ()> {
    parse_address(object.get(field).and_then(Value::as_str).ok_or(())?)
}

fn optional_address_field(object: &Map<String, Value>, field: &str) -> Result<Option<Address>, ()> {
    match object.get(field) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => parse_address(value).map(Some),
        _ => Err(()),
    }
}

fn bytes_field(object: &Map<String, Value>, field: &str, maximum: usize) -> Result<Vec<u8>, ()> {
    let raw = object.get(field).and_then(Value::as_str).ok_or(())?;
    let digits = raw.strip_prefix("0x").ok_or(())?;
    if !digits.len().is_multiple_of(2)
        || digits.len() / 2 > maximum
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(());
    }
    hex::decode(digits).map_err(|_| ())
}

fn descriptor_schema_id(name: &'static str) -> Result<SchemaId, EvmWalletLiveError> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:1").as_bytes()),
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes, EvmWalletLiveError> {
    let encoded = serde_json::to_string(value).map_err(|_| EvmWalletLiveError::InvalidContract)?;
    PlainCanonicalJsonBytes::from_json_str(&encoded)
        .map_err(|_| EvmWalletLiveError::InvalidContract)
}

#[cfg(test)]
#[path = "wallet_rpc_tests.rs"]
mod tests;
