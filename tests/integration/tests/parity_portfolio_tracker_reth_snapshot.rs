#![cfg(feature = "parity-tests")]

use std::sync::Arc;

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory};
use mfm_event_store_mem::MemEventStore;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{RunPhase, Stores};
use mfm_machine::errors::ContextError;
use mfm_machine::ids::{ContextKey, OpId, RunId, StateId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransportFactory};
use mfm_machine::stores::{ArtifactStore, EventStore};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::unstable::{single_op_pipeline, DefaultRunLauncher};

#[derive(Default)]
struct MapContext {
    inner: std::collections::HashMap<String, serde_json::Value>,
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

fn run_config_live() -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff: BackoffPolicy::Fixed {
                delay: std::time::Duration::from_millis(0),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
        nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
    }
}

fn parse_u64_hex(s: &str) -> u64 {
    let Some(rest) = s.strip_prefix("0x") else {
        panic!("missing 0x prefix: {s}");
    };
    if rest.is_empty() || rest.len() > 16 {
        panic!("invalid hex: {s}");
    }
    u64::from_str_radix(rest, 16).expect("hex u64")
}

async fn rpc_call(rpc_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(tmp.path()));

    let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
        rpc_url: Some(rpc_url.to_string()),
        authorization: None,
        ..EvmJsonRpcHttpConfig::default()
    });
    let mut t = factory.make(LiveIoEnv {
        stores: Stores {
            events,
            artifacts,
        },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId("parity.main.rpc".to_string()),
        attempt: 0,
    });

    let v = t
        .call(IoCall {
            namespace: "evm".to_string(),
            request: serde_json::json!({
                "method": method,
                "params": params,
            }),
            fact_key: None,
        })
        .await
        .expect("rpc call");

    v
}

#[tokio::test]
async fn parity_portfolio_tracker_snapshot_against_reth_eth_only() {
    let rpc_url = std::env::var("MFM_EVM_RPC_URL").expect("MFM_EVM_RPC_URL is required");

    let chain_id_hex = rpc_call(&rpc_url, "eth_chainId", serde_json::json!([])).await;
    let chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");

    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(tmp.path()));

    // Intentionally use a fixed address. This keeps the test independent of `eth_accounts`
    // support/configuration in the node.
    let wallet_address = "0x000000000000000000000000000000000000dead";

    let op_config = serde_json::json!({
        "wallet_address": wallet_address,
        "chain_id": chain_id,
        "tokens": [],
    });

    let bundle = mfm_rest_api::make_engine_bundle();
    let pipeline = single_op_pipeline(OpId("portfolio_tracker".to_string()), "v1".to_string(), op_config)
        .expect("pipeline");

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
                run_config: run_config_live(),
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
        .expect("start portfolio snapshot");

    assert_eq!(run.phase, RunPhase::Completed);

    let final_snapshot_id = run.final_snapshot_id.expect("final snapshot id");
    let snapshot_bytes = artifacts
        .get(&final_snapshot_id)
        .await
        .expect("read final snapshot");
    let snapshot: serde_json::Value =
        serde_json::from_slice(&snapshot_bytes).expect("decode snapshot json");

    let out_id = snapshot
        .get("portfolio_tracker.main.snapshot_artifact_id")
        .and_then(|v| v.as_str())
        .expect("snapshot_artifact_id");

    let out_bytes = artifacts
        .get(&mfm_machine::ids::ArtifactId(out_id.to_string()))
        .await
        .expect("read output artifact");
    let out: serde_json::Value = serde_json::from_slice(&out_bytes).expect("output json");

    assert_eq!(
        out.get("wallet_address").and_then(|v| v.as_str()),
        Some(wallet_address)
    );
    assert_eq!(out.get("chain_id").and_then(|v| v.as_u64()), Some(chain_id));
    assert!(out.get("block_number").and_then(|v| v.as_u64()).is_some());
    assert!(out.get("native").is_some());
    assert_eq!(
        out.get("tokens")
            .and_then(|v| v.as_array())
            .map(|a| a.len()),
        Some(0)
    );
}
