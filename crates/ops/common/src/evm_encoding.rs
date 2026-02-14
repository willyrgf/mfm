use crate::hex::{bytes_to_hex_prefixed, hex_to_bytes, normalize_hex_str};
use crate::util_error::UtilError;

pub const ERC20_SELECTOR_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
pub const ERC20_SELECTOR_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];

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
