#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_integration_tests::rpc_control;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{RunPhase, Stores};
use mfm_machine::errors::ContextError;
use mfm_machine::events::{event_envelopes_from_stream_records, Event, KernelEvent};
use mfm_machine::ids::{ContextKey, OpId, RunId};
use mfm_machine::stores::{ArtifactStore, StreamId, StreamStore};
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::DefaultRunLauncher;
use mfm_stream_store_postgres::PostgresStreamStore;
use mfm_transports_rpc_control::RpcControlBootstrapSource;

const NETWORK_ID: &str = "ethereum-mainnet";
const CONTROL_SCOPE: &str = "parity.portfolio_tracker.mock_erc20";

#[derive(Default)]
struct MapContext {
    inner: HashMap<String, serde_json::Value>,
}

impl DynContext for MapContext {
    fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
        Ok(self.inner.get(&key.0).cloned())
    }

    fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
        self.inner.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
        self.inner.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, ContextError> {
        let mut out = serde_json::Map::new();
        for (k, v) in &self.inner {
            out.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(out))
    }
}

fn snapshot_value_with_slot_fallback<'a>(
    snapshot: &'a serde_json::Value,
    key: &str,
) -> Option<&'a serde_json::Value> {
    let mut candidates = vec![key.to_string()];
    if !key.contains(".in.") && !key.contains(".out.") && !key.contains(".work.") {
        if let Some((prefix, leaf)) = key.rsplit_once('.') {
            candidates.push(format!("{prefix}.out.{leaf}"));
            candidates.push(format!("{prefix}.work.{leaf}"));
        }
    }

    candidates
        .into_iter()
        .find_map(|candidate| snapshot.get(&candidate))
}

fn required_snapshot_value<'a>(
    snapshot: &'a serde_json::Value,
    key: &str,
) -> &'a serde_json::Value {
    snapshot_value_with_slot_fallback(snapshot, key).unwrap_or_else(|| {
        let keys = snapshot
            .as_object()
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        panic!("missing snapshot key `{key}`; keys={keys:?}");
    })
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let s = serde_json::to_string(&body).expect("json request must serialize");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(s))
        .expect("request")
}

fn canonical_mock_erc20_snapshot_payload(
    wallet_address: &str,
    chain_id: u64,
    token_address: &str,
    control_scope: &str,
) -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "reth-mock-erc20",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "chain_id": chain_id,
                    "control_scope": control_scope,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_mainnet",
                    "address": wallet_address,
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": [
                        "eth.native.ethereum-mainnet",
                        "mock.wallet.ethereum-mainnet"
                    ],
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
                    "balance_reader": {
                        "kind": "native_balance"
                    },
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
                },
                {
                    "symbol_id": "mock.wallet.ethereum-mainnet",
                    "display_symbol": "MOCK",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": token_address
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "mock.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1.00"
                                }
                            }
                        ]
                    },
                    "decimals": null,
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

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
}

fn find_quote_total<'a>(totals: &'a serde_json::Value, quote: &str) -> &'a serde_json::Value {
    totals
        .as_array()
        .expect("quote totals array")
        .iter()
        .find(|total| total["quote"] == quote)
        .unwrap_or_else(|| panic!("missing quote total for {quote}"))
}

fn run_config_with_allowlist(allowlist: Vec<String>) -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff: BackoffPolicy::Fixed {
                delay: Duration::from_millis(0),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
        nix_flake_allowlist: allowlist,
    }
}

fn parse_u64_hex(s: &str) -> u64 {
    let trimmed = s
        .strip_prefix("0x")
        .expect("hex strings must start with 0x");
    u64::from_str_radix(trimmed, 16).expect("hex string must parse as u64")
}

fn normalize_address_lower(s: &str) -> String {
    let s = s.trim();
    let rest = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .expect("0x");
    assert_eq!(rest.len(), 40, "address must be 20 bytes hex");
    assert!(
        rest.chars().all(|c| c.is_ascii_hexdigit()),
        "address must be hex"
    );
    format!("0x{}", rest.to_ascii_lowercase())
}

fn contract_artifact_program_path_mock_erc20() -> String {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v mfm-contract-artifact-mock-erc20")
        .output()
        .expect("resolve mfm-contract-artifact-mock-erc20 path");
    assert!(
        out.status.success(),
        "mfm-contract-artifact-mock-erc20 must be in PATH"
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

async fn rpc_call(
    rpc_sources: &[RpcControlBootstrapSource],
    control_scope: &str,
    streams: Arc<dyn StreamStore>,
    artifacts: Arc<dyn ArtifactStore>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    rpc_control::call_in_scope(
        rpc_sources,
        NETWORK_ID,
        control_scope,
        streams,
        artifacts,
        method,
        params,
    )
    .await
}

async fn connect_postgres_with_retry(max_attempts: u32, delay_ms: u64) -> PostgresStreamStore {
    let mut last_err: Option<mfm_machine::errors::StorageError> = None;
    for _ in 0..max_attempts {
        match PostgresStreamStore::connect_env().await {
            Ok(pg) => return pg,
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "<missing>".to_string());
    panic!(
        "postgres config after retries (DATABASE_URL={}): {:?}",
        db_url, last_err
    );
}

fn summarize_exec_error_details(details: Option<&serde_json::Value>) -> Option<String> {
    let obj = details?.as_object()?;
    let mut parts = Vec::new();

    if let Some(program_path) = obj.get("program_path").and_then(|v| v.as_str()) {
        parts.push(format!("program_path={program_path}"));
    }
    if let Some(timeout_ms) = obj.get("timeout_ms").and_then(|v| v.as_u64()) {
        parts.push(format!("timeout_ms={timeout_ms}"));
    }
    if let Some(exit_code) = obj.get("exit_code").and_then(|v| v.as_i64()) {
        parts.push(format!("exit_code={exit_code}"));
    }
    if let Some(signal) = obj.get("signal").and_then(|v| v.as_i64()) {
        parts.push(format!("signal={signal}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

async fn run_failure_diagnostics(streams: Arc<dyn StreamStore>, run_id: RunId) -> String {
    let stream = match streams
        .read_range(&StreamId::run(run_id), 1, None)
        .await
        .and_then(|records| event_envelopes_from_stream_records(run_id, records))
    {
        Ok(stream) => stream,
        Err(err) => {
            return format!("run_id={} read_range_failed={err:?}", run_id.0);
        }
    };

    let mut last_state_entered: Option<(u64, String, u32)> = None;
    let mut last_state_failed: Option<(u64, String, String, bool, String, Option<String>)> = None;

    for envelope in stream {
        match envelope.event {
            Event::Kernel(KernelEvent::StateEntered {
                state_id, attempt, ..
            }) => {
                last_state_entered = Some((envelope.seq, state_id.to_string(), attempt));
            }
            Event::Kernel(KernelEvent::StateFailed {
                state_id, error, ..
            }) => {
                let detail_summary = summarize_exec_error_details(error.info.details.as_ref());
                last_state_failed = Some((
                    envelope.seq,
                    state_id.to_string(),
                    error.info.code.0,
                    error.info.retryable,
                    error.info.message,
                    detail_summary,
                ));
            }
            _ => {}
        }
    }

    let mut parts = vec![format!("run_id={}", run_id.0)];
    if let Some((seq, state_id, attempt)) = last_state_entered {
        parts.push(format!(
            "last_state_entered={state_id} attempt={attempt} seq={seq}"
        ));
    }
    if let Some((seq, state_id, code, retryable, message, detail_summary)) = last_state_failed {
        parts.push(format!(
            "state_failed={state_id} seq={seq} code={code} retryable={retryable} message={message}"
        ));
        if let Some(details) = detail_summary {
            parts.push(format!("state_failed_details={details}"));
        }
    } else {
        parts.push("no_state_failed_event_found".to_string());
    }

    parts.join("; ")
}

const RETH_DEV_ACCOUNT0_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[tokio::test]
async fn parity_portfolio_tracker_snapshot_with_mock_erc20_mint() {
    let pg = connect_postgres_with_retry(20, 250).await;
    let streams: Arc<dyn StreamStore> = Arc::new(pg);

    let s3 = S3ArtifactStore::from_env().expect("s3 config");
    s3.ensure_bucket_exists().await.expect("bucket exists");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(s3);

    let rpc_sources = rpc_control::required_bootstrap_sources_from_env_for_network(NETWORK_ID);
    let control_scope = format!("{CONTROL_SCOPE}.{}", uuid::Uuid::new_v4().simple());
    let bootstrap_control_scope = format!("{control_scope}.bootstrap");

    let accounts = rpc_call(
        &rpc_sources,
        &bootstrap_control_scope,
        Arc::clone(&streams),
        Arc::clone(&artifacts),
        "eth_accounts",
        serde_json::json!([]),
    )
    .await;
    let from = accounts
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .expect("eth_accounts first address")
        .to_string();
    let from_norm = normalize_address_lower(&from);

    // Used only by the deploy step (signed raw tx). `evm_configure` uses `eth_sendTransaction`.
    let signing_key_env = "MFM_EVM_PARITY_DEPLOY_SIGNING_KEY";
    std::env::set_var(signing_key_env, RETH_DEV_ACCOUNT0_PRIVATE_KEY);

    let chain_id_hex = rpc_call(
        &rpc_sources,
        &bootstrap_control_scope,
        Arc::clone(&streams),
        Arc::clone(&artifacts),
        "eth_chainId",
        serde_json::json!([]),
    )
    .await;
    let chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");

    let contract_program_path = contract_artifact_program_path_mock_erc20();

    // 1) Deploy + mint a MockERC20.
    let setup_pipeline = Pipeline {
        machine_id: MachineId("evm_mock_erc20_setup".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("fetch".to_string()),
                op_id: OpId::must_new("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "program_path": contract_program_path,
                    "stdin_json": {},
                    "timeout_ms": 300000,
                    "write_result_to": "result",
                }),
            },
            PipelineStep {
                step_id: StepId("adapt".to_string()),
                op_id: OpId::must_new("evm_contract_from_nix".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "result_pointer": "/artifact"
                }),
            },
            PipelineStep {
                step_id: StepId("deploy".to_string()),
                op_id: OpId::must_new("evm_deploy".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact_port": "contract_artifact",
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope.as_str(),
                    "from": from,
                    "signing_key_env": signing_key_env,
                    "constructor_args": ["MockToken", "MOCK", 6],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("mint".to_string()),
                op_id: OpId::must_new("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact_port": "contract_artifact",
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope.as_str(),
                    "from": from_norm,
                    "calls": [
                        {"function": "mint", "args": [from_norm, 1000000]}
                    ],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
        ],
    };

    let run_config = run_config_with_allowlist(mfm_machine::config::default_nix_flake_allowlist());
    let bundle = mfm_rest_api::make_engine_bundle();
    let launcher = DefaultRunLauncher;
    let setup_run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                streams: Arc::clone(&streams),
                artifacts: Arc::clone(&artifacts),
            },
            Arc::clone(&bundle.registry),
            Arc::clone(&bundle.planner),
            LaunchPipeline {
                pipeline: setup_pipeline,
                input: serde_json::json!({}),
                run_config: run_config.clone(),
                build: BuildProvenance {
                    git_commit: None,
                    cargo_lock_hash: None,
                    flake_lock_hash: None,
                    rustc_version: None,
                    target_triple: None,
                    env_allowlist: Vec::new(),
                },
                initial_context: Box::new(MapContext::default()),
            },
        )
        .await
        .expect("start setup pipeline");

    if setup_run.phase != RunPhase::Completed {
        let diagnostics = run_failure_diagnostics(Arc::clone(&streams), setup_run.run_id).await;
        panic!("setup pipeline failed: {diagnostics}");
    }

    let setup_snapshot_id = setup_run.final_snapshot_id.expect("final snapshot");

    let snapshot_bytes = artifacts
        .get(&setup_snapshot_id)
        .await
        .expect("read final snapshot");
    let snapshot: serde_json::Value =
        serde_json::from_slice(&snapshot_bytes).expect("decode snapshot json");

    let token_address =
        required_snapshot_value(&snapshot, "evm_mock_erc20_setup.deploy.contract_address")
            .as_str()
            .expect("contract address");
    let token_address_norm = normalize_address_lower(token_address);

    // 2) Run `portfolio.snapshot` feature end-to-end (REST feature execution path).
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        streams: Arc::clone(&streams),
        artifacts: Arc::clone(&artifacts),
    });

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/features/portfolio.snapshot/execute",
            serde_json::json!({
                "payload": canonical_mock_erc20_snapshot_payload(
                    &from_norm,
                    chain_id,
                    &token_address_norm,
                    &control_scope
                )
            }),
        ))
        .await
        .expect("feature execute response");

    assert_eq!(resp.status(), StatusCode::OK);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "success");
    assert_eq!(v["data"]["feature_id"], "portfolio.snapshot");
    assert_eq!(v["data"]["result"]["phase"], "completed");
    assert_eq!(
        v["data"]["result"]["report"]["portfolio_id"],
        "reth-mock-erc20"
    );
    assert_eq!(v["data"]["result"]["report"]["error_count"], 0);
    let report_wallet = &v["data"]["result"]["report"]["wallet_summaries"][0];
    let report_wallet_usd = find_quote_total(&report_wallet["totals_by_quote"], "USD");
    let report_portfolio_usd =
        find_quote_total(&v["data"]["result"]["report"]["totals_by_quote"], "USD");
    assert_eq!(report_wallet_usd, report_portfolio_usd);
    assert_eq!(report_wallet_usd["collateral_value_dec"], "0");
    assert_eq!(report_wallet_usd["debt_value_dec"], "0");
    assert_eq!(report_wallet_usd["staked_value_dec"], "0");
    assert_eq!(
        report_wallet_usd["assets_value_dec"],
        report_wallet_usd["net_value_dec"]
    );

    let snapshot_artifact_id = v["data"]["result"]["snapshot_artifact_id"]
        .as_str()
        .expect("snapshot_artifact_id")
        .to_string();

    let artifact_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/artifacts/{snapshot_artifact_id}"))
                .body(Body::empty())
                .expect("artifact request"),
        )
        .await
        .expect("artifact get response");

    assert_eq!(artifact_resp.status(), StatusCode::OK);
    let a = response_json(artifact_resp).await;

    assert_eq!(a["status"], "success");
    assert_eq!(a["data"]["artifact_id"], snapshot_artifact_id);
    assert_eq!(a["data"]["encoding"], "json");

    let out = a
        .get("data")
        .and_then(|v| v.get("value"))
        .expect("output value");

    assert_eq!(
        out.get("portfolio_id").and_then(|v| v.as_str()),
        Some("reth-mock-erc20")
    );
    assert_eq!(
        out.get("network_pins")
            .and_then(|v| v.as_array())
            .and_then(|pins| pins.first())
            .and_then(|pin| pin.get("chain_id"))
            .and_then(|v| v.as_u64()),
        Some(chain_id)
    );
    let wallet = out
        .get("wallets")
        .and_then(|v| v.as_array())
        .and_then(|wallets| wallets.first())
        .expect("wallet snapshot");
    assert_eq!(
        wallet.get("address").and_then(|v| v.as_str()),
        Some(from_norm.as_str())
    );
    let observations = wallet
        .get("observations")
        .and_then(|v| v.as_array())
        .expect("observations array");
    assert_eq!(observations.len(), 2);
    let token_observation = observations
        .iter()
        .find(|observation| {
            observation.get("symbol_id").and_then(|v| v.as_str())
                == Some("mock.wallet.ethereum-mainnet")
        })
        .expect("mock token observation");
    assert_eq!(
        token_observation
            .get("display_symbol")
            .and_then(|v| v.as_str()),
        Some("MOCK")
    );
    assert_eq!(token_observation["quantity"]["decimals"].as_u64(), Some(6));
    assert_eq!(
        token_observation["quantity"]["raw_dec"].as_str(),
        Some("1000000")
    );
    assert_eq!(
        token_observation["quantity"]["amount_dec"].as_str(),
        Some("1.000000")
    );
    assert_eq!(
        token_observation["values"][0]["quote"].as_str(),
        Some("USD")
    );
    assert_eq!(
        token_observation["values"][0]["unit_price_dec"].as_str(),
        Some("1.00")
    );
    assert_eq!(
        token_observation["source"]["balance_reader_kind"].as_str(),
        Some("erc20_balance")
    );
}
