#![cfg(feature = "parity-tests")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{
    EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory, EvmJsonRpcSource, EvmSourceKind,
};
use mfm_event_store_postgres::PostgresEventStore;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{RunPhase, Stores};
use mfm_machine::errors::{ContextError, IoError};
use mfm_machine::ids::{ContextKey, OpId, RunId, StateId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransportFactory};
use mfm_machine::stores::{ArtifactStore, EventStore};
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::DefaultRunLauncher;

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

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let s = serde_json::to_string(&body).expect("json request must serialize");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(s))
        .expect("request")
}

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
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
    rpc_url: &str,
    events: Arc<dyn EventStore>,
    artifacts: Arc<dyn ArtifactStore>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
        sources: vec![EvmJsonRpcSource {
            id: "helper_primary".to_string(),
            rpc_url: rpc_url.to_string(),
            authorization: None,
            kind: EvmSourceKind::RemoteUser,
            require_get_proof_probe: false,
        }],
        preferred_order: vec!["helper_primary".to_string()],
        ..EvmJsonRpcHttpConfig::default()
    });
    let env = LiveIoEnv {
        stores: Stores { events, artifacts },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId::must_new("rpc.helper.call".to_string()),
        attempt: 0,
    };
    let mut transport = factory.make(env);
    transport
        .call(IoCall {
            namespace: "evm".to_string(),
            request: serde_json::json!({
                "method": method,
                "params": params,
            }),
            fact_key: None,
        })
        .await
        .unwrap_or_else(|err| match err {
            IoError::MissingFactKey(info)
            | IoError::MissingFact { info, .. }
            | IoError::Transport(info)
            | IoError::RateLimited(info)
            | IoError::Other(info) => panic!("rpc call failed: {}", info.code.0),
        })
}

async fn connect_postgres_with_retry(max_attempts: u32, delay_ms: u64) -> PostgresEventStore {
    let mut last_err: Option<mfm_machine::errors::StorageError> = None;
    for _ in 0..max_attempts {
        match PostgresEventStore::connect_env().await {
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

const RETH_DEV_ACCOUNT0_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[tokio::test]
async fn parity_portfolio_tracker_snapshot_with_mock_erc20_mint() {
    let pg = connect_postgres_with_retry(20, 250).await;
    let events: Arc<dyn EventStore> = Arc::new(pg);

    let s3 = S3ArtifactStore::from_env().expect("s3 config");
    s3.ensure_bucket_exists().await.expect("bucket exists");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(s3);

    let rpc_url = std::env::var("MFM_EVM_RPC_URL").expect("MFM_EVM_RPC_URL is required");

    let accounts = rpc_call(
        &rpc_url,
        Arc::clone(&events),
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
    let signing_key_env = "MFM_EVM_PARITY_DEPLOY_SIGNING_KEY_PORTFOLIO";
    std::env::set_var(signing_key_env, RETH_DEV_ACCOUNT0_PRIVATE_KEY);

    let chain_id_hex = rpc_call(
        &rpc_url,
        Arc::clone(&events),
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
                events: Arc::clone(&events),
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

    assert_eq!(setup_run.phase, RunPhase::Completed);
    let setup_snapshot_id = setup_run.final_snapshot_id.expect("final snapshot");

    let snapshot_bytes = artifacts
        .get(&setup_snapshot_id)
        .await
        .expect("read final snapshot");
    let snapshot: serde_json::Value =
        serde_json::from_slice(&snapshot_bytes).expect("decode snapshot json");

    let token_address = snapshot
        .get("evm_mock_erc20_setup.deploy.contract_address")
        .and_then(|v| v.as_str())
        .expect("contract address");
    let token_address_norm = normalize_address_lower(token_address);

    // 2) Run `portfolio.snapshot` feature end-to-end (REST feature execution path).
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events: Arc::clone(&events),
        artifacts: Arc::clone(&artifacts),
    });

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/features/portfolio.snapshot/execute",
            serde_json::json!({
                "payload": {
                    "address": from_norm,
                    "chain_id": chain_id,
                    "tokens": [
                        {
                            "address": token_address_norm,
                            "symbol": "MOCK",
                            "decimals": null
                        }
                    ],
                }
            }),
        ))
        .await
        .expect("feature execute response");

    assert_eq!(resp.status(), StatusCode::OK);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "success");
    assert_eq!(v["data"]["feature_id"], "portfolio.snapshot");
    assert_eq!(v["data"]["result"]["phase"], "completed");

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
        out.get("wallet_address").and_then(|v| v.as_str()),
        Some(from_norm.as_str())
    );
    assert_eq!(out.get("chain_id").and_then(|v| v.as_u64()), Some(chain_id));
    assert!(out.get("block_number").and_then(|v| v.as_u64()).is_some());
    assert!(out.get("native").is_some());

    let tokens = out
        .get("tokens")
        .and_then(|v| v.as_array())
        .expect("tokens array");
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].get("address").and_then(|v| v.as_str()),
        Some(token_address_norm.as_str())
    );
    assert_eq!(
        tokens[0].get("symbol").and_then(|v| v.as_str()),
        Some("MOCK")
    );
    assert_eq!(tokens[0].get("decimals").and_then(|v| v.as_u64()), Some(6));
    assert_eq!(
        tokens[0].get("raw_u256_dec").and_then(|v| v.as_str()),
        Some("1000000")
    );
    assert_eq!(
        tokens[0].get("amount_dec").and_then(|v| v.as_str()),
        Some("1.000000")
    );
}
