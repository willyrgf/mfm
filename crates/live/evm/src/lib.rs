#![warn(missing_docs)]
//! Live EVM integration with a reusable public transport and a private runtime adapter.
//!
//! The adapter is intentionally unavailable as a module; consumers use only the narrow root
//! registration and replay functions.
//!
//! ```compile_fail
//! use mfm_evm_live::adapter::register_evm_balance_runners;
//! ```
//!
//! The checked transport does not expose an arbitrary JSON-RPC call surface.
//!
//! ```compile_fail
//! use mfm_evm_live::transport::EvmJsonRpcSession;
//!
//! async fn bypass(session: &EvmJsonRpcSession) {
//!     let _ = session.rpc_call("eth_chainId", serde_json::json!([])).await;
//! }
//! ```

mod adapter;
pub mod transport;

pub use adapter::{
    is_evm_transaction_replay_intent, register_evm_balance_runners,
    register_evm_transaction_runner, register_evm_validation_runner,
    verify_evm_balance_collection_replay, verify_evm_transaction_replay,
    verify_evm_validation_replay, EvmMutationValidationFuture, EvmReadRunnerCapabilities,
    EvmTransactionRunnerCapabilities,
};

#[cfg(test)]
mod role_tests;
