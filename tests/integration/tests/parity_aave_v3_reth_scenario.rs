#![cfg(feature = "parity-tests")]

use std::collections::HashMap;
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

    let pipeline = Pipeline {
        machine_id: MachineId("aave_v3_reth_pipeline".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("fetch_contracts".to_string()),
                op_id: OpId("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "app": "path:.#aave-v3-contracts-fetch",
                    "stdin_json": {},
                    "timeout_ms": 300000,
                    "write_result_to": "result",
                }),
            },
            PipelineStep {
                step_id: StepId("compile_contracts".to_string()),
                op_id: OpId("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "app": "path:.#aave-v3-contracts-compile",
                    "stdin_json": {},
                    "timeout_ms": 300000,
                    "write_result_to": "result",
                }),
            },
            PipelineStep {
                step_id: StepId("deploy_runtime".to_string()),
                op_id: OpId("aave_v3_deploy_runtime".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "compile_manifest_port": "result",
                    "deployer_account_index": 0,
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                    "deploy_manifest_export_key": "deploy_manifest",
                }),
            },
            PipelineStep {
                step_id: StepId("configure_runtime".to_string()),
                op_id: OpId("aave_v3_configure_runtime".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "deploy_manifest_port": "deploy_manifest",
                    "from_account_index": 0,
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                    "config_report_export_key": "config_report",
                }),
            },
            PipelineStep {
                step_id: StepId("run_scenario".to_string()),
                op_id: OpId("aave_v3_scenario".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "deploy_manifest_port": "deploy_manifest",
                    "config_report_port": "config_report",
                    "funder_account_index": 0,
                    "merican_account_index": 1,
                    "saylor_account_index": 2,
                    "fund_wei": "1000000000000000000",
                    "usdc_supply_amount": 1000000000000u64,
                    "wbtc_collateral_amount": 1000000000u64,
                    "usdc_borrow_amount": 400000000000u64,
                    "borrow_rate_mode": 2,
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                    "scenario_report_export_key": "scenario_report",
                    "scenario_report_artifact_key": "scenario_report_artifact_id",
                }),
            },
        ],
    };

    let mut allowlist = mfm_machine::config::default_nix_flake_allowlist();
    allowlist.push("path:.".to_string());
    let run_config = run_config_with_allowlist(allowlist);

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

    assert_eq!(
        snapshot
            .get("aave_v3_reth_pipeline.deploy_runtime.deploy_manifest.kind")
            .and_then(|v| v.as_str()),
        Some("aave_v3_deploy_manifest_v1")
    );
    assert_eq!(
        snapshot
            .get("aave_v3_reth_pipeline.configure_runtime.config_report.kind")
            .and_then(|v| v.as_str()),
        Some("aave_v3_config_report_v1")
    );

    let report = snapshot
        .get("aave_v3_reth_pipeline.run_scenario.scenario_report")
        .expect("scenario report");
    assert_eq!(
        report.get("kind").and_then(|v| v.as_str()),
        Some("aave_v3_reth_scenario_report_v1")
    );
    assert_eq!(
        report.get("chain_id").and_then(|v| v.as_u64()),
        Some(expected_chain_id)
    );
    assert_eq!(
        report
            .get("amounts")
            .and_then(|v| v.get("usdc_supply"))
            .and_then(|v| v.as_u64()),
        Some(1_000_000_000_000)
    );
    assert_eq!(
        report
            .get("amounts")
            .and_then(|v| v.get("wbtc_collateral"))
            .and_then(|v| v.as_u64()),
        Some(1_000_000_000)
    );
    assert_eq!(
        report
            .get("amounts")
            .and_then(|v| v.get("usdc_borrow"))
            .and_then(|v| v.as_u64()),
        Some(400_000_000_000)
    );
    assert_eq!(
        report
            .get("assertions")
            .and_then(|v| v.get("strict"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        report
            .get("assertions")
            .and_then(|v| v.get("passed"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );

    let report_artifact_id = snapshot
        .get("aave_v3_reth_pipeline.run_scenario.scenario_report_artifact_id")
        .and_then(|v| v.as_str())
        .expect("scenario report artifact id");
    let report_artifact = artifacts
        .get(&mfm_machine::ids::ArtifactId(
            report_artifact_id.to_string(),
        ))
        .await
        .expect("get report artifact");
    let report_artifact_json: serde_json::Value =
        serde_json::from_slice(&report_artifact).expect("decode report artifact");
    assert_eq!(
        report_artifact_json.get("kind").and_then(|v| v.as_str()),
        Some("aave_v3_reth_scenario_report_v1")
    );
}
