//! cli context module for handling cli operations

use mfm_core::blockchain::adapter::types::U256;
use mfm_core::blockchain::{BlockchainProvider, DexProvider};
use mfm_core::config::Config;
use mfm_core::contexts::portfolio::PortfolioContext;
use mfm_core::portfolio::{PortfolioOperation, PortfolioStatus};

/// Handles the execution of CLI commands by wrapping the core portfolio operations
pub struct CliContext {
    context: PortfolioContext,
}

impl CliContext {
    /// Create a new CLI context with the given providers and configuration
    pub fn new(
        blockchain_provider: Box<dyn BlockchainProvider>,
        dex_provider: Box<dyn DexProvider>,
        config: &Config,
    ) -> Self {
        Self {
            context: PortfolioContext::new(blockchain_provider, dex_provider, config),
        }
    }

    /// Check the current balances in the portfolio
    pub async fn check_balances(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::operations::portfolio::CheckBalancesOperation;
        use mfm_core::operations::Operation;

        let check_op = CheckBalancesOperation {
            context: self.context.clone(),
        };

        check_op.execute().await.map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to check balances: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        println!("Balance check completed successfully");
        Ok(())
    }

    /// Handle the rebalance operation
    pub async fn handle_rebalance(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::operations::portfolio::{
            CalculateQuotesOperation, CheckBalancesOperation, ExecuteSwapsOperation,
            StartOperationOperation,
        };
        use mfm_core::operations::Operation;
        use mfm_core::portfolio::PortfolioStatus;

        // Create a rebalance operation
        let start_op = StartOperationOperation {
            context: self.context.clone(),
            operation: PortfolioOperation::Rebalance,
        };

        // Execute start operation
        start_op.execute().await.map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to start rebalance: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        // Execute the operations in sequence based on the current state
        while !matches!(self.context.get_status(), PortfolioStatus::Completed) {
            match self.context.get_status() {
                PortfolioStatus::CheckingBalances => {
                    let check_op = CheckBalancesOperation {
                        context: self.context.clone(),
                    };
                    check_op.execute().await.map_err(|e| {
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to check balances: {}", e),
                        )) as Box<dyn std::error::Error>
                    })?;
                }
                PortfolioStatus::CalculatingQuotes => {
                    let quotes_op = CalculateQuotesOperation {
                        context: self.context.clone(),
                    };
                    quotes_op.execute().await.map_err(|e| {
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to calculate quotes: {}", e),
                        )) as Box<dyn std::error::Error>
                    })?;
                }
                PortfolioStatus::ExecutingSwaps => {
                    let swaps_op = ExecuteSwapsOperation {
                        context: self.context.clone(),
                    };
                    swaps_op.execute().await.map_err(|e| {
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to execute swaps: {}", e),
                        )) as Box<dyn std::error::Error>
                    })?;
                }
                PortfolioStatus::Error(ref error) => {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Portfolio operation error: {}", error),
                    )) as Box<dyn std::error::Error>);
                }
                _ => break,
            }
        }

        println!("Rebalance operation completed successfully");
        Ok(())
    }

    /// Handle a swap operation between two tokens
    pub async fn handle_swap(
        &mut self,
        from_token: &str,
        to_token: &str,
        amount: &str,
        exact_approval: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::blockchain::adapter::types::{Address, U256};
        use mfm_core::operations::portfolio::{
            CalculateQuotesOperation, CheckBalancesOperation, ExecuteSwapsOperation,
            StartOperationOperation,
        };
        use mfm_core::operations::Operation;
        use mfm_core::portfolio::PortfolioOperation;
        use mfm_core::portfolio::PortfolioStatus;
        use std::str::FromStr;

        // Parse token addresses
        let from_addr = Address::from_str(from_token).map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid from_token address: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        let to_addr = Address::from_str(to_token).map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid to_token address: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        // Convert decimal amount to wei using the token's decimals
        let amount_float: f64 = amount.parse()?;
        let decimals = 18; // Default to 18 decimals if not specified

        // Using alloy_primitives::U256 to create our adapter U256
        let amount_wei_raw = (amount_float * 10f64.powi(decimals)) as u64;
        let amount_wei = U256::from(alloy_primitives::U256::from(amount_wei_raw));

        println!(
            "Converting {} tokens to wei with {} decimals: {}",
            amount_float, decimals, amount_wei.0
        );

        // Create the start operation for swapping
        let start_op = StartOperationOperation {
            context: self.context.clone(),
            operation: PortfolioOperation::Swap {
                from_token: from_addr,
                to_token: to_addr,
                amount: amount_wei,
                exact_approval,
            },
        };

        // Execute the start operation
        start_op.execute().await.map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to start swap operation: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        // Based on the portfolio status, execute the appropriate operations
        while !matches!(self.context.get_status(), PortfolioStatus::Completed) {
            match self.context.get_status() {
                PortfolioStatus::CheckingBalances => {
                    let check_op = CheckBalancesOperation {
                        context: self.context.clone(),
                    };
                    check_op.execute().await.map_err(|e| {
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to check balances: {}", e),
                        )) as Box<dyn std::error::Error>
                    })?;
                }
                PortfolioStatus::CalculatingQuotes => {
                    let quotes_op = CalculateQuotesOperation {
                        context: self.context.clone(),
                    };
                    quotes_op.execute().await.map_err(|e| {
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to calculate quotes: {}", e),
                        )) as Box<dyn std::error::Error>
                    })?;
                }
                PortfolioStatus::ExecutingSwaps => {
                    let swaps_op = ExecuteSwapsOperation {
                        context: self.context.clone(),
                    };
                    swaps_op.execute().await.map_err(|e| {
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to execute swaps: {}", e),
                        )) as Box<dyn std::error::Error>
                    })?;
                }
                PortfolioStatus::Error(ref error) => {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Portfolio operation error: {}", error),
                    )) as Box<dyn std::error::Error>);
                }
                _ => break,
            }
        }

        println!("Swap operation completed successfully");
        Ok(())
    }

    /// Helper function to convert U256 to f64 with decimals
    #[allow(dead_code)]
    fn u256_to_f64(&self, value: &U256, decimals: u8) -> f64 {
        let divisor = 10u128.pow(decimals as u32);
        let high_bits = ((value.0.as_limbs()[3] as u128) << 96)
            | ((value.0.as_limbs()[2] as u128) << 64)
            | ((value.0.as_limbs()[1] as u128) << 32)
            | (value.0.as_limbs()[0] as u128);

        high_bits as f64 / divisor as f64
    }

    /// Handle the AAVE health check
    pub async fn handle_aave_health(
        &self,
        wallet_address: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::operations::defi;
        use mfm_core::operations::Operation;

        println!("Checking AAVE health factor...");

        // Create an Aave health check stateful operation
        let health_check_op =
            defi::create_aave_health_check_stateful_operation(self.context.clone(), wallet_address);

        // Execute the health check operation - it will handle data collection, formatting, and display
        health_check_op.execute().await.map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to execute Aave health check: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        // The state machine handles all of the presentation logic
        Ok(())
    }

    /// Handle status command
    pub async fn handle_status(&self) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::operations::portfolio::CheckBalancesOperation;
        use mfm_core::operations::Operation;

        println!("Portfolio Status: {:?}", self.context.get_status());

        // Print token balances
        println!("Current Balances:");
        for token_balance in self.context.get_balances() {
            println!(
                "  {}: {} ({}%)",
                token_balance.token_name, token_balance.balance, token_balance.current_percentage
            );
        }

        println!("Wallet Address: {}", self.context.get_wallet_address());

        // Optionally refresh balances if the user wants the latest information
        if matches!(self.context.get_status(), PortfolioStatus::Init) {
            println!("\nRefreshing balances...");
            let check_op = CheckBalancesOperation {
                context: self.context.clone(),
            };

            match check_op.execute().await {
                Ok(_) => {
                    println!("\nUpdated Balances:");
                    for token_balance in self.context.get_balances() {
                        println!(
                            "  {}: {} ({}%)",
                            token_balance.token_name,
                            token_balance.balance,
                            token_balance.current_percentage
                        );
                    }
                }
                Err(e) => {
                    println!("Error refreshing balances: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Resume an interrupted operation
    pub fn handle_resume(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Just print a message for now - actual implementation would depend on saved state
        println!("Resume functionality not yet implemented");
        Ok(())
    }

    /// Handle the token approval command
    pub async fn handle_token_approval(
        &self,
        token_address: &str,
        spender_address: &str,
        amount: &str,
        exact_approval: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::blockchain::adapter::types::{Address, U256};
        use mfm_core::operations::{token, Operation};
        use std::str::FromStr;

        // Parse addresses and amount
        let token_addr = Address::from_str(token_address).map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid token address: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        let spender_addr = Address::from_str(spender_address).map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid spender address: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        let token_amount = if amount == "max" {
            U256::from(alloy_primitives::U256::MAX)
        } else {
            U256::from_str(amount).map_err(|e| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Invalid amount: {}", e),
                )) as Box<dyn std::error::Error>
            })?
        };

        // Create portfolio context from the CLI context
        let portfolio_ctx = self.context.clone();

        // Create token approval operation
        let approval_operation = token::create_token_approval_operation(
            portfolio_ctx,
            token_addr,
            spender_addr,
            token_amount,
            exact_approval,
        );

        // Execute the token approval operation
        approval_operation.execute().await.map_err(|e| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Token approval failed: {}", e),
            )) as Box<dyn std::error::Error>
        })?;

        println!("Token approval completed successfully");

        Ok(())
    }

    /// Handle keystore import command
    pub async fn handle_keystore_import(
        keystore_path: &str,
        private_key: &str,
        password: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::keystore::KeystoreManager;
        println!(
            "Attempting to import key into keystore at: {}",
            keystore_path
        );
        let manager = KeystoreManager::new(keystore_path)?;
        match manager.import_key(private_key, password).await {
            Ok(pubkey) => {
                // BlsPublicKey now wraps G1Projective. We can get its compressed bytes for hex encoding.
                let pubkey_bytes = pubkey.0.to_affine().to_compressed();
                let pubkey_hex = hex::encode(pubkey_bytes);
                println!(
                    "Key imported successfully. Public Key (hex): 0x{}",
                    pubkey_hex
                );
                Ok(())
            }
            Err(e) => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to import key: {}", e),
            )) as Box<dyn std::error::Error>),
        }
    }

    /// Handle keystore list command
    pub async fn handle_keystore_list(
        keystore_path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::keystore::KeystoreManager;
        println!("Listing keys from keystore at: {}", keystore_path);
        let manager = KeystoreManager::new(keystore_path)?;
        match manager.list_keys().await {
            Ok(pubkeys) => {
                if pubkeys.is_empty() {
                    println!("No keys found in the keystore.");
                } else {
                    println!("Public Keys found:");
                    for pubkey in pubkeys {
                        let pubkey_bytes = pubkey.0.to_affine().to_compressed();
                        let pubkey_hex = hex::encode(pubkey_bytes);
                        println!("  - 0x{}", pubkey_hex);
                    }
                }
                Ok(())
            }
            Err(e) => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to list keys: {}", e),
            )) as Box<dyn std::error::Error>),
        }
    }

    /// Handle keystore delete command
    pub async fn handle_keystore_delete(
        keystore_path: &str,
        pubkey: &str,
        password: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use mfm_core::keystore::KeystoreManager;
        println!(
            "Attempting to delete key {} from keystore at: {}",
            pubkey, keystore_path
        );
        let manager = KeystoreManager::new(keystore_path)?;
        match manager.delete_key(pubkey, password.as_deref()).await {
            Ok(()) => {
                println!("Key {} deleted successfully.", pubkey);
                Ok(())
            }
            Err(e) => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to delete key {}: {}", pubkey, e),
            )) as Box<dyn std::error::Error>),
        }
    }
}
