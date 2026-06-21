//! Static token registry configuration.
//!
//! Token definitions are keyed by logical asset name and expanded into per-network deployment
//! records. Helpers in this module keep user-authored slippage strings in a normalized basis-point
//! form for downstream execution layers.

use super::decimal;
use alloy_primitives::Address;
use alloy_primitives::U256;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Supported token families in static configuration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// ERC-20-compatible fungible token.
    Erc20,
}

/// Network-specific token deployment configuration.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetwork {
    /// Human-readable token name for this network.
    pub name: String,
    /// Token family identifier.
    pub kind: Kind,
    /// Logical network identifier where this token is deployed.
    pub network_id: String,
    /// Token contract address.
    pub address: Address,
    /// Default slippage tolerance expressed as a percentage string.
    pub slippage: String,
    /// Path token identifier used for routing or quoting.
    pub path_token: String,
    /// Token decimals, when known.
    pub decimals: Option<u8>,
}

/// Errors raised while parsing configured slippage values.
#[derive(Debug)]
pub enum SlippageParseError {
    /// The configured slippage string was malformed or out of range.
    InvalidSlippage {
        /// Original configured value.
        value: String,
        /// Parse or range-check failure reason.
        reason: String,
    },
}

impl fmt::Display for SlippageParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSlippage { value, reason } => {
                write!(f, "invalid slippage {value:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for SlippageParseError {}

impl TokenNetwork {
    /// Parses slippage (in percent, e.g. `"0.50"` for 0.50%) into basis points (bps).
    ///
    /// The parser accepts an optional trailing `%` and enforces at most two decimal places because
    /// one basis point equals `0.01%`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use alloy_primitives::Address;
    /// use mfm_core::config::token::{Kind, TokenNetwork};
    ///
    /// let token = TokenNetwork {
    ///     name: "Wrapped Ether".to_string(),
    ///     kind: Kind::Erc20,
    ///     network_id: "mainnet".to_string(),
    ///     address: Address::ZERO,
    ///     slippage: "0.50%".to_string(),
    ///     path_token: "weth".to_string(),
    ///     decimals: Some(18),
    /// };
    ///
    /// assert_eq!(token.slippage_bps()?, 50);
    /// # Ok::<(), mfm_core::config::token::SlippageParseError>(())
    /// ```
    pub fn slippage_bps(&self) -> Result<u32, SlippageParseError> {
        let raw = self.slippage.trim();
        let raw = raw.strip_suffix('%').unwrap_or(raw).trim();
        let scaled = decimal::parse_scaled_u256(raw, 2, true).map_err(|reason| {
            SlippageParseError::InvalidSlippage {
                value: self.slippage.clone(),
                reason,
            }
        })?;

        if scaled > U256::from(10_000u32) {
            return Err(SlippageParseError::InvalidSlippage {
                value: self.slippage.clone(),
                reason: "must be <= 100% (<= 10000 bps)".to_string(),
            });
        }

        let bps = u32::try_from(scaled).map_err(|_| SlippageParseError::InvalidSlippage {
            value: self.slippage.clone(),
            reason: "value out of range".to_string(),
        })?;

        if bps > 10_000 {
            return Err(SlippageParseError::InvalidSlippage {
                value: self.slippage.clone(),
                reason: "must be <= 100% (<= 10000 bps)".to_string(),
            });
        }

        Ok(bps)
    }
}

/// Mapping of named network-specific token definitions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct TokenNetworks(HashMap<String, TokenNetwork>);

impl TokenNetworks {
    /// Returns the underlying map of token-network definitions.
    ///
    /// This is mainly useful for validation or full-registry iteration.
    pub fn hashmap(&self) -> &HashMap<String, TokenNetwork> {
        &self.0
    }

    /// Looks up a token-network definition by key.
    ///
    /// Keys are the logical per-network identifiers nested under a token entry.
    pub fn get(&self, key: &str) -> Option<&TokenNetwork> {
        self.0.get(key)
    }
}

/// Static token definition spanning one or more networks.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Token {
    // TODO: rethink tokens to be any kind of token, but each
    // chain/network will may have an different token kind.
    /// Per-network deployments for this logical token.
    pub networks: TokenNetworks,
}

/// Mapping of named token definitions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Tokens(HashMap<String, Token>);
impl Tokens {
    /// Returns the underlying map of token definitions.
    ///
    /// Prefer [`Self::get`] when you only need one token entry.
    pub fn hashmap(&self) -> &HashMap<String, Token> {
        &self.0
    }

    /// Looks up a token definition by key.
    ///
    /// Keys are the logical token identifiers from the YAML registry such as `weth` or `usdc`.
    pub fn get(&self, key: &str) -> Option<&Token> {
        self.0.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slippage_bps_parses_percent_strings() {
        let token = TokenNetwork {
            name: "Wrapped Ether".to_string(),
            kind: Kind::Erc20,
            network_id: "mainnet".to_string(),
            address: Address::ZERO,
            slippage: "0.50%".to_string(),
            path_token: "weth".to_string(),
            decimals: Some(18),
        };
        assert_eq!(token.slippage_bps().expect("valid"), 50);
    }

    #[test]
    fn slippage_bps_rejects_out_of_range_or_malformed_input() {
        let invalid_cases = [
            ("", "empty string"),
            ("-1", "must be non-negative"),
            ("100.001", "too many decimal places"),
            ("abc", "invalid"),
            ("101", "must be <= 100%"),
            ("100.01", "must be <= 100%"),
        ];

        for (value, reason_fragment) in invalid_cases {
            let token = TokenNetwork {
                name: "Wrapped Ether".to_string(),
                kind: Kind::Erc20,
                network_id: "mainnet".to_string(),
                address: Address::ZERO,
                slippage: value.to_string(),
                path_token: "weth".to_string(),
                decimals: Some(18),
            };
            let err = token
                .slippage_bps()
                .expect_err("invalid slippage should fail");
            assert!(err.to_string().contains(reason_fragment), "{value} {err:?}");
        }
    }
}
