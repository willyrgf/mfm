use std::{fs::File, io::Read};

pub mod blockchain;
pub mod cli;
pub mod config;
pub mod portfolio;

pub use blockchain::{
    cow_swap::CowSwapProvider,
    dex::{DexError, DexProvider},
    evm::{BlockchainError, BlockchainProvider},
    EvmProvider,
};

pub mod contexts;
pub mod operations;
pub mod states;

use anyhow::Error;
use serde::de::DeserializeOwned;

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
mod tests {
    mod encryption_tests;
}
