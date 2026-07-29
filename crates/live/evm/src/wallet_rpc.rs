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
    TargetOperationReceipt,
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
    "mfm.evm-live.wallet-target-callback-surface.v1";
/// Exact durable target-entry descriptor version.
pub const EVM_WALLET_TARGET_ENTRY_DESCRIPTOR_VERSION: &str = "mfm.evm-live.wallet-target-entry.v1";

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

    /// Returns its exact content identity.
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

pub(crate) struct PreparedEvmWalletBroadcast {
    descriptor: EvmWalletTargetEntryDescriptor,
    candidate: EvmWalletTransactionCandidate,
    signed: TransientSignedEip1559Envelope,
}

impl PreparedEvmWalletBroadcast {
    /// Returns the public candidate committed by authorization.
    pub(crate) const fn candidate(&self) -> &EvmWalletTransactionCandidate {
        &self.candidate
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

pub(crate) struct EvmWalletBroadcastReturn {
    receipt: TargetOperationReceipt,
    candidate: EvmWalletTransactionCandidate,
}

impl EvmWalletBroadcastReturn {
    /// Splits the receipt and public candidate.
    pub(crate) fn into_parts(self) -> (TargetOperationReceipt, EvmWalletTransactionCandidate) {
        (self.receipt, self.candidate)
    }
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

    /// Returns the generation required by both guarded signing and target entry.
    pub(crate) fn durable_generation_ref(&self) -> &ContentRef {
        self.qualification
            .signer_descriptor()
            .durable_generation_ref()
    }

    /// Guardedly prepares one exact candidate before durable authorization.
    pub(crate) async fn prepare_broadcast(
        &self,
        request: &EvmSubmitTransactionRequest,
        allocated_nonce: u64,
        fee_ordinal: u16,
    ) -> Result<PreparedEvmWalletBroadcast, EvmWalletLiveError> {
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
        Ok(PreparedEvmWalletBroadcast {
            descriptor,
            candidate,
            signed,
        })
    }

    /// Performs one authorized `eth_sendRawTransaction` exchange.
    pub(crate) async fn broadcast(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        prepared: PreparedEvmWalletBroadcast,
    ) -> Result<EvmWalletBroadcastReturn, EvmWalletLiveError> {
        self.qualification.verify_request(request)?;
        let PreparedEvmWalletBroadcast {
            descriptor,
            candidate,
            signed,
        } = prepared;
        require_target_entry(&authority, &descriptor, self.durable_generation_ref())?;
        require_candidate_request(request, &candidate)?;
        let signed_hash = signed.transaction_hash();
        if descriptor.operation_id() != EVM_WALLET_BROADCAST_OPERATION_ID
            || parse_hash(candidate.transaction_hash()) != Some(signed_hash)
        {
            return Err(EvmWalletLiveError::InvalidContract);
        }
        let route = self
            .rpc_route(request)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let response = self
            .qualification
            .transport()
            .send_raw_transaction(&route, self.qualification.chain_id(), signed)
            .await;
        let outcome = match response {
            Ok(WalletBroadcastResponse::Accepted(acknowledged)) => {
                if acknowledged != signed_hash {
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
            Ok(WalletBroadcastResponse::AlreadyKnown)
                if request.policy().already_known_classifier_ref()
                    == self.qualification.already_known_classifier_ref() =>
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
            Ok(WalletBroadcastResponse::AlreadyKnown) => self.indeterminate_conflict()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(EvmWalletBroadcastReturn {
            receipt: authority.complete(outcome),
            candidate,
        })
    }

    /// Performs one `eth_getTransactionByHash` exchange.
    pub(crate) async fn transaction_lookup(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        self.qualification.verify_request(request)?;
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::transaction_lookup(request, candidate)?;
        require_target_entry(&authority, &descriptor, self.durable_generation_ref())?;
        let route = self
            .rpc_route(request)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let transaction_hash =
            parse_hash(candidate.transaction_hash()).ok_or(EvmWalletLiveError::InvalidContract)?;
        let response = self
            .qualification
            .transport()
            .transaction_by_hash(&route, self.qualification.chain_id(), transaction_hash)
            .await;
        let outcome = match response {
            Ok(None) => self.returned(
                request,
                EvmWalletAttemptResult::TransactionLookup {
                    transaction_hash: candidate.transaction_hash().to_owned(),
                    transaction: None,
                },
            )?,
            Ok(Some(transaction)) if transaction.matches_candidate(candidate).unwrap_or(false) => {
                self.returned(
                    request,
                    EvmWalletAttemptResult::TransactionLookup {
                        transaction_hash: candidate.transaction_hash().to_owned(),
                        transaction: Some(transaction),
                    },
                )?
            }
            Ok(Some(_)) => self.indeterminate_unclassified()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
    }

    /// Performs one `eth_getTransactionReceipt` exchange.
    pub(crate) async fn receipt_lookup(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        self.qualification.verify_request(request)?;
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::receipt_lookup(request, candidate)?;
        require_target_entry(&authority, &descriptor, self.durable_generation_ref())?;
        let route = self
            .rpc_route(request)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let transaction_hash =
            parse_hash(candidate.transaction_hash()).ok_or(EvmWalletLiveError::InvalidContract)?;
        let response = self
            .qualification
            .transport()
            .receipt_by_hash(&route, self.qualification.chain_id(), transaction_hash)
            .await;
        let outcome = match response {
            Ok(None) => self.returned(
                request,
                EvmWalletAttemptResult::ReceiptLookup {
                    transaction_hash: candidate.transaction_hash().to_owned(),
                    receipt: None,
                },
            )?,
            Ok(Some(receipt)) if receipt.transaction_hash() == candidate.transaction_hash() => self
                .returned(
                    request,
                    EvmWalletAttemptResult::ReceiptLookup {
                        transaction_hash: candidate.transaction_hash().to_owned(),
                        receipt: Some(receipt),
                    },
                )?,
            Ok(Some(_)) => self.indeterminate_unclassified()?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
    }

    /// Performs one `eth_getBlockByNumber("finalized", false)` exchange.
    pub(crate) async fn finalized_head(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        self.qualification.verify_request(request)?;
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::finalized_head(request, candidate)?;
        require_target_entry(&authority, &descriptor, self.durable_generation_ref())?;
        let route = self
            .rpc_route(request)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let response = self
            .qualification
            .transport()
            .finalized_head(&route, self.qualification.chain_id())
            .await;
        let outcome = match response {
            Ok(block) => self.returned(request, EvmWalletAttemptResult::FinalizedHead { block })?,
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
    }

    /// Performs one inclusion-number block lookup and admits terminal evidence only on equality.
    pub(crate) async fn canonical_inclusion(
        &self,
        authority: TargetEntryAuthority,
        request: &EvmSubmitTransactionRequest,
        candidate: &EvmWalletTransactionCandidate,
        inclusion_number: U256,
        terminal_candidate: Option<EvmWalletTerminalEvidence>,
    ) -> Result<TargetOperationReceipt, EvmWalletLiveError> {
        self.qualification.verify_request(request)?;
        require_candidate_request(request, candidate)?;
        let descriptor = EvmWalletTargetEntryDescriptor::canonical_inclusion(
            request,
            candidate,
            inclusion_number,
        )?;
        require_target_entry(&authority, &descriptor, self.durable_generation_ref())?;
        let route = self
            .rpc_route(request)
            .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        let response = self
            .qualification
            .transport()
            .inclusion_block(&route, self.qualification.chain_id(), inclusion_number)
            .await;
        let outcome = match response {
            Ok(None) => self.returned(
                request,
                EvmWalletAttemptResult::CanonicalInclusion {
                    block: None,
                    terminal: None,
                },
            )?,
            Ok(Some(block)) => {
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
            Err(failure) => self.failure_outcome(failure)?,
        };
        Ok(authority.complete(outcome))
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
        failure: WalletRpcFailure,
    ) -> Result<DeliveryAttemptOutcome, EvmWalletLiveError> {
        let (code, class, stage, did_not_enter) = match failure {
            WalletRpcFailure::GenerationFenced => (
                ReferenceFailureCode::GenerationFenced,
                FailureClass::Authorization,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            ),
            WalletRpcFailure::AccessCancelled => (
                ReferenceFailureCode::AccessCancelled,
                FailureClass::Cancellation,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            ),
            WalletRpcFailure::UnavailableBeforeEntry => (
                ReferenceFailureCode::DestinationUnavailable,
                FailureClass::Transport,
                BoundaryStage::BeforeBoundaryEntry,
                true,
            ),
            WalletRpcFailure::ResponseLost => (
                ReferenceFailureCode::DestinationUnavailable,
                FailureClass::Transport,
                BoundaryStage::BoundaryEntry,
                false,
            ),
            WalletRpcFailure::DestinationRejected => (
                ReferenceFailureCode::RequestConflict,
                FailureClass::Destination,
                BoundaryStage::BoundaryObservation,
                false,
            ),
            WalletRpcFailure::InvalidResponse => (
                ReferenceFailureCode::UnclassifiedFailure,
                FailureClass::Unclassified,
                BoundaryStage::BoundaryObservation,
                false,
            ),
        };
        let failure = reference_safe_failure(
            self.qualification
                .executor_binding()
                .contract()
                .safe_failure_contract_ref()
                .clone(),
            code,
            class,
            stage,
        )
        .map_err(|_| EvmWalletLiveError::InvalidContract)?;
        if did_not_enter {
            DeliveryAttemptOutcome::did_not_enter(failure)
        } else {
            DeliveryAttemptOutcome::indeterminate(failure)
        }
        .map_err(|_| EvmWalletLiveError::InvalidContract)
    }

    fn indeterminate_conflict(&self) -> Result<DeliveryAttemptOutcome, EvmWalletLiveError> {
        self.failure_outcome(WalletRpcFailure::DestinationRejected)
    }

    fn indeterminate_unclassified(&self) -> Result<DeliveryAttemptOutcome, EvmWalletLiveError> {
        self.failure_outcome(WalletRpcFailure::InvalidResponse)
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
