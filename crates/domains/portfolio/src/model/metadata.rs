use std::collections::BTreeMap;
use std::ops::Deref;

use mfm_program_derive::MfmValue;
use mfm_values::string_map_secret_marker_key;
use serde::{Deserialize, Serialize};

/// Error returned when public metadata contains secret-shaped content.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("public metadata key `{key}` contains secret-shaped content")]
pub struct PublicMetadataError {
    key: String,
}

impl PublicMetadataError {
    /// Returns the metadata key associated with the rejected content.
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Redaction-safe public metadata stored on portfolio model surfaces.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    try_from = "BTreeMap<String, String>",
    into = "BTreeMap<String, String>"
)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "public-metadata",
    schema = "mfm.portfolio.public_metadata",
    transparent_map
)]
pub struct PublicMetadata {
    entries: BTreeMap<String, String>,
}

impl PublicMetadata {
    /// Creates checked public metadata.
    pub fn new(entries: BTreeMap<String, String>) -> Result<Self, PublicMetadataError> {
        if let Some(key) = string_map_secret_marker_key(&entries) {
            return Err(PublicMetadataError {
                key: key.to_owned(),
            });
        }
        Ok(Self { entries })
    }

    /// Returns an empty public metadata map.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the checked metadata entries.
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.entries
    }

    /// Consumes this metadata into its underlying map.
    pub fn into_map(self) -> BTreeMap<String, String> {
        self.entries
    }

    /// Returns whether this metadata has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl TryFrom<BTreeMap<String, String>> for PublicMetadata {
    type Error = PublicMetadataError;

    fn try_from(entries: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        Self::new(entries)
    }
}

impl From<PublicMetadata> for BTreeMap<String, String> {
    fn from(metadata: PublicMetadata) -> Self {
        metadata.entries
    }
}

impl AsRef<BTreeMap<String, String>> for PublicMetadata {
    fn as_ref(&self) -> &BTreeMap<String, String> {
        self.as_map()
    }
}

impl Deref for PublicMetadata {
    type Target = BTreeMap<String, String>;

    fn deref(&self) -> &Self::Target {
        self.as_map()
    }
}

impl mfm_values::MfmDefault for PublicMetadata {}
