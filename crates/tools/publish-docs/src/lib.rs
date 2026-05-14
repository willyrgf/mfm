#![allow(clippy::disallowed_methods)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Phase-1 publish planner and executor for the docs.rs release wave.
//!
//! This crate is intentionally scoped to the current publish-wave workflow.
//! It replaces the inline shell implementation behind `nix run .#publish-docs`
//! with typed Rust logic, exact remote observation, and sanitized run artifacts.

use clap::Parser;

/// Orchestration entrypoints used by the binary.
mod app;
/// Apply-phase execution helpers.
mod apply;
/// Sanitized per-run artifact emission.
mod artifacts;
/// Wave catalog loading and package selection.
mod catalog;
/// CLI parsing and presentation helpers.
mod cli;
/// Authored publish-wave and desired-catalog config parsing.
mod config;
/// Shared error types.
mod error;
/// Release provenance ledger helpers.
mod ledger;
/// Shared internal data model.
mod model;
/// Planning and action classification.
mod plan;
/// Remote observation adapters.
mod remote;
/// Umbrella README generation.
mod umbrella;
/// Local workspace discovery.
mod workspace;

/// Runs the publish-docs CLI entrypoint.
pub async fn run() -> std::process::ExitCode {
    app::run(cli::Cli::parse()).await
}
