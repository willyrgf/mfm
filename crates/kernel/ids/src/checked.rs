use super::*;

/// Result type for checked string primitive construction.
pub type CheckedStringResult<T> = std::result::Result<T, CheckedStringError>;

/// Stable reason returned when a checked string primitive rejects input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedStringErrorReason {
    /// The value was empty.
    Empty,
    /// The value exceeded the maximum byte length.
    TooLong {
        /// Maximum allowed byte length.
        max: usize,
    },
    /// The value did not contain a required separator.
    MissingSeparator {
        /// Required separator.
        separator: char,
    },
    /// A delimited segment was empty.
    EmptySegment,
    /// A delimited segment exceeded its maximum byte length.
    SegmentTooLong {
        /// Maximum allowed segment byte length.
        max: usize,
    },
    /// The value used a reserved prefix.
    ReservedPrefix {
        /// Reserved prefix.
        prefix: &'static str,
    },
    /// The first character was not accepted by the grammar.
    InvalidStart,
    /// The last character was not accepted by the grammar.
    InvalidEnd,
    /// A character was not accepted by the grammar.
    InvalidCharacter {
        /// Rejected character.
        ch: char,
        /// Character index.
        index: usize,
    },
}

/// Error returned when a checked string primitive violates its grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{grammar} failed validation: {reason}")]
pub struct CheckedStringError {
    grammar: &'static str,
    reason: CheckedStringErrorReason,
}

impl CheckedStringError {
    pub(super) fn new(grammar: &'static str, reason: CheckedStringErrorReason) -> Self {
        Self { grammar, reason }
    }

    /// Returns the checked-string grammar that rejected input.
    pub const fn grammar(&self) -> &'static str {
        self.grammar
    }

    /// Returns the stable validation failure reason.
    pub const fn reason(&self) -> &CheckedStringErrorReason {
        &self.reason
    }
}

impl fmt::Display for CheckedStringErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty"),
            Self::TooLong { max } => write!(f, "too long; max {max} bytes"),
            Self::MissingSeparator { separator } => {
                write!(f, "missing required separator `{separator}`")
            }
            Self::EmptySegment => f.write_str("empty segment"),
            Self::SegmentTooLong { max } => write!(f, "segment too long; max {max} bytes"),
            Self::ReservedPrefix { prefix } => write!(f, "reserved prefix `{prefix}`"),
            Self::InvalidStart => f.write_str("invalid start character"),
            Self::InvalidEnd => f.write_str("invalid end character"),
            Self::InvalidCharacter { ch, index } => {
                write!(f, "invalid character `{ch}` at index {index}")
            }
        }
    }
}

impl From<CheckedStringError> for IdentityError {
    fn from(error: CheckedStringError) -> Self {
        Self::new(error.to_string())
    }
}

/// Returns a short stable display fragment from an id-like string.
///
/// The fragment is derived from the suffix after the final `:` separator, keeps
/// ASCII alphanumeric characters only, and is capped at `max_len` characters.
pub fn short_stable_id_fragment(value: &str, max_len: usize) -> String {
    value
        .rsplit(':')
        .next()
        .unwrap_or(value)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(max_len)
        .collect()
}

macro_rules! checked_string_type {
    ($ty:ident, $grammar:literal, $validator:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            #[doc = concat!("Creates a checked `", stringify!($ty), "`.")]
            pub fn new(value: impl AsRef<str>) -> CheckedStringResult<Self> {
                let value = value.as_ref();
                $validator($grammar, value)?;
                Ok(Self {
                    raw: value.to_owned(),
                })
            }

            #[doc = concat!("Returns this `", stringify!($ty), "` as a string slice.")]
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            #[doc = concat!("Consumes this `", stringify!($ty), "` into its string.")]
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
            type Err = CheckedStringError;

            fn from_str(value: &str) -> CheckedStringResult<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = CheckedStringError;

            fn try_from(value: String) -> CheckedStringResult<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $ty {
            type Error = CheckedStringError;

            fn try_from(value: &str) -> CheckedStringResult<Self> {
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

checked_string_type!(
    NameToken,
    "name token",
    validate_name_token,
    "Checked identity name/version token."
);

checked_string_type!(
    StableAuthorKey,
    "stable author key",
    validate_stable_author_key,
    "Stable author-provided key used as deterministic identity input."
);

checked_string_type!(
    StableId,
    "stable id",
    validate_stable_id,
    "Stable identifier using the exact primitive identifier grammar."
);

checked_string_type!(
    EntryPointId,
    "entry-point id",
    validate_entry_point_id,
    "Versioned public entry-point identifier using the exact identifier grammar."
);

checked_string_type!(
    AppendRequestId,
    "append request id",
    validate_stable_id,
    "Stable caller-selected idempotency identity for one append request."
);

checked_string_type!(
    FieldSegment,
    "field segment",
    validate_field_segment,
    "Checked nonempty segment used by a field path or typed source projection."
);

checked_string_type!(
    FieldPath,
    "field path",
    validate_field_path,
    "Checked dot-separated field path."
);

checked_string_type!(
    LocalPublicId,
    "local public id",
    validate_local_public_id,
    "Checked process-local public identifier."
);

checked_string_type!(
    RuntimeEnvName,
    "runtime env name",
    validate_runtime_env_name,
    "Checked runtime environment variable name."
);
