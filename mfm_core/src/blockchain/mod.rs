mod dex;

use anyhow::anyhow;
use ethers::{
    middleware::Middleware,
    providers::{Http, Provider},
    signers::{LocalWallet, Signer},
    types::{
        transaction::eip2718::TypedTransaction, Address, Bytes, NameOrAddress, TransactionRequest,
        U256,
    },
};
use hex;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;
use url;

pub use dex::{CowSwapProvider, DexError, DexProvider, SwapQuote, UniswapV3Provider};

#[derive(Debug, Error)]
pub enum BlockchainError {
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Contract error: {0}")]
    ContractError(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Invalid amount: {0}")]
    InvalidAmount(String),
    #[error("Wallet error: {0}")]
    WalletError(String),
    #[error("Overflow error")]
    Overflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    pub rpc_url: String,
    pub chain_id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    pub address: Address,
    pub private_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenConfig {
    pub address: Address,
    pub decimals: u8,
    pub symbol: String,
}

#[async_trait::async_trait]
pub trait BlockchainProvider: fmt::Debug + Send + Sync {
    async fn get_balance(
        &self,
        address: Address,
        token: Option<Address>,
    ) -> Result<U256, BlockchainError>;
    async fn get_allowance(
        &self,
        token: Address,
        owner: Address,
        spender: Address,
    ) -> Result<U256, BlockchainError>;
    async fn approve(
        &self,
        token: Address,
        spender: Address,
        amount: U256,
    ) -> Result<String, BlockchainError>;
    fn get_wallet_address(&self) -> Address;
    async fn get_token_value(&self, token: Address, amount: U256) -> Result<U256, BlockchainError>;
}

pub struct EvmProvider {
    provider: Provider<Http>,
    chain_config: ChainConfig,
    wallet: LocalWallet,
}

impl fmt::Debug for EvmProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmProvider")
            .field("chain_config", &self.chain_config)
            .finish()
    }
}

impl Clone for EvmProvider {
    fn clone(&self) -> Self {
        Self {
            provider: Provider::new(Http::new(
                url::Url::parse(&self.chain_config.rpc_url).unwrap(),
            )),
            chain_config: self.chain_config.clone(),
            wallet: self.wallet.clone(),
        }
    }
}

impl EvmProvider {
    pub fn new(chain_config: ChainConfig, private_key: &str) -> Result<Self, BlockchainError> {
        let provider = Provider::<Http>::try_from(&chain_config.rpc_url)
            .map_err(|e| BlockchainError::ProviderError(e.to_string()))?;

        let wallet = private_key
            .parse::<LocalWallet>()
            .map_err(|e| BlockchainError::WalletError(e.to_string()))?;

        Ok(Self {
            provider,
            chain_config,
            wallet,
        })
    }
}

#[async_trait::async_trait]
impl BlockchainProvider for EvmProvider {
    async fn get_balance(
        &self,
        address: Address,
        token: Option<Address>,
    ) -> Result<U256, BlockchainError> {
        match token {
            Some(token_address) => {
                eprintln!("\n=== Checking ERC20 Token Balance ===");
                eprintln!("Token address: {}", token_address);
                eprintln!("Wallet address: {}", address);

                // Function selector for balanceOf(address)
                let selector = "70a08231"; // balanceOf(address)

                // Convert address to bytes and pad to 32 bytes
                let mut padded_address = [0u8; 32];
                address
                    .to_fixed_bytes()
                    .iter()
                    .enumerate()
                    .take(20)
                    .for_each(|(i, &byte)| {
                        padded_address[i + 12] = byte;
                    });

                // Combine selector and padded address
                let mut call_data = Vec::with_capacity(36); // 4 bytes selector + 32 bytes address
                call_data.extend_from_slice(&hex::decode(selector).map_err(|e| {
                    BlockchainError::ContractError(format!("Failed to decode selector: {}", e))
                })?);
                call_data.extend_from_slice(&padded_address);

                eprintln!("Call data: 0x{}", hex::encode(&call_data));
                eprintln!("Call data length: {} bytes", call_data.len());

                let tx = TransactionRequest::new()
                    .to(token_address)
                    .data(Bytes::from(call_data));

                eprintln!("Making contract call to token...");
                let result = self.provider.call(&tx.into(), None).await.map_err(|e| {
                    BlockchainError::ContractError(format!("Contract call failed: {}", e))
                })?;

                eprintln!("Raw response: 0x{}", hex::encode(&result));
                eprintln!("Response length: {} bytes", result.len());

                if result.len() < 32 {
                    return Err(BlockchainError::ContractError(format!(
                        "Invalid response length from contract: {} bytes",
                        result.len()
                    )));
                }

                let balance = U256::from_big_endian(&result[..32]);
                eprintln!("Decoded balance: {}", balance);
                eprintln!("=== End of ERC20 Balance Check ===\n");

                Ok(balance)
            }
            None => {
                eprintln!("\n=== Checking Native ETH Balance ===");
                eprintln!("Wallet address: {}", address);

                let balance = self
                    .provider
                    .get_balance(NameOrAddress::Address(address), None)
                    .await
                    .map_err(|e| BlockchainError::ProviderError(e.to_string()))?;

                eprintln!("Native ETH balance: {}", balance);
                eprintln!("=== End of Native Balance Check ===\n");

                Ok(balance)
            }
        }
    }

    async fn get_allowance(
        &self,
        _token: Address,
        _owner: Address,
        _spender: Address,
    ) -> Result<U256, BlockchainError> {
        // TODO: Implement ERC20 allowance check
        Err(BlockchainError::ContractError(
            "Not implemented".to_string(),
        ))
    }

    async fn approve(
        &self,
        _token: Address,
        _spender: Address,
        _amount: U256,
    ) -> Result<String, BlockchainError> {
        // TODO: Implement ERC20 approve
        Err(BlockchainError::ContractError(
            "Not implemented".to_string(),
        ))
    }

    fn get_wallet_address(&self) -> Address {
        self.wallet.address()
    }

    async fn get_token_value(
        &self,
        _token: Address,
        amount: U256,
    ) -> Result<U256, BlockchainError> {
        // TODO: Implement token value calculation using price feeds
        // For now, just return the amount as is
        Ok(amount)
    }
}
