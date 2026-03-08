//! Shared observability helpers for application-facing binaries.
//!
//! The CLI and REST API use this module to resolve environment-driven tracing configuration while
//! preserving the repository-wide logging contract: logs on stderr, stable payloads on stdout, and
//! compatibility support for both canonical and legacy environment variables.

use std::io::IsTerminal;

use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::{AppError, ErrorClass};

/// Legacy component-specific log filter override.
pub const ENV_MFM_LOG: &str = "MFM_LOG";
/// Legacy component-specific log format override.
pub const ENV_MFM_LOG_FORMAT: &str = "MFM_LOG_FORMAT";
/// Legacy component-specific span-event override.
pub const ENV_MFM_LOG_SPAN_EVENTS: &str = "MFM_LOG_SPAN_EVENTS";
/// Canonical baseline log filter override.
pub const ENV_LOG_LEVEL: &str = "LOG_LEVEL";
/// Canonical log format selector.
pub const ENV_LOG_FORMAT: &str = "LOG_FORMAT";
/// Canonical span lifecycle selector.
pub const ENV_LOG_SPAN_EVENTS: &str = "LOG_SPAN_EVENTS";
/// Standard Rust fallback log filter override.
pub const ENV_RUST_LOG: &str = "RUST_LOG";

const DEFAULT_FILTER: &str = "warn,mfm=info,tower_http=info";

/// Supported log output encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Compact human-readable text logs.
    Text,
    /// Structured JSON logs.
    Json,
}

/// Fully resolved observability settings for an application process.
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Application name used in initialization errors.
    pub app_name: &'static str,
    /// Default filter used when no environment override is present.
    pub default_filter: String,
    /// Effective filter resolved from the environment.
    pub filter: String,
    /// Effective output format.
    pub format: LogFormat,
    /// Whether log target names should be included.
    pub include_targets: bool,
    /// Whether ANSI colors should be emitted.
    pub ansi: bool,
    /// Span lifecycle events to emit.
    pub span_events: FmtSpan,
}

fn parse_format(raw: &str) -> LogFormat {
    match raw.trim().to_ascii_lowercase().as_str() {
        "json" => LogFormat::Json,
        _ => LogFormat::Text,
    }
}

fn parse_span_events(raw: &str) -> FmtSpan {
    match raw.trim().to_ascii_lowercase().as_str() {
        "new" => FmtSpan::NEW,
        "close" => FmtSpan::CLOSE,
        "active" => FmtSpan::ACTIVE,
        _ => FmtSpan::NONE,
    }
}

fn resolve_log_filter<F>(default_filter: &str, mut lookup: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    for key in [ENV_MFM_LOG, ENV_LOG_LEVEL, ENV_RUST_LOG] {
        if let Some(value) = lookup(key) {
            return value;
        }
    }
    default_filter.to_string()
}

fn resolve_log_format<F>(mut lookup: F) -> LogFormat
where
    F: FnMut(&str) -> Option<String>,
{
    for key in [ENV_MFM_LOG_FORMAT, ENV_LOG_FORMAT] {
        if let Some(raw) = lookup(key) {
            return parse_format(&raw);
        }
    }
    LogFormat::Text
}

fn resolve_log_span_events<F>(mut lookup: F) -> FmtSpan
where
    F: FnMut(&str) -> Option<String>,
{
    for key in [ENV_MFM_LOG_SPAN_EVENTS, ENV_LOG_SPAN_EVENTS] {
        if let Some(raw) = lookup(key) {
            return parse_span_events(&raw);
        }
    }
    FmtSpan::NONE
}

/// Resolves observability settings from environment variables using the default filter.
pub fn observability_from_env(app_name: &'static str) -> ObservabilityConfig {
    observability_from_env_with_default(app_name, DEFAULT_FILTER)
}

/// Resolves observability settings from environment variables with an explicit default filter.
pub fn observability_from_env_with_default(
    app_name: &'static str,
    default_filter: &str,
) -> ObservabilityConfig {
    let default_filter = default_filter.to_string();
    let filter = resolve_log_filter(&default_filter, |name| std::env::var(name).ok());
    let format = resolve_log_format(|name| std::env::var(name).ok());
    let span_events = resolve_log_span_events(|name| std::env::var(name).ok());

    let ansi = matches!(format, LogFormat::Text) && std::io::stderr().is_terminal();

    ObservabilityConfig {
        app_name,
        default_filter,
        filter,
        format,
        include_targets: true,
        ansi,
        span_events,
    }
}

/// Installs the process-wide tracing subscriber from the provided configuration.
pub fn init_observability(config: ObservabilityConfig) -> Result<(), AppError> {
    let filter = EnvFilter::try_new(config.filter.clone()).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "InvalidLogFilter",
            format!(
                "invalid log filter value (supported env vars: {}, {}, {})",
                ENV_MFM_LOG, ENV_LOG_LEVEL, ENV_RUST_LOG
            ),
        )
    })?;

    let text_layer = tracing_subscriber::fmt::layer()
        .with_target(config.include_targets)
        .with_span_events(config.span_events)
        .with_ansi(config.ansi)
        .with_writer(std::io::stderr);

    let init_result = match config.format {
        LogFormat::Text => tracing_subscriber::registry()
            .with(filter)
            .with(text_layer.compact())
            .try_init(),
        LogFormat::Json => tracing_subscriber::registry()
            .with(filter)
            .with(text_layer.json().flatten_event(true))
            .try_init(),
    };

    if let Err(err) = init_result {
        let msg = err.to_string();
        if msg.contains("already been set") {
            return Ok(());
        }
        return Err(AppError::new(
            ErrorClass::Internal,
            "ObservabilityInitFailed",
            format!("failed to initialize observability for {}", config.app_name),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup_from(entries: &[(&str, &str)]) -> impl FnMut(&str) -> Option<String> {
        let vars: HashMap<String, String> = entries
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        move |name| vars.get(name).cloned()
    }

    #[test]
    fn filter_resolution_prefers_component_overrides_before_global_level() {
        let filter = resolve_log_filter(
            "warn,mfm=info",
            lookup_from(&[
                (ENV_LOG_LEVEL, "warn"),
                (ENV_RUST_LOG, "debug"),
                (ENV_MFM_LOG, "trace,mfm=trace"),
            ]),
        );

        assert_eq!(filter, "trace,mfm=trace");
    }

    #[test]
    fn filter_resolution_uses_log_level_before_rust_log() {
        let filter = resolve_log_filter(
            "warn,mfm=info",
            lookup_from(&[(ENV_LOG_LEVEL, "info"), (ENV_RUST_LOG, "debug")]),
        );

        assert_eq!(filter, "info");
    }

    #[test]
    fn filter_resolution_falls_back_to_default() {
        let filter = resolve_log_filter("warn,mfm=info", lookup_from(&[]));
        assert_eq!(filter, "warn,mfm=info");
    }

    #[test]
    fn format_resolution_supports_global_alias() {
        let format = resolve_log_format(lookup_from(&[(ENV_LOG_FORMAT, "json")]));
        assert_eq!(format, LogFormat::Json);
    }

    #[test]
    fn format_resolution_prefers_legacy_override() {
        let format = resolve_log_format(lookup_from(&[
            (ENV_LOG_FORMAT, "text"),
            (ENV_MFM_LOG_FORMAT, "json"),
        ]));
        assert_eq!(format, LogFormat::Json);
    }

    #[test]
    fn span_event_resolution_supports_global_alias() {
        let span_events = resolve_log_span_events(lookup_from(&[(ENV_LOG_SPAN_EVENTS, "active")]));
        assert_eq!(span_events, FmtSpan::ACTIVE);
    }

    #[test]
    fn span_event_resolution_prefers_legacy_override() {
        let span_events = resolve_log_span_events(lookup_from(&[
            (ENV_LOG_SPAN_EVENTS, "new"),
            (ENV_MFM_LOG_SPAN_EVENTS, "close"),
        ]));
        assert_eq!(span_events, FmtSpan::CLOSE);
    }
}
