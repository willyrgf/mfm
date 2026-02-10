use std::net::SocketAddr;

use axum::http::StatusCode;
use axum::routing::get;
use axum::Json;
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;

const ENV_ADDR: &str = "MFM_REST_API_ADDR";

fn ok(data: serde_json::Value) -> serde_json::Value {
    json!({ "status": "success", "data": data })
}

fn err(code: &'static str, message: &'static str) -> serde_json::Value {
    json!({
        "status": "error",
        "error": {
            "code": code,
            "message": message,
        }
    })
}

async fn health() -> Json<serde_json::Value> {
    Json(ok(json!({ "ok": true })))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(err("not_found", "not found")))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let addr: SocketAddr = std::env::var(ENV_ADDR)
        .unwrap_or_else(|_| "127.0.0.1:3001".to_string())
        .parse()
        .map_err(|_| format!("invalid {ENV_ADDR} socket addr"))?;

    let app = Router::new()
        .route("/v1/health", get(health))
        .fallback(not_found);

    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}
