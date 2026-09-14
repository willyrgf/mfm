use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use std::fmt;

/// Native JSON failure with a reviewed category/location projection.
#[derive(thiserror::Error)]
#[error("JSON processing failed")]
pub struct JsonError {
    #[source]
    source: serde_json::Error,
}
impl JsonError {
    /// Retains the original parser/serializer error and its available diagnostic text.
    pub fn new(source: serde_json::Error) -> Self {
        Self { source }
    }
}
impl fmt::Debug for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JsonError { message: withheld }")
    }
}
impl Serialize for JsonError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serializer.serialize_struct("JsonError", 4)?;
        value.serialize_field(
            "category",
            &match self.source.classify() {
                serde_json::error::Category::Io => "io",
                serde_json::error::Category::Syntax => "syntax",
                serde_json::error::Category::Data => "data",
                serde_json::error::Category::Eof => "eof",
            },
        )?;
        value.serialize_field("line", &self.source.line())?;
        value.serialize_field("column", &self.source.column())?;
        value.serialize_field("message", &self.source.to_string())?;
        value.end()
    }
}

/// Canonical grammar or encoding failure retaining its concrete source.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalError {
    /// Canonical grammar rejection.
    #[error("{message}")]
    Grammar {
        /// Available rejection reason.
        message: String,
    },
    /// JSON parsing or encoding failed.
    #[error("invalid canonical JSON")]
    Json(#[source] JsonError),
    /// The input is not UTF-8.
    #[error("canonical JSON must be UTF-8")]
    Utf8(
        #[source]
        #[serde(serialize_with = "serialize_utf8")]
        std::str::Utf8Error,
    ),
    /// Serialization stopped at its actual accumulation ceiling.
    #[error("JSON serialization exceeds its byte ceiling")]
    SerializationLimit {
        /// Inclusive accumulation limit.
        limit: usize,
        /// Bytes observed before serialization stopped.
        observed_at_least: usize,
        /// Original serializer failure.
        #[source]
        source: JsonError,
    },
}
impl CanonicalError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self::Grammar {
            message: message.into(),
        }
    }
    pub(crate) fn json(source: serde_json::Error) -> Self {
        Self::Json(JsonError::new(source))
    }
    pub(crate) fn utf8(source: std::str::Utf8Error) -> Self {
        Self::Utf8(source)
    }
    pub(crate) fn serialization_limit(
        limit: usize,
        observed_at_least: usize,
        source: serde_json::Error,
    ) -> Self {
        Self::SerializationLimit {
            limit,
            observed_at_least,
            source: JsonError::new(source),
        }
    }
    /// Returns the serialization ceiling and observed lower bound when accumulation stopped.
    pub fn serialization_bound(&self) -> Option<(usize, usize)> {
        match self {
            Self::SerializationLimit {
                limit,
                observed_at_least,
                ..
            } => Some((*limit, *observed_at_least)),
            _ => None,
        }
    }
    /// Returns the owner's existing diagnostic message.
    pub fn message(&self) -> &str {
        match self {
            Self::Grammar { message } => message,
            Self::Json(_) => "invalid canonical JSON",
            Self::Utf8(_) => "canonical JSON must be UTF-8",
            Self::SerializationLimit { .. } => "JSON serialization exceeds its byte ceiling",
        }
    }
}
fn serialize_utf8<S: Serializer>(
    source: &std::str::Utf8Error,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut value = serializer.serialize_struct("Utf8Error", 2)?;
    value.serialize_field("valid_up_to", &source.valid_up_to())?;
    value.serialize_field("error_len", &source.error_len())?;
    value.end()
}
