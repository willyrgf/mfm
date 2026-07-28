#![warn(missing_docs)]
//! Pure EVM protocol, signing, audited-read, and recoverable wallet-effect contracts.
//!
//! The package owns deterministic state and graph semantics. Live RPC, guarded
//! signing, durable delivery, and recovery remain adapter responsibilities.

mod capability;
mod model;
mod operation;
mod product;
mod qualification;
mod signing;
mod state;
mod wallet;
mod wallet_state;

pub use capability::{
    EvmAnchorConfirmationRequest, EvmAnchoredSource, EvmBlockResponse, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCheckedSource, EvmCoarseSizeClass, EvmFeeInputs,
    EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmNetworkBinding, EvmObservedTransaction,
    EvmProtocolError, EvmQuantityResponse, EvmReadFailure, EvmReceipt, EvmReceiptLog,
    EvmReceiptStatus, EvmResponseInvalidKind, EvmRoutingGenerationRef, EvmSafeFailure,
    EvmTokenBalanceRequest, EvmTokenDecimalsRequest, EvmTokenDecimalsResponse,
    EvmTransactionEstimate, EvmTransactionPlacement, Result as EvmProtocolResult,
    EVM_CHAIN_ID_OPERATION_ID, EVM_CONFIRM_ANCHOR_OPERATION_ID, EVM_EIP1559_TRANSACTION_TYPE,
    EVM_LATEST_ANCHOR_OPERATION_ID, EVM_NATIVE_BALANCE_OPERATION_ID, EVM_READ_MAX_RESPONSE_BYTES,
    EVM_READ_OPERATION_IDS, EVM_TOKEN_BALANCE_OPERATION_ID, EVM_TOKEN_DECIMALS_OPERATION_ID,
};
pub use model::{EvmBlockAnchor, EvmBlockAnchorError};
pub use operation::{
    evm_submit_transaction_entry_point_contract, evm_submit_transaction_planning_profile,
    EvmBalanceCollectionAuthoringInputs, EvmBalanceCollectionOperation,
    EvmBalanceCollectionOutputs, EvmSubmitTransactionAuthoringInputs,
    EvmSubmitTransactionOperation, EvmSubmitTransactionOutputs,
    EVM_BALANCE_COLLECTION_OPERATION_ID,
};
pub use product::{
    evm_submit_transaction_entry_point_registration,
    evm_submit_transaction_unit_config_member_path, EvmSubmitTransactionEntryPointArtifacts,
    EVM_SUBMIT_TRANSACTION_UNIT_CONFIG_MEMBER_PATH,
};
pub use qualification::{
    evm_balance_collection_callback_surfaces, evm_balance_collection_value_contracts,
    evm_balance_fact_support_objects, evm_read_value_contracts,
    evm_submit_transaction_callback_surface, evm_submit_transaction_leaf_expansion,
    evm_submit_transaction_value_contracts, qualify_evm_balance_collection_states,
    qualify_evm_submit_transaction_state, EvmBalanceCollectionCallbackSurfaces,
    EvmBalanceCollectionStateArtifacts, EvmBalanceCollectionStateImplementations,
    EvmBalanceCollectionValueContracts, EvmBalanceFactSupportObjects, EvmReadValueContracts,
    EvmStateCallbackSurface, EvmSubmitTransactionStateArtifacts,
    EvmSubmitTransactionValueContracts, QualifiedEvmBalanceCollectionStates,
    QualifiedEvmSubmitTransactionState, EVM_STATE_CALLBACK_SURFACE_VERSION,
};
pub use signing::{
    sign_eip1559, sign_eip1559_guarded, Eip1559QuantityField, EvmSignatureError, EvmSigningError,
    TransientSignedEip1559Envelope, UnsignedEip1559Envelope,
};
pub use state::{
    evm_read_capability_contract_canonical, evm_read_capability_contract_ref,
    evm_read_capability_support_contract, evm_safe_failure_contract_canonical,
    evm_safe_failure_contract_ref, evm_safe_failure_support_contract,
    validate_evm_balance_collection_config, AggregateEvmBalancesState, BootstrapEvmSourceState,
    ConfirmEvmAnchorState, EvmAnchorConfirmationInput, EvmBalanceAggregationInput, EvmBalanceAsset,
    EvmBalanceCollection, EvmBalanceCollectionConfig, EvmBalanceCollectionError,
    EvmBalanceGraphResult, EvmBalanceReadInput, EvmBalanceSnapshotFact, EvmBalanceSource,
    EvmBootstrapInput, EvmCollectedBalance, EvmInitialAnchorInput, EvmTokenDecimalsInput,
    ReadEvmInitialAnchorState, ReadEvmNativeBalanceState, ReadEvmTokenBalanceState,
    ReadEvmTokenDecimalsState, EVM_BALANCE_COLLECTION_NODE_LIMIT,
    EVM_BALANCE_COLLECTION_SOURCE_LIMIT, EVM_BALANCE_COLLECTION_TOKEN_LIMIT,
};
pub use wallet::{
    EvmSubmitTransactionFailure, EvmSubmitTransactionInput, EvmSubmitTransactionPublicOutputs,
    EvmSubmitTransactionRequest, EvmSubmitTransactionSelector, EvmTransactionOutcome,
    EvmTransactionTarget, EvmWalletAccessListEntry, EvmWalletAttemptResult,
    EvmWalletBroadcastStatus, EvmWalletConvergencePlan, EvmWalletError, EvmWalletFeeCandidate,
    EvmWalletObservedTransaction, EvmWalletPolicy, EvmWalletReceipt, EvmWalletReceiptLog,
    EvmWalletReceiptStatus, EvmWalletReference, EvmWalletReplacementPolicy,
    EvmWalletTerminalEvidence, EvmWalletTransactionAction, EvmWalletTransactionCandidate,
    EvmWalletTransactionPlacement, EvmWalletTransactionTemplate,
    EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID, EVM_SUBMIT_TRANSACTION_OPERATION_ID,
    EVM_WALLET_ACCESS_LIST_MAX_ENTRIES, EVM_WALLET_ACCESS_LIST_MAX_STORAGE_KEYS,
    EVM_WALLET_BROADCAST_OPERATION_ID, EVM_WALLET_DATA_MAX_BYTES,
    EVM_WALLET_EXECUTOR_RECORD_OVERHEAD_BYTES, EVM_WALLET_FINALITY_TAG,
    EVM_WALLET_FINALIZED_HEAD_OPERATION_ID, EVM_WALLET_INCLUSION_BLOCK_OPERATION_ID,
    EVM_WALLET_RECEIPT_LOG_DATA_MAX_BYTES, EVM_WALLET_RECEIPT_LOG_LIMIT,
    EVM_WALLET_RECEIPT_LOOKUP_OPERATION_ID, EVM_WALLET_REPLACEMENT_LIMIT,
    EVM_WALLET_TRANSACTION_LOOKUP_OPERATION_ID, EVM_WALLET_TRANSACTION_TYPE,
};
pub use wallet_state::{
    SubmitEvmTransactionState, EVM_WALLET_REVERTED_TERMINAL_OUTCOME,
    EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
};

#[cfg(test)]
#[path = "capability_tests.rs"]
mod capability_tests;
