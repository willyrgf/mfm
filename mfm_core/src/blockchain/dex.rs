use ethers::{
    abi::parse_abi,
    contract::Contract,
    providers::{Http, Provider},
    types::{Address, U256},
};
use hex;
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};
use thiserror::Error;

use crate::blockchain::abi;

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
    provider: Arc<Provider<Http>>,
    router_address: Address,
    pool_fee: u32,
}

impl UniswapV3Provider {
    pub fn new(provider: Provider<Http>) -> Self {
        Self {
            provider: Arc::new(provider),
            router_address: "0xE592427A0AEce92De3Edee1F18E0157C05861564"
                .parse()
                .unwrap(), // Default Uniswap V3 Router
            pool_fee: 3000, // Default 0.3% fee
        }
    }

    pub fn with_config(mut self, router_address: Address, pool_fee: u32) -> Self {
        self.router_address = router_address;
        self.pool_fee = pool_fee;
        self
    }
}

#[async_trait::async_trait]
impl DexProvider for UniswapV3Provider {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError> {
        let contract = Contract::new(
            self.router_address,
            parse_abi(&[abi::uniswap_v3::UNISWAP_V3_ROUTER_ABI])
                .map_err(|e| DexError::ContractError(e.to_string()))?,
            self.provider.clone(),
        );

        let amount_out: U256 = contract
            .method(
                "quoteExactInputSingle",
                (from_token, to_token, self.pool_fee, amount),
            )
            .unwrap()
            .call()
            .await
            .map_err(|e| DexError::ContractError(e.to_string()))?;

        // Calculate price impact
        let price_impact = if amount.is_zero() {
            0.0
        } else {
            let expected_amount = amount_out;
            let actual_amount = amount_out;
            ((expected_amount.as_u128() as f64 - actual_amount.as_u128() as f64)
                / expected_amount.as_u128() as f64)
                * 100.0
        };

        Ok(SwapQuote {
            from_token,
            to_token,
            from_amount: amount,
            to_amount: amount_out,
            price_impact,
            route: vec![from_token, to_token],
        })
    }

    async fn execute_swap(
        &self,
        quote: SwapQuote,
        wallet_address: Address,
        min_amount_out: U256,
    ) -> Result<String, DexError> {
        let contract = Contract::new(
            self.router_address,
            parse_abi(&[abi::uniswap_v3::UNISWAP_V3_ROUTER_ABI])
                .map_err(|e| DexError::ContractError(e.to_string()))?,
            self.provider.clone(),
        );

        let deadline = U256::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 1800, // 30 minutes
        );

        let params = abi::uniswap_v3::ExactInputSingleParams {
            token_in: quote.from_token,
            token_out: quote.to_token,
            fee: self.pool_fee,
            recipient: wallet_address,
            deadline,
            amount_in: quote.from_amount,
            amount_out_minimum: min_amount_out,
            sqrt_price_limit_x96: U256::zero(),
        };

        let method = contract
            .method::<_, U256>("exactInputSingle", (params,))
            .unwrap();

        let tx = method
            .send()
            .await
            .map_err(|e| DexError::ContractError(e.to_string()))?;

        Ok(format!("{:?}", tx.tx_hash()))
    }
}

#[derive(Debug, Clone)]
pub struct CowSwapProvider {
    provider: Provider<Http>,
    chain_id: u64,
}

impl CowSwapProvider {
    pub fn new(provider: Provider<Http>) -> Self {
        Self {
            provider: provider.clone(),
            chain_id: 1, // Default to Ethereum mainnet
        }
    }

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
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
        let cow_swap =
            crate::blockchain::cow_swap::CowSwapProvider::new(self.provider.clone(), self.chain_id);
        let quote = cow_swap
            .api_client
            .get_quote(from_token, to_token, amount, false)
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
    }

    async fn execute_swap(
        &self,
        quote: SwapQuote,
        wallet_address: Address,
        min_amount_out: U256,
    ) -> Result<String, DexError> {
        let cow_swap =
            crate::blockchain::cow_swap::CowSwapProvider::new(self.provider.clone(), self.chain_id);

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

        let response = cow_swap
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
    }
}
