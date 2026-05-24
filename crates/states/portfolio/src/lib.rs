#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared portfolio-domain semantic runtime for canonical portfolio snapshots.
//!
//! Canonical schema lives in `mfm-portfolio-model`, semantic planning vocabulary lives in
//! `mfm-portfolio-plan`, and this crate owns the reusable runtime adapters and production states
//! used by `portfolio_tracker`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_portfolio_model::portfolio::decode_portfolio_config;
//! use mfm_portfolio_model::symbol::QuoteCode;
//!
//! let raw = serde_json::json!({
//!     "portfolio_id": "portfolio_main",
//!     "quote_codes": ["USD"],
//!     "networks": [
//!         {
//!             "network_id": "ethereum-mainnet",
//!             "family": "evm",
//!             "chain_id": 1,
//!             "control_scope": "shared",
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
//! # Ok::<(), mfm_portfolio_model::portfolio::PortfolioConfigError>(())
//! ```
/// Base semantic runtime adapters shared by portfolio execution flows.
pub mod dispatch_adapters;
/// Fixed semantic runtime state family for compiled portfolio execution.
pub mod execution_states;
