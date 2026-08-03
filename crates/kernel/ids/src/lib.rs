#![warn(missing_docs)]
//! Strong typed identity primitives for the MFM typed kernel.
//!
//! This crate owns the category-branded identity and version primitives used by
//! the typed-core contract. It intentionally performs only checked parsing and
//! formatting; canonical byte production and digest computation live in later
//! kernel crates.
//!
//! ```
//! use mfm_ids::{OccurrenceId, SchemaVersion};
//!
//! let occurrence_id = OccurrenceId::parse(
//!     "occurrence:sha256-jcs-v1:\
//!      0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
//! )?;
//! let schema_version = SchemaVersion::new("1")?;
//!
//! assert!(occurrence_id.as_str().starts_with("occurrence:"));
//! assert_eq!(schema_version.as_str(), "1");
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

#[path = "checked.rs"]
mod checked;
pub use self::checked::*;

#[path = "identity.rs"]
mod identity;
pub use self::identity::*;

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

/// Marker for effect kind ids.
pub enum EffectKindKind {}

/// Marker for capability kind ids.
pub enum CapabilityKindKind {}

/// Marker for run ids.
pub enum RunIdKind {}

/// Marker for artifact ids.
pub enum ArtifactIdKind {}

/// Marker for generic content digests.
pub enum ContentDigestKind {}

/// Marker for schema versions.
pub enum SchemaVersionKind {}

/// Marker for effect versions.
pub enum EffectVersionKind {}

/// Marker for capability versions.
pub enum CapabilityVersionKind {}

/// Typed semantic type identity.
pub type SemanticTypeId = Identity<SemanticTypeKind>;

/// Typed schema identity.
pub type SchemaId = Identity<SchemaKind>;

/// Typed effect kind identity.
pub type EffectKind = Identity<EffectKindKind>;

/// Typed capability kind identity.
pub type CapabilityKind = Identity<CapabilityKindKind>;

/// Typed run identity.
pub type RunId = Identity<RunIdKind>;

/// Artifact storage object identity.
pub type ArtifactId = Identity<ArtifactIdKind>;

/// Generic digest of canonical bytes or artifact bytes.
pub type ContentDigest = Identity<ContentDigestKind>;

fn validate_scope_id(value: &str, prefix: &str, label: &str) -> Result<()> {
    let suffix = value
        .strip_prefix(prefix)
        .ok_or_else(|| IdentityError::new(format!("{label} prefix mismatch")))?;
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(IdentityError::new(format!(
            "{label} must use 32 lowercase hex characters"
        )));
    }
    Ok(())
}

/// Store-owned deployment scope identifier.
///
/// This non-secret value identifies a deployment trust domain for run identity derivation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreScopeId(String);

impl StoreScopeId {
    /// Stable v1 store scope prefix.
    pub const PREFIX: &'static str = "mfm.store_scope.v1:";

    /// Creates a store scope id from the stable persisted string shape.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_scope_id(&value, Self::PREFIX, "store scope id")?;
        Ok(Self(value))
    }

    /// Returns the persisted store scope id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StoreScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for StoreScopeId {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl Serialize for StoreScopeId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StoreScopeId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Store generation used to fence append writers.
///
/// The recoverability-v1 wire form is a canonical decimal `u64` JSON string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreEpoch(u64);

impl StoreEpoch {
    /// Creates an epoch from its numeric value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric epoch.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Parses the exact canonical decimal `u64` spelling.
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        if value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(IdentityError::new(
                "store epoch must use canonical decimal u64 spelling",
            ));
        }
        value
            .parse::<u64>()
            .map(Self)
            .map_err(|_| IdentityError::new("store epoch exceeds u64"))
    }
}

impl fmt::Display for StoreEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for StoreEpoch {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for StoreEpoch {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for StoreEpoch {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// App-owned non-secret tenant ownership scope.
///
/// This scalar is copied into admitted roots and qualified resource deployments. It is not a content
/// reference, credential, principal, or mutable membership identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantScopeId(String);

impl TenantScopeId {
    /// Stable v1 tenant scope prefix.
    pub const PREFIX: &'static str = "mfm.tenant_scope.v1:";

    /// Creates a tenant scope id from the stable persisted string shape.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_scope_id(&value, Self::PREFIX, "tenant scope id")?;
        Ok(Self(value))
    }

    /// Returns the persisted tenant scope id string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TenantScopeId {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl Serialize for TenantScopeId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TenantScopeId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Lightweight content identity containing only interpretation and exact-byte digest.
///
/// This value proves neither retention, producer lineage, run reachability, object evidence, nor
/// access authority. Journal-retained values use the separate producer-bound `ValueRef` contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentRef {
    schema_id: SchemaId,
    content_digest: ContentDigest,
}

impl ContentRef {
    /// Constructs a checked raw-content reference.
    pub fn new(schema_id: SchemaId, content_digest: ContentDigest) -> Result<Self> {
        if schema_id.algorithm() != DigestAlgorithm::Sha256JcsV1 {
            return Err(IdentityError::new(
                "content ref schema id must use sha256-jcs-v1",
            ));
        }
        if content_digest.algorithm() != DigestAlgorithm::Sha256V1 {
            return Err(IdentityError::new("content ref digest must use sha256-v1"));
        }
        Ok(Self {
            schema_id,
            content_digest,
        })
    }

    /// Returns the schema identity selecting interpretation.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the exact-byte content digest.
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }
}

impl Serialize for ContentRef {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            content_digest: &'a str,
            schema_id: &'a str,
        }

        Wire {
            content_digest: self.content_digest.as_str(),
            schema_id: self.schema_id.as_str(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ContentRef {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_id: String,
            content_digest: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let schema_id = SchemaId::parse(wire.schema_id).map_err(serde::de::Error::custom)?;
        let content_digest =
            ContentDigest::parse(wire.content_digest).map_err(serde::de::Error::custom)?;
        Self::new(schema_id, content_digest).map_err(serde::de::Error::custom)
    }
}

/// Schema version string.
pub type SchemaVersion = Version<SchemaVersionKind>;

/// Effect descriptor version string.
pub type EffectVersion = Version<EffectVersionKind>;

/// Capability implementation version string.
pub type CapabilityVersion = Version<CapabilityVersionKind>;

fn parse_identity<K>(value: &str) -> Result<Identity<K>>
where
    K: private::IdentityCategory,
{
    if value.len() > 512 {
        return Err(IdentityError::new(
            "identity exceeds the 512-byte grammar bound",
        ));
    }
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
            validate_identity_algorithm::<K>(algorithm)?;
            let digest = parts[5].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: Some(format!("{}/{}", parts[1], parts[2])),
                algorithm,
                digest,
                _kind: PhantomData,
            })
        }
        private::IdentityLayout::NameVersionDigest => {
            require_part_count(K::PREFIX, &parts, 5)?;
            validate_token("name", parts[1])?;
            validate_token("version", parts[2])?;
            if K::POSITIVE_CANONICAL_U64_VERSION {
                validate_positive_canonical_u64_version(parts[2])?;
            }
            if K::REQUIRED_VERSION.is_some_and(|version| version != parts[2]) {
                return Err(IdentityError::new(
                    "identity version is not admitted for this category",
                ));
            }
            let algorithm = parts[3].parse()?;
            validate_identity_algorithm::<K>(algorithm)?;
            let digest = parts[4].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: Some(parts[1].to_owned()),
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
            validate_identity_algorithm::<K>(algorithm)?;
            let digest = parts[4].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: Some(format!("{}/{}", parts[1], parts[2])),
                algorithm,
                digest,
                _kind: PhantomData,
            })
        }
        private::IdentityLayout::DigestOnly => {
            require_part_count(K::PREFIX, &parts, 3)?;
            let algorithm = parts[1].parse()?;
            validate_identity_algorithm::<K>(algorithm)?;
            let digest = parts[2].parse()?;
            Ok(Identity {
                raw: value.to_owned(),
                canonical_name: None,
                algorithm,
                digest,
                _kind: PhantomData,
            })
        }
    }
}

fn validate_identity_algorithm<K>(algorithm: DigestAlgorithm) -> Result<()>
where
    K: private::IdentityCategory,
{
    if K::ALGORITHM.is_some_and(|expected| expected != algorithm) {
        return Err(IdentityError::new(
            "digest algorithm is not admitted for this identity category",
        ));
    }
    Ok(())
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

fn validate_positive_canonical_u64_version(value: &str) -> Result<()> {
    if value.starts_with('0')
        || value
            .parse::<u64>()
            .ok()
            .filter(|version| *version > 0)
            .is_none()
    {
        return Err(IdentityError::new(
            "identity version must use positive canonical u64 spelling",
        ));
    }
    Ok(())
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

fn validate_stable_id(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_len(value, grammar, 512)?;
    if !value.bytes().next().is_some_and(is_lower_or_digit_byte) {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidStart,
        ));
    }
    for (index, byte) in value.bytes().enumerate().skip(1) {
        if is_lower_or_digit_byte(byte) || matches!(byte, b'.' | b'_' | b'/' | b'-') {
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

fn validate_entry_point_id(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_len(value, grammar, 512)?;
    let Some(body) = value.strip_prefix("mfm.") else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidStart,
        ));
    };
    let Some((qualified_name, version)) = body.rsplit_once('@') else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::MissingSeparator { separator: '@' },
        ));
    };
    let mut components = qualified_name.split('/');
    let Some(domain) = components.next() else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::EmptySegment,
        ));
    };
    let Some(name) = components.next() else {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::MissingSeparator { separator: '/' },
        ));
    };
    if components.next().is_some() || domain.is_empty() || name.is_empty() {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::EmptySegment,
        ));
    }
    for component in [domain, name] {
        let first = component.as_bytes().first();
        let last = component.as_bytes().last();
        if !first.is_some_and(|byte| is_lower_or_digit_byte(*byte)) {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::InvalidStart,
            ));
        }
        if !last.is_some_and(|byte| is_lower_or_digit_byte(*byte)) {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::InvalidEnd,
            ));
        }
        for (index, byte) in component.bytes().enumerate() {
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
    }
    if version.is_empty()
        || version.starts_with('0')
        || !version.bytes().all(|byte| byte.is_ascii_digit())
        || version
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    {
        return Err(CheckedStringError::new(
            grammar,
            CheckedStringErrorReason::InvalidEnd,
        ));
    }
    Ok(())
}

fn validate_invocation_identity(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    if value.len() != 36 {
        return Err(CheckedStringError::new(
            grammar,
            if value.is_empty() {
                CheckedStringErrorReason::Empty
            } else {
                CheckedStringErrorReason::InvalidEnd
            },
        ));
    }
    for (index, byte) in value.bytes().enumerate() {
        let accepted = match index {
            8 | 13 | 18 | 23 => byte == b'-',
            14 => byte == b'4',
            19 => matches!(byte, b'8' | b'9' | b'a' | b'b'),
            _ => byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'),
        };
        if !accepted {
            return Err(CheckedStringError::new(
                grammar,
                CheckedStringErrorReason::InvalidCharacter {
                    ch: byte as char,
                    index,
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
    byte.is_ascii_digit() || byte.is_ascii_lowercase()
}

mod private {
    use super::DigestAlgorithm;

    pub trait IdentityCategory {
        const PREFIX: &'static str;
        const LAYOUT: IdentityLayout;
        const ALGORITHM: Option<DigestAlgorithm>;
        const REQUIRED_VERSION: Option<&'static str>;
        const POSITIVE_CANONICAL_U64_VERSION: bool;
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
            const ALGORITHM: Option<DigestAlgorithm> = Some(DigestAlgorithm::Sha256JcsV1);
            const REQUIRED_VERSION: Option<&'static str> = None;
            const POSITIVE_CANONICAL_U64_VERSION: bool = false;
        }
    };
}

macro_rules! impl_digest_only_category {
    ($marker:ty, $prefix:literal) => {
        impl_identity_category!($marker, $prefix, DigestOnly);
        impl private::DigestOnlyCategory for $marker {}
    };
}

macro_rules! impl_unrestricted_digest_only_category {
    ($marker:ty, $prefix:literal) => {
        impl private::IdentityCategory for $marker {
            const PREFIX: &'static str = $prefix;
            const LAYOUT: private::IdentityLayout = private::IdentityLayout::DigestOnly;
            const ALGORITHM: Option<DigestAlgorithm> = None;
            const REQUIRED_VERSION: Option<&'static str> = None;
            const POSITIVE_CANONICAL_U64_VERSION: bool = false;
        }
        impl private::DigestOnlyCategory for $marker {}
    };
}

impl_identity_category!(SemanticTypeKind, "semantic", NamespaceNameVersionDigest);
impl private::IdentityCategory for SchemaKind {
    const PREFIX: &'static str = "schema";
    const LAYOUT: private::IdentityLayout = private::IdentityLayout::NameVersionDigest;
    const ALGORITHM: Option<DigestAlgorithm> = Some(DigestAlgorithm::Sha256JcsV1);
    const REQUIRED_VERSION: Option<&'static str> = None;
    const POSITIVE_CANONICAL_U64_VERSION: bool = true;
}
impl_identity_category!(EffectKindKind, "effect", NamespaceNameDigest);
impl_identity_category!(CapabilityKindKind, "capability", NamespaceNameDigest);

impl_digest_only_category!(RunIdKind, "run");
impl_digest_only_category!(ArtifactIdKind, "artifact");
impl_unrestricted_digest_only_category!(ContentDigestKind, "content");

macro_rules! impl_version_category {
    ($marker:ty, $field:literal) => {
        impl private::VersionCategory for $marker {
            const FIELD_NAME: &'static str = $field;
        }
    };
}

impl_version_category!(SchemaVersionKind, "schema version");
impl_version_category!(EffectVersionKind, "effect version");
impl_version_category!(CapabilityVersionKind, "capability version");

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
