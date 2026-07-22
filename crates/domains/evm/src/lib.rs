#![warn(missing_docs)]
//! Pure EVM model, capability, signing, state, and operation contracts.
//!
//! Internal role modules are private and flow in one direction: model, capability, signing, state,
//! then operation. Live protocol and runtime binding code belongs outside this crate.

mod capability;
mod model;
mod operation;
mod signing;
mod state;

pub use capability::{
    evm_diagnostic, source_mismatch_error, EvmBlockSelector, EvmCall, EvmCapabilityError,
    EvmCapabilityFailureDisposition, EvmCapabilityPhase, EvmCode, EvmFeeInputs, EvmInvalidRequest,
    EvmNetworkBinding, EvmObservedTransaction, EvmReadCapability, EvmReadSession,
    EvmReadSessionSet, EvmReceipt, EvmReceiptLog, EvmReceiptStatus, EvmSessionEvidence,
    EvmSessionFuture, EvmTransactionCapability, EvmTransactionEstimate, EvmTransactionPlacement,
    EvmTransactionSession, EvmTransactionSessionSet, Result as EvmCapabilityResult,
    EVM_CALL_MAX_RESPONSE_BYTES, EVM_CODE_MAX_RESPONSE_BYTES, EVM_EIP1559_TRANSACTION_TYPE,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
pub use model::{EvmBlockAnchor, EvmBlockAnchorError};
pub use operation::{
    evm_collectors_operation_registry, evm_collectors_state_registry,
    register_evm_collectors_certification_descriptors, EvmBalanceCollectionOperation,
    EvmBalanceCollectionOutputs,
};
pub use signing::{
    sign_eip1559, Eip1559QuantityField, EvmSignatureError, EvmSigningError,
    TransientSignedEip1559Envelope, UnsignedEip1559Envelope,
};
pub use state::{
    decode_evm_balance_snapshot_response, evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version,
    evm_sender_lane_resource_claim, reduce_evm_balance_collection, CollectEvmBalancesState,
    EvmAccessListEntry, EvmBalanceAsset, EvmBalanceCollectionConfig, EvmBalanceCollectionError,
    EvmBalanceCollectionEvidence, EvmBalanceCollectionPlan, EvmBalanceCollectionReceipt,
    EvmBalanceReadEvidence, EvmBalanceSnapshotFact, EvmBalanceSnapshotResponse,
    EvmBalanceSnapshotSubject, EvmBalanceSource, EvmContractCallCheck, EvmContractCallContext,
    EvmContractValidationConfig, EvmContractValidationEvidence,
    EvmContractValidationEvidenceBuilder, EvmContractValidationObservation,
    EvmContractValidationPlan, EvmContractValidationTarget, EvmExecutionStatus,
    EvmPreparedTransaction, EvmSenderLane, EvmStateError, EvmTokenDecimalsEvidence,
    EvmTransactionAction, EvmTransactionActionKind, EvmTransactionConfig,
    EvmTransactionConfirmation, EvmTransactionIntent, EvmTransactionLog, EvmTransactionOutcome,
    EvmTransactionReceipt, EvmTransactionRecoveryEvidence, EvmTransactionResult,
    EvmTransactionSubmission, EvmTransactionSuccess, EvmUnsignedTransaction,
    SubmitEvmTransactionState, ValidateEvmContractState, VerifiedEvmContract,
    EVM_BALANCE_COLLECTION_SOURCE_LIMIT, EVM_CONTRACT_VALIDATION_MAX_CALLS,
    EVM_CONTRACT_VALIDATION_MAX_EVIDENCE_BYTES,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_ENTRIES,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_STORAGE_KEYS,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_CALLDATA_BYTES,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES, EVM_GAS_POLICY, EVM_SENDER_LANE_NAMESPACE,
    EVM_TRANSACTION_DATA_MAX_BYTES, EVM_TRANSACTION_FEE_POLICY,
};

#[cfg(test)]
mod role_tests;
