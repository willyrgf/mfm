#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Reusable keystore-oriented runtime states and signing helpers.
//!
//! This crate keeps local keystore execution logic out of binaries and thin op planners by
//! packaging the reusable `State` implementations and transaction helpers needed for key
//! management and transaction signing workflows.
//!
//! # Examples
//!
//! ```rust
//! use alloy_primitives::Address;
//! use mfm_machine::ids::ContextKey;
//! use mfm_state_keystore::tx::{output_context_key, Eip1559TxToSign};
//!
//! let tx = Eip1559TxToSign {
//!     to: Address::from([0u8; 20]),
//!     value_wei: 0,
//!     chain_id: 1,
//!     nonce: 0,
//!     max_fee_per_gas: 1,
//!     max_priority_fee_per_gas: 1,
//!     gas_limit: 21_000,
//!     data: Vec::new(),
//! };
//! let report_key: ContextKey = output_context_key("keystore_tx.sign");
//!
//! assert_eq!(tx.chain_id, 1);
//! assert_eq!(report_key.0, "keystore_tx.sign.out.report");
//! ```

/// Local keystore-specific `State` implementations used by op planners.
pub mod states;
/// Transaction parsing, signing, and output helpers for keystore flows.
pub mod tx;
