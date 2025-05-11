//! cli module for mfm_cli

pub mod context;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
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
    /// Manage BLS12-381 keystores (ERC-2335)
    Keystore {
        #[command(subcommand)]
        command: KeystoreCommands,
    },
}

// helper function to determine the default keystore directory
fn default_keystore_dir() -> PathBuf {
    match dirs::home_dir() {
        Some(mut path) => {
            path.push(".local");
            path.push("mfm");
            path
        }
        None => {
            // this case is unlikely on most systems.
            // panicking is one option, or using a conventional fallback.
            // for now, let's panic as it indicates a significant environment issue.
            // TODO: AI: consider a more graceful fallback or error propagation if home_dir is None.
            panic!("failed to determine home directory for default keystore path. please specify --keystore-path manually.");
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum KeystoreCommands {
    /// Import a private key into the keystore
    Import {
        /// Path to the keystore directory.
        /// Defaults to $HOME/.local/mfm/
        #[arg(long, value_parser = clap::value_parser!(PathBuf), default_value_os_t = default_keystore_dir())]
        keystore_path: PathBuf,
        /// Private key to import (hex encoded)
        #[arg(long)]
        private_key: String,
        /// Password to encrypt the key in the keystore
        #[arg(long)]
        password: String,
    },
    /// List all public keys in the keystore
    List {
        /// Path to the keystore directory.
        /// Defaults to $HOME/.local/mfm/
        #[arg(long, value_parser = clap::value_parser!(PathBuf), default_value_os_t = default_keystore_dir())]
        keystore_path: PathBuf,
    },
    /// Delete a key from the keystore
    Delete {
        /// Path to the keystore directory.
        /// Defaults to $HOME/.local/mfm/
        #[arg(long, value_parser = clap::value_parser!(PathBuf), default_value_os_t = default_keystore_dir())]
        keystore_path: PathBuf,
        /// Public key to delete (hex encoded)
        #[arg(long)]
        pubkey: String,
        /// Password to decrypt the keystore (if required for deletion, e.g. to verify)
        #[arg(long)]
        password: Option<String>,
    },
}

impl Commands {
    pub fn get_config_path(&self) -> Option<&str> {
        // AI: changed to option for keystore
        match self {
            Commands::Rebalance { config } => Some(config),
            Commands::Swap { config, .. } => Some(config),
            Commands::Status { config } => Some(config),
            Commands::Resume { config } => Some(config),
            Commands::AaveHealth { config, .. } => Some(config),
            Commands::Encrypt {
                private_key_path, ..
            } => Some(private_key_path),
            Commands::TokenApprove { config, .. } => Some(config),
            Commands::Keystore { .. } => None, // keystore commands use keystore_path directly
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
