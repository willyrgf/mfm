use alloy_primitives::U256;

#[derive(Clone, Copy, Debug)]
struct DecimalParts<'a> {
    integer: &'a str,
    fraction: &'a str,
}

impl<'a> DecimalParts<'a> {
    fn split(raw: &'a str) -> Option<Self> {
        let (integer, fraction) = match raw.split_once('.') {
            Some((integer, fraction)) => {
                if fraction.contains('.') {
                    return None;
                }
                (integer, fraction)
            }
            None => (raw, ""),
        };
        Some(Self { integer, fraction })
    }
}

pub(crate) fn parse_scaled_u256(
    value: &str,
    scale: u8,
    allow_trailing_fraction_zeros: bool,
) -> Result<U256, String> {
    let raw = value.trim();
    if raw.is_empty() {
        return Err("empty string".to_string());
    }
    if raw.starts_with('-') {
        return Err("must be non-negative".to_string());
    }

    let Some(parts) = DecimalParts::split(raw) else {
        return Err("invalid decimal separators".to_string());
    };
    let (int_part, frac_part) = (parts.integer, parts.fraction);

    let int_part = if int_part.is_empty() { "0" } else { int_part };
    if !int_part.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("invalid integer digits".to_string());
    }
    if !frac_part.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("invalid fractional digits".to_string());
    }

    let scale_usize = scale as usize;
    let normalized_frac = if frac_part.len() > scale_usize {
        if scale == 0 {
            return Err("fractional part not allowed when decimals=0".to_string());
        }
        if allow_trailing_fraction_zeros {
            let (head, tail) = frac_part.split_at(scale_usize);
            if tail.chars().all(|ch| ch == '0') {
                head.to_string()
            } else {
                return Err(format!(
                    "too many decimal places (max {scale}, got {})",
                    frac_part.len()
                ));
            }
        } else {
            return Err(format!(
                "too many decimal places (max {scale}, got {})",
                frac_part.len()
            ));
        }
    } else {
        let mut frac = frac_part.to_string();
        frac.push_str(&"0".repeat(scale_usize - frac_part.len()));
        frac
    };

    let scaled = format!("{int_part}{normalized_frac}");
    parse_u256_decimal(&scaled)
}

pub(crate) fn parse_u256_decimal(s: &str) -> Result<U256, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty number".to_string());
    }

    let mut value = U256::from(0u8);
    for ch in s.chars() {
        if !ch.is_ascii_digit() {
            return Err("invalid digit".to_string());
        }
        let digit = ch as u8 - b'0';
        value = value
            .checked_mul(U256::from(10u8))
            .ok_or_else(|| "value out of range".to_string())?;
        value = value
            .checked_add(U256::from(digit))
            .ok_or_else(|| "value out of range".to_string())?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scaled_u256_rounds_with_trailing_zeros_only() {
        let got = parse_scaled_u256("0.5000", 2, true).expect("scaled");
        assert_eq!(got, U256::from(50u8));
    }

    #[test]
    fn parse_scaled_u256_rejects_non_digit_tail() {
        assert!(parse_scaled_u256("0.5a", 2, false).is_err());
    }

    #[test]
    fn parse_scaled_u256_rejects_multiple_decimal_separators() {
        assert!(parse_scaled_u256("1.2.3", 2, false).is_err());
    }
}
