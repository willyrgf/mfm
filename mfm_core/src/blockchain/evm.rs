use crate::blockchain::adapter::erc20::ERC20;
use crate::blockchain::adapter::provider::{Transaction, TxHash};
use crate::blockchain::adapter::{Address, Bytes, LocalWallet, Provider, U256};
use alloy_primitives;
use alloy_rpc_client::RpcClient;
use alloy_transport_http::Http;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use url::Url;

#[derive(Debug, Error)]
pub enum BlockchainError {
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Contract error: {0}")]
    ContractError(String),
    #[error("Wallet error: {0}")]
    WalletError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
    #[error("Insufficient balance: {0}")]
    InsufficientBalance(String),
    #[error("Transaction error: {0}")]
    TransactionError(String),
    #[error("Arithmetic overflow")]
    Overflow,
    #[error("Quote error: {0}")]
    QuoteError(String),
    #[error("Swap error: {0}")]
    SwapError(String),
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Signing error: {0}")]
    SigningError(String),
    #[error("No signer configured")]
    NoSignerConfigured,
    #[error("Other error: {0}")]
    Other(String),
    #[error("Contract call error: {0}")]
    ContractCallError(String),
    #[error("Balance error: {0}")]
    BalanceError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub name: String,
    pub rpc_url: String,
    pub rpc_urls: Vec<String>,
    pub block_confirmations: u64,
    pub gas_multiplier: f64,
    pub gas_limit: Option<u64>,
    pub gas_price: Option<U256>,
    pub max_fee_per_gas: Option<U256>,
    pub retry_attempts: u64,
}

#[async_trait::async_trait]
pub trait BlockchainProvider: std::fmt::Debug + Send + Sync {
    async fn get_balance(
        &self,
        address: Address,
        token_address: Option<Address>,
    ) -> Result<U256, BlockchainError>;

    async fn get_token_value(
        &self,
        _token_address: Address,
        amount: U256,
    ) -> Result<U256, BlockchainError>;

    fn get_wallet_address(&self) -> Address;

    async fn get_aave_provider(
        &self,
    ) -> Result<Box<dyn crate::blockchain::AaveProvider>, BlockchainError>;
}

#[derive(Debug, Clone)]
pub struct EvmProvider {
    providers: Vec<Arc<Provider>>,
    active_provider_index: Arc<RwLock<usize>>,
    wallet: LocalWallet,
    #[allow(dead_code)]
    config: ChainConfig,
}

impl EvmProvider {
    pub fn new(config: ChainConfig, private_key: String) -> Result<Self, BlockchainError> {
        // Create a list of providers from all available RPC URLs
        let mut providers = Vec::new();

        // First add the primary RPC URL if it's not empty
        if !config.rpc_url.is_empty() {
            let http_client =
                Http::new(Url::parse(&config.rpc_url).map_err(|e| {
                    BlockchainError::Other(format!("Invalid primary RPC URL: {}", e))
                })?);
            let rpc_client = RpcClient::new(http_client, false);
            let provider = Arc::new(Provider::new(Arc::new(rpc_client)));
            providers.push(provider);
        }

        // Then add all the additional RPC URLs
        for url in &config.rpc_urls {
            if !url.is_empty() && (config.rpc_url.is_empty() || url != &config.rpc_url) {
                match Url::parse(url) {
                    Ok(parsed_url) => {
                        let http_client = Http::new(parsed_url);
                        let rpc_client = RpcClient::new(http_client, false);
                        let provider = Arc::new(Provider::new(Arc::new(rpc_client)));
                        providers.push(provider);
                    }
                    Err(e) => {
                        println!("Warning: Invalid RPC URL '{}': {}", url, e);
                        // Continue with other URLs
                    }
                }
            }
        }

        // Ensure we have at least one provider
        if providers.is_empty() {
            return Err(BlockchainError::Other(
                "No valid RPC URLs provided".to_string(),
            ));
        }

        let wallet = LocalWallet::from_private_key(&private_key)
            .map_err(|e| BlockchainError::WalletError(e.to_string()))?
            .with_chain_id(config.chain_id);

        Ok(Self {
            providers,
            active_provider_index: Arc::new(RwLock::new(0)),
            wallet,
            config,
        })
    }

    /// Create a new EVM provider with a wallet
    pub async fn with_private_key(
        rpc_url: &str,
        private_key: String,
    ) -> Result<Self, BlockchainError> {
        // We need to create a Provider from our adapter
        let provider = crate::blockchain::adapter::Provider::connect(rpc_url)
            .await
            .map_err(|e| BlockchainError::ProviderError(e.to_string()))?;

        // Create a wallet from the private key
        let wallet = crate::blockchain::adapter::LocalWallet::from_private_key(&private_key)
            .map_err(|e| BlockchainError::WalletError(e.to_string()))?;

        // For the ChainConfig
        let config = ChainConfig {
            chain_id: 1, // Default to mainnet
            name: "Ethereum Mainnet".to_string(),
            rpc_url: rpc_url.to_string(),
            rpc_urls: vec![rpc_url.to_string()],
            block_confirmations: 1,
            gas_multiplier: 1.2,
            gas_limit: Some(2000000),
            gas_price: None,
            max_fee_per_gas: None,
            retry_attempts: 3,
        };

        Ok(Self {
            providers: vec![Arc::new(provider)],
            wallet,
            config,
            active_provider_index: Arc::new(RwLock::new(0)),
        })
    }

    /// Get the current provider
    async fn current_provider(&self) -> Arc<Provider> {
        let index = *self.active_provider_index.read().await;
        self.providers[index].clone()
    }

    /// Get the balance of an address
    pub async fn get_balance(&self, address: Address) -> Result<U256, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Use the adapter's get_balance method
        let result = provider
            .get_balance(address)
            .await
            .map_err(|e| BlockchainError::BalanceError(e.to_string()));

        if result.is_err() && self.providers.len() > 1 {
            // Try switching providers and retry
            self.try_switch_provider().await?;
            let new_provider = self.current_provider().await;
            return new_provider
                .get_balance(address)
                .await
                .map_err(|e| BlockchainError::BalanceError(e.to_string()));
        }

        result
    }

    /// Call a contract at an address with the given data
    /// Returns the raw bytes response
    pub async fn call_contract(
        &self,
        address: Address,
        data: Bytes,
    ) -> Result<Bytes, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Call the contract
        let result = provider
            .call(address, data.clone())
            .await
            .map_err(|e| BlockchainError::ContractCallError(e.to_string()));

        if result.is_err() && self.providers.len() > 1 {
            // Try switching providers and retry
            self.try_switch_provider().await?;
            let new_provider = self.current_provider().await;
            return new_provider
                .call(address, data)
                .await
                .map_err(|e| BlockchainError::ContractCallError(e.to_string()));
        }

        result
    }

    /// Get a token balance for an address
    pub async fn get_token_balance(
        &self,
        token: Option<Address>,
        address: Address,
    ) -> Result<U256, BlockchainError> {
        match token {
            Some(token_address) => {
                // Get the current provider
                let provider = self.current_provider().await;

                // Create an ERC20 contract instance
                let token_contract = ERC20::new(token_address, (*provider).clone());

                // Call balance_of on the token contract
                token_contract
                    .balance_of(address)
                    .await
                    .map_err(|e| BlockchainError::BalanceError(e.to_string()))
            }
            None => {
                // For native token (ETH) balance
                self.get_balance(address).await
            }
        }
    }

    /// Estimates gas for a transaction
    pub async fn estimate_gas(
        &self,
        to: Address,
        data: Bytes,
        value: Option<U256>,
    ) -> Result<U256, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Create the transaction data for gas estimation
        let tx = Transaction {
            to: Some(to),
            value,
            data: data.clone(),
            nonce: None,
            gas_price: None,
            gas: None,
            chain_id: Some(self.config.chain_id),
        };

        // Estimate gas using the provider
        let result = provider.estimate_gas(&tx).await.map_err(|e| {
            BlockchainError::ContractCallError(format!("Gas estimation failed: {}", e))
        });

        if result.is_err() && self.providers.len() > 1 {
            // Try switching providers and retry
            self.try_switch_provider().await?;
            let new_provider = self.current_provider().await;

            // Try again with the new provider
            return new_provider.estimate_gas(&tx).await.map_err(|e| {
                BlockchainError::ContractCallError(format!("Gas estimation failed: {}", e))
            });
        }

        result
    }

    /// Sends a transaction and returns the transaction hash
    pub async fn send_transaction(
        &self,
        to: Address,
        data: Bytes,
        value: Option<U256>,
        gas: Option<U256>,
    ) -> Result<TxHash, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Calculate gas limit either from parameter or estimate it
        let gas_limit = match gas {
            Some(g) => g,
            None => {
                let estimated_gas = self.estimate_gas(to, data.clone(), value).await?;
                // Apply the gas multiplier from config
                let multiplier = self.config.gas_multiplier;
                let gas_value = estimated_gas.0.to::<u64>() as f64;
                let gas_with_buffer = (gas_value * multiplier) as u64;
                U256(alloy_primitives::U256::from(gas_with_buffer))
            }
        };

        // Apply max gas limit from config if needed
        if let Some(max_gas) = self.config.gas_limit {
            // Using temporary variable to compare, but ignoring it since wallet.send_transaction
            // will take care of gas estimation internally
            let _ = std::cmp::min(gas_limit, U256(alloy_primitives::U256::from(max_gas)));
        }

        // Send the transaction using our wallet
        let result = self
            .wallet
            .send_transaction(&provider, Some(to), value, data.clone())
            .await
            .map_err(|e| {
                BlockchainError::TransactionError(format!("Failed to send transaction: {}", e))
            });

        if result.is_err() && self.providers.len() > 1 {
            // Try switching providers and retry
            self.try_switch_provider().await?;
            let new_provider = self.current_provider().await;

            // Try again with the new provider
            return self
                .wallet
                .send_transaction(&new_provider, Some(to), value, data)
                .await
                .map_err(|e| {
                    BlockchainError::TransactionError(format!("Failed to send transaction: {}", e))
                });
        }

        result
    }

    /// Gets the transaction receipt for a transaction hash
    pub async fn get_transaction_receipt(
        &self,
        tx_hash: TxHash,
    ) -> Result<Option<serde_json::Value>, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Call the eth_getTransactionReceipt RPC method
        let result = provider
            .inner()
            .request::<_, serde_json::Value>("eth_getTransactionReceipt", [tx_hash.clone()])
            .await
            .map_err(|e| {
                BlockchainError::Other(format!("Failed to get transaction receipt: {}", e))
            });

        if result.is_err() && self.providers.len() > 1 {
            // Try switching providers and retry
            self.try_switch_provider().await?;
            let new_provider = self.current_provider().await;

            // Try again with the new provider
            return new_provider
                .inner()
                .request::<_, serde_json::Value>("eth_getTransactionReceipt", [tx_hash])
                .await
                .map_err(|e| {
                    BlockchainError::Other(format!("Failed to get transaction receipt: {}", e))
                })
                .map(|receipt| {
                    if receipt.is_null() {
                        None
                    } else {
                        Some(receipt)
                    }
                });
        }

        let receipt = result?;

        // Check if the receipt is null
        if receipt.is_null() {
            return Ok(None);
        }

        Ok(Some(receipt))
    }

    /// Wait for a transaction to be confirmed
    pub async fn wait_for_transaction(
        &self,
        tx_hash: TxHash,
        confirmations: Option<u64>,
    ) -> Result<serde_json::Value, BlockchainError> {
        // Get the confirmations from config or use the provided one
        let conf = confirmations.unwrap_or(self.config.block_confirmations);

        // Loop until we have enough confirmations
        let mut attempts = 0;
        let max_attempts = 50; // Prevent infinite loops

        loop {
            if attempts >= max_attempts {
                return Err(BlockchainError::TransactionError(
                    "Transaction not confirmed after max attempts".to_string(),
                ));
            }

            // Wait a bit between checks
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            // Get the current provider
            let provider = self.current_provider().await;

            // Get the transaction receipt
            if let Some(receipt) = self.get_transaction_receipt(tx_hash.clone()).await? {
                // Check if the receipt has a block number
                if let Some(block_number) = receipt.get("blockNumber") {
                    // Get the latest block number
                    let latest_block = provider
                        .inner()
                        .request::<_, alloy_primitives::U256>("eth_blockNumber", ())
                        .await
                        .map_err(|e| {
                            BlockchainError::Other(format!("Failed to get block number: {}", e))
                        })?;

                    // Parse the receipt block number
                    let receipt_block_str = block_number.as_str().unwrap_or("0x0");
                    let receipt_block = alloy_primitives::U256::from_str_radix(
                        receipt_block_str.trim_start_matches("0x"),
                        16,
                    )
                    .map_err(|e| {
                        BlockchainError::Other(format!("Failed to parse block number: {}", e))
                    })?;

                    // Calculate confirmations
                    let confirmation_blocks =
                        latest_block.checked_sub(receipt_block).unwrap_or_default();

                    if confirmation_blocks >= alloy_primitives::U256::from(conf) {
                        return Ok(receipt);
                    }
                }
            }

            attempts += 1;
        }
    }

    /// Transfer ETH to an address
    pub async fn transfer_eth(&self, to: Address, amount: U256) -> Result<TxHash, BlockchainError> {
        // Create an empty data field for a simple ETH transfer
        let data = Bytes(Vec::new());

        // Send the transaction with the ETH value
        self.send_transaction(to, data, Some(amount), None).await
    }

    /// Transfer ERC20 tokens to an address
    pub async fn transfer_erc20(
        &self,
        token_address: Address,
        to: Address,
        amount: U256,
    ) -> Result<TxHash, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Create an ERC20 contract instance - use as_ref() to get Provider from Arc<Provider>
        let token_contract = ERC20::new(token_address, (*provider).clone());

        // Check our balance before transferring
        let balance = token_contract
            .balance_of(self.wallet.address())
            .await
            .map_err(|e| BlockchainError::BalanceError(e.to_string()))?;

        if balance < amount {
            return Err(BlockchainError::InsufficientBalance(format!(
                "Insufficient token balance. Required: {}, Available: {}",
                amount.0, balance.0
            )));
        }

        // Prepare the transfer call
        let data = token_contract
            .encode_transfer(to, amount)
            .map_err(|e| BlockchainError::ContractError(e.to_string()))?;

        // Send the transaction
        self.send_transaction(token_address, data, None, None).await
    }

    /// Approve ERC20 tokens for a spender
    pub async fn approve_erc20(
        &self,
        token_address: Address,
        spender: Address,
        amount: U256,
    ) -> Result<TxHash, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Create an ERC20 contract instance - use as_ref() to get Provider from Arc<Provider>
        let token_contract = ERC20::new(token_address, (*provider).clone());

        // Prepare the approve call
        let data = token_contract
            .encode_approve(spender, amount)
            .map_err(|e| BlockchainError::ContractError(e.to_string()))?;

        // Send the transaction
        self.send_transaction(token_address, data, None, None).await
    }

    /// Check allowance for ERC20 tokens
    pub async fn erc20_allowance(
        &self,
        token_address: Address,
        owner: Address,
        spender: Address,
    ) -> Result<U256, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Create an ERC20 contract instance - use as_ref() to get Provider from Arc<Provider>
        let token_contract = ERC20::new(token_address, (*provider).clone());

        // Call allowance on the token contract
        token_contract
            .allowance(owner, spender)
            .await
            .map_err(|e| BlockchainError::ContractError(e.to_string()))
    }

    /// Switch to a different provider if the current one fails
    async fn try_switch_provider(&self) -> Result<(), BlockchainError> {
        if self.providers.len() <= 1 {
            return Err(BlockchainError::ProviderError(
                "No alternative providers available".to_string(),
            ));
        }

        let current_index = *self.active_provider_index.read().await;
        let next_index = (current_index + 1) % self.providers.len();

        // Update the active provider index
        let mut index = self.active_provider_index.write().await;
        *index = next_index;

        println!("Switched to provider {}", next_index);
        Ok(())
    }

    /// Get the chain ID from the provider
    pub async fn get_chain_id(&self) -> Result<u64, BlockchainError> {
        // Get the current provider
        let provider = self.current_provider().await;

        // Call the chain_id method
        provider
            .get_chainid()
            .await
            .map_err(|e| BlockchainError::ProviderError(format!("Failed to get chain ID: {}", e)))
    }
}

#[async_trait::async_trait]
impl BlockchainProvider for EvmProvider {
    async fn get_balance(
        &self,
        address: Address,
        token_address: Option<Address>,
    ) -> Result<U256, BlockchainError> {
        match token_address {
            Some(token) => {
                // Get the current provider
                let provider = self.current_provider().await;

                // Create an ERC20 contract instance
                let token_contract = ERC20::new(token, (*provider).clone());

                // Call balance_of on the token contract
                token_contract
                    .balance_of(address)
                    .await
                    .map_err(|e| BlockchainError::BalanceError(e.to_string()))
            }
            None => {
                // For native token (ETH) balance
                self.get_balance(address).await
            }
        }
    }

    async fn get_token_value(
        &self,
        _token_address: Address,
        amount: U256,
    ) -> Result<U256, BlockchainError> {
        // For now, we'll just return the amount as-is
        // In a real implementation, this would fetch price data and calculate the value
        // This will be implemented in Phase 3 with oracle integration

        // Return the same amount for now - treating all tokens as 1:1 value
        Ok(amount)
    }

    fn get_wallet_address(&self) -> Address {
        // Get the address from our adapter wallet
        self.wallet.address()
    }

    async fn get_aave_provider(
        &self,
    ) -> Result<Box<dyn crate::blockchain::AaveProvider>, BlockchainError> {
        use std::str::FromStr;

        // Get the current provider - now we can use await properly
        let provider = self.current_provider().await;

        // Default Aave V3 mainnet addresses
        let lending_pool_address = Address::from_str("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2")
            .map_err(|_| {
                BlockchainError::InvalidAddress("Invalid Aave lending pool address".to_string())
            })?;

        let data_provider_address = Address::from_str("0x7B4EB56E7CD4b454BA8ff71E4518426369a138a3")
            .map_err(|_| {
                BlockchainError::InvalidAddress("Invalid Aave data provider address".to_string())
            })?;

        // Create the Aave provider - we're removing the block_on calls that cause runtime nesting
        let aave_provider = crate::blockchain::aave::AaveAdapterProvider::new(
            provider,
            lending_pool_address,
            data_provider_address,
        );

        // Box it and return
        Ok(Box::new(aave_provider))
    }
}
