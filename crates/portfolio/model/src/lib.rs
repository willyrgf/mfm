#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Canonical portfolio, symbol, and wallet models for MFM portfolio snapshots.
//!
//! This crate owns pure schema and validation helpers. Runtime dispatch, state execution, and live
//! IO adapters live outside this crate.
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
//!     "wallets": [],
//!     "symbol_configs": [],
//!     "metadata": {}
//! });
//!
//! let cfg = decode_portfolio_config(&raw)?;
//! assert_eq!(cfg.quote_codes, vec![QuoteCode::Usd]);
//! # Ok::<(), mfm_portfolio_model::portfolio::PortfolioConfigError>(())
//! ```

/// Aave V3 portfolio-position config models and validation helpers.
pub mod aave;
/// Stable portfolio domain keys used by typed fanout/fanin planning.
pub mod domain_key;
/// Strong portfolio scalar identifiers used by config authorities.
pub mod ids;
/// Redaction-safe public metadata model used by portfolio surfaces.
pub mod metadata;
/// Canonical portfolio-domain models, validation helpers, and normalization utilities.
pub mod portfolio;
/// Canonical symbol-domain models, validation helpers, and normalization utilities.
pub mod symbol;
/// Canonical wallet-domain models, validation helpers, and normalization utilities.
pub mod wallet;
