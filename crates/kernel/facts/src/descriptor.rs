use std::fmt;
use std::str::FromStr;

use mfm_ids::{ContentRef, SchemaId};

use crate::{FactError, Result};

/// Stable domain-owned fact kind.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FactKind(String);

impl FactKind {
    /// Creates a checked lower-case dotted fact kind.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        if value.is_empty() || value.len() > 256 {
            return Err(FactError::Descriptor(
                "fact kind must contain 1 through 256 bytes",
            ));
        }
        if value.split('.').any(|segment| {
            segment.is_empty()
                || !segment
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_lowercase)
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }) {
            return Err(FactError::Descriptor(
                "fact kind must be a lower-case dotted identifier",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the stable fact kind.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FactKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for FactKind {
    type Err = FactError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// Pure reviewed descriptor identity used while authoring and evaluating facts.
///
/// The descriptor bytes themselves are retained objects selected by
/// `descriptor_ref`; this projection neither serializes those bytes nor grants
/// object authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactDescriptor {
    kind: FactKind,
    descriptor_ref: ContentRef,
    subject_schema_id: SchemaId,
    response_schema_id: SchemaId,
}

impl FactDescriptor {
    /// Constructs an exact descriptor projection.
    pub fn new(
        kind: FactKind,
        descriptor_ref: ContentRef,
        subject_schema_id: SchemaId,
        response_schema_id: SchemaId,
    ) -> Self {
        Self {
            kind,
            descriptor_ref,
            subject_schema_id,
            response_schema_id,
        }
    }

    /// Returns the domain-owned fact kind.
    pub const fn kind(&self) -> &FactKind {
        &self.kind
    }

    /// Returns the exact retained descriptor identity.
    pub const fn descriptor_ref(&self) -> &ContentRef {
        &self.descriptor_ref
    }

    /// Returns the exact typed subject schema.
    pub const fn subject_schema_id(&self) -> &SchemaId {
        &self.subject_schema_id
    }

    /// Returns the exact typed response schema.
    pub const fn response_schema_id(&self) -> &SchemaId {
        &self.response_schema_id
    }
}
