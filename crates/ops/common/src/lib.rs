//! Shared building blocks for operation crates.
//!
//! This crate sits between the machine/runtime layer and domain operation crates:
//! reusable states + common helpers live here.

pub mod ctx;
pub mod errors;
pub mod states;
