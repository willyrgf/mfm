#![warn(missing_docs)]
//! Live Bitcoin integration with a reusable public transport and a private runtime adapter.
//!
//! The adapter is intentionally unavailable as a module; consumers use only the narrow root
//! registration functions.
//!
//! ```compile_fail
//! use mfm_bitcoin_live::adapter::register_bitcoin_jsonrpc_runners;
//! ```
//!
//! The checked transport does not expose an arbitrary JSON-RPC call surface.
//!
//! ```compile_fail
//! use mfm_bitcoin_live::transport::BitcoinRpcSession;
//!
//! fn bypass(session: &BitcoinRpcSession) {
//!     let _ = session.rpc_call("getblockchaininfo", serde_json::json!([]));
//! }
//! ```

mod adapter;
pub mod transport;

pub use adapter::{register_bitcoin_jsonrpc_runners, verify_bitcoin_jsonrpc_replay};

#[cfg(test)]
mod role_tests;
