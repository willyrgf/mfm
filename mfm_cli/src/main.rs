use clap::Parser;
use mfm_core::blockchain::adapter::LocalWallet;
use mfm_core::blockchain::adapter::Provider;
use std::sync::Arc;

use mfm_core::blockchain::cow_swap::CowSwapProvider;
use mfm_core::blockchain::uniswap_v3::UniswapV3Provider;
use mfm_core::blockchain::DexProvider;
use mfm_core::blockchain::{evm::ChainConfig, EvmProvider};
use mfm_core::cli::{Cli, CliContext, Commands};
use mfm_core::config::authentication::encryption::Encryption;
use mfm_core::config::Config;
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

    // Parse CLI arguments
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
        Commands::Rebalance {
            config: config_path,
        } => {
            let config = Config::load(&config_path)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::connect(&config.network.rpc_url).await?;
            let wallet_signer = LocalWallet::from_private_key(wallet.get_private_key())?
                .with_chain_id(config.network.chain_id);

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    rpc_urls: config.network.rpc_urls.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                    block_confirmations: 1,
                    gas_multiplier: 1.2,
                    gas_limit: Some(2000000),
                    gas_price: None,
                    max_fee_per_gas: None,
                    retry_attempts: 3,
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = create_dex_provider(&config, &provider, &wallet_signer)?;

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            context.handle_rebalance().await?;
        }
        Commands::Swap {
            config: config_path,
            from_token,
            to_token,
            amount,
            exact_approval,
        } => {
            let config = Config::load(&config_path)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::connect(&config.network.rpc_url).await?;
            let wallet_signer = LocalWallet::from_private_key(wallet.get_private_key())?
                .with_chain_id(config.network.chain_id);

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    rpc_urls: config.network.rpc_urls.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                    block_confirmations: 1,
                    gas_multiplier: 1.2,
                    gas_limit: Some(2000000),
                    gas_price: None,
                    max_fee_per_gas: None,
                    retry_attempts: 3,
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = create_dex_provider(&config, &provider, &wallet_signer)?;

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            context
                .handle_swap(&from_token, &to_token, &amount, exact_approval)
                .await?;
        }
        Commands::Status {
            config: config_path,
        } => {
            let config = Config::load(&config_path)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::connect(&config.network.rpc_url).await?;
            let wallet_signer = LocalWallet::from_private_key(wallet.get_private_key())?
                .with_chain_id(config.network.chain_id);

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    rpc_urls: config.network.rpc_urls.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                    block_confirmations: 1,
                    gas_multiplier: 1.2,
                    gas_limit: Some(2000000),
                    gas_price: None,
                    max_fee_per_gas: None,
                    retry_attempts: 3,
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = create_dex_provider(&config, &provider, &wallet_signer)?;

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            // Check balances and display status
            context.check_balances().await?;
            context.handle_status()?;
        }
        Commands::Resume {
            config: config_path,
        } => {
            let config = Config::load(&config_path)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::connect(&config.network.rpc_url).await?;
            let wallet_signer = LocalWallet::from_private_key(wallet.get_private_key())?
                .with_chain_id(config.network.chain_id);

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                    rpc_urls: config.network.rpc_urls.clone(),
                    block_confirmations: 1,
                    gas_multiplier: 1.2,
                    gas_limit: Some(2000000),
                    gas_price: None,
                    max_fee_per_gas: None,
                    retry_attempts: 3,
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = create_dex_provider(&config, &provider, &wallet_signer)?;

            // Create CLI context
            let mut context = CliContext::new(blockchain_provider, dex_provider, &config);

            context.handle_resume()?;
        }
        Commands::AaveHealth {
            config: config_path,
            wallet_address,
        } => {
            let config = Config::load(&config_path)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::connect(&config.network.rpc_url).await?;
            let wallet_signer = LocalWallet::from_private_key(wallet.get_private_key())?
                .with_chain_id(config.network.chain_id);

            let blockchain_provider = Box::new(EvmProvider::new(
                ChainConfig {
                    rpc_url: config.network.rpc_url.clone(),
                    rpc_urls: config.network.rpc_urls.clone(),
                    chain_id: config.network.chain_id,
                    name: config.network.name.clone(),
                    block_confirmations: 1,
                    gas_multiplier: 1.2,
                    gas_limit: Some(2000000),
                    gas_price: None,
                    max_fee_per_gas: None,
                    retry_attempts: 3,
                },
                wallet.get_private_key().to_string(),
            )?);

            let dex_provider = create_dex_provider(&config, &provider, &wallet_signer)?;

            // Create CLI context
            let context = CliContext::new(blockchain_provider, dex_provider, &config);

            // Handle AAVE health check
            context.handle_aave_health(wallet_address).await?;
        }
    }

    Ok(())
}

// Helper function to create the appropriate DEX provider based on configuration
fn create_dex_provider(
    config: &Config,
    provider: &Provider,
    wallet_signer: &LocalWallet,
) -> Result<Box<dyn DexProvider>, Box<dyn std::error::Error>> {
    match config.dex.provider.as_str() {
        "uniswap_v3" => {
            // Get the DEX configuration
            let _dex_config = config
                .dexes
                .get("uniswap_v3")
                .ok_or("Uniswap V3 configuration not found")?;

            // Create the provider with default parameters
            Ok(Box::new(UniswapV3Provider::new(
                Arc::new(provider.clone()),
                config.network.chain_id,
                Some(wallet_signer.clone()),
                None, // default weth
                None, // default factory
                None, // default router
                None, // default quoter
            )))
        }
        "cow_swap" => {
            // Get the DEX configuration
            let _dex_config = config
                .dexes
                .get("cow_swap")
                .ok_or("CowSwap configuration not found")?;

            // Create the settlement contract address
            let settlement_address = match &config.dex.cowswap {
                Some(cowswap_config) => cowswap_config.settlement_contract,
                None => return Err("CowSwap configuration not found in dex config".into()),
            };

            Ok(Box::new(CowSwapProvider::new(
                Arc::new(provider.clone()),
                Some(wallet_signer.clone()),
                settlement_address,
                config.network.chain_id,
            )))
        }
        _ => {
            // Default to Uniswap V3
            Ok(Box::new(UniswapV3Provider::new(
                Arc::new(provider.clone()),
                config.network.chain_id,
                Some(wallet_signer.clone()),
                None, // default weth
                None, // default factory
                None, // default router
                None, // default quoter
            )))
        }
    }
}
