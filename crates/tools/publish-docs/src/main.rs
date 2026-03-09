#![allow(clippy::disallowed_methods)]
use std::io::IsTerminal;

use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

const DEFAULT_FILTER: &str = "warn";
const ENV_MFM_LOG: &str = "MFM_LOG";
const ENV_MFM_LOG_FORMAT: &str = "MFM_LOG_FORMAT";
const ENV_MFM_LOG_SPAN_EVENTS: &str = "MFM_LOG_SPAN_EVENTS";
const ENV_LOG_LEVEL: &str = "LOG_LEVEL";
const ENV_LOG_FORMAT: &str = "LOG_FORMAT";
const ENV_LOG_SPAN_EVENTS: &str = "LOG_SPAN_EVENTS";
const ENV_RUST_LOG: &str = "RUST_LOG";

#[derive(Clone, Copy)]
enum LogFormat {
    Text,
    Json,
}

fn resolve_log_filter() -> String {
    for key in [ENV_MFM_LOG, ENV_LOG_LEVEL, ENV_RUST_LOG] {
        if let Ok(value) = std::env::var(key) {
            return value;
        }
    }
    DEFAULT_FILTER.to_string()
}

fn resolve_log_format() -> LogFormat {
    for key in [ENV_MFM_LOG_FORMAT, ENV_LOG_FORMAT] {
        if let Ok(value) = std::env::var(key) {
            if value.trim().eq_ignore_ascii_case("json") {
                return LogFormat::Json;
            }
        }
    }
    LogFormat::Text
}

fn resolve_span_events() -> FmtSpan {
    for key in [ENV_MFM_LOG_SPAN_EVENTS, ENV_LOG_SPAN_EVENTS] {
        if let Ok(value) = std::env::var(key) {
            return match value.trim().to_ascii_lowercase().as_str() {
                "new" => FmtSpan::NEW,
                "close" => FmtSpan::CLOSE,
                "active" => FmtSpan::ACTIVE,
                _ => FmtSpan::NONE,
            };
        }
    }
    FmtSpan::NONE
}

fn init_observability() {
    let filter =
        EnvFilter::try_new(resolve_log_filter()).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let text_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_span_events(resolve_span_events())
        .with_ansi(
            matches!(resolve_log_format(), LogFormat::Text) && std::io::stderr().is_terminal(),
        )
        .with_writer(std::io::stderr);

    let _ = match resolve_log_format() {
        LogFormat::Text => tracing_subscriber::registry()
            .with(filter)
            .with(text_layer.compact())
            .try_init(),
        LogFormat::Json => tracing_subscriber::registry()
            .with(filter)
            .with(text_layer.json().flatten_event(true))
            .try_init(),
    };
}

/// Binary entrypoint for the Phase-1 `publish-docs` tool.
#[tokio::main]
async fn main() -> std::process::ExitCode {
    init_observability();
    mfm_publish_docs::run().await
}
