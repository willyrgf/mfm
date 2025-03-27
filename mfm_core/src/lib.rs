use std::path::Path;
use std::{fs::File, io::Read};

use anyhow::Error;
use serde::de::DeserializeOwned;

// Re-export SafeContext from mfm_machine
pub use mfm_machine::state::safe_context::{create_default_safe_context, SafeContext};

pub mod blockchain;
pub mod cli;
pub mod config;
pub mod portfolio;

pub mod contexts;
#[path = "old_safe_context.rs"]
mod old_safe_context;
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

fn read_yaml<T: DeserializeOwned>(path: String) -> Result<T, Error> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    let instance: T = serde_yaml::from_str(&contents)?;
    Ok(instance)
}

#[cfg(test)]
pub mod tests {
    mod encryption_tests;
    pub mod utils;
}
