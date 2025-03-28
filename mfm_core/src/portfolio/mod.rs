use crate::blockchain::adapter::types::{Address, U256};
use crate::blockchain::{BlockchainProvider, DexProvider};
use alloy_primitives::address;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PortfolioError {
    #[error("Blockchain error: {0}")]
    BlockchainError(#[from] crate::blockchain::BlockchainError),
    #[error("DEX error: {0}")]
    DexError(#[from] crate::blockchain::DexError),
    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),
    #[error("Operation interrupted: {0}")]
    OperationInterrupted(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBalance {
    pub token: Address,
    pub token_name: String,
    pub balance: U256,
    pub target_percentage: f64,
    pub current_percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioState {
    pub balances: Vec<TokenBalance>,
    pub total_value: U256,
    pub last_operation: Option<String>,
    pub last_error: Option<String>,
    pub last_quote: Option<crate::blockchain::dex::SwapQuote>,
}

/// status of the portfolio operations
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PortfolioStatus {
    /// initial state
    Init,
    /// checking token balances
    CheckingBalances,
    /// calculating quotes for swaps
    CalculatingQuotes,
    /// executing swaps
    ExecutingSwaps,
    /// operation completed
    Completed,
    /// operation failed with error
    Error(String),
}

/// type of operation to perform on the portfolio
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PortfolioOperation {
    /// rebalance the portfolio
    Rebalance,
    /// execute a swap between tokens
    Swap {
        /// token to swap from
        from_token: Address,
        /// token to swap to
        to_token: Address,
        /// amount to swap
        amount: U256,
        /// whether to approve exact amount
        exact_approval: bool,
    },
    /// check health of the portfolio
    HealthCheck,
    /// no operation
    None,
}

#[derive(Debug)]
pub struct Portfolio {
    pub state: PortfolioState,
    pub status: PortfolioStatus,
    pub operation: Option<PortfolioOperation>,
    pub blockchain_provider: Box<dyn BlockchainProvider>,
    pub dex_provider: Box<dyn DexProvider>,
    pub config: crate::config::Config,
}

impl Serialize for Portfolio {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Portfolio", 3)?;
        state.serialize_field("state", &self.state)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("operation", &self.operation)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for Portfolio {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PortfolioHelper {
            #[allow(dead_code)]
            state: PortfolioState,
            #[allow(dead_code)]
            status: PortfolioStatus,
            #[allow(dead_code)]
            operation: Option<PortfolioOperation>,
        }

        let _helper = PortfolioHelper::deserialize(deserializer)?;

        // This is a placeholder - the actual providers should be created elsewhere
        Err(serde::de::Error::custom(
            "Portfolio cannot be deserialized without providers",
        ))
    }
}

impl Portfolio {
    pub fn new(
        blockchain_provider: Box<dyn BlockchainProvider>,
        dex_provider: Box<dyn DexProvider>,
        config: &crate::config::Config,
    ) -> Self {
        let mut balances = Vec::new();

        // Add ETH balance using Zero address from alloy primitives
        let zero_address = Address(address!("0000000000000000000000000000000000000000"));

        balances.push(TokenBalance {
            token: zero_address, // ETH uses zero address
            token_name: "ETH".to_string(),
            balance: U256::from(alloy_primitives::U256::ZERO),
            target_percentage: 0.0,
            current_percentage: 0.0,
        });

        // Add token balances from config
        for (token_key, token) in config.tokens.hashmap() {
            if let Some(network) = token.networks.hashmap().get(&config.network.name) {
                if let Ok(address) = Address::from_str(&network.address) {
                    balances.push(TokenBalance {
                        token: address,
                        token_name: token_key.to_uppercase(),
                        balance: U256::from(alloy_primitives::U256::ZERO),
                        target_percentage: 0.0,
                        current_percentage: 0.0,
                    });
                }
            }
        }

        Self {
            state: PortfolioState {
                balances,
                total_value: U256::from(alloy_primitives::U256::ZERO),
                last_operation: None,
                last_error: None,
                last_quote: None,
            },
            status: PortfolioStatus::Init,
            operation: None,
            blockchain_provider,
            dex_provider,
            config: config.clone(),
        }
    }

    pub async fn start_operation(
        &mut self,
        operation: PortfolioOperation,
    ) -> Result<(), PortfolioError> {
        if self.status != PortfolioStatus::Init {
            return Err(PortfolioError::InvalidStateTransition(
                "Cannot start new operation while another is in progress".to_string(),
            ));
        }

        let operation_str = format!("{:?}", operation);
        self.operation = Some(operation);
        self.status = PortfolioStatus::CheckingBalances;
        self.state.last_operation = Some(operation_str);
        self.state.last_error = None;

        Ok(())
    }

    pub async fn check_balances(&mut self) -> Result<(), PortfolioError> {
        if self.status != PortfolioStatus::CheckingBalances {
            return Err(PortfolioError::InvalidStateTransition(
                "Not in balance checking state".to_string(),
            ));
        }

        eprintln!("\n=== Starting Portfolio Balance Check ===");
        let wallet_address = self.blockchain_provider.get_wallet_address();
        eprintln!("Using wallet address: {}", wallet_address);

        // Get balances for all tokens
        let zero = U256::from(alloy_primitives::U256::ZERO);
        let mut total_value = zero;
        let mut balances = Vec::new();

        // Zero address constant
        let zero_address = Address(address!("0000000000000000000000000000000000000000"));

        // Check balances for each configured token
        for token_balance in &self.state.balances {
            if token_balance.token == zero_address {
                // Handle native ETH
                eprintln!("\n--- Checking Native ETH Balance ---");
                eprintln!("Token: ETH ({})", token_balance.token);
                let eth_balance = self
                    .blockchain_provider
                    .get_balance(wallet_address, None)
                    .await?;

                eprintln!("Native ETH balance: {}", eth_balance);

                let token_value = eth_balance; // For ETH, value is the same as balance
                total_value = match alloy_primitives::U256::from(total_value.0)
                    .checked_add(alloy_primitives::U256::from(token_value.0))
                {
                    Some(sum) => U256(sum),
                    None => {
                        return Err(PortfolioError::BlockchainError(
                            crate::blockchain::BlockchainError::Overflow,
                        ))
                    }
                };

                balances.push(TokenBalance {
                    token: zero_address,
                    token_name: "ETH".to_string(),
                    balance: eth_balance,
                    target_percentage: token_balance.target_percentage,
                    current_percentage: 0.0, // Will be calculated below
                });
            } else {
                // Handle ERC20 tokens
                eprintln!("\n--- Checking ERC20 Token Balance ---");
                eprintln!(
                    "Token: {} ({})",
                    token_balance.token_name, token_balance.token
                );
                let balance = self
                    .blockchain_provider
                    .get_balance(wallet_address, Some(token_balance.token))
                    .await?;

                eprintln!("ERC20 balance: {}", balance);

                let token_value = self
                    .blockchain_provider
                    .get_token_value(token_balance.token, balance)
                    .await?;

                eprintln!("Token value: {}", token_value);

                total_value = match alloy_primitives::U256::from(total_value.0)
                    .checked_add(alloy_primitives::U256::from(token_value.0))
                {
                    Some(sum) => U256(sum),
                    None => {
                        return Err(PortfolioError::BlockchainError(
                            crate::blockchain::BlockchainError::Overflow,
                        ))
                    }
                };

                balances.push(TokenBalance {
                    token: token_balance.token,
                    token_name: token_balance.token_name.clone(),
                    balance,
                    target_percentage: token_balance.target_percentage,
                    current_percentage: 0.0, // Will be calculated below
                });
            }
        }

        eprintln!("\n=== Calculating Portfolio Percentages ===");
        eprintln!("Total portfolio value: {}", total_value);

        // Calculate current percentages
        if total_value > U256::from(alloy_primitives::U256::ZERO) {
            for balance in &mut balances {
                let token_value = if balance.token == zero_address {
                    balance.balance // For ETH, value is the same as balance
                } else {
                    self.blockchain_provider
                        .get_token_value(balance.token, balance.balance)
                        .await?
                };
                balance.current_percentage =
                    (token_value.as_u128() as f64 / total_value.as_u128() as f64) * 100.0;
                eprintln!(
                    "{}: {}% (Value: {})",
                    balance.token_name, balance.current_percentage, token_value
                );
            }
        }

        // Update portfolio state
        self.state.balances = balances;
        self.state.total_value = total_value;
        self.status = PortfolioStatus::CalculatingQuotes;

        eprintln!("=== Portfolio Balance Check Complete ===\n");

        Ok(())
    }

    pub async fn calculate_quotes(&mut self) -> Result<(), PortfolioError> {
        if self.status != PortfolioStatus::CalculatingQuotes {
            return Err(PortfolioError::InvalidStateTransition(
                "Not in quote calculation state".to_string(),
            ));
        }

        eprintln!("\n=== Calculating Quotes ===");

        match &self.operation {
            Some(PortfolioOperation::Swap {
                from_token,
                to_token,
                amount,
                exact_approval: _exact_approval,
            }) => {
                eprintln!("Getting quote for swap:");
                eprintln!("From: {}", from_token);
                eprintln!("To: {}", to_token);
                eprintln!("Amount: {}", amount);

                let quote = self
                    .dex_provider
                    .get_quote(*from_token, *to_token, *amount)
                    .await
                    .map_err(PortfolioError::DexError)?;

                eprintln!("\nQuote received:");
                eprintln!("To amount: {}", quote.to_amount);
                eprintln!("Price impact: {}%", quote.price_impact);
                eprintln!("Route: {:?}", quote.route);

                // Store the quote in the state for later use
                self.state.last_quote = Some(quote);
            }
            Some(PortfolioOperation::Rebalance) => {
                // TODO: Implement rebalancing quotes
                eprintln!("Rebalancing quotes not implemented yet");
            }
            Some(PortfolioOperation::HealthCheck) => {
                // TODO: Implement health check
                eprintln!("Health check calculation not implemented yet");
            }
            Some(PortfolioOperation::None) => {
                eprintln!("No operation specified, skipping quote calculation");
            }
            None => {
                return Err(PortfolioError::InvalidStateTransition(
                    "No operation in progress".to_string(),
                ));
            }
        }

        self.status = PortfolioStatus::ExecutingSwaps;
        eprintln!("=== Quote Calculation Complete ===\n");
        Ok(())
    }

    pub async fn execute_swaps(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(PortfolioOperation::Swap {
            from_token,
            to_token,
            amount,
            exact_approval,
        }) = self.operation
        {
            eprintln!("Executing swap:");
            eprintln!("  From token: {}", from_token);
            eprintln!("  To token: {}", to_token);
            eprintln!("  Amount: {}", amount);
            eprintln!("  Exact approval: {}", exact_approval);

            let tx_hash = self
                .dex_provider
                .execute_swap(from_token, to_token, amount, exact_approval)
                .await?;

            eprintln!("Swap executed successfully!");
            eprintln!("Transaction hash: {}", tx_hash);

            self.status = PortfolioStatus::Completed;
            Ok(())
        } else {
            Err("No swap operation in progress".into())
        }
    }

    #[allow(dead_code)]
    pub fn interrupt(&mut self) {
        self.status = PortfolioStatus::Init;
        self.operation = None;
    }

    pub fn resume(&mut self) -> Result<(), PortfolioError> {
        if self.operation.is_none() {
            return Err(PortfolioError::OperationInterrupted(
                "No operation to resume".to_string(),
            ));
        }

        match self.status {
            PortfolioStatus::Error(_) => {
                self.status = PortfolioStatus::CheckingBalances;
                Ok(())
            }
            _ => Err(PortfolioError::InvalidStateTransition(
                "Cannot resume from current state".to_string(),
            )),
        }
    }
}
