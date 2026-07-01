use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001";
const VALID_SCHEMA_ID: &str =
    "schema:mfm.test.public:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000002";

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
    let _env_guard = ENV_LOCK.lock().await;
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

#[tokio::test]
async fn read_only_routes_ignore_malformed_runtime_config_env() {
    let _env = locked_env([(
        mfm_app::MFM_RUNTIME_CONFIG_FILE,
        "/definitely/not/runtime.toml",
    )])
    .await;
    let app = test_app();

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/runs")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("list response");
    assert_eq!(list.status(), StatusCode::OK);

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);

    let ready = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("ready response");
    assert_eq!(ready.status(), StatusCode::OK);

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/status"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("status response");
    assert_eq!(status.status(), StatusCode::NOT_FOUND);

    let stream = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/stream"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("stream response");
    assert_eq!(stream.status(), StatusCode::NOT_FOUND);

    let replay = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/replay"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::NOT_FOUND);

    let output = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{VALID_RUN_ID}/public-output/{VALID_SCHEMA_ID}"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("public output response");
    assert_eq!(output.status(), StatusCode::NOT_FOUND);
}

async fn locked_env<const N: usize>(pairs: [(&'static str, &str); N]) -> EnvGuard {
    let guard = ENV_LOCK.lock().await;
    let mut previous = Vec::new();
    for (key, value) in pairs {
        previous.push((key, std::env::var(key).ok()));
        set_env(key, value);
    }
    EnvGuard {
        _guard: guard,
        previous,
    }
}

struct EnvGuard {
    _guard: tokio::sync::MutexGuard<'static, ()>,
    previous: Vec<(&'static str, Option<String>)>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => set_env(key, value),
                None => remove_env(key),
            }
        }
    }
}

fn set_env(key: &str, value: &str) {
    // SAFETY: these tests serialize environment mutation through ENV_LOCK and
    // restore each variable before releasing that lock.
    unsafe { std::env::set_var(key, value) };
}

fn remove_env(key: &str) {
    // SAFETY: these tests serialize environment mutation through ENV_LOCK and
    // restore each variable before releasing that lock.
    unsafe { std::env::remove_var(key) };
}

fn test_app() -> axum::Router {
    make_app(AppState {
        store: store::AsyncInMemoryRunStore::default(),
        runtime_config_path: None,
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
