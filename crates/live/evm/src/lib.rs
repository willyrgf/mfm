#![warn(missing_docs)]
//! Exact-generation EVM transport, audited reads, and sealed wallet qualification.
//!
//! One live-owned [`EvmWalletRequestQualification`] closes the actual transport catalog, executor
//! semantics, guarded signer, nonce/finality/assurance policies, and evidence bounds before app
//! admission or executor allocation. The app admission path, durable wallet executor, and target
//! share that proof. Replay, reducers, ingress validation, and aggregate-reader registration
//! surfaces remain absent.

mod adapter;
pub mod transport;
mod wallet_executor;
mod wallet_qualification;
mod wallet_rpc;

pub use adapter::{
    evm_adapter_callback_surface_canonical, evm_adapter_callback_surface_ref,
    evm_adapter_callback_surface_support_contract, evm_safe_classifier_canonical,
    evm_safe_classifier_contract_ref, evm_safe_classifier_support_contract,
    qualify_evm_read_entries, EvmReadAdapter, EvmReadQualificationArtifacts,
    QualifiedEvmReadEntries, EVM_ADAPTER_CALLBACK_SURFACE_VERSION,
};
pub use wallet_executor::EvmWalletExecutor;
pub use wallet_qualification::EvmWalletRequestQualification;
pub use wallet_rpc::{
    evm_already_known_classifier_canonical, evm_already_known_classifier_ref,
    evm_wallet_target_callback_surface_canonical, evm_wallet_target_callback_surface_ref,
    evm_wallet_target_callback_surface_support_contract, EvmWalletBroadcastReturn,
    EvmWalletJsonRpcTarget, EvmWalletLiveError, EvmWalletRpcClient, EvmWalletRpcError,
    EvmWalletRpcFailure, EvmWalletRpcFuture, EvmWalletRpcResponse, EvmWalletTargetEntryDescriptor,
    PreparedEvmWalletBroadcast, EVM_ALREADY_KNOWN_CLASSIFIER_VERSION,
    EVM_WALLET_TARGET_CALLBACK_SURFACE_VERSION, EVM_WALLET_TARGET_ENTRY_DESCRIPTOR_VERSION,
};

#[cfg(test)]
#[path = "wallet_executor_tests.rs"]
mod wallet_executor_tests;
