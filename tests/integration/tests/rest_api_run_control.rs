#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_events::v1 as events;
use mfm_ids::RunId;
use mfm_integration_tests::test_support::{self, empty_post, json_post, response_json};
use mfm_store::v1 as store;
use mfm_store::v1::{RunEventStore, RunObservationStore};
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

fn state_and_app_with_runtime_config(
    runtime_config_path: std::path::PathBuf,
) -> (test_support::InMemoryRestAppState, axum::Router) {
    let mut state = in_memory_state();
    state.runtime_config_path = Some(runtime_config_path);
    let app = mfm_rest_api::make_app(state.clone());
    (state, app)
}

struct RuntimeConfigApp {
    _runtime_config_dir: tempfile::TempDir,
    runtime_config_path: std::path::PathBuf,
    state: test_support::InMemoryRestAppState,
    app: axum::Router,
}

fn runtime_config_app(network_id: &str, rpc_url: &str) -> RuntimeConfigApp {
    runtime_config_app_with(network_id, rpc_url, |dir, network_id, rpc_url| {
        test_support::write_evm_runtime_config_for_test(dir, network_id, rpc_url, None)
    })
}

fn malformed_signer_runtime_config_app(network_id: &str, rpc_url: &str) -> RuntimeConfigApp {
    runtime_config_app_with(
        network_id,
        rpc_url,
        write_evm_runtime_config_with_malformed_signers,
    )
}

fn runtime_config_app_with(
    network_id: &str,
    rpc_url: &str,
    write: impl FnOnce(&std::path::Path, &str, &str) -> std::path::PathBuf,
) -> RuntimeConfigApp {
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = write(runtime_config_dir.path(), network_id, rpc_url);
    let (state, app) = state_and_app_with_runtime_config(runtime_config_path.clone());
    RuntimeConfigApp {
        _runtime_config_dir: runtime_config_dir,
        runtime_config_path,
        state,
        app,
    }
}

fn get(uri: impl Into<String>) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri.into())
        .body(Body::empty())
        .expect("request")
}

async fn assert_error_response(response: axum::response::Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    let value = response_json(response).await;
    assert_eq!(value["status"], "error");
    assert_eq!(value["error"]["code"], code);
}

async fn assert_start_request_error(request: Request<Body>, code: &str) {
    let response = test_app().oneshot(request).await.expect("response");
    assert_error_response(response, StatusCode::BAD_REQUEST, code).await;
}

async fn assert_start_json_error(body: serde_json::Value, code: &str) {
    assert_start_request_error(json_post("/v1/runs/start", body), code).await;
}

async fn get_success(path: &str) -> serde_json::Value {
    let response = test_app().oneshot(get(path)).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let value = response_json(response).await;
    assert_eq!(value["status"], "success");
    assert_eq!(value["data"]["ok"], true);
    value
}

async fn start_entry_point(
    app: &axum::Router,
    op: &str,
    config: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": op,
                "op_version": 2,
                "config_format": "json",
                "config": config,
            }),
        ))
        .await
        .expect("entry-point start response");
    let status = response.status();
    (status, response_json(response).await)
}

async fn start_portfolio_snapshot(app: &axum::Router) -> (serde_json::Value, RunId) {
    let response = app
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
        .expect("portfolio snapshot start response");
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["outcome"], "admitted");
    let run_id = RunId::parse(body["data"]["run"]["run_id"].as_str().expect("run id"))
        .expect("typed run id");
    (body, run_id)
}

fn event_index(
    stream: &[store::KernelEventEnvelope],
    label: &str,
    matches_payload: impl Fn(&events::KernelEventPayload) -> bool,
) -> usize {
    stream
        .iter()
        .position(|event| matches_payload(event.payload()))
        .unwrap_or_else(|| panic!("{label}"))
}

fn first_failed_attempt(
    stream: &[store::KernelEventEnvelope],
) -> (usize, &events::StateAttemptFailed) {
    let index = event_index(stream, "StateAttemptFailed", |payload| {
        matches!(payload, events::KernelEventPayload::StateAttemptFailed(_))
    });
    let events::KernelEventPayload::StateAttemptFailed(failed) = stream[index].payload() else {
        panic!("expected failed attempt");
    };
    (index, failed)
}

async fn assert_evm_start_fails_before_admission(
    state: &test_support::InMemoryRestAppState,
    app: &axum::Router,
    op: &str,
    config: serde_json::Value,
    message: &str,
) {
    let prepared = test_support::prepare_entry_point_launch_for_store(
        &state.store,
        op,
        Some(mfm_app::OpVersion::new(2).expect("op version")),
        &config,
        None,
    )
    .await;
    assert_evm_entry_point_evidence(&prepared.evidence, op);

    let (status, body) = start_entry_point(app, op, config).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["code"], "LaunchRunnerUnavailable");

    let stream = state
        .store
        .load_run_stream(&prepared.request.run_id)
        .await
        .expect("run stream");
    assert!(stream.is_empty(), "{message}");
}

async fn assert_no_run_observations(state: &test_support::InMemoryRestAppState, message: &str) {
    let page = state
        .store
        .read_run_observations(store::RunObservationQuery::new(None, 100, 0))
        .await
        .expect("run observations");
    assert!(page.runs.is_empty(), "{message}: {:?}", page.runs);
}

async fn assert_absent_run_routes_not_found(app: &axum::Router, include_resume: bool) {
    let mut requests = vec![(
        get(format!("/v1/runs/{VALID_RUN_ID}/status")),
        "status response",
    )];
    if include_resume {
        requests.push((
            empty_post(&format!("/v1/runs/{VALID_RUN_ID}/resume")),
            "resume response",
        ));
    }
    requests.extend([
        (
            empty_post(&format!("/v1/runs/{VALID_RUN_ID}/replay")),
            "replay response",
        ),
        (
            get(format!(
                "/v1/runs/{VALID_RUN_ID}/public-output/{VALID_SCHEMA_ID}"
            )),
            "public output response",
        ),
    ]);
    for (request, label) in requests {
        let response = app.clone().oneshot(request).await.expect(label);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn health_and_ready_endpoints_report_liveness_and_readiness() {
    get_success("/v1/health").await;

    let ready = get_success("/v1/ready").await;
    assert_eq!(ready["data"]["checks"]["run_store"], "ready");
}

#[tokio::test]
async fn start_rejects_invalid_json_with_stable_envelope() {
    assert_start_request_error(
        Request::builder()
            .method("POST")
            .uri("/v1/runs/start")
            .header("content-type", "application/json")
            .body(Body::from("not json"))
            .expect("request"),
        "InvalidJson",
    )
    .await;
}

#[tokio::test]
async fn start_validates_entry_point_request_shapes() {
    for (body, code) in [
        (serde_json::json!({ "unexpected": true }), "InvalidJson"),
        (
            serde_json::json!({
                "op": "missing_contract_test_op",
                "config": "portfolio_id = \"main\"\n",
            }),
            "EntryPointOpNotFound",
        ),
        (
            serde_json::json!({
                "op": "missing_contract_test_op",
                "op_version": 1,
                "config_format": "toml",
                "config": "portfolio_id = \"main\"\n",
            }),
            "EntryPointOpNotFound",
        ),
        (
            serde_json::json!({
                "op": "missing_contract_test_op",
                "config_format": "json",
                "config": {
                    "portfolio_id": "main",
                    "wallets": []
                },
            }),
            "EntryPointOpNotFound",
        ),
        (
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": {
                    "portfolio_id": "main",
                    "wallets": []
                },
                "run_id": VALID_RUN_ID,
            }),
            "InvalidJson",
        ),
    ] {
        assert_start_json_error(body, code).await;
    }
}

#[tokio::test]
async fn start_invocation_key_derives_separate_run_without_persisting_raw_key() {
    let rpc_url = test_support::start_portfolio_rpc_mock(31337).await;
    let runtime = runtime_config_app(PORTFOLIO_NETWORK_ID, &rpc_url);
    let app = runtime.app.clone();
    let store = &runtime.state.store;
    let raw_key = "invocation-alpha";
    let first_run_id =
        test_support::prepare_portfolio_launch_for_store(store, &portfolio_snapshot_config(), None)
            .await
            .request
            .run_id;

    let response = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": portfolio_snapshot_config(),
                "invocation_key": raw_key,
            }),
        ))
        .await
        .expect("invocation start response");
    assert_eq!(response.status(), StatusCode::OK);
    let response_body = response_json(response).await;
    assert_eq!(response_body["data"]["outcome"], "admitted");
    assert!(!response_body.to_string().contains(raw_key));
    let invocation_run_id = RunId::parse(
        response_body["data"]["run"]["run_id"]
            .as_str()
            .expect("invocation id"),
    )
    .expect("typed invocation id");
    assert_ne!(first_run_id, invocation_run_id);

    let stream = store
        .load_run_stream(&invocation_run_id)
        .await
        .expect("invocation stream");
    let admitted = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(payload),
            _ => None,
        })
        .expect("RunAdmitted");
    assert!(admitted
        .identity_material
        .invocation_key_digest
        .as_str()
        .starts_with("content:sha256-jcs-v1:"));
    assert!(!format!("{admitted:?}").contains(raw_key));
}

#[tokio::test]
async fn portfolio_report_hard_fails_without_platform_facts_after_admission() {
    // Report-only cutover: live chain mismatch diagnostics are gone. Without admitted
    // Platform holding facts, SelectHoldings hard-fails after RunAdmitted.
    let state = in_memory_state();
    let app = mfm_rest_api::make_app(state.clone());
    let store = &state.store;

    let (body, run_id) = start_portfolio_snapshot(&app).await;
    assert_ne!(body["data"]["run"]["run_mode"], "completed");

    let stream = store.load_run_stream(&run_id).await.expect("stream");
    let admitted_index = event_index(&stream, "RunAdmitted", |payload| {
        matches!(payload, events::KernelEventPayload::RunAdmitted(_))
    });
    let (failed_index, failed) = first_failed_attempt(&stream);
    assert!(admitted_index < failed_index);
    // Redaction-safe: no RPC URL / runtime path leakage on missing facts.
    let rendered = format!("{body:?} {stream:?} {failed:?}");
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("rpc_url"));

    let replay = app
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/replay")))
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::OK);
}

#[tokio::test]
async fn portfolio_select_holdings_missing_facts_is_attempt_failure() {
    let state = in_memory_state();
    let app = mfm_rest_api::make_app(state.clone());
    let store = &state.store;

    let (_, run_id) = start_portfolio_snapshot(&app).await;
    let stream = store.load_run_stream(&run_id).await.expect("stream");
    let admitted_index = event_index(&stream, "RunAdmitted", |payload| {
        matches!(payload, events::KernelEventPayload::RunAdmitted(_))
    });
    let (failed_index, _failed) = first_failed_attempt(&stream);
    assert!(admitted_index < failed_index);

    // Live pin_views / observe_batch must not appear after cutover.
    for event in &stream {
        if let events::KernelEventPayload::StateAttemptStarted(payload) = event.payload() {
            let kind = payload
                .state_kind
                .canonical_name()
                .unwrap_or_else(|| payload.state_kind.as_str());
            assert!(
                !kind.contains("pin_views")
                    && !kind.contains("observe_batch")
                    && !kind.contains("merge_observations"),
                "deleted live portfolio states must not run: {kind}"
            );
        }
    }

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
        assert_evm_start_fails_before_admission(
            &state,
            &app,
            op,
            config,
            &format!("{op} must fail capability ingress before RunAdmitted"),
        )
        .await;
    }
}

#[tokio::test]
async fn evm_contract_start_rejects_old_configure_validate_envelopes_before_admission() {
    let state = in_memory_state();
    let app = mfm_rest_api::make_app(state.clone());

    for (op, config) in [
        (
            "evm_contract_configure",
            evm_old_configure_entry_config_json(),
        ),
        (
            "evm_contract_validate",
            evm_old_validate_entry_config_json(),
        ),
    ] {
        let (status, body) = start_entry_point(&app, op, config).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["status"], "error");
        assert_eq!(body["error"]["code"], "AuthoredConfigDecodeFailed");
        assert_no_run_observations(&state, &format!("{op} old envelope must not admit a run"))
            .await;
    }
}

#[tokio::test]
async fn evm_contract_validation_rejects_present_malformed_signers_before_admission() {
    let rpc_url = test_support::start_portfolio_rpc_mock(1).await;
    let runtime = malformed_signer_runtime_config_app("ethereum-mainnet", &rpc_url);

    let (status, body) = start_entry_point(
        &runtime.app,
        "evm_contract_validate",
        evm_validate_entry_config_json(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["code"], "LaunchRunnerUnavailable");
    assert_no_run_observations(
        &runtime.state,
        "malformed runtime config must fail before run admission",
    )
    .await;
}

#[tokio::test]
async fn evm_contract_mutation_signer_ingress_failures_happen_before_admission() {
    let rpc_url = test_support::start_portfolio_rpc_mock(1).await;

    for (runtime, message) in [
        (
            runtime_config_app("ethereum-mainnet", &rpc_url),
            "missing signer must fail before admission",
        ),
        (
            malformed_signer_runtime_config_app("ethereum-mainnet", &rpc_url),
            "malformed signer config must fail before admission",
        ),
    ] {
        assert_evm_start_fails_before_admission(
            &runtime.state,
            &runtime.app,
            "evm_contract_deploy",
            evm_deploy_config_json(),
            message,
        )
        .await;
    }
}

#[tokio::test]
async fn portfolio_status_route_reports_interrupted_attempt_and_framework_attempts_from_history() {
    // Report-only: no live RPC required to admit/resume. Without Platform facts the
    // resumed run hard-fails (does not complete with a public snapshot).
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
    // Without Platform holding facts the report path hard-fails after cutover.
    assert_ne!(resume_body["data"]["run_mode"], "completed");

    let status = app
        .clone()
        .oneshot(get(format!("/v1/runs/{run_id}/status")))
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

    // After cutover, resume without Platform facts hard-fails; framework public-output
    // completion is not required. Status history must still surface the interrupted attempt.
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt["disposition"] == "failed"
                || attempt["disposition"] == "interrupted"
                || attempt["disposition"] == "completed"),
        "expected attempt dispositions after resume: {attempts:?}"
    );

    let stream = app
        .oneshot(get(format!("/v1/runs/{run_id}/stream")))
        .await
        .expect("stream response");
    assert_eq!(stream.status(), StatusCode::OK);
    let stream_body = response_json(stream).await;
    assert_eq!(stream_body["status"], "success");
    let stream_events = stream_body["data"]["events"]
        .as_array()
        .expect("stream events");
    assert!(
        !stream_events.is_empty(),
        "stream must retain history after failed report resume"
    );
}

#[tokio::test]
async fn status_invalid_run_id_has_domain_error() {
    let app = test_app();

    let resp = app
        .oneshot(get("/v1/runs/not-a-uuid/status"))
        .await
        .expect("response");

    assert_error_response(resp, StatusCode::BAD_REQUEST, "InvalidRunId").await;
}

#[tokio::test]
async fn absent_run_status_resume_replay_and_public_output_are_not_found() {
    let app = test_app();

    assert_absent_run_routes_not_found(&app, true).await;
}

#[tokio::test]
async fn malformed_runtime_config_does_not_block_read_only_routes() {
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = runtime_config_dir.path().join("malformed-runtime.toml");
    std::fs::write(&runtime_config_path, "not valid toml = [").expect("runtime config");
    let (_, app) = state_and_app_with_runtime_config(runtime_config_path.clone());

    let health = app
        .clone()
        .oneshot(get("/v1/health"))
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);

    assert_absent_run_routes_not_found(&app, false).await;

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
        .oneshot(get(format!(
            "/v1/runs/{VALID_RUN_ID}/stream?from_seq=3&to_seq=2"
        )))
        .await
        .expect("response");

    assert_error_response(resp, StatusCode::BAD_REQUEST, "InvalidSequenceRange").await;
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

fn evm_content_digest_str(byte: u8) -> String {
    format!("content:sha256-jcs-v1:{}", format!("{byte:02x}").repeat(32))
}

fn evm_context_json() -> serde_json::Value {
    serde_json::json!({
        "lifecycle_key": "rest-test-lifecycle",
        "network": {
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "chain_fingerprint": null,
            "finality_or_observation_policy": null,
        },
        "contract_profile": {
            "profile_id": "rest-test-contract",
            "artifact_digest": evm_content_digest_str(0x20),
            "interface_digest": evm_content_digest_str(0x21),
            "creation_bytecode_digest": null,
            "deployed_code_hash": null,
            "selector_event_policy_digest": null,
        },
    })
}

fn evm_signer_json() -> serde_json::Value {
    serde_json::json!({
        "signer_ref": "deployer",
        "expected_signer_address": "0x000000000000000000000000000000000000dead",
    })
}

fn evm_deploy_action_json() -> serde_json::Value {
    serde_json::json!({
        "signer": evm_signer_json(),
    })
}

fn evm_configure_action_json() -> serde_json::Value {
    serde_json::json!({
        "signer": evm_signer_json(),
        "calls": [],
    })
}

fn evm_validate_action_json() -> serde_json::Value {
    serde_json::json!({})
}

fn evm_deploy_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": evm_context_json(),
        "deploy": evm_deploy_action_json(),
    })
}

fn evm_lifecycle_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": evm_context_json(),
        "deploy": evm_deploy_action_json(),
        "configure": evm_configure_action_json(),
        "validate": evm_validate_action_json(),
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
[signers.deployer]
provider = "raw-private-key"
entry_id = "not-a-uuid"
private_key = "placeholder-private-key-value"
"#,
    );
    std::fs::write(&path, config).expect("write malformed signer runtime config");
    path
}

fn evm_import_deployed_json() -> serde_json::Value {
    serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000dead",
            "provenance_label": "rest-test-external",
            "evidence_policy": {
                "require_code": false
            }
        },
    })
}

fn evm_import_configured_json() -> serde_json::Value {
    serde_json::json!({
        "kind": "adopt_external_address",
        "adoption": {
            "address": "0x000000000000000000000000000000000000dead",
            "provenance_label": "rest-test-external",
            "evidence_policy": {
                "require_code": false,
                "allow_external_claimed_configured": true
            }
        },
    })
}

fn evm_configure_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": evm_context_json(),
        "import_deployed": evm_import_deployed_json(),
        "configure": evm_configure_action_json(),
    })
}

fn evm_validate_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "context": evm_context_json(),
        "import_configured": evm_import_configured_json(),
        "validate": evm_validate_action_json(),
    })
}

fn evm_old_network_json() -> serde_json::Value {
    serde_json::json!({
        "network_id": "ethereum-mainnet",
        "expected_chain_id": 1,
    })
}

fn evm_old_deployed_contract_json() -> serde_json::Value {
    serde_json::json!({
        "network_id": "ethereum-mainnet",
        "expected_chain_id": 1,
        "contract_address": "0x000000000000000000000000000000000000dead",
    })
}

fn evm_old_configured_contract_json() -> serde_json::Value {
    serde_json::json!({
        "deployed": evm_old_deployed_contract_json(),
        "network_id": "ethereum-mainnet",
        "expected_chain_id": 1,
        "contract_address": "0x000000000000000000000000000000000000dead",
    })
}

fn evm_old_configure_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "config": {
            "network": evm_old_network_json(),
            "signer": evm_signer_json(),
            "calls": [],
        },
        "deployed": evm_old_deployed_contract_json(),
    })
}

fn evm_old_validate_entry_config_json() -> serde_json::Value {
    serde_json::json!({
        "config": {
            "network": evm_old_network_json(),
            "read_assertions": [],
            "event_assertions": [],
        },
        "configured": evm_old_configured_contract_json(),
    })
}

fn assert_evm_entry_point_evidence(evidence: &mfm_app::EntryPointLaunchEvidence, op_name: &str) {
    let registry = mfm_app::production_entry_point_op_registry().expect("entry-point registry");
    let public_name = mfm_app::PublicOpName::new(op_name).expect("public op name");
    let op = registry
        .resolve(
            &public_name,
            Some(mfm_app::OpVersion::new(2).expect("op version")),
        )
        .expect("EVM entry-point op");

    assert_eq!(evidence.resolved_op_id, op.op_id());
    assert_eq!(
        evidence.entry_point_registry_digest,
        registry.registry_digest().expect("registry digest")
    );
}
