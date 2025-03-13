mod telemetry;

pub const APP_NAME: &str = "mfm";
pub const DEFAULT_LOG_LEVEL: &str = "info";

use clap::Parser;
use ethers::{
    providers::{Http, Provider},
    signers::LocalWallet,
};
use mfm_core::{
    blockchain::{
        dex::{CowSwapProvider, UniswapV3Provider},
        evm::{ChainConfig, EvmProvider},
    },
    config::{authentication::encryption::Encryption, Config},
    Cli, CliContext, Commands,
};
use std::sync::{Arc, Mutex};
use url::Url;

use crate::telemetry::{get_subscriber, init_subscriber};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let subscriber = get_subscriber(APP_NAME.into(), DEFAULT_LOG_LEVEL.into(), std::io::stdout);
    init_subscriber(subscriber);

    let cli = Cli::parse();

    match cli.command {
        Commands::Encrypt {
            private_key_path,
            password,
            force,
        } => {
            let private_key = std::fs::read_to_string(&private_key_path)?;
            let private_key = private_key.trim();

            // Validate private key format (64 hex characters)
            if !private_key.chars().all(|c| c.is_ascii_hexdigit()) || private_key.len() != 64 {
                return Err(
                    "Invalid private key format. Must be 64 hexadecimal characters.".into(),
                );
            }

            let mut encryption = Encryption::new(&password);
            let encrypted = encryption.encrypt(private_key)?;

            if force || !std::path::Path::new(&private_key_path).exists() {
                std::fs::write(&private_key_path, encrypted)?;
                println!("Private key encrypted and saved to {}", private_key_path);
            } else {
                println!("File already exists. Use --force to overwrite.");
            }
        }
        Commands::Rebalance { config } => {
            // Load configuration
            let config = Config::load(&config)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = Box::new(UniswapV3Provider::new(provider.clone()));

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            context.handle_rebalance().await?;
        }
        Commands::Swap {
            config,
            from_token,
            to_token,
            amount,
        } => {
            // Load configuration
            let config = Config::load(&config)?;

            // Get token addresses from config
            let from_token_config = config
                .tokens
                .get(&from_token)
                .ok_or_else(|| format!("Token {} not found in config", from_token))?;
            let from_token_network = from_token_config
                .networks
                .get(&config.network.name)
                .ok_or_else(|| {
                    format!(
                        "Token {} not configured for network {}",
                        from_token, config.network.name
                    )
                })?;

            let to_token_config = config
                .tokens
                .get(&to_token)
                .ok_or_else(|| format!("Token {} not found in config", to_token))?;
            let to_token_network = to_token_config
                .networks
                .get(&config.network.name)
                .ok_or_else(|| {
                    format!(
                        "Token {} not configured for network {}",
                        to_token, config.network.name
                    )
                })?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));
            let wallet_signer = LocalWallet::from_str(wallet.get_private_key())?
                .with_chain_id(config.network.chain_id);

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = Box::new(
                CowSwapProvider::new(provider.clone())
                    .with_chain_id(config.network.chain_id)
                    .with_signer(wallet_signer),
            );

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            // Convert amount to wei using the token's decimals
            let amount_float: f64 = amount.parse()?;
            let decimals = from_token_network.decimals.unwrap_or(18);
            let amount_wei = format!("{}", (amount_float * 10f64.powi(decimals as i32)) as u64);

            context
                .handle_swap(
                    &from_token_network.address,
                    &to_token_network.address,
                    &amount_wei,
                )
                .await?;
        }
        Commands::Status { config } => {
            // Load configuration
            let config = Config::load(&config)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = Box::new(UniswapV3Provider::new(provider.clone()));

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            // Check balances and display status
            context.check_balances().await?;
            context.handle_status()?;
        }
        Commands::Resume { config } => {
            // Load configuration
            let config = Config::load(&config)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = Box::new(UniswapV3Provider::new(provider.clone()));

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            context.handle_resume()?;
        }
    }

    Ok(())
}
