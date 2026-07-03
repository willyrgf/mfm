use serde::{Deserialize, Serialize};

use crate::*;

/// Source category for descriptor-declared fact field extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactFieldSource {
    /// Field is extracted from fact subject material.
    Subject,
    /// Field is extracted from fact response material.
    Result,
    /// Field is extracted from claim or store metadata.
    Metadata,
}

impl FactFieldSource {
    /// Returns the stable source prefix used by public descriptor paths.
    pub const fn path_prefix(self) -> &'static str {
        self.as_str()
    }
}

impl_fact_tag!(FactFieldSource, "fact field source", pub, "Returns the canonical source tag used in descriptors.", {
    Self::Subject => "subject",
    Self::Result => "result",
    Self::Metadata => "metadata",
});

/// Declarative extraction recipe for a fact field value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source", content = "path")]
pub enum FactFieldExtraction {
    /// Extract the field from canonical subject material.
    #[serde(rename = "subject")]
    SubjectPath(CanonicalValuePath),
    /// Extract the field from canonical response material.
    #[serde(rename = "result")]
    ResponsePath(CanonicalValuePath),
    /// Extract the field from claim or store metadata.
    Metadata(FactMetadataField),
}

impl FactFieldExtraction {
    /// Returns the source category represented by this extraction recipe.
    pub const fn source(&self) -> FactFieldSource {
        match self {
            Self::SubjectPath(_) => FactFieldSource::Subject,
            Self::ResponsePath(_) => FactFieldSource::Result,
            Self::Metadata(_) => FactFieldSource::Metadata,
        }
    }
}

/// Metadata fields that fact descriptors may index or order by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactMetadataField {
    /// Store-assigned time when the claim was recorded.
    RecordedAt,
    /// Optional source or domain observation time carried by the claim.
    ObservedAt,
    /// Store-owned append ordering coordinate.
    StoreCommitOrder,
}

impl FactMetadataField {
    /// Returns the expected value type for this metadata field.
    pub const fn value_type(self) -> FactFieldValueType {
        match self {
            Self::RecordedAt | Self::ObservedAt => FactFieldValueType::Timestamp,
            Self::StoreCommitOrder => FactFieldValueType::UnsignedInteger,
        }
    }
}

impl_fact_tag!(FactMetadataField, "fact metadata field", pub(crate), "Returns the canonical metadata field tag used in descriptors.", {
    Self::RecordedAt => "recorded_at",
    Self::ObservedAt => "observed_at",
    Self::StoreCommitOrder => "store_commit_order",
});

/// Scalar value type for an indexed fact field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactFieldValueType {
    /// UTF-8 string scalar.
    String,
    /// Boolean scalar.
    Boolean,
    /// Signed 64-bit integer scalar.
    SignedInteger,
    /// Unsigned 64-bit integer scalar.
    UnsignedInteger,
    /// Timestamp encoded as a normalized string.
    Timestamp,
    /// Bounded decimal encoded as a canonical decimal string.
    DecimalString,
    /// Typed digest string.
    Digest,
}

impl FactFieldValueType {
    /// Returns whether this value type supports the supplied query operator.
    pub const fn supports_operator(self, operator: FactQueryOperator) -> bool {
        match operator {
            FactQueryOperator::Equal => true,
            FactQueryOperator::LessThan
            | FactQueryOperator::LessThanOrEqual
            | FactQueryOperator::GreaterThan
            | FactQueryOperator::GreaterThanOrEqual => matches!(
                self,
                Self::SignedInteger | Self::UnsignedInteger | Self::Timestamp | Self::DecimalString
            ),
        }
    }

    /// Returns whether this value type has a descriptor-defined sortable encoding.
    pub const fn is_sortable(self) -> bool {
        matches!(
            self,
            Self::SignedInteger | Self::UnsignedInteger | Self::Timestamp | Self::DecimalString
        )
    }
}

impl_fact_tag!(FactFieldValueType, "field value type", pub, "Returns the canonical snake-case tag used in descriptors, queries, and receipts.", {
    Self::String => "string",
    Self::Boolean => "boolean",
    Self::SignedInteger => "signed_integer",
    Self::UnsignedInteger => "unsigned_integer",
    Self::Timestamp => "timestamp",
    Self::DecimalString => "decimal_string",
    Self::Digest => "digest",
});

/// Query operator allowed for a descriptor field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactQueryOperator {
    /// Equality comparison.
    Equal,
    /// Strict less-than comparison.
    LessThan,
    /// Less-than-or-equal comparison.
    LessThanOrEqual,
    /// Strict greater-than comparison.
    GreaterThan,
    /// Greater-than-or-equal comparison.
    GreaterThanOrEqual,
}

impl_fact_tag!(FactQueryOperator, "query operator", pub, "Returns the canonical snake-case tag used in descriptors and queries.", {
    Self::Equal => "equal",
    Self::LessThan => "less_than",
    Self::LessThanOrEqual => "less_than_or_equal",
    Self::GreaterThan => "greater_than",
    Self::GreaterThanOrEqual => "greater_than_or_equal",
});

/// Public exposure policy for a descriptor field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactFieldExposure {
    /// Public surfaces may return this field.
    Returnable,
    /// Public surfaces may filter or order by this field but must not return it.
    QueryOnly,
    /// Public surfaces must not filter, order, or return this field.
    Hidden,
}

impl_fact_tag!(FactFieldExposure, "field exposure", pub, "Returns the canonical snake-case tag used in descriptors.", {
    Self::Returnable => "returnable",
    Self::QueryOnly => "query_only",
    Self::Hidden => "hidden",
});

/// Base-10 scale attached to a fact field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FactScale {
    exponent: i16,
}

impl FactScale {
    /// Minimum supported base-10 exponent.
    pub const MIN_EXPONENT: i16 = -38;
    /// Maximum supported base-10 exponent.
    pub const MAX_EXPONENT: i16 = 38;

    /// Creates a bounded base-10 field scale.
    pub fn new(exponent: i16) -> Result<Self> {
        if !(Self::MIN_EXPONENT..=Self::MAX_EXPONENT).contains(&exponent) {
            return Err(FactDescriptorError::descriptor(format!(
                "scale exponent {exponent} outside supported range {}..={}",
                Self::MIN_EXPONENT,
                Self::MAX_EXPONENT
            )));
        }
        Ok(Self { exponent })
    }

    /// Returns the base-10 exponent.
    pub const fn exponent(self) -> i16 {
        self.exponent
    }
}

/// Fact visibility selected by the producer for a recorded claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactVisibility {
    /// Fact remains private run evidence and is not inserted into fact indexes.
    RunPrivate,
    /// Fact is indexed for the selected audience and scope.
    Indexed {
        /// Audience allowed to discover the indexed fact.
        audience: FactAudience,
        /// Visibility scope recorded with the indexed fact.
        scope: FactVisibilityScope,
    },
}

impl FactVisibility {
    /// Creates indexed visibility for the selected audience in the default scope.
    pub const fn indexed_default(audience: FactAudience) -> Self {
        Self::Indexed {
            audience,
            scope: FactVisibilityScope::Default,
        }
    }
}

/// Audience for an indexed fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactAudience {
    /// Internal operational audience for cross-run control facts.
    Control,
    /// Public platform fact audience, still subject to scope and exposure policy.
    Platform,
}

impl_fact_tag!(FactAudience, "fact audience", pub, "Returns the canonical snake-case tag used in query scopes and receipts.", {
    Self::Control => "control",
    Self::Platform => "platform",
});

/// Visibility scope for an indexed fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactVisibilityScope {
    /// V1 default scope.
    Default,
}

impl_fact_tag!(FactVisibilityScope, "fact visibility scope", pub, "Returns the canonical snake-case tag used in query scopes and receipts.", {
    Self::Default => "default",
});

/// Sort direction for a fact ordering term.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Ascending order.
    Ascending,
    /// Descending order.
    Descending,
}

impl_fact_tag!(SortDirection, "sort direction", pub, "Returns the canonical snake-case tag used in descriptor orderings.", {
    Self::Ascending => "ascending",
    Self::Descending => "descending",
});

/// Null placement for a fact ordering term.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullOrdering {
    /// Null values sort before non-null values.
    First,
    /// Null values sort after non-null values.
    Last,
}

impl_fact_tag!(NullOrdering, "null ordering", pub, "Returns the canonical snake-case tag used in descriptor orderings.", {
    Self::First => "first",
    Self::Last => "last",
});
