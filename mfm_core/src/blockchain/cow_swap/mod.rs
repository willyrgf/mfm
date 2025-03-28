//! cow swap module
use crate::blockchain::adapter::erc20::ERC20;
use crate::blockchain::adapter::types::{Address, U256};
use crate::blockchain::dex::{DexError, DexProvider, SwapQuote};
use alloy_primitives::Address as AlloyAddress;
use async_trait::async_trait;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::blockchain::config::Config;
use crate::blockchain::cow_swap::api::{CowSwapApiClient, Quote};

/// hash type for transaction hashes
pub type H256 = String;

/// cowswap provider for swaps
#[derive(Clone)]
pub struct CowSwapProvider {
    /// provider for blockchain interactions
    pub provider: Arc<crate::blockchain::adapter::Provider>,
    /// signer for transactions
    pub signer: Option<crate::blockchain::adapter::LocalWallet>,
    /// settlement contract address
    pub settlement_contract: Address,
    /// chain id
    pub chain_id: u64,
    /// api client
    pub api_client: CowSwapApiClient,
    /// base api url
    pub api_base_url: String,
}

impl fmt::Debug for CowSwapProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CowSwapProvider")
            .field("settlement_contract", &self.settlement_contract)
            .field("signer", &self.signer.is_some())
            .field("chain_id", &self.chain_id)
            .field("api_base_url", &self.api_base_url)
            .finish_non_exhaustive()
    }
}

impl CowSwapProvider {
    /// create a new cow swap provider from config
    pub fn from_config(
        provider: Arc<crate::blockchain::adapter::Provider>,
        signer: Option<crate::blockchain::adapter::LocalWallet>,
        config: &Config,
    ) -> Result<Self, DexError> {
        // Get settlement contract address from config
        let settlement_contract = match &config.dex.settlement_contract {
            Some(addr) => Address(AlloyAddress::from_str(addr).map_err(|e| {
                DexError::Other(format!("Invalid settlement contract address: {}", e))
            })?),
            None => {
                return Err(DexError::Other(
                    "Settlement contract address not specified in config".into(),
                ))
            }
        };

        // Get chain id from config
        let chain_id = config.network.chain_id;

        // Determine API base URL based on chain id
        let api_base_url = match chain_id {
            1 => "https://api.cow.fi/mainnet/api/v1".to_string(), // Ethereum Mainnet
            5 => "https://api.cow.fi/goerli/api/v1".to_string(),  // Goerli Testnet
            100 => "https://api.cow.fi/xdai/api/v1".to_string(),  // Gnosis Chain
            _ => {
                return Err(DexError::Other(format!(
                    "Unsupported chain ID for CowSwap: {}",
                    chain_id
                )))
            }
        };

        Ok(Self {
            provider,
            signer,
            settlement_contract,
            chain_id,
            api_client: CowSwapApiClient::new(),
            api_base_url,
        })
    }

    /// create a new cow swap provider
    pub fn new(
        provider: Arc<crate::blockchain::adapter::Provider>,
        signer: Option<crate::blockchain::adapter::LocalWallet>,
        settlement_contract: Address,
        chain_id: u64,
    ) -> Self {
        // Determine API base URL based on chain id
        let api_base_url = match chain_id {
            1 => "https://api.cow.fi/mainnet/api/v1",
            5 => "https://api.cow.fi/goerli/api/v1",
            100 => "https://api.cow.fi/xdai/api/v1",
            _ => "https://api.cow.fi/mainnet/api/v1", // Default to mainnet if unknown
        }
        .to_string();

        Self {
            provider,
            signer,
            settlement_contract,
            chain_id,
            api_client: CowSwapApiClient::new(),
            api_base_url,
        }
    }

    /// set the signer
    pub fn with_signer(mut self, signer: crate::blockchain::adapter::LocalWallet) -> Self {
        self.signer = Some(signer);
        self
    }

    /// create and sign an order message using EIP-712
    async fn sign_order(
        &self,
        _from_token: Address,
        _to_token: Address,
        quote: &Quote,
    ) -> Result<String, DexError> {
        // Get wallet from signer
        let wallet = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;

        // This is a simplified signature process for now
        // A full implementation would need to:
        // 1. Create the EIP-712 domain and type data
        // 2. Hash it according to the spec
        // 3. Sign it with the wallet
        // 4. Format the signature in the expected format

        // For now, we'll just sign a dummy message with our wallet to show the process
        let message = format!(
            "CowSwap Order: {} {} to {} {} at {}",
            quote.sell_amount, quote.sell_token, quote.buy_amount, quote.buy_token, quote.valid_to
        );

        // Sign the message
        let signature = wallet
            .sign_message(message.as_bytes())
            .await
            .map_err(|e| DexError::Other(format!("Failed to sign order: {}", e)))?;

        // Format the signature as a hex string with 0x prefix
        Ok(format!("0x{}", hex::encode(signature)))
    }
}

#[async_trait]
impl DexProvider for CowSwapProvider {
    /// get a quote from cow swap
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError> {
        // Check for signer
        let wallet = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        let wallet_address = wallet.address();

        // Get quote from CowSwap API using the configured base URL
        let quote_response = self
            .api_client
            .get_quote_with_base_url(
                &self.api_base_url,
                from_token,
                to_token,
                amount,
                wallet_address,
                true,
            )
            .await
            .map_err(|e| DexError::QuoteError(format!("Failed to get quote: {}", e)))?;

        // Parse the amounts from the quote
        let buy_amount = quote_response.quote.buy_amount.clone();

        // Remove 0x prefix and parse as U256
        let to_amount = if let Some(stripped) = buy_amount.strip_prefix("0x") {
            let bytes = hex::decode(stripped)
                .map_err(|e| DexError::QuoteError(format!("Failed to decode buy amount: {}", e)))?;

            // Parse bytes as U256
            if bytes.len() > 32 {
                return Err(DexError::QuoteError("Buy amount too large".into()));
            }

            let mut padded_bytes = [0u8; 32];
            padded_bytes[32 - bytes.len()..].copy_from_slice(&bytes);
            U256(alloy_primitives::U256::from_be_bytes(padded_bytes))
        } else {
            // Try to parse as decimal
            match buy_amount.parse::<u128>() {
                Ok(value) => U256::from(alloy_primitives::U256::from(value)),
                Err(_) => return Err(DexError::QuoteError("Invalid buy amount format".into())),
            }
        };

        // Calculate price impact
        // For CoW, the price impact is generally very low due to batch auctions
        // We'll use a fixed value for now
        let price_impact = 0.1; // 0.1% as a placeholder

        Ok(SwapQuote {
            from_token,
            to_token,
            from_amount: amount,
            to_amount,
            price_impact,
            route: vec![from_token, to_token], // CowSwap doesn't have routes like Uniswap
        })
    }

    /// execute a swap through cow swap
    async fn execute_swap(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        _exact_approval: bool,
    ) -> Result<H256, DexError> {
        // Check for signer
        let wallet = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        let wallet_address = wallet.address();

        // First, approve the settlement contract to spend our tokens
        self.check_and_approve_token(from_token, amount).await?;

        // Get a quote using the configured base URL
        let quote_response = self
            .api_client
            .get_quote_with_base_url(
                &self.api_base_url,
                from_token,
                to_token,
                amount,
                wallet_address,
                true,
            )
            .await
            .map_err(|e| DexError::QuoteError(format!("Failed to get quote: {}", e)))?;

        // Sign the order
        let signature = self
            .sign_order(from_token, to_token, &quote_response.quote)
            .await?;

        // Valid until timestamp (30 minutes by default)
        let valid_to = quote_response.quote.valid_to;

        // Fee amount from quote
        let fee_amount_str = quote_response.quote.fee_amount.clone();
        let fee_amount = if let Some(stripped) = fee_amount_str.strip_prefix("0x") {
            let bytes = hex::decode(stripped)
                .map_err(|e| DexError::QuoteError(format!("Failed to decode fee amount: {}", e)))?;

            let mut padded_bytes = [0u8; 32];
            padded_bytes[32 - bytes.len()..].copy_from_slice(&bytes);
            U256(alloy_primitives::U256::from_be_bytes(padded_bytes))
        } else {
            // Try to parse as decimal
            match fee_amount_str.parse::<u128>() {
                Ok(value) => U256::from(alloy_primitives::U256::from(value)),
                Err(_) => return Err(DexError::QuoteError("Invalid fee amount format".into())),
            }
        };

        // Buy amount from quote
        let buy_amount_str = quote_response.quote.buy_amount.clone();
        let buy_amount = if let Some(stripped) = buy_amount_str.strip_prefix("0x") {
            let bytes = hex::decode(stripped)
                .map_err(|e| DexError::QuoteError(format!("Failed to decode buy amount: {}", e)))?;

            let mut padded_bytes = [0u8; 32];
            padded_bytes[32 - bytes.len()..].copy_from_slice(&bytes);
            U256(alloy_primitives::U256::from_be_bytes(padded_bytes))
        } else {
            // Try to parse as decimal
            match buy_amount_str.parse::<u128>() {
                Ok(value) => U256::from(alloy_primitives::U256::from(value)),
                Err(_) => return Err(DexError::QuoteError("Invalid buy amount format".into())),
            }
        };

        // App data (used for order identification)
        let app_data = quote_response.quote.app_data;

        // Submit the order using the configured base URL
        let response = self
            .api_client
            .submit_order_with_base_url(
                &self.api_base_url,
                api::OrderParams {
                    from_token,
                    to_token,
                    sell_amount: amount,
                    buy_amount,
                    valid_to,
                    app_data,
                    fee_amount,
                    signature,
                    from_address: wallet_address,
                },
            )
            .await
            .map_err(|e| DexError::SwapError(format!("Failed to submit order: {}", e)))?;

        // Return the order UID as the transaction hash
        Ok(response.order_uid)
    }

    /// check approval and approve tokens
    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError> {
        let wallet = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;

        // Create ERC20 contract instance
        let token_contract = ERC20::new(token, (*self.provider).clone());

        // Check current allowance
        let allowance = token_contract
            .allowance(wallet.address(), self.settlement_contract)
            .await
            .map_err(|e| {
                DexError::TokenApprovalError(format!("Failed to check allowance: {}", e))
            })?;

        // If allowance is sufficient, no need to approve again
        if allowance.0 >= amount.0 {
            return Ok(());
        }

        // Approve settlement contract to spend tokens
        let tx_hash = token_contract
            .approve(self.settlement_contract, amount, wallet)
            .await
            .map_err(|e| {
                DexError::TokenApprovalError(format!("Failed to approve tokens: {}", e))
            })?;

        // Log the approval transaction
        println!(
            "Approved token {} for CowSwap settlement contract: {}",
            token, tx_hash
        );

        Ok(())
    }
}

pub mod api;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockchain::adapter::Provider;
    use crate::blockchain::config::{Config, DexConfig, DexType, Network};
    use std::collections::HashMap;

    fn create_test_config() -> Config {
        Config {
            network: Network {
                name: "mainnet".to_string(),
                rpc_url: "https://eth-mainnet.example.com".to_string(),
                rpc_urls: vec![],
                chain_id: 1,
                explorer_url: "https://etherscan.io".to_string(),
            },
            dex: DexConfig {
                name: "CowSwap".to_string(),
                dex_type: DexType::CowSwap,
                router_address: None,
                factory_address: None,
                settlement_contract: Some("0xc92e8bdf79f0507f65a392b0ab4667716bfe0110".to_string()),
            },
            tokens: HashMap::new(),
            wallets: HashMap::new(),
        }
    }

    async fn create_test_provider() -> Arc<Provider> {
        let provider = Provider::connect("https://eth-mainnet.example.com")
            .await
            .expect("Failed to create test provider");
        Arc::new(provider)
    }

    #[tokio::test]
    async fn test_from_config_success() {
        let config = create_test_config();
        let provider = create_test_provider().await;

        let result = CowSwapProvider::from_config(provider, None, &config);
        assert!(
            result.is_ok(),
            "Failed to create CowSwapProvider from config"
        );

        let cow_swap = result.unwrap();
        assert_eq!(cow_swap.chain_id, 1);
        assert_eq!(cow_swap.api_base_url, "https://api.cow.fi/mainnet/api/v1");
        assert_eq!(
            cow_swap.settlement_contract.to_string().to_lowercase(),
            "0xc92e8bdf79f0507f65a392b0ab4667716bfe0110"
        );
    }

    #[tokio::test]
    async fn test_from_config_missing_settlement() {
        let mut config = create_test_config();
        config.dex.settlement_contract = None;
        let provider = create_test_provider().await;

        let result = CowSwapProvider::from_config(provider, None, &config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Settlement contract address not specified"));
    }

    #[tokio::test]
    async fn test_from_config_unsupported_chain() {
        let mut config = create_test_config();
        config.network.chain_id = 31337; // Local hardhat chain
        let provider = create_test_provider().await;

        let result = CowSwapProvider::from_config(provider, None, &config);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported chain ID"));
    }

    #[tokio::test]
    async fn test_api_url_by_chain() {
        // Test Mainnet
        let mut config = create_test_config();
        config.network.chain_id = 1;
        let provider = create_test_provider().await;

        let result = CowSwapProvider::from_config(provider.clone(), None, &config);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().api_base_url,
            "https://api.cow.fi/mainnet/api/v1"
        );

        // Test Goerli
        config.network.chain_id = 5;
        let result = CowSwapProvider::from_config(provider.clone(), None, &config);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().api_base_url,
            "https://api.cow.fi/goerli/api/v1"
        );

        // Test Gnosis Chain
        config.network.chain_id = 100;
        let result = CowSwapProvider::from_config(provider, None, &config);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().api_base_url,
            "https://api.cow.fi/xdai/api/v1"
        );
    }
}
