#![warn(missing_docs)]
//! Reusable EVM transaction and exact-anchor contract-validation states.
//!
//! This package deliberately owns only two state kinds:
//! [`SubmitEvmTransactionState`] for one `Create` or `Call` transaction and
//! [`ValidateEvmContractState`] for exact code/call checks at one canonical anchor.
//! Portfolio balance collection belongs to `mfm-state-portfolio`.
//!
//! # Examples
//!
//! ```rust
//! use alloy_primitives::{Address, U256};
//! use mfm_signing::SignerRef;
//! use mfm_states_evm::{
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

mod canonical;
mod contract_validation;
mod identity;
mod transaction;

pub use contract_validation::{
    validate_evm_contract, validate_evm_contract_validation_config, EvmContractCallCheck,
    EvmContractCallContext, EvmContractValidationConfig, EvmContractValidationEvidence,
    EvmContractValidationObservation, EvmContractValidationPlan, EvmContractValidationTarget,
    ValidateEvmContractState, VerifiedEvmContract, EVM_CONTRACT_CODE_MAX_BYTES,
    EVM_CONTRACT_VALIDATION_MAX_CALLS, EVM_CONTRACT_VALIDATION_MAX_EVIDENCE_BYTES,
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
