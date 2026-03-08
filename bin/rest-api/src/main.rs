#![allow(clippy::disallowed_methods)]
use std::net::SocketAddr;

use mfm_app::observability::{init_observability, observability_from_env};
use tokio::net::TcpListener;

const ENV_ADDR: &str = "MFM_REST_API_ADDR";

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_observability(observability_from_env("mfm_rest_api"))
        .map_err(|e| std::io::Error::other(format!("observability init failed: {}", e.message)))?;

    let addr: SocketAddr = std::env::var(ENV_ADDR)
        .unwrap_or_else(|_| "127.0.0.1:3001".to_string())
        .parse()
        .map_err(|_| format!("invalid {ENV_ADDR} socket addr"))?;

    let events = mfm_rest_api::make_default_event_store().await?;
    let artifacts = mfm_rest_api::make_default_artifact_store().await?;
    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events,
        artifacts,
    });

    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}
