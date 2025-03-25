use anyhow::{Context, Result};
use ethers::{
    prelude::*,
    providers::{Http, Provider},
    abi::{Token, Tokenizable, InvalidOutputType},
    types::U256,
};
use mfm_core::config::{authentication::Method, network::{Kind, Network}, Config};
use serde::Deserialize;
use std::sync::Arc;

const AAVE_V3_POOLS: &[(&str, &str)] = &[
    ("ethereum", "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2"),
    ("polygon", "0x794a61358D6845594F94dc1DB02A252b5b4814aD"),
    ("arbitrum", "0x794a61358D6845594F94dc1DB02A252b5b4814aD"),
];

#[derive(Debug, Deserialize)]
struct CoinGeckoPrice {
    usd: f64,
}

/// Check AAVE health factor for a wallet in a specific network
pub async fn check_health(config: &Config, network_name: &str) -> Result<()> {
    // Get network configuration
    let network = config
        .networks
        .get(network_name)
        .context(format!("Network {} not found in configuration", network_name))?;

    // Validate network is EVM compatible
    if network.kind != Kind::EVM {
        anyhow::bail!("AAVE health check is only supported on EVM networks");
    }

    // Get AAVE pool address for the network
    let pool_address = AAVE_V3_POOLS
        .iter()
        .find(|(name, _)| *name == network_name)
        .map(|(_, addr)| *addr)
        .context(format!("AAVE V3 pool not found for network {}", network_name))?;

    // Get wallet from configuration
    let wallet = get_wallet_from_config(config)?;

    // Create provider and connect to network
    let provider = create_provider(network)?;
    let client = Arc::new(provider);
    
    // Create wallet instance from private key
    let wallet_bytes = wallet.private_key.as_bytes();
    let wallet_address = H160::from_slice(&wallet_bytes[..20]);
    
    // Create AAVE V3 Pool contract instance
    let abi = include_str!("../../../contracts/aave_v3_pool.json");
    let abi = serde_json::from_str(abi).context("Failed to parse ABI")?;
    let pool = Contract::new(
        pool_address.parse::<Address>()?,
        abi,
        client,
    );
    
    // Get user account data
    let (
        total_collateral_eth,
        total_debt_eth,
        _available_borrows_eth,
        _current_liquidation_threshold,
        _ltv,
        health_factor,
    ): (U256, U256, U256, U256, U256, U256) = pool
        .method("getUserAccountData", wallet_address)?
        .call()
        .await?;

    let user_data = UserAccountData {
        total_collateral_eth,
        total_debt_eth,
        health_factor,
    };
    
    print_health_report(&user_data, network);
    
    Ok(())
}

fn get_wallet_from_config(config: &Config) -> Result<&mfm_core::config::authentication::wallet::Wallet> {
    // Find the first wallet authentication method
    config
        .auth_methods
        .get_methods()
        .iter()
        .find_map(|method| match method {
            Method::Wallet(wallet) => Some(wallet),
            _ => None,
        })
        .context("No wallet authentication method found in configuration")
}

fn create_provider(network: &Network) -> Result<Provider<Http>> {
    Provider::<Http>::try_from(&network.node_url)
        .or_else(|_| {
            network
                .node_url_failover
                .as_ref()
                .and_then(|url| Provider::<Http>::try_from(url).ok())
                .context("Failed to connect to both primary and failover nodes")
        })
}

fn print_health_report(user_data: &UserAccountData, network: &Network) {
    println!("AAVE Health Check Results for {}:", network.name);
    println!("----------------------------------------");
    println!("Total Collateral ({}): {:.4}", 
        network.symbol, 
        user_data.total_collateral_eth
    );
    println!("Total Debt ({}): {:.4}", 
        network.symbol, 
        user_data.total_debt_eth
    );
    println!("Health Factor: {:.2}", user_data.health_factor);
    
    if user_data.health_factor == U256::MAX {
        println!("No borrow value. The health factor is infinite, and there is no liquidation risk.");
    } else if user_data.health_factor > U256::from(1) {
        let max_decrease = (1.0 - (1.0 / user_data.health_factor.as_u128() as f64)) * 100.0;
        println!("The collateral assets can decrease by up to {:.2}% before the health factor drops to 1.", max_decrease);
    } else if user_data.health_factor == U256::from(1) {
        println!("The health factor is exactly 1. Any decrease in collateral value will put the position at risk of liquidation.");
    } else {
        println!("WARNING: The health factor is less than 1. The position is at risk of liquidation!");
    }
}

#[derive(Debug)]
pub struct UserAccountData {
    pub total_collateral_eth: U256,
    pub total_debt_eth: U256,
    pub health_factor: U256,
}

impl Tokenizable for UserAccountData {
    fn from_token(token: Token) -> std::result::Result<Self, InvalidOutputType> {
        if let Token::Tuple(tokens) = token {
            if tokens.len() >= 6 {
                let total_collateral_eth = U256::from_token(tokens[0].clone())
                    .map_err(|_| InvalidOutputType("Failed to parse total_collateral_eth".into()))?;
                let total_debt_eth = U256::from_token(tokens[1].clone())
                    .map_err(|_| InvalidOutputType("Failed to parse total_debt_eth".into()))?;
                let health_factor = U256::from_token(tokens[5].clone())
                    .map_err(|_| InvalidOutputType("Failed to parse health_factor".into()))?;
                
                Ok(UserAccountData {
                    total_collateral_eth,
                    total_debt_eth,
                    health_factor,
                })
            } else {
                Err(InvalidOutputType("Not enough tuple elements".into()))
            }
        } else {
            Err(InvalidOutputType("Expected tuple".into()))
        }
    }

    fn into_token(self) -> Token {
        Token::Tuple(vec![
            self.total_collateral_eth.into_token(),
            self.total_debt_eth.into_token(),
            U256::zero().into_token(), // available_borrows_eth
            U256::zero().into_token(), // current_liquidation_threshold
            U256::zero().into_token(), // ltv
            self.health_factor.into_token(),
        ])
    }
}
