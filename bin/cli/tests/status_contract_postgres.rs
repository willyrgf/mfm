#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use assert_cmd::Command;
use mfm_app::{ProductionPostgresSchema, ProductionRunStore};
use mfm_events::v1 as events;
use mfm_ids::{AttemptId, DigestAlgorithm, DigestBytes, RunId, SpecHash};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use serde_json::Value;
use std::process::Output;
use tempfile::TempDir;

const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";
const PORTFOLIO_NETWORK_ID: &str = "ethereum-mainnet";
static RPC_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn run_status_reports_interrupted_attempt_and_framework_attempts_from_history() {
    let _rpc_env_guard = RPC_ENV_LOCK.lock().await;
    let rpc_url = start_rpc_mock();
    let _rpc_restore = set_rpc_env(rpc_url);
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");

    ProductionPostgresSchema::migrate(&database_url)
        .await
        .expect("migrate typed postgres schema");
    let store = ProductionRunStore::connect(&database_url)
        .await
        .expect("connect typed postgres store");
    let temp = TempDir::new().expect("temp dir");
    let config_path = temp.path().join("portfolio.json");
    let config = sample_portfolio_config_json();
    std::fs::write(&config_path, &config).expect("write portfolio config");
    let run_id = mfm_app::new_run_id();
    let entry_point_registry = mfm_app::production_entry_point_op_registry().expect("entrypoints");
    let certification_registry = mfm_app::production_certification_registry().expect("cert");
    let prepared = mfm_app::prepare_entry_point_run_launch(mfm_app::EntryPointRunLaunchInput {
        entry_point_registry: &entry_point_registry,
        public_op_name: mfm_app::PublicOpName::new("portfolio_snapshot").expect("op name"),
        op_version: None,
        authored_config: mfm_authored_config::AuthoredConfig::new(
            mfm_authored_config::AuthoredConfigFormat::Json,
            config,
        )
        .expect("authored config"),
        certification_registry: &certification_registry,
        run_id: run_id.clone(),
        drive: mfm_app::DriveMode::AppendOnly,
    })
    .expect("prepared entry-point launch");
    let certified = prepared.request.certified_spec.clone();

    let start_args = vec![
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "start".to_owned(),
        "--op".to_owned(),
        "portfolio_snapshot".to_owned(),
        "--config".to_owned(),
        config_path.display().to_string(),
        "--config-format".to_owned(),
        "json".to_owned(),
        "--run-id".to_owned(),
        run_id.as_str().to_owned(),
        "--drive".to_owned(),
        "append-only".to_owned(),
        "--database-url".to_owned(),
        database_url.clone(),
    ];
    let start = run_cli(&start_args);
    assert_success(&start);
    let start_json = parse_success_json(&start.stdout);
    assert_eq!(start_json["run"]["run_mode"], "forward");

    let interrupted_node = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("domain node");
    let interrupted_attempt_id = fixed_attempt_id(0x51);
    append_interrupted_attempt(
        &store,
        &run_id,
        certified.spec_hash(),
        interrupted_node,
        &interrupted_attempt_id,
    )
    .await;

    let resume = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "resume".to_owned(),
        run_id.as_str().to_owned(),
        "--drive".to_owned(),
        "until-blocked".to_owned(),
        "--database-url".to_owned(),
        database_url.clone(),
    ]);
    assert_success(&resume);
    let resume_json = parse_success_json(&resume.stdout);
    assert_eq!(resume_json["run_mode"], "completed");

    let status = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "status".to_owned(),
        run_id.as_str().to_owned(),
        "--database-url".to_owned(),
        database_url.clone(),
    ]);
    assert_success(&status);
    let status_json = parse_success_json(&status.stdout);
    assert_ne!(status_json["run_mode"], "interrupted");
    let attempts = status_json["attempt_dispositions"]
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

    let stream = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "stream".to_owned(),
        run_id.as_str().to_owned(),
        "--database-url".to_owned(),
        database_url,
    ]);
    assert_success(&stream);
    let stream_json = parse_success_json(&stream.stdout);
    let stream_events = stream_json["events"].as_array().expect("stream events");
    assert_framework_started_before_terminal_evidence(
        stream_events,
        attempts,
        &certified.envelope().spec.nodes,
        &run_id,
    );
}

fn sample_portfolio_config_json() -> String {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "chain_id": 1,
                    "control_scope": "shared",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "implementation": { "kind": "address_only" },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1800.00"
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
        "valuation_source_registry": { "sources": [] }
    })
    .to_string()
}

fn start_rpc_mock() -> String {
    let app = axum::Router::new().route("/", axum::routing::post(rpc_handler));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind rpc mock");
    let addr = listener.local_addr().expect("rpc mock addr");
    listener
        .set_nonblocking(true)
        .expect("set rpc mock nonblocking");
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rpc mock runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("tokio rpc listener");
            axum::serve(listener, app).await.expect("rpc mock serve");
        });
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
        "eth_chainId" => serde_json::json!("0x1"),
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
    let previous = std::env::var(ENV_EVM_RPC_SOURCES_JSON).ok();
    std::env::set_var(
        ENV_EVM_RPC_SOURCES_JSON,
        serde_json::json!({
            "sources": [
                {
                    "id": PORTFOLIO_NETWORK_ID,
                    "expected_chain_id": 1,
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
        name: ENV_EVM_RPC_SOURCES_JSON,
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

async fn append_interrupted_attempt(
    store: &ProductionRunStore,
    run_id: &RunId,
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) {
    let start = store::CommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .expect("expected next seq"),
        store::CommitKey::new("cli-interrupted-attempt-start").expect("commit key"),
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
    append_run_commit(store, start).await;

    let interrupted = store::CommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .expect("expected next seq"),
        store::CommitKey::new("cli-interrupted-attempt-terminal").expect("commit key"),
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
    append_run_commit(store, interrupted).await;
}

async fn append_run_commit(store: &ProductionRunStore, request: store::CommitRequest) {
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
    let bundle = test_bundle_from_plan(plan).expect("prepared commit bundle");
    store
        .append_prepared_commit_bundle(bundle)
        .await
        .expect("append typed commit");
}

fn test_bundle_from_plan(
    plan: store::PreparedCommitPlan,
) -> store::Result<store::PreparedCommitBundle> {
    let existing = plan
        .admitted_artifacts()
        .iter()
        .map(|evidence| {
            Ok(store::ExistingArtifactAdmission::new(
                evidence.artifact_id.clone(),
                evidence.evidence_hash()?,
            ))
        })
        .collect::<store::Result<Vec<_>>>()?;
    store::PreparedCommitBundle::new(plan, Vec::new(), existing)
}

fn fixed_attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn run_cli(args: &[String]) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .args(args)
        .output()
        .expect("execute mfm_cli")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_success_json(stdout: &[u8]) -> Value {
    let parsed: Value = serde_json::from_slice(stdout).expect("stdout must be valid json");
    assert_eq!(parsed["status"], "success");
    parsed["data"].clone()
}

fn sanitize_machine_readable_cli_env(cmd: &mut Command) -> &mut Command {
    cmd.env_remove("LOG_LEVEL")
        .env_remove("RUST_LOG")
        .env_remove("LOG_FORMAT")
        .env_remove("LOG_SPAN_EVENTS")
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
