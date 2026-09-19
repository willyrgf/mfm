use std::fmt;

use mfm_ids::{DigestAlgorithm, SemanticTypeId};
use serde::{Deserialize, Serialize};

use crate::{
    framework_value_descriptor, EnumTagging, EnumVariantDescriptor, MfmValue, SchemaDescriptor,
    SchemaShape, StringGrammar, ValueError,
};

const MAX_DECIMAL: &str =
    "115792089237316195423570985008687907853269984665640564039457584007913129639935";

/// A rejected canonical unsigned 256-bit decimal; rejected text is never retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum Unsigned256Error {
    /// The text is empty, signed, nondecimal, or has redundant leading zeroes.
    #[error("unsigned 256-bit decimal is not canonical")]
    NonCanonical,
    /// The canonical decimal exceeds `2^256 - 1`.
    #[error("unsigned 256-bit decimal exceeds its range")]
    OutOfRange,
}

/// Canonical decimal integer in the inclusive range `0..=2^256-1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Unsigned256(String);

impl Unsigned256 {
    /// Checks the decimal grammar and complete 256-bit range.
    pub fn new(value: impl Into<String>) -> Result<Self, Unsigned256Error> {
        let value = value.into();
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(Unsigned256Error::NonCanonical);
        }
        if !in_range(&value) {
            return Err(Unsigned256Error::OutOfRange);
        }
        Ok(Self(value))
    }

    /// Constructs a value from the narrower unsigned range.
    pub fn from_u64(value: u64) -> Self {
        Self(value.to_string())
    }

    /// Borrows the canonical decimal representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the value into its canonical decimal representation.
    pub fn into_string(self) -> String {
        self.0
    }

    /// Returns whether this is canonical zero.
    pub fn is_zero(&self) -> bool {
        self.0 == "0"
    }

    /// Projects into the narrower range only when the value fits.
    pub fn to_u128(&self) -> Option<u128> {
        self.0.parse().ok()
    }

    /// Adds two checked values, returning `None` only on 256-bit overflow.
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        let mut left = self.0.bytes().rev();
        let mut right = other.0.bytes().rev();
        let mut digits = String::with_capacity(self.0.len().max(other.0.len()) + 1);
        let mut carry = 0;
        loop {
            let pair = (left.next(), right.next());
            if pair == (None, None) {
                break;
            }
            let sum = pair.0.unwrap_or(b'0') - b'0' + pair.1.unwrap_or(b'0') - b'0' + carry;
            digits.push(char::from(b'0' + sum % 10));
            carry = sum / 10;
        }
        if carry != 0 {
            digits.push(char::from(b'0' + carry));
        }
        // Canonical operands produce canonical decimal digits; only the range needs checking.
        let value: String = digits.chars().rev().collect();
        in_range(&value).then_some(Self(value))
    }
}

fn in_range(value: &str) -> bool {
    value.len() < MAX_DECIMAL.len() || (value.len() == MAX_DECIMAL.len() && value <= MAX_DECIMAL)
}

impl fmt::Display for Unsigned256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Unsigned256 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl MfmValue for Unsigned256 {
    fn schema_descriptor() -> crate::Result<SchemaDescriptor> {
        framework_value_descriptor(
            "mfm-values",
            Self::semantic_id()?,
            "mfm.unsigned256",
            SchemaShape::BoundedString {
                minimum_bytes: 1,
                maximum_bytes: 78,
                grammar: StringGrammar::UnicodeScalarText,
            },
            "mfm_values::Unsigned256",
        )
    }

    fn semantic_id() -> crate::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.values",
            "unsigned256",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"semantic:mfm.values:unsigned256:1"),
        )
        .map_err(ValueError::Identity)
    }
}

impl MfmValue for Unsigned256Error {
    fn schema_descriptor() -> crate::Result<SchemaDescriptor> {
        framework_value_descriptor(
            "mfm-values",
            Self::semantic_id()?,
            "mfm.unsigned256-error",
            SchemaShape::Enum {
                tagging: EnumTagging::External,
                variants: vec![
                    EnumVariantDescriptor::new("non_canonical", SchemaShape::Unit),
                    EnumVariantDescriptor::new("out_of_range", SchemaShape::Unit),
                ],
            },
            "mfm_values::Unsigned256Error",
        )
    }

    fn semantic_id() -> crate::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.values",
            "unsigned256-error",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"semantic:mfm.values:unsigned256-error:1"),
        )
        .map_err(ValueError::Identity)
    }
}
