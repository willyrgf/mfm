//! Reusable EVM balance, transaction, and exact-anchor validation states.
//!
//! This package owns exactly three state kinds: [`CollectEvmBalancesState`],
//! [`SubmitEvmTransactionState`], and [`ValidateEvmContractState`].
//!
//! # Examples
//!
//! ```rust
//! use alloy_primitives::{Address, U256};
//! use mfm_signing::SignerRef;
//! use mfm_evm::{
//!     EvmTransactionAction, EvmTransactionActionKind, EvmTransactionConfig,
//! };
//!
//! let config = EvmTransactionConfig::new(
//!     "ethereum-mainnet",
//!     1,
//!     Address::from([0x11; 20]),
//!     SignerRef::new("treasury")?,
//!     Vec::new(),
//! )?;
//! let action = EvmTransactionAction::call(Address::from([0x22; 20]), [], U256::ZERO)?;
//! assert_eq!(config.chain_id(), 1);
//! assert_eq!(action.action_kind(), EvmTransactionActionKind::Call);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#[path = "state/balance_collection.rs"]
mod balance_collection;
#[path = "state/canonical.rs"]
mod canonical;
#[path = "state/contract_validation.rs"]
mod contract_validation;
#[path = "state/identity.rs"]
mod identity;
#[path = "state/transaction.rs"]
mod transaction;

#[cfg(test)]
#[path = "state/balance_collection_tests.rs"]
mod balance_collection_tests;

pub use balance_collection::{
    decode_evm_balance_snapshot_response, reduce_evm_balance_collection, CollectEvmBalancesState,
    EvmBalanceAsset, EvmBalanceCollectionConfig, EvmBalanceCollectionError,
    EvmBalanceCollectionEvidence, EvmBalanceCollectionPlan, EvmBalanceCollectionReceipt,
    EvmBalanceReadEvidence, EvmBalanceSnapshotFact, EvmBalanceSnapshotResponse,
    EvmBalanceSnapshotSubject, EvmBalanceSource, EvmTokenDecimalsEvidence,
    EVM_BALANCE_COLLECTION_SOURCE_LIMIT,
};
pub use contract_validation::{
    EvmContractCallCheck, EvmContractCallContext, EvmContractValidationConfig,
    EvmContractValidationEvidence, EvmContractValidationEvidenceBuilder,
    EvmContractValidationObservation, EvmContractValidationPlan, EvmContractValidationTarget,
    ValidateEvmContractState, VerifiedEvmContract, EVM_CONTRACT_VALIDATION_MAX_CALLS,
    EVM_CONTRACT_VALIDATION_MAX_EVIDENCE_BYTES,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_ENTRIES,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_ACCESS_LIST_STORAGE_KEYS,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_CALLDATA_BYTES,
    EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES,
};
pub use identity::{evm_jsonrpc_adapter_kind, evm_jsonrpc_adapter_version};
pub use transaction::{
    evm_sender_lane_resource_claim, EvmAccessListEntry, EvmExecutionStatus, EvmPreparedTransaction,
    EvmSenderLane, EvmTransactionAction, EvmTransactionActionKind, EvmTransactionConfig,
    EvmTransactionConfirmation, EvmTransactionIntent, EvmTransactionLog, EvmTransactionOutcome,
    EvmTransactionReceipt, EvmTransactionRecoveryEvidence, EvmTransactionResult,
    EvmTransactionSubmission, EvmTransactionSuccess, EvmUnsignedTransaction,
    SubmitEvmTransactionState, EVM_GAS_POLICY, EVM_SENDER_LANE_NAMESPACE,
    EVM_TRANSACTION_DATA_MAX_BYTES, EVM_TRANSACTION_FEE_POLICY,
};

use mfm_program::StateError;

/// Redaction-safe failure from a reusable EVM state contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmStateError {
    /// Certified config, input, or retained evidence was invalid.
    #[error("EVM state input was invalid: {reason}")]
    InvalidInput {
        /// Stable redaction-safe reason.
        reason: String,
    },
}

impl EvmStateError {
    fn invalid(reason: String) -> Self {
        Self::InvalidInput { reason }
    }
}

impl From<EvmStateError> for StateError {
    fn from(error: EvmStateError) -> Self {
        Self::Message(error.to_string())
    }
}
