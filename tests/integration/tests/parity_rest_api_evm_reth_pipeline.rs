#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeSet;

use mfm_app::{DriveMode, TypedRunResumeRequest};
use mfm_artifact_store_fs::{FsTypedArtifactStore, TypedArtifactDescriptor};
use mfm_events::v1 as typed_events;
use mfm_op_evm_deploy_configure_validate::{
    certified_dcv_spec, dcv_config_artifacts_for_spec, dcv_program_draft,
    decode_deploy_configure_validate_canonical_config, DcvConfigArtifact,
};
use mfm_store::v1::TypedRunEventStore;
use serde::Deserialize;

const NETWORK_ID: &str = "ethereum-mainnet";
const CONTROL_SCOPE: &str = "parity.evm_reth_pipeline";
const DEFAULT_PARITY_RETH_HTTP_PORT: &str = "8565";
const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";

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

#[derive(Deserialize)]
struct RpcSource {
    network_id: Option<String>,
    rpc_url: String,
}

fn required_rpc_url_for_network(network_id: &str) -> String {
    if let Ok(raw) = std::env::var(ENV_EVM_RPC_SOURCES_JSON) {
        let sources: Vec<RpcSource> =
            serde_json::from_str(&raw).expect("MFM_EVM_RPC_SOURCES_JSON must decode");
        if let Some(source) = sources
            .into_iter()
            .find(|source| source.network_id.as_deref() == Some(network_id))
        {
            return source.rpc_url;
        }
    }

    let port = std::env::var("RETH_HTTP_PORT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PARITY_RETH_HTTP_PORT.to_string());
    format!("http://127.0.0.1:{port}")
}

async fn rpc_call(rpc_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let response = reqwest::Client::new()
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("send json-rpc request")
        .error_for_status()
        .expect("json-rpc http status");
    let payload: serde_json::Value = response.json().await.expect("json-rpc response json");
    if let Some(error) = payload.get("error") {
        panic!("json-rpc {method} returned error: {error}");
    }
    payload
        .get("result")
        .cloned()
        .unwrap_or_else(|| panic!("json-rpc {method} response missing result: {payload}"))
}

const RETH_DEV_ACCOUNT0_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

#[tokio::test]
async fn parity_reth_deploy_configure_validate_root_op() {
    let rpc_url = required_rpc_url_for_network(NETWORK_ID);
    let control_scope = format!("{CONTROL_SCOPE}.typed.{}", uuid::Uuid::new_v4().simple());

    let accounts = rpc_call(&rpc_url, "eth_accounts", serde_json::json!([])).await;
    let from = accounts
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .expect("eth_accounts first address")
        .to_string();
    let signing_key_env = "MFM_EVM_PARITY_DEPLOY_SIGNING_KEY";
    std::env::set_var(signing_key_env, RETH_DEV_ACCOUNT0_PRIVATE_KEY);

    let chain_id_hex = rpc_call(&rpc_url, "eth_chainId", serde_json::json!([])).await;
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
        .envelope()
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
        dcv_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("typed EVM DCV config artifacts"),
    )
    .await;

    let spec_bytes = certified
        .envelope()
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
                certified_spec: certified.clone(),
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
        certified.envelope(),
        &run_id,
        &stream,
    )
    .await
    .expect("typed EVM DCV replay authority");
    let replay_broker = services
        .replay_broker(certified.envelope().clone(), &run_id, authority)
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
