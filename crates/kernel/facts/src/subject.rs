use std::fmt;

use mfm_canonical::CanonicalValue;
use mfm_ids::ContentDigest;
use mfm_ids::SchemaId;

use crate::extraction::validate_scalar_size;
use crate::*;

/// Canonical subject namespace for a fact descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactSubjectNamespaceV2 {
    pub(crate) fact_kind: FactKind,
    pub(crate) subject_schema_id: SchemaId,
}

impl FactSubjectNamespaceV2 {
    /// Stable subject namespace version string.
    pub const VERSION: &'static str = "mfm.fact-subject-namespace.v2";

    /// Creates a namespace for one fact kind and typed subject schema.
    pub(crate) fn new(fact_kind: FactKind, subject_schema_id: SchemaId) -> Self {
        Self {
            fact_kind,
            subject_schema_id,
        }
    }
}

/// Descriptor field value with its field id, declared type, and canonical scalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactFieldValue {
    pub(crate) field_id: FactFieldId,
    pub(crate) value_type: FactFieldValueType,
    pub(crate) value: FactCanonicalScalar,
}

impl FactFieldValue {
    /// Creates a field value and checks that the scalar matches the declared value type.
    pub fn new(
        field_id: FactFieldId,
        value_type: FactFieldValueType,
        value: FactCanonicalScalar,
    ) -> Result<Self> {
        if value.value_type() != value_type {
            return Err(FactError::field(
                field_id,
                format!(
                    "field value type {:?} does not match scalar type {:?}",
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

/// Canonical full typed subject material for one fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectMaterialV2 {
    pub(crate) subject: CanonicalValue,
}

impl FactSubjectMaterialV2 {
    /// Stable subject material version string.
    pub const VERSION: &'static str = "mfm.fact-subject-material.v2";

    /// Creates subject material from the complete canonical typed subject object.
    pub fn new(subject: CanonicalValue) -> Result<Self> {
        if !matches!(subject, CanonicalValue::Object(_)) {
            return Err(FactError::descriptor(
                "fact subject material must retain a canonical object",
            ));
        }
        Ok(Self { subject })
    }

    /// Returns the complete canonical typed subject object.
    pub const fn subject(&self) -> &CanonicalValue {
        &self.subject
    }
}

/// Stable content-derived key for a fact subject.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactKey {
    pub(crate) digest: ContentDigest,
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
