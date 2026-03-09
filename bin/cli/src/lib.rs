//! Shared library surface for the `mfm` CLI.
//!
//! The CLI keeps transport concerns in this crate and delegates workflow execution to `mfm-app`
//! and the op/state-machine layers.
//!
//! # Examples
//!
//! ```rust
//! use mfm::presentation::output::ResponseStatus;
//!
//! assert!(matches!(ResponseStatus::Success, ResponseStatus::Success));
//! ```
use clap::Parser;
use mfm_app::observability::{init_observability, observability_from_env_with_default};

/// CLI command parsing and dispatch modules.
mod commands;
/// CLI output formatting helpers.
pub mod presentation;
/// Thin CLI adaptation helpers for app services and stores.
mod support;

const CLI_DEFAULT_LOG_FILTER: &str =
    "warn,mfm_machine=error,mfm_app=error,mfm_op_keystore_tx=error,tower_http=error";

/// Runs the CLI entrypoint with stable observability defaults.
pub async fn run() -> ! {
    let observability = observability_from_env_with_default("mfm_cli", CLI_DEFAULT_LOG_FILTER);

    if let Err(err) = init_observability(observability) {
        eprintln!("failed to initialize observability: {}", err.message);
        std::process::exit(1);
    }

    let cli = commands::Cli::parse();
    cli.execute().await;
}
