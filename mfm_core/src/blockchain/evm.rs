use ethers::{
    providers::{Http, Middleware, Provider},
    signers::{LocalWallet, Signer},
    types::{Address, Bytes, TransactionRequest, U256},
    utils::keccak256,
};
use serde::{Deserialize, Serialize};
use std::{str::FromStr, sync::Arc};
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    pub rpc_url: String,
    pub rpc_urls: Vec<String>,
    pub chain_id: u64,
    pub name: String,
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
}

#[derive(Debug, Clone)]
pub struct EvmProvider {
    providers: Vec<Provider<Http>>,
    active_provider_index: Arc<RwLock<usize>>,
    wallet: LocalWallet,
}

impl EvmProvider {
    pub fn new(config: ChainConfig, private_key: String) -> Result<Self, BlockchainError> {
        // Create a list of providers from all available RPC URLs
        let mut providers = Vec::new();

        // First add the primary RPC URL if it's not empty
        if !config.rpc_url.is_empty() {
            let provider =
                Provider::new(Http::new(Url::parse(&config.rpc_url).map_err(|e| {
                    BlockchainError::Other(format!("Invalid primary RPC URL: {}", e))
                })?));
            providers.push(provider);
        }

        // Then add all the additional RPC URLs
        for url in &config.rpc_urls {
            if !url.is_empty() && (config.rpc_url.is_empty() || url != &config.rpc_url) {
                match Url::parse(url) {
                    Ok(parsed_url) => {
                        let provider = Provider::new(Http::new(parsed_url));
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

        let wallet = LocalWallet::from_str(&private_key)
            .map_err(|e| BlockchainError::WalletError(e.to_string()))?
            .with_chain_id(config.chain_id);

        Ok(Self {
            providers,
            active_provider_index: Arc::new(RwLock::new(0)),
            wallet,
        })
    }

    // Get the current active provider
    async fn get_provider(&self) -> Provider<Http> {
        let index = *self.active_provider_index.read().await;
        self.providers[index].clone()
    }

    // Try to use the next provider in the list
    async fn try_next_provider(&self) -> Result<Provider<Http>, BlockchainError> {
        let mut index = self.active_provider_index.write().await;
        *index = (*index + 1) % self.providers.len();
        println!("Switching to RPC provider {}", *index);
        Ok(self.providers[*index].clone())
    }

    async fn call_contract_with_retry(
        &self,
        address: Address,
        data: Bytes,
        max_retries: usize,
    ) -> Result<Bytes, BlockchainError> {
        self.call_contract_with_retry_internal(address, data, 0, max_retries)
            .await
    }

    // Recursive implementation for retrying with different providers
    async fn call_contract_with_retry_internal(
        &self,
        address: Address,
        data: Bytes,
        current_retry: usize,
        max_retries: usize,
    ) -> Result<Bytes, BlockchainError> {
        if current_retry > max_retries {
            return Err(BlockchainError::ContractError(
                "Max retries exceeded".to_string(),
            ));
        }

        let provider = self.get_provider().await;
        let tx = TransactionRequest::new()
            .to(address)
            .data(data.clone())
            .from(self.wallet.address());

        match provider.call(&tx.into(), None).await {
            Ok(result) => Ok(result),
            Err(e) => {
                println!("RPC call failed: {}. Trying next provider...", e);
                // Try the next provider
                self.try_next_provider().await?;
                // Use Box::pin for recursive async call
                Box::pin(self.call_contract_with_retry_internal(
                    address,
                    data,
                    current_retry + 1,
                    max_retries,
                ))
                .await
            }
        }
    }

    #[allow(dead_code)]
    async fn call_contract(&self, address: Address, data: Bytes) -> Result<Bytes, BlockchainError> {
        // Use 3 retries by default (will try up to 4 providers)
        self.call_contract_with_retry(address, data, 3).await
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
            None => {
                // Get native ETH balance
                let provider = self.get_provider().await;
                let max_retries = 3;
                let current_retry = 0;

                // Use recursion for retries following functional programming principles
                async fn get_balance_with_retry(
                    provider: Provider<Http>,
                    address: Address,
                    current_retry: usize,
                    max_retries: usize,
                    evm_provider: &EvmProvider,
                ) -> Result<U256, BlockchainError> {
                    match provider.get_balance(address, None).await {
                        Ok(balance) => Ok(balance),
                        Err(e) => {
                            if current_retry >= max_retries {
                                return Err(BlockchainError::ProviderError(e.to_string()));
                            }
                            println!(
                                "RPC call failed for get_balance: {}. Trying next provider...",
                                e
                            );
                            // Try the next provider
                            let new_provider = evm_provider.try_next_provider().await?;
                            // Use Box::pin for recursive async call
                            Box::pin(get_balance_with_retry(
                                new_provider,
                                address,
                                current_retry + 1,
                                max_retries,
                                evm_provider,
                            ))
                            .await
                        }
                    }
                }

                get_balance_with_retry(provider, address, current_retry, max_retries, self).await
            }
            Some(token_address) => {
                // Get ERC20 token balance
                let function_signature = "balanceOf(address)";
                let selector = &keccak256(function_signature.as_bytes())[0..4];

                let params = ethers::abi::encode(&[ethers::abi::Token::Address(address)]);
                let data = [selector, &params[..]].concat();

                let result = self
                    .call_contract_with_retry(token_address, data.into(), 3)
                    .await?;

                if result.len() < 32 {
                    return Err(BlockchainError::ContractError(
                        "Invalid response length for token balance".to_string(),
                    ));
                }

                Ok(U256::from_big_endian(&result[..32]))
            }
        }
    }

    async fn get_token_value(
        &self,
        _token_address: Address,
        amount: U256,
    ) -> Result<U256, BlockchainError> {
        // For now, just return the amount as the value
        // In a real implementation, this would fetch the token price from an oracle
        Ok(amount)
    }

    fn get_wallet_address(&self) -> Address {
        self.wallet.address()
    }
}
