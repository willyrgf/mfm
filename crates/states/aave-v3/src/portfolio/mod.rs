//! Shared Aave V3 portfolio-position config and runtime states.
//!
//! The portfolio module owns the typed meaning of `protocol = "aave_v3"` for canonical
//! `protocol_position` symbols. It validates the config blob carried by the generic symbol model
//! and normalizes Aave reserve and debt token reads into the canonical `Observation` surface.
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_aave_v3::portfolio::model::{
//!     AaveDebtKind, AaveDebtPositionConfig, AaveMarketConfig, AaveReserveConfig,
//! };
//!
//! let cfg = AaveDebtPositionConfig {
//!     market: AaveMarketConfig {
//!         market_id: "aave-v3-mainnet".to_string(),
//!         network_id: "ethereum-mainnet".to_string(),
//!         chain_id: 1,
//!         pool_address: "0x0000000000000000000000000000000000000001".to_string(),
//!         reserves: vec![AaveReserveConfig {
//!             reserve_id: "usdc".to_string(),
//!             reserve_index: 0,
//!             underlying_token_address: "0x0000000000000000000000000000000000000002".to_string(),
//!             a_token_address: "0x0000000000000000000000000000000000000003".to_string(),
//!             variable_debt_token_address: Some(
//!                 "0x0000000000000000000000000000000000000004".to_string(),
//!             ),
//!             stable_debt_token_address: None,
//!             metadata: Default::default(),
//!         }],
//!         metadata: Default::default(),
//!     },
//!     reserve_id: "usdc".to_string(),
//!     debt_kind: AaveDebtKind::Variable,
//! };
//!
//! assert_eq!(cfg.market.market_id, "aave-v3-mainnet");
//! assert_eq!(cfg.debt_kind.as_str(), "variable");
//! ```

/// Typed Aave V3 config models and validation helpers for portfolio positions.
pub mod model;
/// Shared semantic payloads for compiled Aave V3 portfolio execution.
pub mod semantic;
/// Reusable Aave V3 portfolio-position runtime states.
pub mod states;
