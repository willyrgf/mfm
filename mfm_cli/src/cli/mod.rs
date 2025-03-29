//! cli module for mfm_cli

pub mod context;

use clap::{Parser, Subcommand};
use thiserror::Error;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Encrypt a private key file
    Encrypt {
        /// Path to the private key file
        #[arg(short = 'k', long)]
        private_key_path: String,

        /// Password to encrypt the private key
        #[arg(short, long)]
        password: String,

        /// Whether to overwrite the existing file
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
    /// Rebalance the portfolio according to target allocations
    Rebalance {
        /// Path to the configuration file
        #[arg(short, long)]
        config: String,
    },
    /// Execute a single swap between tokens
    Swap {
        /// Path to the configuration file
        #[arg(short, long)]
        config: String,
        /// Address of the token to swap from
        #[arg(short, long)]
        from_token: String,
        /// Address of the token to swap to
        #[arg(short, long)]
        to_token: String,
        /// Amount to swap (in wei)
        #[arg(short, long)]
        amount: String,
        /// Auto-approve exact amount instead of max amount
        #[arg(long, default_value_t = false)]
        exact_approval: bool,
    },
    /// Check the current portfolio status
    Status {
        /// Path to the configuration file
        #[arg(short, long)]
        config: String,
    },
    /// Resume an interrupted operation
    Resume {
        /// Path to the configuration file
        #[arg(short, long)]
        config: String,
    },
    /// Check AAVE health factor for a wallet
    AaveHealth {
        /// Path to the configuration file
        #[arg(short, long)]
        config: String,
        /// Wallet address to check (defaults to wallet in config)
        #[arg(short, long)]
        wallet_address: Option<String>,
    },
    /// Approve a token for spending by a contract
    TokenApprove {
        /// Path to the configuration file
        #[arg(short, long)]
        config: String,
        /// Address of the token to approve
        #[arg(short, long)]
        token: String,
        /// Address of the spender contract
        #[arg(short, long)]
        spender: String,
        /// Amount to approve (in wei) or "max" for maximum
        #[arg(short, long)]
        amount: String,
        /// Auto-approve exact amount instead of max amount
        #[arg(long, default_value_t = false)]
        exact: bool,
    },
}

impl Commands {
    pub fn get_config_path(&self) -> &str {
        match self {
            Commands::Rebalance { config } => config,
            Commands::Swap { config, .. } => config,
            Commands::Status { config } => config,
            Commands::Resume { config } => config,
            Commands::AaveHealth { config, .. } => config,
            Commands::Encrypt {
                private_key_path, ..
            } => private_key_path,
            Commands::TokenApprove { config, .. } => config,
        }
    }
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Blockchain error: {0}")]
    BlockchainError(String),

    #[error("Portfolio error: {0}")]
    PortfolioError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
