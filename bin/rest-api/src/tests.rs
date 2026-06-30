use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

struct FailingSerialize;

impl Serialize for FailingSerialize {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("boom"))
    }
}

#[test]
fn serialize_response_returns_api_error_instead_of_panicking() {
    let err = serialize_response(FailingSerialize)
        .expect_err("serialization failures should be returned as api errors");

    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err.code, "SerializationError");
    assert_eq!(err.message, "Failed to serialize response payload");
}

#[test]
fn invalid_run_id_has_domain_error() {
    let err = parse_run_id("not-a-uuid").expect_err("dynamic ids are rejected");

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.code, "InvalidRunId");
}

#[tokio::test]
async fn run_start_accepts_entry_point_shape() {
    let response = test_app()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "op": "missing_entry_point_op",
                "config": "portfolio_id = \"main\"\n"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = response_json(response).await;
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "EntryPointOpNotFound");
}

#[test]
fn run_routes_do_not_import_dynamic_semantic_surfaces() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("read rest source");
    let forbidden = [
        format!("mfm_{}", "sdk"),
        format!("mfm_{}", "machine"),
        format!("mfm_app_{}", "legacy"),
        format!("Runs{}Request", "Start"),
        format!("{}line", "Pipe"),
        format!("Port{}", "Key"),
        format!("Dyn{}", "Context"),
        format!("State{}", "Graph"),
        format!("Dependency{}", "Edge"),
    ];

    for needle in forbidden {
        assert!(
            !source.contains(&needle),
            "REST typed run surface must not mention dynamic semantic surface `{needle}`"
        );
    }
}

fn test_app() -> axum::Router {
    make_app(AppState {
        store: store::AsyncInMemoryRunStore::default(),
    })
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("response json")
}
