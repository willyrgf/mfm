#![allow(clippy::disallowed_methods)]

use clap::Parser;

mod commands;
mod presentation;
mod support;

use commands::Cli;
use mfm_app::observability::{init_observability, observability_from_env_with_default};

const CLI_DEFAULT_LOG_FILTER: &str =
    "warn,mfm_machine=error,mfm_app=error,mfm_op_keystore_tx=error,tower_http=error";

#[tokio::main]
async fn main() -> ! {
    // Keep CLI stderr JSON contracts stable by default while preserving CLI-originated warnings
    // (for example insecure password-env usage).
    let observability = observability_from_env_with_default("mfm_cli", CLI_DEFAULT_LOG_FILTER);

    if let Err(err) = init_observability(observability) {
        eprintln!("failed to initialize observability: {}", err.message);
        std::process::exit(1);
    }

    let cli = Cli::parse();
    cli.execute().await;
}
