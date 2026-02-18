#![cfg(feature = "parity-tests")]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory};
use mfm_event_store_postgres::PostgresEventStore;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{RunPhase, Stores};
use mfm_machine::errors::{ContextError, IoError, StorageError};
use mfm_machine::events::{Event, KernelEvent, RunStatus};
use mfm_machine::ids::{ContextKey, OpId, RunId, StateId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransportFactory};
use mfm_machine::stores::{ArtifactStore, EventStore};
use mfm_machine_test_support::init_test_observability;
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::DefaultRunLauncher;
use serde::Deserialize;
use tracing::info;

const RETH_DEV_ACCOUNT0_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const EVM_REQUIRED_STATES: &[&str] = &[
    "evm_reth_pipeline.fetch.run",
    "evm_reth_pipeline.adapt.adapt",
    "evm_reth_pipeline.deploy.deploy",
    "evm_reth_pipeline.configure.configure",
    "evm_reth_pipeline.validate.validate",
];

const AAVE_PHASE_A_REQUIRED_STATES: &[&str] = &[
    "aave_v3_reth_pipeline.fetch_origin.run",
    "aave_v3_reth_pipeline.compile_origin.run",
    "aave_v3_reth_pipeline.deploy_origin_stack.run",
    "aave_v3_reth_pipeline.adapt_origin_deploy.adapt_origin_deploy",
];

const AAVE_PHASE_B_REQUIRED_STATES: &[&str] = &[
    "aave_v3_reth_scenario_generic_pipeline.approve_usdc.configure",
    "aave_v3_reth_scenario_generic_pipeline.approve_wbtc.configure",
    "aave_v3_reth_scenario_generic_pipeline.supply_usdc.configure",
    "aave_v3_reth_scenario_generic_pipeline.supply_wbtc.configure",
    "aave_v3_reth_scenario_generic_pipeline.borrow_usdc.configure",
    "aave_v3_reth_scenario_generic_pipeline.validate_scenario.validate",
];

const CONTRACT_USDC: &str = "usdc";
const CONTRACT_WBTC: &str = "wbtc";
const CONTRACT_POOL: &str = "pool";

const USDC_SUPPLY_AMOUNT: u64 = 1_000_000_000_000;
const WBTC_COLLATERAL_AMOUNT: u64 = 1_000_000_000;
const USDC_BORROW_AMOUNT: u64 = 1_000_000;
const BORROW_RATE_MODE: u64 = 2;

#[derive(Clone, Debug, Deserialize)]
struct AaveDeployManifest {
    contracts: Vec<AaveDeployManifestContract>,
}

#[derive(Clone, Debug, Deserialize)]
struct AaveDeployManifestContract {
    id: String,
    address: String,
    artifact: serde_json::Value,
}

#[derive(Clone, Debug)]
struct ScenarioActors {
    supplier: String,
    borrower: String,
}

#[derive(Clone, Debug)]
struct AavePipelineRunIds {
    phase_a_run_id: RunId,
    phase_b_run_id: RunId,
}

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

fn workspace_flake_ref() -> String {
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for dir in start.ancestors() {
        if dir.join("flake.nix").is_file() {
            let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
            return format!("path:{}", canonical.display());
        }
    }

    panic!(
        "could not locate workspace root from CARGO_MANIFEST_DIR={}",
        start.display()
    );
}

fn snapshot_value<'a>(snapshot: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    if let Some(v) = snapshot.get(key) {
        return Some(v);
    }

    let mut current = snapshot;
    for segment in key.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn contract_from_manifest<'a>(
    manifest: &'a AaveDeployManifest,
    id: &str,
) -> &'a AaveDeployManifestContract {
    manifest
        .contracts
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("missing contract in deploy manifest: {id}"))
}

async fn connect_postgres_with_retry(max_attempts: u32, delay_ms: u64) -> PostgresEventStore {
    let mut last_err: Option<StorageError> = None;
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

async fn rpc_call(
    rpc_url: &str,
    events: Arc<dyn EventStore>,
    artifacts: Arc<dyn ArtifactStore>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
        rpc_url: Some(rpc_url.to_string()),
        authorization: None,
        ..EvmJsonRpcHttpConfig::default()
    });
    let env = LiveIoEnv {
        stores: Stores { events, artifacts },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId("rpc.helper.call".to_string()),
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

async fn run_evm_reth_pipeline(
    events: Arc<dyn EventStore>,
    artifacts: Arc<dyn ArtifactStore>,
    rpc_url: &str,
) -> RunId {
    let accounts = rpc_call(
        rpc_url,
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

    let chain_id_hex = rpc_call(
        rpc_url,
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

    std::env::set_var(
        "MFM_EVM_PARITY_DEPLOY_SIGNING_KEY",
        RETH_DEV_ACCOUNT0_PRIVATE_KEY,
    );

    let pipeline = Pipeline {
        machine_id: MachineId("evm_reth_pipeline".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("fetch".to_string()),
                op_id: OpId("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "program_path": contract_artifact_program_path(),
                    "stdin_json": {},
                    "timeout_ms": 300000,
                    "write_result_to": "result",
                }),
            },
            PipelineStep {
                step_id: StepId("adapt".to_string()),
                op_id: OpId("evm_contract_from_nix".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "result_pointer": "/artifact"
                }),
            },
            PipelineStep {
                step_id: StepId("deploy".to_string()),
                op_id: OpId("evm_deploy".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact_port": "contract_artifact",
                    "from": from,
                    "signing_key_env": "MFM_EVM_PARITY_DEPLOY_SIGNING_KEY",
                    "constructor_args": [1],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("configure".to_string()),
                op_id: OpId("evm_configure".to_string()),
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
                op_id: OpId("evm_validate".to_string()),
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
            Stores { events, artifacts },
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
        .expect("start evm_reth_pipeline");

    assert_eq!(run.phase, RunPhase::Completed);
    run.run_id
}

fn phase_a_pipeline(workspace_flake: &str) -> Pipeline {
    let fetch_origin_app = format!("{workspace_flake}#aave-v3-origin-fetch");
    let compile_origin_app = format!("{workspace_flake}#aave-v3-origin-compile");
    let deploy_origin_app = format!("{workspace_flake}#aave-v3-origin-deploy");

    Pipeline {
        machine_id: MachineId("aave_v3_reth_pipeline".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("fetch_origin".to_string()),
                op_id: OpId("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "app": fetch_origin_app,
                    "stdin_json": {},
                    "timeout_ms": 300000,
                    "write_result_to": "result",
                }),
            },
            PipelineStep {
                step_id: StepId("compile_origin".to_string()),
                op_id: OpId("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "app": compile_origin_app,
                    "stdin_json": {},
                    "timeout_ms": 300000,
                    "write_result_to": "result",
                }),
            },
            PipelineStep {
                step_id: StepId("deploy_origin_stack".to_string()),
                op_id: OpId("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "app": deploy_origin_app,
                    "stdin_json": {},
                    "timeout_ms": 600000,
                    "write_result_to": "result",
                }),
            },
            PipelineStep {
                step_id: StepId("adapt_origin_deploy".to_string()),
                op_id: OpId("aave_v3_origin_adapt_deploy".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "origin_deploy_port": "result",
                    "deploy_manifest_export_key": "deploy_manifest",
                }),
            },
        ],
    }
}

fn phase_b_pipeline(
    deploy_manifest: &AaveDeployManifest,
    actors: &ScenarioActors,
    expected_chain_id: u64,
) -> Pipeline {
    let pool = contract_from_manifest(deploy_manifest, CONTRACT_POOL);
    let usdc = contract_from_manifest(deploy_manifest, CONTRACT_USDC);
    let wbtc = contract_from_manifest(deploy_manifest, CONTRACT_WBTC);
    let supplier = actors.supplier.clone();
    let borrower = actors.borrower.clone();
    let pool_artifact = pool.artifact.clone();
    let pool_address = pool.address.clone();
    let usdc_artifact = usdc.artifact.clone();
    let usdc_address = usdc.address.clone();
    let wbtc_artifact = wbtc.artifact.clone();
    let wbtc_address = wbtc.address.clone();

    Pipeline {
        machine_id: MachineId("aave_v3_reth_scenario_generic_pipeline".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("approve_usdc".to_string()),
                op_id: OpId("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": usdc_artifact,
                    "from": supplier.clone(),
                    "contract_address": usdc_address.clone(),
                    "calls": [{
                        "function": "approve",
                        "args": [pool_address.clone(), USDC_SUPPLY_AMOUNT],
                    }],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("approve_wbtc".to_string()),
                op_id: OpId("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": wbtc_artifact,
                    "from": borrower.clone(),
                    "contract_address": wbtc_address.clone(),
                    "calls": [{
                        "function": "approve",
                        "args": [pool_address.clone(), WBTC_COLLATERAL_AMOUNT],
                    }],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("supply_usdc".to_string()),
                op_id: OpId("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact.clone(),
                    "from": supplier.clone(),
                    "contract_address": pool_address.clone(),
                    "calls": [{
                        "function": "supply",
                        "args": [
                            usdc_address.clone(),
                            USDC_SUPPLY_AMOUNT,
                            supplier.clone(),
                            0,
                        ],
                    }],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("supply_wbtc".to_string()),
                op_id: OpId("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact.clone(),
                    "from": borrower.clone(),
                    "contract_address": pool_address.clone(),
                    "calls": [{
                        "function": "supply",
                        "args": [
                            wbtc_address.clone(),
                            WBTC_COLLATERAL_AMOUNT,
                            borrower.clone(),
                            0,
                        ],
                    }],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("borrow_usdc".to_string()),
                op_id: OpId("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact.clone(),
                    "from": borrower.clone(),
                    "contract_address": pool_address.clone(),
                    "calls": [{
                        "function": "borrow",
                        "args": [
                            usdc_address.clone(),
                            USDC_BORROW_AMOUNT,
                            BORROW_RATE_MODE,
                            0,
                            borrower.clone(),
                        ],
                    }],
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("validate_scenario".to_string()),
                op_id: OpId("evm_validate".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact,
                    "contract_address": pool_address,
                    "expected_chain_id": expected_chain_id,
                    "require_client_substring": "reth",
                    "read_assertions": [],
                    "event_assertions": [],
                }),
            },
        ],
    }
}

async fn run_aave_v3_pipelines(
    events: Arc<dyn EventStore>,
    artifacts: Arc<dyn ArtifactStore>,
    rpc_url: &str,
) -> AavePipelineRunIds {
    let chain_id_hex = rpc_call(
        rpc_url,
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
    let accounts_json = rpc_call(
        rpc_url,
        Arc::clone(&events),
        Arc::clone(&artifacts),
        "eth_accounts",
        serde_json::json!([]),
    )
    .await;
    let accounts = accounts_json
        .as_array()
        .expect("eth_accounts returned array");
    let actors = ScenarioActors {
        supplier: accounts
            .get(1)
            .and_then(|v| v.as_str())
            .expect("supplier account")
            .to_string(),
        borrower: accounts
            .get(2)
            .and_then(|v| v.as_str())
            .expect("borrower account")
            .to_string(),
    };

    std::env::set_var(
        "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY",
        RETH_DEV_ACCOUNT0_PRIVATE_KEY,
    );
    std::env::set_var("MFM_AAVE_V3_ORIGIN_SUPPLIER", &actors.supplier);
    std::env::set_var("MFM_AAVE_V3_ORIGIN_BORROWER", &actors.borrower);
    std::env::set_var(
        "MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT",
        USDC_SUPPLY_AMOUNT.to_string(),
    );
    std::env::set_var(
        "MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT",
        WBTC_COLLATERAL_AMOUNT.to_string(),
    );

    let workspace_flake = workspace_flake_ref();
    let mut allowlist = mfm_machine::config::default_nix_flake_allowlist();
    allowlist.push(workspace_flake.clone());
    let run_config_phase_a = run_config_with_allowlist(allowlist);

    let bundle = mfm_rest_api::make_engine_bundle();
    let launcher = DefaultRunLauncher;

    let phase_a_run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                events: Arc::clone(&events),
                artifacts: Arc::clone(&artifacts),
            },
            Arc::clone(&bundle.registry),
            Arc::clone(&bundle.planner),
            LaunchPipeline {
                pipeline: phase_a_pipeline(&workspace_flake),
                input: serde_json::json!({}),
                run_config: run_config_phase_a,
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
        .expect("start aave_v3 phase A pipeline");
    assert_eq!(phase_a_run.phase, RunPhase::Completed);

    let phase_a_snapshot_id = phase_a_run
        .final_snapshot_id
        .as_ref()
        .expect("phase A final snapshot");
    let phase_a_snapshot_bytes = artifacts
        .get(phase_a_snapshot_id)
        .await
        .expect("read phase A final snapshot");
    let phase_a_snapshot: serde_json::Value =
        serde_json::from_slice(&phase_a_snapshot_bytes).expect("decode phase A snapshot");
    let deploy_manifest_value = snapshot_value(
        &phase_a_snapshot,
        "aave_v3_reth_pipeline.adapt_origin_deploy.deploy_manifest",
    )
    .cloned()
    .expect("deploy manifest in phase A snapshot");
    let deploy_manifest: AaveDeployManifest =
        serde_json::from_value(deploy_manifest_value).expect("decode deploy manifest");

    let phase_b_run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                events: Arc::clone(&events),
                artifacts: Arc::clone(&artifacts),
            },
            Arc::clone(&bundle.registry),
            Arc::clone(&bundle.planner),
            LaunchPipeline {
                pipeline: phase_b_pipeline(&deploy_manifest, &actors, expected_chain_id),
                input: serde_json::json!({}),
                run_config: run_config_with_allowlist(
                    mfm_machine::config::default_nix_flake_allowlist(),
                ),
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
        .expect("start aave_v3 phase B pipeline");
    assert_eq!(phase_b_run.phase, RunPhase::Completed);

    AavePipelineRunIds {
        phase_a_run_id: phase_a_run.run_id,
        phase_b_run_id: phase_b_run.run_id,
    }
}

fn assert_required_state_order(
    machine_id: &str,
    entered_state_ids: &[String],
    required_states: &[&str],
) {
    let mut cursor = 0usize;
    for required in required_states {
        let relative = entered_state_ids[cursor..]
            .iter()
            .position(|state| state == required)
            .unwrap_or_else(|| {
                panic!(
                    "missing required state `{required}` for `{machine_id}`; entered states={:?}",
                    entered_state_ids
                )
            });
        cursor += relative + 1;
    }
}

async fn audit_run_events(
    events: Arc<dyn EventStore>,
    run_id: RunId,
    machine_id: &str,
    required_states: &[&str],
    min_state_count: usize,
) {
    let head_seq = events.head_seq(run_id).await.expect("head seq");
    assert!(
        head_seq > 0,
        "run `{machine_id}` must emit at least one event"
    );

    let stream = events
        .read_range(run_id, 1, None)
        .await
        .expect("event stream");
    assert_eq!(
        stream.len() as u64,
        head_seq,
        "stream length must equal head seq for `{machine_id}`"
    );
    assert_eq!(stream.first().map(|e| e.seq), Some(1));
    assert_eq!(stream.last().map(|e| e.seq), Some(head_seq));
    for (idx, envelope) in stream.iter().enumerate() {
        assert_eq!(
            envelope.seq,
            (idx + 1) as u64,
            "event seq continuity broken for `{machine_id}` at index {idx}"
        );
    }

    let mut entered_state_ids: Vec<String> = Vec::new();
    let mut active_states: HashSet<String> = HashSet::new();
    let mut entered_count = 0usize;
    let mut terminal_count = 0usize;
    let mut run_started_idx: Option<usize> = None;
    let mut run_completed_idx: Option<usize> = None;
    let mut run_completed_status: Option<RunStatus> = None;

    for (idx, envelope) in stream.iter().enumerate() {
        let Event::Kernel(kernel) = &envelope.event else {
            continue;
        };

        match kernel {
            KernelEvent::RunStarted { .. } => {
                assert!(
                    run_started_idx.is_none(),
                    "run `{machine_id}` emitted multiple RunStarted events"
                );
                run_started_idx = Some(idx);
            }
            KernelEvent::StateEntered { state_id, .. } => {
                let state = state_id.0.clone();
                assert!(
                    state.starts_with(machine_id),
                    "run `{machine_id}` saw unexpected state id `{state}`"
                );
                assert!(
                    active_states.insert(state.clone()),
                    "state `{state}` re-entered before terminal event in `{machine_id}`"
                );
                entered_state_ids.push(state);
                entered_count += 1;
            }
            KernelEvent::StateCompleted { state_id, .. }
            | KernelEvent::StateFailed { state_id, .. } => {
                let state = state_id.0.clone();
                assert!(
                    active_states.remove(&state),
                    "state `{state}` terminal event without matching entry in `{machine_id}`"
                );
                terminal_count += 1;
            }
            KernelEvent::RunCompleted { status, .. } => {
                assert!(
                    run_completed_idx.is_none(),
                    "run `{machine_id}` emitted multiple RunCompleted events"
                );
                run_completed_idx = Some(idx);
                run_completed_status = Some(status.clone());
            }
        }
    }

    let started = run_started_idx.expect("missing RunStarted");
    let completed = run_completed_idx.expect("missing RunCompleted");
    assert!(
        started < completed,
        "RunStarted must happen before RunCompleted for `{machine_id}`"
    );
    assert_eq!(
        run_completed_status,
        Some(RunStatus::Completed),
        "run `{machine_id}` must complete successfully"
    );
    assert_eq!(
        entered_count, terminal_count,
        "run `{machine_id}` has unmatched state entry/terminal counts"
    );
    assert!(
        active_states.is_empty(),
        "run `{machine_id}` left active states without terminal events: {:?}",
        active_states
    );
    assert!(
        entered_count >= min_state_count,
        "run `{machine_id}` must emit at least {min_state_count} StateEntered events"
    );

    assert_required_state_order(machine_id, &entered_state_ids, required_states);

    let report = serde_json::json!({
        "kind": "parity_postgres_state_events_audit_report_v1",
        "machine_id": machine_id,
        "run_id": run_id.0.to_string(),
        "head_seq": head_seq,
        "state_entered_count": entered_count,
        "state_terminal_count": terminal_count,
        "required_states": required_states,
    });
    info!(
        report = %serde_json::to_string(&report).expect("serialize report"),
        "postgres state events audit passed"
    );
}

#[tokio::test]
async fn parity_postgres_state_events_audit_for_multi_state_pipelines() {
    init_test_observability();

    let pg = connect_postgres_with_retry(20, 250).await;
    let events: Arc<dyn EventStore> = Arc::new(pg);

    let s3 = S3ArtifactStore::from_env().expect("s3 config");
    s3.ensure_bucket_exists().await.expect("bucket exists");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(s3);

    let rpc_url = std::env::var("MFM_EVM_RPC_URL").expect("MFM_EVM_RPC_URL is required");

    let evm_run_id =
        run_evm_reth_pipeline(Arc::clone(&events), Arc::clone(&artifacts), &rpc_url).await;
    audit_run_events(
        Arc::clone(&events),
        evm_run_id,
        "evm_reth_pipeline",
        EVM_REQUIRED_STATES,
        5,
    )
    .await;

    let aave_run_ids =
        run_aave_v3_pipelines(Arc::clone(&events), Arc::clone(&artifacts), &rpc_url).await;
    audit_run_events(
        Arc::clone(&events),
        aave_run_ids.phase_a_run_id,
        "aave_v3_reth_pipeline",
        AAVE_PHASE_A_REQUIRED_STATES,
        4,
    )
    .await;
    audit_run_events(
        events,
        aave_run_ids.phase_b_run_id,
        "aave_v3_reth_scenario_generic_pipeline",
        AAVE_PHASE_B_REQUIRED_STATES,
        6,
    )
    .await;
}
