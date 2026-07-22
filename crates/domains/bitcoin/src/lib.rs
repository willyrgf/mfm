#![warn(missing_docs)]
//! Pure Bitcoin model, capability, state, and operation contracts.
//!
//! Internal role modules are private and flow in one direction: model, capability, state, then
//! operation. Live protocol and runtime binding code belongs outside this crate.

mod capability;
mod model;
mod operation;
mod state;

pub use capability::{
    BitcoinBalanceCollectionReadCapability, BitcoinBalanceSession, BitcoinSessionFuture,
    BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
pub use model::{
    BitcoinAddress, BitcoinAddressBalance, BitcoinBalanceCollectionRequest,
    BitcoinBalanceCollectionResponse, BitcoinCapabilityError, BitcoinInvalidRequest,
    BitcoinNetworkId, BitcoinNetworkTag, BitcoinSourceBinding, BitcoinSourceIdentity,
    BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT,
};
pub use operation::{
    bitcoin_collectors_authoring_catalog, bitcoin_collectors_operation_registry,
    bitcoin_collectors_state_registry, register_bitcoin_collectors_certification_descriptors,
    BitcoinBalanceCollectionOperation, BitcoinBalanceCollectionOutputs,
};
pub use state::{
    bitcoin_jsonrpc_adapter_kind, bitcoin_jsonrpc_adapter_version,
    decode_bitcoin_balance_snapshot_response, BitcoinBalanceCollectionConfig,
    BitcoinBalanceCollectionError, BitcoinBalanceCollectionEvidence, BitcoinBalanceCollectionPlan,
    BitcoinBalanceCollectionReceipt, BitcoinBalanceSnapshotFact, BitcoinBalanceSnapshotResponse,
    BitcoinBalanceSnapshotSubject, CollectBitcoinBalancesState,
};
