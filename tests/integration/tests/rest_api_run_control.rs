#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_spec::v1 as spec;
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

fn certified_bundle_json(spec_bytes: &[u8], certificate_bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "kind": "certified_typed_spec_bundle_v1",
        "spec": serde_json::from_slice::<serde_json::Value>(spec_bytes)
            .expect("spec bundle JSON"),
        "certificate": serde_json::from_slice::<serde_json::Value>(certificate_bytes)
            .expect("certificate bundle JSON")
    })
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
    assert_eq!(v["error"]["code"], "CertifiedBundleVerificationFailed");

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
    assert_eq!(v["error"]["code"], "CertifiedBundleInvalid");

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
async fn start_rejects_certifier_invalid_runtime_shape_valid_bundle_before_stream_creation() {
    let app = test_app();
    let certified =
        mfm_op_proof::certified_proof_spec(mfm_op_proof::ProofWorkflowConfig::default())
            .expect("proof spec");
    let mut invalid_spec = certified.spec().clone();
    invalid_spec.config_refs.clear();
    let invalid_spec_bytes = invalid_spec
        .canonical_json()
        .expect("invalid spec remains parseable")
        .to_vec();
    let mut evidence = certified.certificate().evidence.clone();
    evidence.spec_hash = invalid_spec.spec_hash().expect("invalid spec hash");
    let certificate =
        mfm_certify::CertifiedSpecCertificate::from_evidence(evidence).expect("certificate");
    let certificate_bytes = certificate
        .canonical_json()
        .expect("certificate json")
        .to_vec();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "typed_run_start_v1",
                "run_id": VALID_RUN_ID,
                "bundle": certified_bundle_json(&invalid_spec_bytes, &certificate_bytes),
                "drive": "append_only"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "CertifiedBundleVerificationFailed");

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
async fn start_rejects_missing_config_inputs_before_stream_creation() {
    let app = test_app();
    let certified =
        mfm_op_proof::certified_proof_spec(mfm_op_proof::ProofWorkflowConfig::default())
            .expect("proof spec");
    let bundle = certified.bundle().expect("proof bundle");
    let bundle_json = certified_bundle_json(bundle.spec_bytes(), bundle.certificate_bytes());

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "typed_run_start_v1",
                "run_id": VALID_RUN_ID,
                "bundle": bundle_json,
                "drive": "append_only"
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "MissingLaunchConfigArtifact");

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
async fn proof_http_start_replay_uses_certified_bundle_evidence() {
    let root = std::env::temp_dir().join(format!("mfm-rest-proof-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("artifact root");
    let state = mfm_rest_api::make_in_memory_app_state(root.clone());
    let proof_config = mfm_op_proof::ProofWorkflowConfig::default();
    let draft = mfm_op_proof::proof_program_draft(proof_config.clone()).expect("proof draft");
    let certified = mfm_op_proof::certified_proof_spec(proof_config).expect("proof spec");
    let configs = config_bodies_for_draft_and_spec(&draft, &certified.envelope().spec);
    let public_schema_id = certified
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .as_str()
        .to_owned();
    let bundle = certified.bundle().expect("proof bundle");
    let bundle_json = certified_bundle_json(bundle.spec_bytes(), bundle.certificate_bytes());
    let run_id = mfm_app::new_run_id().to_string();
    let app = mfm_rest_api::make_app(state);

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "typed_run_start_v1",
                "run_id": run_id,
                "bundle": bundle_json,
                "configs": configs,
                "framework_version": "mfm.integration.rest.proof.typed.v1",
                "source_revision": "integration-test",
                "drive": "until_blocked"
            }),
        ))
        .await
        .expect("start response");
    assert_eq!(start.status(), StatusCode::OK);
    let start_body = response_json(start).await;
    assert_eq!(start_body["status"], "success");
    assert_eq!(start_body["data"]["phase"], "completed");
    assert_eq!(
        start_body["data"]["spec_hash"],
        certified.spec_hash().as_str()
    );

    let replay = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/replay")))
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = response_json(replay).await;
    assert_eq!(replay_body["status"], "success");
    assert_eq!(replay_body["data"]["phase"], "completed");
    assert_eq!(
        replay_body["data"]["spec_hash"],
        start_body["data"]["spec_hash"]
    );
    assert!(
        replay_body["data"]["retained_artifacts"]
            .as_u64()
            .expect("retained artifact count")
            > 0,
        "replay must be backed by retained recorded evidence"
    );

    let public_output = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{run_id}/public-output/{public_schema_id}"
                ))
                .body(Body::empty())
                .expect("public output request"),
        )
        .await
        .expect("public output response");
    assert_eq!(public_output.status(), StatusCode::OK);
    let public_body = response_json(public_output).await;
    assert_eq!(
        public_body["data"]["json"]["output"]["fact"]["n"],
        serde_json::json!(1)
    );
    assert_eq!(
        public_body["data"]["json"]["output"]["side_effect"]["status"],
        "confirmed"
    );

    let _ = std::fs::remove_dir_all(root);
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

fn config_bodies_for_draft_and_spec(
    draft: &mfm_program::TypedProgramDraft,
    typed_spec: &spec::TypedExecutionSpec,
) -> Vec<serde_json::Value> {
    let mut configs = draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
        .map(|config| config_body(config.schema_id.as_str(), config.canonical_json.as_bytes()))
        .collect::<Vec<_>>();
    for node in &typed_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes = spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
            .expect("canonical framework config");
        assert_eq!(
            bytes.content_digest(),
            node.config_ref.digest,
            "framework config helper must match certified config ref"
        );
        configs.push(config_body(
            node.config_ref.schema_id.as_str(),
            bytes.as_bytes(),
        ));
    }
    configs
}

fn config_body(schema_id: &str, bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "schema_id": schema_id,
        "json": serde_json::from_slice::<serde_json::Value>(bytes).expect("config JSON"),
    })
}
