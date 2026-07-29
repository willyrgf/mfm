#![warn(missing_docs)]
//! Exact-generation EVM JSON-RPC transport, audited reads, and durable wallet target bindings.
//!
//! The wallet executor uses the domain-free keyed ledger and guarded signer.
//! Replay, reducers, ingress validation, and aggregate-reader registration
//! surfaces remain absent.

mod adapter;
pub mod transport;
mod wallet_executor;
mod wallet_rpc;

pub use adapter::{
    evm_adapter_callback_surface_canonical, evm_adapter_callback_surface_ref,
    evm_adapter_callback_surface_support_contract, evm_safe_classifier_canonical,
    evm_safe_classifier_contract_ref, evm_safe_classifier_support_contract,
    qualify_evm_read_entries, EvmReadAdapter, EvmReadQualificationArtifacts,
    QualifiedEvmReadEntries, EVM_ADAPTER_CALLBACK_SURFACE_VERSION,
};
pub use wallet_executor::EvmWalletExecutor;
pub use wallet_rpc::{
    evm_already_known_classifier_canonical, evm_already_known_classifier_ref,
    evm_wallet_target_callback_surface_canonical, evm_wallet_target_callback_surface_ref,
    evm_wallet_target_callback_surface_support_contract, EvmWalletBroadcastReturn,
    EvmWalletJsonRpcTarget, EvmWalletLiveError, EvmWalletRpcClient, EvmWalletRpcError,
    EvmWalletRpcFailure, EvmWalletRpcFuture, EvmWalletRpcResponse, EvmWalletSignerBinding,
    EvmWalletTargetEntryDescriptor, PreparedEvmWalletBroadcast,
    EVM_ALREADY_KNOWN_CLASSIFIER_VERSION, EVM_WALLET_TARGET_CALLBACK_SURFACE_VERSION,
    EVM_WALLET_TARGET_ENTRY_DESCRIPTOR_VERSION,
};

#[cfg(test)]
#[path = "wallet_executor_tests.rs"]
mod wallet_executor_tests;
