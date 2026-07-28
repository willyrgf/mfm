#![warn(missing_docs)]
//! Unregistered Bitcoin Core transport qualification primitives.
//!
//! This crate intentionally exports no MFM capability, state, runner, replay helper, or catalog
//! registration. Its public transport methods each perform exactly one application-protocol
//! operation.

pub mod transport;
