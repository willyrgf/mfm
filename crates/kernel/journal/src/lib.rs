#![warn(missing_docs)]
//! Canonical five-family append-only history values for structured runs.
//!
//! Store atomicity, successor validation, folding, scheduling, replay,
//! callbacks, and ambient access are deliberately outside this crate.

pub mod structured;
