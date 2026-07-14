use super::*;
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request};
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
                "entry_point": "mfm.unknown/missing@1",
                "request": {}
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = response_json(response).await;
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "EntryPointNotFound");
}

#[tokio::test]
async fn read_role_refuses_live_start_and_serves_public_fact_queries() {
    let fixture = mfm_app::PublicFactVisibilityFixtureForTest::new();
    let app = make_app(AppState {
        role: RestProcessRole::Read,
        store: fixture.store.clone(),
        catalog_store: None,
        runtime_config_path: None,
        fact_index: mfm_app::ProjectionFactIndexProvider::empty_arc(),
    });

    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/runs/start")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"entry_point":"mfm.portfolio/portfolio_snapshot@1","request":{}}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("start response");
    assert_eq!(start.status(), StatusCode::SERVICE_UNAVAILABLE);
    let start_body = response_json(start).await;
    assert_eq!(start_body["error"]["code"], "RestRoleReadOnly");

    let facts = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/facts/kinds")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("facts response");
    assert_eq!(facts.status(), StatusCode::OK);
    let facts_body = response_json(facts).await;
    assert_eq!(facts_body["status"], "success");
    assert_eq!(facts_body["data"][0]["fact_kind"], fixture.fact_kind);
}

#[tokio::test]
async fn live_routes_reuse_cached_services_after_first_construction() {
    let dir = test_temp_dir("live-routes-cache");
    let config_path = dir.path().join("runtime.toml");
    std::fs::write(&config_path, "").expect("write initial runtime config");
    let app = make_app(AppState {
        role: RestProcessRole::Live,
        store: store::AsyncInMemoryRunStore::default(),
        catalog_store: None,
        runtime_config_path: Some(config_path.clone()),
        fact_index: mfm_app::ProjectionFactIndexProvider::empty_arc(),
    });

    assert_entry_point_not_found(&app).await;

    std::fs::write(&config_path, "not valid toml = [").expect("replace runtime config");

    assert_entry_point_not_found(&app).await;
}

#[tokio::test]
async fn read_only_routes_ignore_malformed_runtime_config_env() {
    let _env = locked_env([(
        mfm_app::MFM_RUNTIME_CONFIG_FILE,
        "/definitely/not/runtime.toml",
    )])
    .await;
    let app = test_app();

    let routes = [
        (Method::GET, "/v1/runs".to_owned(), StatusCode::OK),
        (Method::GET, "/v1/health".to_owned(), StatusCode::OK),
        (Method::GET, "/v1/ready".to_owned(), StatusCode::OK),
        (
            Method::GET,
            format!("/v1/runs/{VALID_RUN_ID}/status"),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::GET,
            format!("/v1/runs/{VALID_RUN_ID}/stream"),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::POST,
            format!("/v1/runs/{VALID_RUN_ID}/replay"),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::GET,
            format!("/v1/runs/{VALID_RUN_ID}/public-output/{VALID_SCHEMA_ID}"),
            StatusCode::NOT_FOUND,
        ),
        (Method::GET, "/v1/facts/kinds".to_owned(), StatusCode::OK),
        (
            Method::GET,
            "/v1/facts/kinds/mfm.rest.test.fact".to_owned(),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::GET,
            "/v1/facts/mfm.rest.test.fact?order=result.amount.asc&field=result.amount&limit=1"
                .to_owned(),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::GET,
            "/v1/facts/mfm.rest.test.fact/latest?order=result.amount.asc&field=result.amount"
                .to_owned(),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::GET,
            format!("/v1/facts/ref/{}", unknown_public_ref()),
            StatusCode::NOT_FOUND,
        ),
    ];
    for (method, uri, status) in routes {
        assert_response_status(&app, request(method, uri), status).await;
    }
}

#[tokio::test]
async fn facts_routes_are_available_and_return_json_envelopes() {
    let app = test_app();

    let value = get_json(&app, "/v1/facts/kinds", StatusCode::OK).await;
    assert_eq!(value["status"], "success");
    assert_eq!(value["data"], json!([]));

    let value = get_json(
        &app,
        "/v1/facts/kinds/mfm.rest.test.fact",
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "FactNotFound");

    let value = get_json(
        &app,
        &format!("/v1/facts/ref/{}", unknown_public_ref()),
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_eq!(value["error"]["code"], "FactNotFound");
}

#[tokio::test]
async fn fact_query_routes_reject_malformed_query_shapes() {
    let app = test_app();

    let cases = [
        (
            "/v1/facts/mfm.rest.test.fact?order=result.amount.asc&field=result.amount&limit=nope",
            "InvalidQuery",
        ),
        (
            "/v1/facts/mfm.rest.test.fact?field=result.amount",
            "FactOrderingMissing",
        ),
        (
            "/v1/facts/mfm.rest.test.fact?order=result.amount.asc",
            "FactReturnFieldMissing",
        ),
        (
            "/v1/facts/mfm.rest.test.fact/latest?order=result.amount.asc&field=result.amount&subject=broken",
            "FactPredicateInvalid",
        ),
    ];
    for (uri, code) in cases {
        let value = get_json(&app, uri, StatusCode::BAD_REQUEST).await;
        assert_eq!(value["error"]["code"], code);
    }
}

#[tokio::test]
async fn facts_routes_expose_only_public_platform_projection_data() {
    let fixture = mfm_app::PublicFactVisibilityFixtureForTest::new();
    let app = make_app(AppState {
        role: RestProcessRole::Live,
        store: fixture.store.clone(),
        catalog_store: None,
        runtime_config_path: None,
        fact_index: mfm_app::ProjectionFactIndexProvider::empty_arc(),
    });

    let value = get_json(&app, "/v1/facts/kinds", StatusCode::OK).await;
    assert_eq!(value["data"][0]["fact_kind"], fixture.fact_kind);
    assert_eq!(value["data"][0]["descriptor_count"], 1);
    fixture.assert_json_redacts_private_tokens(&value);

    let describe_uri = format!("/v1/facts/kinds/{}", fixture.fact_kind);
    let value = get_json(&app, &describe_uri, StatusCode::OK).await;
    assert_eq!(value["data"][0]["fields"].as_array().unwrap().len(), 2);
    fixture.assert_json_redacts_private_tokens(&value);

    let query_uri = format!(
        "/v1/facts/{}?shape={}&order=result.amount.asc&field=subject.account&field=result.amount&subject=account%3Dpublic-account&result=amount.gte%3Du64%3A10&limit=10",
        fixture.fact_kind, fixture.shape
    );
    let value = get_json(&app, &query_uri, StatusCode::OK).await;
    let facts = value["data"]["facts"].as_array().expect("facts array");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["public_ref"], fixture.platform_public_ref);
    assert_eq!(facts[0]["fields"].as_array().unwrap().len(), 2);
    fixture.assert_json_redacts_private_tokens(&value);

    let latest_uri = format!(
        "/v1/facts/{}/latest?shape={}&order=result.amount.asc&field=result.amount&limit=99",
        fixture.fact_kind, fixture.shape
    );
    let value = get_json(&app, &latest_uri, StatusCode::OK).await;
    assert_eq!(value["data"]["facts"].as_array().unwrap().len(), 1);
    fixture.assert_json_redacts_private_tokens(&value);

    let platform_ref_uri = format!("/v1/facts/ref/{}", fixture.platform_public_ref);
    let value = get_json(&app, &platform_ref_uri, StatusCode::OK).await;
    assert_eq!(value["data"]["public_ref"], fixture.platform_public_ref);
    fixture.assert_json_redacts_private_tokens(&value);

    let control_ref_uri = format!("/v1/facts/ref/{}", fixture.control_public_ref);
    let control_error = get_json(&app, &control_ref_uri, StatusCode::NOT_FOUND).await;
    assert_eq!(control_error["error"]["code"], "FactNotFound");
    fixture.assert_json_redacts_private_tokens(&control_error);

    let unknown_ref_uri = format!("/v1/facts/ref/{}", unknown_public_ref());
    let unknown_error = get_json(&app, &unknown_ref_uri, StatusCode::NOT_FOUND).await;
    assert_eq!(unknown_error["error"], control_error["error"]);
}

async fn assert_entry_point_not_found(app: &axum::Router) {
    let response = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "entry_point": "mfm.unknown/missing@1",
                "request": {}
            }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = response_json(response).await;
    assert_eq!(value["error"]["code"], "EntryPointNotFound");
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

struct TestTempDir {
    path: std::path::PathBuf,
}

impl TestTempDir {
    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn test_temp_dir(name: &str) -> TestTempDir {
    let unique = format!(
        "mfm-rest-api-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir(&path).expect("create temp dir");
    TestTempDir { path }
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
        role: RestProcessRole::Live,
        store: store::AsyncInMemoryRunStore::default(),
        catalog_store: None,
        runtime_config_path: None,
        fact_index: mfm_app::ProjectionFactIndexProvider::empty_arc(),
    })
}

fn request(method: Method, uri: impl AsRef<str>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .body(Body::empty())
        .expect("request")
}

fn get(uri: &str) -> Request<Body> {
    request(Method::GET, uri)
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn assert_response_status(
    app: &axum::Router,
    request: Request<Body>,
    status: StatusCode,
) -> axum::response::Response {
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), status);
    response
}

async fn get_json(app: &axum::Router, uri: &str, status: StatusCode) -> serde_json::Value {
    response_json(assert_response_status(app, get(uri), status).await).await
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("response json")
}

fn unknown_public_ref() -> String {
    format!("pfr_{}", "a".repeat(64))
}
