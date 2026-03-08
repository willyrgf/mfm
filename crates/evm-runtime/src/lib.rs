#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![warn(missing_docs)]
//! Reusable EVM execution helpers and runtime states.
//!
//! This crate holds the shared deploy/configure/validate helpers and runtime-facing EVM states
//! used by thin operation crates.
//!
//! # Examples
//!
//! ```rust
//! use mfm_evm_runtime::dcv::BlockTag;
//!
//! let _tag = BlockTag::Tag("latest".to_string());
//! ```
/// Shared deploy/configure/validate helpers and manifest adapters.
pub mod dcv;
/// JSON-RPC helpers used by reusable EVM states.
pub mod rpc;
/// Reusable EVM read and write state implementations.
pub mod states;
