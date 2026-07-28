#![warn(missing_docs)]
//! Pure EVM protocol, signing, audited-read state, and graph contracts.
//!
//! Production mutation lifecycle and aggregate capability surfaces are
//! intentionally absent. Signing and transaction models remain reusable
//! protocol primitives for a future qualified executor.

mod capability;
mod model;
mod operation;
mod qualification;
mod signing;
mod state;

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
    EvmBalanceCollectionAuthoringInputs, EvmBalanceCollectionOperation,
    EvmBalanceCollectionOutputs, EVM_BALANCE_COLLECTION_OPERATION_ID,
};
pub use qualification::{
    evm_balance_collection_callback_surfaces, evm_balance_collection_value_contracts,
    evm_balance_fact_support_objects, evm_read_value_contracts,
    qualify_evm_balance_collection_states, EvmBalanceCollectionCallbackSurfaces,
    EvmBalanceCollectionStateArtifacts, EvmBalanceCollectionStateImplementations,
    EvmBalanceCollectionValueContracts, EvmBalanceFactSupportObjects, EvmReadValueContracts,
    EvmStateCallbackSurface, QualifiedEvmBalanceCollectionStates,
    EVM_STATE_CALLBACK_SURFACE_VERSION,
};
pub use signing::{
    sign_eip1559, Eip1559QuantityField, EvmSignatureError, EvmSigningError,
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

#[cfg(test)]
#[path = "capability_tests.rs"]
mod capability_tests;
