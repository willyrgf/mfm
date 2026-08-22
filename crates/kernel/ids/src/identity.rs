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
            private::IdentityLayout::DigestOnly => None,
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
    pub(super) fn with_digest_algorithm(algorithm: DigestAlgorithm, digest: DigestBytes) -> Self {
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

impl Identity<RunIdKind> {
    /// Constructs a run id with the fixed `Sha256JcsV1` algorithm.
    pub fn from_digest(digest: DigestBytes) -> Self {
        Self::with_digest_algorithm(DigestAlgorithm::Sha256JcsV1, digest)
    }
}

impl Identity<EffectIdKind> {
    /// Constructs an effect id with the fixed `Sha256JcsV1` algorithm.
    pub fn from_digest(digest: DigestBytes) -> Self {
        Self::with_digest_algorithm(DigestAlgorithm::Sha256JcsV1, digest)
    }
}

impl Identity<ArtifactIdKind> {
    /// Constructs an artifact id with the fixed `Sha256JcsV1` algorithm.
    pub fn from_digest(digest: DigestBytes) -> Self {
        Self::with_digest_algorithm(DigestAlgorithm::Sha256JcsV1, digest)
    }
}

impl Identity<ContentDigestKind> {
    /// Constructs a content digest with one caller-selected supported algorithm.
    pub fn from_digest(algorithm: DigestAlgorithm, digest: DigestBytes) -> Self {
        Self::with_digest_algorithm(algorithm, digest)
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
