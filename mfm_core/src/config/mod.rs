use serde_derive::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub mod authentication;
pub mod dexes;
pub mod network;
pub mod token;

use alloy_primitives::Address;
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
    #[serde(default)]
    pub rpc_urls: Vec<String>,
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
    pub chain_id: u64,
    pub name: String,
}

fn default_rpc_url() -> String {
    "".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WalletConfig {
    pub address: Address,
    #[serde(skip_serializing)]
    pub private_key_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DexConfig {
    pub provider: String,
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
        if let Err(errors) = config.validate() {
            let message = format!("config validation failed:\n{}", errors.join("\n"));
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message,
            )));
        }
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // C1: Cross-reference validation
        if self.dexes.get(&self.dex.provider).is_none() {
            errors.push(format!(
                "dex.provider {:?} not found in dexes",
                self.dex.provider
            ));
        }

        // C1: Token networks must reference a known network id
        for (token_id, token) in self.tokens.hashmap() {
            for (token_network_id, token_network) in token.networks.hashmap() {
                if self.networks.get(&token_network.network_id).is_none() {
                    errors.push(format!(
                        "tokens.{token_id}.networks.{token_network_id}.network_id {:?} not found in networks",
                        token_network.network_id
                    ));
                }
            }
        }

        // C1: Dex entries must reference a known network id
        for (dex_id, dex) in self.dexes.hashmap() {
            if self.networks.get(&dex.network_id).is_none() {
                errors.push(format!(
                    "dexes.{dex_id}.network_id {:?} not found in networks",
                    dex.network_id
                ));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
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
