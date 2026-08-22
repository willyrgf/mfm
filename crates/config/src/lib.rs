#![warn(missing_docs)]
//! Opaque, versioned configuration custody.
//!
//! The repository never interprets configuration documents.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use mfm_ids::{ContentDigest, DigestAlgorithm};
use serde::{Deserialize, Deserializer, Serialize};

mod memory;

pub use memory::MemoryConfigRepository;

/// Maximum retained canonical bytes in one configuration document.
pub const MAX_CONFIG_DOCUMENT_BYTES: usize = 256 * 1024;

/// Error returned when a configuration name violates its public grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("config name is invalid")]
pub struct ConfigNameError;

/// A bounded lowercase name in the durable configuration repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ConfigName(String);

impl ConfigName {
    /// Parses a configuration name.
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

impl fmt::Display for ConfigName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
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

/// Error returned when a configuration digest is not a canonical-JSON digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("config digest is invalid")]
pub struct ConfigDigestError;

/// Content digest of one canonical configuration document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ConfigDigest(ContentDigest);

impl ConfigDigest {
    /// Checks a content digest for the required `sha256-jcs-v1` algorithm.
    pub fn new(digest: ContentDigest) -> Result<Self, ConfigDigestError> {
        if digest.algorithm() != DigestAlgorithm::Sha256JcsV1 {
            return Err(ConfigDigestError);
        }
        Ok(Self(digest))
    }

    /// Parses a checked configuration digest.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, ConfigDigestError> {
        let digest = ContentDigest::parse(value).map_err(|_| ConfigDigestError)?;
        Self::new(digest)
    }

    /// Returns the canonical identity string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ConfigDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
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

/// Error returned when opaque revision bytes violate their mechanical bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("config revision is invalid")]
pub struct ConfigRevisionError;

/// One immutable, opaque configuration revision.
///
/// Construction enforces only mechanical bounds. Application is responsible
/// for canonical-document parsing and digest verification after every load.
#[derive(Clone)]
pub struct ConfigRevision {
    name: ConfigName,
    digest: ConfigDigest,
    canonical: Vec<u8>,
}

impl ConfigRevision {
    /// Constructs one bounded opaque revision.
    pub fn new(
        name: ConfigName,
        digest: ConfigDigest,
        canonical: Vec<u8>,
    ) -> Result<Self, ConfigRevisionError> {
        if canonical.is_empty() || canonical.len() > MAX_CONFIG_DOCUMENT_BYTES {
            return Err(ConfigRevisionError);
        }
        Ok(Self {
            name,
            digest,
            canonical,
        })
    }

    /// Returns the revision name.
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

    /// Consumes the revision into its owned parts.
    pub fn into_parts(self) -> (ConfigName, ConfigDigest, Vec<u8>) {
        (self.name, self.digest, self.canonical)
    }
}

/// Atomic configuration import outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigImportResult {
    /// The exact revision was absent and was created.
    Created,
    /// The exact supplied revision was already retained and nothing was written.
    Unchanged,
}

/// Redaction-safe configuration repository failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigRepositoryError {
    /// Retained mechanical rows were invalid.
    #[error("config repository is corrupt")]
    Corrupt,
    /// The operation definitely did not complete.
    #[error("config repository is unavailable")]
    Unavailable,
    /// A mutation may have committed but acknowledgement was unavailable.
    #[error("config mutation outcome is indeterminate")]
    Indeterminate,
}

/// Boxed asynchronous configuration repository operation.
pub type ConfigFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ConfigRepositoryError>> + Send + 'a>>;

/// Object-safe, domain-generic configuration custody contract.
pub trait ConfigRepository: Send + Sync {
    /// Atomically creates or compares one exact immutable revision.
    fn import_config<'a>(
        &'a self,
        revision: &'a ConfigRevision,
    ) -> ConfigFuture<'a, ConfigImportResult>;

    /// Loads one exact retained revision.
    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, Option<ConfigRevision>>;

    /// Lists every retained revision in ascending name/digest order.
    fn list_configs(&self) -> ConfigFuture<'_, Vec<ConfigRevision>>;

    /// Idempotently deletes one exact retained revision.
    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, ()>;
}
