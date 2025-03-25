use clap::{Parser, Subcommand};
use mfm_core::config::Config;
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_subscriber;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Command to execute
    #[command(subcommand)]
    command: Commands,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: Option<Level>,

    /// Configuration file path
    #[arg(long, default_value = "config.toml")]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Check AAVE health factor for a wallet in a specific network
    AaveHealth {
        /// Network name (e.g., ethereum, polygon, arbitrum)
        #[arg(long)]
        network: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse command line arguments
    let cli = Cli::parse();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(cli.log_level.unwrap_or(Level::INFO))
        .init();

    info!("mfm starting...");

    // Load configuration
    let config_path = cli.config.unwrap_or_else(|| PathBuf::from("config.toml"));
    let config = Config::from_toml_file(&config_path)?;

    // Execute subcommand
    match cli.command {
        Commands::AaveHealth { network } => {
            crate::commands::aave::check_health(&config, &network).await?;
        }
    }

    Ok(())
}

mod commands;
