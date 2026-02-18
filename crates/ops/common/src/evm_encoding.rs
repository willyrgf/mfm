use alloy_primitives::{Address, U256};
use mfm_machine::errors::StateError;

use crate::errors::state_unknown;
use crate::hex::{bytes_to_hex_prefixed, hex_to_bytes, normalize_hex_str};
use crate::util_error::UtilError;

pub const ERC20_SELECTOR_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
pub const ERC20_SELECTOR_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];

pub fn address_hex_lower(addr: &Address) -> String {
    // Debug formatting is lowercase and stable.
    format!("{addr:?}")
}

pub fn address_hex_lower_no0x(addr: &Address) -> String {
    address_hex_lower(addr)
        .strip_prefix("0x")
        .unwrap_or("")
        .to_string()
}

pub fn encode_erc20_balance_of(owner: &Address) -> String {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&ERC20_SELECTOR_BALANCE_OF);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(owner.as_slice());
    format!("0x{}", hex::encode(data))
}

pub fn encode_erc20_decimals() -> String {
    let mut data = Vec::with_capacity(4);
    data.extend_from_slice(&ERC20_SELECTOR_DECIMALS);
    format!("0x{}", hex::encode(data))
}

pub(crate) fn u64_hex_quantity(n: u64) -> String {
    if n == 0 {
        return "0x0".to_string();
    }
    format!("0x{:x}", n)
}

pub(crate) fn format_u256_units(raw: &U256, decimals: u8) -> String {
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

pub(crate) fn parse_u256_hex(s: &str) -> Result<U256, StateError> {
    let Some(rest) = s.strip_prefix("0x") else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    };
    if rest.is_empty() || rest.len() > 64 {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    }

    let mut hex_str = rest.to_string();
    if hex_str.len() % 2 == 1 {
        hex_str = format!("0{hex_str}");
    }
    let bytes = hex::decode(hex_str)
        .map_err(|_| state_unknown("evm_response_invalid", "evm response was not a hex u256"))?;
    Ok(U256::from_be_slice(&bytes))
}

pub(crate) fn parse_u256_hex_value(v: &serde_json::Value) -> Result<U256, StateError> {
    let Some(s) = v.as_str() else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    };
    parse_u256_hex(s)
}

pub(crate) fn parse_u8_u256(v: U256) -> Result<u8, StateError> {
    if v > U256::from(u8::MAX) {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was out of range for u8",
        ));
    }
    Ok(v.to::<u8>())
}

pub(crate) fn parse_hex_string_response(value: &serde_json::Value) -> Result<String, StateError> {
    let Some(s) = value.as_str() else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    };
    let Some(rest) = s.strip_prefix("0x") else {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    };
    if !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    }
    Ok(s.to_string())
}

pub(crate) fn parse_u256_hex_response(value: &serde_json::Value) -> Result<String, StateError> {
    let s = parse_hex_string_response(value)?;
    let rest = s.strip_prefix("0x").unwrap_or_default();
    if rest.is_empty() || rest.len() > 64 {
        return Err(state_unknown(
            "evm_response_invalid",
            "evm response was not a hex u256",
        ));
    }
    Ok(s)
}

pub fn encode_u64_word(n: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..32].copy_from_slice(&n.to_be_bytes());
    out
}

pub fn encode_len_word(n: usize) -> Result<[u8; 32], UtilError> {
    let n64 = u64::try_from(n).map_err(|_| UtilError::new("length_overflow", "length overflow"))?;
    Ok(encode_u64_word(n64))
}

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
