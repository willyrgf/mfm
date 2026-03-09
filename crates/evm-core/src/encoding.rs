//! EVM encoding helpers for common JSON-RPC and ERC-20 interactions.
//!
//! The functions here are intentionally transport-agnostic. They normalize addresses, format
//! JSON-RPC quantities, and build small ABI snippets that runtime callers can embed into requests.
//!
//! # Examples
//!
//! ```rust
//! use alloy_primitives::Address;
//! use mfm_evm_core::encoding::{encode_erc20_balance_of, normalize_address, u64_hex_quantity};
//!
//! let owner = Address::from([0x11; 20]);
//! let calldata = encode_erc20_balance_of(&owner);
//! assert!(calldata.starts_with("0x70a08231"));
//! assert_eq!(u64_hex_quantity(15), "0xf");
//! assert_eq!(
//!     normalize_address("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")?,
//!     "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
//! );
//! # Ok::<(), mfm_evm_core::util_error::UtilError>(())
//! ```

use alloy_primitives::{Address, U256};

use crate::hex::{bytes_to_hex_prefixed, hex_to_bytes, normalize_hex_str};
use crate::util_error::UtilError;

/// Function selector for `balanceOf(address)`.
pub const ERC20_SELECTOR_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
/// Function selector for `decimals()`.
pub const ERC20_SELECTOR_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];

/// Formats an [`Address`] as a lowercase `0x`-prefixed string.
pub fn address_hex_lower(addr: &Address) -> String {
    // Debug formatting is lowercase and stable.
    format!("{addr:?}")
}

/// Formats an [`Address`] as lowercase hex without the `0x` prefix.
pub fn address_hex_lower_no0x(addr: &Address) -> String {
    address_hex_lower(addr)
        .strip_prefix("0x")
        .unwrap_or("")
        .to_string()
}

/// Encodes the calldata for `balanceOf(address)`.
pub fn encode_erc20_balance_of(owner: &Address) -> String {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&ERC20_SELECTOR_BALANCE_OF);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(owner.as_slice());
    format!("0x{}", hex::encode(data))
}

/// Encodes the calldata for `decimals()`.
pub fn encode_erc20_decimals() -> String {
    let mut data = Vec::with_capacity(4);
    data.extend_from_slice(&ERC20_SELECTOR_DECIMALS);
    format!("0x{}", hex::encode(data))
}

/// Formats a JSON-RPC quantity value.
pub fn u64_hex_quantity(n: u64) -> String {
    if n == 0 {
        return "0x0".to_string();
    }
    format!("0x{:x}", n)
}

/// Formats a `U256` using a decimal point at `decimals` places.
pub fn format_u256_units(raw: &U256, decimals: u8) -> String {
    let s = raw.to_string();
    let d = decimals as usize;
    if d == 0 {
        return s;
    }
    if s.len() <= d {
        let mut out = String::with_capacity(2 + d + 1);
        out.push_str("0.");
        out.push_str(&"0".repeat(d - s.len()));
        out.push_str(&s);
        out
    } else {
        let split = s.len() - d;
        let mut out = String::with_capacity(s.len() + 1);
        out.push_str(&s[..split]);
        out.push('.');
        out.push_str(&s[split..]);
        out
    }
}

/// Parses a `0x`-prefixed hex string into a `U256`.
pub fn parse_u256_hex(s: &str) -> Result<U256, UtilError> {
    let Some(rest) = s.strip_prefix("0x") else {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    };
    if rest.is_empty() || rest.len() > 64 {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    }

    let mut hex_str = rest.to_string();
    if hex_str.len() % 2 == 1 {
        hex_str = format!("0{hex_str}");
    }
    let bytes = hex::decode(hex_str)
        .map_err(|_| UtilError::new("evm_response_invalid", "evm response was not a hex u256"))?;
    Ok(U256::from_be_slice(&bytes))
}

/// Parses a JSON value that must contain a hex-encoded `U256` string.
pub fn parse_u256_hex_value(v: &serde_json::Value) -> Result<U256, UtilError> {
    let Some(s) = v.as_str() else {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    };
    parse_u256_hex(s)
}

/// Converts a `U256` into `u8`, returning an error on overflow.
pub fn parse_u8_u256(v: U256) -> Result<u8, UtilError> {
    if v > U256::from(u8::MAX) {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was out of range for u8",
        ));
    }
    Ok(v.to::<u8>())
}

/// Validates that a JSON value contains a `0x`-prefixed hex string response.
pub fn parse_hex_string_response(value: &serde_json::Value) -> Result<String, UtilError> {
    let Some(s) = value.as_str() else {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    };
    let Some(rest) = s.strip_prefix("0x") else {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    };
    if !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    }
    Ok(s.to_string())
}

/// Validates that a JSON value contains a hex-encoded `U256` response.
pub fn parse_u256_hex_response(value: &serde_json::Value) -> Result<String, UtilError> {
    let s = parse_hex_string_response(value)?;
    let rest = s.strip_prefix("0x").unwrap_or_default();
    if rest.is_empty() || rest.len() > 64 {
        return Err(UtilError::new(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    }
    Ok(s)
}

/// Encodes a `u64` into a 32-byte ABI word.
pub fn encode_u64_word(n: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&n.to_be_bytes());
    out
}

/// Encodes a byte length into a 32-byte ABI word.
pub fn encode_len_word(n: usize) -> Result<[u8; 32], UtilError> {
    let n64 = u64::try_from(n).map_err(|_| UtilError::new("length_overflow", "length overflow"))?;
    Ok(encode_u64_word(n64))
}

/// Parses a hex-encoded address into its raw 20-byte form.
pub fn parse_address_hex(raw: &str) -> Result<[u8; 20], UtilError> {
    let b = hex_to_bytes(raw)?;
    if b.len() != 20 {
        return Err(UtilError::new(
            "invalid_address",
            "address must be 20 bytes",
        ));
    }
    let mut out = [0u8; 20];
    out.copy_from_slice(&b);
    Ok(out)
}

/// Normalizes an address string to lowercase `0x`-prefixed form.
///
/// This helper accepts upper- or lower-case input, enforces 20-byte width, and returns the
/// canonical lowercase form used elsewhere in MFM docs and manifests.
pub fn normalize_address(raw: &str) -> Result<String, UtilError> {
    let normalized = normalize_hex_str(raw)?;
    let rest = normalized.strip_prefix("0x").unwrap_or_default();
    if rest.len() != 40 {
        return Err(UtilError::new(
            "invalid_address",
            "address must be 20 bytes",
        ));
    }
    Ok(format!("0x{}", rest.to_ascii_lowercase()))
}

/// Encodes a raw address as a lowercase `0x`-prefixed hex string.
pub fn address_to_hex_prefixed(addr: &[u8; 20]) -> String {
    bytes_to_hex_prefixed(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_address_lowercases() {
        let got = normalize_address("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").expect("ok");
        assert_eq!(got, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    }

    #[test]
    fn encode_word_has_big_endian_value() {
        let word = encode_u64_word(7);
        assert_eq!(word[31], 7u8);
    }
}
