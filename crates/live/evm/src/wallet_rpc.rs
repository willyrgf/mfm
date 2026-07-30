//! Stateless, one-exchange EVM wallet target operations.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::{B256, U256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{
    sign_eip1559_guarded, EvmSubmitTransactionRequest, EvmWalletAttemptResult,
    EvmWalletBroadcastStatus, EvmWalletReference, EvmWalletTerminalEvidence,
    EvmWalletTransactionCandidate, TransientSignedEip1559Envelope,
    EVM_WALLET_BROADCAST_OPERATION_ID, EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
    EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID, EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
    EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
};
use mfm_executor::{
    reference_safe_failure, BoundaryStage, CanonicalExecutorRequest, DeliveryAttemptOutcome,
    FailureClass, ReferenceFailureCode, SchemaQualifiedCanonicalValue, TargetEntryAuthority,
};
use mfm_ids::{ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_program::{boundary_content_ref, encode_boundary};
use mfm_signing::GenerationGuardedDeterministicSigningProviderBinder;
use mfm_values::{MfmValue, RetainedValueContract};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    transport::{
        WalletBroadcastResponse, WalletRpcFailure, EVM_BLOCK_BY_NUMBER_METHOD,
        EVM_RECEIPT_BY_HASH_METHOD, EVM_SEND_RAW_TRANSACTION_METHOD,
        EVM_TRANSACTION_BY_HASH_METHOD, EXACT_ALREADY_KNOWN_CODE, EXACT_ALREADY_KNOWN_MESSAGE,
    },
    EvmWalletRequestQualification,
};

/// Exact already-known classifier descriptor version.
pub const EVM_ALREADY_KNOWN_CLASSIFIER_VERSION: &str = "mfm.evm-live.already-known-classifier.v1";
/// Exact five-method wallet target callback-surface version.
pub const EVM_WALLET_TARGET_CALLBACK_SURFACE_VERSION: &str =
    "mfm.evm-live.wallet-target-callback-surface.v2";
/// Exact durable target-entry descriptor version.
pub const EVM_WALLET_TARGET_ENTRY_DESCRIPTOR_VERSION: &str = "mfm.evm-live.wallet-target-entry.v2";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvmWalletTargetEntryDescriptor {
    wire: EvmWalletTargetEntryWire,
    canonical: SchemaQualifiedCanonicalValue,
}

impl EvmWalletTargetEntryDescriptor {
    /// Builds the exact raw-transaction entry descriptor.
    pub(crate) fn broadcast(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_BROADCAST_OPERATION_ID,
            EVM_SEND_RAW_TRANSACTION_METHOD,
            EvmWalletRpcInputCommitment::RawTransaction {
                transaction_hash: candidate.transaction_hash().to_owned(),
            },
        )
    }

    /// Builds the exact transaction-by-hash entry descriptor.
    pub(crate) fn transaction_lookup(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
            EVM_TRANSACTION_BY_HASH_METHOD,
            EvmWalletRpcInputCommitment::TransactionHash {
                transaction_hash: candidate.transaction_hash().to_owned(),
            },
        )
    }

    /// Builds the exact receipt-by-hash entry descriptor.
    pub(crate) fn receipt_lookup(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
            EVM_RECEIPT_BY_HASH_METHOD,
            EvmWalletRpcInputCommitment::TransactionHash {
                transaction_hash: candidate.transaction_hash().to_owned(),
            },
        )
    }

    /// Builds the exact finalized-tag entry descriptor.
    pub(crate) fn finalized_head(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
            EVM_BLOCK_BY_NUMBER_METHOD,
            EvmWalletRpcInputCommitment::FinalizedTag {
                block_tag: "finalized".to_owned(),
                full_transactions: false,
            },
        )
    }

    /// Builds the exact inclusion-number entry descriptor.
    pub(crate) fn canonical_inclusion(
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
        inclusion_number: U256,
    ) -> Result<Self, EvmWalletLiveError> {
        Self::new(
            request,
            candidate,
            EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
            EVM_BLOCK_BY_NUMBER_METHOD,
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
    pub(crate) fn strict_decode(
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
    pub(crate) const fn canonical(&self) -> &SchemaQualifiedCanonicalValue {
        &self.canonical
    }

    #[cfg(test)]
    pub(crate) fn content_ref(&self) -> Result<ContentRef, EvmWalletLiveError> {
        self.canonical
            .reference()
            .map_err(|_| EvmWalletLiveError::InvalidContract)
    }

    /// Returns the reviewed target-operation family.
    pub(crate) fn operation_id(&self) -> &str {
        &self.wire.operation_id
    }

    /// Returns the exact candidate content identity.
    pub(crate) const fn candidate_ref(&self) -> &EvmWalletReference {
        &self.wire.candidate_ref
    }

    /// Returns the exact signed transaction hash.
    pub(crate) fn transaction_hash(&self) -> &str {
        &self.wire.transaction_hash
    }

    /// Returns the permanently allocated account nonce.
    pub(crate) fn allocated_nonce(&self) -> Result<u64, EvmWalletLiveError> {
        self.wire
            .allocated_nonce
            .parse()
            .map_err(|_| EvmWalletLiveError::InvalidContract)
    }

    /// Returns the immutable fee-schedule ordinal.
    pub(crate) const fn fee_ordinal(&self) -> u16 {
        self.wire.fee_ordinal
    }

    /// Returns the exact inclusion number for an inclusion lookup.
    pub(crate) fn inclusion_number(&self) -> Result<Option<U256>, EvmWalletLiveError> {
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
    pub(crate) fn candidate(
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
                EVM_SEND_RAW_TRANSACTION_METHOD,
                EvmWalletRpcInputCommitment::RawTransaction { transaction_hash },
            )
            | (
                EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
                EVM_TRANSACTION_BY_HASH_METHOD,
                EvmWalletRpcInputCommitment::TransactionHash { transaction_hash },
            )
            | (
                EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
                EVM_RECEIPT_BY_HASH_METHOD,
                EvmWalletRpcInputCommitment::TransactionHash { transaction_hash },
            ) => transaction_hash == &self.wire.transaction_hash,
            (
                EVM_WALLET_FINALIZED_HEAD_OPERATION_ID,
                EVM_BLOCK_BY_NUMBER_METHOD,
                EvmWalletRpcInputCommitment::FinalizedTag {
                    block_tag,
                    full_transactions,
                },
            ) => block_tag == "finalized" && !full_transactions,
            (
                EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
                EVM_BLOCK_BY_NUMBER_METHOD,
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

pub(crate) struct PreparedEvmWalletCommon {
    descriptor: EvmWalletTargetEntryDescriptor,
    request: EvmSubmitTransactionRequest,
    candidate: EvmWalletTransactionCandidate,
    candidate_ref: EvmWalletReference,
    route: ContentRef,
    chain_id: u64,
    transaction_hash: B256,
    result_schema: SchemaId,
    max_result_bytes: usize,
    outcomes: PreparedWalletOutcomes,
}

/// One fully validated wallet target invocation prepared before durable authorization.
pub(crate) enum PreparedEvmWalletTarget {
    /// One signed raw-transaction broadcast.
    Broadcast {
        common: Box<PreparedEvmWalletCommon>,
        signed: TransientSignedEip1559Envelope,
    },
    /// One transaction-by-hash lookup.
    TransactionLookup {
        common: Box<PreparedEvmWalletCommon>,
    },
    /// One receipt-by-hash lookup.
    ReceiptLookup {
        common: Box<PreparedEvmWalletCommon>,
    },
    /// One finalized-head lookup.
    FinalizedHead {
        common: Box<PreparedEvmWalletCommon>,
    },
    /// One exact-number canonical-inclusion lookup.
    CanonicalInclusion {
        common: Box<PreparedEvmWalletCommon>,
        inclusion_number: U256,
        terminal_candidate: Option<Box<EvmWalletTerminalEvidence>>,
    },
}

impl PreparedEvmWalletTarget {
    const fn common(&self) -> &PreparedEvmWalletCommon {
        match self {
            Self::Broadcast { common, .. }
            | Self::TransactionLookup { common }
            | Self::ReceiptLookup { common }
            | Self::FinalizedHead { common }
            | Self::CanonicalInclusion { common, .. } => common,
        }
    }

    /// Returns the public candidate committed by authorization.
    pub(crate) const fn candidate(&self) -> &EvmWalletTransactionCandidate {
        match self {
            Self::Broadcast { common, .. }
            | Self::TransactionLookup { common }
            | Self::ReceiptLookup { common }
            | Self::FinalizedHead { common }
            | Self::CanonicalInclusion { common, .. } => &common.candidate,
        }
    }

    /// Returns the exact descriptor committed by authorization.
    pub(crate) const fn descriptor(&self) -> &EvmWalletTargetEntryDescriptor {
        match self {
            Self::Broadcast { common, .. }
            | Self::TransactionLookup { common }
            | Self::ReceiptLookup { common }
            | Self::FinalizedHead { common }
            | Self::CanonicalInclusion { common, .. } => &common.descriptor,
        }
    }

    #[cfg(test)]
    pub(crate) fn classify_with_result_bound_for_test(
        &self,
        result: EvmWalletAttemptResult,
        max_result_bytes: usize,
    ) -> DeliveryAttemptOutcome {
        self.common()
            .returned_with_encoder(result, max_result_bytes, |result| {
                encode_boundary(result).map_err(|_| EvmWalletLiveError::ResultEncoding)
            })
    }

    #[cfg(test)]
    pub(crate) fn classify_encoding_failure_for_test(
        &self,
        result: EvmWalletAttemptResult,
    ) -> DeliveryAttemptOutcome {
        self.common()
            .returned_with_encoder(result, self.common().max_result_bytes, |_| {
                Err(EvmWalletLiveError::ResultEncoding)
            })
    }
}

impl fmt::Debug for PreparedEvmWalletTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedEvmWalletTarget")
            .field("descriptor", self.descriptor())
            .field("candidate", self.candidate())
            .finish()
    }
}

/// Fresh target-entry authority paired inseparably with its preauthorized invocation.
pub(crate) struct AuthorizedEvmWalletTarget {
    authority: TargetEntryAuthority,
    prepared: PreparedEvmWalletTarget,
}

impl AuthorizedEvmWalletTarget {
    pub(crate) const fn new(
        authority: TargetEntryAuthority,
        prepared: PreparedEvmWalletTarget,
    ) -> Self {
        Self {
            authority,
            prepared,
        }
    }
}

#[derive(Clone)]
pub(crate) struct PreparedWalletOutcomes {
    generation_fenced: DeliveryAttemptOutcome,
    access_cancelled: DeliveryAttemptOutcome,
    unavailable_before_entry: DeliveryAttemptOutcome,
    response_lost: DeliveryAttemptOutcome,
    request_conflict: DeliveryAttemptOutcome,
    unclassified: DeliveryAttemptOutcome,
    result_unrepresentable: DeliveryAttemptOutcome,
    adapter_contract_violation: DeliveryAttemptOutcome,
    result_encoding_failure: DeliveryAttemptOutcome,
}

#[derive(Clone)]
pub(crate) struct EvmWalletJsonRpcTarget {
    signer: GenerationGuardedDeterministicSigningProviderBinder,
    qualification: Arc<EvmWalletRequestQualification>,
}

impl EvmWalletJsonRpcTarget {
    pub(crate) fn new(
        signer: GenerationGuardedDeterministicSigningProviderBinder,
        qualification: Arc<EvmWalletRequestQualification>,
    ) -> Result<Self, EvmWalletLiveError> {
        let signer_descriptor = signer
            .binding()
            .public_descriptor()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if signer_descriptor != *qualification.signer_descriptor() {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        Ok(Self {
            signer,
            qualification,
        })
    }

    /// Guardedly prepares one exact signed candidate before durable authorization.
    pub(crate) async fn prepare_broadcast(
        &self,
        request: &EvmSubmitTransactionRequest,
        allocated_nonce: u64,
        fee_ordinal: u16,
    ) -> Result<PreparedEvmWalletTarget, EvmWalletLiveError> {
        self.qualification.verify_request(request)?;
        let envelope = request
            .unsigned_envelope(allocated_nonce, fee_ordinal)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let provider = self
            .signer
            .bind()
            .await
            .map_err(|_| EvmWalletLiveError::SignerUnavailable)?;
        let expected_sender = self.qualification.sender();
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
        let common = self.prepare_common(request, candidate, descriptor)?;
        Ok(PreparedEvmWalletTarget::Broadcast {
            common: Box::new(common),
            signed,
        })
    }

    /// Prepares one exact transaction lookup before durable authorization.
    pub(crate) fn prepare_transaction_lookup(
        &self,
        request: &EvmSubmitTransactionRequest,
        candidate: EvmWalletTransactionCandidate,
    ) -> Result<PreparedEvmWalletTarget, EvmWalletLiveError> {
        let descriptor = EvmWalletTargetEntryDescriptor::transaction_lookup(request, &candidate)?;
        Ok(PreparedEvmWalletTarget::TransactionLookup {
            common: Box::new(self.prepare_common(request, candidate, descriptor)?),
        })
    }

    /// Prepares one exact receipt lookup before durable authorization.
    pub(crate) fn prepare_receipt_lookup(
        &self,
        request: &EvmSubmitTransactionRequest,
        candidate: EvmWalletTransactionCandidate,
    ) -> Result<PreparedEvmWalletTarget, EvmWalletLiveError> {
        let descriptor = EvmWalletTargetEntryDescriptor::receipt_lookup(request, &candidate)?;
        Ok(PreparedEvmWalletTarget::ReceiptLookup {
            common: Box::new(self.prepare_common(request, candidate, descriptor)?),
        })
    }

    /// Prepares one exact finalized-head lookup before durable authorization.
    pub(crate) fn prepare_finalized_head(
        &self,
        request: &EvmSubmitTransactionRequest,
        candidate: EvmWalletTransactionCandidate,
    ) -> Result<PreparedEvmWalletTarget, EvmWalletLiveError> {
        let descriptor = EvmWalletTargetEntryDescriptor::finalized_head(request, &candidate)?;
        Ok(PreparedEvmWalletTarget::FinalizedHead {
            common: Box::new(self.prepare_common(request, candidate, descriptor)?),
        })
    }

    /// Prepares one exact canonical-inclusion lookup before durable authorization.
    pub(crate) fn prepare_canonical_inclusion(
        &self,
        request: &EvmSubmitTransactionRequest,
        candidate: EvmWalletTransactionCandidate,
        inclusion_number: U256,
        terminal_candidate: Option<EvmWalletTerminalEvidence>,
    ) -> Result<PreparedEvmWalletTarget, EvmWalletLiveError> {
        let descriptor = EvmWalletTargetEntryDescriptor::canonical_inclusion(
            request,
            &candidate,
            inclusion_number,
        )?;
        Ok(PreparedEvmWalletTarget::CanonicalInclusion {
            common: Box::new(self.prepare_common(request, candidate, descriptor)?),
            inclusion_number,
            terminal_candidate: terminal_candidate.map(Box::new),
        })
    }

    /// Performs exactly one exchange and returns one unbound outcome candidate.
    pub(crate) async fn invoke_target(
        &self,
        authorized: AuthorizedEvmWalletTarget,
    ) -> DeliveryAttemptOutcome {
        let AuthorizedEvmWalletTarget {
            authority,
            prepared,
        } = authorized;
        let target_operation_ref = prepared.descriptor().canonical().reference();
        let authority_mismatch = match target_operation_ref.as_ref() {
            Ok(target_operation_ref) => {
                target_operation_ref != authority.target_operation_ref()
                    || authority.identity().executor_binding_ref()
                        != self.qualification.executor_binding().binding_ref()
                    || authority.identity().tenant_scope_id()
                        != self
                            .qualification
                            .executor_binding()
                            .deployment()
                            .tenant_scope_id()
                    || authority.durable_ledger_generation_ref()
                        != self
                            .qualification
                            .executor_binding()
                            .deployment()
                            .durable_ledger_generation_ref()
            }
            Err(_) => true,
        };
        if authority_mismatch {
            return prepared
                .common()
                .outcomes
                .adapter_contract_violation
                .clone();
        }
        let outcome = match prepared {
            PreparedEvmWalletTarget::Broadcast { common, signed } => {
                let response = self
                    .qualification
                    .transport()
                    .send_raw_transaction(&common.route, common.chain_id, signed)
                    .await;
                match response {
                    Ok(WalletBroadcastResponse::Accepted(acknowledged))
                        if acknowledged == common.transaction_hash =>
                    {
                        common.returned_or_unrepresentable(EvmWalletAttemptResult::Broadcast {
                            candidate_ref: common.candidate_ref.clone(),
                            transaction_hash: common.candidate.transaction_hash().to_owned(),
                            status: EvmWalletBroadcastStatus::Accepted,
                            classifier_ref: None,
                        })
                    }
                    Ok(WalletBroadcastResponse::Accepted(_)) => {
                        common.outcomes.unclassified.clone()
                    }
                    Ok(WalletBroadcastResponse::AlreadyKnown)
                        if common.request.policy().already_known_classifier_ref()
                            == self.qualification.already_known_classifier_ref() =>
                    {
                        common.returned_or_unrepresentable(EvmWalletAttemptResult::Broadcast {
                            candidate_ref: common.candidate_ref.clone(),
                            transaction_hash: common.candidate.transaction_hash().to_owned(),
                            status: EvmWalletBroadcastStatus::AlreadyKnown,
                            classifier_ref: Some(
                                common
                                    .request
                                    .policy()
                                    .already_known_classifier_ref()
                                    .clone(),
                            ),
                        })
                    }
                    Ok(WalletBroadcastResponse::AlreadyKnown) => {
                        common.outcomes.request_conflict.clone()
                    }
                    Err(failure) => common.outcomes.for_rpc_failure(failure),
                }
            }
            PreparedEvmWalletTarget::TransactionLookup { common } => {
                let response = self
                    .qualification
                    .transport()
                    .transaction_by_hash(&common.route, common.chain_id, common.transaction_hash)
                    .await;
                match response {
                    Ok(None) => common.returned_or_unrepresentable(
                        EvmWalletAttemptResult::TransactionLookup {
                            transaction_hash: common.candidate.transaction_hash().to_owned(),
                            transaction: None,
                        },
                    ),
                    Ok(Some(transaction))
                        if transaction
                            .matches_candidate(&common.candidate)
                            .unwrap_or(false) =>
                    {
                        common.returned_or_unrepresentable(
                            EvmWalletAttemptResult::TransactionLookup {
                                transaction_hash: common.candidate.transaction_hash().to_owned(),
                                transaction: Some(transaction),
                            },
                        )
                    }
                    Ok(Some(_)) => common.outcomes.unclassified.clone(),
                    Err(failure) => common.outcomes.for_rpc_failure(failure),
                }
            }
            PreparedEvmWalletTarget::ReceiptLookup { common } => {
                let response = self
                    .qualification
                    .transport()
                    .receipt_by_hash(&common.route, common.chain_id, common.transaction_hash)
                    .await;
                match response {
                    Ok(None) => {
                        common.returned_or_unrepresentable(EvmWalletAttemptResult::ReceiptLookup {
                            transaction_hash: common.candidate.transaction_hash().to_owned(),
                            receipt: None,
                        })
                    }
                    Ok(Some(receipt))
                        if receipt.transaction_hash() == common.candidate.transaction_hash() =>
                    {
                        common.returned_or_unrepresentable(EvmWalletAttemptResult::ReceiptLookup {
                            transaction_hash: common.candidate.transaction_hash().to_owned(),
                            receipt: Some(receipt),
                        })
                    }
                    Ok(Some(_)) => common.outcomes.unclassified.clone(),
                    Err(failure) => common.outcomes.for_rpc_failure(failure),
                }
            }
            PreparedEvmWalletTarget::FinalizedHead { common } => {
                let response = self
                    .qualification
                    .transport()
                    .finalized_head(&common.route, common.chain_id)
                    .await;
                match response {
                    Ok(block) => {
                        common.returned_or_unrepresentable(EvmWalletAttemptResult::FinalizedHead {
                            block,
                        })
                    }
                    Err(failure) => common.outcomes.for_rpc_failure(failure),
                }
            }
            PreparedEvmWalletTarget::CanonicalInclusion {
                common,
                inclusion_number,
                terminal_candidate,
            } => {
                let response = self
                    .qualification
                    .transport()
                    .inclusion_block(&common.route, common.chain_id, inclusion_number)
                    .await;
                match response {
                    Ok(None) => common.returned_or_unrepresentable(
                        EvmWalletAttemptResult::CanonicalInclusion {
                            block: None,
                            terminal: None,
                        },
                    ),
                    Ok(Some(block)) => {
                        let terminal = terminal_candidate
                            .filter(|terminal| terminal.inclusion_block() == &block)
                            .filter(|terminal| terminal.outcome().is_ok())
                            .map(|terminal| *terminal);
                        common.returned_or_unrepresentable(
                            EvmWalletAttemptResult::CanonicalInclusion {
                                block: Some(block),
                                terminal,
                            },
                        )
                    }
                    Err(failure) => common.outcomes.for_rpc_failure(failure),
                }
            }
        };
        outcome
    }

    fn prepare_common(
        &self,
        request: &EvmSubmitTransactionRequest,
        candidate: EvmWalletTransactionCandidate,
        descriptor: EvmWalletTargetEntryDescriptor,
    ) -> Result<PreparedEvmWalletCommon, EvmWalletLiveError> {
        self.qualification.verify_request(request)?;
        require_candidate_request(request, &candidate)?;
        descriptor.validate_for_request(request)?;
        let candidate_ref = candidate
            .reference()
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if descriptor.candidate_ref() != &candidate_ref {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        let transaction_hash =
            parse_hash(candidate.transaction_hash()).ok_or(EvmWalletLiveError::InvalidContract)?;
        let route = self
            .rpc_route(request)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let result_schema =
            EvmWalletAttemptResult::schema_id().map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let max_result_bytes =
            usize::try_from(request.policy().convergence().max_attempt_result_bytes())
                .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let outcomes = PreparedWalletOutcomes::new(
            self.qualification
                .executor_binding()
                .contract()
                .safe_failure_contract_ref()
                .clone(),
        )?;
        Ok(PreparedEvmWalletCommon {
            descriptor,
            request: request.clone(),
            candidate,
            candidate_ref,
            route,
            chain_id: self.qualification.chain_id(),
            transaction_hash,
            result_schema,
            max_result_bytes,
            outcomes,
        })
    }

    fn rpc_route(
        &self,
        request: &EvmSubmitTransactionRequest,
    ) -> Result<ContentRef, WalletRpcFailure> {
        self.qualification
            .verify_request(request)
            .map_err(|_| WalletRpcFailure::GenerationFenced)?;
        self.qualification
            .route_generation_ref()
            .to_content_ref()
            .map_err(|_| WalletRpcFailure::GenerationFenced)
    }
}

impl PreparedEvmWalletCommon {
    fn returned_or_unrepresentable(
        &self,
        result: EvmWalletAttemptResult,
    ) -> DeliveryAttemptOutcome {
        self.returned_with_encoder(result, self.max_result_bytes, |result| {
            encode_boundary(result).map_err(|_| EvmWalletLiveError::ResultEncoding)
        })
    }

    fn returned_with_encoder<Encode>(
        &self,
        result: EvmWalletAttemptResult,
        max_result_bytes: usize,
        encode: Encode,
    ) -> DeliveryAttemptOutcome
    where
        Encode:
            FnOnce(&EvmWalletAttemptResult) -> Result<PlainCanonicalJsonBytes, EvmWalletLiveError>,
    {
        if result.validate_for_candidate(&self.candidate).is_err() {
            return self.outcomes.adapter_contract_violation.clone();
        }
        let canonical = match encode(&result) {
            Ok(canonical) => canonical,
            Err(_) => return self.outcomes.result_encoding_failure.clone(),
        };
        if canonical.as_bytes().len() > max_result_bytes {
            return self.outcomes.result_unrepresentable.clone();
        }
        let result = match SchemaQualifiedCanonicalValue::new(
            self.result_schema.clone(),
            canonical.as_bytes(),
        ) {
            Ok(result) => result,
            Err(_) => return self.outcomes.result_encoding_failure.clone(),
        };
        DeliveryAttemptOutcome::returned(result)
            .unwrap_or_else(|_| self.outcomes.result_encoding_failure.clone())
    }
}

impl PreparedWalletOutcomes {
    pub(crate) fn new(safe_failure_contract_ref: ContentRef) -> Result<Self, EvmWalletLiveError> {
        let outcome = |code, class, stage, did_not_enter| {
            let failure =
                reference_safe_failure(safe_failure_contract_ref.clone(), code, class, stage)
                    .map_err(|_| EvmWalletLiveError::InvalidContract)?;
            if did_not_enter {
                DeliveryAttemptOutcome::did_not_enter(failure)
            } else {
                DeliveryAttemptOutcome::indeterminate(failure)
            }
            .map_err(|_| EvmWalletLiveError::InvalidContract)
        };
        Ok(Self {
            generation_fenced: outcome(
                ReferenceFailureCode::GenerationFenced,
                FailureClass::Authorization,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            )?,
            access_cancelled: outcome(
                ReferenceFailureCode::AccessCancelled,
                FailureClass::Cancellation,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            )?,
            unavailable_before_entry: outcome(
                ReferenceFailureCode::DestinationUnavailable,
                FailureClass::Transport,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            )?,
            response_lost: outcome(
                ReferenceFailureCode::DestinationUnavailable,
                FailureClass::Transport,
                BoundaryStage::BoundaryEntry,
                false,
            )?,
            request_conflict: outcome(
                ReferenceFailureCode::RequestConflict,
                FailureClass::Destination,
                BoundaryStage::BoundaryObservation,
                false,
            )?,
            unclassified: outcome(
                ReferenceFailureCode::UnclassifiedFailure,
                FailureClass::Unclassified,
                BoundaryStage::BoundaryObservation,
                false,
            )?,
            result_unrepresentable: outcome(
                ReferenceFailureCode::ResultUnrepresentable,
                FailureClass::UnrepresentableResponse,
                BoundaryStage::BoundaryObservation,
                false,
            )?,
            adapter_contract_violation: DeliveryAttemptOutcome::non_domain_failure(
                mfm_journal::v2::NonDomainFailure::new(
                    mfm_journal::v2::NonDomainEntryStatus::MayHaveEntered,
                    mfm_journal::v2::NonDomainDisposition::IntegrityBlocked,
                    mfm_journal::v2::NonDomainFailureCode::AdapterContractViolation,
                )
                .map_err(|_| EvmWalletLiveError::InvalidContract)?,
            )
            .map_err(|_| EvmWalletLiveError::InvalidContract)?,
            result_encoding_failure: DeliveryAttemptOutcome::non_domain_failure(
                mfm_journal::v2::NonDomainFailure::new(
                    mfm_journal::v2::NonDomainEntryStatus::MayHaveEntered,
                    mfm_journal::v2::NonDomainDisposition::IntegrityBlocked,
                    mfm_journal::v2::NonDomainFailureCode::ResultEncodingFailure,
                )
                .map_err(|_| EvmWalletLiveError::InvalidContract)?,
            )
            .map_err(|_| EvmWalletLiveError::InvalidContract)?,
        })
    }

    pub(crate) fn completion_outcomes(&self) -> [&DeliveryAttemptOutcome; 9] {
        [
            &self.generation_fenced,
            &self.access_cancelled,
            &self.unavailable_before_entry,
            &self.response_lost,
            &self.request_conflict,
            &self.unclassified,
            &self.result_unrepresentable,
            &self.adapter_contract_violation,
            &self.result_encoding_failure,
        ]
    }

    fn for_rpc_failure(&self, failure: WalletRpcFailure) -> DeliveryAttemptOutcome {
        match failure {
            WalletRpcFailure::GenerationFenced => self.generation_fenced.clone(),
            WalletRpcFailure::AccessCancelled => self.access_cancelled.clone(),
            WalletRpcFailure::UnavailableBeforeEntry => self.unavailable_before_entry.clone(),
            WalletRpcFailure::ResponseLost => self.response_lost.clone(),
            WalletRpcFailure::DestinationRejected => self.request_conflict.clone(),
            WalletRpcFailure::InvalidResponse => self.unclassified.clone(),
        }
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
            "2",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"semantic:mfm.evm-live:wallet-target-callback-surface:2"),
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|_| EvmWalletLiveError::InvalidContract)
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

fn descriptor_schema_id(name: &'static str) -> Result<SchemaId, EvmWalletLiveError> {
    let version = match name {
        "mfm.evm-live.wallet-target-entry" | "mfm.evm-live.wallet-target-callback-surface" => "2",
        _ => "1",
    };
    SchemaId::new(
        name,
        version,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:{version}").as_bytes()),
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
