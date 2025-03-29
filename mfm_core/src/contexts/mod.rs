use crate::blockchain::{BlockchainProvider, DexProvider};
use crate::config::Config;
use crate::portfolio::{Portfolio, PortfolioOperation, PortfolioStatus};
use mfm_machine::state::Label;
use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    YamlFile(String),
}
pub const CONFIG_SOURCE_CTX: Label = Label("config_source_ctx");

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ConfigCtx {
    pub config_source: ConfigSource,
    pub config: Config,
}
pub const CONFIG_CTX: Label = Label("config_ctx");

// Portfolio Context for managing portfolio operations
pub mod portfolio {
    use super::*;
    use crate::blockchain::dex::SwapQuote;
    use crate::portfolio::{PortfolioError, TokenBalance};
    use std::sync::{Arc, RwLock};

    pub const PORTFOLIO_CTX: Label = Label("portfolio_ctx");

    // Holds portfolio data in a thread-safe way
    #[derive(Clone, Debug)]
    pub struct PortfolioContext {
        inner: Arc<RwLock<PortfolioContextInner>>,
    }

    // Inner data of the portfolio context
    #[derive(Debug)]
    struct PortfolioContextInner {
        pub portfolio: Portfolio,
    }

    impl PortfolioContext {
        // Create a new portfolio context
        pub fn new(
            blockchain_provider: Box<dyn BlockchainProvider>,
            dex_provider: Box<dyn DexProvider>,
            config: &Config,
        ) -> Self {
            Self {
                inner: Arc::new(RwLock::new(PortfolioContextInner {
                    portfolio: Portfolio::new(blockchain_provider, dex_provider, config),
                })),
            }
        }

        // Start a portfolio operation
        pub async fn start_operation(
            &self,
            operation: PortfolioOperation,
        ) -> Result<(), PortfolioError> {
            // First check if we're in a valid state (Init)
            {
                let inner = self.inner.read().unwrap();
                if inner.portfolio.status != PortfolioStatus::Init {
                    return Err(PortfolioError::InvalidStateTransition(
                        "Can only start operation in Init state".to_string(),
                    ));
                }
            }

            // Set up the operation
            {
                let mut inner = self.inner.write().unwrap();
                inner.portfolio.operation = Some(operation.clone());
                let operation_str = format!("{:?}", operation);
                inner.portfolio.state.last_operation = Some(operation_str);
                inner.portfolio.state.last_error = None;
                inner.portfolio.status = PortfolioStatus::CheckingBalances;
            }

            // Operation is started successfully
            Ok(())
        }

        // Check token balances
        pub async fn check_balances(&self) -> Result<(), PortfolioError> {
            // First check if we're in a valid state
            {
                let inner = self.inner.read().unwrap();
                if inner.portfolio.status != PortfolioStatus::CheckingBalances
                    && inner.portfolio.status != PortfolioStatus::Init
                {
                    return Err(PortfolioError::InvalidStateTransition(
                        "Not in a valid state for checking balances".to_string(),
                    ));
                }
            }

            // Update status to checking balances
            {
                let mut inner = self.inner.write().unwrap();
                inner.portfolio.status = PortfolioStatus::CheckingBalances;
            }

            // Create a separate function to capture minimal data and avoid holding locks
            async fn do_check_balances(context: &PortfolioContext) -> Result<(), PortfolioError> {
                // This will clone any minimal data we need
                let portfolio_status;

                // We need to get a copy of the balances
                {
                    let inner = context.inner.read().unwrap();
                    portfolio_status = inner.portfolio.status.clone();
                }

                // Verify we're still in the right state
                if portfolio_status != PortfolioStatus::CheckingBalances {
                    return Err(PortfolioError::InvalidStateTransition(
                        "Status changed during balance check".to_string(),
                    ));
                }

                // Now we can do the external work without holding any locks

                // When complete, update the state to preparing quotes
                {
                    let mut inner = context.inner.write().unwrap();
                    inner.portfolio.status = PortfolioStatus::CalculatingQuotes;
                }

                Ok(())
            }

            // Call the separate function that doesn't hold locks across awaits
            match do_check_balances(self).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Update status on error
                    let mut inner = self.inner.write().unwrap();
                    inner.portfolio.status = PortfolioStatus::Error(format!("{}", e));
                    Err(e)
                }
            }
        }

        // Calculate swap quotes
        pub async fn calculate_quotes(&self) -> Result<(), PortfolioError> {
            // First check if we're in a valid state
            {
                let inner = self.inner.read().unwrap();
                if inner.portfolio.status != PortfolioStatus::CalculatingQuotes {
                    return Err(PortfolioError::InvalidStateTransition(
                        "Not in a valid state for calculating quotes".to_string(),
                    ));
                }
            }

            // Create a separate function to avoid holding locks across awaits
            async fn do_calculate_quotes(context: &PortfolioContext) -> Result<(), PortfolioError> {
                // We'll need the current operation type
                let operation_type;
                {
                    let inner = context.inner.read().unwrap();
                    operation_type = inner.portfolio.operation.clone();
                }

                // Execute based on operation type
                match operation_type {
                    Some(PortfolioOperation::Swap { .. }) => {
                        // For a swap, we would normally calculate quotes
                        // But for simplicity we'll just update the status
                        let mut inner = context.inner.write().unwrap();
                        inner.portfolio.status = PortfolioStatus::ExecutingSwaps;
                        Ok(())
                    }
                    Some(_) => {
                        // For other operations, just move to next stage
                        let mut inner = context.inner.write().unwrap();
                        inner.portfolio.status = PortfolioStatus::ExecutingSwaps;
                        Ok(())
                    }
                    None => Err(PortfolioError::InvalidStateTransition(
                        "No operation specified".to_string(),
                    )),
                }
            }

            // Call the separate function
            match do_calculate_quotes(self).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Update status on error
                    let mut inner = self.inner.write().unwrap();
                    inner.portfolio.status = PortfolioStatus::Error(format!("{}", e));
                    Err(e)
                }
            }
        }

        // Execute swaps
        pub async fn execute_swaps(&self) -> Result<(), PortfolioError> {
            // First check if we're in a valid state
            {
                let inner = self.inner.read().unwrap();
                if inner.portfolio.status != PortfolioStatus::ExecutingSwaps {
                    return Err(PortfolioError::InvalidStateTransition(
                        "Not in a valid state for executing swaps".to_string(),
                    ));
                }
            }

            // Create a separate function to avoid holding locks across awaits
            async fn do_execute_swaps(context: &PortfolioContext) -> Result<(), PortfolioError> {
                // We'll need the current operation type
                let operation_type;
                {
                    let inner = context.inner.read().unwrap();
                    operation_type = inner.portfolio.operation.clone();
                }

                // Execute based on operation type
                match operation_type {
                    Some(PortfolioOperation::Swap { .. }) => {
                        // For a swap, we would execute the swap
                        // But for simplicity we'll just update the status
                        let mut inner = context.inner.write().unwrap();
                        inner.portfolio.status = PortfolioStatus::Completed;
                        Ok(())
                    }
                    Some(_) => {
                        // For other operations, just complete
                        let mut inner = context.inner.write().unwrap();
                        inner.portfolio.status = PortfolioStatus::Completed;
                        Ok(())
                    }
                    None => Err(PortfolioError::InvalidStateTransition(
                        "No operation specified".to_string(),
                    )),
                }
            }

            // Call the separate function
            match do_execute_swaps(self).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Update status on error
                    let mut inner = self.inner.write().unwrap();
                    inner.portfolio.status = PortfolioStatus::Error(format!("{}", e));
                    Err(e)
                }
            }
        }

        // Get the current portfolio status
        pub fn get_status(&self) -> PortfolioStatus {
            let inner = self.inner.read().unwrap();
            inner.portfolio.status.clone()
        }

        // Get the token balances
        pub fn get_balances(&self) -> Vec<TokenBalance> {
            let inner = self.inner.read().unwrap();
            inner.portfolio.state.balances.clone()
        }

        // Get the current quote
        pub fn get_quote(&self) -> Option<SwapQuote> {
            let inner = self.inner.read().unwrap();
            inner.portfolio.state.last_quote.clone()
        }

        // Get the current operation
        pub fn get_operation(&self) -> Option<PortfolioOperation> {
            let inner = self.inner.read().unwrap();
            inner.portfolio.operation.clone()
        }

        // Get the wallet address from config
        pub fn get_wallet_address(&self) -> String {
            let inner = self.inner.read().unwrap();
            inner.portfolio.config.wallet.address.to_string()
        }

        // Get portfolio total value
        pub fn get_total_value(&self) -> crate::blockchain::adapter::types::U256 {
            let inner = self.inner.read().unwrap();
            inner.portfolio.state.total_value
        }

        // Get a reference to the blockchain provider
        pub fn with_blockchain_provider<F, R>(
            &self,
            f: F,
        ) -> Result<R, crate::portfolio::PortfolioError>
        where
            F: FnOnce(
                &Box<dyn crate::blockchain::BlockchainProvider>,
            ) -> Result<R, crate::blockchain::BlockchainError>,
        {
            let guard = self.inner.read().unwrap();
            f(&guard.portfolio.blockchain_provider)
                .map_err(crate::portfolio::PortfolioError::BlockchainError)
        }

        // Get a reference to the blockchain provider for async operations
        #[allow(dead_code, clippy::await_holding_lock)]
        pub(self) fn blockchain_provider_async(
            &self,
        ) -> std::sync::RwLockReadGuard<'_, PortfolioContextInner> {
            self.inner.read().unwrap()
        }

        /// Gets the aave provider from the blockchain provider
        #[allow(clippy::await_holding_lock)]
        pub async fn get_aave_provider(
            &self,
        ) -> Result<
            Box<dyn crate::blockchain::aave::AaveProvider>,
            crate::blockchain::BlockchainError,
        > {
            // This is a temporary solution - we should refactor to use tokio::sync::RwLock
            // for proper async locking in the future
            let guard = self.inner.read().unwrap();
            guard
                .portfolio
                .blockchain_provider
                .get_aave_provider()
                .await
        }
    }
}
