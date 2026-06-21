//! Static network registry configuration.
//!
//! These types describe the configured networks that token and DEX entries reference, plus helper
//! conversions for moving human-readable balance thresholds into base units.

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use super::decimal;

/// Supported network families in static configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// EVM-compatible network.
    Evm,
}

/// Static network definition keyed from [`Networks`].
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Network {
    /// Human-readable network name.
    pub name: String,
    /// Network family identifier.
    pub kind: Kind,
    /// Native coin symbol.
    pub symbol: String,
    /// Native coin decimals, when known.
    pub decimals: Option<u8>,
    /// EVM chain identifier.
    pub chain_id: u32,
    /// Preferred HTTP RPC endpoint.
    pub node_url_http: Option<String>,
    /// Preferred gRPC endpoint.
    pub node_url_grpc: Option<String>,
    /// Optional block explorer base URL.
    pub blockexplorer_url: Option<String>,
    /// Minimum balance threshold expressed in whole coins.
    pub min_balance_coin: String,
    /// Wrapped native token identifier, when applicable.
    pub wrapped_token: Option<String>,
}

/// Errors raised while converting configured network values into runtime primitives.
#[derive(Debug)]
pub enum NetworkValueError {
    /// `min_balance_coin` could not be parsed into base units.
    InvalidMinBalance {
        /// Original configured value.
        value: String,
        /// Parse or range-check failure reason.
        reason: String,
    },
}

impl fmt::Display for NetworkValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMinBalance { value, reason } => {
                write!(f, "invalid min_balance_coin {value:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for NetworkValueError {}

impl Network {
    /// Parses `min_balance_coin` as a decimal coin amount into base units (e.g. wei).
    ///
    /// This is useful when a human-authored config stores thresholds in whole-coin notation but
    /// execution code needs integer base units for comparisons or RPC calls.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mfm_core::config::network::{Kind, Network};
    ///
    /// let network = Network {
    ///     name: "Ethereum".to_string(),
    ///     kind: Kind::Evm,
    ///     symbol: "ETH".to_string(),
    ///     decimals: Some(18),
    ///     chain_id: 1,
    ///     node_url_http: None,
    ///     node_url_grpc: None,
    ///     blockexplorer_url: None,
    ///     min_balance_coin: "0.5".to_string(),
    ///     wrapped_token: Some("weth".to_string()),
    /// };
    ///
    /// assert_eq!(network.min_balance_wei(18)?.to_string(), "500000000000000000");
    /// # Ok::<(), mfm_core::config::network::NetworkValueError>(())
    /// ```
    pub fn min_balance_wei(&self, decimals: u8) -> Result<U256, NetworkValueError> {
        decimal::parse_scaled_u256(&self.min_balance_coin, decimals, false).map_err(|reason| {
            NetworkValueError::InvalidMinBalance {
                value: self.min_balance_coin.clone(),
                reason,
            }
        })
    }
}

/// Mapping of named network definitions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Networks(HashMap<String, Network>);
impl Networks {
    /// Looks up a network definition by key.
    ///
    /// Keys are the logical identifiers from the YAML registry, not human-readable names.
    pub fn get(&self, key: &str) -> Option<&Network> {
        self.0.get(key)
    }

    /// Returns the underlying map of named network definitions.
    ///
    /// Prefer [`Self::get`] when you only need one entry; use this when validating or iterating
    /// across the full registry.
    pub fn hashmap(&self) -> &HashMap<String, Network> {
        &self.0
    }
}
