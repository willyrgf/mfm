//! Decimal helpers shared by portfolio and protocol runtime adapters.
//!
//! The helpers avoid ad-hoc string math in callers while preserving existing decimal-string
//! representations with bounded precision and stable formatting.

use std::fmt;

use num_bigint::BigInt;
use num_traits::{Signed, Zero};

/// Error returned by decimal helpers when inputs are malformed or invalid for the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecimalArithmeticError {
    /// The supplied decimal string was malformed.
    InvalidDecimalString {
        /// The original input string that failed parsing.
        value: String,
    },
    /// Division requires a non-zero denominator.
    DivisionByZero,
}

impl fmt::Display for DecimalArithmeticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDecimalString { value } => {
                write!(f, "invalid decimal string `{value}`")
            }
            Self::DivisionByZero => write!(f, "division by zero"),
        }
    }
}

impl std::error::Error for DecimalArithmeticError {}

/// Multiplies two decimal strings and returns a normalized string with a deterministic scale.
pub fn multiply_decimal_strings(left: &str, right: &str) -> Result<String, DecimalArithmeticError> {
    let left = DecimalValue::parse_non_negative(left)?;
    let right = DecimalValue::parse_non_negative(right)?;
    let product = DecimalValue {
        digits: left.digits * right.digits,
        scale: left.scale + right.scale,
    };
    Ok(product.to_string_with_min_scale(left.scale.max(right.scale)))
}

/// Divides decimal strings using a fixed intermediate precision and returns a normalized string.
pub fn divide_decimal_strings(
    numerator: &str,
    denominator: &str,
) -> Result<String, DecimalArithmeticError> {
    let numerator = DecimalValue::parse_non_negative(numerator)?;
    let denominator = DecimalValue::parse_non_negative(denominator)?;
    if denominator.digits.is_zero() {
        return Err(DecimalArithmeticError::DivisionByZero);
    }

    let min_scale = numerator.scale.max(denominator.scale).max(8);
    let working_scale = min_scale + 18;
    let scaled_numerator = numerator.digits * ten_pow(working_scale + denominator.scale);
    let scaled_denominator = denominator.digits * ten_pow(numerator.scale);
    let quotient = scaled_numerator / scaled_denominator;

    Ok(DecimalValue {
        digits: quotient,
        scale: working_scale,
    }
    .to_string_with_min_scale(min_scale))
}

/// Parsed decimal in a lossless scaled-integer form.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DecimalValue {
    digits: BigInt,
    scale: u32,
}

impl DecimalValue {
    /// Parses a non-negative decimal string into a scaled representation.
    pub fn parse_non_negative(input: &str) -> Result<Self, DecimalArithmeticError> {
        Self::parse(input, false)
    }

    /// Parses a decimal string into a scaled representation, preserving a leading minus sign.
    pub fn parse_signed(input: &str) -> Result<Self, DecimalArithmeticError> {
        Self::parse(input, true)
    }

    fn parse(input: &str, allow_negative: bool) -> Result<Self, DecimalArithmeticError> {
        let trimmed = input.trim();
        let (negative, digits_part) = match trimmed.strip_prefix('-') {
            Some(rest) => {
                if !allow_negative {
                    return Err(DecimalArithmeticError::InvalidDecimalString {
                        value: input.to_string(),
                    });
                }
                (true, rest)
            }
            None => (false, trimmed),
        };
        if digits_part.is_empty() {
            return Err(DecimalArithmeticError::InvalidDecimalString {
                value: input.to_string(),
            });
        }

        let mut it = digits_part.split('.');
        let whole = it.next().unwrap_or("");
        let frac = it.next().unwrap_or("");
        if it.next().is_some() {
            return Err(DecimalArithmeticError::InvalidDecimalString {
                value: input.to_string(),
            });
        }

        let has_invalid_digits =
            |part: &str| !part.is_empty() && !part.chars().all(|ch| ch.is_ascii_digit());
        if has_invalid_digits(whole) || has_invalid_digits(frac) {
            return Err(DecimalArithmeticError::InvalidDecimalString {
                value: input.to_string(),
            });
        }

        let digits = format!("{whole}{frac}");
        let digits = if digits.is_empty() {
            "0"
        } else {
            digits.as_str()
        };
        let parsed: BigInt =
            digits
                .parse()
                .map_err(|_| DecimalArithmeticError::InvalidDecimalString {
                    value: input.to_string(),
                })?;
        Ok(Self {
            digits: if negative && !parsed.is_zero() {
                -parsed
            } else {
                parsed
            },
            scale: frac.len() as u32,
        })
    }

    /// Creates a numeric zero value.
    pub fn zero() -> Self {
        Self {
            digits: BigInt::from(0u8),
            scale: 0,
        }
    }

    /// Adds this decimal to another, returning a normalized sum.
    pub fn add(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        Self {
            digits: self.scaled_digits(scale) + other.scaled_digits(scale),
            scale,
        }
    }

    /// Subtracts another decimal from this one.
    pub fn sub(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        Self {
            digits: self.scaled_digits(scale) - other.scaled_digits(scale),
            scale,
        }
    }

    fn scaled_digits(&self, scale: u32) -> BigInt {
        if self.scale == scale {
            self.digits.clone()
        } else {
            &self.digits * ten_pow(scale - self.scale)
        }
    }

    /// Returns a canonical textual decimal representation with minimal trailing precision.
    pub fn to_canonical_string(&self) -> String {
        self.to_string_with_min_scale(self.scale)
    }

    fn to_string_with_min_scale(&self, min_scale: u32) -> String {
        let negative = self.digits.is_negative();
        let digits = self.digits.abs().to_string();
        let scale = self.scale as usize;
        let mut out = if scale == 0 {
            digits
        } else if digits.len() <= scale {
            format!("0.{}{}", "0".repeat(scale - digits.len()), digits)
        } else {
            let split = digits.len() - scale;
            format!("{}.{}", &digits[..split], &digits[split..])
        };

        if let Some((whole, frac)) = out.split_once('.') {
            let mut frac = frac.to_string();
            while frac.len() > min_scale as usize && frac.ends_with('0') {
                frac.pop();
            }
            if frac.len() < min_scale as usize {
                frac.push_str(&"0".repeat(min_scale as usize - frac.len()));
            }
            out = format!("{whole}.{frac}");
        } else if min_scale > 0 {
            out.push('.');
            out.push_str(&"0".repeat(min_scale as usize));
        }

        if negative && out != "0" {
            format!("-{out}")
        } else {
            out
        }
    }
}

fn ten_pow(n: u32) -> BigInt {
    BigInt::from(10u8).pow(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_signed_decimal_values() {
        let raw = DecimalValue::parse_signed("-1.250").expect("parse");
        assert_eq!(raw.to_canonical_string(), "-1.250");
        let raw = DecimalValue::parse_signed("-0").expect("parse");
        assert_eq!(raw.to_canonical_string(), "0");
    }

    #[test]
    fn decimal_value_add_sub_preserves_scale() {
        let left = DecimalValue::parse_signed("1.2").expect("parse");
        let right = DecimalValue::parse_signed("-0.2").expect("parse");
        assert_eq!(left.sub(&right).to_canonical_string(), "1.4");

        assert_eq!(
            left.add(&DecimalValue::parse_signed("2.3").expect("parse"))
                .to_canonical_string(),
            "3.5"
        );
    }

    #[test]
    fn multiply_decimal_strings_retains_scale() {
        assert_eq!(
            multiply_decimal_strings("1.50", "2.00").expect("mul"),
            "3.00".to_string()
        );
        assert_eq!(multiply_decimal_strings("2", "0.25").expect("mul"), "0.50");
    }

    #[test]
    fn divide_decimal_strings_divides_with_precision() {
        assert_eq!(
            divide_decimal_strings("1.0", "2").expect("div"),
            "0.50000000"
        );
    }

    #[test]
    fn decimal_arithmetic_rejects_invalid_inputs() {
        assert!(matches!(
            multiply_decimal_strings("bad", "1").err(),
            Some(DecimalArithmeticError::InvalidDecimalString { .. })
        ));
        assert!(matches!(
            divide_decimal_strings("1", "0").err(),
            Some(DecimalArithmeticError::DivisionByZero)
        ));
    }
}
