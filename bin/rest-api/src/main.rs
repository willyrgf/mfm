use std::convert::Infallible;
use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;
use tokio::net::TcpListener;

const ENV_ADDR: &str = "MFM_REST_API_ADDR";

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(&value).expect("json response must serialize");

    let mut resp = Response::new(Full::new(Bytes::from(body)));
    *resp.status_mut() = status;
    resp.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    resp
}

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

async fn route(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/v1/health") => Ok(json_response(StatusCode::OK, ok(json!({"ok": true})))),
        (&Method::GET, _) => Ok(json_response(
            StatusCode::NOT_FOUND,
            err("not_found", "not found"),
        )),
        _ => Ok(json_response(
            StatusCode::METHOD_NOT_ALLOWED,
            err("method_not_allowed", "method not allowed"),
        )),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let addr: SocketAddr = std::env::var(ENV_ADDR)
        .unwrap_or_else(|_| "127.0.0.1:3001".to_string())
        .parse()
        .map_err(|_| format!("invalid {ENV_ADDR} socket addr"))?;

    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutdown signal received");
    };
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accept = listener.accept() => {
                let (stream, peer_addr) = accept?;
                let io = TokioIo::new(stream);
                tokio::task::spawn(async move {
                    let service = service_fn(route);
                    if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                        tracing::debug!(%peer_addr, error = %err, "http connection error");
                    }
                });
            }
        }
    }

    Ok(())
}
