// Re-export SafeContext from mfm_machine
pub use mfm_machine::state::safe_context::{create_default_safe_context, SafeContext};

pub mod blockchain;
pub mod cli;
pub mod config;
pub mod portfolio;

pub mod contexts;
pub mod operations;
pub mod states;

// Re-export types
pub use blockchain::{
    cow_swap::CowSwapProvider,
    dex::{DexError, DexProvider},
    evm::{BlockchainError, BlockchainProvider},
    EvmProvider,
};

pub use cli::{Cli, CliContext, Commands};
pub use portfolio::{Portfolio, PortfolioOperation, PortfolioState, PortfolioStatus, TokenBalance};
pub use states::*;

#[cfg(test)]
pub mod tests {
    mod encryption_tests;
    pub mod utils;
}
