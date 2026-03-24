#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared Aave V3 state-layer types and runtime states.
//!
//! This crate holds reusable manifest contracts and execution states for:
//! - Aave V3 deploy/configure flows
//! - compiled Aave V3 semantic portfolio adapters
//!
//! Operations should wire these states into plans rather than reimplementing the runtime behavior
//! in binary or op layers.
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_aave_v3::manifest::AaveDeployRuntimeConfig;
//!
//! let _cfg = AaveDeployRuntimeConfig {
//!     network_id: "ethereum-mainnet".to_string(),
//!     control_scope: "shared".to_string(),
//!     compile_manifest_port: "result".to_string(),
//!     deployer_account_index: 0,
//!     signing_key_env: None,
//!     poll_interval_ms: 200,
//!     max_receipt_polls: 120,
//!     deploy_manifest_export_key: "deploy_manifest".to_string(),
//! };
//! ```
/// Manifest types and validation helpers for Aave V3 deploy/configure flows.
pub mod manifest;
/// Shared Aave V3 portfolio-position config models and runtime states.
pub mod portfolio;
/// Reusable runtime `State` implementations for Aave V3 deploy/configure flows.
pub mod states;
