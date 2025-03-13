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

    let provider = Provider::new(Http::new(Url::parse(&config.network.rpc_url)?));

    let blockchain_provider = Arc::new(Mutex::new(Box::new(blockchain::EvmProvider::new(
        blockchain::ChainConfig {
            rpc_url: config.network.rpc_url.clone(),
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
    }

    Ok(())
}
