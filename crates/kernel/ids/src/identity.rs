use super::*;

/// Digest algorithm identifiers accepted by typed kernel identity strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DigestAlgorithm {
    /// SHA-256 over exact retained bytes.
    Sha256V1,
    /// SHA-256 over JCS-style canonical JSON bytes.
    Sha256JcsV1,
}

impl DigestAlgorithm {
    /// Returns the canonical persisted spelling for this algorithm.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sha256V1 => "sha256-v1",
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
            "sha256-v1" => Ok(Self::Sha256V1),
            "sha256-jcs-v1" => Ok(Self::Sha256JcsV1),
            _ => Err(IdentityError::new("unsupported digest algorithm")),
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

/// SHA-256 digest of one registered semantic-domain canonical envelope.
///
/// The algorithm is fixed by the type and therefore cannot be substituted by a caller.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticDigest {
    raw: String,
    digest: DigestBytes,
}

impl SemanticDigest {
    const PREFIX: &'static str = "sha256-jcs-v1:";

    /// Constructs a semantic digest from already computed SHA-256 bytes.
    pub fn from_digest(digest: DigestBytes) -> Self {
        Self {
            raw: format!("{}{digest}", Self::PREFIX),
            digest,
        }
    }

    /// Parses the exact semantic-digest grammar.
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        let digest = value
            .strip_prefix(Self::PREFIX)
            .ok_or_else(|| IdentityError::new("semantic digest prefix mismatch"))?
            .parse()?;
        Ok(Self {
            raw: value.to_owned(),
            digest,
        })
    }

    /// Returns the persisted semantic digest string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Returns the SHA-256 digest bytes.
    pub const fn digest(&self) -> &DigestBytes {
        &self.digest
    }
}

impl fmt::Debug for SemanticDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SemanticDigest").field(&self.raw).finish()
    }
}

impl fmt::Display for SemanticDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SemanticDigest {
    type Err = IdentityError;

    fn from_str(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl Serialize for SemanticDigest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SemanticDigest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

macro_rules! semantic_identity {
    ($(#[$meta:meta])* $name:ident, $prefix:literal, $label:literal) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name {
            raw: String,
            semantic_digest: SemanticDigest,
        }

        impl $name {
            /// Constructs the identity from already computed semantic digest bytes.
            pub fn from_digest(digest: DigestBytes) -> Self {
                Self::from_semantic_digest(SemanticDigest::from_digest(digest))
            }

            /// Constructs the identity from a checked semantic digest.
            pub fn from_semantic_digest(semantic_digest: SemanticDigest) -> Self {
                Self {
                    raw: format!("{}{}", $prefix, semantic_digest),
                    semantic_digest,
                }
            }

            /// Parses the exact identity grammar.
            pub fn parse(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                let semantic_digest = SemanticDigest::parse(
                    value
                        .strip_prefix($prefix)
                        .ok_or_else(|| IdentityError::new(concat!($label, " prefix mismatch")))?,
                )?;
                Ok(Self {
                    raw: value.to_owned(),
                    semantic_digest,
                })
            }

            /// Returns the persisted identity string.
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            /// Returns the semantic digest carried by this identity.
            pub const fn semantic_digest(&self) -> &SemanticDigest {
                &self.semantic_digest
            }

            /// Returns the SHA-256 digest bytes.
            pub const fn digest(&self) -> &DigestBytes {
                self.semantic_digest.digest()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.raw).finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentityError;

            fn from_str(value: &str) -> Result<Self> {
                Self::parse(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

macro_rules! branded_semantic_digest {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(SemanticDigest);

        impl $name {
            /// Constructs the branded digest from already computed SHA-256 bytes.
            pub fn from_digest(digest: DigestBytes) -> Self {
                Self(SemanticDigest::from_digest(digest))
            }

            /// Constructs the branded digest from a checked semantic digest.
            pub const fn from_semantic_digest(digest: SemanticDigest) -> Self {
                Self(digest)
            }

            /// Parses the exact semantic-digest grammar.
            pub fn parse(value: impl AsRef<str>) -> Result<Self> {
                SemanticDigest::parse(value).map(Self)
            }

            /// Returns the persisted semantic-digest string.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Returns the underlying checked semantic digest.
            pub const fn semantic_digest(&self) -> &SemanticDigest {
                &self.0
            }

            /// Returns the SHA-256 digest bytes.
            pub const fn digest(&self) -> &DigestBytes {
                self.0.digest()
            }

            /// Consumes this value into its underlying checked semantic digest.
            pub fn into_semantic_digest(self) -> SemanticDigest {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentityError;

            fn from_str(value: &str) -> Result<Self> {
                Self::parse(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

branded_semantic_digest!(
    /// Content identity digest for one retained fact.
    FactContentIdentityDigest
);

branded_semantic_digest!(
    /// Logical identity digest for one retained fact.
    FactLogicalIdentityDigest
);

branded_semantic_digest!(
    /// Digest of one closed fact-selection request.
    FactQueryDigest
);

branded_semantic_digest!(
    /// Digest of one assigned journal commit.
    JournalCommitDigest
);

branded_semantic_digest!(
    /// Semantic hash of one complete journal record.
    JournalRecordHash
);

branded_semantic_digest!(
    /// Semantic digest of a reviewed external request.
    RequestDigest
);

branded_semantic_digest!(
    /// Digest of the semantic state reconstructed for a run.
    RunSemanticStateDigest
);

semantic_identity!(
    /// Stable semantic identity of one authored structured-program call.
    SemanticCallId,
    "semantic-call:",
    "semantic call id"
);

semantic_identity!(
    /// Exact normalized identity of one executable structured-program occurrence.
    OccurrenceId,
    "occurrence:",
    "occurrence id"
);

semantic_identity!(
    /// Exact non-executable identity of one structured fragment boundary.
    FragmentBoundaryId,
    "fragment-boundary:",
    "fragment boundary id"
);

semantic_identity!(
    /// Exact identity of one certified typed-failure continuation.
    FailurePlanId,
    "failure-plan:",
    "failure plan id"
);

semantic_identity!(
    /// Immutable identity of one structured Runtime access attempt.
    AccessAttemptId,
    "access-attempt:",
    "access attempt id"
);

/// Category-branded identity with private fields and checked construction.
pub struct Identity<K> {
    pub(super) raw: String,
    pub(super) canonical_name: Option<String>,
    pub(super) algorithm: DigestAlgorithm,
    pub(super) digest: DigestBytes,
    pub(super) _kind: PhantomData<fn(K) -> K>,
}

impl<K> Clone for Identity<K> {
    fn clone(&self) -> Self {
        Self {
            raw: self.raw.clone(),
            canonical_name: self.canonical_name.clone(),
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
        match K::LAYOUT {
            private::IdentityLayout::NamespaceNameVersionDigest => self.raw.split(':').nth(3),
            private::IdentityLayout::NameVersionDigest => self.raw.split(':').nth(2),
            private::IdentityLayout::NamespaceNameDigest | private::IdentityLayout::DigestOnly => {
                None
            }
        }
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

impl<K> Serialize for Identity<K>
where
    K: private::IdentityCategory,
{
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de, K> Deserialize<'de> for Identity<K>
where
    K: private::IdentityCategory,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
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

impl_kind_identity_constructor!(EffectKindKind, "effect");
impl_kind_identity_constructor!(CapabilityKindKind, "capability");
