#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared wallet-domain schema and validation helpers for canonical portfolio snapshots.
//!
//! This crate owns the canonical wallet-domain schema consumed by semantic portfolio planners and
//! runtime adapters.
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_wallet::model::{decode_wallet_config, WalletImplementationConfig};
//!
//! let raw = serde_json::json!({
//!     "wallet_id": "wallet_treasury_eth",
//!     "address": "0x000000000000000000000000000000000000dead",
//!     "network_id": "ethereum-mainnet",
//!     "implementation": { "kind": "address_only" },
//!     "symbol_ids": ["eth.native.ethereum-mainnet"],
//!     "metadata": {}
//! });
//!
//! let cfg = decode_wallet_config(&raw)?;
//! assert!(matches!(cfg.implementation, WalletImplementationConfig::AddressOnly {}));
//! # Ok::<(), mfm_state_wallet::model::WalletConfigError>(())
//! ```
/// Canonical wallet-domain models, validation helpers, and normalization utilities.
pub mod model;
