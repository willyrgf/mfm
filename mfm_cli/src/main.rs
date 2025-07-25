use clap::Parser;

mod cli;

use cli::Cli;

#[tokio::main]
async fn main() -> ! {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();
    cli.execute().await;
}
