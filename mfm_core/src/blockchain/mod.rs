use ethers::types::Address;

pub mod abi;
pub mod cow_swap;
pub mod dex;
pub mod evm;

pub use cow_swap::CowSwapProvider;
pub use dex::{DexError, DexProvider, SwapQuote};
pub use evm::{BlockchainError, BlockchainProvider, EvmProvider};

use ethers::{
    providers::{Http, Provider},
    signers::LocalWallet,
};
use serde::{Deserialize, Serialize};

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
