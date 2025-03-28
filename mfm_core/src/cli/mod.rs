//! cli module for mfm

use crate::blockchain::adapter::types::{Address, U256};
use crate::blockchain::BlockchainProvider;
use crate::blockchain::DexProvider;
use crate::portfolio::{Portfolio, PortfolioOperation, PortfolioStatus};
use clap::{Parser, Subcommand};
use std::str::FromStr;
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
}

impl Commands {
    pub fn get_config_path(&self) -> &str {
        match self {
            Commands::Rebalance { config } => config,
            Commands::Swap { config, .. } => config,
            Commands::Status { config } => config,
            Commands::Resume { config } => config,
            Commands::AaveHealth { config, .. } => config,
            Commands::Encrypt {
                private_key_path, ..
            } => private_key_path,
        }
    }
}

pub struct CliContext {
    portfolio: Portfolio,
}

impl CliContext {
    pub fn new(
        blockchain_provider: Box<dyn BlockchainProvider>,
        dex_provider: Box<dyn DexProvider>,
        config: &crate::config::Config,
    ) -> Self {
        Self {
            portfolio: Portfolio::new(blockchain_provider, dex_provider, config),
        }
    }

    #[allow(dead_code)]
    pub async fn check_balances(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.portfolio.status = PortfolioStatus::CheckingBalances;
        self.portfolio.check_balances().await?;
        Ok(())
    }

    pub async fn handle_rebalance(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.portfolio
            .start_operation(PortfolioOperation::Rebalance)
            .await?;

        while matches!(self.portfolio.status, PortfolioStatus::Completed) {
            match self.portfolio.status {
                PortfolioStatus::CheckingBalances => {
                    self.portfolio.check_balances().await?;
                }
                PortfolioStatus::CalculatingQuotes => {
                    self.portfolio.calculate_quotes().await?;
                }
                PortfolioStatus::ExecutingSwaps => {
                    self.portfolio.execute_swaps().await?;
                }
                PortfolioStatus::Error(ref error) => {
                    return Err(error.clone().into());
                }
                _ => break,
            }
        }

        Ok(())
    }

    pub async fn handle_swap(
        &mut self,
        from_token: &str,
        to_token: &str,
        amount: &str,
        exact_approval: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Get token addresses from config
        let from_token_config = self
            .portfolio
            .config
            .tokens
            .get(from_token)
            .ok_or_else(|| format!("Token {} not found in config", from_token))?;
        let from_token_network = from_token_config
            .networks
            .get(&self.portfolio.config.network.name)
            .ok_or_else(|| {
                format!(
                    "Token {} not configured for network {}",
                    from_token, self.portfolio.config.network.name
                )
            })?;

        let to_token_config = self
            .portfolio
            .config
            .tokens
            .get(to_token)
            .ok_or_else(|| format!("Token {} not found in config", to_token))?;
        let to_token_network = to_token_config
            .networks
            .get(&self.portfolio.config.network.name)
            .ok_or_else(|| {
                format!(
                    "Token {} not configured for network {}",
                    to_token, self.portfolio.config.network.name
                )
            })?;

        // Parse the addresses using our adapter Address type
        let from_addr =
            Address::from_str(&from_token_network.address).unwrap_or_else(|_| Address::zero());
        let to_addr =
            Address::from_str(&to_token_network.address).unwrap_or_else(|_| Address::zero());

        // Convert decimal amount to wei using the token's decimals
        let amount_float: f64 = amount.parse()?;
        let decimals = from_token_network.decimals.unwrap_or(18);

        // Using alloy_primitives::U256 to create our adapter U256
        let amount_wei_raw = (amount_float * 10f64.powi(decimals as i32)) as u64;
        let amount_wei = U256::from(alloy_primitives::U256::from(amount_wei_raw));

        eprintln!(
            "Converting {} tokens to wei with {} decimals: {}",
            amount_float, decimals, amount_wei.0
        );

        self.portfolio
            .start_operation(PortfolioOperation::Swap {
                from_token: from_addr,
                to_token: to_addr,
                amount: amount_wei,
                exact_approval,
            })
            .await?;

        while !matches!(self.portfolio.status, PortfolioStatus::Completed) {
            match self.portfolio.status {
                PortfolioStatus::CheckingBalances => {
                    self.portfolio.check_balances().await?;
                }
                PortfolioStatus::CalculatingQuotes => {
                    self.portfolio.calculate_quotes().await?;
                }
                PortfolioStatus::ExecutingSwaps => {
                    self.portfolio.execute_swaps().await?;
                }
                PortfolioStatus::Error(ref error) => {
                    return Err(error.clone().into());
                }
                _ => break,
            }
        }

        Ok(())
    }

    pub async fn handle_aave_health(
        &self,
        wallet_address: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::blockchain::adapter::types::Address;
        use std::str::FromStr;

        println!("Checking AAVE health factor...");

        // Get the Aave provider from the blockchain provider
        let aave_provider = self
            .portfolio
            .blockchain_provider
            .get_aave_provider()
            .await?;

        // Get wallet address (from parameter or config)
        let wallet_addr = match wallet_address {
            Some(addr) => {
                Address::from_str(&addr).map_err(|_| "Invalid wallet address format".to_string())?
            }
            None => self.portfolio.blockchain_provider.get_wallet_address(),
        };

        // Calculate health factor using the adapter
        let health_result = aave_provider.calculate_health_factor(wallet_addr).await?;

        // Print with formatting
        println!("\n=== AAVE Health Check Results ===");
        println!(
            "Total Collateral (USD): {:.2}",
            health_result.total_collateral_usd
        );
        println!("Total Debt (USD): {:.2}", health_result.total_debt_usd);
        println!("Health Factor: {:.4}", health_result.health_factor);
        println!(
            "Max Collateral Decrease: {:.2}%",
            health_result.max_decrease_percentage
        );

        if health_result.user_reserves.is_empty() {
            println!("\nNo reserves found or not fully implemented yet.");
        } else {
            println!("\nReserves:");
            for reserve in health_result.user_reserves {
                println!(
                    "{}: Balance: {}, Variable Debt: {}, USD Value: ${:.2}",
                    reserve.symbol,
                    reserve.current_atoken_balance,
                    reserve.current_variable_debt,
                    reserve.price_usd
                );
            }
        }

        Ok(())
    }

    pub fn handle_status(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!(
            "\nWallet Address: {}",
            self.portfolio.blockchain_provider.get_wallet_address()
        );
        println!("\nCurrent Portfolio Status:");
        println!("Operation: {:?}", self.portfolio.operation);
        println!("Status: {:?}", self.portfolio.status);
        println!("Last Operation: {:?}", self.portfolio.state.last_operation);
        println!("Last Error: {:?}", self.portfolio.state.last_error);
        println!("Total Value: {}", self.portfolio.state.total_value);
        println!("\nBalances:");
        for balance in &self.portfolio.state.balances {
            println!(
                "{} ({}): {} ({:.2}% of portfolio)",
                balance.token_name, balance.token, balance.balance, balance.current_percentage
            );
        }
        Ok(())
    }

    pub fn handle_resume(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.portfolio.resume()?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum CliError {
    // ... rest of the file ...
}
