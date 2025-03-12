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
    let wallet = config.load_wallet()?;

    let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));

    let blockchain_provider = Arc::new(Mutex::new(Box::new(blockchain::EvmProvider::new(
        blockchain::ChainConfig {
            rpc_url: config.network.rpc_url.clone(),
            chain_id: config.network.chain_id,
            name: config.network.name.clone(),
        },
        wallet.get_private_key(),
    )?)));

    let dex_provider = Arc::new(Mutex::new(Box::new(blockchain::UniswapV3Provider::new(
        provider.clone(),
    ))));

    let portfolio = Portfolio::new(
        Box::new(blockchain::EvmProvider::new(
            blockchain::ChainConfig {
                rpc_url: config.network.rpc_url,
                chain_id: config.network.chain_id,
                name: config.network.name,
            },
            wallet.get_private_key(),
        )?),
        Box::new(blockchain::UniswapV3Provider::new(provider)),
    );

    // Create CLI context
    let mut context = CliContext::new(
        blockchain_provider.lock().await.clone(),
        dex_provider.lock().await.clone(),
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
        } => {
            context.handle_swap(&from_token, &to_token, &amount).await?;
        }
        cli::Commands::Status { config: _ } => {
            context.handle_status()?;
        }
        cli::Commands::Resume { config: _ } => {
            context.handle_resume()?;
        }
    }

    Ok(())
}
