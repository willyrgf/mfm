use super::*;
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request};
use axum::response::IntoResponse;
use tower::ServiceExt;

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001";
const VALID_SCHEMA_ID: &str =
    "schema:mfm.test.public:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000002";

struct FailingSerialize;

#[test]
fn rest_surface_has_no_secret_bearing_keystore_ingress() {
    let source = include_str!("lib.rs");
    for forbidden in [
        "/v1/keystore",
        "SecretInput",
        "KeystoreImportRequest",
        "prepare_import_keystore_access",
    ] {
        assert!(
            !source.contains(forbidden),
            "REST surface must not admit secret-bearing keystore ingress: {forbidden}"
        );
    }
}

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

    assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err.public_error().code, "SerializationError");
    assert_eq!(
        err.public_error().message,
        "Failed to serialize response payload"
    );
}

#[test]
fn invalid_run_id_has_domain_error() {
    let err = parse_run_id("not-a-uuid").expect_err("dynamic ids are rejected");

    assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    assert_eq!(err.public_error().code, "InvalidRunId");
}

#[tokio::test]
async fn runtime_config_error_keeps_shared_payload_without_cli_syntax() {
    let mut public: PublicError = serde_json::from_value(json!({
        "code": "RuntimeConfigRequired",
        "message": "test/primary requires EVM runtime routes",
        "diagnostics": [{
            "provider_family": "evm",
            "code": "provider_configuration_missing",
            "operation": null,
            "fields": {}
        }]
    }))
    .expect("shared public error JSON");
    public.class = ErrorClass::ServiceUnavailable;

    let response = ApiError::from(public).into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let value = response_json(response).await;
    assert_eq!(value["error"]["code"], "RuntimeConfigRequired");
    assert_eq!(
        value["error"]["message"],
        "test/primary requires EVM runtime routes"
    );
    assert_eq!(value["error"]["diagnostics"][0]["provider_family"], "evm");
    assert!(!value["error"]["message"]
        .as_str()
        .expect("message")
        .contains("--runtime-config"));
}

#[tokio::test]
async fn run_start_accepts_entry_point_and_target_shape_before_configuration_lookup() {
    let response = test_app()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "entry_point": "mfm.unknown/missing@1",
                "target": "acme/primary"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let value = response_json(response).await;
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], "ConfiguredStoreUnavailable");
}

#[tokio::test]
async fn run_start_recognizes_the_snapshot_entry_point_before_target_resolution() {
    let response = test_app()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "entry_point": "mfm.portfolio/snapshot@2",
                "target": "acme/primary"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let value = response_json(response).await;
    assert_eq!(value["error"]["code"], "ConfiguredStoreUnavailable");
}

#[tokio::test]
async fn read_role_refuses_live_start_and_serves_public_fact_queries() {
    let fixture = mfm_app::PublicFactFixtureForTest::new();
    let app = make_app(AppState {
        role: RestProcessRole::Read,
        store: fixture.store.clone(),
        configured_store: None,
        runtime_config_path: None,
    });

    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/runs/start")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"entry_point":"mfm.unknown/missing@1","target":"acme/primary"}"#,
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
        configured_store: None,
        runtime_config_path: Some(config_path.clone()),
    });

    assert_start_requires_configured_store(&app).await;

    std::fs::write(&config_path, "not valid toml = [").expect("replace runtime config");

    assert_start_requires_configured_store(&app).await;
}

#[tokio::test]
async fn read_only_routes_ignore_an_explicit_malformed_runtime_config() {
    let app = make_app(AppState {
        role: RestProcessRole::Live,
        store: store::AsyncInMemoryRunStore::default(),
        configured_store: None,
        runtime_config_path: Some(PathBuf::from("/definitely/not/runtime.toml")),
    });

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
async fn facts_routes_expose_public_projection_data() {
    let fixture = mfm_app::PublicFactFixtureForTest::new();
    let app = make_app(AppState {
        role: RestProcessRole::Live,
        store: fixture.store.clone(),
        configured_store: None,
        runtime_config_path: None,
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
    assert_eq!(facts[0]["public_ref"], fixture.public_ref);
    assert_eq!(facts[0]["fields"].as_array().unwrap().len(), 2);
    fixture.assert_json_redacts_private_tokens(&value);

    let latest_uri = format!(
        "/v1/facts/{}/latest?shape={}&order=result.amount.asc&field=result.amount&limit=99",
        fixture.fact_kind, fixture.shape
    );
    let value = get_json(&app, &latest_uri, StatusCode::OK).await;
    assert_eq!(value["data"]["facts"].as_array().unwrap().len(), 1);
    fixture.assert_json_redacts_private_tokens(&value);

    let public_ref_uri = format!("/v1/facts/ref/{}", fixture.public_ref);
    let value = get_json(&app, &public_ref_uri, StatusCode::OK).await;
    assert_eq!(value["data"]["public_ref"], fixture.public_ref);
    fixture.assert_json_redacts_private_tokens(&value);

    let unknown_ref_uri = format!("/v1/facts/ref/{}", unknown_public_ref());
    let unknown_error = get_json(&app, &unknown_ref_uri, StatusCode::NOT_FOUND).await;
    assert_eq!(unknown_error["error"]["code"], "FactNotFound");
    fixture.assert_json_redacts_private_tokens(&unknown_error);
}

async fn assert_start_requires_configured_store(app: &axum::Router) {
    let response = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "entry_point": "mfm.unknown/missing@1",
                "target": "acme/primary"
            }),
        ))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let value = response_json(response).await;
    assert_eq!(value["error"]["code"], "ConfiguredStoreUnavailable");
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

fn test_app() -> axum::Router {
    make_app(AppState {
        role: RestProcessRole::Live,
        store: store::AsyncInMemoryRunStore::default(),
        configured_store: None,
        runtime_config_path: None,
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
