//! Static token registry configuration.
//!
//! Token definitions are keyed by logical asset name and expanded into per-network deployment
//! records. Helpers in this module keep user-authored slippage strings in a normalized basis-point
//! form for downstream execution layers.

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

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
#[derive(Debug, Error)]
pub enum SlippageParseError {
    /// The configured slippage string was malformed or out of range.
    #[error("invalid slippage {value:?}: {reason}")]
    InvalidSlippage {
        /// Original configured value.
        value: String,
        /// Parse or range-check failure reason.
        reason: String,
    },
}

impl TokenNetwork {
    /// Parses slippage (in percent, e.g. `"0.50"` for 0.50%) into basis points (bps).
    pub fn slippage_bps(&self) -> Result<u32, SlippageParseError> {
        let raw = self.slippage.trim();
        let raw = raw.strip_suffix('%').unwrap_or(raw).trim();

        if raw.is_empty() {
            return Err(SlippageParseError::InvalidSlippage {
                value: self.slippage.clone(),
                reason: "empty string".to_string(),
            });
        }
        if raw.starts_with('-') {
            return Err(SlippageParseError::InvalidSlippage {
                value: self.slippage.clone(),
                reason: "must be non-negative".to_string(),
            });
        }

        let (int_part, frac_part_opt) = match raw.split_once('.') {
            Some((a, b)) => (a, Some(b)),
            None => (raw, None),
        };

        let int_part = if int_part.is_empty() { "0" } else { int_part };
        if !int_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(SlippageParseError::InvalidSlippage {
                value: self.slippage.clone(),
                reason: "invalid integer digits".to_string(),
            });
        }

        let int_percent: u32 =
            int_part
                .parse()
                .map_err(|_| SlippageParseError::InvalidSlippage {
                    value: self.slippage.clone(),
                    reason: "integer part out of range".to_string(),
                })?;

        let frac_part = frac_part_opt.unwrap_or("");
        if !frac_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(SlippageParseError::InvalidSlippage {
                value: self.slippage.clone(),
                reason: "invalid fractional digits".to_string(),
            });
        }

        // Percent has 2 decimal places of precision for bps (0.01% = 1 bps).
        let frac_padded = match frac_part.len() {
            0 => "00".to_string(),
            1 => format!("{frac_part}0"),
            2 => frac_part.to_string(),
            _ => {
                let (head, tail) = frac_part.split_at(2);
                if tail.chars().any(|c| c != '0') {
                    return Err(SlippageParseError::InvalidSlippage {
                        value: self.slippage.clone(),
                        reason: "too many decimal places (max 2)".to_string(),
                    });
                }
                head.to_string()
            }
        };

        let frac_percent: u32 = frac_padded.parse().unwrap_or(0);
        let bps = int_percent
            .checked_mul(100)
            .and_then(|v| v.checked_add(frac_percent))
            .ok_or_else(|| SlippageParseError::InvalidSlippage {
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
    pub fn hashmap(&self) -> &HashMap<String, TokenNetwork> {
        &self.0
    }

    /// Looks up a token-network definition by key.
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
    pub fn hashmap(&self) -> &HashMap<String, Token> {
        &self.0
    }

    /// Looks up a token definition by key.
    pub fn get(&self, key: &str) -> Option<&Token> {
        self.0.get(key)
    }
}
