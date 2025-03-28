//! blockchain module - ethereum blockchain integrations
pub mod aave;
pub mod abi;
pub mod adapter;
pub mod config;
pub mod cow_swap;
pub mod dex;
pub mod evm;
pub mod uniswap_v3;

pub use aave::{create_aave_provider, AaveProvider};
pub use config::{Config, DexConfig, Network, TokenConfig, TokenNetwork};
pub use cow_swap::CowSwapProvider;
pub use dex::{DexError, DexProvider, SwapQuote};
pub use evm::{BlockchainError, BlockchainProvider, ChainConfig, EvmProvider};
pub use uniswap_v3::UniswapV3Provider;

pub use adapter::erc20::ERC20;
pub use adapter::provider::Provider;
pub use adapter::signer::LocalWallet;
/// re-export adapter types for convenience
pub use adapter::types::{AdapterError, Address, Bytes, U256};
