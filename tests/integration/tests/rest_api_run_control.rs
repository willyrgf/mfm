//! REST run-control contract tests for exact entry-point and target ingress.

#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use mfm_integration_tests::test_support::{empty_post, json_post, response_json};
use tower::ServiceExt;

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001";
const VALID_SCHEMA_ID: &str =
    "schema:mfm.test.public:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000002";
const UNKNOWN_ENTRY_POINT: &str = "mfm.unknown/missing@1";

fn test_app() -> axum::Router {
    mfm_rest_api::make_app(mfm_integration_tests::test_support::in_memory_rest_app_state())
}

async fn assert_error(response: axum::response::Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    let body = response_json(response).await;
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["code"], code);
}

#[tokio::test]
async fn health_and_ready_endpoints_report_liveness_and_readiness() {
    let app = test_app();
    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    let ready = app
        .oneshot(
            Request::builder()
                .uri("/v1/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let ready = response_json(ready).await;
    assert_eq!(ready["data"]["checks"]["run_store"], "ready");
}

#[tokio::test]
async fn start_requires_the_strict_entry_point_target_envelope() {
    let app = test_app();
    for body in [
        serde_json::json!({"entry_point": UNKNOWN_ENTRY_POINT}),
        serde_json::json!({
            "entry_point": UNKNOWN_ENTRY_POINT,
            "target": {"name": "acme/primary", "digest": "content:sha256-jcs-v1:deadbeef"},
        }),
        serde_json::json!({
            "entry_point": UNKNOWN_ENTRY_POINT,
            "request": {},
            "unexpected": true
        }),
    ] {
        let response = app
            .clone()
            .oneshot(json_post("/v1/runs/start", body))
            .await
            .unwrap();
        assert_error(response, StatusCode::BAD_REQUEST, "InvalidJson").await;
    }
}

#[tokio::test]
async fn start_requires_configuration_authority_after_a_valid_target_envelope() {
    let response = test_app()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({"entry_point": "mfm.unknown/missing@1", "target": "acme/primary"}),
        ))
        .await
        .unwrap();
    assert_error(
        response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "ConfiguredStoreUnavailable",
    )
    .await;
}

#[tokio::test]
async fn unknown_entry_points_do_not_bypass_configuration_authority() {
    let response = test_app()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({"entry_point": UNKNOWN_ENTRY_POINT, "target": "acme/primary"}),
        ))
        .await
        .unwrap();
    assert_error(
        response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "ConfiguredStoreUnavailable",
    )
    .await;
}

#[tokio::test]
async fn read_role_refuses_live_start() {
    let fixture = mfm_app::PublicFactFixtureForTest::new();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        role: mfm_rest_api::RestProcessRole::Read,
        store: fixture.store.clone(),
        configured_store: None,
        runtime_config_path: None,
    });
    let response = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({"entry_point": UNKNOWN_ENTRY_POINT, "target": "acme/primary"}),
        ))
        .await
        .unwrap();
    assert_error(
        response,
        StatusCode::SERVICE_UNAVAILABLE,
        "RestRoleReadOnly",
    )
    .await;
}

#[tokio::test]
async fn absent_run_routes_are_not_found() {
    let app = test_app();
    for request in [
        Request::builder()
            .method(Method::GET)
            .uri(format!("/v1/runs/{VALID_RUN_ID}/status"))
            .body(Body::empty())
            .unwrap(),
        empty_post(&format!("/v1/runs/{VALID_RUN_ID}/resume")),
        empty_post(&format!("/v1/runs/{VALID_RUN_ID}/replay")),
        Request::builder()
            .method(Method::GET)
            .uri(format!(
                "/v1/runs/{VALID_RUN_ID}/public-output/{VALID_SCHEMA_ID}"
            ))
            .body(Body::empty())
            .unwrap(),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn malformed_runtime_config_does_not_block_read_only_routes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runtime.toml");
    std::fs::write(&path, "not valid toml = [").unwrap();
    let mut state = mfm_integration_tests::test_support::in_memory_rest_app_state();
    state.runtime_config_path = Some(path);
    let app = mfm_rest_api::make_app(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn stream_validates_sequence_range_before_authoritative_read_and_filtering() {
    let source = include_str!("../../../bin/rest-api/src/lib.rs");
    let start = source.find("async fn runs_stream").unwrap();
    let end = source[start..]
        .find("async fn runs_replay")
        .map(|offset| start + offset)
        .unwrap();
    let body = &source[start..end];
    let range_validation = body
        .find("validate_sequence_range(query.from_seq, query.to_seq)")
        .unwrap();
    let authoritative_read = body.find(".run_stream(&run_id)").unwrap();
    let range_filter = body.find(".filter(|event|").unwrap();
    assert!(!body.contains(".load_run_stream("));
    assert!(range_validation < authoritative_read);
    assert!(authoritative_read < range_filter);
}
