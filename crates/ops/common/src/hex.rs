use crate::util_error::UtilError;

pub fn normalize_hex_str(raw: &str) -> Result<String, UtilError> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .ok_or_else(|| {
            UtilError::new("hex_prefix_missing", "hex string must start with 0x or 0X")
        })?;

    if rest.is_empty() {
        return Ok("0x".to_string());
    }
    if !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UtilError::new(
            "hex_invalid_chars",
            "hex string contains non-hex characters",
        ));
    }

    let lowered = rest.to_ascii_lowercase();
    let even = if lowered.len().is_multiple_of(2) {
        lowered
    } else {
        format!("0{lowered}")
    };
    Ok(format!("0x{even}"))
}

pub fn normalize_nonempty_hex_str(raw: &str) -> Result<String, UtilError> {
    let normalized = normalize_hex_str(raw)?;
    if normalized == "0x" {
        return Err(UtilError::new("hex_empty", "hex string cannot be empty"));
    }
    Ok(normalized)
}

pub fn hex_to_bytes(raw: &str) -> Result<Vec<u8>, UtilError> {
    let normalized = normalize_hex_str(raw)?;
    let rest = normalized.strip_prefix("0x").expect("prefix");
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    hex::decode(rest).map_err(|_| UtilError::new("hex_decode_failed", "invalid hex"))
}

pub fn bytes_to_hex_prefixed(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

pub fn hex_encode_utf8(value: &str) -> String {
    hex::encode(value.as_bytes())
}

pub fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_hex_str_pads_odd_length() {
        assert_eq!(
            normalize_hex_str("0xabc").expect("normalize"),
            "0x0abc".to_string()
        );
    }

    #[test]
    fn normalize_nonempty_hex_str_rejects_empty() {
        let err = normalize_nonempty_hex_str("0x").expect_err("must fail");
        assert_eq!(err.code, "hex_empty");
    }

    #[test]
    fn hex_to_bytes_roundtrip() {
        let bytes = hex_to_bytes("0x0102ff").expect("decode");
        assert_eq!(bytes_to_hex_prefixed(&bytes), "0x0102ff");
    }
}
