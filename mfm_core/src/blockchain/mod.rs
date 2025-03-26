use ethers::types::Address;
use serde::{Deserialize, Serialize};

pub mod aave;
pub mod abi;
pub mod cow_swap;
pub mod dex;
pub mod evm;
pub mod uniswap_v3;

pub use aave::{create_aave_provider, AaveProvider};
pub use dex::{DexError, DexProvider};
pub use evm::{BlockchainError, BlockchainProvider, ChainConfig, EvmProvider};
pub use uniswap_v3::UniswapV3Provider;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexConfig {
    pub kind: String,
    pub provider: String,
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
