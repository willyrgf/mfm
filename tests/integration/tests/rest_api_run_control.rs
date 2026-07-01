#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_events::v1 as events;
use mfm_ids::RunId;
use mfm_integration_tests::test_support::{self, empty_post, json_post, response_json};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_store::v1::{RetainedArtifactReadProvider, RunEventStore};
use tower::ServiceExt;

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001";
const VALID_SCHEMA_ID: &str =
    "schema:mfm.test.public:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000002";
const PORTFOLIO_NETWORK_ID: &str = "rest-control-eth";
static RPC_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_app() -> axum::Router {
    let state = test_support::in_memory_rest_app_state();
    mfm_rest_api::make_app(state)
}

fn in_memory_state() -> test_support::InMemoryRestAppState {
    test_support::in_memory_rest_app_state()
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
async fn ready_endpoint_reports_run_store_readiness() {
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
    assert_eq!(v["data"]["checks"]["run_store"], "ready");
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
async fn start_rejects_unknown_start_fields() {
    let app = test_app();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "unexpected": true
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
async fn start_accepts_entry_point_toml_shape_with_default_format() {
    let app = test_app();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "missing_contract_test_op",
                "config": "portfolio_id = \"main\"\n",
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "EntryPointOpNotFound");
}

#[tokio::test]
async fn start_accepts_entry_point_toml_shape_with_explicit_version() {
    let app = test_app();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "missing_contract_test_op",
                "op_version": 1,
                "config_format": "toml",
                "config": "portfolio_id = \"main\"\n",
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "EntryPointOpNotFound");
}

#[tokio::test]
async fn start_accepts_entry_point_json_object_config_shape() {
    let app = test_app();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "missing_contract_test_op",
                "config_format": "json",
                "config": {
                    "portfolio_id": "main",
                    "wallets": []
                },
            }),
        ))
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "EntryPointOpNotFound");
}

#[tokio::test]
async fn start_rejects_raw_run_id_field_as_unknown_json() {
    let app = test_app();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": {
                    "portfolio_id": "main",
                    "wallets": []
                },
                "run_id": VALID_RUN_ID,
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
async fn start_distinct_run_key_derives_separate_run_without_persisting_raw_key() {
    let rpc_url = test_support::start_portfolio_rpc_mock(31337).await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = test_support::write_evm_runtime_config_for_test(
        runtime_config_dir.path(),
        PORTFOLIO_NETWORK_ID,
        &rpc_url,
        None,
    );
    let mut state = in_memory_state();
    state.runtime_config_path = Some(runtime_config_path);
    let app = mfm_rest_api::make_app(state.clone());
    let raw_key = "distinct-alpha";
    let first_run_id = test_support::prepare_portfolio_launch_for_store(
        &state.store,
        &portfolio_snapshot_config(),
        None,
    )
    .await
    .request
    .run_id;

    let distinct = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": portfolio_snapshot_config(),
                "distinct_run_key": raw_key,
            }),
        ))
        .await
        .expect("distinct start response");
    assert_eq!(distinct.status(), StatusCode::OK);
    let distinct_body = response_json(distinct).await;
    assert_eq!(distinct_body["data"]["outcome"], "admitted");
    assert!(!distinct_body.to_string().contains(raw_key));
    let distinct_run_id = RunId::parse(
        distinct_body["data"]["run"]["run_id"]
            .as_str()
            .expect("distinct run id"),
    )
    .expect("typed distinct run id");
    assert_ne!(first_run_id, distinct_run_id);

    let stream = state
        .store
        .load_run_stream(&distinct_run_id)
        .await
        .expect("distinct stream");
    let admitted = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload),
            _ => None,
        })
        .expect("RunAdmitted");
    assert!(admitted.identity_material.distinct_run_key_digest.is_some());
    assert!(!format!("{admitted:?}").contains(raw_key));
}

#[tokio::test]
async fn portfolio_chain_mismatch_fails_after_admission_with_redacted_diagnostic() {
    let rpc_url = test_support::start_portfolio_rpc_mock(31338).await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = test_support::write_evm_runtime_config_for_test(
        runtime_config_dir.path(),
        PORTFOLIO_NETWORK_ID,
        &rpc_url,
        None,
    );
    let mut state = in_memory_state();
    state.runtime_config_path = Some(runtime_config_path.clone());
    let app = mfm_rest_api::make_app(state.clone());

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": portfolio_snapshot_config(),
            }),
        ))
        .await
        .expect("chain mismatch start response");
    let status = resp.status();
    let body = response_json(resp).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["outcome"], "admitted");
    assert_ne!(body["data"]["run"]["run_mode"], "completed");

    let run_id = RunId::parse(body["data"]["run"]["run_id"].as_str().expect("run id"))
        .expect("typed run id");
    let stream = state.store.load_run_stream(&run_id).await.expect("stream");
    let admitted_index = stream
        .iter()
        .position(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_)))
        .expect("RunAdmitted");
    let failed_index = stream
        .iter()
        .position(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptFailed(_)
            )
        })
        .expect("StateAttemptFailed");
    assert!(admitted_index < failed_index);
    let failed = stream[failed_index].payload();
    let events::KernelEventPayload::StateAttemptFailed(failed) = failed else {
        panic!("expected failed attempt");
    };
    assert_eq!(failed.error.category, events::ErrorCategory::Validation);
    assert_eq!(failed.error.safe_message, "runner output failed validation");
    let diagnostic = failed
        .error
        .diagnostic_ref
        .as_ref()
        .expect("chain mismatch records diagnostic artifact");
    let details_digest = failed
        .error
        .public_details
        .as_ref()
        .expect("chain mismatch records public details digest")
        .content_digest
        .clone();
    let diagnostic_artifact = state
        .store
        .read_retained_artifact(&diagnostic_artifact_requirement(diagnostic))
        .await
        .expect("diagnostic artifact");
    let diagnostic_json = serde_json::from_slice::<serde_json::Value>(diagnostic_artifact.bytes())
        .expect("diagnostic json");
    assert_eq!(
        diagnostic_json["public_details"],
        serde_json::json!({
            "network_id": PORTFOLIO_NETWORK_ID,
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": PORTFOLIO_NETWORK_ID,
            "policy_id": PORTFOLIO_NETWORK_ID,
        })
    );
    assert_eq!(
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
            &serde_json::to_string(&diagnostic_json["public_details"]).expect("details json")
        )
        .expect("canonical details")
        .content_digest(),
        details_digest
    );

    let rendered = format!("{body:?} {stream:?}");
    assert!(!rendered.contains(&rpc_url));
    assert!(!rendered.contains(&runtime_config_path.display().to_string()));
    assert!(!rendered.contains("31338"));
    assert!(!diagnostic_json.to_string().contains(&rpc_url));
    assert!(!diagnostic_json
        .to_string()
        .contains(&runtime_config_path.display().to_string()));

    std::fs::remove_file(&runtime_config_path).expect("remove runtime config");
    let replay = app
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/replay")))
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::OK);
}

#[tokio::test]
async fn evm_contract_start_requires_capability_before_admission_for_all_entry_point_ops() {
    let state = in_memory_state();
    let app = mfm_rest_api::make_app(state.clone());

    for (op, config) in evm_entry_point_configs() {
        let prepared = test_support::prepare_entry_point_launch_for_store(
            &state.store,
            op,
            Some(mfm_app::OpVersion::new(1).expect("op version")),
            &config,
            None,
        )
        .await;
        assert_evm_entry_point_evidence(&prepared.evidence, op);
        let run_id = prepared.request.run_id.clone();
        let resp = app
            .clone()
            .oneshot(json_post(
                "/v1/runs/start",
                serde_json::json!({
                    "op": op,
                    "op_version": 1,
                    "config_format": "json",
                    "config": config,
                }),
            ))
            .await
            .expect("EVM entry-point start response");
        let status = resp.status();
        let body = response_json(resp).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["status"], "error");
        assert_eq!(body["error"]["code"], "LaunchRunnerUnavailable");

        let stream = state
            .store
            .load_run_stream(&run_id)
            .await
            .expect("run stream");
        assert!(
            stream.is_empty(),
            "{op} must fail capability ingress before RunAdmitted"
        );
    }
}

#[tokio::test]
async fn evm_contract_validation_ignores_unused_malformed_signers() {
    let rpc_url = test_support::start_portfolio_rpc_mock(1).await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = write_evm_runtime_config_with_malformed_signers(
        runtime_config_dir.path(),
        "ethereum-mainnet",
        &rpc_url,
    );
    let mut state = in_memory_state();
    state.runtime_config_path = Some(runtime_config_path);
    let app = mfm_rest_api::make_app(state.clone());

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "evm_contract_validate",
                "op_version": 1,
                "config_format": "json",
                "config": evm_validate_entry_config_json(),
            }),
        ))
        .await
        .expect("validate start response");
    let status = resp.status();
    let body = response_json(resp).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let run_id = RunId::parse(body["data"]["run"]["run_id"].as_str().expect("run id"))
        .expect("typed run id");
    let stream = state.store.load_run_stream(&run_id).await.expect("stream");
    assert!(
        stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_))),
        "validation must pass capability ingress without requiring signer bindings"
    );
}

#[tokio::test]
async fn evm_contract_mutation_requires_signers_before_admission() {
    let rpc_url = test_support::start_portfolio_rpc_mock(1).await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = test_support::write_evm_runtime_config_for_test(
        runtime_config_dir.path(),
        "ethereum-mainnet",
        &rpc_url,
        None,
    );
    let mut state = in_memory_state();
    state.runtime_config_path = Some(runtime_config_path);
    let app = mfm_rest_api::make_app(state.clone());
    let prepared = test_support::prepare_entry_point_launch_for_store(
        &state.store,
        "evm_contract_deploy",
        Some(mfm_app::OpVersion::new(1).expect("op version")),
        &evm_deploy_config_json(),
        None,
    )
    .await;
    let run_id = prepared.request.run_id.clone();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "evm_contract_deploy",
                "op_version": 1,
                "config_format": "json",
                "config": evm_deploy_config_json(),
            }),
        ))
        .await
        .expect("deploy start response");
    let status = resp.status();
    let body = response_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "LaunchRunnerUnavailable");
    let stream = state
        .store
        .load_run_stream(&run_id)
        .await
        .expect("run stream");
    assert!(
        stream.is_empty(),
        "missing signer must fail before admission"
    );
}

#[tokio::test]
async fn evm_contract_mutation_rejects_malformed_signers_before_admission() {
    let rpc_url = test_support::start_portfolio_rpc_mock(1).await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = write_evm_runtime_config_with_malformed_signers(
        runtime_config_dir.path(),
        "ethereum-mainnet",
        &rpc_url,
    );
    let mut state = in_memory_state();
    state.runtime_config_path = Some(runtime_config_path);
    let app = mfm_rest_api::make_app(state.clone());
    let prepared = test_support::prepare_entry_point_launch_for_store(
        &state.store,
        "evm_contract_deploy",
        Some(mfm_app::OpVersion::new(1).expect("op version")),
        &evm_deploy_config_json(),
        None,
    )
    .await;
    let run_id = prepared.request.run_id.clone();

    let resp = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "evm_contract_deploy",
                "op_version": 1,
                "config_format": "json",
                "config": evm_deploy_config_json(),
            }),
        ))
        .await
        .expect("deploy start response");
    let status = resp.status();
    let body = response_json(resp).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "LaunchRunnerUnavailable");
    let stream = state
        .store
        .load_run_stream(&run_id)
        .await
        .expect("run stream");
    assert!(
        stream.is_empty(),
        "malformed signer config must fail before admission"
    );
}

#[tokio::test]
async fn portfolio_status_route_reports_interrupted_attempt_and_framework_attempts_from_history() {
    let _env_guard = RPC_ENV_LOCK.lock().await;
    let rpc_url = test_support::start_portfolio_rpc_mock(31337).await;
    let _env_restore =
        test_support::set_evm_runtime_config_env_for_test(PORTFOLIO_NETWORK_ID, &rpc_url);
    let state = in_memory_state();
    let config = portfolio_snapshot_config();
    let app = mfm_rest_api::make_app(state.clone());
    let (run_id, certified) =
        test_support::admit_portfolio_run_without_driving(&state.store, &config).await;

    let interrupted_node = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("domain node");
    let interrupted_attempt_id = store::test_support::fixed_attempt_id_for_test(0x41);
    store::test_support::append_interrupted_attempt_for_test(
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
    test_support::assert_framework_started_before_terminal_evidence(
        stream_events,
        attempts,
        &certified.envelope().spec.nodes,
        &run_id,
    );
}

#[tokio::test]
async fn status_invalid_run_id_has_domain_error() {
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
async fn absent_run_status_resume_replay_and_public_output_are_not_found() {
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
async fn malformed_runtime_config_does_not_block_read_only_routes() {
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = runtime_config_dir.path().join("malformed-runtime.toml");
    std::fs::write(&runtime_config_path, "not valid toml = [").expect("runtime config");
    let mut state = in_memory_state();
    state.runtime_config_path = Some(runtime_config_path.clone());
    let app = mfm_rest_api::make_app(state);

    let health = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/health")
                .body(Body::empty())
                .expect("health request"),
        )
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/status"))
                .body(Body::empty())
                .expect("status request"),
        )
        .await
        .expect("status response");
    assert_eq!(status.status(), StatusCode::NOT_FOUND);

    let replay = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{VALID_RUN_ID}/replay")))
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::NOT_FOUND);

    let public_output = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{VALID_RUN_ID}/public-output/{VALID_SCHEMA_ID}"
                ))
                .body(Body::empty())
                .expect("public output request"),
        )
        .await
        .expect("public output response");
    assert_eq!(public_output.status(), StatusCode::NOT_FOUND);

    let start = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": portfolio_snapshot_config(),
            }),
        ))
        .await
        .expect("live start response");
    assert_eq!(start.status(), StatusCode::BAD_REQUEST);
    let body = response_json(start).await;
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["code"], "LaunchRunnerUnavailable");
    let rendered = body.to_string();
    assert!(!rendered.contains(&runtime_config_path.display().to_string()));
    assert!(!rendered.contains("not valid toml"));
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

fn portfolio_snapshot_config() -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "rest-control",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": PORTFOLIO_NETWORK_ID,
                    "family": "evm",
                    "chain_id": 31337,
                    "control_scope": "rest-control",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_rest_control",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": PORTFOLIO_NETWORK_ID,
                    "symbol_ids": ["eth.native.rest-control-eth"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.rest-control-eth",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": PORTFOLIO_NETWORK_ID,
                    "protocol": null,
                    "balance_reader": {
                        "kind": "native_balance"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.rest-control-eth",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "2.50"
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        },
        "valuation_source_registry": {
            "sources": []
        }
    })
}

fn evm_entry_point_configs() -> [(&'static str, serde_json::Value); 4] {
    [
        ("evm_contract_deploy", evm_deploy_config_json()),
        ("evm_contract_configure", evm_configure_entry_config_json()),
        ("evm_contract_validate", evm_validate_entry_config_json()),
        ("evm_contract_lifecycle", evm_lifecycle_config_json()),
    ]
}

fn evm_network_json() -> serde_json::Value {
    serde_json::json!({
        "network_id": "ethereum-mainnet",
        "expected_chain_id": 1,
    })
}

fn evm_signer_json() -> serde_json::Value {
    serde_json::json!({
        "signer_ref": "deployer",
        "expected_signer_address": "0x000000000000000000000000000000000000dead",
    })
}

fn evm_deploy_config_json() -> serde_json::Value {
    serde_json::json!({
        "network": evm_network_json(),
        "signer": evm_signer_json(),
    })
}

fn evm_configure_config_json() -> serde_json::Value {
    serde_json::json!({
        "network": evm_network_json(),
        "signer": evm_signer_json(),
        "calls": [],
    })
}

fn evm_validate_config_json() -> serde_json::Value {
    serde_json::json!({
        "network": evm_network_json(),
    })
}

fn evm_lifecycle_config_json() -> serde_json::Value {
    serde_json::json!({
        "deploy": evm_deploy_config_json(),
        "configure": evm_configure_config_json(),
        "validate": evm_validate_config_json(),
    })
}

fn write_evm_runtime_config_with_malformed_signers(
    dir: &std::path::Path,
    network_id: &str,
    rpc_url: &str,
) -> std::path::PathBuf {
    let path = test_support::write_evm_runtime_config_for_test(dir, network_id, rpc_url, None);
    let mut config = std::fs::read_to_string(&path).expect("runtime config");
    config.push_str(
        r#"
[evm.signers.deployer]
provider = "raw-private-key"
entry_id = "not-a-uuid"
keystore_path = "/runtime/keystore.json"
unlock_file = "/runtime/unlock"
"#,
    );
    std::fs::write(&path, config).expect("write malformed signer runtime config");
    path
}

fn diagnostic_artifact_requirement(
    reference: &events::ArtifactEvidenceRef,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: reference.artifact_id.clone(),
        digest: Some(reference.content_digest.clone()),
        byte_len: Some(reference.byte_len),
        media_type: Some(reference.media_type.clone()),
        schema_id: Some(reference.schema_id.clone()),
        semantic_type_id: reference.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(reference.role),
    }
}

fn evm_deployed_contract_json() -> serde_json::Value {
    serde_json::json!({
        "lifecycle_version": 1,
        "network_id": "ethereum-mainnet",
        "expected_chain_id": 1,
        "contract_address": "0x000000000000000000000000000000000000dead",
        "deploy_tx_hash": "0x01",
        "deploy_receipt_evidence": null,
        "deployed_block_number": 1,
    })
}

fn evm_configured_contract_json() -> serde_json::Value {
    serde_json::json!({
        "lifecycle_version": 1,
        "deployed": evm_deployed_contract_json(),
        "configure_calls": [],
        "confirmation_read_assertions": [],
        "confirmation_event_assertions": [],
        "configure_tx_hashes": [],
        "configure_receipt_evidence": [],
        "configured_block_number": 2,
    })
}

fn evm_configure_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "config": evm_configure_config_json(),
        "deployed": evm_deployed_contract_json(),
    })
}

fn evm_validate_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "config": evm_validate_config_json(),
        "configured": evm_configured_contract_json(),
    })
}

fn assert_evm_entry_point_evidence(evidence: &mfm_app::EntryPointLaunchEvidence, op_name: &str) {
    let registry = mfm_app::production_entry_point_op_registry().expect("entry-point registry");
    let public_name = mfm_app::PublicOpName::new(op_name).expect("public op name");
    let op = registry
        .resolve(
            &public_name,
            Some(mfm_app::OpVersion::new(1).expect("op version")),
        )
        .expect("EVM entry-point op");

    assert_eq!(evidence.resolved_op_id, op.op_id());
    assert_eq!(
        evidence.entry_point_registry_digest,
        registry.registry_digest().expect("registry digest")
    );
}
