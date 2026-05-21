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
use std::str::FromStr;

/// Result type for identity parsing and construction.
pub type Result<T> = std::result::Result<T, IdentityError>;

/// Error returned when an identity, version, algorithm, or digest violates the
/// typed kernel grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IdentityError {}

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

/// Marker for capability kind ids.
pub enum CapabilityKindKind {}

/// Marker for adapter kind ids.
pub enum AdapterKindKind {}

/// Marker for operation kind ids.
pub enum OperationKindKind {}

/// Marker for descriptor ids.
pub enum DescriptorKind {}

/// Marker for typed execution spec hashes.
pub enum SpecHashKind {}

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

/// Typed capability kind identity.
pub type CapabilityKind = Identity<CapabilityKindKind>;

/// Typed adapter kind identity.
pub type AdapterKind = Identity<AdapterKindKind>;

/// Typed operation kind identity.
pub type OperationKind = Identity<OperationKindKind>;

/// Typed descriptor identity.
pub type DescriptorId = Identity<DescriptorKind>;

/// Digest of a certified typed execution spec.
pub type SpecHash = Identity<SpecHashKind>;

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

/// Semantic type version string.
pub type SemanticTypeVersion = Version<SemanticTypeVersionKind>;

/// Schema version string.
pub type SchemaVersion = Version<SchemaVersionKind>;

/// State implementation version string.
pub type StateVersion = Version<StateVersionKind>;

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

fn validate_token(field: &str, value: &str) -> Result<()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(IdentityError::new(format!("{field} must not be empty")));
    };

    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(IdentityError::new(format!(
            "{field} must start with [a-z0-9], got '{value}'"
        )));
    }

    for ch in chars {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-' | '/') {
            continue;
        }
        return Err(IdentityError::new(format!(
            "{field} contains invalid character '{ch}' in '{value}'"
        )));
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
impl_identity_category!(CapabilityKindKind, "capability", NamespaceNameDigest);
impl_identity_category!(AdapterKindKind, "adapter", NamespaceNameDigest);
impl_identity_category!(OperationKindKind, "operation", NamespaceNameDigest);

impl_digest_only_category!(DescriptorKind, "descriptor");
impl_digest_only_category!(SpecHashKind, "spec");
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
impl_version_category!(CapabilityVersionKind, "capability version");
impl_version_category!(AdapterVersionKind, "adapter version");
impl_version_category!(OperationVersionKind, "operation version");
impl_version_category!(SpecVersionKind, "spec version");
impl_version_category!(LoweringVersionKind, "lowering version");

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn digest() -> DigestBytes {
        DigestBytes::from_hex(DIGEST_HEX).expect("fixture digest is valid")
    }

    #[test]
    fn accepts_category_identity_golden_strings() {
        fn assert_identity<K>(
            identity: &Identity<K>,
            prefix: &str,
            canonical_name: Option<&str>,
            version: Option<&str>,
        ) where
            K: private::IdentityCategory,
        {
            assert!(identity.as_str().starts_with(prefix));
            assert_eq!(identity.canonical_name(), canonical_name);
            assert_eq!(identity.version(), version);
            assert_eq!(identity.algorithm(), DigestAlgorithm::Sha256JcsV1);
            assert_eq!(identity.digest(), &digest());
            assert_eq!(identity.to_string(), identity.as_str());
        }

        assert_identity(
            &SemanticTypeId::parse(format!(
                "semantic:mfm.kernel:portfolio/position:1.2.3:sha256-jcs-v1:{DIGEST_HEX}"
            ))
            .expect("semantic type id"),
            "semantic:mfm.kernel:portfolio/position:1.2.3:sha256-jcs-v1:",
            Some("mfm.kernel/portfolio/position"),
            Some("1.2.3"),
        );
        assert_identity(
            &SchemaId::parse(format!(
                "schema:mfm.kernel.position:1:sha256-jcs-v1:{DIGEST_HEX}"
            ))
            .expect("schema id"),
            "schema:mfm.kernel.position:1:sha256-jcs-v1:",
            Some("mfm.kernel.position"),
            Some("1"),
        );
        assert_identity(
            &StateKind::parse(format!(
                "state:mfm.evm:read-balance:sha256-jcs-v1:{DIGEST_HEX}"
            ))
            .expect("state kind"),
            "state:mfm.evm:read-balance:sha256-jcs-v1:",
            Some("mfm.evm/read-balance"),
            None,
        );
        assert_identity(
            &CapabilityKind::parse(format!(
                "capability:mfm.evm:rpc-read:sha256-jcs-v1:{DIGEST_HEX}"
            ))
            .expect("capability kind"),
            "capability:mfm.evm:rpc-read:sha256-jcs-v1:",
            Some("mfm.evm/rpc-read"),
            None,
        );
        assert_identity(
            &AdapterKind::parse(format!(
                "adapter:mfm.evm:json-rpc:sha256-jcs-v1:{DIGEST_HEX}"
            ))
            .expect("adapter kind"),
            "adapter:mfm.evm:json-rpc:sha256-jcs-v1:",
            Some("mfm.evm/json-rpc"),
            None,
        );
        assert_identity(
            &OperationKind::parse(format!(
                "operation:mfm.portfolio:track:sha256-jcs-v1:{DIGEST_HEX}"
            ))
            .expect("operation kind"),
            "operation:mfm.portfolio:track:sha256-jcs-v1:",
            Some("mfm.portfolio/track"),
            None,
        );
    }

    #[test]
    fn accepts_digest_only_identity_golden_strings() {
        let cases = [
            DescriptorId::parse(format!("descriptor:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("descriptor id")
                .to_string(),
            SpecHash::parse(format!("spec:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("spec hash")
                .to_string(),
            NodeId::parse(format!("node:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("node id")
                .to_string(),
            CellId::parse(format!("cell:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("cell id")
                .to_string(),
            ScopeId::parse(format!("scope:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("scope id")
                .to_string(),
            SeedId::parse(format!("seed:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("seed id")
                .to_string(),
            AttemptId::parse(format!("attempt:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("attempt id")
                .to_string(),
            RunId::parse(format!("run:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("run id")
                .to_string(),
            EventId::parse(format!("event:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("event id")
                .to_string(),
            ArtifactId::parse(format!("artifact:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("artifact id")
                .to_string(),
            ContentDigest::parse(format!("content:sha256-jcs-v1:{DIGEST_HEX}"))
                .expect("content digest")
                .to_string(),
        ];

        assert_eq!(
            cases,
            [
                format!("descriptor:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("spec:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("node:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("cell:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("scope:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("seed:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("attempt:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("run:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("event:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("artifact:sha256-jcs-v1:{DIGEST_HEX}"),
                format!("content:sha256-jcs-v1:{DIGEST_HEX}"),
            ]
        );
    }

    #[test]
    fn checked_constructors_produce_canonical_strings() {
        assert_eq!(
            SemanticTypeId::new(
                "mfm.kernel",
                "balance",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest()
            )
            .expect("semantic constructor")
            .as_str(),
            format!("semantic:mfm.kernel:balance:1:sha256-jcs-v1:{DIGEST_HEX}")
        );

        assert_eq!(
            SchemaId::new(
                "mfm.kernel.balance",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest()
            )
            .expect("schema constructor")
            .as_str(),
            format!("schema:mfm.kernel.balance:1:sha256-jcs-v1:{DIGEST_HEX}")
        );

        assert_eq!(
            StateKind::new(
                "mfm.portfolio",
                "load",
                DigestAlgorithm::Sha256JcsV1,
                digest()
            )
            .expect("state constructor")
            .as_str(),
            format!("state:mfm.portfolio:load:sha256-jcs-v1:{DIGEST_HEX}")
        );

        assert_eq!(
            DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest()).as_str(),
            format!("descriptor:sha256-jcs-v1:{DIGEST_HEX}")
        );
    }

    #[test]
    fn checked_versions_reject_stringly_mixups() {
        let state_version = StateVersion::new("mfm.state.v1").expect("state version");
        let operation_version =
            OperationVersion::new("mfm.operation.v1").expect("operation version");

        assert_eq!(state_version.as_str(), "mfm.state.v1");
        assert_eq!(operation_version.as_str(), "mfm.operation.v1");
        assert!(StateVersion::new("_private").is_err());
        assert!(SpecVersion::new("MFM.typed.v1").is_err());
    }

    #[test]
    fn rejects_invalid_identity_strings() {
        type RejectCase = (&'static str, fn(&str) -> bool);

        let cases: &[RejectCase] = &[
            ("schema:mfm.name:1:sha256-jcs-v1:0123", |value| {
                SchemaId::parse(value).is_err()
            }),
            (
                "state:mfm:reader:1:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                |value| StateKind::parse(value).is_err(),
            ),
            (
                "state:Mfm:reader:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                |value| StateKind::parse(value).is_err(),
            ),
            (
                "state:mfm:_reader:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                |value| StateKind::parse(value).is_err(),
            ),
            (
                "state:mfm:reader:sha256-jcs-v2:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                |value| StateKind::parse(value).is_err(),
            ),
            (
                "state:mfm:reader:sha256-jcs-v1:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
                |value| StateKind::parse(value).is_err(),
            ),
            (
                "schema:mfm.name:1:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                |value| StateKind::parse(value).is_err(),
            ),
            (
                "descriptor:mfm.name:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                |value| DescriptorId::parse(value).is_err(),
            ),
        ];

        for (value, rejects) in cases {
            assert!(rejects(value), "expected rejection for {value}");
        }
    }
}
