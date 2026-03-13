#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared symbol-domain schema and validation helpers for canonical portfolio snapshots.
//!
//! Milestone 1 intentionally keeps this crate at the model layer: serde types, validation, and
//! deterministic normalization helpers only. Runtime `State` implementations land in later
//! milestones.
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_symbol::model::{decode_symbol_config, QuoteCode, SymbolConfig};
//!
//! let raw = serde_json::json!({
//!     "symbol_id": "eth.native.ethereum-mainnet",
//!     "display_symbol": "ETH",
//!     "kind": "native_balance",
//!     "role": "native",
//!     "network_id": "ethereum-mainnet",
//!     "protocol": null,
//!     "balance_reader": { "kind": "native_balance" },
//!     "valuation": {
//!         "quotes": [
//!             {
//!                 "quote": "USD",
//!                 "priced_symbol_id": "eth.native.ethereum-mainnet",
//!                 "reader": {
//!                     "kind": "direct_price",
//!                     "source": {
//!                         "source_id": "chainlink_eth_usd",
//!                         "network_id": "ethereum-mainnet",
//!                         "base_symbol_id": "eth.native.ethereum-mainnet",
//!                         "quote": "USD"
//!                     }
//!                 }
//!             }
//!         ]
//!     },
//!     "decimals": 18,
//!     "underlying_symbol_id": null,
//!     "metadata": {}
//! });
//!
//! let cfg = decode_symbol_config(&raw)?;
//! assert_eq!(cfg.symbol_id, "eth.native.ethereum-mainnet");
//! assert_eq!(cfg.valuation.quotes[0].quote, QuoteCode::Usd);
//! # Ok::<(), mfm_state_symbol::model::SymbolConfigError>(())
//! ```
/// Canonical symbol-domain models, validation helpers, and normalization utilities.
pub mod model;
/// Reusable symbol-domain runtime states for canonical portfolio observations and valuation.
pub mod states;
