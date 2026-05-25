#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use mfm_app::{DriveMode, TypedRunResumeRequest};
use mfm_artifact_store_fs::{FsTypedArtifactStore, TypedArtifactDescriptor};
use mfm_events::v1 as typed_events;
use mfm_integration_tests::artifact_stores;
use mfm_integration_tests::parity_run_ids::write_parity_evm_run_id;
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
use mfm_machine_test_support::init_test_observability;
use mfm_op_evm_deploy_configure_validate::{
    certified_dcv_spec, dcv_config_artifacts_for_spec, dcv_program_draft,
    decode_deploy_configure_validate_canonical_config, DcvConfigArtifact,
};
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::DefaultRunLauncher;
use mfm_store::v1::TypedRunEventStore;
use mfm_stream_store_postgres::PostgresStreamStore;
use mfm_transports_rpc_control::RpcControlBootstrapSource;

const NETWORK_ID: &str = "ethereum-mainnet";
const CONTROL_SCOPE: &str = "parity.evm_reth_pipeline";
const PARITY_RUN_MAX_ATTEMPTS: u32 = 3;
const PARITY_RUN_RETRY_DELAY_MS: u64 = 250;

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

fn run_config_with_allowlist(allowlist: Vec<String>) -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: PARITY_RUN_MAX_ATTEMPTS,
            backoff: BackoffPolicy::Fixed {
                delay: Duration::from_millis(PARITY_RUN_RETRY_DELAY_MS),
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

fn contract_artifact_json() -> serde_json::Value {
    let out = std::process::Command::new(contract_artifact_program_path())
        .output()
        .expect("run mfm-contract-artifact-configurable-counter");
    assert!(
        out.status.success(),
        "mfm-contract-artifact-configurable-counter must emit an artifact"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("decode contract artifact JSON");
    value
        .get("artifact")
        .cloned()
        .expect("contract artifact output must include /artifact")
}

async fn persist_dcv_config_artifacts(
    artifacts: &FsTypedArtifactStore,
    configs: Vec<DcvConfigArtifact>,
) {
    for config in configs {
        artifacts
            .put_artifact(
                config.bytes,
                TypedArtifactDescriptor {
                    media_type: config.media_type,
                    schema_id: Some(config.schema_id),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: typed_events::ArtifactRole::TypedConfig,
                },
            )
            .await
            .expect("persist typed EVM DCV config artifact");
    }
}

fn typed_deploy_configure_validate_config(
    artifact: serde_json::Value,
    from: &str,
    control_scope: &str,
    signing_key_env: &str,
    expected_chain_id: u64,
) -> serde_json::Value {
    serde_json::json!({
        "machine_id": "evm_reth_typed_dcv",
        "pipeline_version": "v1",
        "input": {
            "scenario": "parity_reth_typed_dcv"
        },
        "deploy": {
            "artifact": artifact,
            "network_id": NETWORK_ID,
            "control_scope": control_scope,
            "from": from,
            "signing_key_env": signing_key_env,
            "constructor_args": [1],
            "poll_interval_ms": 200,
            "max_receipt_polls": 120,
        },
        "configure": {
            "network_id": NETWORK_ID,
            "control_scope": control_scope,
            "from": from,
            "signing_key_env": signing_key_env,
            "calls": [
                {"function": "setValue", "args": [7]}
            ],
            "poll_interval_ms": 200,
            "max_receipt_polls": 120,
        },
        "validate": {
            "network_id": NETWORK_ID,
            "control_scope": control_scope,
            "expected_chain_id": expected_chain_id,
            "require_client_substring": "reth",
            "read_assertions": [
                {"function": "getValue", "args": [], "expected": 7}
            ],
            "event_assertions": [
                {"event": "ValueSet", "min_count": 2}
            ],
        },
    })
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

fn summarize_error_details(details: Option<&serde_json::Value>) -> Option<String> {
    details.and_then(|value| serde_json::to_string(value).ok())
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
                last_state_failed = Some((
                    envelope.seq,
                    state_id.to_string(),
                    error.info.code.as_str().to_string(),
                    error.info.retryable,
                    error.info.message,
                    summarize_error_details(error.info.details.as_ref()),
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
    if let Some((seq, state_id, code, retryable, message, details)) = last_state_failed {
        parts.push(format!(
            "state_failed={state_id} seq={seq} code={code} retryable={retryable} message={message}"
        ));
        if let Some(details) = details {
            parts.push(format!("state_failed_details={details}"));
        }
    }
    if parts.len() == 1 {
        parts.push("no_state_failed_event_found".to_string());
    }

    parts.join("; ")
}

const RETH_DEV_ACCOUNT0_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[tokio::test]
async fn parity_reth_pipeline_contract_from_nix() {
    init_test_observability();

    let pg = connect_postgres_with_retry(20, 250).await;
    let streams: Arc<dyn StreamStore> = Arc::new(pg);

    let artifacts = artifact_stores::protected_s3_from_env().await;

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
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope.as_str(),
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
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope.as_str(),
                    "from": from,
                    "signing_key_env": signing_key_env,
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
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope.as_str(),
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

    let bundle = mfm_app_legacy::make_engine_bundle();
    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                streams: Arc::clone(&streams),
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

    if run.phase != RunPhase::Completed {
        let diagnostics = run_failure_diagnostics(Arc::clone(&streams), run.run_id).await;
        panic!("expected Completed, got {:?}; {}", run.phase, diagnostics);
    }
    let final_snapshot_id = run.final_snapshot_id.expect("final snapshot");

    let snapshot_bytes = artifacts
        .get(&final_snapshot_id)
        .await
        .expect("read final snapshot");
    let snapshot: serde_json::Value =
        serde_json::from_slice(&snapshot_bytes).expect("decode snapshot json");

    let contract_address =
        required_snapshot_value(&snapshot, "evm_reth_pipeline.deploy.contract_address")
            .as_str()
            .expect("contract address");
    assert!(contract_address.starts_with("0x"));

    let deploy_tx_hash =
        required_snapshot_value(&snapshot, "evm_reth_pipeline.deploy.deploy_tx_hash")
            .as_str()
            .expect("deploy tx hash");
    assert!(deploy_tx_hash.starts_with("0x"));

    let configure_tx_hashes =
        required_snapshot_value(&snapshot, "evm_reth_pipeline.configure.configure_tx_hashes")
            .as_array()
            .expect("configure tx hashes");
    assert!(!configure_tx_hashes.is_empty());

    assert_eq!(
        required_snapshot_value(&snapshot, "evm_reth_pipeline.validate.validated"),
        &serde_json::json!(true)
    );

    let chain_id = required_snapshot_value(&snapshot, "evm_reth_pipeline.validate.chain_id")
        .as_u64()
        .expect("validate chain id");
    assert_eq!(chain_id, expected_chain_id);

    let client_version =
        required_snapshot_value(&snapshot, "evm_reth_pipeline.validate.client_version")
            .as_str()
            .expect("validate client version");
    assert!(client_version.to_ascii_lowercase().contains("reth"));

    let head = streams
        .head_seq(&StreamId::run(run.run_id))
        .await
        .expect("head seq");
    assert!(head > 0);
    let stream = streams
        .read_range(&StreamId::run(run.run_id), 1, None)
        .await
        .and_then(|records| event_envelopes_from_stream_records(run.run_id, records))
        .expect("event stream");
    assert!(!stream.is_empty());

    write_parity_evm_run_id(&run.run_id);
}

#[tokio::test]
async fn parity_reth_deploy_configure_validate_root_op() {
    init_test_observability();

    let pg = connect_postgres_with_retry(20, 250).await;
    let streams: Arc<dyn StreamStore> = Arc::new(pg);
    let legacy_artifacts = artifact_stores::protected_s3_from_env().await;
    let rpc_sources = rpc_control::required_bootstrap_sources_from_env_for_network(NETWORK_ID);
    let control_scope = format!("{CONTROL_SCOPE}.typed.{}", uuid::Uuid::new_v4().simple());
    let bootstrap_control_scope = format!("{control_scope}.bootstrap");

    let accounts = rpc_call(
        &rpc_sources,
        &bootstrap_control_scope,
        Arc::clone(&streams),
        Arc::clone(&legacy_artifacts),
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
        &rpc_sources,
        &bootstrap_control_scope,
        Arc::clone(&streams),
        Arc::clone(&legacy_artifacts),
        "eth_chainId",
        serde_json::json!([]),
    )
    .await;
    let expected_chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");

    let canonical =
        decode_deploy_configure_validate_canonical_config(&typed_deploy_configure_validate_config(
            contract_artifact_json(),
            &from,
            &control_scope,
            signing_key_env,
            expected_chain_id,
        ))
        .expect("typed EVM DCV canonical config");
    let draft = dcv_program_draft(canonical.clone()).expect("typed EVM DCV draft");
    let certified = certified_dcv_spec(canonical).expect("typed EVM DCV certified spec");
    let public_schema_id = certified
        .envelope
        .spec
        .public_outputs
        .public_schema_id
        .clone();

    let tmp = tempfile::tempdir().expect("typed EVM DCV artifact tempdir");
    let typed_artifacts = FsTypedArtifactStore::new(tmp.path());
    let runners =
        mfm_app::production_typed_runner_registry(typed_artifacts.clone()).expect("typed runners");
    let services = mfm_app::make_in_memory_typed_services(runners, tmp.path());

    persist_dcv_config_artifacts(
        services.artifacts(),
        dcv_config_artifacts_for_spec(&draft, &certified.envelope.spec)
            .expect("typed EVM DCV config artifacts"),
    )
    .await;

    let spec_bytes = certified
        .envelope
        .spec
        .canonical_json()
        .expect("canonical typed spec")
        .to_vec();
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::build_typed_run_start_request(
        services.artifacts(),
        &spec_bytes,
        run_id.clone(),
        "mfm.integration.evm_dcv.typed.v1",
        "integration-test",
        Vec::new(),
        DriveMode::AppendOnly,
    )
    .await
    .expect("typed EVM DCV append-only start request");
    let mut response = services
        .start_certified_run(request)
        .await
        .expect("start typed EVM DCV run");
    assert_eq!(response.phase, mfm_app::TypedRunPhase::Started);
    assert_eq!(response.scheduler_status, "blocked");

    const MAX_TYPED_DCV_RESUMES: usize = 96;
    let mut observed_payloads = BTreeSet::new();
    let mut step_payloads = Vec::<BTreeSet<String>>::new();
    let mut previous_head = response.head_seq;
    let mut single_step_resumes = 0usize;
    for _ in 0..MAX_TYPED_DCV_RESUMES {
        if response.phase == mfm_app::TypedRunPhase::Completed {
            break;
        }
        response = services
            .resume_certified_run(TypedRunResumeRequest {
                envelope: certified.envelope.clone(),
                run_id: run_id.clone(),
                drive: DriveMode::Once,
            })
            .await
            .expect("resume typed EVM DCV run");
        single_step_resumes += 1;
        assert!(
            response.head_seq > previous_head,
            "single-step typed EVM DCV resume made no durable progress before completion"
        );
        let store = services.store();
        let stream = {
            let store = store.lock().await;
            store.load_run_stream(&run_id)
        };
        step_payloads.push(BTreeSet::new());
        for event in stream
            .iter()
            .filter(|event| event.seq().as_u64() > previous_head)
        {
            let payload_name = typed_payload_name(event.payload()).to_owned();
            observed_payloads.insert(payload_name.clone());
            step_payloads
                .last_mut()
                .expect("step payload set exists")
                .insert(payload_name);
        }
        previous_head = response.head_seq;
    }
    assert_eq!(
        response.phase,
        mfm_app::TypedRunPhase::Completed,
        "typed DCV run did not complete within {MAX_TYPED_DCV_RESUMES} single-step resumes after append-only start"
    );
    assert!(
        single_step_resumes >= 8,
        "typed DCV run completed without exercising enough restart boundaries: {single_step_resumes}"
    );
    for expected_payload in [
        "StateAttemptStarted",
        "SideEffectIntentPersisted",
        "SideEffectClaimed",
        "SideEffectInvocationPrepared",
        "SideEffectInvocationStarted",
        "SideEffectSubmissionObserved",
        "SideEffectReceiptObserved",
        "SideEffectConfirmationObserved",
        "FactRecorded",
        "CellProduced",
        "PublicOutputProduced",
        "RetentionManifestProjected",
        "StateAttemptCompleted",
        "RunCompleted",
    ] {
        assert!(
            observed_payloads.contains(expected_payload),
            "typed DCV single-step replay missed payload {expected_payload}; observed={observed_payloads:?}"
        );
    }
    assert_restart_boundary(
        &step_payloads,
        "SideEffectIntentPersisted",
        &["SideEffectClaimed", "SideEffectInvocationPrepared"],
    );
    assert_restart_boundary(
        &step_payloads,
        "SideEffectClaimed",
        &[
            "SideEffectInvocationPrepared",
            "SideEffectInvocationStarted",
        ],
    );
    assert_restart_boundary(
        &step_payloads,
        "SideEffectInvocationPrepared",
        &[
            "SideEffectInvocationStarted",
            "SideEffectSubmissionObserved",
        ],
    );
    assert_restart_boundary(
        &step_payloads,
        "SideEffectInvocationStarted",
        &["SideEffectSubmissionObserved"],
    );

    let public_output = services
        .typed_public_output(&run_id, &public_schema_id)
        .await
        .expect("typed EVM DCV public output");
    let json = public_output.json.expect("rendered typed public output");
    let report = json
        .get("validation_report")
        .expect("validation_report public output");

    assert_eq!(report["valid"], serde_json::json!(true));
    assert_eq!(
        report["event_results"][0]["observed_count"],
        serde_json::json!(2),
        "typed DCV replay boundary fixture should not apply duplicate configure mutations"
    );
    assert_eq!(
        report["observed_chain_id"],
        serde_json::json!(expected_chain_id)
    );
    let contract_address = report["configured_contract"]["contract_address"]
        .as_str()
        .expect("configured contract address");
    assert!(contract_address.starts_with("0x"));
    let client_version = report["client_version"]
        .as_str()
        .expect("validate client version");
    assert!(client_version.to_ascii_lowercase().contains("reth"));

    let stream = {
        let store = services.store();
        let store = store.lock().await;
        store.load_run_stream(&run_id)
    };
    let authority = mfm_app::replay_authority_for_run(
        services.artifacts(),
        &certified.envelope,
        &run_id,
        &stream,
    )
    .await
    .expect("typed EVM DCV replay authority");
    let replay_broker = services
        .replay_broker(certified.envelope.clone(), &run_id, authority)
        .await
        .expect("typed EVM DCV evidence-only replay broker");
    let replay_verified = mfm_transports_evm_dcv::verify_evm_dcv_replay(
        &replay_broker,
        &stream,
        services.artifacts(),
    )
    .await
    .expect("typed EVM DCV replay verifier");
    assert!(
        replay_verified,
        "typed EVM DCV replay evidence was verified"
    );
}

fn typed_payload_name(payload: &typed_events::KernelEventPayload) -> &'static str {
    match payload {
        typed_events::KernelEventPayload::RunStarted(_) => "RunStarted",
        typed_events::KernelEventPayload::StateAttemptStarted(_) => "StateAttemptStarted",
        typed_events::KernelEventPayload::FactRecorded(_) => "FactRecorded",
        typed_events::KernelEventPayload::ArtifactReferenced(_) => "ArtifactReferenced",
        typed_events::KernelEventPayload::CellProduced(_) => "CellProduced",
        typed_events::KernelEventPayload::CellSkipped(_) => "CellSkipped",
        typed_events::KernelEventPayload::SideEffectIntentPersisted(_) => {
            "SideEffectIntentPersisted"
        }
        typed_events::KernelEventPayload::SideEffectClaimed(_) => "SideEffectClaimed",
        typed_events::KernelEventPayload::SideEffectClaimTakenOver(_) => "SideEffectClaimTakenOver",
        typed_events::KernelEventPayload::SideEffectInvocationPrepared(_) => {
            "SideEffectInvocationPrepared"
        }
        typed_events::KernelEventPayload::SideEffectInvocationStarted(_) => {
            "SideEffectInvocationStarted"
        }
        typed_events::KernelEventPayload::SideEffectNotSubmittedProven(_) => {
            "SideEffectNotSubmittedProven"
        }
        typed_events::KernelEventPayload::SideEffectSubmissionObserved(_) => {
            "SideEffectSubmissionObserved"
        }
        typed_events::KernelEventPayload::SideEffectSubmissionUnknown(_) => {
            "SideEffectSubmissionUnknown"
        }
        typed_events::KernelEventPayload::SideEffectReceiptObserved(_) => {
            "SideEffectReceiptObserved"
        }
        typed_events::KernelEventPayload::SideEffectConfirmationObserved(_) => {
            "SideEffectConfirmationObserved"
        }
        typed_events::KernelEventPayload::SideEffectAmbiguous(_) => "SideEffectAmbiguous",
        typed_events::KernelEventPayload::SideEffectFailed(_) => "SideEffectFailed",
        typed_events::KernelEventPayload::PublicOutputProduced(_) => "PublicOutputProduced",
        typed_events::KernelEventPayload::PublicOutputRenderFailed(_) => "PublicOutputRenderFailed",
        typed_events::KernelEventPayload::StateAttemptCompleted(_) => "StateAttemptCompleted",
        typed_events::KernelEventPayload::StateAttemptFailed(_) => "StateAttemptFailed",
        typed_events::KernelEventPayload::RunCompleted(_) => "RunCompleted",
        typed_events::KernelEventPayload::RetentionRefsAppended(_) => "RetentionRefsAppended",
        typed_events::KernelEventPayload::RetentionManifestProjected(_) => {
            "RetentionManifestProjected"
        }
    }
}

fn assert_restart_boundary(
    steps: &[BTreeSet<String>],
    boundary_payload: &str,
    forbidden_same_step: &[&str],
) {
    let Some(step) = steps.iter().find(|step| step.contains(boundary_payload)) else {
        panic!("typed DCV single-step replay never observed {boundary_payload}");
    };
    for forbidden in forbidden_same_step {
        assert!(
            !step.contains(*forbidden),
            "typed DCV did not preserve restart boundary after {boundary_payload}; step={step:?}"
        );
    }
}
