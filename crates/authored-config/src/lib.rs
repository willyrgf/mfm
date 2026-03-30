#![warn(missing_docs)]
//! Shared helpers for config-driven workflow ingress.
//!
//! Transport boundaries stay responsible for reading bytes, but many workflow families need the
//! same small set of authored-config rules:
//!
//! - recognize JSON versus TOML from a path hint when available
//! - sniff a preferred format when the path is ambiguous
//! - fall back to the alternate format only when no explicit hint was given
//!
//! These helpers keep that behavior consistent across workflow-family config crates.

use std::fmt;
use std::path::Path;

/// Supported authored-config formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthoredConfigFormat {
    /// JSON-authored config.
    Json,
    /// TOML-authored config.
    Toml,
}

impl AuthoredConfigFormat {
    /// Returns the format implied by a path extension, when recognized.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("json") => Some(Self::Json),
            Some(ext) if ext.eq_ignore_ascii_case("toml") => Some(Self::Toml),
            _ => None,
        }
    }

    /// Returns the alternate authored format.
    pub fn alternate(self) -> Self {
        match self {
            Self::Json => Self::Toml,
            Self::Toml => Self::Json,
        }
    }
}

impl fmt::Display for AuthoredConfigFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json => f.write_str("json"),
            Self::Toml => f.write_str("toml"),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_prefers_json_for_object_shapes() {
        assert_eq!(
            detect_authored_config_format("  {\"k\":1}"),
            AuthoredConfigFormat::Json
        );
        assert_eq!(
            detect_authored_config_format(" \n [1,2,3]"),
            AuthoredConfigFormat::Json
        );
    }

    #[test]
    fn detect_defaults_to_toml_for_non_json_prefixes() {
        assert_eq!(
            detect_authored_config_format("title = \"demo\""),
            AuthoredConfigFormat::Toml
        );
        assert_eq!(
            detect_authored_config_format(""),
            AuthoredConfigFormat::Toml
        );
    }

    #[test]
    fn parse_with_hint_uses_fallback_only_without_extension() {
        let parsed = parse_authored_config_with_hint(
            "answer = 41",
            None,
            |_| Err("json"),
            |_| Ok::<_, &'static str>(41_u64),
        )
        .expect("fallback parse");
        assert_eq!(parsed, 41);
    }

    #[test]
    fn parse_with_hint_respects_explicit_extension() {
        let path = Path::new("config.json");
        let err = parse_authored_config_with_hint(
            "answer = 41",
            Some(path),
            |_| Err("json"),
            |_| Ok::<_, &'static str>(41_u64),
        )
        .expect_err("json hint should disable fallback");
        assert_eq!(err, "json");
    }
}
