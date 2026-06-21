#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_events::v1 as events;
use mfm_ids::{AttemptId, DigestAlgorithm, DigestBytes, RunId, SpecHash};
use mfm_integration_tests::test_support;
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, PortfolioSnapshotAuthoredConfig,
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
const PORTFOLIO_NETWORK_ID: &str = "rest-control-eth";
static RPC_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
                "drive": "append_only"
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
                "drive": "append_only"
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
                "drive": "append_only"
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
async fn status_route_reports_interrupted_attempt_and_framework_attempts_from_history() {
    let _env_guard = RPC_ENV_LOCK.lock().await;
    let rpc_url = start_rpc_mock().await;
    let _env_restore = set_rpc_env(rpc_url);
    let (root, state) = in_memory_state_with_root();
    let config = portfolio_snapshot_config();
    let certified = certified_portfolio_spec_for_config(&config);
    let run_id = mfm_app::new_run_id();
    let app = mfm_rest_api::make_app(state.clone());

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": config,
                "run_id": run_id.as_str(),
                "framework_version": "mfm.integration.rest.portfolio.typed.v1",
                "source_revision": "integration-test",
                "drive": "append_only"
            }),
        ))
        .await
        .expect("start response");
    assert_eq!(start.status(), StatusCode::OK);
    let start_body = response_json(start).await;
    assert_eq!(start_body["status"], "success");
    assert_eq!(start_body["data"]["run"]["run_mode"], "forward");

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

async fn start_rpc_mock() -> String {
    let app = axum::Router::new().route("/", axum::routing::post(rpc_handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc mock");
    let addr = listener.local_addr().expect("rpc mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("rpc mock serve");
    });
    format!("http://{addr}")
}

async fn rpc_handler(
    axum::Json(request): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let id = request
        .get("id")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(1));
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .expect("json-rpc method");
    let result = match method {
        "eth_chainId" => serde_json::json!("0x7a69"),
        "eth_getBlockByNumber" => serde_json::json!({
            "number": "0x64",
            "hash": "0x1111111111111111111111111111111111111111111111111111111111111111"
        }),
        "eth_getBalance" => serde_json::json!("0xde0b6b3a7640000"),
        other => panic!("unexpected rpc method {other}"),
    };
    axum::Json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
}

fn set_rpc_env(rpc_url: String) -> EnvVarRestore {
    let previous = std::env::var("MFM_EVM_RPC_SOURCES_JSON").ok();
    std::env::set_var(
        "MFM_EVM_RPC_SOURCES_JSON",
        serde_json::json!({
            "sources": [
                {
                    "id": PORTFOLIO_NETWORK_ID,
                    "expected_chain_id": 31337,
                    "rpc_url": rpc_url,
                    "authorization": null
                }
            ],
            "policies": [
                {
                    "id": PORTFOLIO_NETWORK_ID,
                    "ordered_sources": [PORTFOLIO_NETWORK_ID]
                }
            ]
        })
        .to_string(),
    );
    EnvVarRestore {
        name: "MFM_EVM_RPC_SOURCES_JSON",
        previous,
    }
}

struct EnvVarRestore {
    name: &'static str,
    previous: Option<String>,
}

impl Drop for EnvVarRestore {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.name, value),
            None => std::env::remove_var(self.name),
        }
    }
}

fn certified_portfolio_spec_for_config(
    config: &serde_json::Value,
) -> mfm_certify::CertifiedTypedSpec {
    let authored: PortfolioSnapshotAuthoredConfig =
        serde_json::from_value(config.clone()).expect("portfolio authored config");
    let canonical = canonicalize_portfolio_snapshot_authored_config(authored)
        .expect("portfolio canonical config");
    mfm_op_portfolio_tracker::certified_portfolio_spec(canonical.into())
        .expect("certified portfolio spec")
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
