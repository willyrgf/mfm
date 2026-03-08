//! Static network registry configuration.
//!
//! These types describe the configured networks that token and DEX entries reference, plus helper
//! conversions for moving human-readable balance thresholds into base units.

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

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
#[derive(Debug, Error)]
pub enum NetworkValueError {
    /// `min_balance_coin` could not be parsed into base units.
    #[error("invalid min_balance_coin {value:?}: {reason}")]
    InvalidMinBalance {
        /// Original configured value.
        value: String,
        /// Parse or range-check failure reason.
        reason: String,
    },
}

impl Network {
    /// Parses `min_balance_coin` as a decimal coin amount into base units (e.g. wei).
    pub fn min_balance_wei(&self, decimals: u8) -> Result<U256, NetworkValueError> {
        decimal_str_to_u256_units(&self.min_balance_coin, decimals).map_err(|reason| {
            NetworkValueError::InvalidMinBalance {
                value: self.min_balance_coin.clone(),
                reason,
            }
        })
    }
}

fn decimal_str_to_u256_units(value: &str, decimals: u8) -> Result<U256, String> {
    let s = value.trim();
    if s.is_empty() {
        return Err("empty string".to_string());
    }
    if s.starts_with('-') {
        return Err("must be non-negative".to_string());
    }

    let (int_part, frac_part_opt) = match s.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };

    let int_part = if int_part.is_empty() { "0" } else { int_part };
    if !int_part.chars().all(|c| c.is_ascii_digit()) {
        return Err("invalid integer digits".to_string());
    }

    let frac_part = frac_part_opt.unwrap_or("");
    if !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return Err("invalid fractional digits".to_string());
    }
    let decimals_usize = decimals as usize;
    if frac_part.len() > decimals_usize {
        return Err(format!(
            "too many decimal places (max {decimals}, got {})",
            frac_part.len()
        ));
    }

    let scale = pow10_u256(decimals);
    let int_units = parse_u256_decimal(int_part)?
        .checked_mul(scale)
        .ok_or_else(|| "value out of range".to_string())?;

    let frac_units = if decimals == 0 {
        if frac_part.chars().any(|c| c != '0') {
            return Err("fractional part not allowed when decimals=0".to_string());
        }
        U256::from(0u8)
    } else if frac_part.is_empty() {
        U256::from(0u8)
    } else {
        let mut frac_scaled = frac_part.to_string();
        frac_scaled.push_str(&"0".repeat(decimals_usize - frac_part.len()));
        parse_u256_decimal(&frac_scaled)?
    };

    int_units
        .checked_add(frac_units)
        .ok_or_else(|| "value out of range".to_string())
}

fn pow10_u256(exp: u8) -> U256 {
    let mut v = U256::from(1u8);
    for _ in 0..exp {
        v *= U256::from(10u8);
    }
    v
}

fn parse_u256_decimal(s: &str) -> Result<U256, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty number".to_string());
    }

    let mut v = U256::from(0u8);
    for ch in s.chars() {
        if !ch.is_ascii_digit() {
            return Err("invalid digit".to_string());
        }
        let digit = ch as u8 - b'0';
        v = v
            .checked_mul(U256::from(10u8))
            .ok_or_else(|| "value out of range".to_string())?;
        v = v
            .checked_add(U256::from(digit))
            .ok_or_else(|| "value out of range".to_string())?;
    }
    Ok(v)
}

/// Mapping of named network definitions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Networks(HashMap<String, Network>);
impl Networks {
    /// Looks up a network definition by key.
    pub fn get(&self, key: &str) -> Option<&Network> {
        self.0.get(key)
    }

    /// Returns the underlying map of named network definitions.
    pub fn hashmap(&self) -> &HashMap<String, Network> {
        &self.0
    }
}
