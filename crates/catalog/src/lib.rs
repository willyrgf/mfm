#![warn(missing_docs)]
//! Opaque named config custody and mechanical run-head enumeration.
//!
//! The catalog never interprets config documents, and the run index never
//! parses Journal frames or derives semantic run state.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use mfm_ids::{ContentDigest, DigestAlgorithm, RunId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod cursor;
mod memory;

pub use cursor::{ConfigCursor, CursorError, RunCursor, MAX_CURSOR_ENCODED_BYTES};
pub use memory::MemoryCatalog;

/// Maximum retained canonical bytes in one config document.
pub const MAX_CONFIG_DOCUMENT_BYTES: usize = 256 * 1024;
/// Maximum number of live named config entries.
pub const MAX_CONFIG_ENTRIES: usize = 256;
/// Maximum number of items returned by one catalog or index page.
pub const MAX_PAGE_ITEMS: usize = 200;
/// Default number of items requested by client surfaces.
pub const DEFAULT_PAGE_ITEMS: usize = 50;

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

/// Checked page size error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("page limit is invalid")]
pub struct PageLimitError;

/// A page size in the inclusive range 1 through 200.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageLimit(usize);

impl PageLimit {
    /// Checks a requested page size.
    pub const fn new(value: usize) -> Result<Self, PageLimitError> {
        if value == 0 || value > MAX_PAGE_ITEMS {
            return Err(PageLimitError);
        }
        Ok(Self(value))
    }

    /// Returns the checked page size.
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Default for PageLimit {
    fn default() -> Self {
        Self(DEFAULT_PAGE_ITEMS)
    }
}

/// Atomic config insertion outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogInsertResult {
    /// The absent name was bound to the supplied entry.
    Inserted,
    /// The name already retained the exact same digest and bytes.
    Unchanged,
    /// The name retained different content and nothing was written.
    Conflict,
}

/// Atomic conditional config deletion outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogDeleteResult {
    /// The matching name and digest were deleted.
    Deleted,
    /// The name was absent.
    Absent,
    /// The name was present with a different digest.
    DigestMismatch,
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

/// One bounded page of owned config custody snapshots.
pub struct CatalogPage {
    items: Vec<CatalogEntry>,
    next_cursor: Option<ConfigCursor>,
}

impl CatalogPage {
    /// Constructs a structurally checked catalog page.
    pub fn new(
        items: Vec<CatalogEntry>,
        next_cursor: Option<ConfigCursor>,
    ) -> Result<Self, CatalogError> {
        if items.len() > MAX_PAGE_ITEMS
            || items.windows(2).any(|pair| pair[0].name >= pair[1].name)
            || next_cursor.as_ref().is_some_and(|cursor| {
                items
                    .last()
                    .is_none_or(|entry| cursor.after() != entry.name())
            })
        {
            return Err(CatalogError::Corrupt);
        }
        Ok(Self { items, next_cursor })
    }

    /// Returns the ordered entries.
    pub fn items(&self) -> &[CatalogEntry] {
        &self.items
    }

    /// Returns the exclusive cursor when another item was observed.
    pub const fn next_cursor(&self) -> Option<&ConfigCursor> {
        self.next_cursor.as_ref()
    }

    /// Consumes the page into owned items and cursor.
    pub fn into_parts(self) -> (Vec<CatalogEntry>, Option<ConfigCursor>) {
        (self.items, self.next_cursor)
    }
}

/// Object-safe named config custody contract.
pub trait ConfigCatalog: Send + Sync {
    /// Atomically inserts one absent name, or compares it with retained content.
    fn insert_config<'a>(
        &'a self,
        entry: &'a CatalogEntry,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogInsertResult, CatalogError>> + Send + 'a>>;

    /// Loads one internally consistent owned name/digest/bytes snapshot.
    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
    ) -> Pin<Box<dyn Future<Output = Result<Option<CatalogEntry>, CatalogError>> + Send + 'a>>;

    /// Lists one ascending keyset page.
    fn list_configs<'a>(
        &'a self,
        cursor: Option<&'a ConfigCursor>,
        limit: PageLimit,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogPage, CatalogError>> + Send + 'a>>;

    /// Atomically deletes only the observed name/digest pair.
    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogDeleteResult, CatalogError>> + Send + 'a>>;
}

/// Error returned when one run-head projection is structurally invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("run summary is invalid")]
pub struct RunSummaryError;

/// Mechanical current-head projection for one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunSummary {
    run_id: RunId,
    head_sequence: u64,
    head_digest: ContentDigest,
    total_bytes: u64,
}

impl RunSummary {
    /// Constructs a structurally checked current-head projection.
    pub fn new(
        run_id: RunId,
        head_sequence: u64,
        head_digest: ContentDigest,
        total_bytes: u64,
    ) -> Result<Self, RunSummaryError> {
        if head_sequence == 0
            || head_digest.algorithm() != DigestAlgorithm::Sha256V1
            || total_bytes == 0
        {
            return Err(RunSummaryError);
        }
        Ok(Self {
            run_id,
            head_sequence,
            head_digest,
            total_bytes,
        })
    }

    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the current retained frame sequence.
    pub const fn head_sequence(&self) -> u64 {
        self.head_sequence
    }

    /// Returns the exact current frame digest.
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }

    /// Returns cumulative retained frame bytes.
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

/// Redaction-safe mechanical run-index failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunIndexError {
    /// Retained head rows were inconsistent.
    #[error("run index is corrupt")]
    Corrupt,
    /// The index could not be read.
    #[error("run index is unavailable")]
    Unavailable,
}

/// One bounded mechanical run-head page.
pub struct RunPage {
    items: Vec<RunSummary>,
    next_cursor: Option<RunCursor>,
}

impl RunPage {
    /// Constructs a structurally checked run page.
    pub fn new(
        items: Vec<RunSummary>,
        next_cursor: Option<RunCursor>,
    ) -> Result<Self, RunIndexError> {
        if items.len() > MAX_PAGE_ITEMS
            || items
                .windows(2)
                .any(|pair| pair[0].run_id >= pair[1].run_id)
            || next_cursor.as_ref().is_some_and(|cursor| {
                items
                    .last()
                    .is_none_or(|summary| cursor.after() != summary.run_id())
            })
        {
            return Err(RunIndexError::Corrupt);
        }
        Ok(Self { items, next_cursor })
    }

    /// Returns the ordered summaries.
    pub fn items(&self) -> &[RunSummary] {
        &self.items
    }

    /// Returns the exclusive cursor when another run was observed.
    pub const fn next_cursor(&self) -> Option<&RunCursor> {
        self.next_cursor.as_ref()
    }

    /// Consumes the page into owned items and cursor.
    pub fn into_parts(self) -> (Vec<RunSummary>, Option<RunCursor>) {
        (self.items, self.next_cursor)
    }
}

/// Object-safe mechanical current-head index.
pub trait RunIndex: Send + Sync {
    /// Lists one ascending keyset page without parsing run history.
    fn list_runs<'a>(
        &'a self,
        cursor: Option<&'a RunCursor>,
        limit: PageLimit,
    ) -> Pin<Box<dyn Future<Output = Result<RunPage, RunIndexError>> + Send + 'a>>;
}
