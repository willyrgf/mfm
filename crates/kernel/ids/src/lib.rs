#![warn(missing_docs)]
//! Strong typed identity primitives for the MFM typed kernel.
//!
//! This crate owns the category-branded identity and version primitives used by
//! the typed-core contract. It intentionally performs only checked parsing and
//! formatting; canonical byte production and digest computation live in later
//! kernel crates.
//!
//! ```
//! use mfm_ids::{StateKind, StateVersion};
//!
//! let state_kind = StateKind::parse(
//!     "state:mfm.portfolio:load:sha256-jcs-v1:\
//!      0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
//! )?;
//! let state_version = StateVersion::new("mfm.state.load.v1")?;
//!
//! assert_eq!(state_kind.category(), "state");
//! assert_eq!(state_version.as_str(), "mfm.state.load.v1");
//! # Ok::<(), mfm_ids::IdentityError>(())
//! ```
//!
//! ```compile_fail
//! use mfm_ids::{SchemaId, SemanticTypeId};
//!
//! let schema_id: SchemaId = SemanticTypeId::parse(
//!     "semantic:mfm.kernel:portfolio/position:1:sha256-jcs-v1:\
//!      0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
//! )
//! .unwrap();
//! ```

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Result type for identity parsing and construction.
pub type Result<T> = std::result::Result<T, IdentityError>;

/// Error returned when an identity, version, algorithm, or digest violates the
/// typed kernel grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct IdentityError {
    message: String,
}

impl IdentityError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns a stable human-readable diagnostic.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Result type for checked string primitive construction.
pub type CheckedStringResult<T> = std::result::Result<T, CheckedStringError>;

/// Stable reason returned when a checked string primitive rejects input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedStringErrorReason {
    /// The value was empty.
    Empty,
    /// The value exceeded the maximum byte length.
    TooLong {
        /// Maximum allowed byte length.
        max: usize,
    },
    /// The value did not contain a required separator.
    MissingSeparator {
        /// Required separator.
        separator: char,
    },
    /// A delimited segment was empty.
    EmptySegment,
    /// A delimited segment exceeded its maximum byte length.
    SegmentTooLong {
        /// Maximum allowed segment byte length.
        max: usize,
    },
    /// The value used a reserved prefix.
    ReservedPrefix {
        /// Reserved prefix.
        prefix: &'static str,
    },
    /// The first character was not accepted by the grammar.
    InvalidStart,
    /// The last character was not accepted by the grammar.
    InvalidEnd,
    /// A character was not accepted by the grammar.
    InvalidCharacter {
        /// Rejected character.
        ch: char,
        /// Character index.
        index: usize,
    },
}

/// Error returned when a checked string primitive violates its grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{grammar} failed validation: {reason}")]
pub struct CheckedStringError {
    grammar: &'static str,
    reason: CheckedStringErrorReason,
}

impl CheckedStringError {
    fn new(grammar: &'static str, reason: CheckedStringErrorReason) -> Self {
        Self { grammar, reason }
    }

    /// Returns the checked-string grammar that rejected input.
    pub const fn grammar(&self) -> &'static str {
        self.grammar
    }

    /// Returns the stable validation failure reason.
    pub const fn reason(&self) -> &CheckedStringErrorReason {
        &self.reason
    }
}

impl fmt::Display for CheckedStringErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty"),
            Self::TooLong { max } => write!(f, "too long; max {max} bytes"),
            Self::MissingSeparator { separator } => {
                write!(f, "missing required separator `{separator}`")
            }
            Self::EmptySegment => f.write_str("empty segment"),
            Self::SegmentTooLong { max } => write!(f, "segment too long; max {max} bytes"),
            Self::ReservedPrefix { prefix } => write!(f, "reserved prefix `{prefix}`"),
            Self::InvalidStart => f.write_str("invalid start character"),
            Self::InvalidEnd => f.write_str("invalid end character"),
            Self::InvalidCharacter { ch, index } => {
                write!(f, "invalid character `{ch}` at index {index}")
            }
        }
    }
}

impl From<CheckedStringError> for IdentityError {
    fn from(error: CheckedStringError) -> Self {
        Self::new(error.to_string())
    }
}

macro_rules! checked_string_type {
    ($ty:ident, $grammar:literal, $validator:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            #[doc = concat!("Creates a checked `", stringify!($ty), "`.")]
            pub fn new(value: impl AsRef<str>) -> CheckedStringResult<Self> {
                let value = value.as_ref();
                $validator($grammar, value)?;
                Ok(Self {
                    raw: value.to_owned(),
                })
            }

            #[doc = concat!("Returns this `", stringify!($ty), "` as a string slice.")]
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            #[doc = concat!("Consumes this `", stringify!($ty), "` into its string.")]
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
            type Err = CheckedStringError;

            fn from_str(value: &str) -> CheckedStringResult<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = CheckedStringError;

            fn try_from(value: String) -> CheckedStringResult<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $ty {
            type Error = CheckedStringError;

            fn try_from(value: &str) -> CheckedStringResult<Self> {
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

checked_string_type!(
    NameToken,
    "name token",
    validate_name_token,
    "Checked identity name/version token."
);

checked_string_type!(
    StableAuthorKey,
    "stable author key",
    validate_stable_author_key,
    "Stable author-provided key used as deterministic identity input."
);

checked_string_type!(
    FieldSegment,
    "field segment",
    validate_field_segment,
    "Checked field-path segment."
);

checked_string_type!(
    FieldPath,
    "field path",
    validate_field_path,
    "Checked dot-separated field path."
);

checked_string_type!(
    ResourceNamespace,
    "resource namespace",
    validate_resource_namespace,
    "Checked cross-run resource namespace."
);

checked_string_type!(
    LocalPublicId,
    "local public id",
    validate_local_public_id,
    "Checked process-local public identifier."
);

checked_string_type!(
    RuntimeEnvName,
    "runtime env name",
    validate_runtime_env_name,
    "Checked runtime environment variable name."
);

checked_string_type!(
    RuntimeBindingId,
    "runtime binding id",
    validate_runtime_binding_id,
    "Checked runtime capability binding identifier."
);

checked_string_type!(
    RuntimeToken,
    "runtime token",
    validate_runtime_token,
    "Checked operational runtime token."
);

/// Checked visible ASCII token with a caller-selected maximum byte length.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisibleAscii<const MAX: usize> {
    raw: String,
}

impl<const MAX: usize> VisibleAscii<MAX> {
    /// Creates a checked visible ASCII token.
    pub fn new(value: impl AsRef<str>) -> CheckedStringResult<Self> {
        let value = value.as_ref();
        validate_visible_ascii("visible ascii", value, MAX)?;
        Ok(Self {
            raw: value.to_owned(),
        })
    }

    /// Returns this token as a string slice.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this token into its string.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl<const MAX: usize> AsRef<str> for VisibleAscii<MAX> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const MAX: usize> Deref for VisibleAscii<MAX> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<const MAX: usize> fmt::Display for VisibleAscii<MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const MAX: usize> FromStr for VisibleAscii<MAX> {
    type Err = CheckedStringError;

    fn from_str(value: &str) -> CheckedStringResult<Self> {
        Self::new(value)
    }
}

impl<const MAX: usize> TryFrom<String> for VisibleAscii<MAX> {
    type Error = CheckedStringError;

    fn try_from(value: String) -> CheckedStringResult<Self> {
        Self::new(value)
    }
}

impl<const MAX: usize> TryFrom<&str> for VisibleAscii<MAX> {
    type Error = CheckedStringError;

    fn try_from(value: &str) -> CheckedStringResult<Self> {
        Self::new(value)
    }
}

impl<const MAX: usize> From<VisibleAscii<MAX>> for String {
    fn from(value: VisibleAscii<MAX>) -> Self {
        value.raw
    }
}

impl<const MAX: usize> Serialize for VisibleAscii<MAX> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de, const MAX: usize> Deserialize<'de> for VisibleAscii<MAX> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).map_err(serde::de::Error::custom)
    }
}

/// Visible ASCII token capped at 256 bytes.
pub type VisibleAscii256 = VisibleAscii<256>;

/// Visible ASCII token capped at 512 bytes.
pub type VisibleAscii512 = VisibleAscii<512>;

/// Checked printable ASCII text with a caller-selected maximum byte length.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrintableAscii<const MAX: usize> {
    raw: String,
}

impl<const MAX: usize> PrintableAscii<MAX> {
    /// Creates checked printable ASCII text.
    pub fn new(value: impl AsRef<str>) -> CheckedStringResult<Self> {
        let value = value.as_ref();
        validate_printable_ascii("printable ascii", value, MAX)?;
        Ok(Self {
            raw: value.to_owned(),
        })
    }

    /// Returns this text as a string slice.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this text into its string.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl<const MAX: usize> AsRef<str> for PrintableAscii<MAX> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const MAX: usize> Deref for PrintableAscii<MAX> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<const MAX: usize> fmt::Display for PrintableAscii<MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const MAX: usize> FromStr for PrintableAscii<MAX> {
    type Err = CheckedStringError;

    fn from_str(value: &str) -> CheckedStringResult<Self> {
        Self::new(value)
    }
}

impl<const MAX: usize> TryFrom<String> for PrintableAscii<MAX> {
    type Error = CheckedStringError;

    fn try_from(value: String) -> CheckedStringResult<Self> {
        Self::new(value)
    }
}

impl<const MAX: usize> TryFrom<&str> for PrintableAscii<MAX> {
    type Error = CheckedStringError;

    fn try_from(value: &str) -> CheckedStringResult<Self> {
        Self::new(value)
    }
}

impl<const MAX: usize> From<PrintableAscii<MAX>> for String {
    fn from(value: PrintableAscii<MAX>) -> Self {
        value.raw
    }
}

impl<const MAX: usize> Serialize for PrintableAscii<MAX> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de, const MAX: usize> Deserialize<'de> for PrintableAscii<MAX> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).map_err(serde::de::Error::custom)
    }
}

/// Printable ASCII text capped at 1024 bytes.
pub type PrintableAscii1024 = PrintableAscii<1024>;

/// Printable ASCII text capped at 512 bytes.
pub type PrintableAscii512 = PrintableAscii<512>;

/// Digest algorithm identifiers accepted by typed kernel identity strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DigestAlgorithm {
    /// SHA-256 over JCS-style canonical JSON bytes.
    Sha256JcsV1,
}

impl DigestAlgorithm {
    /// Returns the canonical persisted spelling for this algorithm.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sha256JcsV1 => "sha256-jcs-v1",
        }
    }
}

impl fmt::Display for DigestAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DigestAlgorithm {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "sha256-jcs-v1" => Ok(Self::Sha256JcsV1),
            _ => Err(IdentityError::new(format!(
                "unsupported digest algorithm '{value}'"
            ))),
        }
    }
}

/// A validated 32-byte digest rendered as 64 lowercase hexadecimal characters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DigestBytes([u8; 32]);

impl DigestBytes {
    /// Creates digest bytes from a raw SHA-256 byte array.
    pub const fn from_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parses a canonical lowercase 64-character hexadecimal digest.
    pub fn from_hex(value: &str) -> Result<Self> {
        if value.len() != 64 {
            return Err(IdentityError::new(format!(
                "digest must be 64 lowercase hex characters, got length {}",
                value.len()
            )));
        }

        let mut bytes = [0_u8; 32];
        let value_bytes = value.as_bytes();
        for index in 0..32 {
            let hi = decode_hex_nibble(value_bytes[index * 2])?;
            let lo = decode_hex_nibble(value_bytes[index * 2 + 1])?;
            bytes[index] = (hi << 4) | lo;
        }
        Ok(Self(bytes))
    }

    /// Returns the raw digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DigestBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for DigestBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for DigestBytes {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::from_hex(value)
    }
}

/// Category-branded identity with private fields and checked construction.
pub struct Identity<K> {
    raw: String,
    canonical_name: Option<String>,
    version: Option<String>,
    algorithm: DigestAlgorithm,
    digest: DigestBytes,
    _kind: PhantomData<fn(K) -> K>,
}

impl<K> Clone for Identity<K> {
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            canonical_name: self.canonical_name.clone(),
            version: self.version.clone(),
            algorithm: self.algorithm,
            digest: self.digest,
            _kind: PhantomData,
        }
    }
}

impl<K> PartialEq for Identity<K> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<K> Eq for Identity<K> {}

impl<K> PartialOrd for Identity<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for Identity<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<K> Hash for Identity<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<K> fmt::Debug for Identity<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Identity").field(&self.raw).finish()
    }
}

impl<K> fmt::Display for Identity<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl<K> Identity<K>
where
    K: private::IdentityCategory,
{
    /// Parses an identity string and verifies the category-specific grammar.
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        parse_identity::<K>(value.as_ref())
    }

    /// Returns the canonical persisted identity string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Returns the category prefix, such as `schema` or `state`.
    pub fn category(&self) -> &'static str {
        K::PREFIX
    }

    /// Returns the human-readable canonical name when the category has one.
    pub fn canonical_name(&self) -> Option<&str> {
        self.canonical_name.as_deref()
    }

    /// Returns the embedded version when the identity grammar carries one.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns the digest algorithm.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    /// Returns the digest bytes.
    pub const fn digest(&self) -> &DigestBytes {
        &self.digest
    }
}

impl<K> FromStr for Identity<K>
where
    K: private::IdentityCategory,
{
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl<K> Identity<K>
where
    K: private::DigestOnlyCategory,
{
    /// Constructs a digest-only identity from already validated digest bytes.
    pub fn from_digest(algorithm: DigestAlgorithm, digest: DigestBytes) -> Self {
        let raw = format!("{}:{algorithm}:{digest}", K::PREFIX);
        Self {
            raw,
            canonical_name: None,
            version: None,
            algorithm,
            digest,
            _kind: PhantomData,
        }
    }
}

impl Identity<SemanticTypeKind> {
    /// Constructs a semantic type id from checked descriptor identity parts.
    pub fn new(
        namespace: &str,
        name: &str,
        version: &str,
        algorithm: DigestAlgorithm,
        digest: DigestBytes,
    ) -> Result<Self> {
        Self::parse(format!(
            "semantic:{namespace}:{name}:{version}:{algorithm}:{digest}"
        ))
    }
}

impl Identity<SchemaKind> {
    /// Constructs a schema id from checked descriptor identity parts.
    pub fn new(
        schema_name: &str,
        schema_version: &str,
        algorithm: DigestAlgorithm,
        digest: DigestBytes,
    ) -> Result<Self> {
        Self::parse(format!(
            "schema:{schema_name}:{schema_version}:{algorithm}:{digest}"
        ))
    }
}

macro_rules! impl_kind_identity_constructor {
    ($marker:ty, $prefix:literal) => {
        impl Identity<$marker> {
            #[doc = concat!("Constructs a `", $prefix, "` kind identity from checked descriptor identity parts.")]
            pub fn new(
                namespace: &str,
                name: &str,
                algorithm: DigestAlgorithm,
                digest: DigestBytes,
            ) -> Result<Self> {
                Self::parse(format!(
                    "{}:{namespace}:{name}:{algorithm}:{digest}",
                    $prefix
                ))
            }
        }
    };
}

impl_kind_identity_constructor!(StateKindKind, "state");
impl_kind_identity_constructor!(EffectKindKind, "effect");
impl_kind_identity_constructor!(CapabilityKindKind, "capability");
impl_kind_identity_constructor!(AdapterKindKind, "adapter");
impl_kind_identity_constructor!(OperationKindKind, "operation");

/// Category-branded version string with checked grammar.
pub struct Version<K> {
    value: String,
    _kind: PhantomData<fn(K) -> K>,
}

impl<K> Clone for Version<K> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            _kind: PhantomData,
        }
    }
}

impl<K> PartialEq for Version<K> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<K> Eq for Version<K> {}

impl<K> PartialOrd for Version<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for Version<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<K> Hash for Version<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<K> fmt::Debug for Version<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Version").field(&self.value).finish()
    }
}

impl<K> fmt::Display for Version<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl<K> Version<K>
where
    K: private::VersionCategory,
{
    /// Creates a checked version string.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        validate_token(K::FIELD_NAME, value)?;
        Ok(Self {
            value: value.to_owned(),
            _kind: PhantomData,
        })
    }

    /// Returns the canonical version string.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl<K> FromStr for Version<K>
where
    K: private::VersionCategory,
{
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// Marker for semantic type ids.
pub enum SemanticTypeKind {}

/// Marker for schema ids.
pub enum SchemaKind {}

/// Marker for state kind ids.
pub enum StateKindKind {}

/// Marker for effect kind ids.
pub enum EffectKindKind {}

/// Marker for capability kind ids.
pub enum CapabilityKindKind {}

/// Marker for adapter kind ids.
pub enum AdapterKindKind {}

/// Marker for operation kind ids.
pub enum OperationKindKind {}

/// Marker for operation instance ids.
pub enum OperationInstanceIdKind {}

/// Marker for descriptor ids.
pub enum DescriptorKind {}

/// Marker for typed execution spec hashes.
pub enum SpecHashKind {}

/// Marker for certified side-effect submit/verify pair ids.
pub enum SideEffectPairIdKind {}

/// Marker for node ids.
pub enum NodeIdKind {}

/// Marker for cell ids.
pub enum CellIdKind {}

/// Marker for scope ids.
pub enum ScopeIdKind {}

/// Marker for seed ids.
pub enum SeedIdKind {}

/// Marker for attempt ids.
pub enum AttemptIdKind {}

/// Marker for run ids.
pub enum RunIdKind {}

/// Marker for event ids.
pub enum EventIdKind {}

/// Marker for artifact ids.
pub enum ArtifactIdKind {}

/// Marker for generic content digests.
pub enum ContentDigestKind {}

/// Marker for semantic type versions.
pub enum SemanticTypeVersionKind {}

/// Marker for schema versions.
pub enum SchemaVersionKind {}

/// Marker for state versions.
pub enum StateVersionKind {}

/// Marker for effect versions.
pub enum EffectVersionKind {}

/// Marker for capability versions.
pub enum CapabilityVersionKind {}

/// Marker for adapter versions.
pub enum AdapterVersionKind {}

/// Marker for operation versions.
pub enum OperationVersionKind {}

/// Marker for typed execution spec versions.
pub enum SpecVersionKind {}

/// Marker for lowering algorithm versions.
pub enum LoweringVersionKind {}

/// Typed semantic type identity.
pub type SemanticTypeId = Identity<SemanticTypeKind>;

/// Typed schema identity.
pub type SchemaId = Identity<SchemaKind>;

/// Typed state kind identity.
pub type StateKind = Identity<StateKindKind>;

/// Typed effect kind identity.
pub type EffectKind = Identity<EffectKindKind>;

/// Typed capability kind identity.
pub type CapabilityKind = Identity<CapabilityKindKind>;

/// Typed adapter kind identity.
pub type AdapterKind = Identity<AdapterKindKind>;

/// Typed operation kind identity.
pub type OperationKind = Identity<OperationKindKind>;

/// Planned operation instance identity.
pub type OperationInstanceId = Identity<OperationInstanceIdKind>;

/// Typed descriptor identity.
pub type DescriptorId = Identity<DescriptorKind>;

/// Digest of a certified typed execution spec.
pub type SpecHash = Identity<SpecHashKind>;

/// Certified side-effect submit/verify pair identity.
pub type SideEffectPairId = Identity<SideEffectPairIdKind>;

/// Planned or certified node identity.
pub type NodeId = Identity<NodeIdKind>;

/// Planned or certified cell identity.
pub type CellId = Identity<CellIdKind>;

/// Typed scope identity.
pub type ScopeId = Identity<ScopeIdKind>;

/// Typed seed identity.
pub type SeedId = Identity<SeedIdKind>;

/// Runtime attempt identity.
pub type AttemptId = Identity<AttemptIdKind>;

/// Typed run identity.
pub type RunId = Identity<RunIdKind>;

/// Store-owned event identity.
pub type EventId = Identity<EventIdKind>;

/// Artifact storage object identity.
pub type ArtifactId = Identity<ArtifactIdKind>;

/// Generic digest of canonical bytes or artifact bytes.
pub type ContentDigest = Identity<ContentDigestKind>;

/// Store-owned deployment trust-scope identifier.
///
/// This non-secret value identifies a deployment trust domain for run identity derivation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrustScopeId(String);

impl TrustScopeId {
    /// Stable v1 trust-scope prefix.
    pub const PREFIX: &'static str = "mfm.trust_scope.v1:";

    /// Creates a trust-scope id from the stable persisted string shape.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let suffix = value
            .strip_prefix(Self::PREFIX)
            .ok_or_else(|| IdentityError::new("trust scope id prefix mismatch"))?;
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(IdentityError::new(
                "trust scope id must use 32 lowercase hex characters",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the persisted trust-scope id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TrustScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TrustScopeId {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

/// Semantic type version string.
pub type SemanticTypeVersion = Version<SemanticTypeVersionKind>;

/// Schema version string.
pub type SchemaVersion = Version<SchemaVersionKind>;

/// State implementation version string.
pub type StateVersion = Version<StateVersionKind>;

/// Effect descriptor version string.
pub type EffectVersion = Version<EffectVersionKind>;

/// Capability implementation version string.
pub type CapabilityVersion = Version<CapabilityVersionKind>;

/// Adapter implementation version string.
pub type AdapterVersion = Version<AdapterVersionKind>;

/// Operation implementation version string.
pub type OperationVersion = Version<OperationVersionKind>;

/// Certified spec contract version string.
pub type SpecVersion = Version<SpecVersionKind>;

/// Lowering algorithm version string.
pub type LoweringVersion = Version<LoweringVersionKind>;

fn parse_identity<K>(value: &str) -> Result<Identity<K>>
where
    K: private::IdentityCategory,
{
    let parts: Vec<&str> = value.split(':').collect();
    if parts.first() != Some(&K::PREFIX) {
        return Err(IdentityError::new(format!(
            "identity prefix mismatch: expected '{}'",
            K::PREFIX
        )));
    }

    match K::LAYOUT {
        private::IdentityLayout::NamespaceNameVersionDigest => {
            require_part_count(K::PREFIX, &parts, 6)?;
            validate_token("namespace", parts[1])?;
            validate_token("name", parts[2])?;
            validate_token("version", parts[3])?;
            let algorithm = parts[4].parse()?;
            let digest = parts[5].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: Some(format!("{}/{}", parts[1], parts[2])),
                version: Some(parts[3].to_owned()),
                algorithm,
                digest,
                _kind: PhantomData,
            })
        }
        private::IdentityLayout::NameVersionDigest => {
            require_part_count(K::PREFIX, &parts, 5)?;
            validate_token("name", parts[1])?;
            validate_token("version", parts[2])?;
            let algorithm = parts[3].parse()?;
            let digest = parts[4].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: Some(parts[1].to_owned()),
                version: Some(parts[2].to_owned()),
                algorithm,
                digest,
                _kind: PhantomData,
            })
        }
        private::IdentityLayout::NamespaceNameDigest => {
            require_part_count(K::PREFIX, &parts, 5)?;
            validate_token("namespace", parts[1])?;
            validate_token("name", parts[2])?;
            let algorithm = parts[3].parse()?;
            let digest = parts[4].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: Some(format!("{}/{}", parts[1], parts[2])),
                version: None,
                algorithm,
                digest,
                _kind: PhantomData,
            })
        }
        private::IdentityLayout::DigestOnly => {
            require_part_count(K::PREFIX, &parts, 3)?;
            let algorithm = parts[1].parse()?;
            let digest = parts[2].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: None,
                version: None,
                algorithm,
                digest,
                _kind: PhantomData,
            })
        }
    }
}

fn require_part_count(prefix: &str, parts: &[&str], expected: usize) -> Result<()> {
    if parts.len() == expected {
        return Ok(());
    }

    Err(IdentityError::new(format!(
        "{prefix} identity expected {expected} ':'-separated parts, got {}",
        parts.len()
    )))
}

fn validate_token(field: &'static str, value: &str) -> Result<()> {
    validate_name_token(field, value).map_err(IdentityError::from)
}

fn decode_hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(IdentityError::new(
            "digest must contain only lowercase hex characters",
        )),
    }
}

fn validate_name_token(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_non_empty(value, grammar)?;
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::Empty,
        ));
    };
    if !is_lower_or_digit(first) {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidStart,
        ));
    }
    for (offset, ch) in chars.enumerate() {
        if is_lower_or_digit(ch) || matches!(ch, '.' | '_' | '-' | '/') {
            continue;
        }
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidCharacter {
                ch,
                index: offset + 1,
            },
        ));
    }
    Ok(())
}

fn validate_stable_author_key(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_len(value, grammar, 256)?;
    for prefix in ["mfm.", "sys.", "_"] {
        if value.starts_with(prefix) {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::ReservedPrefix { prefix },
            ));
        }
    }
    for segment in value.split('/') {
        validate_segment_len(grammar, segment, 64)?;
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::EmptySegment,
            ));
        };
        if !is_lower_or_digit(first) {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::InvalidStart,
            ));
        }
        for (offset, ch) in chars.enumerate() {
            if is_lower_or_digit(ch) || matches!(ch, '.' | '_' | '-') {
                continue;
            }
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::InvalidCharacter {
                    ch,
                    index: offset + 1,
                },
            ));
        }
    }
    Ok(())
}

fn validate_field_segment(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_non_empty(value, grammar)?;
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::Empty,
        ));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidStart,
        ));
    }
    for (offset, ch) in chars.enumerate() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/') {
            continue;
        }
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidCharacter {
                ch,
                index: offset + 1,
            },
        ));
    }
    Ok(())
}

fn validate_field_path(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_non_empty(value, grammar)?;
    for segment in value.split('.') {
        validate_field_segment(grammar, segment)?;
    }
    Ok(())
}

fn validate_resource_namespace(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_len(value, grammar, 256)?;
    if !value.contains('.') {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::MissingSeparator { separator: '.' },
        ));
    }
    for segment in value.split('.') {
        validate_non_empty(segment, grammar).map_err(|_| {
            CheckedStringError::new(grammar, CheckedStringErrorReason::EmptySegment)
        })?;
        let mut chars = segment.chars();
        let Some(first) = chars.next() else {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::EmptySegment,
            ));
        };
        if !is_lower_or_digit(first) {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::InvalidStart,
            ));
        }
        for (offset, ch) in chars.enumerate() {
            if is_lower_or_digit(ch) || matches!(ch, '_' | '-') {
                continue;
            }
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::InvalidCharacter {
                    ch,
                    index: offset + 1,
                },
            ));
        }
    }
    Ok(())
}

fn validate_local_public_id(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_len(value, grammar, 128)?;
    let Some(first) = value.bytes().next() else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::Empty,
        ));
    };
    let Some(last) = value.bytes().last() else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::Empty,
        ));
    };
    if !is_lower_or_digit_byte(first) {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidStart,
        ));
    }
    if !is_lower_or_digit_byte(last) {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidEnd,
        ));
    }
    for (index, byte) in value.bytes().enumerate() {
        if is_lower_or_digit_byte(byte) || matches!(byte, b'.' | b'_' | b'-') {
            continue;
        }
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidCharacter {
                ch: byte as char,
                index,
            },
        ));
    }
    Ok(())
}

fn validate_runtime_env_name(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_len(value, grammar, 256)?;
    for (index, byte) in value.bytes().enumerate() {
        if matches!(byte, b'A'..=b'Z' | b'0'..=b'9' | b'_') {
            continue;
        }
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidCharacter {
                ch: byte as char,
                index,
            },
        ));
    }
    Ok(())
}

fn validate_runtime_binding_id(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_runtime_identifier(grammar, value, 128)
}

fn validate_runtime_token(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_runtime_identifier(grammar, value, 512)
}

fn validate_runtime_identifier(
    grammar: &'static str,
    value: &str,
    max: usize,
) -> CheckedStringResult<()> {
    validate_len(value, grammar, max)?;
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | ':') {
            continue;
        }
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidCharacter { ch, index },
        ));
    }
    Ok(())
}

fn validate_visible_ascii(
    grammar: &'static str,
    value: &str,
    max: usize,
) -> CheckedStringResult<()> {
    validate_len(value, grammar, max)?;
    for (index, byte) in value.bytes().enumerate() {
        if matches!(byte, 0x21..=0x7e) {
            continue;
        }
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidCharacter {
                ch: byte as char,
                index,
            },
        ));
    }
    Ok(())
}

fn validate_printable_ascii(
    grammar: &'static str,
    value: &str,
    max: usize,
) -> CheckedStringResult<()> {
    validate_len(value, grammar, max)?;
    for (index, byte) in value.bytes().enumerate() {
        if matches!(byte, 0x20..=0x7e) {
            continue;
        }
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidCharacter {
                ch: byte as char,
                index,
            },
        ));
    }
    Ok(())
}

fn validate_len(value: &str, grammar: &'static str, max: usize) -> CheckedStringResult<()> {
    validate_non_empty(value, grammar)?;
    if value.len() > max {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::TooLong { max },
        ));
    }
    Ok(())
}

fn validate_non_empty(value: &str, grammar: &'static str) -> CheckedStringResult<()> {
    if value.is_empty() {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::Empty,
        ));
    }
    Ok(())
}

fn validate_segment_len(
    grammar: &'static str,
    segment: &str,
    max: usize,
) -> CheckedStringResult<()> {
    if segment.is_empty() {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::EmptySegment,
        ));
    }
    if segment.len() > max {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::SegmentTooLong { max },
        ));
    }
    Ok(())
}

fn is_lower_or_digit(ch: char) -> bool {
    ch.is_ascii_lowercase() || ch.is_ascii_digit()
}

fn is_lower_or_digit_byte(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'z')
}

mod private {
    pub trait IdentityCategory {
        const PREFIX: &'static str;
        const LAYOUT: IdentityLayout;
    }

    pub trait DigestOnlyCategory: IdentityCategory {}

    pub trait VersionCategory {
        const FIELD_NAME: &'static str;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IdentityLayout {
        NamespaceNameVersionDigest,
        NameVersionDigest,
        NamespaceNameDigest,
        DigestOnly,
    }
}

macro_rules! impl_identity_category {
    ($marker:ty, $prefix:literal, $layout:ident) => {
        impl private::IdentityCategory for $marker {
            const PREFIX: &'static str = $prefix;
            const LAYOUT: private::IdentityLayout = private::IdentityLayout::$layout;
        }
    };
}

macro_rules! impl_digest_only_category {
    ($marker:ty, $prefix:literal) => {
        impl_identity_category!($marker, $prefix, DigestOnly);
        impl private::DigestOnlyCategory for $marker {}
    };
}

impl_identity_category!(SemanticTypeKind, "semantic", NamespaceNameVersionDigest);
impl_identity_category!(SchemaKind, "schema", NameVersionDigest);
impl_identity_category!(StateKindKind, "state", NamespaceNameDigest);
impl_identity_category!(EffectKindKind, "effect", NamespaceNameDigest);
impl_identity_category!(CapabilityKindKind, "capability", NamespaceNameDigest);
impl_identity_category!(AdapterKindKind, "adapter", NamespaceNameDigest);
impl_identity_category!(OperationKindKind, "operation", NamespaceNameDigest);

impl_digest_only_category!(OperationInstanceIdKind, "op");
impl_digest_only_category!(DescriptorKind, "descriptor");
impl_digest_only_category!(SpecHashKind, "spec");
impl_digest_only_category!(SideEffectPairIdKind, "side_effect_pair");
impl_digest_only_category!(NodeIdKind, "node");
impl_digest_only_category!(CellIdKind, "cell");
impl_digest_only_category!(ScopeIdKind, "scope");
impl_digest_only_category!(SeedIdKind, "seed");
impl_digest_only_category!(AttemptIdKind, "attempt");
impl_digest_only_category!(RunIdKind, "run");
impl_digest_only_category!(EventIdKind, "event");
impl_digest_only_category!(ArtifactIdKind, "artifact");
impl_digest_only_category!(ContentDigestKind, "content");

macro_rules! impl_version_category {
    ($marker:ty, $field:literal) => {
        impl private::VersionCategory for $marker {
            const FIELD_NAME: &'static str = $field;
        }
    };
}

impl_version_category!(SemanticTypeVersionKind, "semantic type version");
impl_version_category!(SchemaVersionKind, "schema version");
impl_version_category!(StateVersionKind, "state version");
impl_version_category!(EffectVersionKind, "effect version");
impl_version_category!(CapabilityVersionKind, "capability version");
impl_version_category!(AdapterVersionKind, "adapter version");
impl_version_category!(OperationVersionKind, "operation version");
impl_version_category!(SpecVersionKind, "spec version");
impl_version_category!(LoweringVersionKind, "lowering version");

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! accepts {
        ($ty:ty, $value:expr) => {
            assert_eq!(<$ty>::new($value).expect($value).as_ref(), $value);
        };
    }

    macro_rules! rejects {
        ($ty:ty, $value:expr) => {
            assert!(<$ty>::new($value).is_err(), "{:?}", $value);
        };
    }

    #[test]
    fn checked_string_primitives_cover_shared_grammars() {
        accepts!(NameToken, "mfm.kernel/value_1");
        accepts!(StableAuthorKey, "portfolio/main-wallet");
        accepts!(FieldPath, "result.total/value");
        accepts!(FieldSegment, "total/value");
        accepts!(ResourceNamespace, "mfm.evm_lane");
        accepts!(LocalPublicId, "ethereum-mainnet");
        accepts!(RuntimeEnvName, "MFM_SECRET_1");
        accepts!(RuntimeBindingId, "mfm.portfolio/runtime:v1");
        accepts!(RuntimeToken, "admission_waiter:0123456789abcdef");
        accepts!(VisibleAscii256, "text/html");
        accepts!(VisibleAscii512, "commit-key");
        accepts!(PrintableAscii512, "redacted message");
        accepts!(PrintableAscii1024, "manual resolution note");

        rejects!(NameToken, "_name");
        rejects!(StableAuthorKey, "mfm.reserved");
        rejects!(FieldPath, "result..total");
        rejects!(ResourceNamespace, "single");
        rejects!(LocalPublicId, "bad/slash");
        rejects!(RuntimeEnvName, "mfm_secret");
        rejects!(RuntimeBindingId, "bad space");
        rejects!(RuntimeToken, "bad\nline");
        rejects!(VisibleAscii256, "has space");
        rejects!(PrintableAscii512, "line\nbreak");
        rejects!(PrintableAscii1024, "line\nbreak");
    }
}
