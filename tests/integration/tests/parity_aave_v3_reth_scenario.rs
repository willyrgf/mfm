#![cfg(feature = "parity-tests")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory};
use mfm_event_store_postgres::PostgresEventStore;
use mfm_integration_tests::parity_run_ids::write_parity_aave_run_ids;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{RunPhase, Stores};
use mfm_machine::errors::{ContextError, IoError};
use mfm_machine::events::{Event, KernelEvent};
use mfm_machine::ids::{ContextKey, OpId, RunId, StateId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransportFactory};
use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};
use mfm_machine_test_support::init_test_observability;
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::DefaultRunLauncher;
use serde::{Deserialize, Serialize};

const RETH_DEV_ACCOUNT0_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const SCENARIO_REPORT_KIND: &str = "aave_v3_reth_scenario_report_v1";
const DEPLOY_MANIFEST_KIND: &str = "aave_v3_deploy_manifest_v1";

const CONTRACT_USDC: &str = "usdc";
const CONTRACT_WBTC: &str = "wbtc";
const CONTRACT_POOL: &str = "pool";
const CONTRACT_USDC_A_TOKEN: &str = "usdc_a_token";
const CONTRACT_WBTC_A_TOKEN: &str = "wbtc_a_token";
const CONTRACT_USDC_VARIABLE_DEBT_TOKEN: &str = "usdc_variable_debt_token";

const USDC_SUPPLY_AMOUNT: u64 = 1_000_000_000_000;
const WBTC_COLLATERAL_AMOUNT: u64 = 1_000_000_000;
const USDC_BORROW_AMOUNT: u64 = 1_000_000;
const BORROW_RATE_MODE: u64 = 2;

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

#[derive(Clone, Debug, Deserialize)]
struct AaveDeployManifest {
    kind: String,
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
    funder: String,
    supplier: String,
    borrower: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioAccounts {
    funder: String,
    supplier: String,
    borrower: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioAmounts {
    usdc_supply: u64,
    wbtc_collateral: u64,
    usdc_borrow: u64,
    borrow_rate_mode: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioPositionSnapshot {
    supplier_supplied_usdc: u64,
    borrower_collateral_wbtc: u64,
    borrower_borrowed_usdc: u64,
    borrower_usdc_balance: u64,
    pool_usdc_balance: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioAssertions {
    strict: bool,
    passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioReport {
    kind: String,
    chain_id: u64,
    accounts: AaveScenarioAccounts,
    amounts: AaveScenarioAmounts,
    positions: AaveScenarioPositionSnapshot,
    assertions: AaveScenarioAssertions,
    deploy_manifest_kind: String,
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

fn snapshot_kind(snapshot: &serde_json::Value, key: &str) -> Option<String> {
    snapshot
        .get(format!("{key}.kind"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            snapshot
                .get(key)
                .and_then(|v| v.get("kind"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
        })
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

fn balance_of_calldata(owner: &str) -> String {
    let normalized = owner
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("address must start with 0x: {owner}"));
    assert_eq!(normalized.len(), 40, "address must be 20 bytes: {owner}");
    format!("0x70a08231{:0>64}", normalized.to_ascii_lowercase())
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

async fn erc20_balance_u64(
    rpc_url: &str,
    events: Arc<dyn EventStore>,
    artifacts: Arc<dyn ArtifactStore>,
    token: &str,
    owner: &str,
) -> u64 {
    let value = rpc_call(
        rpc_url,
        events,
        artifacts,
        "eth_call",
        serde_json::json!([
            {
                "to": token,
                "data": balance_of_calldata(owner),
            },
            "latest"
        ]),
    )
    .await;

    let raw = value.as_str().expect("eth_call must return hex string");
    parse_u64_hex(raw)
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

async fn run_failure_diagnostics(events: Arc<dyn EventStore>, run_id: RunId) -> String {
    let stream = match events.read_range(run_id.clone(), 1, None).await {
        Ok(stream) => stream,
        Err(err) => {
            return format!("run_id={} read_range_failed={err:?}", run_id.0);
        }
    };

    let mut last_state_entered: Option<(u64, String, u32)> = None;
    let mut last_state_failed: Option<(u64, String, String, bool, String)> = None;
    let mut last_fact_key: Option<(u64, String)> = None;

    for envelope in stream {
        match envelope.event {
            Event::Kernel(KernelEvent::StateEntered {
                state_id, attempt, ..
            }) => {
                last_state_entered = Some((envelope.seq, state_id.0, attempt));
            }
            Event::Kernel(KernelEvent::StateFailed {
                state_id, error, ..
            }) => {
                last_state_failed = Some((
                    envelope.seq,
                    state_id.0,
                    error.info.code.0,
                    error.info.retryable,
                    error.info.message,
                ));
            }
            Event::Domain(domain) if domain.name == "fact_recorded" => {
                if let Some(key) = domain.payload.get("key").and_then(|v| v.as_str()) {
                    last_fact_key = Some((envelope.seq, key.to_string()));
                }
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
    if let Some((seq, state_id, code, retryable, message)) = last_state_failed {
        parts.push(format!(
            "state_failed={state_id} seq={seq} code={code} retryable={retryable} message={message}"
        ));
    }
    if let Some((seq, fact_key)) = last_fact_key {
        parts.push(format!("last_fact_key={fact_key} seq={seq}"));
    }
    if parts.len() == 1 {
        parts.push("no_state_failed_event_found".to_string());
    }

    parts.join("; ")
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

#[tokio::test]
async fn parity_aave_v3_reth_scenario_pipeline() {
    init_test_observability();

    let pg = connect_postgres_with_retry(20, 250).await;
    let events: Arc<dyn EventStore> = Arc::new(pg);

    let s3 = S3ArtifactStore::from_env().expect("s3 config");
    s3.ensure_bucket_exists().await.expect("bucket exists");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(s3);

    let rpc_url = std::env::var("MFM_EVM_RPC_URL").expect("MFM_EVM_RPC_URL is required");
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
    let accounts_json = rpc_call(
        &rpc_url,
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
        funder: accounts
            .first()
            .and_then(|v| v.as_str())
            .expect("funder account")
            .to_string(),
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
        .expect("start phase A pipeline");

    if phase_a_run.phase != RunPhase::Completed {
        let diagnostics =
            run_failure_diagnostics(Arc::clone(&events), phase_a_run.run_id.clone()).await;
        panic!(
            "phase A expected Completed, got {:?}; {}",
            phase_a_run.phase, diagnostics
        );
    }
    let phase_a_snapshot_id = phase_a_run
        .final_snapshot_id
        .expect("phase A final snapshot");
    let phase_a_snapshot_bytes = artifacts
        .get(&phase_a_snapshot_id)
        .await
        .expect("read phase A final snapshot");
    let phase_a_snapshot: serde_json::Value =
        serde_json::from_slice(&phase_a_snapshot_bytes).expect("decode phase A snapshot json");

    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.adapt_origin_deploy.deploy_manifest",
        )
        .as_deref(),
        Some(DEPLOY_MANIFEST_KIND)
    );
    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.fetch_origin.result"
        )
        .as_deref(),
        Some("aave_v3_origin_source_v1")
    );
    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.compile_origin.result"
        )
        .as_deref(),
        Some("aave_v3_origin_compile_manifest_v1")
    );
    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.deploy_origin_stack.result"
        )
        .as_deref(),
        Some("aave_v3_origin_deploy_output_v1")
    );

    let deploy_manifest = snapshot_value(
        &phase_a_snapshot,
        "aave_v3_reth_pipeline.adapt_origin_deploy.deploy_manifest",
    )
    .cloned()
    .expect("deploy manifest in phase A snapshot");
    let deploy_manifest: AaveDeployManifest =
        serde_json::from_value(deploy_manifest).expect("decode deploy manifest");
    assert_eq!(deploy_manifest.kind, DEPLOY_MANIFEST_KIND);

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
        .expect("start phase B pipeline");

    if phase_b_run.phase != RunPhase::Completed {
        let diagnostics =
            run_failure_diagnostics(Arc::clone(&events), phase_b_run.run_id.clone()).await;
        panic!(
            "phase B expected Completed, got {:?}; {}",
            phase_b_run.phase, diagnostics
        );
    }

    let pool = contract_from_manifest(&deploy_manifest, CONTRACT_POOL);
    let usdc = contract_from_manifest(&deploy_manifest, CONTRACT_USDC);
    let usdc_a_token = contract_from_manifest(&deploy_manifest, CONTRACT_USDC_A_TOKEN);
    let wbtc_a_token = contract_from_manifest(&deploy_manifest, CONTRACT_WBTC_A_TOKEN);
    let usdc_variable_debt =
        contract_from_manifest(&deploy_manifest, CONTRACT_USDC_VARIABLE_DEBT_TOKEN);

    let positions = AaveScenarioPositionSnapshot {
        supplier_supplied_usdc: erc20_balance_u64(
            &rpc_url,
            Arc::clone(&events),
            Arc::clone(&artifacts),
            &usdc_a_token.address,
            &actors.supplier,
        )
        .await,
        borrower_collateral_wbtc: erc20_balance_u64(
            &rpc_url,
            Arc::clone(&events),
            Arc::clone(&artifacts),
            &wbtc_a_token.address,
            &actors.borrower,
        )
        .await,
        borrower_borrowed_usdc: erc20_balance_u64(
            &rpc_url,
            Arc::clone(&events),
            Arc::clone(&artifacts),
            &usdc_variable_debt.address,
            &actors.borrower,
        )
        .await,
        borrower_usdc_balance: erc20_balance_u64(
            &rpc_url,
            Arc::clone(&events),
            Arc::clone(&artifacts),
            &usdc.address,
            &actors.borrower,
        )
        .await,
        pool_usdc_balance: erc20_balance_u64(
            &rpc_url,
            Arc::clone(&events),
            Arc::clone(&artifacts),
            &usdc.address,
            &pool.address,
        )
        .await,
    };

    assert!(positions.supplier_supplied_usdc >= USDC_SUPPLY_AMOUNT);
    assert!(positions.borrower_collateral_wbtc >= WBTC_COLLATERAL_AMOUNT);
    assert!(positions.borrower_borrowed_usdc >= USDC_BORROW_AMOUNT);

    let report = AaveScenarioReport {
        kind: SCENARIO_REPORT_KIND.to_string(),
        chain_id: expected_chain_id,
        accounts: AaveScenarioAccounts {
            funder: actors.funder,
            supplier: actors.supplier,
            borrower: actors.borrower,
        },
        amounts: AaveScenarioAmounts {
            usdc_supply: USDC_SUPPLY_AMOUNT,
            wbtc_collateral: WBTC_COLLATERAL_AMOUNT,
            usdc_borrow: USDC_BORROW_AMOUNT,
            borrow_rate_mode: BORROW_RATE_MODE,
        },
        positions,
        assertions: AaveScenarioAssertions {
            strict: true,
            passed: true,
        },
        deploy_manifest_kind: deploy_manifest.kind,
    };

    assert_eq!(report.kind, SCENARIO_REPORT_KIND);
    assert_eq!(report.chain_id, expected_chain_id);
    assert_eq!(report.amounts.usdc_supply, USDC_SUPPLY_AMOUNT);
    assert_eq!(report.amounts.wbtc_collateral, WBTC_COLLATERAL_AMOUNT);
    assert_eq!(report.amounts.usdc_borrow, USDC_BORROW_AMOUNT);
    assert!(report.assertions.strict);
    assert!(report.assertions.passed);

    let report_artifact_id = artifacts
        .put(
            ArtifactKind::Output,
            serde_json::to_vec(&report).expect("serialize scenario report"),
        )
        .await
        .expect("write scenario report artifact");
    let report_artifact = artifacts
        .get(&report_artifact_id)
        .await
        .expect("get report artifact");
    let report_artifact_json: serde_json::Value =
        serde_json::from_slice(&report_artifact).expect("decode report artifact");
    assert_eq!(
        report_artifact_json.get("kind").and_then(|v| v.as_str()),
        Some(SCENARIO_REPORT_KIND)
    );

    write_parity_aave_run_ids(&phase_a_run.run_id, &phase_b_run.run_id);
}
