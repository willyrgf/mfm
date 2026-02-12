use clap::Parser;

mod commands;
mod presentation;
mod support;

use commands::Cli;

#[tokio::main]
async fn main() -> ! {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();
    cli.execute().await;
}
