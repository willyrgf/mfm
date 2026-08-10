//! Owner-local persisted-contract vocabulary.
//!
//! This module owns the closed grammar/profile vocabulary that descriptors name
//! and the two traits every retained owner type implements. It deliberately
//! contains no registry: a descriptor names a grammar identifier, and the
//! checked Rust owner enforces it.

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{CanonicalJsonLinesBounds, Result, SchemaIdentity, SchemaKind, ValueError};

/// Maximum byte length of one registered media type.
pub const MAX_MEDIA_TYPE_BYTES: usize = 127;

/// Closed set of string grammars a persisted shape may name.
///
/// The identifier is part of the hashed schema identity. The regular expression
/// itself never enters the identity; the checked Rust owner listed for each
/// variant is the one implementation of the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StringGrammar {
    /// Free Unicode scalar text within the declared byte bounds.
    UnicodeScalarText,
    /// `content:sha256-v1:<64 lowercase hex>` owned by `ContentDigest`.
    ContentDigest,
    /// `sha256-jcs-v1:<64 lowercase hex>` owned by `SemanticDigest`.
    SemanticDigest,
    /// `run:sha256-jcs-v1:<64 lowercase hex>` owned by `RunId`.
    RunId,
    /// `occurrence:sha256-jcs-v1:<64 lowercase hex>` owned by `OccurrenceId`.
    OccurrenceId,
    /// `semantic-call:sha256-jcs-v1:<64 lowercase hex>` owned by `SemanticCallId`.
    SemanticCallId,
    /// `fragment-boundary:sha256-jcs-v1:<64 lowercase hex>` owned by `FragmentBoundaryId`.
    FragmentBoundaryId,
    /// `failure-plan:sha256-jcs-v1:<64 lowercase hex>` owned by `FailurePlanId`.
    FailurePlanId,
    /// `access-attempt:sha256-jcs-v1:<64 lowercase hex>` owned by `AccessAttemptId`.
    AccessAttemptId,
    /// `artifact:sha256-jcs-v1:<64 lowercase hex>` owned by `ArtifactId`.
    ArtifactId,
    /// `schema:<name>:<version>:sha256-jcs-v1:<64 lowercase hex>` owned by `SchemaId`.
    SchemaId,
    /// Versioned semantic identity owned by `SemanticTypeId`.
    SemanticTypeId,
    /// Versioned entry-point identity.
    EntryPointId,
    /// `mfm.<domain>/<name>@<positive canonical u64>` owned by `StableId`.
    StableId,
    /// `mfm.store_scope.v1:<32 lowercase hex>` owned by `StoreScopeId`.
    StoreScopeId,
    /// `mfm.tenant_scope.v1:<32 lowercase hex>` owned by `TenantScopeId`.
    TenantScopeId,
    /// RFC 4122 version 4 UUID.
    UuidV4,
    /// `0|[1-9][0-9]{0,19}` canonical unsigned text.
    CanonicalUnsignedText,
    /// `[a-z0-9][a-z0-9._/-]*` lowercase path token.
    LowerPathToken,
    /// Lowercase registered media type without parameters, owned by [`MediaType`].
    MediaType,
}

impl StringGrammar {
    /// Returns the stable identifier hashed into a schema identity.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnicodeScalarText => "unicode_scalar_text",
            Self::ContentDigest => "content_digest",
            Self::SemanticDigest => "semantic_digest",
            Self::RunId => "run_id",
            Self::OccurrenceId => "occurrence_id",
            Self::SemanticCallId => "semantic_call_id",
            Self::FragmentBoundaryId => "fragment_boundary_id",
            Self::FailurePlanId => "failure_plan_id",
            Self::AccessAttemptId => "access_attempt_id",
            Self::ArtifactId => "artifact_id",
            Self::SchemaId => "schema_id",
            Self::SemanticTypeId => "semantic_type_id",
            Self::EntryPointId => "entry_point_id",
            Self::StableId => "stable_id",
            Self::StoreScopeId => "store_scope_id",
            Self::TenantScopeId => "tenant_scope_id",
            Self::UuidV4 => "uuid_v4",
            Self::CanonicalUnsignedText => "canonical_unsigned_text",
            Self::LowerPathToken => "lower_path_token",
            Self::MediaType => "media_type",
        }
    }
}

/// Closed number profile of a bounded canonical-JSON terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalJsonProfile {
    /// Framework surfaces that deliberately admit signed and unsigned integers.
    GeneralFloatFree,
    /// Fact surfaces restricted to unsigned integers.
    UnsignedNative,
}

impl CanonicalJsonProfile {
    /// Returns the stable identifier hashed into a schema identity.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GeneralFloatFree => "general_float_free",
            Self::UnsignedNative => "unsigned_native",
        }
    }
}

/// Declared ordering of a bounded sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SequenceOrdering {
    /// Authored order is retained exactly.
    Preserved,
    /// Elements are sorted by RFC 8785 UTF-16 key order.
    Utf16Key,
    /// Elements are sorted ascending by their canonical bytes.
    CanonicalAscending,
}

impl SequenceOrdering {
    /// Returns the stable identifier hashed into a schema identity.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preserved => "preserved",
            Self::Utf16Key => "utf16_key",
            Self::CanonicalAscending => "canonical_ascending",
        }
    }
}

/// One scalar literal admitted by a persisted shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiteralValue {
    /// JSON `null`.
    Null,
    /// Exact boolean.
    Bool(bool),
    /// Exact unsigned integer.
    Unsigned(u64),
    /// Exact signed integer.
    Signed(i64),
    /// Exact string.
    String(String),
}

/// Lowercase registered media type without parameters.
///
/// This is the one owner of the media-type grammar. Retained contracts and
/// persisted encodings reuse it instead of carrying an ad hoc string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaType(String);

impl MediaType {
    /// Parses one lowercase registered media type without parameters.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_MEDIA_TYPE_BYTES {
            return Err(ValueError::Descriptor(
                "media type is outside its byte bounds".to_owned(),
            ));
        }
        let mut parts = value.split('/');
        let (Some(kind), Some(subtype), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(ValueError::Descriptor(
                "media type must be exactly type/subtype".to_owned(),
            ));
        };
        if !is_media_token(kind) || !is_media_token(subtype) {
            return Err(ValueError::Descriptor(
                "media type must use lowercase registered tokens without parameters".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the exact lowercase media type.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for MediaType {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for MediaType {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn is_media_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MEDIA_TYPE_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'+')
        })
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// One retained owner type with a shape-derived persisted identity.
///
/// Implementors own their exact fields, literals, bounds, and grammar. There is
/// no registry: the identity is derived from the declared shape, so changing any
/// hashed element changes the derived [`SchemaId`].
pub trait PersistedSchema: Sized {
    /// Returns the hash-defining identity of this persisted contract.
    fn schema_identity() -> Result<SchemaIdentity>;

    /// Returns the exact canonical-JSON shape owned by this contract.
    ///
    /// Derived owners override this operation so an enclosing owner clones the
    /// cached shape once instead of cloning and then discarding a complete
    /// nested identity.
    #[doc(hidden)]
    fn schema_shape() -> Result<crate::SchemaShape> {
        Self::schema_identity()?.into_canonical_json_shape()
    }

    /// Validates canonical bytes against this owner's exact shape.
    #[doc(hidden)]
    fn validate_canonical_bytes(bytes: &[u8]) -> Result<()> {
        Self::schema_identity()?.validate_canonical_value(bytes)
    }

    /// Validates every structural cross-field invariant this owner enforces.
    fn validate(&self) -> Result<()>;

    /// Derives the persisted schema identity.
    fn schema_id() -> Result<SchemaId> {
        Self::schema_identity()?.schema_id()
    }
}

/// A persisted contract whose one encoding is canonical JSON.
///
/// This is a marker over the general contract, not a second identity mechanism.
pub trait CanonicalJsonPersistedSchema: PersistedSchema + Serialize + DeserializeOwned {
    /// Encodes this value as its exact canonical JSON bytes.
    fn encode_canonical(&self) -> Result<PlainCanonicalJsonBytes> {
        self.validate()?;
        let json = serde_json::to_string(self).map_err(|_| ValueError::SchemaShapeMismatch)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|_| ValueError::SchemaShapeMismatch)?;
        Self::validate_canonical_bytes(canonical.as_bytes())?;
        Ok(canonical)
    }

    /// Strictly decodes exact canonical JSON bytes under this owner's shape.
    fn decode_canonical(bytes: &[u8]) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ValueError::SchemaShapeMismatch)?;
        Self::validate_canonical_bytes(canonical.as_bytes())?;
        let value: Self = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| ValueError::SchemaShapeMismatch)?;
        value.validate()?;
        if value.encode_canonical()?.as_bytes() != bytes {
            return Err(ValueError::SchemaShapeMismatch);
        }
        Ok(value)
    }

    /// Derives this owner's exact schema-and-canonical-byte content reference.
    fn content_ref(&self) -> Result<ContentRef> {
        let canonical = self.encode_canonical()?;
        ContentRef::new(
            Self::schema_id()?,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(canonical.as_bytes()),
            ),
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
    }
}

impl<T> CanonicalJsonPersistedSchema for T where T: PersistedSchema + Serialize + DeserializeOwned {}

/// A canonical persisted owner that fixes the one object kind under which its
/// bytes may enter content-addressed object storage.
pub trait PersistedObjectPayload: CanonicalJsonPersistedSchema {
    /// Returns the one checked object kind owned by this payload.
    fn object_type() -> Result<StableId>;
}

/// A concrete persisted record language that owns one bounded canonical
/// JSON-lines stream identity.
///
/// The record owner's ordinary derived shape is embedded verbatim, so the
/// stream cannot describe an erased payload or a different record union.
pub trait CanonicalJsonLinesPersistedSchema: PersistedSchema {
    /// Complete-stream schema name.
    const STREAM_SCHEMA_NAME: &'static str;
    /// Complete-stream schema version.
    const STREAM_SCHEMA_VERSION: &'static str;
    /// Minimum number of records.
    const MINIMUM_RECORDS: u32;
    /// Maximum number of records.
    const MAXIMUM_RECORDS: u32;
    /// Maximum bytes in one record including its terminating LF.
    const MAXIMUM_FRAMED_RECORD_BYTES: u32;
    /// Maximum bytes in the complete stream.
    const MAXIMUM_STREAM_BYTES: u64;

    /// Derives the complete-stream identity from this exact record language.
    fn json_lines_schema_identity() -> Result<SchemaIdentity> {
        SchemaIdentity::new_canonical_json_lines(
            SchemaKind::PersistedContract,
            Self::STREAM_SCHEMA_NAME,
            mfm_ids::SchemaVersion::new(Self::STREAM_SCHEMA_VERSION)
                .map_err(|error| ValueError::Identity(error.to_string()))?,
            Self::schema_identity()?.canonical_json_shape()?.clone(),
            CanonicalJsonLinesBounds {
                minimum_records: Self::MINIMUM_RECORDS,
                maximum_records: Self::MAXIMUM_RECORDS,
                maximum_framed_record_bytes: Self::MAXIMUM_FRAMED_RECORD_BYTES,
                maximum_stream_bytes: Self::MAXIMUM_STREAM_BYTES,
            },
        )
    }

    /// Derives this record language's complete-stream schema id.
    fn json_lines_schema_id() -> Result<SchemaId> {
        Self::json_lines_schema_identity()?.schema_id()
    }

    /// Validates exact complete-stream bytes against the derived record shape.
    fn validate_json_lines(bytes: &[u8]) -> Result<()> {
        Self::json_lines_schema_identity()?.validate_canonical_json_lines(bytes)
    }
}

/// Validates the serialized shape generated for a derive-owned persisted type.
///
/// This is public only because proc-macro expansions execute in downstream
/// crates. Persisted owners should call their trait operations instead.
#[doc(hidden)]
pub fn validate_derived_persisted_owner<T: Serialize>(
    value: &T,
    identity: &SchemaIdentity,
) -> Result<()> {
    let json = serde_json::to_string(value).map_err(|_| ValueError::SchemaShapeMismatch)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| ValueError::SchemaShapeMismatch)?;
    identity.validate_canonical_value(canonical.as_bytes())
}

/// Validates a derived owner against an already checked cached identity.
///
/// This is public only because proc-macro expansions execute downstream.
#[doc(hidden)]
pub fn validate_derived_persisted_owner_prevalidated<T: Serialize>(
    value: &T,
    identity: &SchemaIdentity,
) -> Result<()> {
    let json = serde_json::to_string(value).map_err(|_| ValueError::SchemaShapeMismatch)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| ValueError::SchemaShapeMismatch)?;
    identity.validate_canonical_value_for_prevalidated_owner(canonical.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_type_accepts_registered_lowercase_and_rejects_parameters() {
        assert_eq!(
            MediaType::new("application/json")
                .expect("media type")
                .as_str(),
            "application/json"
        );
        assert!(MediaType::new("application/vnd.mfm+json").is_ok());
        assert!(MediaType::new("Application/JSON").is_err());
        assert!(MediaType::new("application/json; charset=utf-8").is_err());
        assert!(MediaType::new("application").is_err());
        assert!(MediaType::new("application/json/extra").is_err());
        assert!(MediaType::new("").is_err());
        assert!(MediaType::new("/json").is_err());
    }
}
