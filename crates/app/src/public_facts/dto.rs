use std::fmt;

use serde::{Deserialize, Serialize};

use super::ref_id::PublicFactRefId;

/// Public descriptor reference that omits descriptor hashes and artifact ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactDescriptorRef {
    /// Descriptor schema id.
    pub descriptor_schema_id: String,
    /// Subject schema id.
    pub subject_schema_id: String,
    /// Response schema id.
    pub response_schema_id: String,
}

/// Public fact field descriptor summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactFieldSummary {
    /// Stable descriptor-owned field id.
    pub field_id: String,
    /// Descriptor path shown to users.
    pub path: String,
    /// Field source: `subject`, `result`, or `metadata`.
    pub source: String,
    /// Field scalar type.
    pub value_type: String,
    /// Public exposure policy.
    pub exposure: String,
    /// Allowed query operators.
    pub operators: Vec<String>,
    /// Optional descriptor unit.
    pub unit: Option<String>,
    /// Optional base-10 scale exponent.
    pub scale: Option<i16>,
    /// Whether the field can participate in descriptor orderings.
    pub sortable: bool,
}

/// Public descriptor ordering summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactOrderingSummary {
    /// Ordering policy name.
    pub name: String,
    /// Ordered public term summaries.
    pub terms: Vec<PublicFactOrderingTermSummary>,
}

/// Public descriptor ordering term summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactOrderingTermSummary {
    /// Descriptor field id.
    pub field_id: String,
    /// Sort direction.
    pub direction: String,
    /// Null placement.
    pub nulls: String,
    /// Whether this term is a tie-breaker.
    pub tie_breaker: bool,
}

/// Public fact kind catalog summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactKindSummary {
    /// Fact kind.
    pub fact_kind: String,
    /// Number of public descriptors for the kind.
    pub descriptor_count: usize,
}

/// Public fact descriptor summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactDescriptorSummary {
    /// Fact kind.
    pub fact_kind: String,
    /// Public descriptor reference.
    pub descriptor: PublicFactDescriptorRef,
    /// Public-safe field summaries.
    pub fields: Vec<PublicFactFieldSummary>,
    /// Public-safe ordering summaries.
    pub orderings: Vec<PublicFactOrderingSummary>,
}

/// Public fact explanation for query construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactExplain {
    /// Fact kind.
    pub fact_kind: String,
    /// Matching public descriptors.
    pub descriptors: Vec<PublicFactDescriptorSummary>,
}

/// Scalar value returned by app public fact services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum PublicFactScalarValue {
    /// String scalar.
    String(String),
    /// Boolean scalar.
    Boolean(bool),
    /// Signed integer scalar.
    SignedInteger(i64),
    /// Unsigned integer scalar.
    UnsignedInteger(u64),
    /// Timestamp string scalar.
    Timestamp(String),
    /// Decimal string scalar.
    DecimalString(String),
    /// Digest string scalar.
    Digest(String),
}

impl fmt::Display for PublicFactScalarValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value)
            | Self::Timestamp(value)
            | Self::DecimalString(value)
            | Self::Digest(value) => f.write_str(value),
            Self::Boolean(value) => write!(f, "{value}"),
            Self::SignedInteger(value) => write!(f, "{value}"),
            Self::UnsignedInteger(value) => write!(f, "{value}"),
        }
    }
}

impl From<&mfm_facts::FactCanonicalScalar> for PublicFactScalarValue {
    fn from(value: &mfm_facts::FactCanonicalScalar) -> Self {
        match value {
            mfm_facts::FactCanonicalScalar::String(value) => Self::String(value.clone()),
            mfm_facts::FactCanonicalScalar::Boolean(value) => Self::Boolean(*value),
            mfm_facts::FactCanonicalScalar::SignedInteger(value) => Self::SignedInteger(*value),
            mfm_facts::FactCanonicalScalar::UnsignedInteger(value) => Self::UnsignedInteger(*value),
            mfm_facts::FactCanonicalScalar::Timestamp(value) => Self::Timestamp(value.clone()),
            mfm_facts::FactCanonicalScalar::DecimalString(value) => {
                Self::DecimalString(value.as_str().to_owned())
            }
            mfm_facts::FactCanonicalScalar::Digest(value) => {
                Self::Digest(value.as_str().to_owned())
            }
        }
    }
}

/// Public fact query page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactQueryPage {
    /// Returned public facts.
    pub facts: Vec<PublicFactRef>,
    /// Opaque cursor for a future page. V1 app services return no cursor.
    pub next_cursor: Option<String>,
}

/// Public fact reference returned by app public fact services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactRef {
    /// Opaque public reference id.
    pub public_ref: PublicFactRefId,
    /// Fact kind.
    pub fact_kind: String,
    /// Public descriptor reference.
    pub descriptor: PublicFactDescriptorRef,
    /// Store-assigned recorded timestamp.
    pub recorded_at: String,
    /// Optional source observation timestamp.
    pub observed_at: Option<String>,
    /// Descriptor-approved returnable fields.
    pub fields: Vec<PublicFactFieldValue>,
}

/// Public returned fact field value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicFactFieldValue {
    /// Stable descriptor-owned field id.
    pub field_id: String,
    /// Descriptor path shown to users.
    pub path: String,
    /// Field source: `subject`, `result`, or `metadata`.
    pub source: String,
    /// Field scalar type.
    pub value_type: String,
    /// Public-safe scalar value.
    pub value: PublicFactScalarValue,
    /// Optional descriptor unit.
    pub unit: Option<String>,
    /// Optional base-10 scale exponent.
    pub scale: Option<i16>,
}
