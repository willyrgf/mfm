// Re-export SafeContext from mfm_machine
pub use mfm_machine::state::safe_context::{create_default_safe_context, SafeContext};

pub mod blockchain;
pub mod config;
pub mod keystore; // new module for keystore management
                  // pub mod cli; // Moved to mfm_cli
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

// pub use cli::{Cli, CliContext, Commands}; // Moved to mfm_cli
pub use portfolio::{Portfolio, PortfolioOperation, PortfolioState, PortfolioStatus, TokenBalance};
pub use states::*;

#[cfg(test)]
pub mod tests {
    // encryption tests are in config/authentication/encryption.rs
    pub mod utils;
}
