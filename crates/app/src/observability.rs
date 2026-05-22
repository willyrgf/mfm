//! Observability setup shared by typed app binaries.

use std::fmt;

use serde::{Deserialize, Serialize};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

/// Observability format selected by environment or binary defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Human-readable compact logs.
    Text,
    /// JSON logs suitable for process supervision.
    Json,
}

/// Observability configuration used during process boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Logical service name.
    pub service_name: String,
    /// Trace/log filter expression.
    pub filter: String,
    /// Log encoding.
    pub format: LogFormat,
}

/// Observability setup error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservabilityError {
    /// Stable error code.
    pub code: String,
    /// Redaction-safe error message.
    pub message: String,
}

impl fmt::Display for ObservabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ObservabilityError {}

/// Builds observability config from environment variables.
#[allow(clippy::disallowed_methods)]
pub fn observability_from_env(service_name: impl Into<String>) -> ObservabilityConfig {
    observability_from_env_with_default(service_name, "info")
}

/// Builds observability config from environment variables with a fallback filter.
#[allow(clippy::disallowed_methods)]
pub fn observability_from_env_with_default(
    service_name: impl Into<String>,
    default_filter: &str,
) -> ObservabilityConfig {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| default_filter.to_owned());
    let format = match std::env::var("MFM_LOG_FORMAT")
        .unwrap_or_else(|_| "text".to_owned())
        .as_str()
    {
        "json" => LogFormat::Json,
        _ => LogFormat::Text,
    };
    ObservabilityConfig {
        service_name: service_name.into(),
        filter,
        format,
    }
}

/// Initializes process-global tracing using the supplied config.
pub fn init_observability(config: ObservabilityConfig) -> Result<(), ObservabilityError> {
    let filter = EnvFilter::try_new(config.filter.clone()).map_err(|error| ObservabilityError {
        code: "ObservabilityFilterInvalid".to_owned(),
        message: error.to_string(),
    })?;

    let registry = tracing_subscriber::registry().with(filter);
    match config.format {
        LogFormat::Text => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_span_events(FmtSpan::CLOSE)
                    .with_target(true)
                    .with_thread_ids(false),
            )
            .try_init()
            .map_err(|error| ObservabilityError {
                code: "ObservabilityInitFailed".to_owned(),
                message: error.to_string(),
            }),
        LogFormat::Json => registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_span_events(FmtSpan::CLOSE)
                    .with_target(true),
            )
            .try_init()
            .map_err(|error| ObservabilityError {
                code: "ObservabilityInitFailed".to_owned(),
                message: error.to_string(),
            }),
    }
}
