use ethers::{
    providers::{Http, Middleware, Provider},
    signers::{LocalWallet, Signer},
    types::{Address, Bytes, TransactionRequest, U256},
    utils::keccak256,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;
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
    #[error("Other error: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    pub rpc_url: String,
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
    provider: Provider<Http>,
    wallet: LocalWallet,
}

impl EvmProvider {
    pub fn new(config: ChainConfig, private_key: String) -> Result<Self, BlockchainError> {
        let provider =
            Provider::new(Http::new(Url::parse(&config.rpc_url).map_err(|e| {
                BlockchainError::Other(format!("Invalid RPC URL: {}", e))
            })?));

        let wallet = LocalWallet::from_str(&private_key)
            .map_err(|e| BlockchainError::WalletError(e.to_string()))?
            .with_chain_id(config.chain_id);

        Ok(Self { provider, wallet })
    }

    async fn call_contract(&self, address: Address, data: Bytes) -> Result<Bytes, BlockchainError> {
        let tx = TransactionRequest::new()
            .to(address)
            .data(data)
            .from(self.wallet.address());

        self.provider
            .call(&tx.into(), None)
            .await
            .map_err(|e| BlockchainError::ContractError(e.to_string()))
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
                self.provider
                    .get_balance(address, None)
                    .await
                    .map_err(|e| BlockchainError::ProviderError(e.to_string()))
            }
            Some(token_address) => {
                // Get ERC20 token balance
                let function = "balanceOf(address)";
                let selector = &keccak256(function.as_bytes())[0..4];
                let address_padded = ethers::abi::encode(&[ethers::abi::Token::Address(address)]);
                let data = [selector, &address_padded].concat();

                let result = self.call_contract(token_address, data.into()).await?;

                if result.len() < 32 {
                    return Err(BlockchainError::ContractError(
                        "Invalid response length".to_string(),
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
