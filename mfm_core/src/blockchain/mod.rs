use async_trait::async_trait;
use ethers::{
    providers::{Http, Provider},
    signers::LocalWallet,
    types::{Address, H256, U256},
};
use serde::{Deserialize, Serialize};

pub mod abi;
pub mod aave;
pub mod cow_swap;
pub mod dex;
pub mod evm;
pub mod uniswap_v3;
pub mod uniswap_v4;

pub use aave::{AaveEVMProvider, AaveHealthCheckResult, AaveProvider, create_aave_provider};
pub use cow_swap::CowSwapProvider;
pub use dex::{DexError, DexProvider, SwapQuote};
pub use evm::{BlockchainError, BlockchainProvider, ChainConfig, EvmProvider};
pub use uniswap_v3::UniswapV3Provider;
pub use uniswap_v4::UniswapV4Provider;

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
