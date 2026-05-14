#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Reusable keystore-oriented runtime states.
//!
//! This crate keeps local keystore execution logic out of binaries and thin op planners by
//! packaging reusable `State` implementations for key management and transaction signing
//! workflows. Pure local-keystore IO DTOs and transaction input helpers live in
//! `mfm-collectors-local-keystore`; live keystore signing stays in `mfm-transports-local-keystore`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_local_keystore::tx::{output_context_key, parse_address, Eip1559TxToSign};
//! use mfm_machine::ids::ContextKey;
//!
//! let tx = Eip1559TxToSign {
//!     to: parse_address("0x0000000000000000000000000000000000000000", "to")?,
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
//! # Ok::<(), mfm_collectors_local_keystore::tx::KeystoreTxError>(())
//! ```

/// Local keystore-specific `State` implementations used by op planners.
pub mod states;
