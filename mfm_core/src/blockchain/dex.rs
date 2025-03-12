use ethers::{
    providers::{Http, Provider},
    types::{Address, U256},
};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DexError {
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Contract error: {0}")]
    ContractError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
    #[error("Quote error: {0}")]
    QuoteError(String),
    #[error("Swap error: {0}")]
    SwapError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuote {
    pub from_token: Address,
    pub to_token: Address,
    pub from_amount: U256,
    pub to_amount: U256,
    pub price_impact: f64,
    pub route: Vec<Address>,
}

#[async_trait::async_trait]
pub trait DexProvider: fmt::Debug + Send + Sync {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError>;
    async fn execute_swap(
        &self,
        quote: SwapQuote,
        wallet_address: Address,
        min_amount_out: U256,
    ) -> Result<String, DexError>;
}

#[derive(Debug, Clone)]
pub struct UniswapV3Provider {
    provider: Provider<Http>,
}

impl UniswapV3Provider {
    pub fn new(provider: Provider<Http>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl DexProvider for UniswapV3Provider {
    async fn get_quote(
        &self,
        _from_token: Address,
        _to_token: Address,
        _amount: U256,
    ) -> Result<SwapQuote, DexError> {
        // TODO: Implement Uniswap V3 quote
        Err(DexError::QuoteError("Not implemented".to_string()))
    }

    async fn execute_swap(
        &self,
        _quote: SwapQuote,
        _wallet_address: Address,
        _min_amount_out: U256,
    ) -> Result<String, DexError> {
        // TODO: Implement Uniswap V3 swap
        Err(DexError::SwapError("Not implemented".to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct CowSwapProvider {
    provider: Provider<Http>,
}

impl CowSwapProvider {
    pub fn new(provider: Provider<Http>) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl DexProvider for CowSwapProvider {
    async fn get_quote(
        &self,
        _from_token: Address,
        _to_token: Address,
        _amount: U256,
    ) -> Result<SwapQuote, DexError> {
        // TODO: Implement CowSwap quote
        Err(DexError::QuoteError("Not implemented".to_string()))
    }

    async fn execute_swap(
        &self,
        _quote: SwapQuote,
        _wallet_address: Address,
        _min_amount_out: U256,
    ) -> Result<String, DexError> {
        // TODO: Implement CowSwap swap
        Err(DexError::SwapError("Not implemented".to_string()))
    }
}
