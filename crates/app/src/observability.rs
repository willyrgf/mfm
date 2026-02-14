use std::io::IsTerminal;

use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::{AppError, ErrorClass};

pub const ENV_MFM_LOG: &str = "MFM_LOG";
pub const ENV_MFM_LOG_FORMAT: &str = "MFM_LOG_FORMAT";
pub const ENV_MFM_LOG_SPAN_EVENTS: &str = "MFM_LOG_SPAN_EVENTS";
pub const ENV_RUST_LOG: &str = "RUST_LOG";

const DEFAULT_FILTER: &str = "warn,mfm=info,tower_http=info";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    pub app_name: &'static str,
    pub default_filter: String,
    pub filter: String,
    pub format: LogFormat,
    pub include_targets: bool,
    pub ansi: bool,
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

pub fn observability_from_env(app_name: &'static str) -> ObservabilityConfig {
    let default_filter = DEFAULT_FILTER.to_string();
    let filter = std::env::var(ENV_MFM_LOG)
        .or_else(|_| std::env::var(ENV_RUST_LOG))
        .unwrap_or_else(|_| default_filter.clone());

    let format = std::env::var(ENV_MFM_LOG_FORMAT)
        .map(|raw| parse_format(&raw))
        .unwrap_or(LogFormat::Text);

    let span_events = std::env::var(ENV_MFM_LOG_SPAN_EVENTS)
        .map(|raw| parse_span_events(&raw))
        .unwrap_or(FmtSpan::NONE);

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

pub fn init_observability(config: ObservabilityConfig) -> Result<(), AppError> {
    let filter = EnvFilter::try_new(config.filter.clone()).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "InvalidLogFilter",
            format!("invalid {} value", ENV_MFM_LOG),
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
