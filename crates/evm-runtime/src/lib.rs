#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Reusable EVM execution helpers and runtime states.
//!
//! This crate holds runtime-facing EVM states used by thin operation crates. Pure
//! deploy/configure/validate models and ABI preparation helpers live in `mfm-evm-dcv-model`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_dcv_model::BlockTag;
//!
//! let _tag = BlockTag::Tag {
//!     tag: "latest".to_string(),
//! };
//! ```
/// Generic compiled/deployed contract-set manifests.
pub mod contract_set;
/// JSON-RPC helpers used by reusable EVM states.
pub mod rpc;
/// Reusable EVM read and write state implementations.
pub mod states;
/// Durable EVM transaction intent records.
pub mod tx_intent;
