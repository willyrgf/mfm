use mfm_canonical::{CanonicalValue, ValidatedCanonicalValueV2};
use mfm_ids::{ContentRef, SchemaId};

use crate::codec;
use crate::{FactError, Result};

const CANONICAL_VALUE_CONTRACT: &str = "mfm.primitive-canonical_value.v1";

/// One bounded, float-free scalar usable as exact fact subject material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactScalar {
    validated: ValidatedCanonicalValueV2,
}

impl FactScalar {
    /// Creates a checked string scalar.
    pub fn string(value: impl Into<String>) -> Result<Self> {
        Self::from_canonical_value(CanonicalValue::String(value.into()))
    }

    /// Creates a checked boolean scalar.
    pub fn boolean(value: bool) -> Result<Self> {
        Self::from_canonical_value(CanonicalValue::Bool(value))
    }

    /// Creates a checked signed-integer scalar.
    pub fn signed(value: i64) -> Result<Self> {
        Self::from_canonical_value(CanonicalValue::Signed(value))
    }

    /// Creates a checked unsigned-integer scalar.
    pub fn unsigned(value: u64) -> Result<Self> {
        Self::from_canonical_value(CanonicalValue::Unsigned(value))
    }

    /// Validates one canonical scalar through the frozen annex.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        if !matches!(
            value,
            CanonicalValue::Bool(_)
                | CanonicalValue::String(_)
                | CanonicalValue::Signed(_)
                | CanonicalValue::Unsigned(_)
        ) {
            return Err(FactError::Descriptor(
                "fact scalar must be a boolean, string, or integer",
            ));
        }
        Ok(Self {
            validated: codec::encode(CANONICAL_VALUE_CONTRACT, &value)?,
        })
    }

    /// Returns the exact float-free canonical JSON bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Reconstructs the checked canonical scalar.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated
            .canonical_value()
            .map_err(FactError::Recoverability)
    }
}

/// Exact canonical subject material used by a fact descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSubject {
    validated: ValidatedCanonicalValueV2,
}

impl FactSubject {
    /// Strictly decodes exact canonical subject bytes through the frozen annex.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            validated: codec::strict_decode(CANONICAL_VALUE_CONTRACT, bytes)?,
        })
    }

    /// Validates and canonicalizes typed subject material through the frozen annex.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        Ok(Self {
            validated: codec::encode(CANONICAL_VALUE_CONTRACT, &value)?,
        })
    }

    /// Creates subject material from one checked scalar.
    pub fn from_scalar(value: &FactScalar) -> Result<Self> {
        Self::from_canonical_value(value.canonical_value()?)
    }

    /// Returns exact canonical JSON bytes.
    pub fn canonical_json(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Returns the annex-derived primitive canonical-value schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.validated.schema_id()
    }

    /// Returns the raw-byte content identity of this subject material.
    pub fn content_ref(&self) -> Result<ContentRef> {
        codec::contract()?
            .content_ref(&self.validated)
            .map_err(Into::into)
    }

    /// Reconstructs the checked canonical subject.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated
            .canonical_value()
            .map_err(FactError::Recoverability)
    }
}

/// Exact state-authored predicate for the reserved fact-selection capability.
///
/// Baseline matching is exact canonical equality with the producer's subject
/// material. Adaptive or range selection is represented by another typed read
/// state, not a mutable or store-injected query language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFactPredicate {
    validated: ValidatedCanonicalValueV2,
}

impl CanonicalFactPredicate {
    /// Strictly decodes exact canonical predicate bytes through the frozen annex.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            validated: codec::strict_decode(CANONICAL_VALUE_CONTRACT, bytes)?,
        })
    }

    /// Validates and canonicalizes a typed predicate through the frozen annex.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        Ok(Self {
            validated: codec::encode(CANONICAL_VALUE_CONTRACT, &value)?,
        })
    }

    /// Builds an exact predicate from canonical fact subject material.
    pub fn exact_subject(subject: &FactSubject) -> Result<Self> {
        Self::from_canonical_value(subject.canonical_value()?)
    }

    /// Returns exact canonical JSON bytes.
    pub fn canonical_json(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Returns the annex-derived primitive canonical-value schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.validated.schema_id()
    }

    /// Reconstructs the checked canonical predicate.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated
            .canonical_value()
            .map_err(FactError::Recoverability)
    }

    /// Returns whether exact canonical subject material satisfies this predicate.
    pub fn matches(&self, subject: &FactSubject) -> bool {
        self.validated.as_bytes() == subject.validated.as_bytes()
    }
}
