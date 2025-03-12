use ethers::types::Address;
use serde_derive::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub mod authentication;
pub mod dexes;
pub mod network;
pub mod token;

use dexes::Dexes;
use network::Networks;
use token::Tokens;

use self::authentication::Methods;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    pub networks: Networks,
    pub dexes: Dexes,
    pub tokens: Tokens,
    pub auth_methods: Methods,
    pub network: NetworkConfig,
    pub wallet: WalletConfig,
    pub dex: DexConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkConfig {
    pub rpc_url: String,
    pub chain_id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WalletConfig {
    pub address: Address,
    #[serde(skip_serializing)]
    pub private_key_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DexConfig {
    pub uniswap_v3: UniswapV3Config,
    pub cowswap: CowSwapConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UniswapV3Config {
    pub router_address: Address,
    pub pool_fee: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CowSwapConfig {
    pub settlement_contract: Address,
    pub api_url: String,
}

pub struct SecureWallet {
    private_key: Zeroizing<String>,
}

impl SecureWallet {
    pub fn new(private_key: String) -> Self {
        Self {
            private_key: Zeroizing::new(private_key),
        }
    }

    pub fn get_private_key(&self) -> &str {
        &self.private_key
    }
}

fn expand_path(path: &Path) -> PathBuf {
    if let Some(path_str) = path.to_str() {
        if path_str.starts_with('~') {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(&path_str[2..]);
            }
        }
    }
    path.to_path_buf()
}

impl Config {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let config: Config = serde_yaml::from_str(&std::fs::read_to_string(path)?)?;
        Ok(config)
    }

    pub fn load_wallet(
        &self,
        password: Option<&str>,
    ) -> Result<SecureWallet, Box<dyn std::error::Error>> {
        // First try to get the wallet from auth_methods
        for method in self.auth_methods.get_methods() {
            if let authentication::Method::Wallet(wallet) = method {
                let private_key = wallet.read_private_key(password)?;
                return Ok(SecureWallet::new(private_key));
            }
        }

        // Fallback to the old wallet config
        let expanded_path = expand_path(&self.wallet.private_key_path);
        let private_key = std::fs::read_to_string(expanded_path)?;
        Ok(SecureWallet::new(private_key.trim().to_string()))
    }
}
