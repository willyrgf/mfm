//! Workspace-wide YAML configuration models.
//!
//! This module groups the static registries (`networks`, `tokens`, `dexes`, and
//! `auth_methods`) with the runtime selections that higher layers use after configuration is
//! loaded and validated.
//!
//! # Examples
//!
//! ```rust
//! use mfm_core::config::Config;
//!
//! let raw = r#"
//! networks:
//!   mainnet:
//!     name: Ethereum
//!     kind: evm
//!     symbol: ETH
//!     decimals: 18
//!     chain_id: 1
//!     node_url_http: https://example.invalid
//!     node_url_grpc: null
//!     blockexplorer_url: null
//!     min_balance_coin: "0.1"
//!     wrapped_token: weth
//! dexes:
//!   uniswap:
//!     name: Uniswap
//!     kind: uniswap_v3
//!     router_address: null
//!     factory_address: null
//!     network_id: mainnet
//!     settlement_contract: null
//!     api_url: null
//! tokens:
//!   weth:
//!     networks:
//!       mainnet:
//!         name: Wrapped Ether
//!         kind: erc20
//!         network_id: mainnet
//!         address: "0x0000000000000000000000000000000000000000"
//!         slippage: "0.50"
//!         path_token: weth
//!         decimals: 18
//! auth_methods:
//!   - method: meta_mask
//! network:
//!   rpc_urls: []
//!   rpc_url: https://example.invalid
//!   chain_id: 1
//!   name: Ethereum
//! wallet:
//!   address: "0x0000000000000000000000000000000000000000"
//!   private_key_path: /tmp/example.key
//! dex:
//!   provider: uniswap
//! "#;
//!
//! let config: Config = serde_yaml::from_str(raw)?;
//! config.validate().expect("cross references should be valid");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Authentication-method configuration entries.
pub mod authentication;
/// DEX registry configuration entries.
pub mod dexes;
/// Network registry configuration entries.
pub mod network;
/// Token registry configuration entries.
pub mod token;

use alloy_primitives::Address;
use dexes::Dexes;
use network::Networks;
use token::Tokens;

use self::authentication::Methods;

/// Top-level MFM configuration loaded from YAML.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// Named network definitions keyed by logical identifier.
    pub networks: Networks,
    /// Named DEX definitions keyed by logical identifier.
    pub dexes: Dexes,
    /// Named token definitions keyed by logical identifier.
    pub tokens: Tokens,
    /// Supported authentication methods for wallet access.
    pub auth_methods: Methods,
    /// Active network selection for the current run.
    pub network: NetworkConfig,
    /// Active wallet selection for the current run.
    pub wallet: WalletConfig,
    /// Active DEX selection for the current run.
    pub dex: DexConfig,
}

/// Runtime network selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkConfig {
    /// Candidate RPC endpoints for failover-aware clients.
    #[serde(default)]
    pub rpc_urls: Vec<String>,
    /// Primary RPC endpoint used by older callers.
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
    /// EVM chain identifier for the selected network.
    pub chain_id: u64,
    /// Human-readable network name.
    pub name: String,
}

fn default_rpc_url() -> String {
    "".to_string()
}

/// Runtime wallet selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WalletConfig {
    /// Expected wallet address for the selected signer.
    pub address: Address,
    ///
    /// # Security
    ///
    /// This path points to secret-bearing material. The path itself is safe to serialize, but the
    /// file contents must never be logged or persisted outside the keystore/config loading flow.
    ///
    /// Filesystem path to a plaintext private key used by legacy flows.
    #[serde(skip_serializing)]
    pub private_key_path: PathBuf,
}

/// Runtime DEX selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DexConfig {
    /// Key of the selected DEX entry in [`Config::dexes`].
    pub provider: String,
}

/// Zeroizing wallet material loaded from configuration.
///
/// # Security
///
/// This type owns raw private-key text. Keep its lifetime short and avoid exposing the borrowed
/// contents in logs, errors, or serialized output.
pub struct SecureWallet {
    private_key: Zeroizing<String>,
}

impl SecureWallet {
    /// Wraps a private key string in a zeroizing container.
    pub fn new(private_key: String) -> Self {
        Self {
            private_key: Zeroizing::new(private_key),
        }
    }

    /// Returns the private key as a borrowed string slice.
    ///
    /// # Security
    ///
    /// The returned slice still references secret material. Borrow it only for the minimum scope
    /// needed to hand off to signing or keystore code.
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
    /// Loads, deserializes, and validates a YAML configuration file.
    ///
    /// Validation is always performed after deserialization so callers do not accidentally operate
    /// on a partially-wired registry.
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

    /// Validates cross-references across networks, tokens, and DEX definitions.
    ///
    /// The current checks ensure that:
    /// - `dex.provider` points at a configured DEX
    /// - each token-network entry points at a configured network
    /// - each DEX entry points at a configured network
    ///
    /// All discovered issues are returned together so callers can present a full config report
    /// instead of failing on only the first mismatch.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mfm_core::config::Config;
    ///
    /// let raw = r#"
    /// networks: {}
    /// dexes: {}
    /// tokens: {}
    /// auth_methods:
    ///   - method: meta_mask
    /// network:
    ///   rpc_urls: []
    ///   rpc_url: https://example.invalid
    ///   chain_id: 1
    ///   name: Ethereum
    /// wallet:
    ///   address: "0x0000000000000000000000000000000000000000"
    ///   private_key_path: /tmp/example.key
    /// dex:
    ///   provider: missing
    /// "#;
    ///
    /// let config: Config = serde_yaml::from_str(raw)?;
    /// let errors = config.validate().expect_err("missing dex registry entry should fail");
    /// assert!(errors.iter().any(|msg| msg.contains("dex.provider")));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
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

    /// Loads wallet material using configured authentication methods, falling back to the legacy
    /// plaintext wallet path when necessary.
    ///
    /// Runtime callers should prefer `auth_methods` because it makes the loading strategy explicit
    /// in configuration. The fallback path exists for older configs that still rely on
    /// [`WalletConfig::private_key_path`].
    ///
    /// # Security
    ///
    /// This method reads raw private-key text from disk. The returned [`SecureWallet`] zeroizes its
    /// owned buffer on drop, but callers must still avoid logging or serializing the secret.
    ///
    /// # Errors
    ///
    /// Returns an error if no configured wallet source can be read successfully.
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
