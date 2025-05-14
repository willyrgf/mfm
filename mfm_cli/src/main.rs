use clap::Parser;
use mfm_core::config::Config;
use std::sync::Arc;
use tracing::info;

// Constants
const APP_NAME: &str = "mfm";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("{} starting...", APP_NAME);

    Ok(())
}
