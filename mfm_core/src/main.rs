use std::{fs::File, io::Read};

pub mod blockchain;
pub mod cli;
pub mod config;
pub mod portfolio;

pub use blockchain::{
    cow_swap::CowSwapProvider,
    dex::{DexError, DexProvider},
    evm::{BlockchainError, BlockchainProvider},
    EvmProvider,
};

pub mod contexts;
pub mod operations;
pub mod states;

use anyhow::Error;
use clap::Parser;
use config::Config;
use ethers::providers::{Http, Provider};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

pub use cli::{Cli, CliContext, Commands};
pub use portfolio::{Portfolio, PortfolioOperation, PortfolioState, PortfolioStatus, TokenBalance};
pub use states::*;

fn read_yaml<T: DeserializeOwned>(path: String) -> Result<T, Error> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let instance: T = serde_yaml::from_str(&contents)?;
    Ok(instance)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = Config::load(cli.command.get_config_path())?;

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

    let _portfolio = Portfolio::new(
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

#[cfg(test)]
mod tests {
    mod encryption_tests;
}
