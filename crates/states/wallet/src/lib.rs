#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared wallet-domain schema and validation helpers for canonical portfolio snapshots.
//!
//! Milestone 1 keeps this crate model-only so later ops and binaries can depend on a single
//! canonical wallet surface without pulling in execution logic prematurely.
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
/// Reusable wallet-domain runtime states for canonical portfolio execution.
pub mod states;
