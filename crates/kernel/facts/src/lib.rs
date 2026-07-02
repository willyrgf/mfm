#![warn(missing_docs)]
//! Fact descriptor and query evidence contracts for the MFM typed kernel.
//!
//! This crate owns the domain-free facts kernel surface described by
//! `docs/RFC_COLLECTORS.md`: descriptor identity, field extraction contracts,
//! fact visibility, fact keys, claim identity, internal refs, and canonical
//! query evidence. The initial crate exists so later commits can add those
//! contracts behind the kernel dependency boundary without mixing them into
//! event, runtime, store, app, or collector code.
//!
//! ```
//! assert_eq!(mfm_facts::FACTS_KERNEL_CONTRACT_VERSION, "mfm.facts.v1");
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use mfm_canonical::{
    sha256_digest_bytes, CanonicalBytes, CanonicalJsonBytes, CanonicalValue, DecimalString,
    PlainCanonicalJsonBytes,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, EventId, RunId, SchemaId,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Stable facts-kernel contract version for the initial collectors RFC surface.
pub const FACTS_KERNEL_CONTRACT_VERSION: &str = "mfm.facts.v1";

/// V1 fact query compiler version recorded in canonical query plans.
pub const FACT_QUERY_COMPILER_VERSION: &str = "mfm.facts.query.v1";

/// V1 canonicalizer version recorded in canonical query plans.
pub const FACT_QUERY_CANONICALIZER_VERSION: &str = "mfm.canonical.v1";

/// Maximum UTF-8 byte length accepted for an extracted scalar in v1.
pub const MAX_FACT_SCALAR_BYTES: usize = 4096;

/// Result type for facts-kernel descriptor operations.
pub type Result<T> = std::result::Result<T, FactDescriptorError>;

/// Error returned when fact descriptors or fact descriptor primitives are invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactDescriptorError {
    /// A checked string primitive failed validation.
    #[error("{kind} {value:?} failed validation: {message}")]
    InvalidString {
        /// The checked string kind.
        kind: &'static str,
        /// The rejected value.
        value: String,
        /// Stable diagnostic.
        message: String,
    },
    /// A fact descriptor failed validation.
    #[error("fact descriptor error: {message}")]
    Descriptor {
        /// Stable diagnostic.
        message: String,
    },
    /// A fact field descriptor failed validation.
    #[error("fact field {field_id} error: {message}")]
    Field {
        /// Field id that failed validation.
        field_id: FactFieldId,
        /// Stable diagnostic.
        message: String,
    },
    /// A fact ordering descriptor failed validation.
    #[error("fact ordering {ordering} error: {message}")]
    Ordering {
        /// Ordering name that failed validation.
        ordering: FactOrderingName,
        /// Stable diagnostic.
        message: String,
    },
    /// Canonicalization failed.
    #[error("fact canonicalization error: {message}")]
    Canonical {
        /// Stable diagnostic.
        message: String,
    },
}

impl FactDescriptorError {
    fn invalid_string(kind: &'static str, value: &str, message: impl Into<String>) -> Self {
        Self::InvalidString {
            kind,
            value: value.to_owned(),
            message: message.into(),
        }
    }

    /// Creates a descriptor validation error with a stable diagnostic.
    pub fn descriptor(message: impl Into<String>) -> Self {
        Self::Descriptor {
            message: message.into(),
        }
    }

    fn field(field_id: FactFieldId, message: impl Into<String>) -> Self {
        Self::Field {
            field_id,
            message: message.into(),
        }
    }

    fn ordering(ordering: FactOrderingName, message: impl Into<String>) -> Self {
        Self::Ordering {
            ordering,
            message: message.into(),
        }
    }

    fn canonical(message: impl Into<String>) -> Self {
        Self::Canonical {
            message: message.into(),
        }
    }
}

macro_rules! checked_fact_string {
    ($ty:ident, $kind:literal, $validator:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            #[doc = concat!("Creates a checked `", stringify!($ty), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                $validator($kind, value)?;
                Ok(Self {
                    raw: value.to_owned(),
                })
            }

            #[doc = concat!("Returns this `", stringify!($ty), "` as a string slice.")]
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            #[doc = concat!("Consumes this `", stringify!($ty), "` into its owned string.")]
            pub fn into_string(self) -> String {
                self.raw
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $ty {
            type Err = FactDescriptorError;

            fn from_str(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = FactDescriptorError;

            fn try_from(value: String) -> Result<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $ty {
            type Error = FactDescriptorError;

            fn try_from(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.raw
            }
        }

        impl Serialize for $ty {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(&value).map_err(serde::de::Error::custom)
            }
        }
    };
}

checked_fact_string!(
    FactKind,
    "fact kind",
    validate_dot_path,
    "Stable fact kind, such as `wallet.balance` or `chain.head`."
);

checked_fact_string!(
    FactFieldId,
    "fact field id",
    validate_dot_path,
    "Descriptor-owned stable field identifier."
);

checked_fact_string!(
    FactFieldPath,
    "fact field path",
    validate_qualified_field_path,
    "Human-readable descriptor field path with a source prefix."
);

checked_fact_string!(
    FactOrderingName,
    "fact ordering name",
    validate_dot_path,
    "Descriptor-owned ordering policy name."
);

checked_fact_string!(
    FactUnit,
    "fact unit",
    validate_dot_path,
    "Descriptor-owned unit identifier for a fact field."
);

checked_fact_string!(
    FactCompatibilityGroup,
    "fact compatibility group",
    validate_dot_path,
    "Descriptor compatibility group used for shape selection."
);

checked_fact_string!(
    CanonicalValuePath,
    "canonical value path",
    validate_dot_path,
    "Relative object path over canonical subject or response value material."
);

checked_fact_string!(
    StoreScopeRef,
    "store scope ref",
    validate_dot_path,
    "Non-secret store scope reference recorded in fact query evidence."
);

checked_fact_string!(
    FactQueryCompilerVersion,
    "fact query compiler version",
    validate_dot_path,
    "Version of the fact query compiler that produced a canonical query plan."
);

checked_fact_string!(
    FactCanonicalizerVersion,
    "fact canonicalizer version",
    validate_dot_path,
    "Version of the canonicalizer that produced fact query evidence."
);

checked_fact_string!(
    StoreIdentity,
    "store identity",
    validate_dot_path,
    "Non-secret store identity used by receipt authentication."
);

checked_fact_string!(
    StoreKeyId,
    "store key id",
    validate_dot_path,
    "Non-secret store receipt authentication key identifier."
);

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
        match self {
            Self::Subject => "subject",
            Self::Result => "result",
            Self::Metadata => "metadata",
        }
    }

    fn as_str(self) -> &'static str {
        self.path_prefix()
    }
}

/// Declarative source of a fact field value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source", content = "path")]
pub enum FactFieldAccessor {
    /// Extract the field from canonical subject material.
    #[serde(rename = "subject")]
    SubjectPath(CanonicalValuePath),
    /// Extract the field from canonical response material.
    #[serde(rename = "result")]
    ResponsePath(CanonicalValuePath),
    /// Extract the field from claim or store metadata.
    Metadata(FactMetadataField),
}

impl FactFieldAccessor {
    /// Returns the source category represented by this accessor.
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

    fn as_str(self) -> &'static str {
        match self {
            Self::RecordedAt => "recorded_at",
            Self::ObservedAt => "observed_at",
            Self::StoreCommitOrder => "store_commit_order",
        }
    }
}

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

    fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::SignedInteger => "signed_integer",
            Self::UnsignedInteger => "unsigned_integer",
            Self::Timestamp => "timestamp",
            Self::DecimalString => "decimal_string",
            Self::Digest => "digest",
        }
    }
}

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

impl FactQueryOperator {
    fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "equal",
            Self::LessThan => "less_than",
            Self::LessThanOrEqual => "less_than_or_equal",
            Self::GreaterThan => "greater_than",
            Self::GreaterThanOrEqual => "greater_than_or_equal",
        }
    }
}

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

impl FactFieldExposure {
    fn as_str(self) -> &'static str {
        match self {
            Self::Returnable => "returnable",
            Self::QueryOnly => "query_only",
            Self::Hidden => "hidden",
        }
    }
}

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

impl FactAudience {
    fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Platform => "platform",
        }
    }
}

/// Visibility scope for an indexed fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactVisibilityScope {
    /// V1 default scope.
    Default,
}

impl FactVisibilityScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
        }
    }
}

/// Sort direction for a fact ordering term.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Ascending order.
    Ascending,
    /// Descending order.
    Descending,
}

impl SortDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ascending => "ascending",
            Self::Descending => "descending",
        }
    }
}

/// Null placement for a fact ordering term.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullOrdering {
    /// Null values sort before non-null values.
    First,
    /// Null values sort after non-null values.
    Last,
}

impl NullOrdering {
    fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Last => "last",
        }
    }
}

/// One term in a descriptor-defined fact ordering policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactOrderingTerm {
    field_id: FactFieldId,
    direction: SortDirection,
    nulls: NullOrdering,
    tie_breaker: bool,
}

impl FactOrderingTerm {
    /// Creates an ordering term for one field.
    pub fn new(
        field_id: FactFieldId,
        direction: SortDirection,
        nulls: NullOrdering,
        tie_breaker: bool,
    ) -> Self {
        Self {
            field_id,
            direction,
            nulls,
            tie_breaker,
        }
    }

    /// Returns the field id used by this ordering term.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns the sort direction.
    pub const fn direction(&self) -> SortDirection {
        self.direction
    }

    /// Returns the null ordering.
    pub const fn nulls(&self) -> NullOrdering {
        self.nulls
    }

    /// Returns whether this term is a deterministic tie-breaker.
    pub const fn tie_breaker(&self) -> bool {
        self.tie_breaker
    }
}

/// Descriptor-defined fact ordering policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactOrderingDescriptor {
    name: FactOrderingName,
    terms: Vec<FactOrderingTerm>,
}

impl FactOrderingDescriptor {
    /// Creates a descriptor-defined ordering policy.
    pub fn new(name: FactOrderingName, terms: Vec<FactOrderingTerm>) -> Result<Self> {
        if terms.is_empty() {
            return Err(FactDescriptorError::ordering(
                name,
                "ordering must contain at least one term",
            ));
        }

        Ok(Self { name, terms })
    }

    /// Returns this ordering name.
    pub const fn name(&self) -> &FactOrderingName {
        &self.name
    }

    /// Returns ordering terms in retained order.
    pub fn terms(&self) -> &[FactOrderingTerm] {
        &self.terms
    }
}

/// One descriptor-declared field that can produce fact index terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactFieldDescriptor {
    field_id: FactFieldId,
    path: FactFieldPath,
    value_type: FactFieldValueType,
    accessor: FactFieldAccessor,
    operators: Vec<FactQueryOperator>,
    exposure: FactFieldExposure,
    unit: Option<FactUnit>,
    scale: Option<FactScale>,
    sortable: bool,
    required: bool,
}

impl FactFieldDescriptor {
    /// Creates a fact field descriptor with validation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        field_id: FactFieldId,
        path: FactFieldPath,
        value_type: FactFieldValueType,
        accessor: FactFieldAccessor,
        operators: Vec<FactQueryOperator>,
        exposure: FactFieldExposure,
        unit: Option<FactUnit>,
        scale: Option<FactScale>,
        sortable: bool,
        required: bool,
    ) -> Result<Self> {
        let descriptor = Self {
            field_id,
            path,
            value_type,
            accessor,
            operators,
            exposure,
            unit,
            scale,
            sortable,
            required,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Returns this field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns this field path.
    pub const fn path(&self) -> &FactFieldPath {
        &self.path
    }

    /// Returns this field value type.
    pub const fn value_type(&self) -> FactFieldValueType {
        self.value_type
    }

    /// Returns this field accessor.
    pub const fn accessor(&self) -> &FactFieldAccessor {
        &self.accessor
    }

    /// Returns allowed query operators in retained order.
    pub fn operators(&self) -> &[FactQueryOperator] {
        &self.operators
    }

    /// Returns this field exposure policy.
    pub const fn exposure(&self) -> FactFieldExposure {
        self.exposure
    }

    /// Returns this field unit.
    pub const fn unit(&self) -> Option<&FactUnit> {
        self.unit.as_ref()
    }

    /// Returns this field scale.
    pub const fn scale(&self) -> Option<FactScale> {
        self.scale
    }

    /// Returns whether this field may be used in descriptor orderings.
    pub const fn sortable(&self) -> bool {
        self.sortable
    }

    /// Returns whether indexed facts must provide this field.
    pub const fn required(&self) -> bool {
        self.required
    }

    fn validate(&self) -> Result<()> {
        let expected_prefix = self.accessor.source().path_prefix();
        if self.path.source_prefix() != Some(expected_prefix) {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                format!(
                    "path prefix must be {expected_prefix:?} for {:?} accessor",
                    self.accessor.source()
                ),
            ));
        }

        if matches!(self.accessor, FactFieldAccessor::Metadata(_))
            && self.accessor_metadata_value_type() != Some(self.value_type)
        {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                "metadata field value type does not match metadata source",
            ));
        }

        if self.operators.is_empty() {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                "field must allow at least one query operator",
            ));
        }

        let mut seen = BTreeSet::new();
        for operator in &self.operators {
            if !seen.insert(*operator) {
                return Err(FactDescriptorError::field(
                    self.field_id.clone(),
                    format!("duplicate operator {operator:?}"),
                ));
            }
            if !self.value_type.supports_operator(*operator) {
                return Err(FactDescriptorError::field(
                    self.field_id.clone(),
                    format!(
                        "operator {operator:?} is incompatible with value type {:?}",
                        self.value_type
                    ),
                ));
            }
        }

        if self.sortable && !self.value_type.is_sortable() {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                format!("value type {:?} is not sortable", self.value_type),
            ));
        }

        Ok(())
    }

    fn accessor_metadata_value_type(&self) -> Option<FactFieldValueType> {
        match self.accessor {
            FactFieldAccessor::Metadata(field) => Some(field.value_type()),
            _ => None,
        }
    }
}

/// Durable descriptor for a fact shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactDescriptor {
    fact_kind: FactKind,
    #[serde(with = "schema_id_serde")]
    descriptor_schema_id: SchemaId,
    #[serde(with = "schema_id_serde")]
    subject_schema_id: SchemaId,
    #[serde(with = "schema_id_serde")]
    response_schema_id: SchemaId,
    compatibility_group: Option<FactCompatibilityGroup>,
    fields: Vec<FactFieldDescriptor>,
    orderings: Vec<FactOrderingDescriptor>,
}

impl FactDescriptor {
    /// Creates a validated fact descriptor.
    pub fn new(
        fact_kind: FactKind,
        descriptor_schema_id: SchemaId,
        subject_schema_id: SchemaId,
        response_schema_id: SchemaId,
        compatibility_group: Option<FactCompatibilityGroup>,
        fields: Vec<FactFieldDescriptor>,
        orderings: Vec<FactOrderingDescriptor>,
    ) -> Result<Self> {
        let descriptor = Self {
            fact_kind,
            descriptor_schema_id,
            subject_schema_id,
            response_schema_id,
            compatibility_group,
            fields,
            orderings,
        };
        validate_descriptor(&descriptor)?;
        Ok(descriptor)
    }

    /// Returns the fact kind.
    pub const fn fact_kind(&self) -> &FactKind {
        &self.fact_kind
    }

    /// Returns the descriptor schema id.
    pub const fn descriptor_schema_id(&self) -> &SchemaId {
        &self.descriptor_schema_id
    }

    /// Returns the subject schema id.
    pub const fn subject_schema_id(&self) -> &SchemaId {
        &self.subject_schema_id
    }

    /// Returns the response schema id.
    pub const fn response_schema_id(&self) -> &SchemaId {
        &self.response_schema_id
    }

    /// Returns the descriptor compatibility group.
    pub const fn compatibility_group(&self) -> Option<&FactCompatibilityGroup> {
        self.compatibility_group.as_ref()
    }

    /// Returns fields in retained descriptor order.
    pub fn fields(&self) -> &[FactFieldDescriptor] {
        &self.fields
    }

    /// Returns ordering policies in retained descriptor order.
    pub fn orderings(&self) -> &[FactOrderingDescriptor] {
        &self.orderings
    }
}

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

    fn canonical_value(&self) -> CanonicalValue {
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

/// Claim and store metadata available to descriptor field extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactExtractionMetadata {
    recorded_at: String,
    observed_at: Option<String>,
    store_commit_order: u64,
}

impl FactExtractionMetadata {
    /// Creates extraction metadata for fact term derivation.
    pub fn new(
        recorded_at: impl Into<String>,
        observed_at: Option<impl Into<String>>,
        store_commit_order: u64,
    ) -> Result<Self> {
        let recorded_at = recorded_at.into();
        if recorded_at.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "recorded_at metadata must not be empty",
            ));
        }
        let observed_at = observed_at.map(Into::into);
        if observed_at.as_ref().is_some_and(|value| value.is_empty()) {
            return Err(FactDescriptorError::descriptor(
                "observed_at metadata must not be empty when present",
            ));
        }
        Ok(Self {
            recorded_at,
            observed_at,
            store_commit_order,
        })
    }

    /// Returns the store-assigned recorded time.
    pub fn recorded_at(&self) -> &str {
        &self.recorded_at
    }

    /// Returns the optional source observation time.
    pub fn observed_at(&self) -> Option<&str> {
        self.observed_at.as_deref()
    }

    /// Returns the store-owned commit ordering coordinate.
    pub const fn store_commit_order(&self) -> u64 {
        self.store_commit_order
    }
}

/// One extracted index term for an indexed fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexTerm {
    field_id: FactFieldId,
    source: FactFieldSource,
    value_type: FactFieldValueType,
    value: FactCanonicalScalar,
    unit: Option<FactUnit>,
    scale: Option<FactScale>,
}

impl FactIndexTerm {
    /// Creates a term from a descriptor field and extracted scalar.
    pub fn from_field(field: &FactFieldDescriptor, value: FactCanonicalScalar) -> Result<Self> {
        validate_extracted_scalar(field, &value)?;
        Ok(Self {
            field_id: field.field_id.clone(),
            source: field.accessor.source(),
            value_type: field.value_type,
            value,
            unit: field.unit.clone(),
            scale: field.scale,
        })
    }

    /// Returns this term's field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns the source category for this term.
    pub const fn source(&self) -> FactFieldSource {
        self.source
    }

    /// Returns this term's value type.
    pub const fn value_type(&self) -> FactFieldValueType {
        self.value_type
    }

    /// Returns this term's scalar value.
    pub const fn value(&self) -> &FactCanonicalScalar {
        &self.value
    }

    /// Returns this term's unit.
    pub const fn unit(&self) -> Option<&FactUnit> {
        self.unit.as_ref()
    }

    /// Returns this term's scale.
    pub const fn scale(&self) -> Option<FactScale> {
        self.scale
    }
}

/// Query-time visibility scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryScope {
    audience: FactAudience,
    scope: FactVisibilityScope,
}

impl FactQueryScope {
    /// Creates a fact query scope.
    pub const fn new(audience: FactAudience, scope: FactVisibilityScope) -> Self {
        Self { audience, scope }
    }

    /// Returns the query audience.
    pub const fn audience(&self) -> FactAudience {
        self.audience
    }

    /// Returns the query visibility scope.
    pub const fn scope(&self) -> FactVisibilityScope {
        self.scope
    }
}

/// Scope decision evidence bound into a canonical fact query plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDecisionEvidence {
    decision_hash: ContentDigest,
}

impl ScopeDecisionEvidence {
    /// Creates scope decision evidence from a policy or authorization digest.
    pub const fn new(decision_hash: ContentDigest) -> Self {
        Self { decision_hash }
    }

    /// Returns the decision digest.
    pub const fn decision_hash(&self) -> &ContentDigest {
        &self.decision_hash
    }
}

/// One descriptor-field predicate requested for a fact query.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactQueryPredicate {
    field_id: FactFieldId,
    operator: FactQueryOperator,
    value: FactCanonicalScalar,
}

impl FactQueryPredicate {
    /// Creates a fact query predicate.
    pub fn new(
        field_id: FactFieldId,
        operator: FactQueryOperator,
        value: FactCanonicalScalar,
    ) -> Self {
        Self {
            field_id,
            operator,
            value,
        }
    }

    /// Returns the descriptor field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns the query operator.
    pub const fn operator(&self) -> FactQueryOperator {
        self.operator
    }

    /// Returns the predicate scalar.
    pub const fn value(&self) -> &FactCanonicalScalar {
        &self.value
    }
}

/// One descriptor field requested in fact query results.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactQueryReturnField {
    field_id: FactFieldId,
}

impl FactQueryReturnField {
    /// Creates a requested return field.
    pub fn new(field_id: FactFieldId) -> Self {
        Self { field_id }
    }

    /// Returns the descriptor field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }
}

/// Descriptor-scoped fact query request accepted by the v1 compiler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryInput {
    store_scope: StoreScopeRef,
    query_scope: FactQueryScope,
    scope_decision_evidence: ScopeDecisionEvidence,
    predicates: Vec<FactQueryPredicate>,
    return_fields: Vec<FactQueryReturnField>,
    ordering: FactOrderingName,
    limit: Option<u64>,
}

impl FactQueryInput {
    /// Creates a descriptor-scoped query compiler input.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope: StoreScopeRef,
        query_scope: FactQueryScope,
        scope_decision_evidence: ScopeDecisionEvidence,
        predicates: Vec<FactQueryPredicate>,
        return_fields: Vec<FactQueryReturnField>,
        ordering: FactOrderingName,
        limit: Option<u64>,
    ) -> Result<Self> {
        if return_fields.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "fact query must request at least one return field",
            ));
        }
        if limit == Some(0) {
            return Err(FactDescriptorError::descriptor(
                "fact query limit must be non-zero when present",
            ));
        }
        let mut seen_predicates = BTreeSet::new();
        for predicate in &predicates {
            if !seen_predicates.insert((
                predicate.field_id.clone(),
                predicate.operator,
                predicate.value.clone(),
            )) {
                return Err(FactDescriptorError::field(
                    predicate.field_id.clone(),
                    "duplicate fact query predicate",
                ));
            }
        }
        let mut seen_return_fields = BTreeSet::new();
        for field in &return_fields {
            if !seen_return_fields.insert(field.field_id.clone()) {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "duplicate fact query return field",
                ));
            }
        }
        Ok(Self {
            store_scope,
            query_scope,
            scope_decision_evidence,
            predicates,
            return_fields,
            ordering,
            limit,
        })
    }

    /// Returns the store scope.
    pub const fn store_scope(&self) -> &StoreScopeRef {
        &self.store_scope
    }

    /// Returns the query scope.
    pub const fn query_scope(&self) -> &FactQueryScope {
        &self.query_scope
    }

    /// Returns scope decision evidence.
    pub const fn scope_decision_evidence(&self) -> &ScopeDecisionEvidence {
        &self.scope_decision_evidence
    }

    /// Returns requested predicates in caller order.
    pub fn predicates(&self) -> &[FactQueryPredicate] {
        &self.predicates
    }

    /// Returns requested result fields in caller order.
    pub fn return_fields(&self) -> &[FactQueryReturnField] {
        &self.return_fields
    }

    /// Returns the selected descriptor ordering name.
    pub const fn ordering(&self) -> &FactOrderingName {
        &self.ordering
    }

    /// Returns the optional result limit.
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }
}

/// Parsed descriptor-scoped query shape from canonical query bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFactQueryShape {
    predicates: Vec<FactQueryPredicate>,
    return_fields: Vec<FactQueryReturnField>,
}

impl CompiledFactQueryShape {
    /// Creates a parsed query shape.
    pub fn new(
        predicates: Vec<FactQueryPredicate>,
        return_fields: Vec<FactQueryReturnField>,
    ) -> Result<Self> {
        if return_fields.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "compiled fact query shape must contain return fields",
            ));
        }
        Ok(Self {
            predicates,
            return_fields,
        })
    }

    /// Returns canonical predicates.
    pub fn predicates(&self) -> &[FactQueryPredicate] {
        &self.predicates
    }

    /// Returns requested return fields.
    pub fn return_fields(&self) -> &[FactQueryReturnField] {
        &self.return_fields
    }
}

/// Ordering selected for a canonical fact query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactOrdering {
    name: FactOrderingName,
    terms: Vec<FactOrderingTerm>,
}

impl FactOrdering {
    /// Creates a selected fact ordering.
    pub fn new(name: FactOrderingName, terms: Vec<FactOrderingTerm>) -> Result<Self> {
        FactOrderingDescriptor::new(name.clone(), terms.clone())?;
        Ok(Self { name, terms })
    }

    /// Creates a selected fact ordering from a descriptor ordering.
    pub fn from_descriptor(ordering: &FactOrderingDescriptor) -> Self {
        Self {
            name: ordering.name.clone(),
            terms: ordering.terms.clone(),
        }
    }

    /// Returns this ordering name.
    pub const fn name(&self) -> &FactOrderingName {
        &self.name
    }

    /// Returns ordering terms in retained order.
    pub fn terms(&self) -> &[FactOrderingTerm] {
        &self.terms
    }
}

/// Canonical single-descriptor v1 fact query plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFactQueryPlan {
    store_scope: StoreScopeRef,
    query_scope: FactQueryScope,
    query_compiler_version: FactQueryCompilerVersion,
    canonicalizer_version: FactCanonicalizerVersion,
    resolved_descriptor: ContentDigest,
    scope_decision_evidence: ScopeDecisionEvidence,
    canonical_query: CanonicalJsonBytes,
    canonical_query_hash: ContentDigest,
    ordering: FactOrdering,
    limit: Option<u64>,
}

impl CanonicalFactQueryPlan {
    /// Creates a canonical fact query plan and computes its query hash.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope: StoreScopeRef,
        query_scope: FactQueryScope,
        query_compiler_version: FactQueryCompilerVersion,
        canonicalizer_version: FactCanonicalizerVersion,
        resolved_descriptor: ContentDigest,
        scope_decision_evidence: ScopeDecisionEvidence,
        canonical_query: CanonicalJsonBytes,
        ordering: FactOrdering,
        limit: Option<u64>,
    ) -> Result<Self> {
        if limit == Some(0) {
            return Err(FactDescriptorError::descriptor(
                "fact query limit must be non-zero when present",
            ));
        }
        let canonical_query_hash = canonical_query.content_digest();
        Ok(Self {
            store_scope,
            query_scope,
            query_compiler_version,
            canonicalizer_version,
            resolved_descriptor,
            scope_decision_evidence,
            canonical_query,
            canonical_query_hash,
            ordering,
            limit,
        })
    }

    /// Returns the store scope.
    pub const fn store_scope(&self) -> &StoreScopeRef {
        &self.store_scope
    }

    /// Returns the query scope.
    pub const fn query_scope(&self) -> &FactQueryScope {
        &self.query_scope
    }

    /// Returns the query compiler version.
    pub const fn query_compiler_version(&self) -> &FactQueryCompilerVersion {
        &self.query_compiler_version
    }

    /// Returns the canonicalizer version.
    pub const fn canonicalizer_version(&self) -> &FactCanonicalizerVersion {
        &self.canonicalizer_version
    }

    /// Returns the resolved descriptor hash.
    pub const fn resolved_descriptor(&self) -> &ContentDigest {
        &self.resolved_descriptor
    }

    /// Returns scope decision evidence.
    pub const fn scope_decision_evidence(&self) -> &ScopeDecisionEvidence {
        &self.scope_decision_evidence
    }

    /// Returns canonical query bytes.
    pub const fn canonical_query(&self) -> &CanonicalJsonBytes {
        &self.canonical_query
    }

    /// Returns the canonical query hash.
    pub const fn canonical_query_hash(&self) -> &ContentDigest {
        &self.canonical_query_hash
    }

    /// Returns the selected ordering.
    pub const fn ordering(&self) -> &FactOrdering {
        &self.ordering
    }

    /// Returns the optional query limit.
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }
}

/// Canonical subject namespace for a fact descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectNamespaceV1 {
    fact_kind: FactKind,
    fields: Vec<FactSubjectNamespaceFieldV1>,
}

impl FactSubjectNamespaceV1 {
    /// Stable subject namespace version string.
    pub const VERSION: &'static str = "mfm.fact-subject-namespace.v1";

    /// Creates a subject namespace and sorts fields by field id.
    pub fn new(fact_kind: FactKind, fields: Vec<FactSubjectNamespaceFieldV1>) -> Result<Self> {
        let mut namespace = Self { fact_kind, fields };
        namespace.sort_and_validate()?;
        Ok(namespace)
    }

    /// Returns the namespace fact kind.
    pub const fn fact_kind(&self) -> &FactKind {
        &self.fact_kind
    }

    /// Returns subject namespace fields sorted by field id.
    pub fn fields(&self) -> &[FactSubjectNamespaceFieldV1] {
        &self.fields
    }

    fn sort_and_validate(&mut self) -> Result<()> {
        self.fields
            .sort_by(|left, right| left.field_id.cmp(&right.field_id));
        let mut seen = BTreeSet::new();
        for field in &self.fields {
            if !seen.insert(field.field_id.clone()) {
                return Err(FactDescriptorError::descriptor(format!(
                    "duplicate subject namespace field {}",
                    field.field_id
                )));
            }
        }
        if self.fields.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "subject namespace must contain at least one field",
            ));
        }
        Ok(())
    }
}

/// Subject namespace field that participates in fact-key derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectNamespaceFieldV1 {
    field_id: FactFieldId,
    value_type: FactFieldValueType,
    unit: Option<FactUnit>,
    scale: Option<FactScale>,
}

impl FactSubjectNamespaceFieldV1 {
    /// Creates a subject namespace field.
    pub fn new(
        field_id: FactFieldId,
        value_type: FactFieldValueType,
        unit: Option<FactUnit>,
        scale: Option<FactScale>,
    ) -> Self {
        Self {
            field_id,
            value_type,
            unit,
            scale,
        }
    }

    /// Returns this field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns this field value type.
    pub const fn value_type(&self) -> FactFieldValueType {
        self.value_type
    }

    /// Returns this field unit.
    pub const fn unit(&self) -> Option<&FactUnit> {
        self.unit.as_ref()
    }

    /// Returns this field scale.
    pub const fn scale(&self) -> Option<FactScale> {
        self.scale
    }
}

/// Canonical subject material for one fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectMaterialV1 {
    values: Vec<FactSubjectValueV1>,
}

impl FactSubjectMaterialV1 {
    /// Stable subject material version string.
    pub const VERSION: &'static str = "mfm.fact-subject-material.v1";

    /// Creates subject material and sorts values by field id.
    pub fn new(values: Vec<FactSubjectValueV1>) -> Result<Self> {
        let mut material = Self { values };
        material.sort_and_validate()?;
        Ok(material)
    }

    /// Returns subject values sorted by field id.
    pub fn values(&self) -> &[FactSubjectValueV1] {
        &self.values
    }

    fn sort_and_validate(&mut self) -> Result<()> {
        self.values
            .sort_by(|left, right| left.field_id.cmp(&right.field_id));
        let mut seen = BTreeSet::new();
        for value in &self.values {
            if !seen.insert(value.field_id.clone()) {
                return Err(FactDescriptorError::descriptor(format!(
                    "duplicate subject material field {}",
                    value.field_id
                )));
            }
        }
        if self.values.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "subject material must contain at least one value",
            ));
        }
        Ok(())
    }
}

/// One canonical subject value for fact-key derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectValueV1 {
    field_id: FactFieldId,
    value_type: FactFieldValueType,
    value: FactCanonicalScalar,
}

impl FactSubjectValueV1 {
    /// Creates a subject value and checks that the scalar matches the declared value type.
    pub fn new(
        field_id: FactFieldId,
        value_type: FactFieldValueType,
        value: FactCanonicalScalar,
    ) -> Result<Self> {
        if value.value_type() != value_type {
            return Err(FactDescriptorError::field(
                field_id,
                format!(
                    "subject value type {:?} does not match scalar type {:?}",
                    value_type,
                    value.value_type()
                ),
            ));
        }
        validate_scalar_size(&field_id, &value)?;
        Ok(Self {
            field_id,
            value_type,
            value,
        })
    }

    /// Returns this value's field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns this value's declared type.
    pub const fn value_type(&self) -> FactFieldValueType {
        self.value_type
    }

    /// Returns this value's scalar.
    pub const fn value(&self) -> &FactCanonicalScalar {
        &self.value
    }
}

/// Stable content-derived key for a fact subject.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactKey {
    digest: ContentDigest,
}

impl FactKey {
    /// Creates a fact key from already-derived digest identity.
    pub const fn from_digest(digest: ContentDigest) -> Self {
        Self { digest }
    }

    /// Returns the underlying content digest.
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Returns the persisted fact key string.
    pub fn as_str(&self) -> &str {
        self.digest.as_str()
    }
}

impl fmt::Display for FactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable identity for one recorded fact claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactClaimId {
    source_run_id: RunId,
    source_seq: u64,
    source_ordinal: u32,
}

impl FactClaimId {
    /// Creates a claim id from run-stream coordinates.
    pub fn new(source_run_id: RunId, source_seq: u64, source_ordinal: u32) -> Result<Self> {
        if source_seq == 0 {
            return Err(FactDescriptorError::descriptor(
                "fact claim source sequence must be non-zero",
            ));
        }
        Ok(Self {
            source_run_id,
            source_seq,
            source_ordinal,
        })
    }

    /// Returns the source run id.
    pub const fn source_run_id(&self) -> &RunId {
        &self.source_run_id
    }

    /// Returns the source run stream sequence.
    pub const fn source_seq(&self) -> u64 {
        self.source_seq
    }

    /// Returns the source event ordinal inside the atomic append.
    pub const fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }
}

/// Descriptor-derived subject authority recorded with a fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectEvidence {
    fact_subject_namespace_hash: ContentDigest,
    subject_material: PlainCanonicalJsonBytes,
    subject_material_hash: ContentDigest,
    fact_key: FactKey,
}

impl FactSubjectEvidence {
    /// Creates subject evidence from canonical subject material values.
    pub fn from_material(
        fact_subject_namespace_hash: ContentDigest,
        material: &FactSubjectMaterialV1,
    ) -> Result<Self> {
        let material_bytes = canonical_fact_subject_material_bytes(material)?;
        let subject_material =
            PlainCanonicalJsonBytes::from_canonical_json_slice(material_bytes.as_bytes())
                .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
        let subject_material_hash = subject_material.content_digest();
        let fact_key = derive_fact_key(
            fact_subject_namespace_hash.clone(),
            subject_material_hash.clone(),
        )?;
        Self::new(
            fact_subject_namespace_hash,
            subject_material,
            subject_material_hash,
            fact_key,
        )
    }

    /// Creates subject evidence from canonical descriptor and subject material authority.
    pub fn new(
        fact_subject_namespace_hash: ContentDigest,
        subject_material: PlainCanonicalJsonBytes,
        subject_material_hash: ContentDigest,
        fact_key: FactKey,
    ) -> Result<Self> {
        if subject_material.content_digest() != subject_material_hash {
            return Err(FactDescriptorError::descriptor(
                "subject material hash does not match subject material bytes",
            ));
        }
        let expected_fact_key = derive_fact_key(
            fact_subject_namespace_hash.clone(),
            subject_material_hash.clone(),
        )?;
        if expected_fact_key != fact_key {
            return Err(FactDescriptorError::descriptor(
                "fact key does not match subject namespace and material hashes",
            ));
        }
        Ok(Self {
            fact_subject_namespace_hash,
            subject_material,
            subject_material_hash,
            fact_key,
        })
    }

    /// Returns the descriptor-derived subject namespace hash.
    pub const fn fact_subject_namespace_hash(&self) -> &ContentDigest {
        &self.fact_subject_namespace_hash
    }

    /// Returns the canonical subject material bytes.
    pub const fn subject_material(&self) -> &PlainCanonicalJsonBytes {
        &self.subject_material
    }

    /// Returns the canonical subject material hash.
    pub const fn subject_material_hash(&self) -> &ContentDigest {
        &self.subject_material_hash
    }

    /// Returns the derived subject fact key.
    pub const fn fact_key(&self) -> &FactKey {
        &self.fact_key
    }
}

/// Optional request evidence recorded with a fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRequestEvidence {
    request_schema_id: SchemaId,
    request_hash: ContentDigest,
}

impl FactRequestEvidence {
    /// Creates request evidence from canonical request authority.
    pub fn new(request_schema_id: SchemaId, request_hash: ContentDigest) -> Self {
        Self {
            request_schema_id,
            request_hash,
        }
    }

    /// Returns the request schema id.
    pub const fn request_schema_id(&self) -> &SchemaId {
        &self.request_schema_id
    }

    /// Returns the canonical request hash.
    pub const fn request_hash(&self) -> &ContentDigest {
        &self.request_hash
    }
}

/// Response artifact evidence recorded with a fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactResponseEvidence {
    response_schema_id: SchemaId,
    response_hash: ContentDigest,
    artifact_id: ArtifactId,
    artifact_evidence_hash: ContentDigest,
}

impl FactResponseEvidence {
    /// Creates response artifact evidence.
    pub fn new(
        response_schema_id: SchemaId,
        response_hash: ContentDigest,
        artifact_id: ArtifactId,
        artifact_evidence_hash: ContentDigest,
    ) -> Self {
        Self {
            response_schema_id,
            response_hash,
            artifact_id,
            artifact_evidence_hash,
        }
    }

    /// Returns the response schema id.
    pub const fn response_schema_id(&self) -> &SchemaId {
        &self.response_schema_id
    }

    /// Returns the canonical response hash.
    pub const fn response_hash(&self) -> &ContentDigest {
        &self.response_hash
    }

    /// Returns the response artifact id.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// Returns the response artifact evidence hash.
    pub const fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.artifact_evidence_hash
    }
}

/// Certified runtime provenance for a producing fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactProducerProvenance {
    capability_kind: CapabilityKind,
    capability_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
}

impl FactProducerProvenance {
    /// Creates producer provenance from certified capability and adapter identity.
    pub fn new(
        capability_kind: CapabilityKind,
        capability_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    ) -> Self {
        Self {
            capability_kind,
            capability_version,
            adapter_kind,
            adapter_version,
        }
    }

    /// Returns the producing capability kind.
    pub const fn capability_kind(&self) -> &CapabilityKind {
        &self.capability_kind
    }

    /// Returns the producing capability version.
    pub const fn capability_version(&self) -> &CapabilityVersion {
        &self.capability_version
    }

    /// Returns the producing adapter kind.
    pub const fn adapter_kind(&self) -> &AdapterKind {
        &self.adapter_kind
    }

    /// Returns the producing adapter version.
    pub const fn adapter_version(&self) -> &AdapterVersion {
        &self.adapter_version
    }
}

/// Constructor parts for a normalized recorded fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactClaimParts {
    /// Visibility selected by the producer.
    pub visibility: FactVisibility,
    /// Fact kind from the validated descriptor.
    pub fact_kind: FactKind,
    /// Content digest of the canonical fact descriptor bytes.
    pub fact_descriptor_hash: ContentDigest,
    /// Descriptor-derived subject evidence.
    pub subject: FactSubjectEvidence,
    /// Optional source observation timestamp.
    pub observed_at: Option<String>,
    /// Optional request evidence.
    pub request: Option<FactRequestEvidence>,
    /// Response artifact evidence.
    pub response: FactResponseEvidence,
    /// Producer provenance.
    pub producer: FactProducerProvenance,
}

/// Normalized fact claim carried by `FactRecorded` events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactClaim {
    parts: FactClaimParts,
}

impl FactClaim {
    /// Creates a normalized fact claim from validated parts.
    pub fn new(parts: FactClaimParts) -> Result<Self> {
        if parts
            .observed_at
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(FactDescriptorError::descriptor(
                "fact claim observed_at must be non-empty when present",
            ));
        }
        Ok(Self { parts })
    }

    /// Returns the claim visibility.
    pub const fn visibility(&self) -> &FactVisibility {
        &self.parts.visibility
    }

    /// Returns the fact kind.
    pub const fn fact_kind(&self) -> &FactKind {
        &self.parts.fact_kind
    }

    /// Returns the fact descriptor hash.
    pub const fn fact_descriptor_hash(&self) -> &ContentDigest {
        &self.parts.fact_descriptor_hash
    }

    /// Returns subject evidence.
    pub const fn subject(&self) -> &FactSubjectEvidence {
        &self.parts.subject
    }

    /// Returns the optional source observation timestamp.
    pub fn observed_at(&self) -> Option<&str> {
        self.parts.observed_at.as_deref()
    }

    /// Returns optional request evidence.
    pub const fn request(&self) -> Option<&FactRequestEvidence> {
        self.parts.request.as_ref()
    }

    /// Returns response artifact evidence.
    pub const fn response(&self) -> &FactResponseEvidence {
        &self.parts.response
    }

    /// Returns producer provenance.
    pub const fn producer(&self) -> &FactProducerProvenance {
        &self.parts.producer
    }
}

/// Constructor parts for an [`InternalFactRef`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalFactRefParts {
    /// Source fact claim id.
    pub fact_claim_id: FactClaimId,
    /// Source event id for the recorded fact event.
    pub source_event_id: EventId,
    /// Store-assigned recorded time.
    pub recorded_at: String,
    /// Optional source observation time.
    pub observed_at: Option<String>,
    /// Fact visibility; must be indexed for an internal ref.
    pub visibility: FactVisibility,
    /// Fact kind.
    pub fact_kind: FactKind,
    /// Fact descriptor hash.
    pub fact_descriptor_hash: ContentDigest,
    /// Subject namespace hash.
    pub fact_subject_namespace_hash: ContentDigest,
    /// Subject fact key.
    pub fact_key: FactKey,
    /// Subject material hash.
    pub subject_material_hash: ContentDigest,
    /// Optional request schema id.
    pub request_schema_id: Option<SchemaId>,
    /// Optional request hash.
    pub request_hash: Option<ContentDigest>,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Response content hash.
    pub response_hash: ContentDigest,
    /// Response artifact id.
    pub artifact_id: ArtifactId,
    /// Response artifact evidence hash.
    pub artifact_evidence_hash: ContentDigest,
    /// Producing capability kind.
    pub capability_kind: CapabilityKind,
    /// Producing capability version.
    pub capability_version: CapabilityVersion,
    /// Producing adapter kind.
    pub adapter_kind: AdapterKind,
    /// Producing adapter version.
    pub adapter_version: AdapterVersion,
}

/// Internal trusted reference to an indexed fact projection row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalFactRef {
    parts: InternalFactRefParts,
}

impl InternalFactRef {
    /// Creates an internal fact ref from validated parts.
    pub fn new(parts: InternalFactRefParts) -> Result<Self> {
        if matches!(parts.visibility, FactVisibility::RunPrivate) {
            return Err(FactDescriptorError::descriptor(
                "internal fact refs require indexed visibility",
            ));
        }
        if parts.recorded_at.is_empty() {
            return Err(FactDescriptorError::descriptor(
                "internal fact refs require recorded_at",
            ));
        }
        if parts
            .observed_at
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(FactDescriptorError::descriptor(
                "internal fact refs require non-empty observed_at when present",
            ));
        }
        if parts.request_schema_id.is_some() != parts.request_hash.is_some() {
            return Err(FactDescriptorError::descriptor(
                "request schema and hash must be present or absent together",
            ));
        }
        Ok(Self { parts })
    }

    /// Returns the source fact claim id.
    pub const fn fact_claim_id(&self) -> &FactClaimId {
        &self.parts.fact_claim_id
    }

    /// Returns the source event id.
    pub const fn source_event_id(&self) -> &EventId {
        &self.parts.source_event_id
    }

    /// Returns the recorded time.
    pub fn recorded_at(&self) -> &str {
        &self.parts.recorded_at
    }

    /// Returns the optional observed time.
    pub fn observed_at(&self) -> Option<&str> {
        self.parts.observed_at.as_deref()
    }

    /// Returns fact visibility.
    pub const fn visibility(&self) -> &FactVisibility {
        &self.parts.visibility
    }

    /// Returns the fact kind.
    pub const fn fact_kind(&self) -> &FactKind {
        &self.parts.fact_kind
    }

    /// Returns the fact descriptor hash.
    pub const fn fact_descriptor_hash(&self) -> &ContentDigest {
        &self.parts.fact_descriptor_hash
    }

    /// Returns the fact key.
    pub const fn fact_key(&self) -> &FactKey {
        &self.parts.fact_key
    }
}

/// Descriptor catalog watermark bound into query receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DescriptorCatalogWatermark(u64);

impl DescriptorCatalogWatermark {
    /// Creates a descriptor catalog watermark.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the watermark value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Fact projection generation or rebuild id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactProjectionGeneration(u64);

impl FactProjectionGeneration {
    /// Creates a fact projection generation.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the generation value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Store commit watermark bound into query receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreCommitWatermark(u64);

impl StoreCommitWatermark {
    /// Creates a store commit watermark.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the watermark value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Store read frontier type for fact query receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreReadFrontierType {
    /// Receipt was evaluated over a complete authorized snapshot.
    Snapshot,
    /// Receipt was evaluated over an authorized prefix frontier.
    Prefix,
}

impl StoreReadFrontierType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Prefix => "prefix",
        }
    }
}

/// Semantic read frontier bound into a fact query receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreReadFrontier {
    store_scope: StoreScopeRef,
    query_scope: FactQueryScope,
    descriptor_catalog_watermark: DescriptorCatalogWatermark,
    projection_generation: FactProjectionGeneration,
    max_included_store_commit_order: u64,
    commit_watermark: StoreCommitWatermark,
}

impl StoreReadFrontier {
    /// Creates a store read frontier.
    pub fn new(
        store_scope: StoreScopeRef,
        query_scope: FactQueryScope,
        descriptor_catalog_watermark: DescriptorCatalogWatermark,
        projection_generation: FactProjectionGeneration,
        max_included_store_commit_order: u64,
        commit_watermark: StoreCommitWatermark,
    ) -> Self {
        Self {
            store_scope,
            query_scope,
            descriptor_catalog_watermark,
            projection_generation,
            max_included_store_commit_order,
            commit_watermark,
        }
    }

    /// Returns the store scope.
    pub const fn store_scope(&self) -> &StoreScopeRef {
        &self.store_scope
    }

    /// Returns the query scope.
    pub const fn query_scope(&self) -> &FactQueryScope {
        &self.query_scope
    }

    /// Returns the descriptor catalog watermark.
    pub const fn descriptor_catalog_watermark(&self) -> DescriptorCatalogWatermark {
        self.descriptor_catalog_watermark
    }

    /// Returns the projection generation.
    pub const fn projection_generation(&self) -> FactProjectionGeneration {
        self.projection_generation
    }

    /// Returns the maximum included store commit order.
    pub const fn max_included_store_commit_order(&self) -> u64 {
        self.max_included_store_commit_order
    }

    /// Returns the commit watermark.
    pub const fn commit_watermark(&self) -> StoreCommitWatermark {
        self.commit_watermark
    }
}

/// Store receipt authentication scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreReceiptAuthenticationScheme {
    /// Local Ed25519 signature over a SHA-256 JCS receipt hash.
    LocalEd25519Sha256JcsV1,
}

impl StoreReceiptAuthenticationScheme {
    /// Returns the canonical authentication scheme tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalEd25519Sha256JcsV1 => "local_ed25519_sha256_jcs_v1",
        }
    }
}

/// Store-owned receipt authentication metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreReceiptAuthentication {
    store_identity: StoreIdentity,
    scheme: StoreReceiptAuthenticationScheme,
    key_id: Option<StoreKeyId>,
    signature_or_mac: Vec<u8>,
}

impl StoreReceiptAuthentication {
    /// Creates receipt authentication metadata.
    pub fn new(
        store_identity: StoreIdentity,
        scheme: StoreReceiptAuthenticationScheme,
        key_id: Option<StoreKeyId>,
        signature_or_mac: Vec<u8>,
    ) -> Result<Self> {
        match scheme {
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1 => {
                if key_id.is_none() {
                    return Err(FactDescriptorError::descriptor(
                        "local Ed25519 receipt authentication requires a key id",
                    ));
                }
                if signature_or_mac.len() != 64 {
                    return Err(FactDescriptorError::descriptor(
                        "local Ed25519 receipt authentication requires a 64-byte signature",
                    ));
                }
            }
        }
        Ok(Self {
            store_identity,
            scheme,
            key_id,
            signature_or_mac,
        })
    }

    /// Returns the store identity.
    pub const fn store_identity(&self) -> &StoreIdentity {
        &self.store_identity
    }

    /// Returns the authentication scheme.
    pub const fn scheme(&self) -> StoreReceiptAuthenticationScheme {
        self.scheme
    }

    /// Returns the optional key id.
    pub const fn key_id(&self) -> Option<&StoreKeyId> {
        self.key_id.as_ref()
    }

    /// Returns signature or MAC bytes.
    pub fn signature_or_mac(&self) -> &[u8] {
        &self.signature_or_mac
    }
}

/// One returned field summary value used by query replay and public shaping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedFieldValueSummary {
    field_id: FactFieldId,
    value_type: FactFieldValueType,
    value: FactCanonicalScalar,
}

impl ReturnedFieldValueSummary {
    /// Creates a returned field value summary.
    pub fn new(
        field_id: FactFieldId,
        value_type: FactFieldValueType,
        value: FactCanonicalScalar,
    ) -> Result<Self> {
        if value.value_type() != value_type {
            return Err(FactDescriptorError::field(
                field_id,
                "returned summary value type mismatch",
            ));
        }
        validate_scalar_size(&field_id, &value)?;
        Ok(Self {
            field_id,
            value_type,
            value,
        })
    }

    /// Returns the summarized field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns the summarized value type.
    pub const fn value_type(&self) -> FactFieldValueType {
        self.value_type
    }

    /// Returns the summarized value.
    pub const fn value(&self) -> &FactCanonicalScalar {
        &self.value
    }
}

/// Returned summaries for one fact ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedFactFieldSummary {
    fact_claim_id: FactClaimId,
    fields: Vec<ReturnedFieldValueSummary>,
}

impl ReturnedFactFieldSummary {
    /// Creates returned field summaries for one fact.
    pub fn new(fact_claim_id: FactClaimId, fields: Vec<ReturnedFieldValueSummary>) -> Self {
        Self {
            fact_claim_id,
            fields,
        }
    }

    /// Returns the fact claim id.
    pub const fn fact_claim_id(&self) -> &FactClaimId {
        &self.fact_claim_id
    }

    /// Returns summarized fields in retained order.
    pub fn fields(&self) -> &[ReturnedFieldValueSummary] {
        &self.fields
    }
}

/// Returned field summaries pinned in a query receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedFieldSummaries {
    summaries: Vec<ReturnedFactFieldSummary>,
}

impl ReturnedFieldSummaries {
    /// Creates returned field summaries.
    pub fn new(summaries: Vec<ReturnedFactFieldSummary>) -> Self {
        Self { summaries }
    }

    /// Returns fact summaries in receipt order.
    pub fn summaries(&self) -> &[ReturnedFactFieldSummary] {
        &self.summaries
    }
}

/// Cardinality statement for a query result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QueryResultCardinality {
    /// The receipt represents the exact number of matching rows.
    Exact(u64),
    /// The receipt hit a limit and represents at least this many matching rows.
    AtLeast(u64),
    /// The receipt did not assert total matching row count.
    NotCounted,
}

/// Store-owned receipt for a canonical fact query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryReceipt {
    read_frontier: StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: Vec<InternalFactRef>,
    returned_field_summaries: Option<ReturnedFieldSummaries>,
    result_set_digest: ContentDigest,
    result_cardinality: QueryResultCardinality,
    store_receipt_hash: ContentDigest,
    store_receipt_authentication: StoreReceiptAuthentication,
}

impl FactQueryReceipt {
    /// Creates a fact query receipt.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        read_frontier: StoreReadFrontier,
        frontier_type: StoreReadFrontierType,
        returned_refs: Vec<InternalFactRef>,
        returned_field_summaries: Option<ReturnedFieldSummaries>,
        result_set_digest: ContentDigest,
        result_cardinality: QueryResultCardinality,
        store_receipt_hash: ContentDigest,
        store_receipt_authentication: StoreReceiptAuthentication,
    ) -> Self {
        Self {
            read_frontier,
            frontier_type,
            returned_refs,
            returned_field_summaries,
            result_set_digest,
            result_cardinality,
            store_receipt_hash,
            store_receipt_authentication,
        }
    }

    /// Returns the read frontier.
    pub const fn read_frontier(&self) -> &StoreReadFrontier {
        &self.read_frontier
    }

    /// Returns the frontier type.
    pub const fn frontier_type(&self) -> StoreReadFrontierType {
        self.frontier_type
    }

    /// Returns refs in receipt order.
    pub fn returned_refs(&self) -> &[InternalFactRef] {
        &self.returned_refs
    }

    /// Returns optional field summaries.
    pub const fn returned_field_summaries(&self) -> Option<&ReturnedFieldSummaries> {
        self.returned_field_summaries.as_ref()
    }

    /// Returns the result-set digest.
    pub const fn result_set_digest(&self) -> &ContentDigest {
        &self.result_set_digest
    }

    /// Returns result cardinality.
    pub const fn result_cardinality(&self) -> QueryResultCardinality {
        self.result_cardinality
    }

    /// Returns the store receipt hash.
    pub const fn store_receipt_hash(&self) -> &ContentDigest {
        &self.store_receipt_hash
    }

    /// Returns store receipt authentication.
    pub const fn store_receipt_authentication(&self) -> &StoreReceiptAuthentication {
        &self.store_receipt_authentication
    }
}

/// State-owned evidence describing selected receipt rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionEvidence {
    selection_policy_hash: ContentDigest,
    selected_indices: Vec<u64>,
    selected_summaries_digest: Option<ContentDigest>,
}

impl FactSelectionEvidence {
    /// Creates selection evidence and validates canonical selected indices.
    pub fn new(
        selection_policy_hash: ContentDigest,
        selected_indices: Vec<u64>,
        selected_summaries_digest: Option<ContentDigest>,
    ) -> Result<Self> {
        for window in selected_indices.windows(2) {
            if window[0] >= window[1] {
                return Err(FactDescriptorError::descriptor(
                    "selected indices must be sorted and unique",
                ));
            }
        }
        Ok(Self {
            selection_policy_hash,
            selected_indices,
            selected_summaries_digest,
        })
    }

    /// Returns the selection policy hash.
    pub const fn selection_policy_hash(&self) -> &ContentDigest {
        &self.selection_policy_hash
    }

    /// Returns selected receipt indices.
    pub fn selected_indices(&self) -> &[u64] {
        &self.selected_indices
    }

    /// Returns the optional selected summaries digest.
    pub const fn selected_summaries_digest(&self) -> Option<&ContentDigest> {
        self.selected_summaries_digest.as_ref()
    }
}

/// Replay evidence for a live fact query performed by a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryEvidence {
    plan: CanonicalFactQueryPlan,
    receipt: FactQueryReceipt,
    selection: FactSelectionEvidence,
}

impl FactQueryEvidence {
    /// Creates fact query replay evidence.
    pub const fn new(
        plan: CanonicalFactQueryPlan,
        receipt: FactQueryReceipt,
        selection: FactSelectionEvidence,
    ) -> Self {
        Self {
            plan,
            receipt,
            selection,
        }
    }

    /// Returns the canonical query plan.
    pub const fn plan(&self) -> &CanonicalFactQueryPlan {
        &self.plan
    }

    /// Returns the store query receipt.
    pub const fn receipt(&self) -> &FactQueryReceipt {
        &self.receipt
    }

    /// Returns state-owned selection evidence.
    pub const fn selection(&self) -> &FactSelectionEvidence {
        &self.selection
    }
}

/// Validates a fact descriptor.
pub fn validate_descriptor(descriptor: &FactDescriptor) -> Result<()> {
    let mut fields_by_id = BTreeMap::new();
    let mut subject_fields = 0usize;

    for field in &descriptor.fields {
        field.validate()?;
        if fields_by_id.insert(field.field_id.clone(), field).is_some() {
            return Err(FactDescriptorError::descriptor(format!(
                "duplicate field id {}",
                field.field_id
            )));
        }

        if matches!(field.accessor, FactFieldAccessor::SubjectPath(_)) {
            subject_fields += 1;
            if !field.required {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "subject fields must be required in v1",
                ));
            }
        }
    }

    if subject_fields == 0 {
        return Err(FactDescriptorError::descriptor(
            "descriptor must contain at least one subject field",
        ));
    }

    let mut ordering_names = BTreeSet::new();
    for ordering in &descriptor.orderings {
        if !ordering_names.insert(ordering.name.clone()) {
            return Err(FactDescriptorError::descriptor(format!(
                "duplicate ordering name {}",
                ordering.name
            )));
        }
        validate_ordering(ordering, &fields_by_id)?;
    }

    Ok(())
}

/// Returns the schema id for fact descriptor artifacts.
pub fn fact_descriptor_schema_id() -> Result<SchemaId> {
    SchemaId::new(
        "mfm.fact_descriptor",
        FACTS_KERNEL_CONTRACT_VERSION,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(
            format!("schema:mfm.fact_descriptor:{FACTS_KERNEL_CONTRACT_VERSION}").as_bytes(),
        ),
    )
    .map_err(|error| FactDescriptorError::descriptor(error.to_string()))
}

/// Returns canonical descriptor bytes.
pub fn canonical_fact_descriptor_bytes(descriptor: &FactDescriptor) -> Result<CanonicalJsonBytes> {
    validate_descriptor(descriptor)?;
    canonical_descriptor_value(descriptor).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Decodes and validates canonical fact descriptor artifact bytes.
pub fn parse_canonical_fact_descriptor_bytes(bytes: &[u8]) -> Result<FactDescriptor> {
    let descriptor = serde_json::from_slice::<FactDescriptor>(bytes)
        .map_err(|error| FactDescriptorError::descriptor(error.to_string()))?;
    let canonical = canonical_fact_descriptor_bytes(&descriptor)?;
    if canonical.as_bytes() != bytes {
        return Err(FactDescriptorError::descriptor(
            "fact descriptor bytes are not canonical",
        ));
    }
    Ok(descriptor)
}

/// Derives the content digest for a fact descriptor.
pub fn fact_descriptor_hash(descriptor: &FactDescriptor) -> Result<ContentDigest> {
    Ok(canonical_fact_descriptor_bytes(descriptor)?.content_digest())
}

/// Builds the subject namespace for a validated fact descriptor.
pub fn fact_subject_namespace(descriptor: &FactDescriptor) -> Result<FactSubjectNamespaceV1> {
    validate_descriptor(descriptor)?;
    let fields = descriptor
        .fields
        .iter()
        .filter(|field| matches!(field.accessor, FactFieldAccessor::SubjectPath(_)))
        .map(|field| {
            FactSubjectNamespaceFieldV1::new(
                field.field_id.clone(),
                field.value_type,
                field.unit.clone(),
                field.scale,
            )
        })
        .collect();
    FactSubjectNamespaceV1::new(descriptor.fact_kind.clone(), fields)
}

/// Returns canonical subject namespace bytes.
pub fn canonical_fact_subject_namespace_bytes(
    namespace: &FactSubjectNamespaceV1,
) -> Result<CanonicalJsonBytes> {
    canonical_subject_namespace_value(namespace).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for a subject namespace.
pub fn fact_subject_namespace_hash(namespace: &FactSubjectNamespaceV1) -> Result<ContentDigest> {
    Ok(canonical_fact_subject_namespace_bytes(namespace)?.content_digest())
}

/// Returns canonical subject material bytes.
pub fn canonical_fact_subject_material_bytes(
    material: &FactSubjectMaterialV1,
) -> Result<CanonicalJsonBytes> {
    canonical_subject_material_value(material).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Decodes and validates canonical subject material bytes.
pub fn parse_canonical_fact_subject_material_bytes(bytes: &[u8]) -> Result<FactSubjectMaterialV1> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| FactDescriptorError::descriptor("subject material must be an object"))?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactDescriptorError::descriptor("subject material version is required"))?;
    if version != FactSubjectMaterialV1::VERSION {
        return Err(FactDescriptorError::descriptor(format!(
            "unsupported subject material version {version:?}"
        )));
    }
    let values = object
        .get("values")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| FactDescriptorError::descriptor("subject material values are required"))?
        .iter()
        .map(parse_subject_material_value)
        .collect::<Result<Vec<_>>>()?;
    let material = FactSubjectMaterialV1::new(values)?;
    let canonical = canonical_fact_subject_material_bytes(&material)?;
    if canonical.as_bytes() != bytes {
        return Err(FactDescriptorError::descriptor(
            "subject material bytes are not canonical",
        ));
    }
    Ok(material)
}

/// Derives the content digest for subject material.
pub fn subject_material_hash(material: &FactSubjectMaterialV1) -> Result<ContentDigest> {
    Ok(canonical_fact_subject_material_bytes(material)?.content_digest())
}

/// Derives a fact key from a subject namespace hash and subject material hash.
pub fn derive_fact_key(
    fact_subject_namespace_hash: ContentDigest,
    subject_material_hash: ContentDigest,
) -> Result<FactKey> {
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-key.v1".to_owned()),
        ),
        (
            "fact_subject_namespace_hash",
            CanonicalValue::String(fact_subject_namespace_hash.as_str().to_owned()),
        ),
        (
            "subject_material_hash",
            CanonicalValue::String(subject_material_hash.as_str().to_owned()),
        ),
    ])?;
    let bytes = CanonicalJsonBytes::from_value(&value);
    Ok(FactKey::from_digest(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(bytes.as_bytes()),
    )))
}

/// Derives a fact claim id from run-stream coordinates.
pub fn derive_fact_claim_id(
    source_run_id: RunId,
    source_seq: u64,
    source_ordinal: u32,
) -> Result<FactClaimId> {
    FactClaimId::new(source_run_id, source_seq, source_ordinal)
}

/// Returns canonical fact claim id bytes.
pub fn canonical_fact_claim_id_bytes(claim_id: &FactClaimId) -> Result<CanonicalJsonBytes> {
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-claim-id.v1".to_owned()),
        ),
        (
            "source_run_id",
            CanonicalValue::String(claim_id.source_run_id.as_str().to_owned()),
        ),
        ("source_seq", CanonicalValue::Unsigned(claim_id.source_seq)),
        (
            "source_ordinal",
            CanonicalValue::Unsigned(u64::from(claim_id.source_ordinal)),
        ),
    ])?;
    Ok(CanonicalJsonBytes::from_value(&value))
}

/// Returns canonical query plan bytes.
pub fn canonical_fact_query_plan_bytes(
    plan: &CanonicalFactQueryPlan,
) -> Result<CanonicalJsonBytes> {
    canonical_query_plan_value(plan).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for a canonical query plan.
pub fn fact_query_plan_hash(plan: &CanonicalFactQueryPlan) -> Result<ContentDigest> {
    Ok(canonical_fact_query_plan_bytes(plan)?.content_digest())
}

/// Compiles a descriptor-scoped query input into a canonical fact query plan.
pub fn compile_fact_query_plan(
    descriptor: &FactDescriptor,
    input: FactQueryInput,
) -> Result<CanonicalFactQueryPlan> {
    validate_descriptor(descriptor)?;
    let descriptor_hash = fact_descriptor_hash(descriptor)?;
    let fields_by_id = descriptor_fields_by_id(descriptor)?;
    let ordering_descriptor = descriptor
        .orderings()
        .iter()
        .find(|ordering| ordering.name() == input.ordering())
        .ok_or_else(|| {
            FactDescriptorError::ordering(
                input.ordering().clone(),
                "fact query ordering is not declared by descriptor",
            )
        })?;

    let mut predicates = input.predicates().to_vec();
    predicates.sort();
    for predicate in &predicates {
        let field = required_query_field(&fields_by_id, predicate.field_id())?;
        require_query_exposed(field, "predicate")?;
        if !field.operators().contains(&predicate.operator()) {
            return Err(FactDescriptorError::field(
                predicate.field_id().clone(),
                format!(
                    "operator {:?} is not declared for field {}",
                    predicate.operator(),
                    predicate.field_id()
                ),
            ));
        }
        validate_extracted_scalar(field, predicate.value())?;
    }

    for return_field in input.return_fields() {
        let field = required_query_field(&fields_by_id, return_field.field_id())?;
        if field.exposure() != FactFieldExposure::Returnable {
            return Err(FactDescriptorError::field(
                return_field.field_id().clone(),
                "field is not returnable by descriptor exposure policy",
            ));
        }
    }

    for term in ordering_descriptor.terms() {
        let field = required_query_field(&fields_by_id, term.field_id())?;
        require_query_exposed(field, "ordering")?;
    }

    let ordering = FactOrdering::from_descriptor(ordering_descriptor);
    let canonical_query = canonical_compiled_query_bytes(
        descriptor,
        &descriptor_hash,
        &predicates,
        input.return_fields(),
        ordering.name(),
        input.limit(),
    )?;
    CanonicalFactQueryPlan::new(
        input.store_scope,
        input.query_scope,
        FactQueryCompilerVersion::new(FACT_QUERY_COMPILER_VERSION)?,
        FactCanonicalizerVersion::new(FACT_QUERY_CANONICALIZER_VERSION)?,
        descriptor_hash,
        input.scope_decision_evidence,
        canonical_query,
        ordering,
        input.limit,
    )
}

/// Parses and validates the canonical query shape embedded in a query plan.
pub fn parse_canonical_fact_query_shape(
    plan: &CanonicalFactQueryPlan,
) -> Result<CompiledFactQueryShape> {
    if plan.canonical_query().content_digest() != *plan.canonical_query_hash() {
        return Err(FactDescriptorError::descriptor(
            "fact query plan canonical query hash mismatch",
        ));
    }
    let value = serde_json::from_slice::<serde_json::Value>(plan.canonical_query().as_bytes())
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| FactDescriptorError::descriptor("canonical fact query must be an object"))?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query version is required")
        })?;
    if version != "mfm.fact-query.v1" {
        return Err(FactDescriptorError::descriptor(format!(
            "unsupported canonical fact query version {version:?}"
        )));
    }
    let resolved_descriptor = object
        .get("resolved_descriptor")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query descriptor hash is required")
        })?;
    if resolved_descriptor != plan.resolved_descriptor().as_str() {
        return Err(FactDescriptorError::descriptor(
            "canonical fact query descriptor hash does not match plan",
        ));
    }
    let ordering = object
        .get("ordering")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query ordering is required")
        })?;
    if ordering != plan.ordering().name().as_str() {
        return Err(FactDescriptorError::descriptor(
            "canonical fact query ordering does not match plan",
        ));
    }
    let query_limit = object
        .get("limit")
        .ok_or_else(|| FactDescriptorError::descriptor("canonical fact query limit is required"))
        .and_then(parse_optional_u64_query_value)?;
    if query_limit != plan.limit() {
        return Err(FactDescriptorError::descriptor(
            "canonical fact query limit does not match plan",
        ));
    }

    let predicates = object
        .get("predicates")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query predicates are required")
        })?
        .iter()
        .map(parse_canonical_query_predicate)
        .collect::<Result<Vec<_>>>()?;
    let return_fields = object
        .get("return_fields")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query return fields are required")
        })?
        .iter()
        .map(parse_canonical_query_return_field)
        .collect::<Result<Vec<_>>>()?;
    CompiledFactQueryShape::new(predicates, return_fields)
}

/// Returns canonical receipt body bytes, excluding receipt hash and authentication fields.
pub fn canonical_fact_query_receipt_body_bytes(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<CanonicalJsonBytes> {
    canonical_query_receipt_body_parts_value(
        plan_hash,
        &receipt.read_frontier,
        receipt.frontier_type,
        &receipt.returned_refs,
        receipt.returned_field_summaries.as_ref(),
        &receipt.result_set_digest,
        receipt.result_cardinality,
    )
    .map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Returns canonical result-set bytes for the rows and summaries pinned in a receipt.
pub fn canonical_fact_query_result_set_bytes(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalJsonBytes> {
    canonical_fact_query_result_set_value(returned_refs, returned_field_summaries)
        .map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the result-set digest for the rows and summaries pinned in a receipt.
pub fn fact_query_result_set_digest(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<ContentDigest> {
    Ok(
        canonical_fact_query_result_set_bytes(returned_refs, returned_field_summaries)?
            .content_digest(),
    )
}

/// Returns canonical receipt body bytes from unsigned receipt parts.
#[allow(clippy::too_many_arguments)]
pub fn canonical_fact_query_receipt_body_bytes_from_parts(
    plan_hash: &ContentDigest,
    read_frontier: &StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
    result_set_digest: &ContentDigest,
    result_cardinality: QueryResultCardinality,
) -> Result<CanonicalJsonBytes> {
    canonical_query_receipt_body_parts_value(
        plan_hash,
        read_frontier,
        frontier_type,
        returned_refs,
        returned_field_summaries,
        result_set_digest,
        result_cardinality,
    )
    .map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the receipt body hash, excluding receipt hash and authentication fields.
pub fn fact_query_receipt_body_hash(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<ContentDigest> {
    Ok(canonical_fact_query_receipt_body_bytes(plan_hash, receipt)?.content_digest())
}

/// Derives the receipt body hash from unsigned receipt parts.
#[allow(clippy::too_many_arguments)]
pub fn fact_query_receipt_body_hash_from_parts(
    plan_hash: &ContentDigest,
    read_frontier: &StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
    result_set_digest: &ContentDigest,
    result_cardinality: QueryResultCardinality,
) -> Result<ContentDigest> {
    Ok(canonical_fact_query_receipt_body_bytes_from_parts(
        plan_hash,
        read_frontier,
        frontier_type,
        returned_refs,
        returned_field_summaries,
        result_set_digest,
        result_cardinality,
    )?
    .content_digest())
}

/// Returns canonical query evidence bytes.
pub fn canonical_fact_query_evidence_bytes(
    evidence: &FactQueryEvidence,
) -> Result<CanonicalJsonBytes> {
    canonical_query_evidence_value(evidence).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for fact query evidence.
pub fn fact_query_evidence_hash(evidence: &FactQueryEvidence) -> Result<ContentDigest> {
    Ok(canonical_fact_query_evidence_bytes(evidence)?.content_digest())
}

/// Extracts canonical subject material using the descriptor's subject fields.
pub fn extract_subject_material(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
) -> Result<FactSubjectMaterialV1> {
    validate_descriptor(descriptor)?;
    let mut values = Vec::new();
    for field in &descriptor.fields {
        if !matches!(field.accessor, FactFieldAccessor::SubjectPath(_)) {
            continue;
        }
        let scalar = extract_field_scalar(field, subject, &CanonicalValue::Null, None)?
            .ok_or_else(|| {
                FactDescriptorError::field(field.field_id.clone(), "required subject field missing")
            })?;
        validate_extracted_scalar(field, &scalar)?;
        values.push(FactSubjectValueV1::new(
            field.field_id.clone(),
            field.value_type,
            scalar,
        )?);
    }
    FactSubjectMaterialV1::new(values)
}

/// Extracts subject, result, and metadata terms for an indexed fact claim.
pub fn extract_terms(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
    response: &CanonicalValue,
    metadata: &FactExtractionMetadata,
) -> Result<Vec<FactIndexTerm>> {
    validate_descriptor(descriptor)?;
    let mut terms = Vec::new();
    for field in &descriptor.fields {
        match extract_field_scalar(field, subject, response, Some(metadata))? {
            Some(scalar) => terms.push(FactIndexTerm::from_field(field, scalar)?),
            None if field.required => {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "required field missing",
                ));
            }
            None => {}
        }
    }
    Ok(terms)
}

/// Decodes canonical fact response bytes using descriptor-declared response scalar types.
pub fn parse_canonical_fact_response_bytes(
    descriptor: &FactDescriptor,
    bytes: &[u8],
) -> Result<CanonicalValue> {
    validate_descriptor(descriptor)?;
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let typed_paths = response_typed_paths(descriptor)?;
    json_to_fact_canonical_value(&value, "", &typed_paths)
}

/// Extracts index terms from persisted subject material plus canonical response material.
pub fn extract_terms_from_material(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV1,
    response: &CanonicalValue,
    metadata: &FactExtractionMetadata,
) -> Result<Vec<FactIndexTerm>> {
    validate_descriptor(descriptor)?;
    let subject_values = subject_material
        .values()
        .iter()
        .map(|value| (value.field_id().clone(), value))
        .collect::<BTreeMap<_, _>>();
    let mut terms = Vec::new();
    for field in &descriptor.fields {
        let scalar = match field.accessor() {
            FactFieldAccessor::SubjectPath(_) => {
                let Some(value) = subject_values.get(field.field_id()) else {
                    if field.required() {
                        return Err(FactDescriptorError::field(
                            field.field_id.clone(),
                            "required subject field missing",
                        ));
                    }
                    continue;
                };
                if value.value_type() != field.value_type() {
                    return Err(FactDescriptorError::field(
                        field.field_id.clone(),
                        "subject material value type does not match descriptor field",
                    ));
                }
                Some(value.value().clone())
            }
            FactFieldAccessor::ResponsePath(_) | FactFieldAccessor::Metadata(_) => {
                extract_field_scalar(field, &CanonicalValue::Null, response, Some(metadata))?
            }
        };
        match scalar {
            Some(scalar) => terms.push(FactIndexTerm::from_field(field, scalar)?),
            None if field.required() => {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "required field missing",
                ));
            }
            None => {}
        }
    }
    Ok(terms)
}

fn canonical_descriptor_value(descriptor: &FactDescriptor) -> Result<CanonicalValue> {
    let mut fields = descriptor.fields.iter().collect::<Vec<_>>();
    fields.sort_by(|left, right| left.field_id.cmp(&right.field_id));
    let fields = fields
        .into_iter()
        .map(canonical_field_descriptor_value)
        .collect::<Result<Vec<_>>>()?;

    let mut orderings = descriptor.orderings.iter().collect::<Vec<_>>();
    orderings.sort_by(|left, right| left.name.cmp(&right.name));
    let orderings = orderings
        .into_iter()
        .map(canonical_ordering_descriptor_value)
        .collect::<Result<Vec<_>>>()?;

    canonical_object([
        (
            "version",
            CanonicalValue::String(FACTS_KERNEL_CONTRACT_VERSION.to_owned()),
        ),
        (
            "fact_kind",
            CanonicalValue::String(descriptor.fact_kind.as_str().to_owned()),
        ),
        (
            "descriptor_schema_id",
            CanonicalValue::String(descriptor.descriptor_schema_id.as_str().to_owned()),
        ),
        (
            "subject_schema_id",
            CanonicalValue::String(descriptor.subject_schema_id.as_str().to_owned()),
        ),
        (
            "response_schema_id",
            CanonicalValue::String(descriptor.response_schema_id.as_str().to_owned()),
        ),
        (
            "compatibility_group",
            optional_checked_string_value(descriptor.compatibility_group.as_ref()),
        ),
        ("fields", CanonicalValue::Array(fields)),
        ("orderings", CanonicalValue::Array(orderings)),
    ])
}

fn canonical_field_descriptor_value(field: &FactFieldDescriptor) -> Result<CanonicalValue> {
    let mut operators = field.operators.clone();
    operators.sort();
    let operators = operators
        .into_iter()
        .map(|operator| CanonicalValue::String(operator.as_str().to_owned()))
        .collect::<Vec<_>>();

    canonical_object([
        (
            "field_id",
            CanonicalValue::String(field.field_id.as_str().to_owned()),
        ),
        (
            "path",
            CanonicalValue::String(field.path.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(field.value_type.as_str().to_owned()),
        ),
        ("accessor", canonical_accessor_value(&field.accessor)?),
        ("operators", CanonicalValue::Array(operators)),
        (
            "exposure",
            CanonicalValue::String(field.exposure.as_str().to_owned()),
        ),
        ("unit", optional_checked_string_value(field.unit.as_ref())),
        ("scale", optional_scale_value(field.scale)),
        ("sortable", CanonicalValue::Bool(field.sortable)),
        ("required", CanonicalValue::Bool(field.required)),
    ])
}

fn canonical_accessor_value(accessor: &FactFieldAccessor) -> Result<CanonicalValue> {
    match accessor {
        FactFieldAccessor::SubjectPath(path) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Subject.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(path.as_str().to_owned())),
        ]),
        FactFieldAccessor::ResponsePath(path) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Result.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(path.as_str().to_owned())),
        ]),
        FactFieldAccessor::Metadata(field) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Metadata.as_str().to_owned()),
            ),
            ("field", CanonicalValue::String(field.as_str().to_owned())),
        ]),
    }
}

fn canonical_ordering_descriptor_value(
    ordering: &FactOrderingDescriptor,
) -> Result<CanonicalValue> {
    let terms = ordering
        .terms
        .iter()
        .map(canonical_ordering_term_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "name",
            CanonicalValue::String(ordering.name.as_str().to_owned()),
        ),
        ("terms", CanonicalValue::Array(terms)),
    ])
}

fn canonical_ordering_term_value(term: &FactOrderingTerm) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(term.field_id.as_str().to_owned()),
        ),
        (
            "direction",
            CanonicalValue::String(term.direction.as_str().to_owned()),
        ),
        (
            "nulls",
            CanonicalValue::String(term.nulls.as_str().to_owned()),
        ),
        ("tie_breaker", CanonicalValue::Bool(term.tie_breaker)),
    ])
}

fn canonical_subject_namespace_value(namespace: &FactSubjectNamespaceV1) -> Result<CanonicalValue> {
    let fields = namespace
        .fields
        .iter()
        .map(canonical_subject_namespace_field_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String(FactSubjectNamespaceV1::VERSION.to_owned()),
        ),
        (
            "fact_kind",
            CanonicalValue::String(namespace.fact_kind.as_str().to_owned()),
        ),
        ("fields", CanonicalValue::Array(fields)),
    ])
}

fn canonical_subject_namespace_field_value(
    field: &FactSubjectNamespaceFieldV1,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(field.field_id.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(field.value_type.as_str().to_owned()),
        ),
        ("unit", optional_checked_string_value(field.unit.as_ref())),
        ("scale", optional_scale_value(field.scale)),
    ])
}

fn canonical_subject_material_value(material: &FactSubjectMaterialV1) -> Result<CanonicalValue> {
    let values = material
        .values
        .iter()
        .map(canonical_subject_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String(FactSubjectMaterialV1::VERSION.to_owned()),
        ),
        ("values", CanonicalValue::Array(values)),
    ])
}

fn canonical_subject_value(value: &FactSubjectValueV1) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(value.field_id.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(value.value_type.as_str().to_owned()),
        ),
        ("value", value.value.canonical_value()),
    ])
}

fn parse_subject_material_value(value: &serde_json::Value) -> Result<FactSubjectValueV1> {
    let object = value.as_object().ok_or_else(|| {
        FactDescriptorError::descriptor("subject material value must be an object")
    })?;
    let field_id = object
        .get("field_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactDescriptorError::descriptor("subject material field_id is required"))
        .and_then(FactFieldId::new)?;
    let value_type = object
        .get("value_type")
        .ok_or_else(|| FactDescriptorError::descriptor("subject material value_type is required"))
        .and_then(parse_field_value_type)?;
    let scalar_value = object
        .get("value")
        .ok_or_else(|| FactDescriptorError::descriptor("subject material value is required"))?;
    let scalar = parse_canonical_scalar_value(value_type, scalar_value, &field_id)?;
    FactSubjectValueV1::new(field_id, value_type, scalar)
}

fn parse_field_value_type(value: &serde_json::Value) -> Result<FactFieldValueType> {
    serde_json::from_value(value.clone())
        .map_err(|error| FactDescriptorError::descriptor(error.to_string()))
}

fn parse_canonical_scalar_value(
    value_type: FactFieldValueType,
    value: &serde_json::Value,
    field_id: &FactFieldId,
) -> Result<FactCanonicalScalar> {
    match value_type {
        FactFieldValueType::String => value
            .as_str()
            .map(|value| FactCanonicalScalar::String(value.to_owned()))
            .ok_or_else(|| FactDescriptorError::field(field_id.clone(), "expected string value")),
        FactFieldValueType::Boolean => value
            .as_bool()
            .map(FactCanonicalScalar::Boolean)
            .ok_or_else(|| FactDescriptorError::field(field_id.clone(), "expected boolean value")),
        FactFieldValueType::SignedInteger => value
            .as_i64()
            .map(FactCanonicalScalar::SignedInteger)
            .ok_or_else(|| {
                FactDescriptorError::field(field_id.clone(), "expected signed integer value")
            }),
        FactFieldValueType::UnsignedInteger => value
            .as_u64()
            .map(FactCanonicalScalar::UnsignedInteger)
            .ok_or_else(|| {
                FactDescriptorError::field(field_id.clone(), "expected unsigned integer value")
            }),
        FactFieldValueType::Timestamp => value
            .as_str()
            .ok_or_else(|| FactDescriptorError::field(field_id.clone(), "expected timestamp value"))
            .and_then(FactCanonicalScalar::timestamp),
        FactFieldValueType::DecimalString => value
            .as_str()
            .ok_or_else(|| FactDescriptorError::field(field_id.clone(), "expected decimal value"))
            .and_then(FactCanonicalScalar::decimal_variable),
        FactFieldValueType::Digest => {
            let raw = value.as_str().ok_or_else(|| {
                FactDescriptorError::field(field_id.clone(), "expected digest value")
            })?;
            raw.parse()
                .map(FactCanonicalScalar::Digest)
                .map_err(|error| {
                    FactDescriptorError::field(field_id.clone(), format!("invalid digest: {error}"))
                })
        }
    }
}

fn parse_optional_u64_query_value(value: &serde_json::Value) -> Result<Option<u64>> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| FactDescriptorError::descriptor("canonical fact query limit must be a u64"))
}

fn parse_canonical_query_predicate(value: &serde_json::Value) -> Result<FactQueryPredicate> {
    let object = value
        .as_object()
        .ok_or_else(|| FactDescriptorError::descriptor("query predicate must be an object"))?;
    let field_id = object
        .get("field_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactDescriptorError::descriptor("query predicate field_id is required"))
        .and_then(FactFieldId::new)?;
    let operator = object
        .get("operator")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactDescriptorError::field(field_id.clone(), "query operator is required"))
        .and_then(|value| parse_query_operator(&field_id, value))?;
    let value_type = object
        .get("value_type")
        .ok_or_else(|| FactDescriptorError::field(field_id.clone(), "query value_type is required"))
        .and_then(parse_field_value_type)?;
    let scalar_value = object
        .get("value")
        .ok_or_else(|| FactDescriptorError::field(field_id.clone(), "query value is required"))?;
    let scalar = parse_canonical_scalar_value(value_type, scalar_value, &field_id)?;
    Ok(FactQueryPredicate::new(field_id, operator, scalar))
}

fn parse_query_operator(field_id: &FactFieldId, value: &str) -> Result<FactQueryOperator> {
    match value {
        "equal" => Ok(FactQueryOperator::Equal),
        "less_than" => Ok(FactQueryOperator::LessThan),
        "less_than_or_equal" => Ok(FactQueryOperator::LessThanOrEqual),
        "greater_than" => Ok(FactQueryOperator::GreaterThan),
        "greater_than_or_equal" => Ok(FactQueryOperator::GreaterThanOrEqual),
        _ => Err(FactDescriptorError::field(
            field_id.clone(),
            format!("unknown query operator {value:?}"),
        )),
    }
}

fn parse_canonical_query_return_field(value: &serde_json::Value) -> Result<FactQueryReturnField> {
    let object = value
        .as_object()
        .ok_or_else(|| FactDescriptorError::descriptor("query return field must be an object"))?;
    let field_id = object
        .get("field_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactDescriptorError::descriptor("query return field_id is required"))
        .and_then(FactFieldId::new)?;
    Ok(FactQueryReturnField::new(field_id))
}

fn descriptor_fields_by_id(
    descriptor: &FactDescriptor,
) -> Result<BTreeMap<FactFieldId, &FactFieldDescriptor>> {
    let mut fields = BTreeMap::new();
    for field in descriptor.fields() {
        if fields.insert(field.field_id().clone(), field).is_some() {
            return Err(FactDescriptorError::field(
                field.field_id().clone(),
                "duplicate descriptor field id",
            ));
        }
    }
    Ok(fields)
}

fn required_query_field<'a>(
    fields_by_id: &'a BTreeMap<FactFieldId, &FactFieldDescriptor>,
    field_id: &FactFieldId,
) -> Result<&'a FactFieldDescriptor> {
    fields_by_id.get(field_id).copied().ok_or_else(|| {
        FactDescriptorError::field(field_id.clone(), "fact query references unknown field")
    })
}

fn require_query_exposed(field: &FactFieldDescriptor, context: &'static str) -> Result<()> {
    if field.exposure() == FactFieldExposure::Hidden {
        return Err(FactDescriptorError::field(
            field.field_id().clone(),
            format!("hidden field cannot be used in fact query {context}"),
        ));
    }
    Ok(())
}

fn canonical_compiled_query_bytes(
    descriptor: &FactDescriptor,
    descriptor_hash: &ContentDigest,
    predicates: &[FactQueryPredicate],
    return_fields: &[FactQueryReturnField],
    ordering: &FactOrderingName,
    limit: Option<u64>,
) -> Result<CanonicalJsonBytes> {
    let predicates = predicates
        .iter()
        .map(canonical_query_predicate_value)
        .collect::<Result<Vec<_>>>()?;
    let return_fields = return_fields
        .iter()
        .map(canonical_query_return_field_value)
        .collect::<Result<Vec<_>>>()?;
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query.v1".to_owned()),
        ),
        (
            "fact_kind",
            CanonicalValue::String(descriptor.fact_kind().as_str().to_owned()),
        ),
        (
            "resolved_descriptor",
            CanonicalValue::String(descriptor_hash.as_str().to_owned()),
        ),
        ("predicates", CanonicalValue::Array(predicates)),
        ("return_fields", CanonicalValue::Array(return_fields)),
        (
            "ordering",
            CanonicalValue::String(ordering.as_str().to_owned()),
        ),
        (
            "limit",
            limit
                .map(CanonicalValue::Unsigned)
                .unwrap_or(CanonicalValue::Null),
        ),
    ])?;
    Ok(CanonicalJsonBytes::from_value(&value))
}

fn canonical_query_predicate_value(predicate: &FactQueryPredicate) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(predicate.field_id.as_str().to_owned()),
        ),
        (
            "operator",
            CanonicalValue::String(predicate.operator.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(predicate.value.value_type().as_str().to_owned()),
        ),
        ("value", predicate.value.canonical_value()),
    ])
}

fn canonical_query_return_field_value(field: &FactQueryReturnField) -> Result<CanonicalValue> {
    canonical_object([(
        "field_id",
        CanonicalValue::String(field.field_id.as_str().to_owned()),
    )])
}

fn canonical_query_plan_value(plan: &CanonicalFactQueryPlan) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-plan.v1".to_owned()),
        ),
        (
            "store_scope",
            CanonicalValue::String(plan.store_scope.as_str().to_owned()),
        ),
        (
            "query_scope",
            canonical_query_scope_value(&plan.query_scope)?,
        ),
        (
            "query_compiler_version",
            CanonicalValue::String(plan.query_compiler_version.as_str().to_owned()),
        ),
        (
            "canonicalizer_version",
            CanonicalValue::String(plan.canonicalizer_version.as_str().to_owned()),
        ),
        (
            "resolved_descriptor",
            CanonicalValue::String(plan.resolved_descriptor.as_str().to_owned()),
        ),
        (
            "scope_decision_evidence",
            canonical_scope_decision_evidence_value(&plan.scope_decision_evidence)?,
        ),
        (
            "canonical_query",
            CanonicalValue::String(plan.canonical_query.as_str().to_owned()),
        ),
        (
            "canonical_query_hash",
            CanonicalValue::String(plan.canonical_query_hash.as_str().to_owned()),
        ),
        ("ordering", canonical_fact_ordering_value(&plan.ordering)?),
        (
            "limit",
            plan.limit
                .map(CanonicalValue::Unsigned)
                .unwrap_or(CanonicalValue::Null),
        ),
    ])
}

fn canonical_query_scope_value(scope: &FactQueryScope) -> Result<CanonicalValue> {
    canonical_object([
        (
            "audience",
            CanonicalValue::String(scope.audience.as_str().to_owned()),
        ),
        (
            "scope",
            CanonicalValue::String(scope.scope.as_str().to_owned()),
        ),
    ])
}

fn canonical_scope_decision_evidence_value(
    evidence: &ScopeDecisionEvidence,
) -> Result<CanonicalValue> {
    canonical_object([(
        "decision_hash",
        CanonicalValue::String(evidence.decision_hash.as_str().to_owned()),
    )])
}

fn canonical_fact_ordering_value(ordering: &FactOrdering) -> Result<CanonicalValue> {
    let terms = ordering
        .terms
        .iter()
        .map(canonical_ordering_term_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "name",
            CanonicalValue::String(ordering.name.as_str().to_owned()),
        ),
        ("terms", CanonicalValue::Array(terms)),
    ])
}

fn canonical_query_receipt_body_value(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<CanonicalValue> {
    canonical_query_receipt_body_parts_value(
        plan_hash,
        &receipt.read_frontier,
        receipt.frontier_type,
        &receipt.returned_refs,
        receipt.returned_field_summaries.as_ref(),
        &receipt.result_set_digest,
        receipt.result_cardinality,
    )
}

fn canonical_query_receipt_body_parts_value(
    plan_hash: &ContentDigest,
    read_frontier: &StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
    result_set_digest: &ContentDigest,
    result_cardinality: QueryResultCardinality,
) -> Result<CanonicalValue> {
    let returned_refs = returned_refs
        .iter()
        .map(canonical_internal_fact_ref_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-receipt-body.v1".to_owned()),
        ),
        (
            "plan_hash",
            CanonicalValue::String(plan_hash.as_str().to_owned()),
        ),
        (
            "read_frontier",
            canonical_store_read_frontier_value(read_frontier)?,
        ),
        (
            "frontier_type",
            CanonicalValue::String(frontier_type.as_str().to_owned()),
        ),
        ("returned_refs", CanonicalValue::Array(returned_refs)),
        (
            "returned_field_summaries",
            optional_returned_field_summaries_value(returned_field_summaries)?,
        ),
        (
            "result_set_digest",
            CanonicalValue::String(result_set_digest.as_str().to_owned()),
        ),
        (
            "result_cardinality",
            canonical_query_result_cardinality_value(result_cardinality)?,
        ),
    ])
}

fn canonical_fact_query_result_set_value(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalValue> {
    let returned_refs = returned_refs
        .iter()
        .map(canonical_internal_fact_ref_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-result-set.v1".to_owned()),
        ),
        ("returned_refs", CanonicalValue::Array(returned_refs)),
        (
            "returned_field_summaries",
            optional_returned_field_summaries_value(returned_field_summaries)?,
        ),
    ])
}

fn canonical_query_receipt_value(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "body",
            canonical_query_receipt_body_value(plan_hash, receipt)?,
        ),
        (
            "store_receipt_hash",
            CanonicalValue::String(receipt.store_receipt_hash.as_str().to_owned()),
        ),
        (
            "store_receipt_authentication",
            canonical_store_receipt_authentication_value(&receipt.store_receipt_authentication)?,
        ),
    ])
}

fn canonical_query_evidence_value(evidence: &FactQueryEvidence) -> Result<CanonicalValue> {
    let plan_hash = fact_query_plan_hash(&evidence.plan)?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-evidence.v1".to_owned()),
        ),
        ("plan", canonical_query_plan_value(&evidence.plan)?),
        (
            "receipt",
            canonical_query_receipt_value(&plan_hash, &evidence.receipt)?,
        ),
        (
            "selection",
            canonical_fact_selection_evidence_value(&evidence.selection)?,
        ),
    ])
}

fn canonical_store_read_frontier_value(frontier: &StoreReadFrontier) -> Result<CanonicalValue> {
    canonical_object([
        (
            "store_scope",
            CanonicalValue::String(frontier.store_scope.as_str().to_owned()),
        ),
        (
            "query_scope",
            canonical_query_scope_value(&frontier.query_scope)?,
        ),
        (
            "descriptor_catalog_watermark",
            CanonicalValue::Unsigned(frontier.descriptor_catalog_watermark.as_u64()),
        ),
        (
            "projection_generation",
            CanonicalValue::Unsigned(frontier.projection_generation.as_u64()),
        ),
        (
            "max_included_store_commit_order",
            CanonicalValue::Unsigned(frontier.max_included_store_commit_order),
        ),
        (
            "commit_watermark",
            CanonicalValue::Unsigned(frontier.commit_watermark.as_u64()),
        ),
    ])
}

fn canonical_internal_fact_ref_value(reference: &InternalFactRef) -> Result<CanonicalValue> {
    let parts = &reference.parts;
    canonical_object([
        (
            "fact_claim_id",
            canonical_fact_claim_id_value(&parts.fact_claim_id)?,
        ),
        (
            "source_event_id",
            CanonicalValue::String(parts.source_event_id.as_str().to_owned()),
        ),
        (
            "recorded_at",
            CanonicalValue::String(parts.recorded_at.clone()),
        ),
        (
            "observed_at",
            optional_string_value(parts.observed_at.as_deref()),
        ),
        (
            "visibility",
            canonical_fact_visibility_value(&parts.visibility)?,
        ),
        (
            "fact_kind",
            CanonicalValue::String(parts.fact_kind.as_str().to_owned()),
        ),
        (
            "fact_descriptor_hash",
            CanonicalValue::String(parts.fact_descriptor_hash.as_str().to_owned()),
        ),
        (
            "fact_subject_namespace_hash",
            CanonicalValue::String(parts.fact_subject_namespace_hash.as_str().to_owned()),
        ),
        (
            "fact_key",
            CanonicalValue::String(parts.fact_key.as_str().to_owned()),
        ),
        (
            "subject_material_hash",
            CanonicalValue::String(parts.subject_material_hash.as_str().to_owned()),
        ),
        (
            "request_schema_id",
            optional_schema_id_value(parts.request_schema_id.as_ref()),
        ),
        (
            "request_hash",
            optional_digest_value(parts.request_hash.as_ref()),
        ),
        (
            "response_schema_id",
            CanonicalValue::String(parts.response_schema_id.as_str().to_owned()),
        ),
        (
            "response_hash",
            CanonicalValue::String(parts.response_hash.as_str().to_owned()),
        ),
        (
            "artifact_id",
            CanonicalValue::String(parts.artifact_id.as_str().to_owned()),
        ),
        (
            "artifact_evidence_hash",
            CanonicalValue::String(parts.artifact_evidence_hash.as_str().to_owned()),
        ),
        (
            "capability_kind",
            CanonicalValue::String(parts.capability_kind.as_str().to_owned()),
        ),
        (
            "capability_version",
            CanonicalValue::String(parts.capability_version.as_str().to_owned()),
        ),
        (
            "adapter_kind",
            CanonicalValue::String(parts.adapter_kind.as_str().to_owned()),
        ),
        (
            "adapter_version",
            CanonicalValue::String(parts.adapter_version.as_str().to_owned()),
        ),
    ])
}

fn canonical_fact_claim_id_value(claim_id: &FactClaimId) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-claim-id.v1".to_owned()),
        ),
        (
            "source_run_id",
            CanonicalValue::String(claim_id.source_run_id.as_str().to_owned()),
        ),
        ("source_seq", CanonicalValue::Unsigned(claim_id.source_seq)),
        (
            "source_ordinal",
            CanonicalValue::Unsigned(u64::from(claim_id.source_ordinal)),
        ),
    ])
}

fn canonical_fact_visibility_value(visibility: &FactVisibility) -> Result<CanonicalValue> {
    match visibility {
        FactVisibility::RunPrivate => {
            canonical_object([("kind", CanonicalValue::String("run_private".to_owned()))])
        }
        FactVisibility::Indexed { audience, scope } => canonical_object([
            ("kind", CanonicalValue::String("indexed".to_owned())),
            (
                "audience",
                CanonicalValue::String(audience.as_str().to_owned()),
            ),
            ("scope", CanonicalValue::String(scope.as_str().to_owned())),
        ]),
    }
}

fn optional_returned_field_summaries_value(
    summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalValue> {
    summaries
        .map(canonical_returned_field_summaries_value)
        .unwrap_or(Ok(CanonicalValue::Null))
}

fn canonical_returned_field_summaries_value(
    summaries: &ReturnedFieldSummaries,
) -> Result<CanonicalValue> {
    let summaries = summaries
        .summaries
        .iter()
        .map(canonical_returned_fact_field_summary_value)
        .collect::<Result<Vec<_>>>()?;
    Ok(CanonicalValue::Array(summaries))
}

fn canonical_returned_fact_field_summary_value(
    summary: &ReturnedFactFieldSummary,
) -> Result<CanonicalValue> {
    let fields = summary
        .fields
        .iter()
        .map(canonical_returned_field_value_summary_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "fact_claim_id",
            canonical_fact_claim_id_value(&summary.fact_claim_id)?,
        ),
        ("fields", CanonicalValue::Array(fields)),
    ])
}

fn canonical_returned_field_value_summary_value(
    summary: &ReturnedFieldValueSummary,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(summary.field_id.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(summary.value_type.as_str().to_owned()),
        ),
        ("value", summary.value.canonical_value()),
    ])
}

fn canonical_query_result_cardinality_value(
    cardinality: QueryResultCardinality,
) -> Result<CanonicalValue> {
    match cardinality {
        QueryResultCardinality::Exact(value) => canonical_object([
            ("kind", CanonicalValue::String("exact".to_owned())),
            ("value", CanonicalValue::Unsigned(value)),
        ]),
        QueryResultCardinality::AtLeast(value) => canonical_object([
            ("kind", CanonicalValue::String("at_least".to_owned())),
            ("value", CanonicalValue::Unsigned(value)),
        ]),
        QueryResultCardinality::NotCounted => {
            canonical_object([("kind", CanonicalValue::String("not_counted".to_owned()))])
        }
    }
}

fn canonical_store_receipt_authentication_value(
    auth: &StoreReceiptAuthentication,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "store_identity",
            CanonicalValue::String(auth.store_identity.as_str().to_owned()),
        ),
        (
            "scheme",
            CanonicalValue::String(auth.scheme.as_str().to_owned()),
        ),
        (
            "key_id",
            optional_checked_string_value(auth.key_id.as_ref()),
        ),
        (
            "signature_or_mac",
            CanonicalValue::Bytes(CanonicalBytes::new(auth.signature_or_mac.clone())),
        ),
    ])
}

fn canonical_fact_selection_evidence_value(
    selection: &FactSelectionEvidence,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "selection_policy_hash",
            CanonicalValue::String(selection.selection_policy_hash.as_str().to_owned()),
        ),
        (
            "selected_indices",
            CanonicalValue::Array(
                selection
                    .selected_indices
                    .iter()
                    .copied()
                    .map(CanonicalValue::Unsigned)
                    .collect(),
            ),
        ),
        (
            "selected_summaries_digest",
            optional_digest_value(selection.selected_summaries_digest.as_ref()),
        ),
    ])
}

fn optional_checked_string_value<T>(value: Option<&T>) -> CanonicalValue
where
    T: AsRef<str>,
{
    value
        .map(|value| CanonicalValue::String(value.as_ref().to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_string_value(value: Option<&str>) -> CanonicalValue {
    value
        .map(|value| CanonicalValue::String(value.to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_schema_id_value(value: Option<&SchemaId>) -> CanonicalValue {
    value
        .map(|value| CanonicalValue::String(value.as_str().to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_digest_value(value: Option<&ContentDigest>) -> CanonicalValue {
    value
        .map(|value| CanonicalValue::String(value.as_str().to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_scale_value(scale: Option<FactScale>) -> CanonicalValue {
    scale
        .map(|scale| CanonicalValue::Signed(i64::from(scale.exponent())))
        .unwrap_or(CanonicalValue::Null)
}

fn canonical_object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))
}

fn extract_field_scalar(
    field: &FactFieldDescriptor,
    subject: &CanonicalValue,
    response: &CanonicalValue,
    metadata: Option<&FactExtractionMetadata>,
) -> Result<Option<FactCanonicalScalar>> {
    match &field.accessor {
        FactFieldAccessor::SubjectPath(path) => extract_path_scalar(field, subject, path),
        FactFieldAccessor::ResponsePath(path) => extract_path_scalar(field, response, path),
        FactFieldAccessor::Metadata(metadata_field) => {
            let metadata = metadata.ok_or_else(|| {
                FactDescriptorError::field(
                    field.field_id.clone(),
                    "metadata extraction requires metadata",
                )
            })?;
            metadata_scalar(field, *metadata_field, metadata)
        }
    }
}

fn extract_path_scalar(
    field: &FactFieldDescriptor,
    root: &CanonicalValue,
    path: &CanonicalValuePath,
) -> Result<Option<FactCanonicalScalar>> {
    let Some(value) = extract_path_value(field, root, path)? else {
        return Ok(None);
    };
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    scalar_from_value(field, value).map(Some)
}

fn extract_path_value<'a>(
    field: &FactFieldDescriptor,
    root: &'a CanonicalValue,
    path: &CanonicalValuePath,
) -> Result<Option<&'a CanonicalValue>> {
    let mut current = root;
    for segment in path.raw.split('.') {
        match current {
            CanonicalValue::Object(object) => {
                let next = object
                    .entries()
                    .find(|(key, _)| *key == segment)
                    .map(|(_, value)| value);
                let Some(next) = next else {
                    return Ok(None);
                };
                current = next;
            }
            CanonicalValue::Null => return Ok(None),
            CanonicalValue::Array(_) => {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "arrays, wildcards, slices, and repeated values are unsupported in fact paths",
                ));
            }
            _ => {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    format!("path segment {segment:?} traversed a non-object scalar"),
                ));
            }
        }
    }
    Ok(Some(current))
}

fn metadata_scalar(
    field: &FactFieldDescriptor,
    metadata_field: FactMetadataField,
    metadata: &FactExtractionMetadata,
) -> Result<Option<FactCanonicalScalar>> {
    let scalar = match metadata_field {
        FactMetadataField::RecordedAt => {
            Some(FactCanonicalScalar::Timestamp(metadata.recorded_at.clone()))
        }
        FactMetadataField::ObservedAt => metadata
            .observed_at
            .as_ref()
            .map(|value| FactCanonicalScalar::Timestamp(value.clone())),
        FactMetadataField::StoreCommitOrder => Some(FactCanonicalScalar::UnsignedInteger(
            metadata.store_commit_order,
        )),
    };
    if let Some(scalar) = &scalar {
        validate_extracted_scalar(field, scalar)?;
    }
    Ok(scalar)
}

fn scalar_from_value(
    field: &FactFieldDescriptor,
    value: &CanonicalValue,
) -> Result<FactCanonicalScalar> {
    let scalar = match (field.value_type, value) {
        (FactFieldValueType::String, CanonicalValue::String(value)) => {
            FactCanonicalScalar::String(value.clone())
        }
        (FactFieldValueType::Boolean, CanonicalValue::Bool(value)) => {
            FactCanonicalScalar::Boolean(*value)
        }
        (FactFieldValueType::SignedInteger, CanonicalValue::Signed(value)) => {
            FactCanonicalScalar::SignedInteger(*value)
        }
        (FactFieldValueType::UnsignedInteger, CanonicalValue::Unsigned(value)) => {
            FactCanonicalScalar::UnsignedInteger(*value)
        }
        (FactFieldValueType::Timestamp, CanonicalValue::String(value)) => {
            FactCanonicalScalar::timestamp(value.clone())?
        }
        (FactFieldValueType::DecimalString, CanonicalValue::Decimal(value)) => {
            FactCanonicalScalar::DecimalString(value.clone())
        }
        (FactFieldValueType::Digest, CanonicalValue::String(value)) => {
            let digest = ContentDigest::parse(value).map_err(|error| {
                FactDescriptorError::field(
                    field.field_id.clone(),
                    format!("invalid digest scalar: {error}"),
                )
            })?;
            FactCanonicalScalar::Digest(digest)
        }
        (_, CanonicalValue::Array(_)) => {
            return Err(FactDescriptorError::field(
                field.field_id.clone(),
                "arrays, wildcards, slices, and repeated values are unsupported fact scalars",
            ));
        }
        (_, CanonicalValue::Object(_)) => {
            return Err(FactDescriptorError::field(
                field.field_id.clone(),
                "object values are unsupported fact scalars",
            ));
        }
        (_, CanonicalValue::Null) => {
            return Err(FactDescriptorError::field(
                field.field_id.clone(),
                "null cannot be normalized as a fact scalar",
            ));
        }
        _ => {
            return Err(FactDescriptorError::field(
                field.field_id.clone(),
                format!("scalar type does not match {:?}", field.value_type),
            ));
        }
    };
    validate_extracted_scalar(field, &scalar)?;
    Ok(scalar)
}

fn response_typed_paths(
    descriptor: &FactDescriptor,
) -> Result<BTreeMap<String, FactFieldValueType>> {
    let mut typed_paths = BTreeMap::new();
    for field in descriptor.fields() {
        let FactFieldAccessor::ResponsePath(path) = field.accessor() else {
            continue;
        };
        if let Some(previous) = typed_paths.insert(path.as_str().to_owned(), field.value_type()) {
            if previous != field.value_type() {
                return Err(FactDescriptorError::field(
                    field.field_id().clone(),
                    format!(
                        "response path {} is declared with incompatible value types",
                        path
                    ),
                ));
            }
        }
    }
    Ok(typed_paths)
}

fn json_to_fact_canonical_value(
    value: &serde_json::Value,
    path: &str,
    typed_paths: &BTreeMap<String, FactFieldValueType>,
) -> Result<CanonicalValue> {
    if let Some(value_type) = typed_paths.get(path) {
        return json_to_typed_fact_scalar(value, *value_type, path);
    }

    match value {
        serde_json::Value::Null => Ok(CanonicalValue::Null),
        serde_json::Value::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                Ok(CanonicalValue::Unsigned(value))
            } else if let Some(value) = value.as_i64() {
                Ok(CanonicalValue::Signed(value))
            } else {
                Err(FactDescriptorError::descriptor(format!(
                    "fact response path {path} contains unsupported floating-point number"
                )))
            }
        }
        serde_json::Value::String(value) => Ok(CanonicalValue::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let child_path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                json_to_fact_canonical_value(value, &child_path, typed_paths)
            })
            .collect::<Result<Vec<_>>>()
            .map(CanonicalValue::Array),
        serde_json::Value::Object(entries) => {
            let values = entries
                .iter()
                .map(|(key, value)| {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    Ok((
                        key.clone(),
                        json_to_fact_canonical_value(value, &child_path, typed_paths)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            canonical_object(values)
        }
    }
}

fn json_to_typed_fact_scalar(
    value: &serde_json::Value,
    value_type: FactFieldValueType,
    path: &str,
) -> Result<CanonicalValue> {
    match value_type {
        FactFieldValueType::String => {
            string_json(value, path).map(|value| CanonicalValue::String(value.to_owned()))
        }
        FactFieldValueType::Boolean => value
            .as_bool()
            .map(CanonicalValue::Bool)
            .ok_or_else(|| fact_scalar_type_error(path, value_type)),
        FactFieldValueType::SignedInteger => value
            .as_i64()
            .map(CanonicalValue::Signed)
            .ok_or_else(|| fact_scalar_type_error(path, value_type)),
        FactFieldValueType::UnsignedInteger => value
            .as_u64()
            .map(CanonicalValue::Unsigned)
            .ok_or_else(|| fact_scalar_type_error(path, value_type)),
        FactFieldValueType::Timestamp => {
            let value = string_json(value, path)?;
            FactCanonicalScalar::timestamp(value.to_owned())?;
            Ok(CanonicalValue::String(value.to_owned()))
        }
        FactFieldValueType::DecimalString => {
            let decimal = DecimalString::new_variable(string_json(value, path)?)
                .map_err(|error| FactDescriptorError::descriptor(error.to_string()))?;
            Ok(CanonicalValue::Decimal(decimal))
        }
        FactFieldValueType::Digest => {
            let value = string_json(value, path)?;
            ContentDigest::parse(value).map_err(|error| {
                FactDescriptorError::descriptor(format!(
                    "fact response path {path} contains invalid digest: {error}"
                ))
            })?;
            Ok(CanonicalValue::String(value.to_owned()))
        }
    }
}

fn string_json<'a>(value: &'a serde_json::Value, path: &str) -> Result<&'a str> {
    value.as_str().ok_or_else(|| {
        FactDescriptorError::descriptor(format!("fact response path {path} is not a string"))
    })
}

fn fact_scalar_type_error(path: &str, value_type: FactFieldValueType) -> FactDescriptorError {
    FactDescriptorError::descriptor(format!(
        "fact response path {path} does not match declared value type {:?}",
        value_type
    ))
}

fn validate_extracted_scalar(
    field: &FactFieldDescriptor,
    scalar: &FactCanonicalScalar,
) -> Result<()> {
    if scalar.value_type() != field.value_type {
        return Err(FactDescriptorError::field(
            field.field_id.clone(),
            format!(
                "field expects {:?} but extracted {:?}",
                field.value_type,
                scalar.value_type()
            ),
        ));
    }
    validate_scalar_size(&field.field_id, scalar)?;
    validate_decimal_scale(field, scalar)?;
    Ok(())
}

fn validate_scalar_size(field_id: &FactFieldId, scalar: &FactCanonicalScalar) -> Result<()> {
    let len = scalar.canonical_text_len();
    if len > MAX_FACT_SCALAR_BYTES {
        return Err(FactDescriptorError::field(
            field_id.clone(),
            format!("scalar exceeds {MAX_FACT_SCALAR_BYTES} byte limit"),
        ));
    }
    Ok(())
}

fn validate_decimal_scale(field: &FactFieldDescriptor, scalar: &FactCanonicalScalar) -> Result<()> {
    let Some(scale) = field.scale else {
        return Ok(());
    };
    let FactCanonicalScalar::DecimalString(value) = scalar else {
        return Ok(());
    };
    if scale.exponent() >= 0 {
        if decimal_fraction_digits(value.as_str()) != 0 {
            return Err(FactDescriptorError::field(
                field.field_id.clone(),
                "decimal field with non-negative scale exponent must not contain fractional digits",
            ));
        }
        return Ok(());
    }

    let expected_digits = usize::from(scale.exponent().unsigned_abs());
    let actual_digits = decimal_fraction_digits(value.as_str());
    if actual_digits != expected_digits {
        return Err(FactDescriptorError::field(
            field.field_id.clone(),
            format!(
                "decimal field scale expects {expected_digits} fractional digits but found {actual_digits}"
            ),
        ));
    }
    Ok(())
}

fn decimal_fraction_digits(value: &str) -> usize {
    value
        .split_once('.')
        .map(|(_, fraction)| fraction.len())
        .unwrap_or(0)
}

fn validate_ordering(
    ordering: &FactOrderingDescriptor,
    fields_by_id: &BTreeMap<FactFieldId, &FactFieldDescriptor>,
) -> Result<()> {
    let mut seen_fields = BTreeSet::new();
    for term in &ordering.terms {
        if !seen_fields.insert(term.field_id.clone()) {
            return Err(FactDescriptorError::ordering(
                ordering.name.clone(),
                format!("duplicate ordering field {}", term.field_id),
            ));
        }

        let Some(field) = fields_by_id.get(&term.field_id) else {
            return Err(FactDescriptorError::ordering(
                ordering.name.clone(),
                format!("ordering references unknown field {}", term.field_id),
            ));
        };

        if !field.sortable {
            return Err(FactDescriptorError::ordering(
                ordering.name.clone(),
                format!("ordering references non-sortable field {}", term.field_id),
            ));
        }
    }

    Ok(())
}

fn validate_dot_path(kind: &'static str, value: &str) -> Result<()> {
    validate_len(kind, value, 256)?;
    let mut saw_segment = false;
    for segment in value.split('.') {
        validate_segment(kind, value, segment)?;
        saw_segment = true;
    }
    if !saw_segment {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "empty value",
        ));
    }
    Ok(())
}

fn validate_qualified_field_path(kind: &'static str, value: &str) -> Result<()> {
    validate_dot_path(kind, value)?;
    let Some(prefix) = value.split('.').next() else {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "missing source prefix",
        ));
    };
    if !matches!(prefix, "subject" | "result" | "metadata") {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "field path must start with subject, result, or metadata",
        ));
    }
    if value.split('.').count() < 2 {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "field path must include a field segment after the source prefix",
        ));
    }
    Ok(())
}

fn validate_len(kind: &'static str, value: &str, max: usize) -> Result<()> {
    if value.is_empty() {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "empty value",
        ));
    }
    if value.len() > max {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            format!("too long; max {max} bytes"),
        ));
    }
    Ok(())
}

fn validate_segment(kind: &'static str, whole: &str, segment: &str) -> Result<()> {
    if segment.is_empty() {
        return Err(FactDescriptorError::invalid_string(
            kind,
            whole,
            "empty path segment",
        ));
    }

    let mut chars = segment.chars();
    let first = chars.next().expect("segment is non-empty");
    if !first.is_ascii_lowercase() {
        return Err(FactDescriptorError::invalid_string(
            kind,
            whole,
            "segment must start with lowercase ascii",
        ));
    }

    for ch in chars {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
            return Err(FactDescriptorError::invalid_string(
                kind,
                whole,
                format!("invalid segment character {ch:?}"),
            ));
        }
    }

    Ok(())
}

impl FactFieldPath {
    fn source_prefix(&self) -> Option<&str> {
        self.raw.split('.').next()
    }
}

mod schema_id_serde {
    use mfm_ids::SchemaId;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(
        schema_id: &SchemaId,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(schema_id.as_str())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> std::result::Result<SchemaId, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        SchemaId::parse(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use mfm_ids::{DigestAlgorithm, DigestBytes};

    use super::*;

    fn schema_id(name: &str) -> SchemaId {
        SchemaId::new(
            name,
            "mfm.test.v1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        )
        .expect("schema id")
    }

    fn digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn event_id(byte: u8) -> EventId {
        EventId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn artifact_id(byte: u8) -> ArtifactId {
        ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn capability_kind() -> CapabilityKind {
        CapabilityKind::new(
            "mfm.test.capability",
            "read",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([31; 32]),
        )
        .expect("capability kind")
    }

    fn adapter_kind() -> AdapterKind {
        AdapterKind::new(
            "mfm.test.adapter",
            "read",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([32; 32]),
        )
        .expect("adapter kind")
    }

    fn run_id(byte: u8) -> RunId {
        RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn subject_field(id: &str, path: &str) -> FactFieldDescriptor {
        FactFieldDescriptor::new(
            FactFieldId::new(id).expect("field id"),
            FactFieldPath::new(path).expect("field path"),
            FactFieldValueType::String,
            FactFieldAccessor::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
            vec![FactQueryOperator::Equal],
            FactFieldExposure::Returnable,
            None,
            None,
            false,
            true,
        )
        .expect("subject field")
    }

    fn sortable_result_field(id: &str, path: &str) -> FactFieldDescriptor {
        FactFieldDescriptor::new(
            FactFieldId::new(id).expect("field id"),
            FactFieldPath::new(path).expect("field path"),
            FactFieldValueType::UnsignedInteger,
            FactFieldAccessor::ResponsePath(CanonicalValuePath::new("height").expect("path")),
            vec![
                FactQueryOperator::Equal,
                FactQueryOperator::GreaterThan,
                FactQueryOperator::LessThan,
            ],
            FactFieldExposure::Returnable,
            None,
            None,
            true,
            true,
        )
        .expect("result field")
    }

    fn optional_result_field(id: &str, path: &str, accessor_path: &str) -> FactFieldDescriptor {
        FactFieldDescriptor::new(
            FactFieldId::new(id).expect("field id"),
            FactFieldPath::new(path).expect("field path"),
            FactFieldValueType::UnsignedInteger,
            FactFieldAccessor::ResponsePath(
                CanonicalValuePath::new(accessor_path).expect("accessor path"),
            ),
            vec![FactQueryOperator::Equal],
            FactFieldExposure::QueryOnly,
            None,
            None,
            true,
            false,
        )
        .expect("optional result field")
    }

    fn metadata_recorded_at_field() -> FactFieldDescriptor {
        FactFieldDescriptor::new(
            FactFieldId::new("metadata.recorded_at").expect("field id"),
            FactFieldPath::new("metadata.recorded_at").expect("field path"),
            FactFieldValueType::Timestamp,
            FactFieldAccessor::Metadata(FactMetadataField::RecordedAt),
            vec![
                FactQueryOperator::Equal,
                FactQueryOperator::GreaterThanOrEqual,
            ],
            FactFieldExposure::Hidden,
            None,
            None,
            true,
            true,
        )
        .expect("metadata field")
    }

    fn decimal_result_field() -> FactFieldDescriptor {
        FactFieldDescriptor::new(
            FactFieldId::new("result.price").expect("field id"),
            FactFieldPath::new("result.price").expect("field path"),
            FactFieldValueType::DecimalString,
            FactFieldAccessor::ResponsePath(CanonicalValuePath::new("price").expect("path")),
            vec![FactQueryOperator::Equal, FactQueryOperator::GreaterThan],
            FactFieldExposure::Returnable,
            Some(FactUnit::new("usd").expect("unit")),
            Some(FactScale::new(-2).expect("scale")),
            true,
            true,
        )
        .expect("decimal field")
    }

    fn descriptor(fields: Vec<FactFieldDescriptor>) -> Result<FactDescriptor> {
        FactDescriptor::new(
            FactKind::new("chain.head")?,
            schema_id("mfm.test.fact.descriptor"),
            schema_id("mfm.test.subject"),
            schema_id("mfm.test.response"),
            None,
            fields,
            vec![FactOrderingDescriptor::new(
                FactOrderingName::new("result.height.desc")?,
                vec![FactOrderingTerm::new(
                    FactFieldId::new("result.height")?,
                    SortDirection::Descending,
                    NullOrdering::Last,
                    false,
                )],
            )?],
        )
    }

    fn descriptor_without_orderings(fields: Vec<FactFieldDescriptor>) -> Result<FactDescriptor> {
        FactDescriptor::new(
            FactKind::new("chain.head")?,
            schema_id("mfm.test.fact.descriptor"),
            schema_id("mfm.test.subject"),
            schema_id("mfm.test.response"),
            None,
            fields,
            Vec::new(),
        )
    }

    #[test]
    fn descriptor_accepts_valid_subject_result_and_ordering() {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("valid descriptor");

        assert_eq!(descriptor.fact_kind().as_str(), "chain.head");
        assert_eq!(descriptor.fields().len(), 2);
    }

    #[test]
    fn descriptor_rejects_duplicate_field_ids() {
        let error = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            subject_field("subject.chain", "subject.network"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect_err("duplicate field id");

        assert!(error.to_string().contains("duplicate field id"));
    }

    #[test]
    fn descriptor_rejects_zero_subject_fields() {
        let error = descriptor(vec![sortable_result_field(
            "result.height",
            "result.height",
        )])
        .expect_err("zero subject fields");

        assert!(error.to_string().contains("at least one subject field"));
    }

    #[test]
    fn descriptor_rejects_optional_subject_fields() {
        let optional_subject = FactFieldDescriptor::new(
            FactFieldId::new("subject.chain").expect("field id"),
            FactFieldPath::new("subject.chain").expect("field path"),
            FactFieldValueType::String,
            FactFieldAccessor::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
            vec![FactQueryOperator::Equal],
            FactFieldExposure::Returnable,
            None,
            None,
            false,
            false,
        )
        .expect("field constructor allows subject required check at descriptor level");

        let error = descriptor(vec![
            optional_subject,
            sortable_result_field("result.height", "result.height"),
        ])
        .expect_err("optional subject");

        assert!(error
            .to_string()
            .contains("subject fields must be required"));
    }

    #[test]
    fn field_constructor_rejects_accessor_path_prefix_conflict() {
        let error = FactFieldDescriptor::new(
            FactFieldId::new("subject.chain").expect("field id"),
            FactFieldPath::new("result.chain").expect("field path"),
            FactFieldValueType::String,
            FactFieldAccessor::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
            vec![FactQueryOperator::Equal],
            FactFieldExposure::Returnable,
            None,
            None,
            false,
            true,
        )
        .expect_err("prefix mismatch");

        assert!(error.to_string().contains("path prefix"));
    }

    #[test]
    fn field_constructor_rejects_incompatible_operator() {
        let error = FactFieldDescriptor::new(
            FactFieldId::new("subject.chain").expect("field id"),
            FactFieldPath::new("subject.chain").expect("field path"),
            FactFieldValueType::String,
            FactFieldAccessor::SubjectPath(CanonicalValuePath::new("chain").expect("path")),
            vec![FactQueryOperator::GreaterThan],
            FactFieldExposure::Returnable,
            None,
            None,
            false,
            true,
        )
        .expect_err("incompatible operator");

        assert!(error.to_string().contains("incompatible"));
    }

    #[test]
    fn descriptor_rejects_ordering_for_missing_field() {
        let fields = vec![subject_field("subject.chain", "subject.chain")];
        let error = descriptor(fields).expect_err("missing ordering field");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn descriptor_rejects_ordering_for_non_sortable_field() {
        let error = FactDescriptor::new(
            FactKind::new("chain.head").expect("kind"),
            schema_id("mfm.test.fact.descriptor"),
            schema_id("mfm.test.subject"),
            schema_id("mfm.test.response"),
            None,
            vec![subject_field("subject.chain", "subject.chain")],
            vec![FactOrderingDescriptor::new(
                FactOrderingName::new("subject.chain.asc").expect("ordering"),
                vec![FactOrderingTerm::new(
                    FactFieldId::new("subject.chain").expect("field"),
                    SortDirection::Ascending,
                    NullOrdering::Last,
                    false,
                )],
            )
            .expect("ordering")],
        )
        .expect_err("non-sortable ordering field");

        assert!(error.to_string().contains("non-sortable"));
    }

    #[test]
    fn checked_strings_reject_invalid_fact_kind() {
        let error = FactKind::new("Wallet Balance").expect_err("invalid kind");

        assert!(error.to_string().contains("lowercase ascii"));
    }

    #[test]
    fn visibility_indexed_default_records_audience_and_scope() {
        let visibility = FactVisibility::indexed_default(FactAudience::Control);

        assert_eq!(
            visibility,
            FactVisibility::Indexed {
                audience: FactAudience::Control,
                scope: FactVisibilityScope::Default,
            }
        );
    }

    #[test]
    fn descriptor_hash_is_stable_for_reordered_fields() {
        let first = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let second = descriptor(vec![
            sortable_result_field("result.height", "result.height"),
            subject_field("subject.chain", "subject.chain"),
        ])
        .expect("descriptor");

        assert_eq!(
            canonical_fact_descriptor_bytes(&first).expect("canonical first"),
            canonical_fact_descriptor_bytes(&second).expect("canonical second")
        );
        assert_eq!(
            fact_descriptor_hash(&first).expect("hash first"),
            fact_descriptor_hash(&second).expect("hash second")
        );
    }

    #[test]
    fn canonical_descriptor_bytes_parse_back_to_descriptor_only_when_canonical() {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let canonical = canonical_fact_descriptor_bytes(&descriptor).expect("canonical");
        let parsed = parse_canonical_fact_descriptor_bytes(canonical.as_bytes()).expect("parsed");

        assert_eq!(
            canonical_fact_descriptor_bytes(&parsed).expect("parsed canonical"),
            canonical
        );
        assert_eq!(
            fact_descriptor_hash(&parsed).expect("parsed hash"),
            fact_descriptor_hash(&descriptor).expect("descriptor hash")
        );

        let serde_json = serde_json::to_vec(&descriptor).expect("serde descriptor");
        assert!(parse_canonical_fact_descriptor_bytes(&serde_json).is_err());
    }

    #[test]
    fn subject_evidence_carries_canonical_subject_material() {
        let material = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
            FactFieldId::new("subject.chain").expect("field"),
            FactFieldValueType::String,
            FactCanonicalScalar::string("bitcoin"),
        )
        .expect("value")])
        .expect("material");
        let namespace_hash = digest(41);
        let evidence = FactSubjectEvidence::from_material(namespace_hash.clone(), &material)
            .expect("subject evidence");
        let canonical = canonical_fact_subject_material_bytes(&material).expect("canonical");

        assert_eq!(evidence.subject_material().as_bytes(), canonical.as_bytes());
        assert_eq!(
            evidence.subject_material_hash(),
            &subject_material_hash(&material).expect("material hash")
        );
        assert_eq!(
            evidence.fact_key(),
            &derive_fact_key(namespace_hash, evidence.subject_material_hash().clone())
                .expect("fact key")
        );
        assert_eq!(
            parse_canonical_fact_subject_material_bytes(evidence.subject_material().as_bytes())
                .expect("parsed"),
            material
        );
    }

    #[test]
    fn subject_namespace_excludes_result_fields() {
        let first = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let second = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.block_number", "result.block_number"),
        ])
        .expect("descriptor");

        let first_namespace = fact_subject_namespace(&first).expect("namespace");
        let second_namespace = fact_subject_namespace(&second).expect("namespace");

        assert_eq!(
            canonical_fact_subject_namespace_bytes(&first_namespace).expect("first bytes"),
            canonical_fact_subject_namespace_bytes(&second_namespace).expect("second bytes")
        );
    }

    #[test]
    fn fact_key_changes_when_subject_value_changes() {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let namespace = fact_subject_namespace(&descriptor).expect("namespace");
        let namespace_hash = fact_subject_namespace_hash(&namespace).expect("namespace hash");

        let bitcoin = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
            FactFieldId::new("subject.chain").expect("field"),
            FactFieldValueType::String,
            FactCanonicalScalar::string("bitcoin"),
        )
        .expect("value")])
        .expect("material");
        let ethereum = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
            FactFieldId::new("subject.chain").expect("field"),
            FactFieldValueType::String,
            FactCanonicalScalar::string("ethereum"),
        )
        .expect("value")])
        .expect("material");

        let bitcoin_key = derive_fact_key(
            namespace_hash.clone(),
            subject_material_hash(&bitcoin).expect("bitcoin material hash"),
        )
        .expect("bitcoin key");
        let ethereum_key = derive_fact_key(
            namespace_hash,
            subject_material_hash(&ethereum).expect("ethereum material hash"),
        )
        .expect("ethereum key");

        assert_ne!(bitcoin_key, ethereum_key);
    }

    #[test]
    fn fact_claim_id_uses_run_stream_coordinates() {
        let run_id = run_id(9);
        let claim_id = derive_fact_claim_id(run_id.clone(), 7, 2).expect("claim id");

        assert_eq!(claim_id.source_run_id(), &run_id);
        assert_eq!(claim_id.source_seq(), 7);
        assert_eq!(claim_id.source_ordinal(), 2);
        assert!(canonical_fact_claim_id_bytes(&claim_id)
            .expect("canonical claim id")
            .as_str()
            .contains("\"source_seq\":7"));
        assert!(derive_fact_claim_id(run_id, 0, 0).is_err());
    }

    #[test]
    fn extraction_derives_subject_material_and_terms() {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
            metadata_recorded_at_field(),
        ])
        .expect("descriptor");
        let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
            .expect("subject");
        let response = CanonicalValue::object([("height", CanonicalValue::Unsigned(850_000))])
            .expect("response");
        let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42)
            .expect("metadata");

        let material = extract_subject_material(&descriptor, &subject).expect("subject material");
        assert_eq!(material.values().len(), 1);
        let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");

        assert_eq!(terms.len(), 3);
        assert!(terms
            .iter()
            .any(|term| term.field_id().as_str() == "metadata.recorded_at"));
    }

    #[test]
    fn extraction_optional_missing_field_yields_no_term() {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            optional_result_field(
                "result.optional_height",
                "result.optional_height",
                "missing",
            ),
        ])
        .expect("descriptor");
        let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
            .expect("subject");
        let response = CanonicalValue::object([("height", CanonicalValue::Unsigned(850_000))])
            .expect("response");
        let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42)
            .expect("metadata");

        let terms = extract_terms(&descriptor, &subject, &response, &metadata).expect("terms");

        assert_eq!(terms.len(), 1);
        assert_eq!(terms[0].field_id().as_str(), "subject.chain");
    }

    #[test]
    fn extraction_required_missing_subject_fails() {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let subject =
            CanonicalValue::object([("network", CanonicalValue::String("mainnet".into()))])
                .expect("subject");

        let error =
            extract_subject_material(&descriptor, &subject).expect_err("missing subject field");

        assert!(error.to_string().contains("required subject field missing"));
    }

    #[test]
    fn extraction_rejects_scalar_type_mismatch() {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
            .expect("subject");
        let response =
            CanonicalValue::object([("height", CanonicalValue::String("850000".into()))])
                .expect("response");
        let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42)
            .expect("metadata");

        let error =
            extract_terms(&descriptor, &subject, &response, &metadata).expect_err("type mismatch");

        assert!(error.to_string().contains("scalar type does not match"));
    }

    #[test]
    fn extraction_rejects_arrays() {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let subject = CanonicalValue::object([(
            "chain",
            CanonicalValue::Array(vec![CanonicalValue::String("bitcoin".into())]),
        )])
        .expect("subject");

        let error =
            extract_subject_material(&descriptor, &subject).expect_err("array subject value");

        assert!(error.to_string().contains("arrays"));
    }

    #[test]
    fn extraction_validates_decimal_scale() {
        let descriptor = descriptor_without_orderings(vec![
            subject_field("subject.chain", "subject.chain"),
            decimal_result_field(),
        ])
        .expect("descriptor");
        let subject = CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".into()))])
            .expect("subject");
        let metadata = FactExtractionMetadata::new("2026-07-01T00:00:00Z", None::<String>, 42)
            .expect("metadata");
        let valid_response = CanonicalValue::object([(
            "price",
            CanonicalValue::Decimal(DecimalString::new_variable("12.34").expect("decimal")),
        )])
        .expect("response");
        let invalid_response = CanonicalValue::object([(
            "price",
            CanonicalValue::Decimal(DecimalString::new_variable("12.3").expect("decimal")),
        )])
        .expect("response");

        assert!(extract_terms(&descriptor, &subject, &valid_response, &metadata).is_ok());
        let error =
            extract_terms(&descriptor, &subject, &invalid_response, &metadata).expect_err("scale");
        assert!(error.to_string().contains("fractional digits"));
    }

    #[test]
    fn query_plan_computes_canonical_query_hash_and_rejects_zero_limit() {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let ordering = FactOrdering::from_descriptor(
            descriptor.orderings().first().expect("descriptor ordering"),
        );
        let canonical_query = CanonicalJsonBytes::from_value(
            &CanonicalValue::object([("kind", CanonicalValue::String("chain.head".into()))])
                .expect("query value"),
        );

        let plan = CanonicalFactQueryPlan::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            FactQueryCompilerVersion::new("mfm.facts.query.v1").expect("compiler"),
            FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
            digest(1),
            ScopeDecisionEvidence::new(digest(2)),
            canonical_query.clone(),
            ordering.clone(),
            Some(10),
        )
        .expect("plan");

        assert_eq!(
            plan.canonical_query_hash(),
            &canonical_query.content_digest()
        );
        assert!(CanonicalFactQueryPlan::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            FactQueryCompilerVersion::new("mfm.facts.query.v1").expect("compiler"),
            FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
            digest(1),
            ScopeDecisionEvidence::new(digest(2)),
            canonical_query,
            ordering,
            Some(0),
        )
        .is_err());
    }

    #[test]
    fn query_compiler_builds_descriptor_scoped_canonical_plan() {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
            optional_result_field(
                "result.confirmations",
                "result.confirmations",
                "confirmations",
            ),
        ])
        .expect("descriptor");
        let descriptor_hash = fact_descriptor_hash(&descriptor).expect("descriptor hash");
        let input = FactQueryInput::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(digest(2)),
            vec![
                FactQueryPredicate::new(
                    FactFieldId::new("subject.chain").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::string("bitcoin"),
                ),
                FactQueryPredicate::new(
                    FactFieldId::new("result.height").expect("field"),
                    FactQueryOperator::GreaterThan,
                    FactCanonicalScalar::UnsignedInteger(800_000),
                ),
            ],
            vec![
                FactQueryReturnField::new(FactFieldId::new("subject.chain").expect("field")),
                FactQueryReturnField::new(FactFieldId::new("result.height").expect("field")),
            ],
            FactOrderingName::new("result.height.desc").expect("ordering"),
            Some(25),
        )
        .expect("input");

        let plan = compile_fact_query_plan(&descriptor, input).expect("plan");

        assert_eq!(plan.resolved_descriptor(), &descriptor_hash);
        assert_eq!(
            plan.query_compiler_version().as_str(),
            FACT_QUERY_COMPILER_VERSION
        );
        assert_eq!(
            plan.canonicalizer_version().as_str(),
            FACT_QUERY_CANONICALIZER_VERSION
        );
        assert_eq!(plan.limit(), Some(25));
        assert_eq!(plan.ordering().name().as_str(), "result.height.desc");
        assert_eq!(
            plan.canonical_query_hash(),
            &plan.canonical_query().content_digest()
        );
        let query: serde_json::Value =
            serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
        assert_eq!(query["version"], "mfm.fact-query.v1");
        assert_eq!(query["fact_kind"], "chain.head");
        assert_eq!(query["resolved_descriptor"], descriptor_hash.as_str());
        assert_eq!(query["limit"], 25);
        assert_eq!(query["ordering"], "result.height.desc");
        let predicates = query["predicates"].as_array().expect("predicates");
        assert_eq!(predicates[0]["field_id"], "result.height");
        assert_eq!(predicates[0]["operator"], "greater_than");
        assert_eq!(predicates[0]["value_type"], "unsigned_integer");
        assert_eq!(predicates[1]["field_id"], "subject.chain");
        let return_fields = query["return_fields"].as_array().expect("return fields");
        assert_eq!(return_fields[0]["field_id"], "subject.chain");
        assert_eq!(return_fields[1]["field_id"], "result.height");
        let parsed = parse_canonical_fact_query_shape(&plan).expect("parsed query shape");
        assert_eq!(parsed.predicates().len(), 2);
        assert_eq!(parsed.return_fields().len(), 2);
    }

    #[test]
    fn query_compiler_enforces_exposure_policy() {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
            optional_result_field(
                "result.confirmations",
                "result.confirmations",
                "confirmations",
            ),
            metadata_recorded_at_field(),
        ])
        .expect("descriptor");

        let hidden_predicate = FactQueryInput::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(digest(2)),
            vec![FactQueryPredicate::new(
                FactFieldId::new("metadata.recorded_at").expect("field"),
                FactQueryOperator::GreaterThanOrEqual,
                FactCanonicalScalar::timestamp("2026-01-02T00:00:00Z").expect("timestamp"),
            )],
            vec![FactQueryReturnField::new(
                FactFieldId::new("subject.chain").expect("field"),
            )],
            FactOrderingName::new("result.height.desc").expect("ordering"),
            Some(10),
        )
        .expect("input");
        assert!(compile_fact_query_plan(&descriptor, hidden_predicate)
            .expect_err("hidden predicate")
            .to_string()
            .contains("hidden field"));

        let query_only_return = FactQueryInput::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(digest(2)),
            Vec::new(),
            vec![FactQueryReturnField::new(
                FactFieldId::new("result.confirmations").expect("field"),
            )],
            FactOrderingName::new("result.height.desc").expect("ordering"),
            Some(10),
        )
        .expect("input");
        assert!(compile_fact_query_plan(&descriptor, query_only_return)
            .expect_err("query-only return")
            .to_string()
            .contains("not returnable"));
    }

    #[test]
    fn query_compiler_rejects_invalid_predicates() {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");

        let undeclared_operator = FactQueryInput::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(digest(2)),
            vec![FactQueryPredicate::new(
                FactFieldId::new("result.height").expect("field"),
                FactQueryOperator::GreaterThanOrEqual,
                FactCanonicalScalar::UnsignedInteger(800_000),
            )],
            vec![FactQueryReturnField::new(
                FactFieldId::new("result.height").expect("field"),
            )],
            FactOrderingName::new("result.height.desc").expect("ordering"),
            Some(10),
        )
        .expect("input");
        assert!(compile_fact_query_plan(&descriptor, undeclared_operator)
            .expect_err("undeclared operator")
            .to_string()
            .contains("not declared"));

        let wrong_type = FactQueryInput::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(digest(2)),
            vec![FactQueryPredicate::new(
                FactFieldId::new("result.height").expect("field"),
                FactQueryOperator::GreaterThan,
                FactCanonicalScalar::string("800000"),
            )],
            vec![FactQueryReturnField::new(
                FactFieldId::new("result.height").expect("field"),
            )],
            FactOrderingName::new("result.height.desc").expect("ordering"),
            Some(10),
        )
        .expect("input");
        assert!(compile_fact_query_plan(&descriptor, wrong_type)
            .expect_err("wrong predicate type")
            .to_string()
            .contains("field expects"));
    }

    #[test]
    fn selection_evidence_requires_sorted_unique_indices() {
        assert!(FactSelectionEvidence::new(digest(1), vec![0, 2, 4], None).is_ok());
        assert!(FactSelectionEvidence::new(digest(1), vec![0, 2, 2], None).is_err());
        assert!(FactSelectionEvidence::new(digest(1), vec![2, 1], None).is_err());
    }

    #[test]
    fn receipt_authentication_requires_local_ed25519_key_id_and_signature() {
        assert!(StoreReceiptAuthentication::new(
            StoreIdentity::new("store.default").expect("store"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(StoreKeyId::new("key.default").expect("key")),
            vec![7; 64],
        )
        .is_ok());

        assert!(StoreReceiptAuthentication::new(
            StoreIdentity::new("store.default").expect("store"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            None,
            vec![7; 64],
        )
        .is_err());

        assert!(StoreReceiptAuthentication::new(
            StoreIdentity::new("store.default").expect("store"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(StoreKeyId::new("key.default").expect("key")),
            vec![7; 63],
        )
        .is_err());
    }

    fn internal_ref_parts(visibility: FactVisibility) -> InternalFactRefParts {
        InternalFactRefParts {
            fact_claim_id: FactClaimId::new(run_id(10), 1, 0).expect("claim id"),
            source_event_id: event_id(11),
            recorded_at: "2026-07-01T00:00:00Z".to_owned(),
            observed_at: None,
            visibility,
            fact_kind: FactKind::new("chain.head").expect("kind"),
            fact_descriptor_hash: digest(12),
            fact_subject_namespace_hash: digest(13),
            fact_key: FactKey::from_digest(digest(14)),
            subject_material_hash: digest(15),
            request_schema_id: None,
            request_hash: None,
            response_schema_id: schema_id("mfm.test.response"),
            response_hash: digest(16),
            artifact_id: artifact_id(17),
            artifact_evidence_hash: digest(18),
            capability_kind: capability_kind(),
            capability_version: CapabilityVersion::new("mfm.capability.test.v1")
                .expect("capability version"),
            adapter_kind: adapter_kind(),
            adapter_version: AdapterVersion::new("mfm.adapter.test.v1").expect("adapter version"),
        }
    }

    fn query_evidence_fixture() -> (CanonicalFactQueryPlan, FactQueryReceipt, FactQueryEvidence) {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let ordering =
            FactOrdering::from_descriptor(descriptor.orderings().first().expect("ordering"));
        let canonical_query = CanonicalJsonBytes::from_value(
            &CanonicalValue::object([("kind", CanonicalValue::String("chain.head".into()))])
                .expect("query value"),
        );
        let plan = CanonicalFactQueryPlan::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            FactQueryCompilerVersion::new("mfm.facts.query.v1").expect("compiler"),
            FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
            digest(1),
            ScopeDecisionEvidence::new(digest(2)),
            canonical_query,
            ordering,
            Some(10),
        )
        .expect("plan");
        let frontier = StoreReadFrontier::new(
            StoreScopeRef::new("default").expect("store scope"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            DescriptorCatalogWatermark::new(3),
            FactProjectionGeneration::new(4),
            99,
            StoreCommitWatermark::new(100),
        );
        let auth = StoreReceiptAuthentication::new(
            StoreIdentity::new("store.default").expect("store"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(StoreKeyId::new("key.default").expect("key")),
            vec![7; 64],
        )
        .expect("auth");
        let receipt = FactQueryReceipt::new(
            frontier,
            StoreReadFrontierType::Snapshot,
            Vec::new(),
            None,
            digest(19),
            QueryResultCardinality::Exact(0),
            digest(20),
            auth,
        );
        let selection =
            FactSelectionEvidence::new(digest(21), vec![0], Some(digest(22))).expect("selection");
        let evidence = FactQueryEvidence::new(plan.clone(), receipt.clone(), selection);
        (plan, receipt, evidence)
    }

    #[test]
    fn internal_fact_ref_requires_indexed_visibility_and_request_pairing() {
        assert!(
            InternalFactRef::new(internal_ref_parts(FactVisibility::indexed_default(
                FactAudience::Platform,
            )))
            .is_ok()
        );
        assert!(InternalFactRef::new(internal_ref_parts(FactVisibility::RunPrivate)).is_err());

        let mut parts = internal_ref_parts(FactVisibility::indexed_default(FactAudience::Platform));
        parts.request_schema_id = Some(schema_id("mfm.test.request"));
        assert!(InternalFactRef::new(parts).is_err());
    }

    #[test]
    fn canonical_goldens_match_expected_values() {
        let descriptor = descriptor(vec![
            subject_field("subject.chain", "subject.chain"),
            sortable_result_field("result.height", "result.height"),
        ])
        .expect("descriptor");
        let namespace = fact_subject_namespace(&descriptor).expect("namespace");
        let material = FactSubjectMaterialV1::new(vec![FactSubjectValueV1::new(
            FactFieldId::new("subject.chain").expect("field"),
            FactFieldValueType::String,
            FactCanonicalScalar::string("bitcoin"),
        )
        .expect("value")])
        .expect("material");
        let namespace_hash = fact_subject_namespace_hash(&namespace).expect("namespace hash");
        let material_hash = subject_material_hash(&material).expect("material hash");
        let fact_key =
            derive_fact_key(namespace_hash.clone(), material_hash.clone()).expect("fact key");
        let claim_id = FactClaimId::new(run_id(9), 7, 2).expect("claim id");
        let (plan, receipt, evidence) = query_evidence_fixture();
        let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");

        assert_eq!(
            canonical_fact_descriptor_bytes(&descriptor)
                .expect("descriptor bytes")
                .as_str(),
            r#"{"compatibility_group":null,"descriptor_schema_id":"schema:mfm.test.fact.descriptor:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","fact_kind":"chain.head","fields":[{"accessor":{"path":"height","source":"result"},"exposure":"returnable","field_id":"result.height","operators":["equal","less_than","greater_than"],"path":"result.height","required":true,"scale":null,"sortable":true,"unit":null,"value_type":"unsigned_integer"},{"accessor":{"path":"chain","source":"subject"},"exposure":"returnable","field_id":"subject.chain","operators":["equal"],"path":"subject.chain","required":true,"scale":null,"sortable":false,"unit":null,"value_type":"string"}],"orderings":[{"name":"result.height.desc","terms":[{"direction":"descending","field_id":"result.height","nulls":"last","tie_breaker":false}]}],"response_schema_id":"schema:mfm.test.response:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","subject_schema_id":"schema:mfm.test.subject:mfm.test.v1:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","version":"mfm.facts.v1"}"#
        );
        assert_eq!(
            fact_descriptor_hash(&descriptor)
                .expect("descriptor hash")
                .as_str(),
            "content:sha256-jcs-v1:de4dea3a9f707ddb5c0d5f10a648910f4f3af02be51bb7b1f97a6887fb647907"
        );
        assert_eq!(
            canonical_fact_subject_namespace_bytes(&namespace)
                .expect("namespace bytes")
                .as_str(),
            r#"{"fact_kind":"chain.head","fields":[{"field_id":"subject.chain","scale":null,"unit":null,"value_type":"string"}],"version":"mfm.fact-subject-namespace.v1"}"#
        );
        assert_eq!(
            namespace_hash.as_str(),
            "content:sha256-jcs-v1:4a7e09033d1106f765d2995eaad05e49e358430516e541b1ad6f803a8fd464e1"
        );
        assert_eq!(
            canonical_fact_subject_material_bytes(&material)
                .expect("material bytes")
                .as_str(),
            r#"{"values":[{"field_id":"subject.chain","value":"bitcoin","value_type":"string"}],"version":"mfm.fact-subject-material.v1"}"#
        );
        assert_eq!(
            material_hash.as_str(),
            "content:sha256-jcs-v1:b0fafb9b373281b4c3fad4c39cc691551d305ec5e744b928e48466782e1cc736"
        );
        assert_eq!(
            fact_key.as_str(),
            "content:sha256-jcs-v1:09c665bf288b20e41297b36b39adda099b2ad87948414c60fba3024f95f62455"
        );
        assert_eq!(
            canonical_fact_claim_id_bytes(&claim_id)
                .expect("claim id bytes")
                .as_str(),
            r#"{"source_ordinal":2,"source_run_id":"run:sha256-jcs-v1:0909090909090909090909090909090909090909090909090909090909090909","source_seq":7,"version":"mfm.fact-claim-id.v1"}"#
        );
        assert_eq!(
            canonical_fact_query_plan_bytes(&plan)
                .expect("plan bytes")
                .as_str(),
            r#"{"canonical_query":"{\"kind\":\"chain.head\"}","canonical_query_hash":"content:sha256-jcs-v1:63dccff9320cdcc68affe1e82a03834050ecbef8210854d4ed865721b8f03020","canonicalizer_version":"mfm.canonical.v1","limit":10,"ordering":{"name":"result.height.desc","terms":[{"direction":"descending","field_id":"result.height","nulls":"last","tie_breaker":false}]},"query_compiler_version":"mfm.facts.query.v1","query_scope":{"audience":"platform","scope":"default"},"resolved_descriptor":"content:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","scope_decision_evidence":{"decision_hash":"content:sha256-jcs-v1:0202020202020202020202020202020202020202020202020202020202020202"},"store_scope":"default","version":"mfm.fact-query-plan.v1"}"#
        );
        assert_eq!(
            plan_hash.as_str(),
            "content:sha256-jcs-v1:9282f7eb855fd79a7f907f2745fbf49f3f39b54dfb80a1e51a92444f4f0981a9"
        );
        assert_eq!(
            canonical_fact_query_receipt_body_bytes(&plan_hash, &receipt)
                .expect("receipt body bytes")
                .as_str(),
            r#"{"frontier_type":"snapshot","plan_hash":"content:sha256-jcs-v1:9282f7eb855fd79a7f907f2745fbf49f3f39b54dfb80a1e51a92444f4f0981a9","read_frontier":{"commit_watermark":100,"descriptor_catalog_watermark":3,"max_included_store_commit_order":99,"projection_generation":4,"query_scope":{"audience":"platform","scope":"default"},"store_scope":"default"},"result_cardinality":{"kind":"exact","value":0},"result_set_digest":"content:sha256-jcs-v1:1313131313131313131313131313131313131313131313131313131313131313","returned_field_summaries":null,"returned_refs":[],"version":"mfm.fact-query-receipt-body.v1"}"#
        );
        assert_eq!(
            fact_query_receipt_body_hash(&plan_hash, &receipt)
                .expect("receipt body hash")
                .as_str(),
            "content:sha256-jcs-v1:4557f9049a8464b53264e4204661d056958797ed2aa7599a5476a1359886d897"
        );
        assert_eq!(
            fact_query_evidence_hash(&evidence)
                .expect("evidence hash")
                .as_str(),
            "content:sha256-jcs-v1:e33cc235b61e4c5c5973cc743ad741465f7ee4539c383a0b259b980be0227c8c"
        );
    }

    #[test]
    fn receipt_body_hash_excludes_receipt_hash_and_authentication() {
        let (plan, receipt, _) = query_evidence_fixture();
        let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");
        let baseline =
            fact_query_receipt_body_hash(&plan_hash, &receipt).expect("baseline body hash");

        let mut changed = receipt.clone();
        changed.store_receipt_hash = digest(99);
        changed.store_receipt_authentication = StoreReceiptAuthentication::new(
            StoreIdentity::new("store.default").expect("store"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(StoreKeyId::new("key.default").expect("key")),
            vec![8; 64],
        )
        .expect("auth");

        assert_eq!(
            baseline,
            fact_query_receipt_body_hash(&plan_hash, &changed).expect("changed body hash")
        );
    }

    #[test]
    fn receipt_body_hash_from_parts_matches_receipt_hash() {
        let (plan, receipt, _) = query_evidence_fixture();
        let plan_hash = fact_query_plan_hash(&plan).expect("plan hash");
        assert_eq!(
            fact_query_receipt_body_hash(&plan_hash, &receipt).expect("receipt hash"),
            fact_query_receipt_body_hash_from_parts(
                &plan_hash,
                receipt.read_frontier(),
                receipt.frontier_type(),
                receipt.returned_refs(),
                receipt.returned_field_summaries(),
                receipt.result_set_digest(),
                receipt.result_cardinality(),
            )
            .expect("parts hash")
        );
    }
}
