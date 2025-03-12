use crate::{
    blockchain::{BlockchainProvider, DexProvider},
    portfolio::{Portfolio, PortfolioOperation, PortfolioStatus},
};
use clap::{Parser, Subcommand};
use ethers::types::{Address, U256};
use std::str::FromStr;

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
}

impl Commands {
    pub fn get_config_path(&self) -> &str {
        match self {
            Commands::Rebalance { config } => config,
            Commands::Swap { config, .. } => config,
            Commands::Status { config } => config,
            Commands::Resume { config } => config,
            Commands::Encrypt { .. } => unreachable!(),
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        let from_addr = Address::from_str(from_token)?;
        let to_addr = Address::from_str(to_token)?;
        let amount_wei = U256::from_str(amount)?;

        self.portfolio
            .start_operation(PortfolioOperation::Swap {
                from_token: from_addr,
                to_token: to_addr,
                amount: amount_wei,
            })
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
