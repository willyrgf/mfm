#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Shared Aave V3 portfolio-position types and runtime adapters.
//!
//! This crate owns the typed meaning of `protocol = "aave_v3"` for canonical portfolio symbols
//! and the runtime adapters that observe those positions through the generic portfolio execution
//! pipeline.
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_aave_v3::portfolio::model::AAVE_V3_PROTOCOL_ID;
//!
//! assert_eq!(AAVE_V3_PROTOCOL_ID, "aave_v3");
//! ```
/// Shared Aave V3 portfolio-position config models and runtime states.
pub mod portfolio;
