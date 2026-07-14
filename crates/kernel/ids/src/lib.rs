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

/// Marker for certified transition context refs.
pub enum ContextRefKind {}

/// Marker for certified transition context descriptor ids.
pub enum ContextDescriptorKind {}

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

/// Content-addressed certified transition context reference.
pub type ContextRef = Identity<ContextRefKind>;

/// Certified transition context descriptor identity.
pub type ContextDescriptorId = Identity<ContextDescriptorKind>;

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
        let suffix = value
            .strip_prefix(Self::PREFIX)
            .ok_or_else(|| IdentityError::new("store scope id prefix mismatch"))?;
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(IdentityError::new(
                "store scope id must use 32 lowercase hex characters",
            ));
        }
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

fn validate_context_component(grammar: &'static str, value: &str) -> CheckedStringResult<()> {
    validate_len(value, grammar, 256)?;
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
    byte.is_ascii_digit() || byte.is_ascii_lowercase()
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
impl_digest_only_category!(ContextRefKind, "context");
impl_digest_only_category!(ContextDescriptorKind, "context_descriptor");
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
#[path = "tests.rs"]
mod tests;
