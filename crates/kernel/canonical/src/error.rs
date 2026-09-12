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
    /// Retains the original parser/serializer error without exposing its message.
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
        value.serialize_field("message", "withheld")?;
        value.end()
    }
}

/// Canonical grammar failure retaining available native parser sources.
#[derive(thiserror::Error)]
#[error("canonical processing failed")]
pub struct CanonicalError {
    message: String,
    #[source]
    source: Option<CanonicalSource>,
}
#[derive(Debug, thiserror::Error)]
enum CanonicalSource {
    #[error("JSON serialization exceeded its byte ceiling")]
    SerializationLimit {
        limit: usize,
        observed_at_least: usize,
        #[source]
        source: JsonError,
    },
    #[error(transparent)]
    Json(#[from] JsonError),
    #[error("invalid UTF-8")]
    Utf8(#[source] std::str::Utf8Error),
}
impl CanonicalError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }
    pub(crate) fn json(source: serde_json::Error) -> Self {
        Self {
            message: "invalid canonical JSON".into(),
            source: Some(CanonicalSource::Json(JsonError::new(source))),
        }
    }
    pub(crate) fn utf8(source: std::str::Utf8Error) -> Self {
        Self {
            message: "canonical JSON must be UTF-8".into(),
            source: Some(CanonicalSource::Utf8(source)),
        }
    }
    pub(crate) fn serialization_limit(
        limit: usize,
        observed_at_least: usize,
        source: serde_json::Error,
    ) -> Self {
        Self {
            message: "JSON serialization exceeds its byte ceiling".into(),
            source: Some(CanonicalSource::SerializationLimit {
                limit,
                observed_at_least,
                source: JsonError::new(source),
            }),
        }
    }
    /// Returns the ceiling and the observed lower bound when output accumulation stopped early.
    pub fn serialization_bound(&self) -> Option<(usize, usize)> {
        match &self.source {
            Some(CanonicalSource::SerializationLimit {
                limit,
                observed_at_least,
                ..
            }) => Some((*limit, *observed_at_least)),
            _ => None,
        }
    }
    /// Returns the existing owner diagnostic; transport projections withhold this text.
    pub fn message(&self) -> &str {
        &self.message
    }
}
impl fmt::Debug for CanonicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CanonicalError { message: withheld }")
    }
}
impl Serialize for CanonicalError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "snake_case")]
        enum Cause<'a> {
            SerializationLimit {
                limit: usize,
                observed_at_least: usize,
                source: &'a JsonError,
            },
            Json(&'a JsonError),
            Utf8 {
                valid_up_to: usize,
                error_len: Option<usize>,
            },
            Grammar {
                message: &'static str,
                message_bytes: usize,
            },
        }
        let cause = match &self.source {
            Some(CanonicalSource::SerializationLimit {
                limit,
                observed_at_least,
                source,
            }) => Cause::SerializationLimit {
                limit: *limit,
                observed_at_least: *observed_at_least,
                source,
            },
            Some(CanonicalSource::Json(source)) => Cause::Json(source),
            Some(CanonicalSource::Utf8(source)) => Cause::Utf8 {
                valid_up_to: source.valid_up_to(),
                error_len: source.error_len(),
            },
            None => Cause::Grammar {
                message: "withheld",
                message_bytes: self.message.len(),
            },
        };
        cause.serialize(serializer)
    }
}
