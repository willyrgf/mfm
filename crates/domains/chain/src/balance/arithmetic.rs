use mfm_program::{Classification, ClassifyError};
use mfm_program_derive::MfmValue;
use mfm_values::{SizeLimitExceeded, Unsigned256};
use serde::{Deserialize, Serialize};

use super::{BalanceRequest, DecimalScale};

const DIGIT_LIMIT: u64 = 80;

/// Arithmetic operation that exceeded the collection decimal capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
pub enum BalanceArithmetic {
    /// Conversion from raw units into the collection scale.
    Scale,
    /// Accumulation of scaled collection amounts.
    Sum,
}

/// Exact semantic arithmetic rejection; input and native evidence remain in the State call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue, thiserror::Error)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum BalanceCollectionFailure {
    /// A non-dust amount has a nonzero discarded remainder.
    #[non_exhaustive]
    #[error("balance amount cannot be scaled exactly")]
    InexactScale {
        /// Complete raw amount before scaling.
        raw_units: Unsigned256,
        /// Observed source decimal scale.
        source_decimals: DecimalScale,
        /// Declared collection decimal scale.
        target_decimals: DecimalScale,
    },
    /// The measured result at rejection exceeded eighty digits.
    #[non_exhaustive]
    #[error("balance decimal capacity exceeded during {operation:?}: {size}")]
    DecimalCapacityExceeded {
        /// Arithmetic operation that rejected the value.
        operation: BalanceArithmetic,
        /// Measured digit count and fixed limit, preserved as a concrete cause.
        #[source]
        size: SizeLimitExceeded,
    },
}

impl BalanceCollectionFailure {
    /// Projects arithmetic failure without replacing its exact operands or size facts.
    pub const fn code(&self) -> super::BalanceFailureCode {
        super::BalanceFailureCode::ObservationUnavailable
    }
}
impl ClassifyError for BalanceCollectionFailure {
    fn classify(&self) -> Classification {
        Classification::Permanent
    }
}

impl<'de> Deserialize<'de> for BalanceCollectionFailure {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            InexactScale {
                raw_units: Unsigned256,
                source_decimals: DecimalScale,
                target_decimals: DecimalScale,
            },
            DecimalCapacityExceeded {
                operation: BalanceArithmetic,
                size: SizeLimitExceeded,
            },
        }
        match Wire::deserialize(deserializer)? {
            Wire::InexactScale {
                raw_units,
                source_decimals,
                target_decimals,
            } => match DecimalUnits80::scale(&raw_units, source_decimals, target_decimals) {
                Err(error @ Self::InexactScale { .. }) => Ok(error),
                _ => Err(serde::de::Error::custom(
                    "inexact-scale facts do not establish rejection",
                )),
            },
            Wire::DecimalCapacityExceeded { operation, size }
                if size.limit() == DIGIT_LIMIT
                    && match operation {
                        // A raw U256 has at most 78 digits; scaling appends at most 30.
                        BalanceArithmetic::Scale => size.actual() <= 108,
                        // Both addends fit 80 digits, so a rejected sum has exactly 81.
                        BalanceArithmetic::Sum => size.actual() == 81,
                    } =>
            {
                Ok(Self::DecimalCapacityExceeded { operation, size })
            }
            Wire::DecimalCapacityExceeded { .. } => Err(serde::de::Error::custom(
                "balance decimal capacity facts contradict operation bounds",
            )),
        }
    }
}

/// Canonical nonnegative scaled amount with a fixed eighty-digit capacity.
struct DecimalUnits80(String);

impl DecimalUnits80 {
    pub(super) fn scale(
        raw: &Unsigned256,
        source: DecimalScale,
        target: DecimalScale,
    ) -> Result<Self, BalanceCollectionFailure> {
        let text = raw.as_str();
        if raw.is_zero() || source == target {
            return Ok(Self(text.to_owned()));
        }
        if source.get() < target.get() {
            let zeros = usize::from(target.get() - source.get());
            SizeLimitExceeded::check((text.len() + zeros) as u64, DIGIT_LIMIT).map_err(|size| {
                BalanceCollectionFailure::DecimalCapacityExceeded {
                    operation: BalanceArithmetic::Scale,
                    size,
                }
            })?;
            let mut scaled = text.to_owned();
            scaled.extend(std::iter::repeat_n('0', zeros));
            return Ok(Self(scaled));
        }
        let shift = usize::from(source.get() - target.get());
        if text.len() <= shift {
            return Ok(Self("0".to_owned()));
        }
        let (whole, discarded) = text.split_at(text.len() - shift);
        if discarded.bytes().any(|digit| digit != b'0') {
            return Err(BalanceCollectionFailure::InexactScale {
                raw_units: raw.clone(),
                source_decimals: source,
                target_decimals: target,
            });
        }
        Ok(Self(whole.to_owned()))
    }
    pub(super) fn into_string(self) -> String {
        self.0
    }

    fn checked_add(&self, other: &Self) -> Result<Self, BalanceCollectionFailure> {
        let mut left = self.0.bytes().rev();
        let mut right = other.0.bytes().rev();
        let mut reversed = String::with_capacity(self.0.len().max(other.0.len()) + 1);
        let mut carry = 0;
        loop {
            let pair = (left.next(), right.next());
            if pair == (None, None) {
                break;
            }
            let sum = pair.0.unwrap_or(b'0') - b'0' + pair.1.unwrap_or(b'0') - b'0' + carry;
            reversed.push(char::from(b'0' + sum % 10));
            carry = sum / 10;
        }
        if carry != 0 {
            reversed.push(char::from(b'0' + carry));
        }
        SizeLimitExceeded::check(reversed.len() as u64, DIGIT_LIMIT).map_err(|size| {
            BalanceCollectionFailure::DecimalCapacityExceeded {
                operation: BalanceArithmetic::Sum,
                size,
            }
        })?;
        Ok(Self(reversed.chars().rev().collect()))
    }
}

impl BalanceRequest {
    /// Scales full-width raw units with the fixed dust, exact-remainder and eighty-digit rules.
    pub fn scale_units(
        &self,
        raw_units: &Unsigned256,
        source_decimals: DecimalScale,
    ) -> Result<String, BalanceCollectionFailure> {
        DecimalUnits80::scale(raw_units, source_decimals, self.decimals())
            .map(DecimalUnits80::into_string)
    }

    /// Scales and sums supplied amounts, rejecting the first overflowing partial sum.
    /// This pure arithmetic helper does not establish source confirmation or collection completion.
    pub fn total_scaled<'a>(
        &self,
        amounts: impl IntoIterator<Item = (&'a Unsigned256, DecimalScale)>,
    ) -> Result<String, BalanceCollectionFailure> {
        let mut total = DecimalUnits80("0".to_owned());
        for (raw, decimals) in amounts {
            let amount = DecimalUnits80::scale(raw, decimals, self.decimals())?;
            total = total.checked_add(&amount)?;
        }
        Ok(total.into_string())
    }
}
