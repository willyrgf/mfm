#![warn(missing_docs)]
//! Opaque named config custody.
//!
//! The catalog never interprets config documents.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use mfm_ids::{ContentDigest, DigestAlgorithm};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod memory;

pub use memory::MemoryCatalog;

/// Maximum retained canonical bytes in one config document.
pub const MAX_CONFIG_DOCUMENT_BYTES: usize = 256 * 1024;
/// Maximum number of live named config entries.
pub const MAX_CONFIG_ENTRIES: usize = 256;

/// Error returned when a config name violates its public grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("config name is invalid")]
pub struct ConfigNameError;

/// A bounded lowercase name in the durable config catalog.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigName(String);

impl ConfigName {
    /// Parses a config name.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ConfigNameError> {
        let value = value.as_ref();
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 64
            || !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit()
            || !bytes[bytes.len() - 1].is_ascii_lowercase()
                && !bytes[bytes.len() - 1].is_ascii_digit()
            || bytes
                .iter()
                .any(|byte| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && *byte != b'-')
        {
            return Err(ConfigNameError);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the checked name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ConfigName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ConfigName").field(&self.0).finish()
    }
}

impl fmt::Display for ConfigName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ConfigName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConfigName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Error returned when a config digest is not a canonical-JSON digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("config digest is invalid")]
pub struct ConfigDigestError;

/// Content digest of one canonical config document.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigDigest(ContentDigest);

impl ConfigDigest {
    /// Checks a content digest for the required `sha256-jcs-v1` algorithm.
    pub fn new(digest: ContentDigest) -> Result<Self, ConfigDigestError> {
        if digest.algorithm() != DigestAlgorithm::Sha256JcsV1 {
            return Err(ConfigDigestError);
        }
        Ok(Self(digest))
    }

    /// Parses a checked config digest.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, ConfigDigestError> {
        let digest = ContentDigest::parse(value).map_err(|_| ConfigDigestError)?;
        Self::new(digest)
    }

    /// Returns the underlying checked content digest.
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.0
    }

    /// Returns the canonical identity string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for ConfigDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ConfigDigest")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for ConfigDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ConfigDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConfigDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Error returned when opaque catalog custody bytes violate their bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("catalog entry is invalid")]
pub struct CatalogEntryError;

/// One owned, opaque config custody snapshot.
///
/// Construction enforces only mechanical bounds. Application is responsible
/// for canonical-document parsing and digest verification after every load.
#[derive(Clone)]
pub struct CatalogEntry {
    name: ConfigName,
    digest: ConfigDigest,
    canonical: Vec<u8>,
}

impl CatalogEntry {
    /// Constructs one bounded opaque custody record.
    pub fn new(
        name: ConfigName,
        digest: ConfigDigest,
        canonical: Vec<u8>,
    ) -> Result<Self, CatalogEntryError> {
        if canonical.is_empty() || canonical.len() > MAX_CONFIG_DOCUMENT_BYTES {
            return Err(CatalogEntryError);
        }
        Ok(Self {
            name,
            digest,
            canonical,
        })
    }

    /// Returns the catalog name observed in this snapshot.
    pub const fn name(&self) -> &ConfigName {
        &self.name
    }

    /// Returns the claimed canonical-document digest.
    pub const fn digest(&self) -> &ConfigDigest {
        &self.digest
    }

    /// Returns the opaque retained canonical bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// Consumes the record into its owned parts.
    pub fn into_parts(self) -> (ConfigName, ConfigDigest, Vec<u8>) {
        (self.name, self.digest, self.canonical)
    }
}

/// Atomic config custody outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogPutResult {
    /// The absent name was bound to the supplied entry.
    Inserted,
    /// The name already retained the exact same digest and bytes.
    Unchanged,
    /// The name retained different content and was atomically replaced.
    Updated,
}

/// Redaction-safe config catalog failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// The fixed live-entry bound was reached.
    #[error("config catalog capacity exceeded")]
    Capacity,
    /// Retained mechanical rows were invalid.
    #[error("config catalog is corrupt")]
    Corrupt,
    /// The operation definitely did not complete.
    #[error("config catalog is unavailable")]
    Unavailable,
    /// A mutation may have committed but acknowledgement was unavailable.
    #[error("config catalog mutation outcome is indeterminate")]
    Indeterminate,
}

/// The complete bounded, ordered set of owned config custody snapshots.
pub struct CatalogEntries {
    items: Vec<CatalogEntry>,
}

impl CatalogEntries {
    /// Constructs a structurally checked complete catalog snapshot.
    pub fn new(items: Vec<CatalogEntry>) -> Result<Self, CatalogError> {
        if items.len() > MAX_CONFIG_ENTRIES
            || items.windows(2).any(|pair| pair[0].name >= pair[1].name)
        {
            return Err(CatalogError::Corrupt);
        }
        Ok(Self { items })
    }

    /// Returns the ordered entries.
    pub fn items(&self) -> &[CatalogEntry] {
        &self.items
    }

    /// Consumes the snapshot into its ordered entries.
    pub fn into_items(self) -> Vec<CatalogEntry> {
        self.items
    }
}

/// Object-safe named config custody contract.
pub trait ConfigCatalog: Send + Sync {
    /// Atomically inserts, compares, or replaces one named config.
    fn put_config<'a>(
        &'a self,
        entry: &'a CatalogEntry,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogPutResult, CatalogError>> + Send + 'a>>;

    /// Loads one internally consistent owned name/digest/bytes snapshot.
    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
    ) -> Pin<Box<dyn Future<Output = Result<Option<CatalogEntry>, CatalogError>> + Send + 'a>>;

    /// Lists the complete bounded catalog in ascending name order.
    fn list_configs<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogEntries, CatalogError>> + Send + 'a>>;
}
