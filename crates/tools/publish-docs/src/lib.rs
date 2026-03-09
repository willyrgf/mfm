#![allow(clippy::disallowed_methods)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Phase-1 publish planner and executor for the docs.rs release wave.
//!
//! This crate is intentionally scoped to the current `publish-wave.json` workflow.
//! It replaces the inline shell implementation behind `nix run .#publish-docs`
//! with typed Rust logic, exact remote observation, and sanitized run artifacts.

/// Orchestration entrypoints used by the binary.
pub mod app;
/// Apply-phase execution helpers.
pub mod apply;
/// Sanitized per-run artifact emission.
pub mod artifacts;
/// Wave catalog loading and package selection.
pub mod catalog;
/// CLI parsing and presentation helpers.
pub mod cli;
/// Shared error types.
pub mod error;
/// Release provenance ledger helpers.
pub mod ledger;
/// Shared internal data model.
pub mod model;
/// Planning and action classification.
pub mod plan;
/// Remote observation adapters.
pub mod remote;
/// Umbrella README generation.
pub mod umbrella;
/// Local workspace discovery.
pub mod workspace;
