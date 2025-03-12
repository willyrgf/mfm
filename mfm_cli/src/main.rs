use clap::Parser;
use ethers::providers::{Http, Provider};
use mfm::{
    telemetry::{get_subscriber, init_subscriber},
    ExitCode, APP_NAME, DEFAULT_LOG_LEVEL,
};
use mfm_core::{
    cli::{Cli, CliContext, Commands},
    config::{authentication::encryption::Encryption, Config},
};
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

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

            let blockchain_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::EvmProvider::new(
                    mfm_core::blockchain::ChainConfig {
                        rpc_url: config.network.rpc_url.clone(),
                        chain_id: config.network.chain_id,
                        name: config.network.name.clone(),
                    },
                    wallet.get_private_key(),
                )?,
            )));

            let dex_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::UniswapV3Provider::new(provider.clone()),
            )));

            // Create CLI context
            let mut context = CliContext::new(
                blockchain_provider.lock().await.clone(),
                dex_provider.lock().await.clone(),
                &config,
            );

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

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));

            let blockchain_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::EvmProvider::new(
                    mfm_core::blockchain::ChainConfig {
                        rpc_url: config.network.rpc_url.clone(),
                        chain_id: config.network.chain_id,
                        name: config.network.name.clone(),
                    },
                    wallet.get_private_key(),
                )?,
            )));

            let dex_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::UniswapV3Provider::new(provider.clone()),
            )));

            // Create CLI context
            let mut context = CliContext::new(
                blockchain_provider.lock().await.clone(),
                dex_provider.lock().await.clone(),
                &config,
            );

            context.handle_swap(&from_token, &to_token, &amount).await?;
        }
        Commands::Status { config } => {
            // Load configuration
            let config = Config::load(&config)?;

            // Load wallet securely
            let wallet = config.load_wallet(Some("your_secure_password"))?;

            let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));

            let blockchain_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::EvmProvider::new(
                    mfm_core::blockchain::ChainConfig {
                        rpc_url: config.network.rpc_url.clone(),
                        chain_id: config.network.chain_id,
                        name: config.network.name.clone(),
                    },
                    wallet.get_private_key(),
                )?,
            )));

            let dex_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::UniswapV3Provider::new(provider.clone()),
            )));

            // Create CLI context
            let mut context = CliContext::new(
                blockchain_provider.lock().await.clone(),
                dex_provider.lock().await.clone(),
                &config,
            );

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

            let blockchain_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::EvmProvider::new(
                    mfm_core::blockchain::ChainConfig {
                        rpc_url: config.network.rpc_url.clone(),
                        chain_id: config.network.chain_id,
                        name: config.network.name.clone(),
                    },
                    wallet.get_private_key(),
                )?,
            )));

            let dex_provider = Arc::new(Mutex::new(Box::new(
                mfm_core::blockchain::UniswapV3Provider::new(provider.clone()),
            )));

            // Create CLI context
            let mut context = CliContext::new(
                blockchain_provider.lock().await.clone(),
                dex_provider.lock().await.clone(),
                &config,
            );

            context.handle_resume()?;
        }
    }

    Ok(())
}
