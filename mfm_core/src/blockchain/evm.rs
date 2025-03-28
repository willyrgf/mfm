use crate::blockchain::adapter::erc20::ERC20;
use crate::blockchain::adapter::{Address, Bytes, LocalWallet, Provider, U256};
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

    fn get_aave_provider(
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
        provider
            .get_balance(address)
            .await
            .map_err(|e| BlockchainError::BalanceError(e.to_string()))
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

        // Use the adapter's call method
        provider
            .call(address, data)
            .await
            .map_err(|e| BlockchainError::ContractCallError(e.to_string()))
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

    fn get_aave_provider(
        &self,
    ) -> Result<Box<dyn crate::blockchain::AaveProvider>, BlockchainError> {
        use crate::blockchain::aave::create_aave_provider;
        use std::str::FromStr;

        // Get the current provider
        let provider_fut = self.current_provider();
        let provider = tokio::runtime::Handle::current().block_on(provider_fut);

        // Default Aave V3 mainnet addresses
        let lending_pool_address = Address::from_str("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2")
            .map_err(|_| {
                BlockchainError::InvalidAddress("Invalid Aave lending pool address".to_string())
            })?;

        let data_provider_address = Address::from_str("0x7B4EB56E7CD4b454BA8ff71E4518426369a138a3")
            .map_err(|_| {
                BlockchainError::InvalidAddress("Invalid Aave data provider address".to_string())
            })?;

        // Create the Aave provider asynchronously but block on it since this method is synchronous
        let aave_provider_fut =
            create_aave_provider(provider, lending_pool_address, data_provider_address);
        let aave_provider = tokio::runtime::Handle::current().block_on(aave_provider_fut)?;

        // Box it and return
        Ok(Box::new(aave_provider))
    }
}
