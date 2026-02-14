use clap::Parser;

mod commands;
mod presentation;
mod support;

use commands::Cli;
use mfm_app::observability::{init_observability, observability_from_env};

#[tokio::main]
async fn main() -> ! {
    if let Err(err) = init_observability(observability_from_env("mfm_cli")) {
        eprintln!("failed to initialize observability: {}", err.message);
        std::process::exit(1);
    }

    let cli = Cli::parse();
    cli.execute().await;
}
