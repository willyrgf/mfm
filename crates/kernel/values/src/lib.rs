#![warn(missing_docs)]
//! Typed value and descriptor contracts for the MFM typed kernel.
//!
//! This crate owns the derive-backed typed-value and persisted-schema descriptor
//! contracts. Domain crates should get implementations from derives; the
//! hand-written implementations here are framework-owned generic constructors.
//!
//! ```
//! use mfm_values::{DecimalScale, FieldDescriptor, SchemaShape};
//!
//! let shape = SchemaShape::named_struct(vec![FieldDescriptor::required(
//!     "amount",
//!     SchemaShape::DecimalString {
//!         scale: DecimalScale::Variable,
//!     },
//! )])?;
//!
//! assert!(matches!(shape, SchemaShape::Struct { .. }));
//! # Ok::<(), mfm_values::ValueError>(())
//! ```

use std::collections::BTreeMap;

use mfm_canonical::{
    CanonicalBytes, CanonicalJsonBytes, DecimalString, PlainCanonicalJsonBytes,
    MAX_CANONICAL_JSON_DEPTH,
};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, NameToken, SchemaId, SchemaVersion, SemanticTypeId,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use mfm_canonical::limits::{
    MAX_ARRAY_ITEMS, MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES, MAX_OBJECT_ENTRIES, MAX_STRING_UTF8_BYTES,
};

mod persisted;
pub use self::persisted::{
    validate_derived_persisted_owner, CanonicalJsonLinesPersistedSchema,
    CanonicalJsonPersistedSchema, CanonicalJsonProfile, LiteralValue, MediaType,
    PersistedObjectPayload, PersistedSchema, SequenceOrdering, StringGrammar, MAX_MEDIA_TYPE_BYTES,
};

// Keep this list intentionally small and high-signal to avoid false positives on public
// descriptive fields while still blocking common secret-bearing persisted surfaces.
const SECRET_MARKERS: &[&str] = &[
    "password",
    "passphrase",
    "mnemonic",
    "private_key",
    "privatekey",
    "seed phrase",
    "seed_phrase",
    "seedphrase",
    "api_key",
    "apikey",
    "x-api-key",
    "x_api_key",
    "access_key",
    "accesskey",
    "secret_key",
    "secretkey",
    "secret",
    "aws_access_key_id",
    "aws_secret_access_key",
    "access_token",
    "refresh_token",
    "id_token",
    "bearer ",
];
const MAX_SCHEMA_IDENTITY_BYTES: usize = 65_536;
/// Maximum canonical bytes of one value retained in a run frame.
pub const MAX_RUN_OBJECT_CANONICAL_BYTES: usize = 8_388_608;
/// Maximum recursive depth admitted by current schema identities and values.
///
/// Schema identities add three object levels around their shape — the identity
/// object, its persisted encoding, and the encoding's shape field — so reserve
/// those levels from the canonical JSON budget. This keeps the advertised schema
/// limit round-trippable through strict canonical decoding.
pub const MAX_SCHEMA_DEPTH: usize = MAX_CANONICAL_JSON_DEPTH - 3;

/// Result type for descriptor and value-contract operations.
pub type Result<T> = std::result::Result<T, ValueError>;

/// Error returned by value descriptor helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// Descriptor construction failed.
    #[error("descriptor error: {0}")]
    Descriptor(String),
    /// Identity parsing failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// A schema identity was not exact canonical descriptor material.
    #[error("invalid schema identity")]
    InvalidSchemaIdentity,
    /// Canonical value bytes did not match the complete closed schema shape.
    #[error("value does not match schema shape")]
    SchemaShapeMismatch,
    /// Canonical value bytes exceeded the retained object ceiling before shape qualification.
    #[error("value capacity exceeded")]
    Capacity,
    /// Artifact reference identity does not match the expected value type.
    #[error("artifact reference {field} mismatch: expected {expected}, got {actual}")]
    ArtifactTypeMismatch {
        /// Field that mismatched.
        field: &'static str,
        /// Expected typed identity.
        expected: String,
        /// Actual typed identity.
        actual: String,
    },
}

/// Returns `true` when `input` matches MFM's high-signal secret-marker policy.
///
/// This is a conservative persisted-surface guard. It is not intended to prove that arbitrary
/// text is safe; it blocks known secret field markers and mnemonic-shaped phrases before values
/// become canonical run objects or public results.
pub fn string_contains_secret_marker(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    if SECRET_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return true;
    }

    looks_like_mnemonic_phrase(input)
}

/// Returns the first string-map key whose key or value matches the secret-marker policy.
pub fn string_map_secret_marker_key(map: &BTreeMap<String, String>) -> Option<&str> {
    map.iter()
        .find(|(key, value)| {
            string_contains_secret_marker(key) || string_contains_secret_marker(value)
        })
        .map(|(key, _)| key.as_str())
}

fn looks_like_mnemonic_phrase(input: &str) -> bool {
    let mut words = input.split_whitespace().peekable();
    if words.peek().is_none() {
        return false;
    }

    let mut count = 0usize;
    for word in words {
        count += 1;
        if !word.chars().all(|ch| ch.is_ascii_lowercase()) {
            return false;
        }
    }

    (12..=24).contains(&count)
}

/// Values that may cross typed state boundaries.
pub trait MfmValue: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Returns the schema descriptor for this value type.
    fn schema_descriptor() -> Result<SchemaDescriptor>;

    /// Returns the stable semantic identity for this value kind.
    fn semantic_id() -> Result<SemanticTypeId>;
}

/// Canonicalizes one typed value and derives its exact schema/content identity.
pub fn canonicalize_mfm_value<T: MfmValue>(
    value: &T,
) -> std::result::Result<(PlainCanonicalJsonBytes, ContentRef), ValueError> {
    let descriptor = T::schema_descriptor()?;
    let semantic_id = T::semantic_id()?;
    if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic_id) {
        return Err(ValueError::Descriptor(
            "value descriptor does not match its Rust owner".to_owned(),
        ));
    }
    let schema_id = descriptor.schema_id()?;

    let json = serde_json::to_string(value).map_err(|_| ValueError::SchemaShapeMismatch)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|_| ValueError::SchemaShapeMismatch)?;
    if canonical.as_bytes().len() > MAX_RUN_OBJECT_CANONICAL_BYTES {
        return Err(ValueError::Capacity);
    }
    descriptor
        .identity()
        .validate_canonical_value(canonical.as_bytes())?;
    let content_ref = ContentRef::new(
        schema_id,
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, canonical.digest_bytes()),
    )
    .map_err(|error| ValueError::Identity(error.to_string()))?;
    Ok((canonical, content_ref))
}

/// Schema descriptor with hash-defining identity fields split from audit-only
/// provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDescriptor {
    /// Descriptor grammar version.
    pub descriptor_version: u32,
    /// Canonicalization algorithm used for descriptor identity hashing.
    pub canonicalization: DigestAlgorithm,
    /// Hash-defining schema identity.
    pub identity: SchemaIdentity,
    /// Audit-only descriptor provenance.
    pub audit: SchemaAudit,
}

impl SchemaDescriptor {
    /// Creates a v1 schema descriptor.
    pub fn new(identity: SchemaIdentity, audit: SchemaAudit) -> Result<Self> {
        identity.validate()?;
        Ok(Self {
            descriptor_version: 1,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            identity,
            audit,
        })
    }

    /// Returns canonical JSON bytes for the hash-defining identity only.
    pub fn identity_canonical_json(&self) -> Result<CanonicalJsonBytes> {
        self.identity.canonical_json()
    }

    /// Returns the complete hash-defining schema identity.
    pub const fn identity(&self) -> &SchemaIdentity {
        &self.identity
    }

    /// Derives the schema id from canonical identity bytes.
    pub fn schema_id(&self) -> Result<SchemaId> {
        self.identity.schema_id()
    }
}

/// Hash-defining schema descriptor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaIdentity {
    /// Schema surface kind.
    pub schema_kind: SchemaKind,
    /// Semantic type identity for value schemas.
    pub semantic_type_id: Option<SemanticTypeId>,
    /// Stable schema name.
    pub schema_name: NameToken,
    /// Manually assigned schema version.
    pub schema_version: SchemaVersion,
    /// Closed persisted encoding, including its serialized shape.
    pub encoding: PersistedEncoding,
    /// Canonicalization algorithm.
    pub canonicalization: DigestAlgorithm,
    /// Versioning policy.
    pub versioning: SchemaVersioningPolicy,
    /// Persisted-surface no-secret/no-float policy.
    pub persisted_surface: PersistedSurfacePolicy,
}

/// The four hashed cardinality and byte bounds of one JSON-lines stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalJsonLinesBounds {
    /// Inclusive minimum record count.
    pub minimum_records: u32,
    /// Inclusive maximum record count.
    pub maximum_records: u32,
    /// Inclusive maximum framed bytes of one record, including its LF delimiter.
    pub maximum_framed_record_bytes: u32,
    /// Inclusive maximum bytes of the complete stream.
    pub maximum_stream_bytes: u64,
}

impl SchemaIdentity {
    /// Creates a schema identity with strict persisted-surface policy.
    pub fn new(
        schema_kind: SchemaKind,
        semantic_type_id: Option<SemanticTypeId>,
        schema_name: impl Into<String>,
        schema_version: SchemaVersion,
        shape: SchemaShape,
    ) -> Result<Self> {
        Self::with_encoding(
            schema_kind,
            semantic_type_id,
            schema_name.into(),
            schema_version,
            PersistedEncoding::CanonicalJson { shape },
        )
    }

    /// Creates a schema identity for the one bounded JSON-lines stream encoding.
    pub fn new_canonical_json_lines(
        schema_kind: SchemaKind,
        schema_name: impl Into<String>,
        schema_version: SchemaVersion,
        record_shape: SchemaShape,
        bounds: CanonicalJsonLinesBounds,
    ) -> Result<Self> {
        Self::with_encoding(
            schema_kind,
            None,
            schema_name.into(),
            schema_version,
            PersistedEncoding::CanonicalJsonLines {
                record_shape,
                minimum_records: bounds.minimum_records,
                maximum_records: bounds.maximum_records,
                maximum_framed_record_bytes: bounds.maximum_framed_record_bytes,
                maximum_stream_bytes: bounds.maximum_stream_bytes,
            },
        )
    }

    fn with_encoding(
        schema_kind: SchemaKind,
        semantic_type_id: Option<SemanticTypeId>,
        raw_schema_name: String,
        schema_version: SchemaVersion,
        encoding: PersistedEncoding,
    ) -> Result<Self> {
        let schema_name = NameToken::new(&raw_schema_name).map_err(|_| {
            ValueError::Descriptor(format!("invalid schema name {raw_schema_name:?}"))
        })?;
        let identity = Self {
            schema_kind,
            semantic_type_id,
            schema_name,
            schema_version,
            encoding,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            versioning: SchemaVersioningPolicy::ManualVersion,
            persisted_surface: PersistedSurfacePolicy::strict(),
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Returns the canonical-JSON shape of this identity.
    ///
    /// A stream encoding has no single JSON object shape and is rejected here;
    /// its record shape belongs to its own codec owner.
    pub fn canonical_json_shape(&self) -> Result<&SchemaShape> {
        match &self.encoding {
            PersistedEncoding::CanonicalJson { shape } => Ok(shape),
            PersistedEncoding::CanonicalJsonLines { .. } => Err(ValueError::SchemaShapeMismatch),
        }
    }

    /// Consumes this identity and returns its canonical-JSON shape.
    ///
    /// This is primarily used while composing the exact shape of a concrete
    /// outer persisted owner, where moving avoids a recursive clone of the
    /// nested identity.
    #[doc(hidden)]
    pub fn into_canonical_json_shape(self) -> Result<SchemaShape> {
        match self.encoding {
            PersistedEncoding::CanonicalJson { shape } => Ok(shape),
            PersistedEncoding::CanonicalJsonLines { .. } => Err(ValueError::SchemaShapeMismatch),
        }
    }

    /// Verifies one complete bounded JSON-lines stream against this identity.
    ///
    /// LF delimiters, the required final LF, the declared record bounds, and
    /// the declared record shape are all enforced here, so a stream owner does
    /// not restate its own framing rules.
    pub fn validate_canonical_json_lines(&self, bytes: &[u8]) -> Result<()> {
        self.validate()?;
        let PersistedEncoding::CanonicalJsonLines {
            record_shape,
            minimum_records,
            maximum_records,
            maximum_framed_record_bytes,
            maximum_stream_bytes,
        } = &self.encoding
        else {
            return Err(ValueError::SchemaShapeMismatch);
        };
        if u64::try_from(bytes.len()).map_err(|_| ValueError::SchemaShapeMismatch)?
            > *maximum_stream_bytes
            || bytes.last() != Some(&b'\n')
        {
            return Err(ValueError::SchemaShapeMismatch);
        }
        let mut records: u32 = 0;
        for record in bytes
            .split(|byte| *byte == b'\n')
            .take_while(|record| !record.is_empty())
        {
            if u32::try_from(record.len().saturating_add(1))
                .map_err(|_| ValueError::SchemaShapeMismatch)?
                > *maximum_framed_record_bytes
            {
                return Err(ValueError::SchemaShapeMismatch);
            }
            let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(record)
                .map_err(|_| ValueError::SchemaShapeMismatch)?;
            let value: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
                .map_err(|_| ValueError::SchemaShapeMismatch)?;
            record_shape
                .validate_json_value(&value, 0)
                .map_err(|_| ValueError::SchemaShapeMismatch)?;
            records = records
                .checked_add(1)
                .ok_or(ValueError::SchemaShapeMismatch)?;
        }
        if records < *minimum_records
            || records > *maximum_records
            || usize::try_from(records).map_err(|_| ValueError::SchemaShapeMismatch)?
                != bytes.iter().filter(|byte| **byte == b'\n').count()
        {
            return Err(ValueError::SchemaShapeMismatch);
        }
        Ok(())
    }

    /// Strictly decodes exact canonical schema-identity bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_SCHEMA_IDENTITY_BYTES {
            return Err(ValueError::InvalidSchemaIdentity);
        }
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        let identity: Self = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        identity
            .validate()
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        if identity.canonical_json()?.as_bytes() != bytes {
            return Err(ValueError::InvalidSchemaIdentity);
        }
        Ok(identity)
    }

    /// Returns exact canonical JSON for this hash-defining identity.
    pub fn canonical_json(&self) -> Result<CanonicalJsonBytes> {
        self.validate()?;
        self.canonical_json_unchecked()
    }

    /// Derives the schema id from this complete identity.
    pub fn schema_id(&self) -> Result<SchemaId> {
        let digest = self.canonical_json()?.digest_bytes();
        SchemaId::new(
            self.schema_name.as_str(),
            self.schema_version.as_str(),
            DigestAlgorithm::Sha256JcsV1,
            digest,
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
    }

    /// Verifies exact canonical value bytes against this identity's complete closed shape.
    pub fn validate_canonical_value(&self, bytes: &[u8]) -> Result<()> {
        self.validate()?;
        self.validate_canonical_value_for_prevalidated_owner(bytes)
    }

    /// Validates value bytes against an identity already checked and retained
    /// by its concrete Rust owner.
    #[doc(hidden)]
    pub fn validate_canonical_value_for_prevalidated_owner(&self, bytes: &[u8]) -> Result<()> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ValueError::SchemaShapeMismatch)?;
        let value: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| ValueError::SchemaShapeMismatch)?;
        self.canonical_json_shape()?
            .validate_json_value(&value, 0)
            .map_err(|_| ValueError::SchemaShapeMismatch)
    }

    fn validate(&self) -> Result<()> {
        if self.canonicalization != DigestAlgorithm::Sha256JcsV1
            || self.versioning != SchemaVersioningPolicy::ManualVersion
            || self.persisted_surface != PersistedSurfacePolicy::strict()
        {
            return Err(ValueError::Descriptor(
                "schema identity uses an unsupported fixed policy".to_owned(),
            ));
        }
        match (self.schema_kind, self.semantic_type_id.as_ref()) {
            (SchemaKind::Value, Some(_)) => {}
            (SchemaKind::Value, None) => Err(ValueError::Descriptor(
                "value schema identities must include a semantic type id".to_owned(),
            ))?,
            (SchemaKind::PersistedContract, None) => {}
            (SchemaKind::PersistedContract, Some(_)) => Err(ValueError::Descriptor(
                "non-value schema identities must not include a semantic type id".to_owned(),
            ))?,
        }
        if self.claims_reserved_never_identity() && !self.is_reserved_never_identity() {
            return Err(ValueError::Descriptor(
                "reserved Never identity does not match its exact schema".to_owned(),
            ));
        }
        if !self.is_reserved_never_identity() {
            self.encoding.validate_descriptor()?;
        }
        if self.canonical_json_unchecked()?.as_bytes().len() > MAX_SCHEMA_IDENTITY_BYTES {
            return Err(ValueError::Descriptor(
                "schema identity exceeds the canonical byte bound".to_owned(),
            ));
        }
        Ok(())
    }

    fn is_reserved_never_identity(&self) -> bool {
        let Some(semantic_type_id) = self.semantic_type_id.as_ref() else {
            return false;
        };
        matches!(
            &self.encoding,
            PersistedEncoding::CanonicalJson {
                shape: SchemaShape::Enum {
                    tagging: EnumTagging::External,
                    variants,
                },
            } if variants.is_empty()
        ) && self.schema_kind == SchemaKind::Value
            && semantic_type_id.as_str()
                == "semantic:mfm.kernel:never:1:sha256-jcs-v1:527c198ca1e6170ddf8966dd86ebd418b62226f14f6ef34d7b7ddd7b91a36835"
            && self.schema_name.as_str() == "mfm.kernel.never"
            && self.schema_version.as_str() == "1"
    }

    fn claims_reserved_never_identity(&self) -> bool {
        self.semantic_type_id.as_ref().is_some_and(|identity| {
            identity.as_str()
                == "semantic:mfm.kernel:never:1:sha256-jcs-v1:527c198ca1e6170ddf8966dd86ebd418b62226f14f6ef34d7b7ddd7b91a36835"
        }) || self.schema_name.as_str() == "mfm.kernel.never"
    }

    fn canonical_json_unchecked(&self) -> Result<CanonicalJsonBytes> {
        let wire = SchemaIdentityWire::from(self);
        let json = serde_json::to_string(&wire).map_err(|_| ValueError::InvalidSchemaIdentity)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        Ok(CanonicalJsonBytes::from_checked_plain(canonical))
    }
}

/// Audit-only descriptor provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaAudit {
    owner_crate: String,
    rust_type_path: String,
    provenance: DescriptorProvenance,
    derive_macro_version: Option<String>,
    source_package: Option<String>,
}

impl SchemaAudit {
    /// Creates framework-owned audit provenance for hand-written descriptors.
    pub(crate) fn framework(
        owner_crate: impl Into<String>,
        rust_type_path: impl Into<String>,
    ) -> Self {
        Self {
            owner_crate: owner_crate.into(),
            rust_type_path: rust_type_path.into(),
            provenance: DescriptorProvenance::FrameworkOwned,
            derive_macro_version: None,
            source_package: None,
        }
    }

    /// Creates derive-generated audit provenance from framework macros.
    ///
    /// This is public so proc-macro expansion can reference it from downstream
    /// crates. It is hidden from normal docs and is paired with source-boundary
    /// checks that reject manual persisted trait impls outside framework
    /// allowlists.
    #[doc(hidden)]
    pub fn __derive_generated(
        owner_crate: impl Into<String>,
        rust_type_path: impl Into<String>,
        derive_macro_version: impl Into<String>,
    ) -> Self {
        Self {
            owner_crate: owner_crate.into(),
            rust_type_path: rust_type_path.into(),
            provenance: DescriptorProvenance::DeriveGenerated,
            derive_macro_version: Some(derive_macro_version.into()),
            source_package: None,
        }
    }

    /// Returns the owner crate.
    pub fn owner_crate(&self) -> &str {
        &self.owner_crate
    }

    /// Returns the Rust type path.
    pub fn rust_type_path(&self) -> &str {
        &self.rust_type_path
    }

    /// Returns descriptor implementation provenance.
    pub const fn provenance(&self) -> DescriptorProvenance {
        self.provenance
    }

    /// Returns the derive macro version, when generated by a derive.
    pub fn derive_macro_version(&self) -> Option<&str> {
        self.derive_macro_version.as_deref()
    }

    /// Returns optional source package/build provenance.
    pub fn source_package(&self) -> Option<&str> {
        self.source_package.as_deref()
    }
}

/// Source of a schema descriptor implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DescriptorProvenance {
    /// Framework-owned hand-written descriptor.
    FrameworkOwned,
    /// Derive-generated descriptor.
    DeriveGenerated,
}

/// Schema surface kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaKind {
    /// Runtime value crossing a state boundary.
    Value,
    /// Retained component contract that is not a runtime value.
    PersistedContract,
}

impl SchemaKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::PersistedContract => "persisted_contract",
        }
    }
}

/// Schema versioning policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaVersioningPolicy {
    /// Breaking shape or semantic changes require a manually assigned new
    /// schema version.
    ManualVersion,
}

/// Persisted-surface policy combining no-secret and no-float requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PersistedSurfacePolicy {
    /// Secret-bearing values are forbidden.
    pub secrets: SecretPolicy,
    /// Floating point values are forbidden.
    pub numbers: NumberPolicy,
}

impl PersistedSurfacePolicy {
    /// Returns the strict v1 persisted-surface policy.
    pub const fn strict() -> Self {
        Self {
            secrets: SecretPolicy::NoSecrets,
            numbers: NumberPolicy::NoFloats,
        }
    }
}

/// Secret policy for persisted surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecretPolicy {
    /// Secret-bearing fields are forbidden.
    NoSecrets,
}

/// Number policy for persisted surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NumberPolicy {
    /// Floating point values are forbidden.
    NoFloats,
}

/// Closed persisted encoding of one retained owner type.
///
/// The encoding is part of the hashed schema identity, so changing a framing
/// rule or a stream bound changes the derived `SchemaId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistedEncoding {
    /// One canonical JSON value.
    CanonicalJson {
        /// Complete serialized shape of that value.
        shape: SchemaShape,
    },
    /// A bounded newline-delimited stream of canonical JSON records.
    ///
    /// LF delimiters and a required final LF are fixed by this encoding; they
    /// are not configurable flags.
    CanonicalJsonLines {
        /// Complete serialized shape of one framed record.
        record_shape: SchemaShape,
        /// Inclusive minimum record count.
        minimum_records: u32,
        /// Inclusive maximum record count.
        maximum_records: u32,
        /// Inclusive maximum framed bytes of one record, excluding its delimiter.
        maximum_framed_record_bytes: u32,
        /// Inclusive maximum bytes of the complete stream.
        maximum_stream_bytes: u64,
    },
}

impl PersistedEncoding {
    fn validate_descriptor(&self) -> Result<()> {
        match self {
            Self::CanonicalJson { shape } => shape.validate_descriptor(0),
            Self::CanonicalJsonLines {
                record_shape,
                minimum_records,
                maximum_records,
                maximum_framed_record_bytes,
                maximum_stream_bytes,
            } => {
                require_descriptor(
                    minimum_records <= maximum_records
                        && *maximum_framed_record_bytes > 0
                        && *maximum_stream_bytes > 0,
                    "canonical JSON-lines bounds are inverted or empty",
                )?;
                record_shape.validate_descriptor(0)
            }
        }
    }
}

/// Canonical serialized schema shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaShape {
    /// Unit shape.
    Unit,
    /// Boolean shape.
    Bool,
    /// UTF-8 string shape.
    String,
    /// Base64url-without-padding bytes shape.
    Bytes,
    /// Signed integer shape.
    SignedInteger {
        /// Bit width.
        bits: u16,
    },
    /// Unsigned integer shape.
    UnsignedInteger {
        /// Bit width.
        bits: u16,
    },
    /// Decimal-string shape.
    DecimalString {
        /// Decimal scale policy.
        scale: DecimalScale,
    },
    /// Optional value shape.
    Option(Box<SchemaShape>),
    /// Vector shape.
    Vec(Box<SchemaShape>),
    /// Non-empty vector shape.
    NonEmptyVec(Box<SchemaShape>),
    /// Tuple shape.
    Tuple(Vec<SchemaShape>),
    /// Named struct shape.
    Struct {
        /// Struct fields in canonical wire-name order.
        fields: Vec<FieldDescriptor>,
    },
    /// Enum shape.
    Enum {
        /// Enum tagging policy.
        tagging: EnumTagging,
        /// Variants in canonical wire-name order.
        variants: Vec<EnumVariantDescriptor>,
    },
    /// `BTreeMap<String, V>` shape.
    BTreeMapString {
        /// Value shape.
        value: Box<SchemaShape>,
    },
    /// Inline serialized shape of another value schema.
    InlineValue {
        /// Inlined value schema id.
        schema_id: SchemaId,
        /// Inlined value semantic type id.
        semantic_type_id: SemanticTypeId,
        /// Complete serialized shape of the inlined value.
        serialized_shape: Box<SchemaShape>,
    },
    /// Framework-owned generic constructor shape.
    Generic {
        /// Generic constructor name.
        constructor: String,
        /// Generic arguments.
        arguments: Vec<GenericArgumentDescriptor>,
        /// Serialized representation of the constructed value.
        serialized_shape: Box<SchemaShape>,
    },
    /// UTF-8 string with inclusive byte bounds and a closed grammar.
    BoundedString {
        /// Inclusive minimum UTF-8 byte length.
        minimum_bytes: u32,
        /// Inclusive maximum UTF-8 byte length.
        maximum_bytes: u32,
        /// Closed grammar identifier enforced by the checked owner.
        grammar: StringGrammar,
    },
    /// Base64url-without-padding bytes with inclusive decoded byte bounds.
    BoundedBytes {
        /// Inclusive minimum decoded byte length.
        minimum_decoded_bytes: u32,
        /// Inclusive maximum decoded byte length.
        maximum_decoded_bytes: u32,
    },
    /// Unsigned integer restricted to an inclusive range.
    UnsignedRange {
        /// Inclusive minimum.
        minimum: u64,
        /// Inclusive maximum.
        maximum: u64,
    },
    /// Signed integer restricted to an inclusive range.
    SignedRange {
        /// Inclusive minimum.
        minimum: i64,
        /// Inclusive maximum.
        maximum: i64,
    },
    /// Exact scalar literal.
    Literal(LiteralValue),
    /// Sequence with element shape, cardinality bounds, ordering, and uniqueness.
    BoundedSequence {
        /// Element shape.
        element: Box<SchemaShape>,
        /// Inclusive minimum cardinality.
        minimum_items: u32,
        /// Inclusive maximum cardinality.
        maximum_items: u32,
        /// Declared element ordering.
        ordering: SequenceOrdering,
        /// Whether elements must be pairwise distinct.
        unique: bool,
    },
    /// String-keyed map with key grammar/bounds, value shape, and entry bounds.
    BoundedStringMap {
        /// Closed key grammar.
        key_grammar: StringGrammar,
        /// Inclusive minimum key byte length.
        key_minimum_bytes: u32,
        /// Inclusive maximum key byte length.
        key_maximum_bytes: u32,
        /// Value shape.
        value: Box<SchemaShape>,
        /// Inclusive minimum entry count.
        minimum_entries: u32,
        /// Inclusive maximum entry count.
        maximum_entries: u32,
    },
    /// Bounded canonical-JSON terminal with an explicit number profile.
    CanonicalJsonTerminal {
        /// Closed number profile.
        profile: CanonicalJsonProfile,
    },
}

impl SchemaShape {
    /// Builds the complete inline shape of one nested [`MfmValue`].
    pub fn inline_value<T: MfmValue>() -> Result<Self> {
        let descriptor = T::schema_descriptor()?;
        let semantic_type_id = T::semantic_id()?;
        if descriptor.identity.semantic_type_id.as_ref() != Some(&semantic_type_id) {
            return Err(ValueError::Descriptor(
                "nested value descriptor semantic identity does not match its value type"
                    .to_owned(),
            ));
        }
        let schema_id = descriptor.schema_id()?;
        Ok(Self::InlineValue {
            schema_id,
            semantic_type_id,
            serialized_shape: Box::new(descriptor.identity.canonical_json_shape()?.clone()),
        })
    }

    /// Builds a bounded string shape for one checked identity grammar.
    ///
    /// Persisted owners describe checked identity fields through this helper so
    /// the grammar is named once and enforced by its checked Rust owner.
    pub fn identity_string(grammar: StringGrammar, maximum_bytes: u32) -> Self {
        Self::BoundedString {
            minimum_bytes: 1,
            maximum_bytes,
            grammar,
        }
    }

    /// Builds the exact serialized shape of one `ContentRef`.
    ///
    /// `ContentRef` is the most common nested persisted field, so its shape is
    /// framework-owned rather than restated by every retained owner.
    pub fn content_ref() -> Result<Self> {
        Self::named_struct(vec![
            FieldDescriptor::required(
                "content_digest",
                Self::identity_string(StringGrammar::ContentDigest, 128),
            ),
            FieldDescriptor::required(
                "schema_id",
                Self::identity_string(StringGrammar::SchemaId, 512),
            ),
        ])
    }

    /// Builds a named struct shape, rejecting duplicate field names.
    pub fn named_struct(mut fields: Vec<FieldDescriptor>) -> Result<Self> {
        reject_duplicate_names(
            fields.iter().map(|field| field.name.as_str()),
            "struct field",
        )?;
        fields.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self::Struct { fields })
    }

    /// Builds an enum shape, rejecting duplicate variant names.
    pub fn external_enum(mut variants: Vec<EnumVariantDescriptor>) -> Result<Self> {
        reject_duplicate_names(
            variants.iter().map(|variant| variant.name.as_str()),
            "enum variant",
        )?;
        variants.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self::Enum {
            tagging: EnumTagging::External,
            variants,
        })
    }

    /// Builds an enum shape with the supplied tagging policy, rejecting
    /// duplicate variant names.
    pub fn tagged_enum(
        tagging: EnumTagging,
        mut variants: Vec<EnumVariantDescriptor>,
    ) -> Result<Self> {
        reject_duplicate_names(
            variants.iter().map(|variant| variant.name.as_str()),
            "enum variant",
        )?;
        variants.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self::Enum { tagging, variants })
    }

    fn validate_descriptor(&self, depth: usize) -> Result<()> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(ValueError::Descriptor(
                "schema shape exceeds the recursive depth bound".to_owned(),
            ));
        }
        match self {
            Self::Unit | Self::Bool | Self::String | Self::Bytes => Ok(()),
            Self::SignedInteger { bits } | Self::UnsignedInteger { bits } => {
                if matches!(bits, 8 | 16 | 32 | 64) {
                    Ok(())
                } else {
                    Err(ValueError::Descriptor(
                        "integer shape uses an unsupported bit width".to_owned(),
                    ))
                }
            }
            Self::DecimalString { .. } => Ok(()),
            Self::Option(value)
            | Self::Vec(value)
            | Self::NonEmptyVec(value)
            | Self::BTreeMapString { value } => value.validate_descriptor(depth + 1),
            Self::Tuple(values) => {
                for value in values {
                    value.validate_descriptor(depth + 1)?;
                }
                Ok(())
            }
            Self::Struct { fields } => {
                if !fields.windows(2).all(|pair| pair[0].name < pair[1].name)
                    || fields.iter().any(|field| field.name.is_empty())
                {
                    return Err(ValueError::Descriptor(
                        "struct fields must be nonempty, unique, and strictly ordered".to_owned(),
                    ));
                }
                for field in fields {
                    field.shape.validate_descriptor(depth + 1)?;
                }
                Ok(())
            }
            Self::Enum { tagging, variants } => {
                tagging.validate()?;
                if variants.is_empty()
                    || !variants.windows(2).all(|pair| pair[0].name < pair[1].name)
                    || variants.iter().any(|variant| variant.name.is_empty())
                {
                    return Err(ValueError::Descriptor(
                        "enum variants must be nonempty, unique, and strictly ordered".to_owned(),
                    ));
                }
                for variant in variants {
                    match tagging {
                        EnumTagging::Internal { tag } => match &variant.shape {
                            Self::Unit => {}
                            Self::Struct { fields } => {
                                if fields.iter().any(|field| &field.name == tag) {
                                    return Err(ValueError::Descriptor(
                                        "internal enum tag collides with a variant field"
                                            .to_owned(),
                                    ));
                                }
                            }
                            _ => {
                                return Err(ValueError::Descriptor(
                                    "internally tagged variants must be unit or struct shaped"
                                        .to_owned(),
                                ));
                            }
                        },
                        EnumTagging::External | EnumTagging::Adjacent { .. } => {}
                    }
                    variant.shape.validate_descriptor(depth + 1)?;
                }
                Ok(())
            }
            Self::InlineValue {
                serialized_shape, ..
            }
            | Self::Generic {
                serialized_shape, ..
            } => {
                if let Self::Generic {
                    constructor,
                    arguments,
                    ..
                } = self
                {
                    if !valid_generic_constructor(constructor) || arguments.is_empty() {
                        return Err(ValueError::Descriptor(
                            "generic schema metadata is malformed".to_owned(),
                        ));
                    }
                }
                serialized_shape.validate_descriptor(depth + 1)
            }
            Self::BoundedString {
                minimum_bytes,
                maximum_bytes,
                ..
            } => require_descriptor(
                minimum_bytes <= maximum_bytes,
                "bounded string bounds are inverted",
            ),
            Self::BoundedBytes {
                minimum_decoded_bytes,
                maximum_decoded_bytes,
            } => require_descriptor(
                minimum_decoded_bytes <= maximum_decoded_bytes,
                "bounded byte bounds are inverted",
            ),
            Self::UnsignedRange { minimum, maximum } => {
                require_descriptor(minimum <= maximum, "unsigned range is inverted")
            }
            Self::SignedRange { minimum, maximum } => {
                require_descriptor(minimum <= maximum, "signed range is inverted")
            }
            Self::Literal(_) | Self::CanonicalJsonTerminal { .. } => Ok(()),
            Self::BoundedSequence {
                element,
                minimum_items,
                maximum_items,
                ..
            } => {
                require_descriptor(
                    minimum_items <= maximum_items,
                    "bounded sequence cardinality is inverted",
                )?;
                element.validate_descriptor(depth + 1)
            }
            Self::BoundedStringMap {
                key_minimum_bytes,
                key_maximum_bytes,
                value,
                minimum_entries,
                maximum_entries,
                ..
            } => {
                require_descriptor(
                    key_minimum_bytes <= key_maximum_bytes && minimum_entries <= maximum_entries,
                    "bounded string-map bounds are inverted",
                )?;
                value.validate_descriptor(depth + 1)
            }
        }
    }

    fn validate_json_value(&self, value: &serde_json::Value, depth: usize) -> Result<()> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(ValueError::SchemaShapeMismatch);
        }
        match self {
            Self::Unit => require(value.is_null()),
            Self::Bool => require(value.is_boolean()),
            Self::String => require(
                value
                    .as_str()
                    .is_some_and(|value| !string_contains_secret_marker(value)),
            ),
            Self::Bytes => require(value.as_str().is_some_and(|value| {
                CanonicalBytes::from_base64url_no_pad(value.to_owned()).is_ok()
            })),
            Self::SignedInteger { bits } => {
                let Some(value) = value.as_i64() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                require(signed_integer_in_range(value, *bits))
            }
            Self::UnsignedInteger { bits } => {
                let Some(value) = value.as_u64() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                require(unsigned_integer_in_range(value, *bits))
            }
            Self::DecimalString { scale } => {
                let Some(value) = value.as_str() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                let valid = match scale {
                    DecimalScale::Variable => DecimalString::new_variable(value.to_owned()).is_ok(),
                    DecimalScale::Fixed(scale) => {
                        DecimalString::new_fixed(value.to_owned(), usize::from(*scale)).is_ok()
                    }
                };
                require(valid)
            }
            Self::Option(element) => {
                if value.is_null() {
                    Ok(())
                } else {
                    element.validate_json_value(value, depth + 1)
                }
            }
            Self::Vec(element) | Self::NonEmptyVec(element) => {
                let Some(values) = value.as_array() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                if matches!(self, Self::NonEmptyVec(_)) && values.is_empty() {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                for value in values {
                    element.validate_json_value(value, depth + 1)?;
                }
                Ok(())
            }
            Self::Tuple(elements) => {
                let Some(values) = value.as_array() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                if values.len() != elements.len() {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                for (element, value) in elements.iter().zip(values) {
                    element.validate_json_value(value, depth + 1)?;
                }
                Ok(())
            }
            Self::Struct { fields } => {
                let Some(object) = value.as_object() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                validate_struct_value(fields, object, None, depth + 1)
            }
            Self::Enum { tagging, variants } => {
                validate_enum_value(tagging, variants, value, depth + 1)
            }
            Self::BTreeMapString { value: element } => {
                let Some(object) = value.as_object() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                for (key, value) in object {
                    if string_contains_secret_marker(key) {
                        return Err(ValueError::SchemaShapeMismatch);
                    }
                    element.validate_json_value(value, depth + 1)?;
                }
                Ok(())
            }
            Self::InlineValue {
                serialized_shape, ..
            }
            | Self::Generic {
                serialized_shape, ..
            } => serialized_shape.validate_json_value(value, depth + 1),
            Self::BoundedString {
                minimum_bytes,
                maximum_bytes,
                grammar,
            } => {
                let Some(text) = value.as_str() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                require(
                    !string_contains_secret_marker(text)
                        && text.len() >= *minimum_bytes as usize
                        && text.len() <= *maximum_bytes as usize
                        && grammar_admits(*grammar, text),
                )
            }
            Self::BoundedBytes {
                minimum_decoded_bytes,
                maximum_decoded_bytes,
            } => {
                let Some(text) = value.as_str() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                let Ok(bytes) = CanonicalBytes::from_base64url_no_pad(text.to_owned()) else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                let length = bytes.as_bytes().len();
                require(
                    length >= *minimum_decoded_bytes as usize
                        && length <= *maximum_decoded_bytes as usize,
                )
            }
            Self::UnsignedRange { minimum, maximum } => {
                let Some(value) = value.as_u64() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                require(value >= *minimum && value <= *maximum)
            }
            Self::SignedRange { minimum, maximum } => {
                let Some(value) = value.as_i64() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                require(value >= *minimum && value <= *maximum)
            }
            Self::Literal(literal) => require(literal_matches(literal, value)),
            Self::BoundedSequence {
                element,
                minimum_items,
                maximum_items,
                ordering,
                unique,
            } => {
                let Some(values) = value.as_array() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                if values.len() < *minimum_items as usize || values.len() > *maximum_items as usize
                {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                for value in values {
                    element.validate_json_value(value, depth + 1)?;
                }
                let encoded = values
                    .iter()
                    .map(|value| serde_json::to_string(value).unwrap_or_default())
                    .collect::<Vec<_>>();
                if *unique {
                    let distinct = encoded.iter().collect::<std::collections::BTreeSet<_>>();
                    if distinct.len() != encoded.len() {
                        return Err(ValueError::SchemaShapeMismatch);
                    }
                }
                match ordering {
                    SequenceOrdering::Preserved => Ok(()),
                    SequenceOrdering::CanonicalAscending | SequenceOrdering::Utf16Key => {
                        require(encoded.windows(2).all(|pair| pair[0] <= pair[1]))
                    }
                }
            }
            Self::BoundedStringMap {
                key_grammar,
                key_minimum_bytes,
                key_maximum_bytes,
                value: element,
                minimum_entries,
                maximum_entries,
            } => {
                let Some(object) = value.as_object() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                if object.len() < *minimum_entries as usize
                    || object.len() > *maximum_entries as usize
                {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                for (key, value) in object {
                    if string_contains_secret_marker(key)
                        || key.len() < *key_minimum_bytes as usize
                        || key.len() > *key_maximum_bytes as usize
                        || !grammar_admits(*key_grammar, key)
                    {
                        return Err(ValueError::SchemaShapeMismatch);
                    }
                    element.validate_json_value(value, depth + 1)?;
                }
                Ok(())
            }
            Self::CanonicalJsonTerminal { profile } => {
                validate_canonical_json_terminal(*profile, value)
            }
        }
    }
}

/// Decimal scale descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecimalScale {
    /// Variable scale with canonical decimal-string grammar.
    Variable,
    /// Fixed scale.
    Fixed(u16),
}

/// Field descriptor for named structs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescriptor {
    /// Canonical wire name.
    pub name: String,
    /// Field shape.
    pub shape: SchemaShape,
    /// Defaulting policy.
    pub default: FieldDefaultPolicy,
}

impl FieldDescriptor {
    /// Creates a required field descriptor.
    pub fn required(name: impl Into<String>, shape: SchemaShape) -> Self {
        Self {
            name: name.into(),
            shape,
            default: FieldDefaultPolicy::Required,
        }
    }

    /// Creates a field descriptor that is absent rather than null when unset.
    pub fn optional_absent(name: impl Into<String>, shape: SchemaShape) -> Self {
        Self {
            name: name.into(),
            shape,
            default: FieldDefaultPolicy::OptionalAbsent,
        }
    }

    /// Creates a field descriptor with framework-recognized default behavior.
    pub fn with_default(name: impl Into<String>, shape: SchemaShape) -> Self {
        Self {
            name: name.into(),
            shape,
            default: FieldDefaultPolicy::MfmDefault,
        }
    }
}

/// Field defaulting policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldDefaultPolicy {
    /// Field must be present.
    Required,
    /// Field may use `MfmDefault`.
    MfmDefault,
    /// Field is absent rather than null when it carries no value.
    ///
    /// This is deliberately distinct from an `Option` shape: an absent field and
    /// an explicit `null` are different retained bytes.
    OptionalAbsent,
}

/// Enum variant descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantDescriptor {
    /// Canonical variant wire name.
    pub name: String,
    /// Variant payload shape.
    pub shape: SchemaShape,
}

impl EnumVariantDescriptor {
    /// Creates an enum variant descriptor.
    pub fn new(name: impl Into<String>, shape: SchemaShape) -> Self {
        Self {
            name: name.into(),
            shape,
        }
    }
}

/// Supported enum tagging policy.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EnumTagging {
    /// Externally tagged enum.
    External,
    /// Internally tagged enum for struct-like variants.
    Internal {
        /// Tag field name.
        tag: String,
    },
    /// Adjacently tagged enum.
    Adjacent {
        /// Tag field name.
        tag: String,
        /// Content field name.
        content: String,
    },
}

impl EnumTagging {
    fn validate(&self) -> Result<()> {
        match self {
            Self::External => Ok(()),
            Self::Internal { tag } => {
                if valid_wire_name(tag) {
                    Ok(())
                } else {
                    Err(ValueError::Descriptor(
                        "internal enum tag is malformed".to_owned(),
                    ))
                }
            }
            Self::Adjacent { tag, content } => {
                if valid_wire_name(tag) && valid_wire_name(content) && tag != content {
                    Ok(())
                } else {
                    Err(ValueError::Descriptor(
                        "adjacent enum tag/content metadata is malformed".to_owned(),
                    ))
                }
            }
        }
    }
}

fn validate_struct_value(
    fields: &[FieldDescriptor],
    object: &serde_json::Map<String, serde_json::Value>,
    extra_field: Option<&str>,
    depth: usize,
) -> Result<()> {
    if object
        .keys()
        .any(|name| extra_field != Some(name.as_str()) && !fields.iter().any(|f| f.name == *name))
    {
        return Err(ValueError::SchemaShapeMismatch);
    }
    for field in fields {
        match object.get(&field.name) {
            Some(value) => field.shape.validate_json_value(value, depth)?,
            None if matches!(
                field.default,
                FieldDefaultPolicy::MfmDefault | FieldDefaultPolicy::OptionalAbsent
            ) => {}
            None => return Err(ValueError::SchemaShapeMismatch),
        }
    }
    Ok(())
}

fn validate_enum_value(
    tagging: &EnumTagging,
    variants: &[EnumVariantDescriptor],
    value: &serde_json::Value,
    depth: usize,
) -> Result<()> {
    match tagging {
        EnumTagging::External => match value {
            serde_json::Value::String(name) => {
                let variant = find_enum_variant(variants, name)?;
                require(matches!(variant.shape, SchemaShape::Unit))
            }
            serde_json::Value::Object(object) if object.len() == 1 => {
                let (name, payload) = object
                    .iter()
                    .next()
                    .ok_or(ValueError::SchemaShapeMismatch)?;
                let variant = find_enum_variant(variants, name)?;
                if matches!(variant.shape, SchemaShape::Unit) {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                validate_enum_payload(&variant.shape, payload, depth)
            }
            _ => Err(ValueError::SchemaShapeMismatch),
        },
        EnumTagging::Internal { tag } => {
            let object = value.as_object().ok_or(ValueError::SchemaShapeMismatch)?;
            let name = object
                .get(tag)
                .and_then(serde_json::Value::as_str)
                .ok_or(ValueError::SchemaShapeMismatch)?;
            let variant = find_enum_variant(variants, name)?;
            match &variant.shape {
                SchemaShape::Unit => require(object.len() == 1),
                SchemaShape::Struct { fields } => {
                    validate_struct_value(fields, object, Some(tag), depth)
                }
                _ => Err(ValueError::SchemaShapeMismatch),
            }
        }
        EnumTagging::Adjacent { tag, content } => {
            let object = value.as_object().ok_or(ValueError::SchemaShapeMismatch)?;
            let name = object
                .get(tag)
                .and_then(serde_json::Value::as_str)
                .ok_or(ValueError::SchemaShapeMismatch)?;
            let variant = find_enum_variant(variants, name)?;
            if matches!(variant.shape, SchemaShape::Unit) {
                return require(object.len() == 1 && !object.contains_key(content));
            }
            if object.len() != 2 {
                return Err(ValueError::SchemaShapeMismatch);
            }
            let payload = object.get(content).ok_or(ValueError::SchemaShapeMismatch)?;
            validate_enum_payload(&variant.shape, payload, depth)
        }
    }
}

fn validate_enum_payload(
    shape: &SchemaShape,
    payload: &serde_json::Value,
    depth: usize,
) -> Result<()> {
    match shape {
        SchemaShape::Tuple(elements) if elements.len() == 1 => {
            elements[0].validate_json_value(payload, depth)
        }
        _ => shape.validate_json_value(payload, depth),
    }
}

fn find_enum_variant<'a>(
    variants: &'a [EnumVariantDescriptor],
    name: &str,
) -> Result<&'a EnumVariantDescriptor> {
    variants
        .binary_search_by(|variant| variant.name.as_str().cmp(name))
        .ok()
        .map(|index| &variants[index])
        .ok_or(ValueError::SchemaShapeMismatch)
}

fn signed_integer_in_range(value: i64, bits: u16) -> bool {
    match bits {
        8 => i8::try_from(value).is_ok(),
        16 => i16::try_from(value).is_ok(),
        32 => i32::try_from(value).is_ok(),
        64 => true,
        _ => false,
    }
}

fn unsigned_integer_in_range(value: u64, bits: u16) -> bool {
    match bits {
        8 => u8::try_from(value).is_ok(),
        16 => u16::try_from(value).is_ok(),
        32 => u32::try_from(value).is_ok(),
        64 => true,
        _ => false,
    }
}

fn require(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ValueError::SchemaShapeMismatch)
    }
}

fn require_descriptor(condition: bool, message: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ValueError::Descriptor(message.to_owned()))
    }
}

fn literal_matches(literal: &LiteralValue, value: &serde_json::Value) -> bool {
    match literal {
        LiteralValue::Null => value.is_null(),
        LiteralValue::Bool(expected) => value.as_bool() == Some(*expected),
        LiteralValue::Unsigned(expected) => value.as_u64() == Some(*expected),
        LiteralValue::Signed(expected) => value.as_i64() == Some(*expected),
        LiteralValue::String(expected) => value.as_str() == Some(expected.as_str()),
    }
}

/// Applies the closed grammar named by a descriptor.
///
/// Each arm delegates to the one checked Rust owner of that rule, so the
/// descriptor never carries regular-expression text and the grammar has exactly
/// one implementation.
fn grammar_admits(grammar: StringGrammar, value: &str) -> bool {
    use mfm_ids::{ContentDigest, RunId, SchemaId, SemanticTypeId, StableId};

    match grammar {
        StringGrammar::UnicodeScalarText => !value.chars().any(|ch| ch.is_control()),
        StringGrammar::ContentDigest => ContentDigest::parse(value)
            .is_ok_and(|digest| digest.algorithm() == DigestAlgorithm::Sha256V1),
        StringGrammar::RunId => RunId::parse(value).is_ok(),
        StringGrammar::ArtifactId => mfm_ids::ArtifactId::parse(value).is_ok(),
        StringGrammar::SchemaId => SchemaId::parse(value).is_ok(),
        StringGrammar::SemanticTypeId => SemanticTypeId::parse(value).is_ok(),
        StringGrammar::EntryPointId => mfm_ids::EntryPointId::new(value).is_ok(),
        StringGrammar::StableId => StableId::new(value).is_ok(),
        StringGrammar::CanonicalUnsignedText => {
            value == "0"
                || (value.len() <= 20
                    && !value.starts_with('0')
                    && value.bytes().all(|byte| byte.is_ascii_digit()))
        }
        StringGrammar::LowerPathToken => {
            value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && value.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'/' | b'-')
                })
        }
        StringGrammar::MediaType => MediaType::new(value).is_ok(),
    }
}

/// Validates one bounded canonical-JSON terminal against its number profile.
///
/// The global canonical byte, depth, string, key, array, and object limits are
/// enforced separately by `mfm-canonical` when the value is canonicalized. This
/// walk enforces the closed number profile and per-container bounds.
fn validate_canonical_json_terminal(
    profile: CanonicalJsonProfile,
    value: &serde_json::Value,
) -> Result<()> {
    use mfm_canonical::limits::{
        MAX_ARRAY_ITEMS, MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES, MAX_OBJECT_ENTRIES,
        MAX_STRING_UTF8_BYTES,
    };

    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) => Ok(()),
        serde_json::Value::Number(number) => {
            if number.is_f64() {
                return Err(ValueError::SchemaShapeMismatch);
            }
            match profile {
                CanonicalJsonProfile::GeneralFloatFree => {
                    require(number.is_u64() || number.is_i64())
                }
            }
        }
        serde_json::Value::String(text) => {
            if text.len() > MAX_STRING_UTF8_BYTES || string_contains_secret_marker(text) {
                return Err(ValueError::SchemaShapeMismatch);
            }
            Ok(())
        }
        serde_json::Value::Array(values) => {
            if values.len() > MAX_ARRAY_ITEMS {
                return Err(ValueError::SchemaShapeMismatch);
            }
            values
                .iter()
                .try_for_each(|value| validate_canonical_json_terminal(profile, value))
        }
        serde_json::Value::Object(entries) => {
            if entries.len() > MAX_OBJECT_ENTRIES {
                return Err(ValueError::SchemaShapeMismatch);
            }
            for (key, value) in entries {
                // A key is a structural name, judged as a declared struct
                // field name is rather than scanned for secret markers. Secret
                // material is a value, and every value below is still scanned,
                // so a structural protocol name is admitted
                // while a `"Bearer …"` value is not.
                if key.is_empty()
                    || key.len() > MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES
                    || key.chars().any(char::is_control)
                {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                validate_canonical_json_terminal(profile, value)?;
            }
            Ok(())
        }
    }
}

fn valid_wire_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn valid_generic_constructor(value: &str) -> bool {
    if value.is_empty() || value.len() > 256 {
        return false;
    }
    let mut segments = value.split('/');
    let Some(namespace) = segments.next() else {
        return false;
    };
    let Some(name) = segments.next() else {
        return false;
    };
    segments.next().is_none()
        && !namespace.is_empty()
        && !name.is_empty()
        && namespace.chars().chain(name.chars()).all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')
        })
}

/// Generic schema argument identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericArgumentDescriptor {
    /// Referenced schema id.
    pub schema_id: SchemaId,
    /// Referenced semantic type id.
    pub semantic_type_id: SemanticTypeId,
}

impl GenericArgumentDescriptor {
    /// Creates a generic argument descriptor for an `MfmValue`.
    pub fn for_value<T: MfmValue>() -> Result<Self> {
        let descriptor = T::schema_descriptor()?;
        let semantic_type_id = T::semantic_id()?;
        if descriptor.identity().semantic_type_id.as_ref() != Some(&semantic_type_id) {
            return Err(ValueError::Descriptor(
                "generic descriptor does not match its Rust owner".to_owned(),
            ));
        }
        Ok(Self {
            schema_id: descriptor.schema_id()?,
            semantic_type_id,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaIdentityWire {
    canonicalization: String,
    persisted_surface: PersistedSurfaceWire,
    schema_kind: String,
    schema_name: String,
    schema_version: String,
    encoding: PersistedEncodingWire,
    semantic_type_id: Option<String>,
    versioning: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedSurfaceWire {
    numbers: String,
    secrets: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PersistedEncodingWire {
    CanonicalJson {
        shape: SchemaShapeWire,
    },
    CanonicalJsonLines {
        maximum_framed_record_bytes: u32,
        maximum_records: u32,
        maximum_stream_bytes: u64,
        minimum_records: u32,
        record_shape: SchemaShapeWire,
    },
}

impl From<&PersistedEncoding> for PersistedEncodingWire {
    fn from(encoding: &PersistedEncoding) -> Self {
        match encoding {
            PersistedEncoding::CanonicalJson { shape } => Self::CanonicalJson {
                shape: SchemaShapeWire::from(shape),
            },
            PersistedEncoding::CanonicalJsonLines {
                record_shape,
                minimum_records,
                maximum_records,
                maximum_framed_record_bytes,
                maximum_stream_bytes,
            } => Self::CanonicalJsonLines {
                maximum_framed_record_bytes: *maximum_framed_record_bytes,
                maximum_records: *maximum_records,
                maximum_stream_bytes: *maximum_stream_bytes,
                minimum_records: *minimum_records,
                record_shape: SchemaShapeWire::from(record_shape),
            },
        }
    }
}

impl TryFrom<PersistedEncodingWire> for PersistedEncoding {
    type Error = ValueError;

    fn try_from(wire: PersistedEncodingWire) -> Result<Self> {
        Ok(match wire {
            PersistedEncodingWire::CanonicalJson { shape } => Self::CanonicalJson {
                shape: SchemaShape::try_from(shape)?,
            },
            PersistedEncodingWire::CanonicalJsonLines {
                maximum_framed_record_bytes,
                maximum_records,
                maximum_stream_bytes,
                minimum_records,
                record_shape,
            } => Self::CanonicalJsonLines {
                record_shape: SchemaShape::try_from(record_shape)?,
                minimum_records,
                maximum_records,
                maximum_framed_record_bytes,
                maximum_stream_bytes,
            },
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SchemaShapeWire {
    Unit,
    Bool,
    String,
    Bytes,
    SignedInteger {
        bits: u16,
    },
    UnsignedInteger {
        bits: u16,
    },
    DecimalString {
        scale: DecimalScaleWire,
    },
    Option {
        element: Box<SchemaShapeWire>,
    },
    Vec {
        element: Box<SchemaShapeWire>,
    },
    NonEmptyVec {
        element: Box<SchemaShapeWire>,
    },
    Tuple {
        elements: Vec<SchemaShapeWire>,
    },
    Struct {
        fields: Vec<FieldDescriptorWire>,
    },
    Enum {
        tagging: EnumTaggingWire,
        variants: Vec<EnumVariantDescriptorWire>,
    },
    #[serde(rename = "btree_map")]
    BTreeMap {
        key: String,
        value: Box<SchemaShapeWire>,
    },
    InlineValue {
        schema_id: String,
        semantic_type_id: String,
        serialized_shape: Box<SchemaShapeWire>,
    },
    Generic {
        arguments: Vec<GenericArgumentDescriptorWire>,
        constructor: String,
        serialized_shape: Box<SchemaShapeWire>,
    },
    BoundedString {
        grammar: String,
        maximum_bytes: u32,
        minimum_bytes: u32,
    },
    BoundedBytes {
        maximum_decoded_bytes: u32,
        minimum_decoded_bytes: u32,
    },
    UnsignedRange {
        maximum: u64,
        minimum: u64,
    },
    SignedRange {
        maximum: i64,
        minimum: i64,
    },
    Literal {
        value: LiteralValueWire,
    },
    BoundedSequence {
        element: Box<SchemaShapeWire>,
        maximum_items: u32,
        minimum_items: u32,
        ordering: String,
        unique: bool,
    },
    BoundedStringMap {
        key_grammar: String,
        key_maximum_bytes: u32,
        key_minimum_bytes: u32,
        maximum_entries: u32,
        minimum_entries: u32,
        value: Box<SchemaShapeWire>,
    },
    CanonicalJsonTerminal {
        profile: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum LiteralValueWire {
    Null,
    Bool { value: bool },
    Unsigned { value: u64 },
    Signed { value: i64 },
    String { value: String },
}

impl From<&LiteralValue> for LiteralValueWire {
    fn from(value: &LiteralValue) -> Self {
        match value {
            LiteralValue::Null => Self::Null,
            LiteralValue::Bool(value) => Self::Bool { value: *value },
            LiteralValue::Unsigned(value) => Self::Unsigned { value: *value },
            LiteralValue::Signed(value) => Self::Signed { value: *value },
            LiteralValue::String(value) => Self::String {
                value: value.clone(),
            },
        }
    }
}

impl From<LiteralValueWire> for LiteralValue {
    fn from(value: LiteralValueWire) -> Self {
        match value {
            LiteralValueWire::Null => Self::Null,
            LiteralValueWire::Bool { value } => Self::Bool(value),
            LiteralValueWire::Unsigned { value } => Self::Unsigned(value),
            LiteralValueWire::Signed { value } => Self::Signed(value),
            LiteralValueWire::String { value } => Self::String(value),
        }
    }
}

fn parse_string_grammar(value: &str) -> Result<StringGrammar> {
    [
        StringGrammar::UnicodeScalarText,
        StringGrammar::ContentDigest,
        StringGrammar::RunId,
        StringGrammar::ArtifactId,
        StringGrammar::SchemaId,
        StringGrammar::SemanticTypeId,
        StringGrammar::EntryPointId,
        StringGrammar::StableId,
        StringGrammar::CanonicalUnsignedText,
        StringGrammar::LowerPathToken,
        StringGrammar::MediaType,
    ]
    .into_iter()
    .find(|grammar| grammar.as_str() == value)
    .ok_or(ValueError::InvalidSchemaIdentity)
}

fn parse_sequence_ordering(value: &str) -> Result<SequenceOrdering> {
    [
        SequenceOrdering::Preserved,
        SequenceOrdering::Utf16Key,
        SequenceOrdering::CanonicalAscending,
    ]
    .into_iter()
    .find(|ordering| ordering.as_str() == value)
    .ok_or(ValueError::InvalidSchemaIdentity)
}

fn parse_canonical_json_profile(value: &str) -> Result<CanonicalJsonProfile> {
    [CanonicalJsonProfile::GeneralFloatFree]
        .into_iter()
        .find(|profile| profile.as_str() == value)
        .ok_or(ValueError::InvalidSchemaIdentity)
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DecimalScaleWire {
    Variable,
    Fixed { scale: u16 },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldDescriptorWire {
    default: String,
    name: String,
    shape: SchemaShapeWire,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumVariantDescriptorWire {
    name: String,
    shape: SchemaShapeWire,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EnumTaggingWire {
    External,
    Internal { tag: String },
    Adjacent { content: String, tag: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenericArgumentDescriptorWire {
    schema_id: String,
    semantic_type_id: String,
}

impl Serialize for SchemaIdentity {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        SchemaIdentityWire::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SchemaIdentity {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SchemaIdentityWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl From<&SchemaIdentity> for SchemaIdentityWire {
    fn from(identity: &SchemaIdentity) -> Self {
        Self {
            canonicalization: identity.canonicalization.as_str().to_owned(),
            persisted_surface: PersistedSurfaceWire {
                numbers: match identity.persisted_surface.numbers {
                    NumberPolicy::NoFloats => "no_floats".to_owned(),
                },
                secrets: match identity.persisted_surface.secrets {
                    SecretPolicy::NoSecrets => "no_secrets".to_owned(),
                },
            },
            schema_kind: identity.schema_kind.as_str().to_owned(),
            schema_name: identity.schema_name.as_str().to_owned(),
            schema_version: identity.schema_version.as_str().to_owned(),
            semantic_type_id: identity.semantic_type_id.as_ref().map(ToString::to_string),
            encoding: PersistedEncodingWire::from(&identity.encoding),
            versioning: match identity.versioning {
                SchemaVersioningPolicy::ManualVersion => "manual_version".to_owned(),
            },
        }
    }
}

impl TryFrom<SchemaIdentityWire> for SchemaIdentity {
    type Error = ValueError;

    fn try_from(wire: SchemaIdentityWire) -> Result<Self> {
        if wire.canonicalization != DigestAlgorithm::Sha256JcsV1.as_str()
            || wire.versioning != "manual_version"
            || wire.persisted_surface.numbers != "no_floats"
            || wire.persisted_surface.secrets != "no_secrets"
        {
            return Err(ValueError::InvalidSchemaIdentity);
        }
        let schema_kind = match wire.schema_kind.as_str() {
            "value" => SchemaKind::Value,
            "persisted_contract" => SchemaKind::PersistedContract,
            _ => return Err(ValueError::InvalidSchemaIdentity),
        };
        let semantic_type_id = wire
            .semantic_type_id
            .map(|value| value.parse().map_err(|_| ValueError::InvalidSchemaIdentity))
            .transpose()?;
        let identity = Self {
            schema_kind,
            semantic_type_id,
            schema_name: NameToken::new(&wire.schema_name)
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
            schema_version: SchemaVersion::new(&wire.schema_version)
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
            encoding: PersistedEncoding::try_from(wire.encoding)?,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            versioning: SchemaVersioningPolicy::ManualVersion,
            persisted_surface: PersistedSurfacePolicy::strict(),
        };
        identity
            .validate()
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        Ok(identity)
    }
}

impl From<&SchemaShape> for SchemaShapeWire {
    fn from(shape: &SchemaShape) -> Self {
        match shape {
            SchemaShape::Unit => Self::Unit,
            SchemaShape::Bool => Self::Bool,
            SchemaShape::String => Self::String,
            SchemaShape::Bytes => Self::Bytes,
            SchemaShape::SignedInteger { bits } => Self::SignedInteger { bits: *bits },
            SchemaShape::UnsignedInteger { bits } => Self::UnsignedInteger { bits: *bits },
            SchemaShape::DecimalString { scale } => Self::DecimalString {
                scale: DecimalScaleWire::from(*scale),
            },
            SchemaShape::Option(element) => Self::Option {
                element: Box::new(Self::from(element.as_ref())),
            },
            SchemaShape::Vec(element) => Self::Vec {
                element: Box::new(Self::from(element.as_ref())),
            },
            SchemaShape::NonEmptyVec(element) => Self::NonEmptyVec {
                element: Box::new(Self::from(element.as_ref())),
            },
            SchemaShape::Tuple(elements) => Self::Tuple {
                elements: elements.iter().map(Self::from).collect(),
            },
            SchemaShape::Struct { fields } => Self::Struct {
                fields: fields.iter().map(FieldDescriptorWire::from).collect(),
            },
            SchemaShape::Enum { tagging, variants } => Self::Enum {
                tagging: EnumTaggingWire::from(tagging),
                variants: variants
                    .iter()
                    .map(EnumVariantDescriptorWire::from)
                    .collect(),
            },
            SchemaShape::BTreeMapString { value } => Self::BTreeMap {
                key: "string".to_owned(),
                value: Box::new(Self::from(value.as_ref())),
            },
            SchemaShape::InlineValue {
                schema_id,
                semantic_type_id,
                serialized_shape,
            } => Self::InlineValue {
                schema_id: schema_id.to_string(),
                semantic_type_id: semantic_type_id.to_string(),
                serialized_shape: Box::new(Self::from(serialized_shape.as_ref())),
            },
            SchemaShape::Generic {
                constructor,
                arguments,
                serialized_shape,
            } => Self::Generic {
                arguments: arguments
                    .iter()
                    .map(GenericArgumentDescriptorWire::from)
                    .collect(),
                constructor: constructor.clone(),
                serialized_shape: Box::new(Self::from(serialized_shape.as_ref())),
            },
            SchemaShape::BoundedString {
                minimum_bytes,
                maximum_bytes,
                grammar,
            } => Self::BoundedString {
                grammar: grammar.as_str().to_owned(),
                maximum_bytes: *maximum_bytes,
                minimum_bytes: *minimum_bytes,
            },
            SchemaShape::BoundedBytes {
                minimum_decoded_bytes,
                maximum_decoded_bytes,
            } => Self::BoundedBytes {
                maximum_decoded_bytes: *maximum_decoded_bytes,
                minimum_decoded_bytes: *minimum_decoded_bytes,
            },
            SchemaShape::UnsignedRange { minimum, maximum } => Self::UnsignedRange {
                maximum: *maximum,
                minimum: *minimum,
            },
            SchemaShape::SignedRange { minimum, maximum } => Self::SignedRange {
                maximum: *maximum,
                minimum: *minimum,
            },
            SchemaShape::Literal(value) => Self::Literal {
                value: LiteralValueWire::from(value),
            },
            SchemaShape::BoundedSequence {
                element,
                minimum_items,
                maximum_items,
                ordering,
                unique,
            } => Self::BoundedSequence {
                element: Box::new(Self::from(element.as_ref())),
                maximum_items: *maximum_items,
                minimum_items: *minimum_items,
                ordering: ordering.as_str().to_owned(),
                unique: *unique,
            },
            SchemaShape::BoundedStringMap {
                key_grammar,
                key_minimum_bytes,
                key_maximum_bytes,
                value,
                minimum_entries,
                maximum_entries,
            } => Self::BoundedStringMap {
                key_grammar: key_grammar.as_str().to_owned(),
                key_maximum_bytes: *key_maximum_bytes,
                key_minimum_bytes: *key_minimum_bytes,
                maximum_entries: *maximum_entries,
                minimum_entries: *minimum_entries,
                value: Box::new(Self::from(value.as_ref())),
            },
            SchemaShape::CanonicalJsonTerminal { profile } => Self::CanonicalJsonTerminal {
                profile: profile.as_str().to_owned(),
            },
        }
    }
}

impl TryFrom<SchemaShapeWire> for SchemaShape {
    type Error = ValueError;

    fn try_from(wire: SchemaShapeWire) -> Result<Self> {
        Ok(match wire {
            SchemaShapeWire::Unit => Self::Unit,
            SchemaShapeWire::Bool => Self::Bool,
            SchemaShapeWire::String => Self::String,
            SchemaShapeWire::Bytes => Self::Bytes,
            SchemaShapeWire::SignedInteger { bits } => Self::SignedInteger { bits },
            SchemaShapeWire::UnsignedInteger { bits } => Self::UnsignedInteger { bits },
            SchemaShapeWire::DecimalString { scale } => Self::DecimalString {
                scale: DecimalScale::from(scale),
            },
            SchemaShapeWire::Option { element } => {
                Self::Option(Box::new(Self::try_from(*element)?))
            }
            SchemaShapeWire::Vec { element } => Self::Vec(Box::new(Self::try_from(*element)?)),
            SchemaShapeWire::NonEmptyVec { element } => {
                Self::NonEmptyVec(Box::new(Self::try_from(*element)?))
            }
            SchemaShapeWire::Tuple { elements } => Self::Tuple(
                elements
                    .into_iter()
                    .map(Self::try_from)
                    .collect::<Result<_>>()?,
            ),
            SchemaShapeWire::Struct { fields } => Self::Struct {
                fields: fields
                    .into_iter()
                    .map(FieldDescriptor::try_from)
                    .collect::<Result<_>>()?,
            },
            SchemaShapeWire::Enum { tagging, variants } => Self::Enum {
                tagging: EnumTagging::from(tagging),
                variants: variants
                    .into_iter()
                    .map(EnumVariantDescriptor::try_from)
                    .collect::<Result<_>>()?,
            },
            SchemaShapeWire::BTreeMap { key, value } => {
                if key != "string" {
                    return Err(ValueError::InvalidSchemaIdentity);
                }
                Self::BTreeMapString {
                    value: Box::new(Self::try_from(*value)?),
                }
            }
            SchemaShapeWire::InlineValue {
                schema_id,
                semantic_type_id,
                serialized_shape,
            } => Self::InlineValue {
                schema_id: schema_id
                    .parse()
                    .map_err(|_| ValueError::InvalidSchemaIdentity)?,
                semantic_type_id: semantic_type_id
                    .parse()
                    .map_err(|_| ValueError::InvalidSchemaIdentity)?,
                serialized_shape: Box::new(Self::try_from(*serialized_shape)?),
            },
            SchemaShapeWire::Generic {
                arguments,
                constructor,
                serialized_shape,
            } => Self::Generic {
                constructor,
                arguments: arguments
                    .into_iter()
                    .map(GenericArgumentDescriptor::try_from)
                    .collect::<Result<_>>()?,
                serialized_shape: Box::new(Self::try_from(*serialized_shape)?),
            },
            SchemaShapeWire::BoundedString {
                grammar,
                maximum_bytes,
                minimum_bytes,
            } => Self::BoundedString {
                minimum_bytes,
                maximum_bytes,
                grammar: parse_string_grammar(&grammar)?,
            },
            SchemaShapeWire::BoundedBytes {
                maximum_decoded_bytes,
                minimum_decoded_bytes,
            } => Self::BoundedBytes {
                minimum_decoded_bytes,
                maximum_decoded_bytes,
            },
            SchemaShapeWire::UnsignedRange { maximum, minimum } => {
                Self::UnsignedRange { minimum, maximum }
            }
            SchemaShapeWire::SignedRange { maximum, minimum } => {
                Self::SignedRange { minimum, maximum }
            }
            SchemaShapeWire::Literal { value } => Self::Literal(LiteralValue::from(value)),
            SchemaShapeWire::BoundedSequence {
                element,
                maximum_items,
                minimum_items,
                ordering,
                unique,
            } => Self::BoundedSequence {
                element: Box::new(Self::try_from(*element)?),
                minimum_items,
                maximum_items,
                ordering: parse_sequence_ordering(&ordering)?,
                unique,
            },
            SchemaShapeWire::BoundedStringMap {
                key_grammar,
                key_maximum_bytes,
                key_minimum_bytes,
                maximum_entries,
                minimum_entries,
                value,
            } => Self::BoundedStringMap {
                key_grammar: parse_string_grammar(&key_grammar)?,
                key_minimum_bytes,
                key_maximum_bytes,
                value: Box::new(Self::try_from(*value)?),
                minimum_entries,
                maximum_entries,
            },
            SchemaShapeWire::CanonicalJsonTerminal { profile } => Self::CanonicalJsonTerminal {
                profile: parse_canonical_json_profile(&profile)?,
            },
        })
    }
}

impl From<DecimalScale> for DecimalScaleWire {
    fn from(scale: DecimalScale) -> Self {
        match scale {
            DecimalScale::Variable => Self::Variable,
            DecimalScale::Fixed(scale) => Self::Fixed { scale },
        }
    }
}

impl From<DecimalScaleWire> for DecimalScale {
    fn from(scale: DecimalScaleWire) -> Self {
        match scale {
            DecimalScaleWire::Variable => Self::Variable,
            DecimalScaleWire::Fixed { scale } => Self::Fixed(scale),
        }
    }
}

impl From<&FieldDescriptor> for FieldDescriptorWire {
    fn from(field: &FieldDescriptor) -> Self {
        Self {
            default: match field.default {
                FieldDefaultPolicy::Required => "required".to_owned(),
                FieldDefaultPolicy::MfmDefault => "mfm_default".to_owned(),
                FieldDefaultPolicy::OptionalAbsent => "optional_absent".to_owned(),
            },
            name: field.name.clone(),
            shape: SchemaShapeWire::from(&field.shape),
        }
    }
}

impl TryFrom<FieldDescriptorWire> for FieldDescriptor {
    type Error = ValueError;

    fn try_from(field: FieldDescriptorWire) -> Result<Self> {
        let default = match field.default.as_str() {
            "required" => FieldDefaultPolicy::Required,
            "mfm_default" => FieldDefaultPolicy::MfmDefault,
            "optional_absent" => FieldDefaultPolicy::OptionalAbsent,
            _ => return Err(ValueError::InvalidSchemaIdentity),
        };
        Ok(Self {
            name: field.name,
            shape: SchemaShape::try_from(field.shape)?,
            default,
        })
    }
}

impl From<&EnumVariantDescriptor> for EnumVariantDescriptorWire {
    fn from(variant: &EnumVariantDescriptor) -> Self {
        Self {
            name: variant.name.clone(),
            shape: SchemaShapeWire::from(&variant.shape),
        }
    }
}

impl TryFrom<EnumVariantDescriptorWire> for EnumVariantDescriptor {
    type Error = ValueError;

    fn try_from(variant: EnumVariantDescriptorWire) -> Result<Self> {
        Ok(Self {
            name: variant.name,
            shape: SchemaShape::try_from(variant.shape)?,
        })
    }
}

impl From<&EnumTagging> for EnumTaggingWire {
    fn from(tagging: &EnumTagging) -> Self {
        match tagging {
            EnumTagging::External => Self::External,
            EnumTagging::Internal { tag } => Self::Internal { tag: tag.clone() },
            EnumTagging::Adjacent { tag, content } => Self::Adjacent {
                content: content.clone(),
                tag: tag.clone(),
            },
        }
    }
}

impl From<EnumTaggingWire> for EnumTagging {
    fn from(tagging: EnumTaggingWire) -> Self {
        match tagging {
            EnumTaggingWire::External => Self::External,
            EnumTaggingWire::Internal { tag } => Self::Internal { tag },
            EnumTaggingWire::Adjacent { tag, content } => Self::Adjacent { tag, content },
        }
    }
}

impl From<&GenericArgumentDescriptor> for GenericArgumentDescriptorWire {
    fn from(argument: &GenericArgumentDescriptor) -> Self {
        Self {
            schema_id: argument.schema_id.to_string(),
            semantic_type_id: argument.semantic_type_id.to_string(),
        }
    }
}

impl TryFrom<GenericArgumentDescriptorWire> for GenericArgumentDescriptor {
    type Error = ValueError;

    fn try_from(argument: GenericArgumentDescriptorWire) -> Result<Self> {
        Ok(Self {
            schema_id: argument
                .schema_id
                .parse()
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
            semantic_type_id: argument
                .semantic_type_id
                .parse()
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
        })
    }
}

/// Marker trait for framework-approved default values.
///
/// Implementations are intentionally not blanket-derived from [`Default`]:
/// derives and framework-owned wrappers must opt in explicitly so secret or
/// unsupported persisted shapes cannot become defaultable by accident.
pub trait MfmDefault: Default {}

impl<T> MfmDefault for Option<T> {}

impl<T> MfmDefault for Vec<T> {}

impl<V> MfmDefault for BTreeMap<String, V> {}

/// Builds a descriptor for a framework-owned, hand-written value contract.
///
/// Domain crates must use the derive path. This hidden helper exists only for
/// typed kernel contracts that cannot use derives because their checked fields
/// intentionally do not implement generic serde traits.
#[doc(hidden)]
pub fn framework_value_descriptor(
    owner_crate: &str,
    semantic_type_id: SemanticTypeId,
    schema_name: &str,
    shape: SchemaShape,
    rust_type_path: &str,
) -> Result<SchemaDescriptor> {
    SchemaDescriptor::new(
        SchemaIdentity::new(
            SchemaKind::Value,
            Some(semantic_type_id),
            schema_name,
            schema_version("1")?,
            shape,
        )?,
        SchemaAudit::framework(owner_crate, rust_type_path),
    )
}

fn schema_version(value: &str) -> Result<SchemaVersion> {
    SchemaVersion::new(value).map_err(|error| ValueError::Identity(error.to_string()))
}

fn reject_duplicate_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    label: &'static str,
) -> Result<()> {
    let mut sorted = Vec::new();
    for name in names {
        if name.is_empty() {
            return Err(ValueError::Descriptor(format!(
                "{label} name must not be empty"
            )));
        }
        sorted.push(name);
    }
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        if pair[0] == pair[1] {
            return Err(ValueError::Descriptor(format!(
                "duplicate {label} name '{}'",
                pair[0]
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod secret_marker_tests {
    use super::{grammar_admits, string_contains_secret_marker, DigestAlgorithm, StringGrammar};
    use mfm_ids::{ContentDigest, DigestBytes};

    #[test]
    fn content_ref_grammar_accepts_only_exact_byte_digest_algorithm() {
        let bytes = DigestBytes::from_array([4; 32]);
        let raw = ContentDigest::from_digest(DigestAlgorithm::Sha256V1, bytes);
        let jcs = ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, bytes);

        assert!(ContentDigest::parse(jcs.as_str()).is_ok());
        assert!(grammar_admits(StringGrammar::ContentDigest, raw.as_str()));
        assert!(!grammar_admits(StringGrammar::ContentDigest, jcs.as_str()));
    }

    #[test]
    fn policy_covers_persisted_secret_canaries() {
        for marker in [
            "password",
            "passphrase",
            "mnemonic",
            "private_key",
            "privatekey",
            "seed phrase",
            "api_key",
            "apikey",
            "x-api-key",
            "access_key",
            "secret_key",
            "aws_access_key_id",
            "access_token",
            "refresh_token",
            "id_token",
            "bearer token",
            "secret",
        ] {
            assert!(
                string_contains_secret_marker(marker),
                "secret marker was not rejected: {marker}"
            );
        }
        assert!(string_contains_secret_marker(
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima"
        ));
        assert!(!string_contains_secret_marker(
            "public operation identifier"
        ));
    }
}
