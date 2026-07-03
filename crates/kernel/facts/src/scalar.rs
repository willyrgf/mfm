use mfm_canonical::{CanonicalValue, DecimalString};
use mfm_ids::ContentDigest;

use crate::*;

/// Scalar fact field value used by canonical subject material.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactCanonicalScalar {
    /// UTF-8 string scalar.
    String(String),
    /// Boolean scalar.
    Boolean(bool),
    /// Signed integer scalar.
    SignedInteger(i64),
    /// Unsigned integer scalar.
    UnsignedInteger(u64),
    /// Timestamp encoded as a normalized string.
    Timestamp(String),
    /// Canonical decimal string scalar.
    DecimalString(DecimalString),
    /// Typed digest scalar.
    Digest(ContentDigest),
}

impl FactCanonicalScalar {
    /// Creates a string scalar.
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    /// Creates a timestamp scalar.
    pub fn timestamp(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "timestamp scalar must not be empty",
            ));
        }
        Ok(Self::Timestamp(value))
    }

    /// Creates a variable-scale decimal scalar using the canonical decimal grammar.
    pub fn decimal_variable(value: impl Into<String>) -> Result<Self> {
        DecimalString::new_variable(value)
            .map(Self::DecimalString)
            .map_err(|error| FactDescriptorError::canonical(error.to_string()))
    }

    /// Returns this scalar's fact field value type.
    pub fn value_type(&self) -> FactFieldValueType {
        match self {
            Self::String(_) => FactFieldValueType::String,
            Self::Boolean(_) => FactFieldValueType::Boolean,
            Self::SignedInteger(_) => FactFieldValueType::SignedInteger,
            Self::UnsignedInteger(_) => FactFieldValueType::UnsignedInteger,
            Self::Timestamp(_) => FactFieldValueType::Timestamp,
            Self::DecimalString(_) => FactFieldValueType::DecimalString,
            Self::Digest(_) => FactFieldValueType::Digest,
        }
    }

    /// Returns the UTF-8 byte length of this scalar's canonical textual representation.
    pub fn canonical_text_len(&self) -> usize {
        match self {
            Self::String(value) | Self::Timestamp(value) => value.len(),
            Self::Boolean(value) => {
                if *value {
                    4
                } else {
                    5
                }
            }
            Self::SignedInteger(value) => value.to_string().len(),
            Self::UnsignedInteger(value) => value.to_string().len(),
            Self::DecimalString(value) => value.as_str().len(),
            Self::Digest(value) => value.as_str().len(),
        }
    }

    pub(crate) fn canonical_value(&self) -> CanonicalValue {
        match self {
            Self::String(value) => CanonicalValue::String(value.clone()),
            Self::Boolean(value) => CanonicalValue::Bool(*value),
            Self::SignedInteger(value) => CanonicalValue::Signed(*value),
            Self::UnsignedInteger(value) => CanonicalValue::Unsigned(*value),
            Self::Timestamp(value) => CanonicalValue::String(value.clone()),
            Self::DecimalString(value) => CanonicalValue::Decimal(value.clone()),
            Self::Digest(value) => CanonicalValue::String(value.as_str().to_owned()),
        }
    }
}

/// Compares same-typed fact scalars using canonical fact-query semantics.
///
/// Decimal strings compare by numeric value rather than by their canonical spelling, matching the
/// durable query store contract.
pub fn fact_query_scalar_cmp(
    left: &FactCanonicalScalar,
    right: &FactCanonicalScalar,
) -> Option<std::cmp::Ordering> {
    if left.value_type() != right.value_type() {
        return None;
    }
    Some(match (left, right) {
        (FactCanonicalScalar::String(left), FactCanonicalScalar::String(right)) => left.cmp(right),
        (FactCanonicalScalar::Boolean(left), FactCanonicalScalar::Boolean(right)) => {
            left.cmp(right)
        }
        (FactCanonicalScalar::SignedInteger(left), FactCanonicalScalar::SignedInteger(right)) => {
            left.cmp(right)
        }
        (
            FactCanonicalScalar::UnsignedInteger(left),
            FactCanonicalScalar::UnsignedInteger(right),
        ) => left.cmp(right),
        (FactCanonicalScalar::Timestamp(left), FactCanonicalScalar::Timestamp(right)) => {
            left.cmp(right)
        }
        (FactCanonicalScalar::DecimalString(left), FactCanonicalScalar::DecimalString(right)) => {
            compare_decimal_strings(left.as_str(), right.as_str())
        }
        (FactCanonicalScalar::Digest(left), FactCanonicalScalar::Digest(right)) => left.cmp(right),
        _ => unreachable!("fact scalar value types matched but variants differed"),
    })
}

/// Returns whether a fact scalar satisfies a query predicate against another same-typed scalar.
pub fn fact_query_scalar_matches_operator(
    left: &FactCanonicalScalar,
    operator: FactQueryOperator,
    right: &FactCanonicalScalar,
) -> bool {
    let Some(ordering) = fact_query_scalar_cmp(left, right) else {
        return false;
    };
    match operator {
        FactQueryOperator::Equal => ordering == std::cmp::Ordering::Equal,
        FactQueryOperator::LessThan => ordering == std::cmp::Ordering::Less,
        FactQueryOperator::LessThanOrEqual => {
            matches!(
                ordering,
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal
            )
        }
        FactQueryOperator::GreaterThan => ordering == std::cmp::Ordering::Greater,
        FactQueryOperator::GreaterThanOrEqual => {
            matches!(
                ordering,
                std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
            )
        }
    }
}

/// Compares optional fact scalars for one query ordering term.
///
/// Sort direction applies only to present scalar values. Null placement follows the term's
/// `NULLS FIRST`/`NULLS LAST` policy independently, matching SQL ordering semantics.
pub fn fact_query_ordering_term_cmp(
    term: &FactOrderingTerm,
    left: Option<&FactCanonicalScalar>,
    right: Option<&FactCanonicalScalar>,
) -> Option<std::cmp::Ordering> {
    Some(match (left, right) {
        (Some(left), Some(right)) => {
            let ordering = fact_query_scalar_cmp(left, right)?;
            match term.direction() {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            }
        }
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => match term.nulls() {
            NullOrdering::First => std::cmp::Ordering::Less,
            NullOrdering::Last => std::cmp::Ordering::Greater,
        },
        (Some(_), None) => match term.nulls() {
            NullOrdering::First => std::cmp::Ordering::Greater,
            NullOrdering::Last => std::cmp::Ordering::Less,
        },
    })
}

fn compare_decimal_strings(left: &str, right: &str) -> std::cmp::Ordering {
    let (left_negative, left_abs) = decimal_sign_and_abs(left);
    let (right_negative, right_abs) = decimal_sign_and_abs(right);
    match (left_negative, right_negative) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => compare_unsigned_decimal_strings(left_abs, right_abs),
        (true, true) => compare_unsigned_decimal_strings(left_abs, right_abs).reverse(),
    }
}

fn decimal_sign_and_abs(value: &str) -> (bool, &str) {
    value
        .strip_prefix('-')
        .map_or((false, value), |abs| (true, abs))
}

fn compare_unsigned_decimal_strings(left: &str, right: &str) -> std::cmp::Ordering {
    let (left_integer, left_fraction) = decimal_parts(left);
    let (right_integer, right_fraction) = decimal_parts(right);
    left_integer
        .len()
        .cmp(&right_integer.len())
        .then_with(|| left_integer.cmp(right_integer))
        .then_with(|| compare_decimal_fractions(left_fraction, right_fraction))
}

fn decimal_parts(value: &str) -> (&str, &str) {
    value
        .split_once('.')
        .map_or((value, ""), |(integer, fraction)| (integer, fraction))
}

fn compare_decimal_fractions(left: &str, right: &str) -> std::cmp::Ordering {
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_digit = left.as_bytes().get(index).copied().unwrap_or(b'0');
        let right_digit = right.as_bytes().get(index).copied().unwrap_or(b'0');
        match left_digit.cmp(&right_digit) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}
