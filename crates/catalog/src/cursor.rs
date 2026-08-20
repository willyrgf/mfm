use std::fmt;

use mfm_canonical::{CanonicalBytes, PlainCanonicalJsonBytes};
use mfm_ids::RunId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ConfigName;

/// Maximum encoded cursor size accepted by public surfaces.
pub const MAX_CURSOR_ENCODED_BYTES: usize = 512;

/// Data-free cursor grammar failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cursor is invalid")]
pub struct CursorError;

/// Exclusive ascending-name config page cursor.
#[derive(Clone, PartialEq, Eq)]
pub struct ConfigCursor {
    after: ConfigName,
    encoded: String,
}

impl ConfigCursor {
    /// Constructs the cursor following one returned config name.
    pub fn after_name(after: ConfigName) -> Self {
        let wire = config_wire(after.as_str());
        let encoded = CanonicalBytes::new(wire.into_bytes()).encoded().to_owned();
        Self { after, encoded }
    }

    /// Parses the exact bounded, resource-specific cursor wire.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, CursorError> {
        let value = value.as_ref();
        if value.is_empty() || value.len() > MAX_CURSOR_ENCODED_BYTES {
            return Err(CursorError);
        }
        let decoded = CanonicalBytes::from_base64url_no_pad(value.to_owned())
            .map_err(|_| CursorError)?
            .into_bytes();
        PlainCanonicalJsonBytes::from_canonical_json_slice(&decoded).map_err(|_| CursorError)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            after: ConfigName,
            order: String,
            resource: String,
            v: u8,
        }
        let wire: Wire = serde_json::from_slice(&decoded).map_err(|_| CursorError)?;
        if wire.order != "name-asc" || wire.resource != "configs" || wire.v != 1 {
            return Err(CursorError);
        }
        let cursor = Self::after_name(wire.after);
        (cursor.encoded == value)
            .then_some(cursor)
            .ok_or(CursorError)
    }

    /// Returns the last name from the preceding page.
    pub const fn after(&self) -> &ConfigName {
        &self.after
    }

    /// Returns the exact unpadded base64url wire.
    pub fn as_str(&self) -> &str {
        &self.encoded
    }
}

impl fmt::Debug for ConfigCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigCursor")
            .field("after", &self.after)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ConfigCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ConfigCursor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ConfigCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// Exclusive ascending-RunId run page cursor.
#[derive(Clone, PartialEq, Eq)]
pub struct RunCursor {
    after: RunId,
    encoded: String,
}

impl RunCursor {
    /// Constructs the cursor following one returned run identity.
    pub fn after_run(after: RunId) -> Self {
        let wire = run_wire(after.as_str());
        let encoded = CanonicalBytes::new(wire.into_bytes()).encoded().to_owned();
        Self { after, encoded }
    }

    /// Parses the exact bounded, resource-specific cursor wire.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, CursorError> {
        let value = value.as_ref();
        if value.is_empty() || value.len() > MAX_CURSOR_ENCODED_BYTES {
            return Err(CursorError);
        }
        let decoded = CanonicalBytes::from_base64url_no_pad(value.to_owned())
            .map_err(|_| CursorError)?
            .into_bytes();
        PlainCanonicalJsonBytes::from_canonical_json_slice(&decoded).map_err(|_| CursorError)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            after: RunId,
            order: String,
            resource: String,
            v: u8,
        }
        let wire: Wire = serde_json::from_slice(&decoded).map_err(|_| CursorError)?;
        if wire.order != "run-id-asc" || wire.resource != "runs" || wire.v != 1 {
            return Err(CursorError);
        }
        let cursor = Self::after_run(wire.after);
        (cursor.encoded == value)
            .then_some(cursor)
            .ok_or(CursorError)
    }

    /// Returns the last RunId from the preceding page.
    pub const fn after(&self) -> &RunId {
        &self.after
    }

    /// Returns the exact unpadded base64url wire.
    pub fn as_str(&self) -> &str {
        &self.encoded
    }
}

impl fmt::Debug for RunCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunCursor")
            .field("after", &self.after)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for RunCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for RunCursor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RunCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

fn config_wire(after: &str) -> String {
    format!(r#"{{"after":"{after}","order":"name-asc","resource":"configs","v":1}}"#)
}

fn run_wire(after: &str) -> String {
    format!(r#"{{"after":"{after}","order":"run-id-asc","resource":"runs","v":1}}"#)
}
