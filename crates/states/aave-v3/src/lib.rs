#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared Aave V3 portfolio-position runtime adapters.
//!
//! Stable Aave semantic config lives in `mfm-portfolio-model`. This crate owns the runtime
//! adapters that observe those positions through the generic portfolio execution pipeline.
//!
//! # Examples
//!
//! ```rust
//! use mfm_portfolio_model::aave::AAVE_V3_PROTOCOL_ID;
//!
//! assert_eq!(AAVE_V3_PROTOCOL_ID, "aave_v3");
//! ```
/// Shared Aave V3 portfolio-position runtime adapters.
pub mod portfolio;
