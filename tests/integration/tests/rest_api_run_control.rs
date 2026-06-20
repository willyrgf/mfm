#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, AttemptId, ContentDigest, DigestAlgorithm, DigestBytes, RunId, SpecHash,
};
use mfm_integration_tests::test_support;
use mfm_manual_auth::{
    ManualAuthorizationSignatureBytes, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
    ManualResolutionPrefixAuthority,
};
use mfm_runtime::{
    manual_resolution_block_reason, manual_resolution_stream_prefix_digest,
    unresolved_manual_obligations_digest,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_store::v1::AsyncTypedRunEventStore;
use serde_json::Value;
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
    let state = test_support::in_memory_rest_app_state(root);
    mfm_rest_api::make_app(state)
}

fn in_memory_state_with_root() -> (std::path::PathBuf, test_support::InMemoryRestAppState) {
    let root = std::env::temp_dir().join(format!("mfm-rest-api-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("artifact root");
    let state = test_support::in_memory_rest_app_state(root.clone());
    (root, state)
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
                "bundle": {
                    "unexpected": "field"
                },
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
    let mut invalid_spec = certified.validated_spec().spec().clone();
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
    let state = test_support::in_memory_rest_app_state(root.clone());
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
    assert_eq!(start_body["data"]["run_mode"], "completed");
    assert_eq!(
        start_body["data"]["spec_hash"],
        certified.spec_hash().as_str()
    );
    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/status"))
                .body(Body::empty())
                .expect("status request"),
        )
        .await
        .expect("status response");
    assert_eq!(status.status(), StatusCode::OK);
    let status_body = response_json(status).await;
    assert_eq!(status_body["status"], "success");
    assert_eq!(status_body["data"]["run_id"], run_id);
    assert_eq!(
        status_body["data"]["spec_hash"],
        start_body["data"]["spec_hash"]
    );
    assert_eq!(status_body["data"]["run_mode"], "completed");
    assert_eq!(status_body["data"]["scheduler_status"], "observed");
    assert!(
        status_body["data"]["attempt_dispositions"]
            .as_array()
            .is_some_and(|attempts| !attempts.is_empty()),
        "status route must expose attempt dispositions"
    );

    let replay = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/replay")))
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = response_json(replay).await;
    assert_eq!(replay_body["status"], "success");
    assert_eq!(replay_body["data"]["run_mode"], "completed");
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
async fn manual_resolution_route_records_resolution_and_hides_proof_bytes() {
    let (root, state) = in_memory_state_with_root();
    let proof_config = mfm_op_proof::ProofWorkflowConfig::default();
    let draft = mfm_op_proof::manual_resolution_proof_program_draft(proof_config.clone())
        .expect("manual proof draft");
    let certified = mfm_op_proof::certified_manual_resolution_proof_spec(proof_config)
        .expect("manual proof spec");
    let configs = config_bodies_for_draft_and_spec(&draft, &certified.envelope().spec);
    let bundle = certified.bundle().expect("manual proof bundle");
    let bundle_json = certified_bundle_json(bundle.spec_bytes(), bundle.certificate_bytes());
    let run_id = mfm_app::new_run_id();
    let app = mfm_rest_api::make_app(state.clone());

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "typed_run_start_v1",
                "run_id": run_id.as_str(),
                "bundle": bundle_json,
                "configs": configs,
                "framework_version": "mfm.integration.rest.manual_resolution.v1",
                "source_revision": "integration-test",
                "drive": "until_blocked"
            }),
        ))
        .await
        .expect("start response");
    assert_eq!(start.status(), StatusCode::OK);
    let start_body = response_json(start).await;
    assert_eq!(start_body["status"], "success");
    assert_eq!(start_body["data"]["run_mode"], "manual_blocked");
    assert_eq!(
        start_body["data"]["saga"]["manual_block_reason"],
        "policy_manual_resolution"
    );

    let evidence_json = serde_json::json!({"operator_note":"reviewed"});
    let evidence_bytes = serde_json::to_vec(&evidence_json).expect("evidence JSON");
    let proof_bytes = signed_manual_resolution_proof_bytes(
        &state.store,
        &run_id,
        &certified,
        manual_policy(&certified),
        &evidence_bytes,
    )
    .await;
    let proof_json: Value = serde_json::from_slice(&proof_bytes).expect("proof JSON");

    let resolved = app
        .clone()
        .oneshot(json_post(
            &format!("/v1/runs/{run_id}/manual-resolution"),
            serde_json::json!({
                "kind": "manual_resolution_v1",
                "outcome": "confirm_remediated",
                "evidence_json": evidence_json,
                "authorization_proof": proof_json,
                "note": "reviewed externally",
                "drive": "until_blocked"
            }),
        ))
        .await
        .expect("manual resolution response");
    assert_eq!(resolved.status(), StatusCode::OK);
    let resolved_body = response_json(resolved).await;
    assert_eq!(resolved_body["status"], "success");
    assert_eq!(resolved_body["data"]["run_mode"], "manually_resolved");
    assert_eq!(
        resolved_body["data"]["saga"]["terminal_resolution"]["claim"],
        "manual_resolution"
    );

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/status"))
                .body(Body::empty())
                .expect("status request"),
        )
        .await
        .expect("status response");
    assert_eq!(status.status(), StatusCode::OK);
    let status_body = response_json(status).await;
    assert_eq!(status_body["data"]["run_mode"], "manually_resolved");
    assert!(
        status_body["data"]["attempt_dispositions"]
            .as_array()
            .expect("attempts")
            .iter()
            .any(|attempt| {
                attempt["disposition"] == "completed"
                    && resolve_saga_node_ids(&certified.envelope().spec)
                        .contains(&attempt["node_id"].as_str().unwrap_or_default())
            }),
        "missing completed ResolveSagaTerminal attempt"
    );

    let raw_stream = state
        .store
        .load_run_stream(&run_id)
        .await
        .expect("raw run stream");
    assert!(
        raw_stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ManualResolutionRecorded(_)
        )),
        "stream must contain manual resolution event"
    );
    assert_framework_started_before_run_completed(&raw_stream, &certified.envelope().spec);

    let stream = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/stream"))
                .body(Body::empty())
                .expect("stream request"),
        )
        .await
        .expect("stream response");
    assert_eq!(stream.status(), StatusCode::OK);
    let stream_body = response_json(stream).await;
    let rendered = serde_json::to_string(&serde_json::json!([
        start_body,
        resolved_body,
        status_body,
        stream_body
    ]))
    .expect("render public JSON");
    let proof_rendered = std::str::from_utf8(&proof_bytes).expect("proof utf8");
    let proof: Value = serde_json::from_slice(&proof_bytes).expect("proof JSON");
    let signature = proof["signatures"][0]["signature_hex"]
        .as_str()
        .expect("signature");
    assert!(!rendered.contains(proof_rendered));
    assert!(!rendered.contains(signature));

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn status_route_reports_interrupted_attempt_and_framework_attempts_from_history() {
    let (root, state) = in_memory_state_with_root();
    let proof_config = mfm_op_proof::ProofWorkflowConfig::default();
    let draft = mfm_op_proof::proof_program_draft(proof_config.clone()).expect("proof draft");
    let certified = mfm_op_proof::certified_proof_spec(proof_config).expect("proof spec");
    let configs = config_bodies_for_draft_and_spec(&draft, &certified.envelope().spec);
    let bundle = certified.bundle().expect("proof bundle");
    let run_id = mfm_app::new_run_id();
    let app = mfm_rest_api::make_app(state.clone());

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "typed_run_start_v1",
                "run_id": run_id.as_str(),
                "bundle": certified_bundle_json(bundle.spec_bytes(), bundle.certificate_bytes()),
                "configs": configs,
                "framework_version": "mfm.integration.rest.proof.typed.v1",
                "source_revision": "integration-test",
                "drive": "append_only"
            }),
        ))
        .await
        .expect("start response");
    assert_eq!(start.status(), StatusCode::OK);
    let start_body = response_json(start).await;
    assert_eq!(start_body["status"], "success");
    assert_eq!(start_body["data"]["run_mode"], "forward");

    let interrupted_node = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("domain node");
    let interrupted_attempt_id = fixed_attempt_id(0x41);
    append_interrupted_attempt(
        &state.store,
        &run_id,
        certified.spec_hash(),
        interrupted_node,
        &interrupted_attempt_id,
    )
    .await;

    let resume = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/resume")))
        .await
        .expect("resume response");
    assert_eq!(resume.status(), StatusCode::OK);
    let resume_body = response_json(resume).await;
    assert_eq!(resume_body["status"], "success");
    assert_eq!(resume_body["data"]["run_mode"], "completed");

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/status"))
                .body(Body::empty())
                .expect("status request"),
        )
        .await
        .expect("status response");
    assert_eq!(status.status(), StatusCode::OK);
    let status_body = response_json(status).await;
    assert_eq!(status_body["status"], "success");
    assert_ne!(status_body["data"]["run_mode"], "interrupted");
    let attempts = status_body["data"]["attempt_dispositions"]
        .as_array()
        .expect("attempt dispositions");
    let interrupted = attempts
        .iter()
        .find(|attempt| {
            attempt["attempt_id"] == interrupted_attempt_id.as_str()
                && attempt["disposition"] == "interrupted"
        })
        .expect("interrupted attempt disposition");
    assert!(interrupted["retryable"].is_null());

    let completed_framework_kinds = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .filter(|node| {
            attempts.iter().any(|attempt| {
                attempt["node_id"] == node.node_id.as_str() && attempt["disposition"] == "completed"
            })
        })
        .filter_map(|node| match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => Some("public_output_render"),
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                Some("project_retention_manifest")
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => Some("complete_run"),
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) => Some("resolve_saga_terminal"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        completed_framework_kinds.contains(&"public_output_render"),
        "missing completed public-output render framework attempt"
    );
    assert!(
        completed_framework_kinds.contains(&"project_retention_manifest"),
        "missing completed retention framework attempt"
    );
    assert!(
        completed_framework_kinds.contains(&"complete_run"),
        "missing completed complete-run framework attempt"
    );

    let stream = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/stream"))
                .body(Body::empty())
                .expect("stream request"),
        )
        .await
        .expect("stream response");
    assert_eq!(stream.status(), StatusCode::OK);
    let stream_body = response_json(stream).await;
    assert_eq!(stream_body["status"], "success");
    let stream_events = stream_body["data"]["events"]
        .as_array()
        .expect("stream events");
    assert_framework_started_before_terminal_evidence(
        stream_events,
        attempts,
        &certified.envelope().spec.nodes,
        &run_id,
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

#[test]
fn stream_filters_range_after_authoritative_run_stream_validation() {
    let source = include_str!("../../../bin/rest-api/src/lib.rs");
    let start = source
        .find("async fn runs_stream")
        .expect("runs_stream handler");
    let end = source[start..]
        .find("async fn runs_replay")
        .map(|offset| start + offset)
        .expect("next handler");
    let body = &source[start..end];
    let range_validation = body
        .find("validate_sequence_range(query.from_seq, query.to_seq)")
        .expect("range validation");
    let authoritative_read = body
        .find(".run_stream(&run_id)")
        .expect("authoritative stream read");
    let range_filter = body.find(".filter(|event|").expect("range filter");

    assert!(
        !body.contains(".load_run_stream("),
        "REST stream handler must delegate to app-level full stream validation"
    );
    assert!(range_validation < authoritative_read);
    assert!(authoritative_read < range_filter);
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

async fn append_interrupted_attempt(
    store: &store::AsyncInMemoryTypedRunStore,
    run_id: &RunId,
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) {
    let start = store::TypedCommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .expect("expected next seq"),
        store::CommitKey::new("rest-interrupted-attempt-start").expect("commit key"),
        vec![events::KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no: 1,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            },
        )],
        Vec::new(),
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    )
    .expect("attempt start request");
    append_typed_commit(store, start).await;

    let interrupted = store::TypedCommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .expect("expected next seq"),
        store::CommitKey::new("rest-interrupted-attempt-terminal").expect("commit key"),
        vec![events::KernelEventPayload::StateAttemptInterrupted(
            events::StateAttemptInterrupted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
            },
        )],
        Vec::new(),
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    )
    .expect("attempt interrupted request");
    append_typed_commit(store, interrupted).await;
}

async fn append_typed_commit(
    store: &store::AsyncInMemoryTypedRunStore,
    request: store::TypedCommitRequest,
) {
    let admitted_artifacts = request.required_artifacts().to_vec();
    let artifacts = store::CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        admitted_artifacts,
    )
    .expect("artifact evidence set");
    let plan = if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, events::KernelEventPayload::StateAttemptStarted(_)))
    {
        store::PreparedCommit::<store::StateAttemptStarted>::new(request, artifacts)
            .expect("prepared attempt-start commit")
            .into()
    } else {
        store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)
            .expect("prepared attempt-terminal commit")
            .into()
    };
    store
        .append_prepared_commit_plan(plan)
        .await
        .expect("append typed commit");
}

fn fixed_attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn assert_framework_started_before_terminal_evidence(
    stream_events: &[Value],
    attempts: &[Value],
    nodes: &[spec::NodeSpec],
    run_id: &RunId,
) {
    for node in nodes.iter().filter(|node| node.framework.is_some()) {
        let required_kind = match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => "public_output_render",
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                "project_retention_manifest"
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => "complete_run",
            _ => continue,
        };
        let attempt = attempts
            .iter()
            .find(|attempt| {
                attempt["node_id"].as_str() == Some(node.node_id.as_str())
                    && attempt["disposition"].as_str() == Some("completed")
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing completed {required_kind} framework attempt for {}",
                    node.node_id
                )
            });
        let attempt_id = attempt["attempt_id"].as_str().expect("attempt id");
        let attempt_key = format!("attempt:{}:{}", node.node_id, attempt_id);
        let start_index = stream_event_position(
            stream_events,
            |event| {
                event["logical_key"].as_str() == Some(attempt_key.as_str())
                    && event["event_schema_id"]
                        .as_str()
                        .is_some_and(|schema| schema.contains("state_attempt_started"))
            },
            &format!("framework start {attempt_key}"),
        );
        let completed_index = stream_event_position(
            stream_events,
            |event| {
                event["logical_key"].as_str() == Some(attempt_key.as_str())
                    && event["event_schema_id"]
                        .as_str()
                        .is_some_and(|schema| schema.contains("state_attempt_completed"))
            },
            &format!("framework completion {attempt_key}"),
        );
        assert!(
            start_index < completed_index,
            "framework StateAttemptStarted must precede StateAttemptCompleted for {attempt_key}"
        );

        match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => {
                let public_output_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"]
                            .as_str()
                            .is_some_and(|key| key.starts_with("public_output:"))
                            && event["event_schema_id"]
                                .as_str()
                                .is_some_and(|schema| schema.contains("public_output_produced"))
                    },
                    "public-output terminal evidence",
                );
                assert!(
                    start_index < public_output_index,
                    "public-output framework start must precede public output evidence"
                );
            }
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                let retention_prefix = format!("retention:{}:manifest:", run_id);
                let retention_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"]
                            .as_str()
                            .is_some_and(|key| key.starts_with(&retention_prefix))
                            && event["event_schema_id"].as_str().is_some_and(|schema| {
                                schema.contains("retention_manifest_projected")
                            })
                    },
                    "retention manifest terminal evidence",
                );
                assert!(
                    start_index < retention_index,
                    "retention framework start must precede retention manifest evidence"
                );
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => {
                let completed_run_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"].as_str() == Some("run:complete")
                            && event["event_schema_id"]
                                .as_str()
                                .is_some_and(|schema| schema.contains("run_completed"))
                    },
                    "run completion terminal evidence",
                );
                assert!(
                    start_index < completed_run_index,
                    "complete-run framework start must precede run completion evidence"
                );
            }
            _ => {}
        }
    }
}

fn stream_event_position(
    events: &[Value],
    predicate: impl Fn(&Value) -> bool,
    label: &str,
) -> usize {
    events
        .iter()
        .position(predicate)
        .unwrap_or_else(|| panic!("missing stream event for {label}"))
}

async fn signed_manual_resolution_proof_bytes(
    store: &store::AsyncInMemoryTypedRunStore,
    run_id: &RunId,
    certified: &mfm_certify::CertifiedTypedSpec,
    manual: spec::ManualResolutionEvidenceSpec,
    evidence_bytes: &[u8],
) -> Vec<u8> {
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(evidence_bytes),
    );
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        artifact_id: ArtifactId::from_digest(evidence_hash.algorithm(), *evidence_hash.digest()),
        content_hash: evidence_hash,
    };
    let stream = store.load_run_stream(run_id).await.expect("run stream");
    let projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("projection rebuild");
    let saga = projection.derive_saga_projection(run_id, &certified.envelope().spec.saga);
    let reason = saga.manual_block_reason.expect("manual block reason");
    let expected_next_seq = store
        .expected_next_seq(run_id)
        .await
        .expect("expected next seq");
    let prefix = ManualResolutionPrefixAuthority::new(
        run_id.clone(),
        certified.spec_hash().clone(),
        expected_next_seq.as_u64(),
        manual_resolution_stream_prefix_digest(&stream).expect("prefix digest"),
        manual_resolution_block_reason(reason),
        unresolved_manual_obligations_digest(&saga).expect("obligation digest"),
        manual.clone(),
    )
    .expect("manual prefix");
    let claim = prefix
        .authorization_claim(events::ManualResolutionOutcome::ConfirmRemediated, evidence)
        .expect("manual claim");
    let operator = manual.authorization.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: manual.authorization.verifier_id,
        signing_scheme: manual.authorization.signing_scheme,
        claim,
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    };
    proof
        .canonical_json()
        .expect("canonical manual proof")
        .to_vec()
}

fn manual_policy(
    certified: &mfm_certify::CertifiedTypedSpec,
) -> spec::ManualResolutionEvidenceSpec {
    match &certified.envelope().spec.saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual.clone(),
        _ => panic!("manual proof scenario must carry manual policy"),
    }
}

fn resolve_saga_node_ids(spec: &spec::TypedExecutionSpec) -> Vec<&str> {
    spec.nodes
        .iter()
        .filter(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .map(|node| node.node_id.as_str())
        .collect()
}

fn assert_framework_started_before_run_completed(
    stream: &[store::KernelEventEnvelope],
    spec: &spec::TypedExecutionSpec,
) {
    let resolve_nodes = resolve_saga_node_ids(spec);
    let started = stream
        .iter()
        .position(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                resolve_nodes.contains(&payload.node_id.as_str())
            }
            _ => false,
        })
        .expect("ResolveSagaTerminal attempt start");
    let completed = stream
        .iter()
        .position(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
        .expect("RunCompleted event");
    assert!(started < completed);
}

fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}
