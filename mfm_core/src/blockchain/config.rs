//! blockchain configuration module
use crate::blockchain::adapter::types::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// network configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Network {
    /// network name
    pub name: String,
    /// rpc url for the network
    pub rpc_url: String,
    /// list of fallback rpc urls
    #[serde(default)]
    pub rpc_urls: Vec<String>,
    /// chain id
    pub chain_id: u64,
    /// block explorer url
    pub explorer_url: String,
}

/// token network configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenNetwork {
    /// token address on this network
    pub address: String,
    /// number of decimals for the token
    pub decimals: Option<u8>,
}

/// token configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenConfig {
    /// token symbol
    pub symbol: String,
    /// token name
    pub name: String,
    /// token logo url
    pub logo_url: Option<String>,
    /// coingecko id for price data
    pub coingecko_id: Option<String>,
    /// token addresses per network
    pub networks: HashMap<String, TokenNetwork>,
    /// is this token enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// target allocation for this token in the portfolio
    pub target_allocation: Option<f64>,
}

/// The type of DEX to use
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum DexType {
    /// CowSwap (0x protocol)
    CowSwap,
    /// Uniswap V2
    UniswapV2,
    /// Uniswap V3
    UniswapV3,
    /// SushiSwap
    SushiSwap,
}

/// dex configuration
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DexConfig {
    /// name of the dex
    pub name: String,
    /// type of dex
    pub dex_type: DexType,
    /// router address
    pub router_address: Option<String>,
    /// factory address
    pub factory_address: Option<String>,
    /// settlement contract address (for CowSwap)
    pub settlement_contract: Option<String>,
}

/// blockchain configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// network configuration
    pub network: Network,
    /// dex configuration
    pub dex: DexConfig,
    /// tokens configuration
    pub tokens: HashMap<String, TokenConfig>,
    /// wallet addresses and configurations
    pub wallets: HashMap<String, String>,
}

/// helper function to return default value for enabled
fn default_enabled() -> bool {
    true
}

impl Config {
    /// get an address from the configuration by symbol
    pub fn get_token_address(&self, symbol: &str) -> Option<Address> {
        self.tokens
            .get(symbol)
            .and_then(|token| token.networks.get(&self.network.name))
            .and_then(|network| Address::from_str(&network.address).ok())
    }

    /// get token decimals for a symbol
    pub fn get_token_decimals(&self, symbol: &str) -> Option<u8> {
        self.tokens
            .get(symbol)
            .and_then(|token| token.networks.get(&self.network.name))
            .and_then(|network| network.decimals)
            .or(Some(18)) // default to 18 decimals if not specified
    }
}
