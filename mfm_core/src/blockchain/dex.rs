use ethers::{
    providers::{Http, Provider},
    signers::{LocalWallet, Signer as EthersSigner},
    types::{Address, U256},
};
use hex;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

use crate::blockchain::cow_swap::api::CowSwapApiClient;

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
    #[error("API error: {0}")]
    ApiError(String),
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
    chain_id: u64,
    signer: Option<LocalWallet>,
    api_client: CowSwapApiClient,
}

impl CowSwapProvider {
    pub fn new(provider: Provider<Http>, chain_id: u64) -> Self {
        Self {
            provider,
            chain_id,
            signer: None,
            api_client: CowSwapApiClient::new(),
        }
    }

    pub fn with_signer(mut self, signer: LocalWallet) -> Self {
        self.signer = Some(signer);
        self
    }
}

#[async_trait::async_trait]
impl DexProvider for CowSwapProvider {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError> {
        if let Some(signer) = &self.signer {
            let quote = self
                .api_client
                .get_quote(from_token, to_token, amount, signer.address(), false)
                .await
                .map_err(|e| DexError::ApiError(e.to_string()))?;

            let to_amount = U256::from_str_radix(&quote.quote.buy_amount[2..], 16)
                .map_err(|e| DexError::QuoteError(e.to_string()))?;

            // Calculate price impact
            let price_impact = if amount.is_zero() {
                0.0
            } else {
                let expected_amount = to_amount;
                let actual_amount = to_amount;
                ((expected_amount.as_u128() as f64 - actual_amount.as_u128() as f64)
                    / expected_amount.as_u128() as f64)
                    * 100.0
            };

            Ok(SwapQuote {
                from_token,
                to_token,
                from_amount: amount,
                to_amount,
                price_impact,
                route: vec![from_token, to_token],
            })
        } else {
            Err(DexError::SwapError("No signer configured".to_string()))
        }
    }

    async fn execute_swap(
        &self,
        quote: SwapQuote,
        wallet_address: Address,
        min_amount_out: U256,
    ) -> Result<String, DexError> {
        if let Some(signer) = &self.signer {
            let cow_swap = crate::blockchain::cow_swap::CowSwapProvider::new(
                self.provider.clone(),
                self.chain_id,
            )
            .with_signer(signer.clone());

            let order = cow_swap
                .create_order(
                    quote.from_token,
                    quote.to_token,
                    quote.from_amount,
                    min_amount_out,
                    Some(wallet_address),
                    None,
                )
                .await
                .map_err(|e| DexError::SwapError(e.to_string()))?;

            let signature = cow_swap
                .sign_order(&order)
                .await
                .map_err(|e| DexError::SwapError(e.to_string()))?;

            let response = self
                .api_client
                .submit_order(
                    order.sell_token,
                    order.buy_token,
                    order.sell_amount,
                    order.buy_amount,
                    order.valid_to,
                    format!("0x{}", hex::encode(order.app_data)),
                    order.fee_amount,
                    signature,
                    wallet_address,
                )
                .await
                .map_err(|e| DexError::SwapError(e.to_string()))?;

            Ok(response.order_uid)
        } else {
            Err(DexError::SwapError("No signer configured".to_string()))
        }
    }
}
