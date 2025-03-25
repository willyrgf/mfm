mod blockchain;
mod cli;
mod config;
mod portfolio;

use clap::Parser;
use cli::{Cli, CliContext};
use config::{Config, SecureWallet};
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

use blockchain::{BlockchainProvider, DexProvider};
use ethers::providers::{Http, Provider};
use portfolio::Portfolio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse CLI arguments
    let cli = Cli::parse();

    // Load configuration
    let config = Config::load(&cli.command.get_config_path())?;

    // Load wallet securely
    let wallet = config.load_wallet(Some("your_secure_password"))?;

    // Combine the primary RPC URL and additional RPC URLs
    let mut all_rpc_urls = Vec::new();
    if !config.network.rpc_url.is_empty() {
        all_rpc_urls.push(config.network.rpc_url.clone());
    }
    all_rpc_urls.extend(config.network.rpc_urls.clone());

    // If no RPC URLs are provided, add default ones
    if all_rpc_urls.is_empty() {
        all_rpc_urls = vec![
            "https://eth.meowrpc.com".to_string(),
            "https://eth.llamarpc.com".to_string(),
            "https://ethereum.publicnode.com".to_string(),
            "https://rpc.eth.gateway.fm".to_string(),
            "https://rpc.ankr.com/eth".to_string(),
        ];
    }

    // Use the first URL for the ethers Provider
    let provider_url = all_rpc_urls
        .first()
        .unwrap_or(&"https://eth.llamarpc.com".to_string())
        .clone();
    let provider = Provider::new(Http::new(Url::parse(&provider_url)?));

    let blockchain_provider = Arc::new(Mutex::new(Box::new(blockchain::EvmProvider::new(
        blockchain::ChainConfig {
            rpc_url: config.network.rpc_url.clone(),
            rpc_urls: all_rpc_urls.clone(),
            chain_id: config.network.chain_id,
            name: config.network.name.clone(),
        },
        wallet.get_private_key().to_string(),
    )?)));

    let dex_provider = Arc::new(Mutex::new(Box::new(blockchain::UniswapV3Provider::new(
        Arc::new(provider.clone()),
        config.network.chain_id,
        None,
    ))));

    let portfolio = Portfolio::new(
        Box::new(blockchain::EvmProvider::new(
            blockchain::ChainConfig {
                rpc_url: config.network.rpc_url.clone(),
                rpc_urls: all_rpc_urls.clone(),
                chain_id: config.network.chain_id,
                name: config.network.name.clone(),
            },
            wallet.get_private_key().to_string(),
        )?),
        Box::new(blockchain::UniswapV3Provider::new(
            Arc::new(provider.clone()),
            config.network.chain_id,
            None,
        )),
        &config,
    );

    // Create CLI context
    let mut context = CliContext::new(
        blockchain_provider.lock().await.clone(),
        dex_provider.lock().await.clone(),
        &config,
    );

    // Handle commands
    match cli.command {
        cli::Commands::Rebalance { config: _ } => {
            context.handle_rebalance().await?;
        }
        cli::Commands::Swap {
            config: _,
            from_token,
            to_token,
            amount,
            exact_approval,
        } => {
            context
                .handle_swap(&from_token, &to_token, &amount, exact_approval)
                .await?;
        }
        cli::Commands::Status { config: _ } => {
            context.handle_status()?;
        }
        cli::Commands::Resume { config: _ } => {
            context.handle_resume()?;
        }
        cli::Commands::Encrypt { .. } => {
            // This command is handled directly in the CLI, not in the main function
            unreachable!("Encrypt command should be handled in the CLI");
        }
        cli::Commands::AaveHealth {
            config: _,
            wallet_address,
        } => {
            context.handle_aave_health(wallet_address).await?;
        }
    }

    Ok(())
}
