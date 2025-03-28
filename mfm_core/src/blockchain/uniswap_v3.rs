//! Uniswap V3 module for DEX interactions
//! This file contains placeholder implementations that will be fully implemented later

use crate::blockchain::adapter::erc20::ERC20;
use crate::blockchain::adapter::{Address, Bytes, LocalWallet, Provider, U256};
use crate::blockchain::cow_swap::H256;
use crate::blockchain::dex::{DexError, DexProvider, SwapQuote};
use alloy_primitives::Address as AlloyAddress;
use async_trait::async_trait;
use std::str::FromStr;
use std::sync::Arc;

// Function selectors for Uniswap V3 contracts
// Router selectors
const EXACT_INPUT_SINGLE_SELECTOR: [u8; 4] = [0x41, 0x4b, 0xf3, 0x89]; // keccak256("exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))")[0..4]
#[allow(dead_code)]
const EXACT_OUTPUT_SINGLE_SELECTOR: [u8; 4] = [0xdb, 0x3e, 0x24, 0x84]; // keccak256("exactOutputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))")[0..4]

// Quoter selectors
const QUOTE_EXACT_INPUT_SINGLE_SELECTOR: [u8; 4] = [0xf7, 0x72, 0x9d, 0x9d]; // keccak256("quoteExactInputSingle(address,address,uint24,uint256,uint160)")[0..4]
#[allow(dead_code)]
const QUOTE_EXACT_OUTPUT_SINGLE_SELECTOR: [u8; 4] = [0x30, 0xd8, 0xe8, 0x60]; // keccak256("quoteExactOutputSingle(address,address,uint24,uint256,uint160)")[0..4]

// Factory selectors
const GET_POOL_SELECTOR: [u8; 4] = [0x1e, 0x3d, 0xd1, 0x8b]; // keccak256("getPool(address,address,uint24)")[0..4]

// Fee tiers for Uniswap V3 pools
pub const FEE_TIER_LOW: u32 = 500; // 0.05%
pub const FEE_TIER_MEDIUM: u32 = 3000; // 0.3%
pub const FEE_TIER_HIGH: u32 = 10000; // 1%

/// Uniswap V3 provider for swap operations
#[derive(Debug, Clone)]
pub struct UniswapV3Provider {
    pub provider: Arc<Provider>,
    pub chain_id: u64,
    pub weth: Address,
    pub factory: Address,
    pub router: Address,
    pub quoter: Address,
    pub signer: Option<LocalWallet>,
}

impl UniswapV3Provider {
    /// Create a new uniswap v3 provider
    pub fn new(
        provider: Arc<Provider>,
        chain_id: u64,
        signer: Option<LocalWallet>,
        weth: Option<Address>,
        factory: Option<Address>,
        router: Option<Address>,
        quoter: Option<Address>,
    ) -> Self {
        // Default to mainnet contract addresses if not provided
        // Convert from hex strings to our Address type via alloy_primitives::Address
        let default_weth =
            Address(AlloyAddress::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap());

        let default_factory =
            Address(AlloyAddress::from_str("0x1F98431c8aD98523631AE4a59f267346ea31F984").unwrap());

        let default_router =
            Address(AlloyAddress::from_str("0xE592427A0AE567641e7c23A3090590d8c169ec17").unwrap());

        let default_quoter =
            Address(AlloyAddress::from_str("0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6").unwrap());

        Self {
            provider,
            chain_id,
            weth: weth.unwrap_or(default_weth),
            factory: factory.unwrap_or(default_factory),
            router: router.unwrap_or(default_router),
            quoter: quoter.unwrap_or(default_quoter),
            signer,
        }
    }

    /// Set the signer wallet
    pub fn with_signer(mut self, signer: LocalWallet) -> Self {
        self.signer = Some(signer);
        self
    }

    /// Get the fee tier for two tokens based on liquidity
    pub async fn get_best_fee_tier(
        &self,
        token_a: Address,
        token_b: Address,
    ) -> Result<u32, DexError> {
        // Check each fee tier to find the pool with the most liquidity
        let fee_tiers = [FEE_TIER_LOW, FEE_TIER_MEDIUM, FEE_TIER_HIGH];

        for &fee in &fee_tiers {
            // Create the getPool call data
            let mut call_data = GET_POOL_SELECTOR.to_vec();

            // Add token A (padded to 32 bytes)
            let mut token_a_bytes = vec![0u8; 12]; // 12 zeros for padding
            token_a_bytes.extend_from_slice(token_a.0.as_ref());
            call_data.extend_from_slice(&token_a_bytes);

            // Add token B (padded to 32 bytes)
            let mut token_b_bytes = vec![0u8; 12]; // 12 zeros for padding
            token_b_bytes.extend_from_slice(token_b.0.as_ref());
            call_data.extend_from_slice(&token_b_bytes);

            // Add fee (padded to 32 bytes)
            let mut fee_bytes = vec![0u8; 28]; // 28 zeros for padding
            fee_bytes.extend_from_slice(&fee.to_be_bytes());
            call_data.extend_from_slice(&fee_bytes);

            // Call the factory contract
            let result = self
                .provider
                .call(self.factory, Bytes(call_data))
                .await
                .map_err(|e| DexError::Other(format!("Failed to call factory: {}", e)))?;

            // If the result is not zero address, this pool exists
            if result.0.len() >= 32 {
                let mut is_zero = true;
                for &byte in &result.0[12..32] {
                    // Skip first 12 bytes, we only need the 20-byte address
                    if byte != 0 {
                        is_zero = false;
                        break;
                    }
                }

                if !is_zero {
                    return Ok(fee);
                }
            }
        }

        // Default to medium fee tier if no pools found
        Ok(FEE_TIER_MEDIUM)
    }

    /// Get a quote for a swap
    pub async fn get_quote_internal(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<U256, DexError> {
        // Get the best fee tier for these tokens
        let fee = self.get_best_fee_tier(from_token, to_token).await?;

        // Create the quoteExactInputSingle call data
        let mut call_data = QUOTE_EXACT_INPUT_SINGLE_SELECTOR.to_vec();

        // Add from_token (padded to 32 bytes)
        let mut from_token_bytes = vec![0u8; 12]; // 12 zeros for padding
        from_token_bytes.extend_from_slice(from_token.0.as_ref());
        call_data.extend_from_slice(&from_token_bytes);

        // Add to_token (padded to 32 bytes)
        let mut to_token_bytes = vec![0u8; 12]; // 12 zeros for padding
        to_token_bytes.extend_from_slice(to_token.0.as_ref());
        call_data.extend_from_slice(&to_token_bytes);

        // Add fee (padded to 32 bytes)
        let mut fee_bytes = vec![0u8; 28]; // 28 zeros for padding
        fee_bytes.extend_from_slice(&fee.to_be_bytes());
        call_data.extend_from_slice(&fee_bytes);

        // Add amount (padded to 32 bytes)
        call_data.extend_from_slice(&amount.0.to_be_bytes::<32>());

        // Add sqrtPriceLimitX96 (set to 0 to use the best price)
        let sqrt_price_limit_bytes = vec![0u8; 32]; // All zeros
        call_data.extend_from_slice(&sqrt_price_limit_bytes);

        // Call the quoter contract
        let result = self
            .provider
            .call(self.quoter, Bytes(call_data))
            .await
            .map_err(|e| DexError::QuoteError(format!("Failed to get quote: {}", e)))?;

        // Parse the result as a uint256
        if result.0.len() < 32 {
            return Err(DexError::QuoteError("Invalid quote result length".into()));
        }

        // Extract result
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result.0[0..32]);
        let amount_out = alloy_primitives::U256::from_be_bytes(bytes);

        Ok(U256(amount_out))
    }
}

#[async_trait]
impl DexProvider for UniswapV3Provider {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError> {
        // Get quote from Uniswap V3
        let amount_out = self
            .get_quote_internal(from_token, to_token, amount)
            .await?;

        // Calculate price impact (simplified calculation)
        // In a full implementation, we'd get the market price and calculate the actual impact
        let price_impact = 0.5; // Default to 0.5% for now

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
        from_token: Address,
        to_token: Address,
        amount: U256,
        _exact_approval: bool,
    ) -> Result<H256, DexError> {
        // Check that we have a signer
        let wallet = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;

        // First, approve the router to spend our tokens
        self.check_and_approve_token(from_token, amount).await?;

        // Get the best fee tier
        let fee = self.get_best_fee_tier(from_token, to_token).await?;

        // Get the minimum amount out (with 0.5% slippage)
        let amount_out = self
            .get_quote_internal(from_token, to_token, amount)
            .await?;
        let slippage_bps = 50; // 0.5%

        // Calculate min_amount_out with slippage
        // We need to multiply by (10000 - slippage_bps) / 10000
        let slippage_factor = U256::from(10000u64 - slippage_bps);
        let divisor = U256::from(10000u64);
        let min_amount_out = U256(
            amount_out.0 * alloy_primitives::U256::from(slippage_factor.0)
                / alloy_primitives::U256::from(divisor.0),
        );

        // Create exactInputSingle params
        // struct ExactInputSingleParams {
        //     address tokenIn;
        //     address tokenOut;
        //     uint24 fee;
        //     address recipient;
        //     uint256 deadline;
        //     uint256 amountIn;
        //     uint256 amountOutMinimum;
        //     uint160 sqrtPriceLimitX96;
        // }

        // Create the call data
        let mut call_data = EXACT_INPUT_SINGLE_SELECTOR.to_vec();

        // Build struct data - need to ABI encode a tuple
        let mut struct_data = Vec::new();

        // tokenIn (padded to 32 bytes)
        let mut from_token_bytes = vec![0u8; 12]; // 12 zeros for padding
        from_token_bytes.extend_from_slice(from_token.0.as_ref());
        struct_data.extend_from_slice(&from_token_bytes);

        // tokenOut (padded to 32 bytes)
        let mut to_token_bytes = vec![0u8; 12]; // 12 zeros for padding
        to_token_bytes.extend_from_slice(to_token.0.as_ref());
        struct_data.extend_from_slice(&to_token_bytes);

        // fee (padded to 32 bytes but only using uint24)
        let mut fee_bytes = vec![0u8; 29]; // 29 zeros for padding (32 - 3 bytes for uint24)
        let fee_bytes_raw = fee.to_be_bytes();
        fee_bytes.extend_from_slice(&fee_bytes_raw[1..]); // Only use last 3 bytes
        struct_data.extend_from_slice(&fee_bytes);

        // recipient (padded to 32 bytes) - use sender's address
        let recipient = wallet.address();
        let mut recipient_bytes = vec![0u8; 12]; // 12 zeros for padding
        recipient_bytes.extend_from_slice(recipient.0.as_ref());
        struct_data.extend_from_slice(&recipient_bytes);

        // deadline (padded to 32 bytes) - use current time + 1 hour
        let deadline = U256::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600,
        ); // 1 hour from now
        struct_data.extend_from_slice(&deadline.0.to_be_bytes::<32>());

        // amountIn (padded to 32 bytes)
        struct_data.extend_from_slice(&amount.0.to_be_bytes::<32>());

        // amountOutMinimum (padded to 32 bytes)
        struct_data.extend_from_slice(&min_amount_out.0.to_be_bytes::<32>());

        // sqrtPriceLimitX96 (padded to 32 bytes) - use 0 for best price
        let sqrt_price_limit = U256::from(0u64);
        struct_data.extend_from_slice(&sqrt_price_limit.0.to_be_bytes::<32>());

        // Now add the struct to the call data
        call_data.extend_from_slice(&struct_data);

        // Send the transaction
        let tx_hash = wallet
            .send_transaction(&self.provider, Some(self.router), None, Bytes(call_data))
            .await
            .map_err(|e| DexError::SwapError(format!("Failed to execute swap: {}", e)))?;

        Ok(tx_hash)
    }

    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError> {
        let wallet = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;

        // Create ERC20 contract instance
        let token_contract = ERC20::new(token, (*self.provider).clone());

        // Check current allowance
        let allowance = token_contract
            .allowance(wallet.address(), self.router)
            .await
            .map_err(|e| {
                DexError::TokenApprovalError(format!("Failed to check allowance: {}", e))
            })?;

        // If allowance is sufficient, no need to approve again
        if allowance.0 >= amount.0 {
            return Ok(());
        }

        // Approve router to spend tokens
        let tx_hash = token_contract
            .approve(self.router, amount, wallet)
            .await
            .map_err(|e| {
                DexError::TokenApprovalError(format!("Failed to approve tokens: {}", e))
            })?;

        // Log the approval transaction
        println!(
            "Approved token {} for Uniswap V3 router: {}",
            token, tx_hash
        );

        Ok(())
    }
}
