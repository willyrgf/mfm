use num_bigint::BigInt;
use num_traits::{Signed, Zero};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum DecimalArithmeticError {
    #[error("invalid decimal string `{value}`")]
    InvalidDecimalString { value: String },
}

pub(super) fn multiply_decimal_strings(
    left: &str,
    right: &str,
) -> Result<String, DecimalArithmeticError> {
    let left = DecimalValue::parse_non_negative(left)?;
    let right = DecimalValue::parse_non_negative(right)?;
    let product = DecimalValue {
        digits: left.digits * right.digits,
        scale: left.scale + right.scale,
    };
    Ok(product.to_string_with_min_scale(left.scale.max(right.scale)))
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(super) struct DecimalValue {
    digits: BigInt,
    scale: u32,
}

impl DecimalValue {
    pub(super) fn parse_non_negative(input: &str) -> Result<Self, DecimalArithmeticError> {
        Self::parse(input, false)
    }

    fn parse(input: &str, allow_negative: bool) -> Result<Self, DecimalArithmeticError> {
        let trimmed = input.trim();
        let (negative, digits_part) = match trimmed.strip_prefix('-') {
            Some(rest) => {
                if !allow_negative {
                    return Err(DecimalArithmeticError::InvalidDecimalString {
                        value: input.to_owned(),
                    });
                }
                (true, rest)
            }
            None => (false, trimmed),
        };
        if digits_part.is_empty() {
            return Err(DecimalArithmeticError::InvalidDecimalString {
                value: input.to_owned(),
            });
        }
        let (whole, frac) = digits_part.split_once('.').unwrap_or((digits_part, ""));
        if frac.contains('.')
            || (!whole.is_empty() && !whole.chars().all(|ch| ch.is_ascii_digit()))
            || (!frac.is_empty() && !frac.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Err(DecimalArithmeticError::InvalidDecimalString {
                value: input.to_owned(),
            });
        }
        let digits = format!("{whole}{frac}");
        let parsed: BigInt = if digits.is_empty() {
            BigInt::from(0u8)
        } else {
            digits
                .parse()
                .map_err(|_| DecimalArithmeticError::InvalidDecimalString {
                    value: input.to_owned(),
                })?
        };
        Ok(Self {
            digits: if negative && !parsed.is_zero() {
                -parsed
            } else {
                parsed
            },
            scale: frac.len() as u32,
        })
    }

    pub(super) fn add(&self, other: &Self) -> Self {
        let scale = self.scale.max(other.scale);
        Self {
            digits: self.scaled_digits(scale) + other.scaled_digits(scale),
            scale,
        }
    }

    pub(super) fn add_assign(&mut self, other: &Self) {
        *self = self.add(other);
    }

    fn scaled_digits(&self, scale: u32) -> BigInt {
        if self.scale == scale {
            self.digits.clone()
        } else {
            &self.digits * ten_pow(scale - self.scale)
        }
    }

    pub(super) fn to_canonical_string(&self) -> String {
        self.to_string_with_min_scale(0)
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
            let mut frac = frac.to_owned();
            while frac.len() > min_scale as usize && frac.ends_with('0') {
                frac.pop();
            }
            if frac.len() < min_scale as usize {
                frac.push_str(&"0".repeat(min_scale as usize - frac.len()));
            }
            out = if frac.is_empty() {
                whole.to_owned()
            } else {
                format!("{whole}.{frac}")
            };
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
