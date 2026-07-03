use std::collections::BTreeSet;
use std::fmt;

use mfm_ids::ContentDigest;

use crate::extraction::validate_scalar_size;
use crate::*;

/// Canonical subject namespace for a fact descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactSubjectNamespaceV1 {
    pub(crate) fact_kind: FactKind,
    pub(crate) fields: Vec<FactSubjectNamespaceFieldV1>,
}

impl FactSubjectNamespaceV1 {
    /// Stable subject namespace version string.
    pub const VERSION: &'static str = "mfm.fact-subject-namespace.v1";

    /// Creates a subject namespace and sorts fields by field id.
    pub(crate) fn new(
        fact_kind: FactKind,
        fields: Vec<FactSubjectNamespaceFieldV1>,
    ) -> Result<Self> {
        let mut namespace = Self { fact_kind, fields };
        namespace.sort_and_validate()?;
        Ok(namespace)
    }

    fn sort_and_validate(&mut self) -> Result<()> {
        self.fields
            .sort_by(|left, right| left.field_id.cmp(&right.field_id));
        let mut seen = BTreeSet::new();
        for field in &self.fields {
            if !seen.insert(field.field_id.clone()) {
                return Err(FactError::descriptor(format!(
                    "duplicate subject namespace field {}",
                    field.field_id
                )));
            }
        }
        if self.fields.is_empty() {
            return Err(FactError::descriptor(
                "subject namespace must contain at least one field",
            ));
        }
        Ok(())
    }
}

/// Subject namespace field that participates in fact-key derivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactSubjectNamespaceFieldV1 {
    pub(crate) field_id: FactFieldId,
    pub(crate) value_type: FactFieldValueType,
    pub(crate) unit: Option<FactUnit>,
    pub(crate) scale: Option<FactScale>,
}

impl FactSubjectNamespaceFieldV1 {
    /// Creates a subject namespace field.
    pub(crate) fn new(
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

/// Canonical subject material for one fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubjectMaterialV1 {
    pub(crate) values: Vec<FactFieldValue>,
}

impl FactSubjectMaterialV1 {
    /// Stable subject material version string.
    pub const VERSION: &'static str = "mfm.fact-subject-material.v1";

    /// Creates subject material and sorts values by field id.
    pub fn new(values: Vec<FactFieldValue>) -> Result<Self> {
        let mut material = Self { values };
        material.sort_and_validate()?;
        Ok(material)
    }

    /// Returns subject values sorted by field id.
    pub fn values(&self) -> &[FactFieldValue] {
        &self.values
    }

    fn sort_and_validate(&mut self) -> Result<()> {
        self.values
            .sort_by(|left, right| left.field_id().cmp(right.field_id()));
        let mut seen = BTreeSet::new();
        for value in &self.values {
            if !seen.insert(value.field_id().clone()) {
                return Err(FactError::descriptor(format!(
                    "duplicate subject material field {}",
                    value.field_id()
                )));
            }
        }
        if self.values.is_empty() {
            return Err(FactError::descriptor(
                "subject material must contain at least one value",
            ));
        }
        Ok(())
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
