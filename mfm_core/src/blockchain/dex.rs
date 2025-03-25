use async_trait::async_trait;
use ethers::{
    prelude::*,
    providers::{Http, Provider},
    signers::{LocalWallet, Signer as EthersSigner},
    types::{Address, H256, U256},
};
use hex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use thiserror::Error;

use crate::blockchain::cow_swap::api::CowSwapApiClient;

// ERC20 ABI for token approval
abigen!(
    IERC20,
    r#"[
        function approve(address spender, uint256 amount) external returns (bool)
        function allowance(address owner, address spender) external view returns (uint256)
        function balanceOf(address account) external view returns (uint256)
    ]"#
);

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
    #[error("Token approval error: {0}")]
    TokenApprovalError(String),
    #[error("No signer configured")]
    NoSignerConfigured,
    #[error("Missing WETH address")]
    MissingWethAddress,
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

#[async_trait]
pub trait DexProvider: fmt::Debug + Send + Sync {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError>;

    async fn execute_swap(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        exact_approval: bool,
    ) -> Result<H256, DexError>;

    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError>;
}

#[derive(Debug, Clone)]
pub struct CowSwapProvider {
    provider: Provider<Http>,
    chain_id: u64,
    signer: Option<LocalWallet>,
    api_client: CowSwapApiClient,
    settlement: Address,
}

impl CowSwapProvider {
    pub fn new(provider: Provider<Http>, chain_id: u64) -> Self {
        Self {
            provider,
            chain_id,
            signer: None,
            api_client: CowSwapApiClient::new(),
            settlement: "0x9008D19f58AAbD9eD0D60971565AA8510560ab41"
                .parse()
                .unwrap(),
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
        from_token: Address,
        to_token: Address,
        amount: U256,
        exact_approval: bool,
    ) -> Result<H256, DexError> {
        if let Some(signer) = &self.signer {
            let cow_swap = crate::blockchain::cow_swap::CowSwapProvider::new(
                Arc::new(self.provider.clone()),
                self.chain_id,
                Some(signer.clone()),
            );

            // Get quote for minimum amount out
            let quote = self.get_quote(from_token, to_token, amount).await?;

            let order = cow_swap
                .create_order(
                    quote.from_token,
                    quote.to_token,
                    quote.from_amount,
                    quote.to_amount,
                    None,
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
                    order.valid_to as u64,
                    format!("0x{}", hex::encode(&order.app_data)),
                    order.fee_amount,
                    hex::encode(signature),
                    signer.address(),
                )
                .await
                .map_err(|e| DexError::SwapError(e.to_string()))?;

            // Convert order_uid to H256
            let order_uid = response
                .order_uid
                .strip_prefix("0x")
                .unwrap_or(&response.order_uid);
            let bytes = hex::decode(order_uid).map_err(|e| DexError::SwapError(e.to_string()))?;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&bytes);
            Ok(H256::from(hash))
        } else {
            Err(DexError::SwapError("No signer configured".to_string()))
        }
    }

    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError> {
        let signer = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        let token_contract = IERC20::new(
            token,
            Arc::new(SignerMiddleware::new(
                Arc::new(self.provider.clone()),
                signer.clone(),
            )),
        );

        let current_allowance = token_contract
            .allowance(signer.address(), self.settlement)
            .call()
            .await
            .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

        if current_allowance < amount {
            let approve_call = token_contract.approve(self.settlement, amount);
            let pending_tx = approve_call
                .send()
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

            let _receipt = pending_tx
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;
        }

        Ok(())
    }
}
