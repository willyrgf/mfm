//! Example usage of mfm_core library
//!
//! This file shows how to use the core functionality programmatically
//! without the CLI interface.

use mfm_core::blockchain::adapter::Provider;
use mfm_core::blockchain::{evm::ChainConfig, EvmProvider};
use mfm_core::config::Config;
use mfm_core::portfolio::{Portfolio, PortfolioOperation};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("This is an example of using mfm_core library programmatically");
    println!("For a full application, please use the mfm_cli crate");

    // This code is just a demonstration and won't be executed
    if false {
        // Example of loading configuration
        let config_path = "config.yml";
        let config = Config::load(config_path)?;

        // Connect to blockchain
        let provider = Provider::connect(&config.network.rpc_url).await?;

        // Load wallet
        let wallet = config.load_wallet(Some("your_secure_password"))?;

        // Create blockchain provider
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

        // Create DEX provider
        let dex_provider = Box::new(mfm_core::blockchain::uniswap_v3::UniswapV3Provider::new(
            Arc::new(provider.clone()),
            config.network.chain_id,
            None,
            None, // Use default WETH address
            None, // Use default factory address
            None, // Use default router address
            None, // Use default quoter address
        ));

        // Create portfolio
        let mut portfolio = Portfolio::new(blockchain_provider, dex_provider, &config);

        // Start rebalance operation
        portfolio
            .start_operation(PortfolioOperation::Rebalance)
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // encryption tests are in config/authentication/encryption.rs
}
