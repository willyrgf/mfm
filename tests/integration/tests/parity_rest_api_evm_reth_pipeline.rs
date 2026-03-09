#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{
    EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory, EvmJsonRpcSource, EvmSourceKind,
};
use mfm_event_store_postgres::PostgresEventStore;
use mfm_integration_tests::parity_run_ids::write_parity_evm_run_id;
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
use mfm_machine_test_support::init_test_observability;
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

fn contract_artifact_program_path() -> String {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v mfm-contract-artifact-configurable-counter")
        .output()
        .expect("resolve mfm-contract-artifact-configurable-counter path");
    assert!(
        out.status.success(),
        "mfm-contract-artifact-configurable-counter must be in PATH"
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
async fn parity_reth_pipeline_contract_from_nix() {
    init_test_observability();

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
    let signing_key_env = "MFM_EVM_PARITY_DEPLOY_SIGNING_KEY";
    std::env::set_var(signing_key_env, RETH_DEV_ACCOUNT0_PRIVATE_KEY);

    let chain_id_hex = rpc_call(
        &rpc_url,
        Arc::clone(&events),
        Arc::clone(&artifacts),
        "eth_chainId",
        serde_json::json!([]),
    )
    .await;
    let expected_chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");

    let contract_program_path = contract_artifact_program_path();

    let pipeline = Pipeline {
        machine_id: MachineId("evm_reth_pipeline".to_string()),
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
                    "constructor_args": [1],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("configure".to_string()),
                op_id: OpId::must_new("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact_port": "contract_artifact",
                    "from": from,
                    "calls": [
                        {"function": "setValue", "args": [7]}
                    ],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("validate".to_string()),
                op_id: OpId::must_new("evm_validate".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact_port": "contract_artifact",
                    "expected_chain_id": expected_chain_id,
                    "require_client_substring": "reth",
                    "read_assertions": [
                        {"function": "getValue", "args": [], "expected": 7}
                    ],
                    "event_assertions": [
                        {"event": "ValueSet", "min_count": 2}
                    ],
                }),
            },
        ],
    };

    let run_config = run_config_with_allowlist(mfm_machine::config::default_nix_flake_allowlist());

    let bundle = mfm_rest_api::make_engine_bundle();
    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                events: Arc::clone(&events),
                artifacts: Arc::clone(&artifacts),
            },
            Arc::clone(&bundle.registry),
            Arc::clone(&bundle.planner),
            LaunchPipeline {
                pipeline,
                input: serde_json::json!({}),
                run_config,
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
        .expect("start pipeline");

    assert_eq!(run.phase, RunPhase::Completed);
    let final_snapshot_id = run.final_snapshot_id.expect("final snapshot");

    let snapshot_bytes = artifacts
        .get(&final_snapshot_id)
        .await
        .expect("read final snapshot");
    let snapshot: serde_json::Value =
        serde_json::from_slice(&snapshot_bytes).expect("decode snapshot json");

    let contract_address = snapshot
        .get("evm_reth_pipeline.deploy.contract_address")
        .and_then(|v| v.as_str())
        .expect("contract address");
    assert!(contract_address.starts_with("0x"));

    let deploy_tx_hash = snapshot
        .get("evm_reth_pipeline.deploy.deploy_tx_hash")
        .and_then(|v| v.as_str())
        .expect("deploy tx hash");
    assert!(deploy_tx_hash.starts_with("0x"));

    let configure_tx_hashes = snapshot
        .get("evm_reth_pipeline.configure.configure_tx_hashes")
        .and_then(|v| v.as_array())
        .expect("configure tx hashes");
    assert!(!configure_tx_hashes.is_empty());

    assert_eq!(
        snapshot.get("evm_reth_pipeline.validate.validated"),
        Some(&serde_json::json!(true))
    );

    let chain_id = snapshot
        .get("evm_reth_pipeline.validate.chain_id")
        .and_then(|v| v.as_u64())
        .expect("validate chain id");
    assert_eq!(chain_id, expected_chain_id);

    let client_version = snapshot
        .get("evm_reth_pipeline.validate.client_version")
        .and_then(|v| v.as_str())
        .expect("validate client version");
    assert!(client_version.to_ascii_lowercase().contains("reth"));

    let head = events.head_seq(run.run_id).await.expect("head seq");
    assert!(head > 0);
    let stream = events
        .read_range(run.run_id, 1, None)
        .await
        .expect("event stream");
    assert!(!stream.is_empty());

    write_parity_evm_run_id(&run.run_id);
}
