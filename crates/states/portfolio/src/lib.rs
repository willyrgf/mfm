#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared portfolio-domain schema and semantic runtime for canonical portfolio snapshots.
//!
//! This crate owns the canonical portfolio-domain schema plus the fixed semantic runtime,
//! planner/runtime payloads, and runtime adapters used by `portfolio_tracker v2`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_portfolio::model::decode_portfolio_config;
//! use mfm_state_symbol::model::QuoteCode;
//!
//! let raw = serde_json::json!({
//!     "portfolio_id": "portfolio_main",
//!     "quote_codes": ["USD"],
//!     "networks": [
//!         {
//!             "network_id": "ethereum-mainnet",
//!             "chain_id": 1,
//!             "metadata": {}
//!         }
//!     ],
//!     "wallets": [
//!         {
//!             "wallet_id": "wallet_treasury_eth",
//!             "address": "0x000000000000000000000000000000000000dead",
//!             "implementation": { "kind": "address_only" },
//!             "network_id": "ethereum-mainnet",
//!             "symbol_ids": ["eth.native.ethereum-mainnet"],
//!             "metadata": {}
//!         }
//!     ],
//!     "symbol_configs": [
//!         {
//!             "symbol_id": "eth.native.ethereum-mainnet",
//!             "display_symbol": "ETH",
//!             "kind": "native_balance",
//!             "role": "native",
//!             "network_id": "ethereum-mainnet",
//!             "protocol": null,
//!             "balance_reader": { "kind": "native_balance" },
//!             "valuation": {
//!                 "quotes": [
//!                     {
//!                         "quote": "USD",
//!                         "priced_symbol_id": "eth.native.ethereum-mainnet",
//!                         "reader": {
//!                             "kind": "direct_price",
//!                             "source": {
//!                                 "source_id": "chainlink_eth_usd",
//!                                 "network_id": "ethereum-mainnet",
//!                                 "base_symbol_id": "eth.native.ethereum-mainnet",
//!                                 "quote": "USD"
//!                             }
//!                         }
//!                     }
//!                 ]
//!             },
//!             "decimals": 18,
//!             "underlying_symbol_id": null,
//!             "metadata": {}
//!         }
//!     ],
//!     "metadata": {}
//! });
//!
//! let cfg = decode_portfolio_config(&raw)?;
//! assert_eq!(cfg.quote_codes, vec![QuoteCode::Usd]);
//! # Ok::<(), mfm_state_portfolio::model::PortfolioConfigError>(())
//! ```
/// Canonical portfolio-domain models, validation helpers, and normalization utilities.
pub mod model;
/// Semantic portfolio vocabulary, compiled execution specs, and validation helpers.
pub mod semantic;
/// Base semantic runtime adapters shared by portfolio execution flows.
pub mod semantic_adapters;
/// Fixed semantic runtime state family for compiled portfolio execution.
pub mod semantic_states;
