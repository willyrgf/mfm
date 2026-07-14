#![warn(missing_docs)]
//! Typed identities for exact MFM configuration-catalog references.
//!
//! The catalog model deliberately contains no database, setup-import, or operation-launch code.
//! A request's `T` supplies the expected schema identity; the wire reference therefore contains
//! only a checked name and content digest.

use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

use mfm_ids::{ContentDigest, IdentityError};
use mfm_values::MfmConfig;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Maximum catalog-name length in UTF-8 bytes.
pub const MAX_CATALOG_NAME_BYTES: usize = 256;

/// Error returned when a catalog identity is malformed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogIdentityError {
    /// A catalog name violates the exact catalog grammar.
    #[error("invalid catalog name: {0}")]
    InvalidName(String),
    /// A content digest is malformed.
    #[error("invalid catalog digest: {0}")]
    InvalidDigest(String),
    /// The catalog reference object has an invalid JSON shape.
    #[error("invalid catalog reference: {0}")]
    InvalidReference(String),
}

/// Checked slash-separated catalog name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogName(String);

impl CatalogName {
    /// Creates a catalog name using the shared Rust/PostgreSQL grammar.
    pub fn new(value: impl AsRef<str>) -> Result<Self, CatalogIdentityError> {
        let value = value.as_ref();
        validate_catalog_name(value)?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the canonical catalog name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the checked name into its string representation.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for CatalogName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CatalogName {
    type Err = CatalogIdentityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl Serialize for CatalogName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CatalogName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Exact typed reference to one immutable catalog value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRef<T: MfmConfig> {
    name: CatalogName,
    digest: ContentDigest,
    _config: PhantomData<fn() -> T>,
}

impl<T: MfmConfig> CatalogRef<T> {
    /// Creates an exact typed catalog reference.
    pub fn new(name: CatalogName, digest: ContentDigest) -> Self {
        Self {
            name,
            digest,
            _config: PhantomData,
        }
    }

    /// Creates an exact typed catalog reference from a name string.
    pub fn from_name(
        name: impl AsRef<str>,
        digest: ContentDigest,
    ) -> Result<Self, CatalogIdentityError> {
        Ok(Self::new(CatalogName::new(name)?, digest))
    }

    /// Returns the exact catalog name.
    pub fn name(&self) -> &CatalogName {
        &self.name
    }

    /// Returns the exact content digest.
    pub fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// Converts this reference into its erased name/digest pair for app assembly.
    pub fn into_parts(self) -> (CatalogName, ContentDigest) {
        (self.name, self.digest)
    }
}

impl<T: MfmConfig> Serialize for CatalogRef<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("CatalogRef", 2)?;
        value.serialize_field("name", &self.name)?;
        value.serialize_field("digest", self.digest.as_str())?;
        value.end()
    }
}

impl<'de, T: MfmConfig> Deserialize<'de> for CatalogRef<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CatalogRefVisitor<T: MfmConfig>(PhantomData<fn() -> T>);

        impl<'de, T: MfmConfig> Visitor<'de> for CatalogRefVisitor<T> {
            type Value = CatalogRef<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an exact catalog reference with name and digest")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut name = None;
                let mut digest = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" if name.is_none() => name = Some(map.next_value::<CatalogName>()?),
                        "digest" if digest.is_none() => digest = Some(map.next_value::<String>()?),
                        "name" | "digest" => {
                            return Err(de::Error::custom("duplicate catalog reference field"));
                        }
                        _ => return Err(de::Error::unknown_field(&key, &["name", "digest"])),
                    }
                }
                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                let digest = digest.ok_or_else(|| de::Error::missing_field("digest"))?;
                let digest = ContentDigest::parse(&digest)
                    .map_err(|error| de::Error::custom(CatalogIdentityError::from_digest(error)))?;
                Ok(CatalogRef::new(name, digest))
            }
        }

        deserializer.deserialize_map(CatalogRefVisitor(PhantomData))
    }
}

impl CatalogIdentityError {
    fn from_digest(error: IdentityError) -> Self {
        Self::InvalidDigest(error.to_string())
    }
}

fn validate_catalog_name(value: &str) -> Result<(), CatalogIdentityError> {
    if value.is_empty() {
        return Err(CatalogIdentityError::InvalidName(
            "name must not be empty".to_owned(),
        ));
    }
    if value.len() > MAX_CATALOG_NAME_BYTES {
        return Err(CatalogIdentityError::InvalidName(format!(
            "name exceeds {MAX_CATALOG_NAME_BYTES} bytes"
        )));
    }
    if !value.is_ascii() {
        return Err(CatalogIdentityError::InvalidName(
            "name must contain ASCII bytes only".to_owned(),
        ));
    }
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(CatalogIdentityError::InvalidName(
                "name segments must be non-empty and not `.` or `..`".to_owned(),
            ));
        }
        let mut chars = segment.bytes();
        let first = chars.next().expect("non-empty segment");
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return Err(CatalogIdentityError::InvalidName(
                "each segment must begin with lower-case ASCII alphanumeric".to_owned(),
            ));
        }
        if chars.any(|byte| {
            !(byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-'))
        }) {
            return Err(CatalogIdentityError::InvalidName(
                "catalog segments contain an unsupported character".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_canonical::PlainCanonicalJsonBytes;
    use mfm_ids::{DigestAlgorithm, SchemaId};
    use serde_json::json;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct ExampleConfig {
        value: u64,
    }

    impl MfmConfig for ExampleConfig {
        fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
            Err(mfm_values::ValueError::Descriptor("test".to_owned()))
        }
    }

    fn digest() -> ContentDigest {
        PlainCanonicalJsonBytes::from_json_str(r#"{"value":1}"#)
            .expect("canonical")
            .content_digest()
    }

    #[test]
    fn catalog_names_follow_the_exact_grammar() {
        for valid in ["portfolios/treasury", "a", "a.b/c_2-d"] {
            CatalogName::new(valid).expect("valid name");
        }
        for invalid in ["", "/a", "a/", "a//b", "A/a", "a/../b", "a/b!", "a/."] {
            CatalogName::new(invalid).expect_err("invalid name");
        }
    }

    #[test]
    fn reference_wire_shape_is_exact_and_rejects_unknown_fields() {
        let reference = CatalogRef::<ExampleConfig>::from_name("portfolios/treasury", digest())
            .expect("reference");
        let value = serde_json::to_value(&reference).expect("serialize");
        assert_eq!(
            value,
            json!({"name":"portfolios/treasury","digest":digest().as_str()})
        );
        let decoded: CatalogRef<ExampleConfig> = serde_json::from_value(value).expect("decode");
        assert_eq!(decoded, reference);
        let error = serde_json::from_value::<CatalogRef<ExampleConfig>>(json!({
            "name":"portfolios/treasury", "digest":digest().as_str(), "schema_id":"wrong"
        }))
        .expect_err("unknown field");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn schema_ids_remain_kernel_typed() {
        let _ = SchemaId::new(
            "mfm.test",
            "v1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0; 32]),
        )
        .expect("schema id");
    }
}
