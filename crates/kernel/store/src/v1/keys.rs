use mfm_ids::{ContentDigest, VisibleAscii512};
use std::fmt;

use super::{Result, StoreError};

fn checked_store_key(field: &'static str, value: impl AsRef<str>) -> Result<VisibleAscii512> {
    let value = value.as_ref();
    VisibleAscii512::new(value)
        .map_err(|_| StoreError::Identity(format!("{field} must be 1..=512 visible ASCII bytes")))
}

/// Contiguous store-owned sequence for an atomic run commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamSeq(u64);

impl StreamSeq {
    /// First run stream sequence.
    pub const FIRST: Self = Self(1);

    #[cfg(any(test, feature = "test-support"))]
    pub(super) const MAX: Self = Self(u64::MAX);

    /// Creates a non-zero stream sequence for caller preconditions.
    pub fn new(value: u64) -> Result<Self> {
        if value == 0 {
            return Err(StoreError::Identity(
                "stream sequence must be non-zero".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the sequence as a `u64`.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub(super) fn checked_next(self) -> Result<Self> {
        Self::new(self.0.checked_add(1).ok_or(StoreError::SequenceOverflow)?)
    }
}

impl fmt::Display for StreamSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Store-owned ordinal for one event inside an atomic commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitOrdinal(u32);

impl CommitOrdinal {
    /// Creates an ordinal loaded from a persisted typed event row.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the ordinal as a `u32`.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    pub(super) fn from_index(index: usize) -> Result<Self> {
        let value = u32::try_from(index).map_err(|_| StoreError::SequenceOverflow)?;
        Ok(Self(value))
    }
}

impl fmt::Display for CommitOrdinal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Commit-key idempotency key supplied by a typed scheduler.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitKey(VisibleAscii512);

impl CommitKey {
    /// Creates a checked commit key.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_store_key("commit key", value).map(Self)
    }

    /// Returns the persisted commit key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for CommitKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Store-derived logical event key used for duplicate/conflict checks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalEventKey(VisibleAscii512);

impl LogicalEventKey {
    /// Creates a checked logical key for typed preconditions.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        checked_store_key("logical event key", value).map(Self)
    }

    /// Returns the persisted logical key string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for LogicalEventKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical fingerprint of one typed commit attempt.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommitFingerprint(ContentDigest);

impl CommitFingerprint {
    /// Returns the content digest backing this fingerprint.
    pub fn as_digest(&self) -> &ContentDigest {
        &self.0
    }

    /// Reconstructs a commit fingerprint loaded from durable authority rows.
    pub fn from_digest(digest: ContentDigest) -> Self {
        Self(digest)
    }
}

impl fmt::Display for CommitFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
