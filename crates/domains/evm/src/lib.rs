#![warn(missing_docs)]
//! Pure EVM protocol, signing, audited-read, and recoverable wallet-effect contracts.
//!
//! The package owns deterministic structured-operation semantics. Live RPC, guarded
//! signing, durable delivery, and recovery remain adapter responsibilities.

mod balance;
mod capability;
mod chain_registry;
mod model;
mod signing;
mod structured_balance;
mod submission;
mod submission_expansion;
mod submission_process;
mod submission_registry;
mod wallet;
mod wallet_authority;

pub use balance::{
    validate_evm_balance_collection_config, EvmBalanceAsset, EvmBalanceCollection,
    EvmBalanceCollectionConfig, EvmBalanceCollectionError, EvmBalanceSource, EvmCollectedBalance,
    EVM_BALANCE_COLLECTION_SOURCE_LIMIT, EVM_BALANCE_COLLECTION_STATE_LIMIT,
    EVM_BALANCE_COLLECTION_TOKEN_LIMIT,
};
pub use capability::{
    EvmAnchorConfirmationRequest, EvmAnchoredSource, EvmBlockResponse, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCheckedSource, EvmCoarseSizeClass, EvmFeeInputs,
    EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmNetworkBinding, EvmObservedTransaction,
    EvmProtocolError, EvmQuantityResponse, EvmReadFailure, EvmReceipt, EvmReceiptLog,
    EvmReceiptStatus, EvmResponseInvalidKind, EvmRoutingGenerationRef, EvmSafeDiagnostic,
    EvmSafeFailure, EvmTokenBalanceRequest, EvmTokenDecimalsRequest, EvmTokenDecimalsResponse,
    EvmTransactionEstimate, EvmTransactionPlacement, Result as EvmProtocolResult,
    EVM_CHAIN_ID_OPERATION_ID, EVM_CONFIRM_ANCHOR_OPERATION_ID, EVM_EIP1559_TRANSACTION_TYPE,
    EVM_LATEST_ANCHOR_OPERATION_ID, EVM_NATIVE_BALANCE_OPERATION_ID, EVM_READ_MAX_RESPONSE_BYTES,
    EVM_READ_OPERATION_IDS, EVM_TOKEN_BALANCE_OPERATION_ID, EVM_TOKEN_DECIMALS_OPERATION_ID,
};
pub use chain_registry::{
    ChainInstanceRegistryAttestation, EvmChainInstanceBinding, EvmRoutingCatalogDescriptor,
    EvmRoutingGenerationDescriptor, EVM_JSON_RPC_PROVIDER_CLASS, EVM_ROUTE_POLICY_ID,
    EVM_ROUTE_POLICY_VERSION, EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION,
    EVM_ROUTING_GENERATION_DESCRIPTOR_VERSION,
};
pub use model::{EvmBlockAnchor, EvmBlockAnchorError};
pub use signing::{
    derive_evm_semantic_signer_id, sign_eip1559, sign_eip1559_guarded, signing_failure_is_integrity,
    AccountAddress, Eip1559QuantityField, EvmSignatureError, EvmSigningError,
    TransientSignedEip1559Envelope, UnsignedEip1559Envelope, EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES,
};
pub use structured_balance::{
    author_evm_balance_fan_out, author_evm_balance_lane_selection, balance_adapter_contract,
    register_evm_balance_process, structured_evm_balance_collection_program,
    EvmBalanceCollectionResult, EvmBalanceLaneCursor, EvmBalanceLaneInput, EvmBalanceLaneResult,
    EvmBalanceOperationInput, EvmChainIdentityCapability, EvmConfirmAnchorCapability,
    EvmLatestAnchorCapability, EvmNativeBalanceCapability, EvmTokenBalanceCapability,
    EvmTokenDecimalsCapability, EVM_BALANCE_PROGRAM_LANE_LIMIT,
    STRUCTURED_EVM_BALANCE_COLLECTION_OPERATION_ID,
};
pub use submission::{
    evm_broadcast_adapter_contract, evm_finalized_head_adapter_contract,
    evm_inclusion_block_adapter_contract, evm_pending_nonce_adapter_contract,
    evm_receipt_lookup_adapter_contract, evm_signer_attestation_adapter_contract,
    evm_transaction_lookup_adapter_contract, structured_evm_submission_entry_program,
    AttestCandidateIdentityCapability, AttestCandidateIdentityRequest,
    BroadcastExactCandidateCapability, BroadcastExactCandidateRequest, BroadcastLineageHead,
    CandidateTransactionObservation, EvmBroadcastResource, EvmCandidateSigner,
    EvmDeploymentSubmissionSemantics, EvmFinalizedHeadCapability, EvmFinalizedHeadObservation,
    EvmFinalizedHeadRequest, EvmInclusionBlockCapability, EvmInclusionBlockObservation,
    EvmInclusionBlockRequest, EvmPendingNonceCapability, EvmPendingNonceRequest,
    EvmReceiptLookupCapability, EvmReceiptLookupObservation, EvmReceiptLookupRequest,
    EvmSubmissionConfiguration, EvmSubmissionExpansion, EvmSubmissionOutput, EvmSubmissionRequest,
    EvmTransactionLookupCapability, EvmTransactionLookupObservation, EvmTransactionLookupRequest,
    StructuredSubmitEvmTransactionState, SubmittedCandidateProof, UnsignedWalletCandidate,
};
pub use submission_registry::{
    register_evm_submission_process, EvmSubmissionCapabilityImplementation,
    EvmSubmissionProcessQualification,
};
pub use wallet::{
    evm_deterministic_signing_profile_canonical, evm_deterministic_signing_profile_ref,
    evm_submission_expansion_policy_canonical, evm_submission_expansion_policy_ref,
    evm_wallet_assurance_policy_canonical, evm_wallet_assurance_policy_ref,
    evm_wallet_finality_policy_canonical, evm_wallet_finality_policy_ref,
    evm_wallet_nonce_policy_canonical, evm_wallet_nonce_policy_ref, EvmCallerSubmissionToken,
    EvmSubmissionFailure, EvmSubmitTransactionSelector, EvmTransactionTarget,
    EvmWalletAccessListEntry, EvmWalletError, EvmWalletFeeCandidate, EvmWalletObservedTransaction,
    EvmWalletReceipt, EvmWalletReceiptLog, EvmWalletReceiptStatus, EvmWalletReference,
    EvmWalletTransactionAction, EvmWalletTransactionPlacement, EvmWalletTransactionTemplate,
    EVM_CALLER_SUBMISSION_TOKEN_MAX_BYTES, EVM_DETERMINISTIC_SIGNING_PROFILE_VERSION,
    EVM_SUBMISSION_EXPANSION_POLICY_VERSION, EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID,
    EVM_SUBMIT_TRANSACTION_OPERATION_ID, EVM_WALLET_ACCESS_LIST_MAX_ENTRIES,
    EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS, EVM_WALLET_ASSURANCE_POLICY_VERSION,
    EVM_WALLET_BROADCAST_OPERATION_ID, EVM_WALLET_DATA_MAX_BYTES,
    EVM_WALLET_FINALITY_POLICY_VERSION, EVM_WALLET_FINALITY_TAG,
    EVM_WALLET_FINALIZED_HEAD_OPERATION_ID, EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
    EVM_WALLET_NONCE_POLICY_VERSION, EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES,
    EVM_WALLET_RECEIPT_LOG_LIMIT, EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID,
    EVM_WALLET_REPLACEMENT_LIMIT, EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID,
    EVM_WALLET_TRANSACTION_TYPE,
};
pub use wallet_authority::{
    activate_wallet_candidate_adapter_contract, canonical_wallet_reference,
    complete_wallet_nonce_adapter_contract, derive_authenticated_intent_issuer_id,
    derive_evm_candidate_operation_key, derive_evm_chain_lineage_id,
    derive_evm_nonce_completion_key, derive_evm_nonce_reservation_key,
    derive_exact_candidate_activation_permit, derive_qualified_chain_instance_id,
    derive_submission_intent_id, derive_wallet_nonce_domain,
    read_wallet_nonce_status_adapter_contract, reserve_wallet_nonce_adapter_contract,
    validate_active_wallet_candidate_prefix, ActivateCandidateResponse,
    ActivateEvmCandidateRequest, ActivateWalletCandidateCapability, ActiveWalletCandidate,
    AttestedWalletCandidate, AuthenticatedIntentIssuerId, CandidateActivationPermit,
    CandidateProgressionConflict, CanonicalTerminalOutcome, ChainInstanceDeclaration,
    CompleteEvmNonceRequest, CompleteWalletNonceCapability, CompleteWalletNonceResponse,
    CompletedWalletNonce, EvmCandidateFamily, EvmCandidateOperationKey, EvmChainLineageId,
    EvmNonceCompletionKey, EvmNonceReservationKey, EvmTransactionIntent, ExclusiveCurrentControl,
    ExecutionDisposition, ObservedPendingNonceFloor, PriorEffectDisposition,
    PriorResourceDisposition, QualifiedChainInstanceId, QualifiedPendingNonceFloor,
    ReadEvmWalletNonceStatusRequest, ReadWalletNonceStatusCapability, ReplayExclusionDisposition,
    ReserveEvmNonceRequest, ReserveWalletNonceCapability, ReserveWalletNonceResponse,
    ReservedWalletNonce, SubmissionIntentId, TerminalWitnesses, TransactionNonce,
    WalletAuthorityContractError, WalletNonceAuthority, WalletNonceAuthorityResource,
    WalletNonceDomain, EVM_TRANSACTION_NONCE_MAX,
    WalletNonceDomainActivationAttestation, WalletNonceDomainActivationRecord, WalletNonceStatus,
    WalletNonceStoreIncarnation, WalletNonceStoreLineageHead, WalletNonceStorePromotionAttestation,
    WalletNonceStoreSuccessor, EVM_WALLET_OBSERVATION_ROUND_LIMIT,
};

#[cfg(test)]
#[path = "capability_tests.rs"]
mod capability_tests;

#[cfg(test)]
#[path = "submission_tests.rs"]
mod submission_tests;
