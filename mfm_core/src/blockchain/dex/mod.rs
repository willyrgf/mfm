//! DEX module for decentralized exchange interactions
//! Currently being migrated from ethers to alloy

use crate::blockchain::adapter::types::{Address, U256};
use crate::blockchain::cow_swap::H256;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// error type for dex operations
#[derive(Debug, Error)]
pub enum DexError {
    /// error getting a quote
    #[error("Quote error: {0}")]
    QuoteError(String),
    /// error executing a swap
    #[error("Swap error: {0}")]
    SwapError(String),
    /// error with approving a token
    #[error("Token approval error: {0}")]
    TokenApprovalError(String),
    /// no signer configured
    #[error("No signer configured")]
    NoSignerConfigured,
    /// other error
    #[error("Other error: {0}")]
    Other(String),
}

/// swap quote information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapQuote {
    /// token being swapped from
    pub from_token: Address,
    /// token being swapped to
    pub to_token: Address,
    /// amount of from_token to swap
    pub from_amount: U256,
    /// amount of to_token expected to receive
    pub to_amount: U256,
    /// price impact percentage of this swap
    pub price_impact: f64,
    /// route of the swap
    pub route: Vec<Address>,
}

/// trait for dex interactions
#[async_trait]
pub trait DexProvider: fmt::Debug + Send + Sync {
    /// get a quote for a swap
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError>;

    /// execute a swap
    async fn execute_swap(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        exact_approval: bool,
    ) -> Result<H256, DexError>;

    /// check and approve token for swapping
    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError>;
}

// export the factory module
pub mod factory;
