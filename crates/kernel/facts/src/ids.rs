use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Result type for facts-kernel descriptor operations.
pub type Result<T> = std::result::Result<T, FactDescriptorError>;

/// Error returned when fact descriptors or fact descriptor primitives are invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactDescriptorError {
    /// A checked string primitive failed validation.
    #[error("{kind} {value:?} failed validation: {message}")]
    InvalidString {
        /// The checked string kind.
        kind: &'static str,
        /// The rejected value.
        value: String,
        /// Stable diagnostic.
        message: String,
    },
    /// A fact descriptor failed validation.
    #[error("fact descriptor error: {message}")]
    Descriptor {
        /// Stable diagnostic.
        message: String,
    },
    /// A fact field descriptor failed validation.
    #[error("fact field {field_id} error: {message}")]
    Field {
        /// Field id that failed validation.
        field_id: FactFieldId,
        /// Stable diagnostic.
        message: String,
    },
    /// A fact ordering descriptor failed validation.
    #[error("fact ordering {ordering} error: {message}")]
    Ordering {
        /// Ordering name that failed validation.
        ordering: FactOrderingName,
        /// Stable diagnostic.
        message: String,
    },
    /// Canonicalization failed.
    #[error("fact canonicalization error: {message}")]
    Canonical {
        /// Stable diagnostic.
        message: String,
    },
}

impl FactDescriptorError {
    fn invalid_string(kind: &'static str, value: &str, message: impl Into<String>) -> Self {
        Self::InvalidString {
            kind,
            value: value.to_owned(),
            message: message.into(),
        }
    }

    /// Creates a descriptor validation error with a stable diagnostic.
    pub fn descriptor(message: impl Into<String>) -> Self {
        Self::Descriptor {
            message: message.into(),
        }
    }

    pub(crate) fn field(field_id: FactFieldId, message: impl Into<String>) -> Self {
        Self::Field {
            field_id,
            message: message.into(),
        }
    }

    pub(crate) fn ordering(ordering: FactOrderingName, message: impl Into<String>) -> Self {
        Self::Ordering {
            ordering,
            message: message.into(),
        }
    }

    pub(crate) fn canonical(message: impl Into<String>) -> Self {
        Self::Canonical {
            message: message.into(),
        }
    }
}

macro_rules! checked_fact_string {
    ($ty:ident, $kind:literal, $validator:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            #[doc = concat!("Creates a checked `", stringify!($ty), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                $validator($kind, value)?;
                Ok(Self {
                    raw: value.to_owned(),
                })
            }

            #[doc = concat!("Returns this `", stringify!($ty), "` as a string slice.")]
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            #[doc = concat!("Consumes this `", stringify!($ty), "` into its owned string.")]
            pub fn into_string(self) -> String {
                self.raw
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $ty {
            type Err = FactDescriptorError;

            fn from_str(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = FactDescriptorError;

            fn try_from(value: String) -> Result<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $ty {
            type Error = FactDescriptorError;

            fn try_from(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.raw
            }
        }

        impl Serialize for $ty {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(&value).map_err(serde::de::Error::custom)
            }
        }
    };
}

checked_fact_string!(
    FactKind,
    "fact kind",
    validate_dot_path,
    "Stable fact kind, such as `wallet.balance` or `chain.head`."
);

checked_fact_string!(
    FactFieldId,
    "fact field id",
    validate_dot_path,
    "Descriptor-owned stable field identifier."
);

checked_fact_string!(
    FactFieldPath,
    "fact field path",
    validate_qualified_field_path,
    "Human-readable descriptor field path with a source prefix."
);

checked_fact_string!(
    FactOrderingName,
    "fact ordering name",
    validate_dot_path,
    "Descriptor-owned ordering policy name."
);

checked_fact_string!(
    FactUnit,
    "fact unit",
    validate_dot_path,
    "Descriptor-owned unit identifier for a fact field."
);

checked_fact_string!(
    CanonicalValuePath,
    "canonical value path",
    validate_dot_path,
    "Relative object path over canonical subject or response value material."
);

checked_fact_string!(
    StoreScopeRef,
    "store scope ref",
    validate_dot_path,
    "Non-secret store scope reference recorded in fact query evidence."
);

checked_fact_string!(
    FactQueryCompilerVersion,
    "fact query compiler version",
    validate_dot_path,
    "Version of the fact query compiler that produced a canonical query plan."
);

checked_fact_string!(
    FactCanonicalizerVersion,
    "fact canonicalizer version",
    validate_dot_path,
    "Version of the canonicalizer that produced fact query evidence."
);

checked_fact_string!(
    StoreIdentity,
    "store identity",
    validate_dot_path,
    "Non-secret store identity used by receipt authentication."
);

checked_fact_string!(
    StoreKeyId,
    "store key id",
    validate_dot_path,
    "Non-secret store receipt authentication key identifier."
);

fn validate_dot_path(kind: &'static str, value: &str) -> Result<()> {
    validate_len(kind, value, 256)?;
    let mut saw_segment = false;
    for segment in value.split('.') {
        validate_segment(kind, value, segment)?;
        saw_segment = true;
    }
    if !saw_segment {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "empty value",
        ));
    }
    Ok(())
}

fn validate_qualified_field_path(kind: &'static str, value: &str) -> Result<()> {
    validate_dot_path(kind, value)?;
    let Some(prefix) = value.split('.').next() else {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "missing source prefix",
        ));
    };
    if !matches!(prefix, "subject" | "result" | "metadata") {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "field path must start with subject, result, or metadata",
        ));
    }
    if value.split('.').count() < 2 {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "field path must include a field segment after the source prefix",
        ));
    }
    Ok(())
}

fn validate_len(kind: &'static str, value: &str, max: usize) -> Result<()> {
    if value.is_empty() {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            "empty value",
        ));
    }
    if value.len() > max {
        return Err(FactDescriptorError::invalid_string(
            kind,
            value,
            format!("too long; max {max} bytes"),
        ));
    }
    Ok(())
}

fn validate_segment(kind: &'static str, whole: &str, segment: &str) -> Result<()> {
    if segment.is_empty() {
        return Err(FactDescriptorError::invalid_string(
            kind,
            whole,
            "empty path segment",
        ));
    }

    let mut chars = segment.chars();
    let first = chars.next().expect("segment is non-empty");
    if !first.is_ascii_lowercase() {
        return Err(FactDescriptorError::invalid_string(
            kind,
            whole,
            "segment must start with lowercase ascii",
        ));
    }

    for ch in chars {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
            return Err(FactDescriptorError::invalid_string(
                kind,
                whole,
                format!("invalid segment character {ch:?}"),
            ));
        }
    }

    Ok(())
}

impl FactFieldPath {
    pub(crate) fn source_prefix(&self) -> Option<&str> {
        self.raw.split('.').next()
    }
}
