#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001";
const VALID_SCHEMA_ID: &str =
    "schema:mfm.test.public:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000002";

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let s = serde_json::to_string(&body).expect("json request must serialize");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(s))
        .expect("request")
}

fn empty_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
}

fn test_app() -> axum::Router {
    let root = std::env::temp_dir().join(format!("mfm-rest-api-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("artifact root");
    let state = mfm_rest_api::make_in_memory_app_state(root);
    mfm_rest_api::make_app(state)
}

#[tokio::test]
async fn health_endpoint_reports_liveness() {
    let app = test_app();

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::OK);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "success");
    assert_eq!(v["data"]["ok"], true);
}

#[tokio::test]
async fn ready_endpoint_reports_typed_store_readiness() {
    let app = test_app();

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::OK);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "success");
    assert_eq!(v["data"]["ok"], true);
    assert_eq!(v["data"]["checks"]["typed_run_store"], "ready");
    assert_eq!(v["data"]["checks"]["typed_artifact_store"], "ready");
}

#[tokio::test]
async fn removed_dynamic_feature_and_artifact_routes_are_not_found() {
    let app = test_app();

    let features = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/features")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("features response");
    assert_eq!(features.status(), StatusCode::NOT_FOUND);

    let artifacts = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/artifacts/0000000000000000000000000000000000000000000000000000000000000000")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("artifact response");
    assert_eq!(artifacts.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn start_rejects_invalid_json_with_stable_envelope() {
    let app = test_app();

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs/start")
                .header("content-type", "application/json")
                .body(Body::from("not json"))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "InvalidJson");
}

#[tokio::test]
async fn start_rejects_dynamic_single_op_payloads() {
    let app = test_app();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "single_op_start_v1",
                "op_id": "proof",
                "op_version": "v1",
                "op_config": {}
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "InvalidJson");
}

#[tokio::test]
async fn start_parses_certified_bundle_and_rejects_invalid_spec() {
    let app = test_app();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "typed_run_start_v1",
                "run_id": VALID_RUN_ID,
                "bundle": {
                    "kind": "certified_typed_spec_bundle_v1",
                    "spec": {},
                    "certificate": {}
                },
                "drive": "append_only"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "TypedCertificationFailed");

    let status = app
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
}

#[tokio::test]
async fn start_rejects_invalid_certified_bundle_before_stream_creation() {
    let app = test_app();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "typed_run_start_v1",
                "run_id": VALID_RUN_ID,
                "bundle": {},
                "drive": "append_only"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "TypedBundleInvalid");

    let status = app
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
}

#[tokio::test]
async fn status_invalid_run_id_is_typed_error() {
    let app = test_app();

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/runs/not-a-uuid/status")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "InvalidRunId");
}

#[tokio::test]
async fn absent_typed_run_status_resume_replay_and_public_output_are_not_found() {
    let app = test_app();

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

    let resume = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{VALID_RUN_ID}/resume")))
        .await
        .expect("resume response");
    assert_eq!(resume.status(), StatusCode::NOT_FOUND);

    let replay = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{VALID_RUN_ID}/replay")))
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

#[tokio::test]
async fn stream_validates_sequence_range_before_reading() {
    let app = test_app();

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{VALID_RUN_ID}/stream?from_seq=3&to_seq=2"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "InvalidSequenceRange");
}
