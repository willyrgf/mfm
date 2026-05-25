//! Shared library surface for the `mfm` CLI.
//!
//! The CLI keeps transport concerns in this crate and delegates workflow execution to `mfm-app`
//! and certified typed workflow ports.
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

const CLI_DEFAULT_LOG_FILTER: &str = "warn,mfm_app=error,tower_http=error";

/// Runs the CLI entrypoint with stable observability defaults.
pub async fn run() -> ! {
    let observability = observability_from_env_with_default("mfm_cli", CLI_DEFAULT_LOG_FILTER);

    if let Err(err) = init_observability(observability) {
        eprintln!("failed to initialize observability: {}", err.message);
        std::process::exit(1);
    }

    let cli = match commands::Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let format = commands::detect_requested_output_format_for_parse_error(
                std::env::args_os(),
                std::env::var_os("MFM_OUTPUT_FORMAT").as_deref(),
            );
            presentation::output::handle_cli_parse_error(err, &format);
        }
    };
    cli.execute().await;
}
