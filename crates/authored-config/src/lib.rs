#![warn(missing_docs)]
//! Shared authored-config ingress for public workflow entry points.
//!
//! Transport boundaries stay responsible for reading bytes, while this crate owns the shared
//! JSON/TOML intake rules used before a typed operation planner sees a config value.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentDigest, DigestAlgorithm};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default authored config format when callers do not provide one.
pub const DEFAULT_AUTHORED_CONFIG_FORMAT: AuthoredConfigFormat = AuthoredConfigFormat::Toml;

/// Maximum authored config payload size accepted before parsing.
pub const DEFAULT_AUTHORED_CONFIG_MAX_BYTES: usize = 256 * 1024;

/// Standard JSON/TOML format set accepted by current public entry points.
pub const TOML_JSON_AUTHORED_CONFIG_FORMATS: &[AuthoredConfigFormat] =
    &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json];

/// Supported authored-config formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredConfigFormat {
    /// TOML authored config.
    Toml,
    /// JSON authored config.
    Json,
}

impl AuthoredConfigFormat {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Json => "json",
        }
    }

    /// Returns the format implied by a path extension, when recognized.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("json") => Some(Self::Json),
            Some(ext) if ext.eq_ignore_ascii_case("toml") => Some(Self::Toml),
            _ => None,
        }
    }

    /// Returns the alternate authored format.
    pub const fn alternate(self) -> Self {
        match self {
            Self::Json => Self::Toml,
            Self::Toml => Self::Json,
        }
    }
}

impl fmt::Display for AuthoredConfigFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AuthoredConfigFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "toml" => Ok(Self::Toml),
            "json" => Ok(Self::Json),
            _ => Err("config format must be `toml` or `json`".to_owned()),
        }
    }
}

/// Public authored config bytes submitted to an entry-point operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredConfig {
    format: AuthoredConfigFormat,
    bytes: Vec<u8>,
    authored_digest: ContentDigest,
}

impl AuthoredConfig {
    /// Creates authored config using the default size limit.
    pub fn new(
        format: AuthoredConfigFormat,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, AuthoredConfigError> {
        Self::with_size_limit(format, bytes, DEFAULT_AUTHORED_CONFIG_MAX_BYTES)
    }

    /// Creates authored config using an explicit size limit.
    pub fn with_size_limit(
        format: AuthoredConfigFormat,
        bytes: impl Into<Vec<u8>>,
        max_bytes: usize,
    ) -> Result<Self, AuthoredConfigError> {
        let bytes = bytes.into();
        if bytes.len() > max_bytes {
            return Err(AuthoredConfigError::new(
                "AuthoredConfigTooLarge",
                "authored config exceeds the maximum accepted size",
            ));
        }
        let authored_digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
        Ok(Self {
            format,
            bytes,
            authored_digest,
        })
    }

    /// Creates authored config from a REST-style JSON `config` value.
    ///
    /// String config values default to TOML when no format is supplied. JSON objects or arrays
    /// imply JSON unless an explicit JSON format is supplied. TOML cannot be represented as a
    /// structured JSON config value.
    pub fn from_json_transport_value(
        explicit_format: Option<AuthoredConfigFormat>,
        value: &Value,
    ) -> Result<Self, AuthoredConfigError> {
        match value {
            Value::String(raw) => Self::new(
                explicit_format.unwrap_or(DEFAULT_AUTHORED_CONFIG_FORMAT),
                raw.as_bytes(),
            ),
            Value::Object(_) | Value::Array(_) => {
                let format = explicit_format.unwrap_or(AuthoredConfigFormat::Json);
                if format != AuthoredConfigFormat::Json {
                    return Err(AuthoredConfigError::new(
                        "AuthoredConfigFormatShapeMismatch",
                        "structured JSON config values require json config format",
                    ));
                }
                let bytes = serde_json::to_vec(value).map_err(|_| {
                    AuthoredConfigError::new(
                        "AuthoredConfigSerializeFailed",
                        "authored config could not be serialized",
                    )
                })?;
                Self::new(AuthoredConfigFormat::Json, bytes)
            }
            _ => Err(AuthoredConfigError::new(
                "AuthoredConfigShapeInvalid",
                "authored config must be a string or structured JSON value",
            )),
        }
    }

    /// Returns the authored config format.
    pub const fn format(&self) -> AuthoredConfigFormat {
        self.format
    }

    /// Returns the original authored config bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the digest of the original authored config bytes.
    pub fn authored_digest(&self) -> &ContentDigest {
        &self.authored_digest
    }

    /// Parses, validates, and canonicalizes this authored config into a typed value.
    pub fn normalize<T>(&self) -> Result<NormalizedAuthoredConfig<T>, AuthoredConfigError>
    where
        T: DeserializeOwned + Serialize,
    {
        normalize_authored_config(self)
    }
}

/// Typed config plus canonical evidence derived from authored config bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedAuthoredConfig<T> {
    /// Decoded typed config.
    pub value: T,
    /// Effective config format used to parse the authored bytes.
    pub format: AuthoredConfigFormat,
    /// Digest of the original authored bytes.
    pub authored_digest: ContentDigest,
    /// Canonical JSON bytes for the decoded typed config.
    pub canonical_json: PlainCanonicalJsonBytes,
    /// Digest of the canonical JSON bytes.
    pub canonical_digest: ContentDigest,
}

/// Static operation entry-point metadata exported by operation crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EntryPointDescriptor {
    /// Domain namespace for the typed entry-point operation.
    pub namespace: &'static str,
    /// Stable domain operation name within the namespace.
    pub name: &'static str,
    /// Public shorthand accepted by transports.
    pub public_name: &'static str,
    /// Public operation version.
    pub version: u32,
    /// Authored config formats accepted by this entry point.
    pub accepted_config_formats: &'static [AuthoredConfigFormat],
}

/// Error returned while creating or normalizing authored config.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AuthoredConfigError {
    code: String,
    message: String,
}

impl AuthoredConfigError {
    /// Creates a public-safe authored config error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Returns the stable machine-readable error code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the public-safe error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Detects the preferred authored format from the leading non-whitespace character.
pub fn detect_authored_config_format(raw: &str) -> AuthoredConfigFormat {
    match raw.chars().find(|ch| !ch.is_whitespace()) {
        Some('{') | Some('[') => AuthoredConfigFormat::Json,
        _ => AuthoredConfigFormat::Toml,
    }
}

/// Parses authored config with optional path-based format detection and fallback.
///
/// When `path_hint` has a recognized extension, only that format is attempted. Otherwise the
/// parser sniffs the preferred format from the leading non-whitespace character and falls back to
/// the alternate format only if the first attempt fails.
pub fn parse_authored_config_with_hint<T, E, FJson, FToml>(
    raw: &str,
    path_hint: Option<&Path>,
    parse_json: FJson,
    parse_toml: FToml,
) -> Result<T, E>
where
    FJson: Fn(&str) -> Result<T, E>,
    FToml: Fn(&str) -> Result<T, E>,
{
    let hinted = path_hint.and_then(AuthoredConfigFormat::from_path);
    let preferred = hinted.unwrap_or_else(|| detect_authored_config_format(raw));

    let parse_for = |format| match format {
        AuthoredConfigFormat::Json => parse_json(raw),
        AuthoredConfigFormat::Toml => parse_toml(raw),
    };

    match parse_for(preferred) {
        Ok(parsed) => Ok(parsed),
        Err(err) if hinted.is_some() => Err(err),
        Err(_) => parse_for(preferred.alternate()),
    }
}

fn normalize_authored_config<T>(
    authored: &AuthoredConfig,
) -> Result<NormalizedAuthoredConfig<T>, AuthoredConfigError>
where
    T: DeserializeOwned + Serialize,
{
    let submitted = parse_authored_value(authored)?;
    let value = serde_json::from_value::<T>(submitted.clone()).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigDecodeFailed",
            "authored config does not match the selected op schema",
        )
    })?;
    let normalized = serde_json::to_value(&value).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigSerializeFailed",
            "authored config could not be serialized",
        )
    })?;
    reject_unknown_submitted_fields(&submitted, &normalized)?;
    reject_json_floats(&normalized)?;
    let canonical_json = canonical_json_value(&normalized)?;
    let canonical_digest = canonical_json.content_digest();
    Ok(NormalizedAuthoredConfig {
        value,
        format: authored.format(),
        authored_digest: authored.authored_digest().clone(),
        canonical_json,
        canonical_digest,
    })
}

fn parse_authored_value(authored: &AuthoredConfig) -> Result<Value, AuthoredConfigError> {
    match authored.format() {
        AuthoredConfigFormat::Json => parse_json_authored_value(authored.bytes()),
        AuthoredConfigFormat::Toml => parse_toml_authored_value(authored.bytes()),
    }
}

fn parse_json_authored_value(bytes: &[u8]) -> Result<Value, AuthoredConfigError> {
    let raw = std::str::from_utf8(bytes).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigInvalidUtf8",
            "authored config must be UTF-8 text",
        )
    })?;
    let loose = serde_json::from_str::<Value>(raw).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigInvalidJson",
            "authored config is not valid JSON",
        )
    })?;
    reject_json_floats(&loose)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(raw).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigInvalidJson",
            "authored config is not valid duplicate-free canonicalizable JSON",
        )
    })?;
    json_value_from_canonical(canonical.as_bytes())
}

fn parse_toml_authored_value(bytes: &[u8]) -> Result<Value, AuthoredConfigError> {
    let raw = std::str::from_utf8(bytes).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigInvalidUtf8",
            "authored config must be UTF-8 text",
        )
    })?;
    let toml_value = raw.parse::<toml::Value>().map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigInvalidToml",
            "authored config is not valid TOML",
        )
    })?;
    let json_value = serde_json::to_value(toml_value).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigSerializeFailed",
            "authored config could not be serialized",
        )
    })?;
    reject_json_floats(&json_value)?;
    let canonical = canonical_json_value(&json_value)?;
    json_value_from_canonical(canonical.as_bytes())
}

fn canonical_json_value(value: &Value) -> Result<PlainCanonicalJsonBytes, AuthoredConfigError> {
    let json = serde_json::to_string(value).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigSerializeFailed",
            "authored config could not be serialized",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigCanonicalJsonFailed",
            "authored config could not be canonicalized",
        )
    })
}

fn json_value_from_canonical(bytes: &[u8]) -> Result<Value, AuthoredConfigError> {
    serde_json::from_slice(bytes).map_err(|_| {
        AuthoredConfigError::new(
            "AuthoredConfigCanonicalJsonFailed",
            "authored config could not be canonicalized",
        )
    })
}

fn reject_json_floats(value: &Value) -> Result<(), AuthoredConfigError> {
    match value {
        Value::Number(number) if !(number.is_i64() || number.is_u64()) => {
            Err(AuthoredConfigError::new(
                "AuthoredConfigFloatUnsupported",
                "authored config must not contain floating point numbers",
            ))
        }
        Value::Array(values) => {
            for value in values {
                reject_json_floats(value)?;
            }
            Ok(())
        }
        Value::Object(entries) => {
            for value in entries.values() {
                reject_json_floats(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn reject_unknown_submitted_fields(
    submitted: &Value,
    normalized: &Value,
) -> Result<(), AuthoredConfigError> {
    match (submitted, normalized) {
        (Value::Object(submitted), Value::Object(normalized)) => {
            for (key, submitted_value) in submitted {
                let Some(normalized_value) = normalized.get(key) else {
                    return Err(unknown_authored_config_field());
                };
                reject_unknown_submitted_fields(submitted_value, normalized_value)?;
            }
            Ok(())
        }
        (Value::Array(submitted), Value::Array(normalized)) => {
            if submitted.len() != normalized.len() {
                return Err(unknown_authored_config_field());
            }
            for (submitted_value, normalized_value) in submitted.iter().zip(normalized) {
                reject_unknown_submitted_fields(submitted_value, normalized_value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn unknown_authored_config_field() -> AuthoredConfigError {
    AuthoredConfigError::new(
        "AuthoredConfigUnknownField",
        "authored config contains fields outside the selected op schema",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct SimpleConfig {
        name: String,
        count: u64,
    }

    #[test]
    fn detect_authored_config_format_classifies_prefixes() {
        for (raw, expected) in [
            ("  {\"k\":1}", AuthoredConfigFormat::Json),
            (" \n [1,2,3]", AuthoredConfigFormat::Json),
            ("title = \"demo\"", AuthoredConfigFormat::Toml),
            ("", AuthoredConfigFormat::Toml),
        ] {
            assert_eq!(detect_authored_config_format(raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn parse_with_hint_uses_fallback_only_without_explicit_extension() {
        let without_extension = parse_authored_config_with_hint(
            "answer = 41",
            None,
            |_| Err("json"),
            |_| Ok::<_, &'static str>(41_u64),
        )
        .expect("fallback parse");
        assert_eq!(without_extension, 41);

        let json_hint = parse_authored_config_with_hint(
            "answer = 41",
            Some(Path::new("config.json")),
            |_| Err("json"),
            |_| Ok::<_, &'static str>(41_u64),
        )
        .expect_err("json hint should disable fallback");
        assert_eq!(json_hint, "json");
    }

    #[test]
    fn rest_config_values_default_to_transport_appropriate_formats() {
        for (value, expected) in [
            (
                serde_json::json!("name = \"demo\"\ncount = 2\n"),
                AuthoredConfigFormat::Toml,
            ),
            (
                serde_json::json!({"name":"demo","count":2}),
                AuthoredConfigFormat::Json,
            ),
        ] {
            let authored =
                AuthoredConfig::from_json_transport_value(None, &value).expect("authored");

            assert_eq!(authored.format(), expected);
        }
    }

    #[test]
    fn normalize_toml_config_to_canonical_json() {
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Toml, "name = \"demo\"\ncount = 2\n")
                .expect("authored");

        let normalized = authored.normalize::<SimpleConfig>().expect("normalized");

        assert_eq!(
            normalized.value,
            SimpleConfig {
                name: "demo".to_owned(),
                count: 2
            }
        );
        assert_eq!(normalized.format, AuthoredConfigFormat::Toml);
        assert_eq!(
            normalized.canonical_json.as_bytes(),
            br#"{"count":2,"name":"demo"}"#
        );
    }

    #[test]
    fn normalize_rejects_invalid_submissions_with_stable_error_codes() {
        enum Case {
            DuplicateJsonKeys,
            UnknownFields,
            Float,
        }

        for (case, expected_code) in [
            (Case::DuplicateJsonKeys, "AuthoredConfigInvalidJson"),
            (Case::UnknownFields, "AuthoredConfigUnknownField"),
            (Case::Float, "AuthoredConfigFloatUnsupported"),
        ] {
            let err = match case {
                Case::DuplicateJsonKeys => AuthoredConfig::new(
                    AuthoredConfigFormat::Json,
                    r#"{"name":"a","name":"b","count":1}"#,
                )
                .expect("authored")
                .normalize::<SimpleConfig>()
                .expect_err("duplicate key should reject"),
                Case::UnknownFields => AuthoredConfig::new(
                    AuthoredConfigFormat::Json,
                    r#"{"name":"demo","count":1,"extra":true}"#,
                )
                .expect("authored")
                .normalize::<SimpleConfig>()
                .expect_err("unknown field should reject"),
                Case::Float => {
                    AuthoredConfig::new(AuthoredConfigFormat::Json, r#"{"amount":1.25}"#)
                        .expect("authored")
                        .normalize::<serde_json::Value>()
                        .expect_err("float should reject")
                }
            };

            assert_eq!(err.code(), expected_code);
        }
    }

    #[test]
    fn authored_config_size_limit_is_enforced_before_parse() {
        let err = AuthoredConfig::with_size_limit(AuthoredConfigFormat::Toml, "name = \"demo\"", 4)
            .expect_err("size limit");

        assert_eq!(err.code(), "AuthoredConfigTooLarge");
    }

    #[test]
    fn normalize_rejects_invalid_toml_without_echoing_input() {
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Toml, "name = @secret").expect("authored");

        let err = authored
            .normalize::<SimpleConfig>()
            .expect_err("invalid TOML");

        assert_eq!(err.code(), "AuthoredConfigInvalidToml");
        assert!(!err.message().contains("secret"));
    }
}
