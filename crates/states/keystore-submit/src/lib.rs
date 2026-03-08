#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Reusable remote-capable keystore submission state.
//!
//! This crate isolates the raw-transaction submission path that depends on EVM RPC helpers so the
//! local keystore admin and signing crate can remain free of remote-capable dependencies.
//!
//! # Examples
//!
//! ```rust
//! use std::path::PathBuf;
//!
//! use mfm_state_keystore_submit::tx::KeystoreTxSendRawStateConfig;
//!
//! let cfg = KeystoreTxSendRawStateConfig {
//!     route_source_id: "reth_local".to_string(),
//!     input_path: PathBuf::from("/tmp/signed.raw"),
//! };
//!
//! assert_eq!(cfg.route_source_id, "reth_local");
//! ```

/// Remote-capable transaction submission state used by thin op planners.
pub mod tx;
