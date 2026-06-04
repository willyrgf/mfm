#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeSet;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_app::{DriveMode, RunLaunchConfigArtifact};
use mfm_artifact_store_fs::FsTypedArtifactStore;
use mfm_events::v1 as typed_events;
use mfm_ids::ArtifactId;
use mfm_integration_tests::test_support::{funded_reth_keystore_wallet, rpc_call};
use mfm_op_evm_deploy_configure_validate::{
    certified_dcv_spec, dcv_config_artifacts_for_spec, dcv_program_draft,
    decode_deploy_configure_validate_canonical_config, DcvConfigArtifact,
};
use mfm_store::v1 as typed_store;
use mfm_store::v1::TypedRunEventStore;
use mfm_transports_evm_dcv::EvmDcvArtifactReader;
use serde::Deserialize;
use tower::ServiceExt;

const NETWORK_ID: &str = "ethereum-mainnet";
const CONTROL_SCOPE: &str = "parity.evm_reth_pipeline";
const DEFAULT_PARITY_RETH_HTTP_PORT: &str = "8565";
const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";
static EVM_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

fn typed_config_inputs(configs: Vec<DcvConfigArtifact>) -> Vec<RunLaunchConfigArtifact> {
    configs
        .into_iter()
        .map(|config| RunLaunchConfigArtifact {
            schema_id: config.schema_id,
            bytes: config.bytes,
            media_type: config.media_type,
        })
        .collect()
}

fn typed_deploy_configure_validate_config(
    artifact: serde_json::Value,
    from: &str,
    control_scope: &str,
    signer: &serde_json::Value,
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
            "signer": signer.clone(),
            "constructor_args": [1],
            "poll_interval_ms": 200,
            "max_receipt_polls": 120,
        },
        "configure": {
            "network_id": NETWORK_ID,
            "control_scope": control_scope,
            "from": from,
            "signer": signer.clone(),
            "calls": [
                {"function": "setValue", "args": [7]}
            ],
            "confirmation_read_assertions": [
                {"function": "getValue", "args": [], "expected": 7}
            ],
            "confirmation_event_assertions": [
                {"event": "ValueSet", "min_count": 2}
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

fn typed_deploy_config(
    artifact: serde_json::Value,
    from: &str,
    control_scope: &str,
    signer: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "artifact": artifact,
        "network_id": NETWORK_ID,
        "control_scope": control_scope,
        "from": from,
        "signer": signer.clone(),
        "constructor_args": [1],
        "poll_interval_ms": 200,
        "max_receipt_polls": 120,
    })
}

fn typed_configure_config(
    artifact: serde_json::Value,
    from: &str,
    control_scope: &str,
    signer: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "artifact": artifact,
        "network_id": NETWORK_ID,
        "control_scope": control_scope,
        "from": from,
        "signer": signer.clone(),
        "calls": [
            {"function": "setValue", "args": [7]}
        ],
        "confirmation_read_assertions": [
            {"function": "getValue", "args": [], "expected": 7}
        ],
        "confirmation_event_assertions": [
            {"event": "ValueSet", "min_count": 2}
        ],
        "poll_interval_ms": 200,
        "max_receipt_polls": 120,
    })
}

fn typed_validate_config(
    artifact: serde_json::Value,
    control_scope: &str,
    expected_chain_id: u64,
) -> serde_json::Value {
    serde_json::json!({
        "artifact": artifact,
        "network_id": NETWORK_ID,
        "control_scope": control_scope,
        "expected_chain_id": expected_chain_id,
        "require_client_substring": "reth",
    })
}

fn rest_test_app() -> (axum::Router, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("typed EVM DCV REST tempdir");
    let app = mfm_rest_api::make_app(mfm_rest_api::make_in_memory_app_state(tmp.path()));
    (app, tmp)
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&body).expect("request body serializes"),
        ))
        .expect("request")
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("response json")
}

async fn rest_post_json(
    app: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(json_post(uri, body))
        .await
        .expect("REST response");
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "REST POST {uri} returned {status}: {body}"
    );
    assert_eq!(body["status"], "success");
    body
}

async fn rest_get_json(app: &axum::Router, uri: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("REST response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "success");
    body
}

async fn run_evm_dcv_phase_to_completion(
    app: &axum::Router,
    uri: &str,
    mut body: serde_json::Value,
) -> (String, String, serde_json::Value) {
    body["drive"] = serde_json::json!("append_only");
    let started = rest_post_json(app, uri, body).await;
    let run_id = started["data"]["run"]["run_id"]
        .as_str()
        .expect("run id")
        .to_owned();
    let public_schema_id = started["data"]["public_schema_id"]
        .as_str()
        .expect("public schema id")
        .to_owned();

    let mut terminal = started["data"]["run"].clone();
    for _ in 0..96 {
        if terminal["phase"] == "completed" {
            break;
        }
        let resumed = rest_post_json(
            app,
            &format!("/v1/runs/{run_id}/resume"),
            serde_json::json!({"drive": "once"}),
        )
        .await;
        terminal = resumed["data"].clone();
    }
    assert_eq!(terminal["phase"], "completed");

    let replay = rest_post_json(
        app,
        &format!("/v1/runs/{run_id}/replay"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(replay["data"]["phase"], "completed");

    let public_output = rest_get_json(
        app,
        &format!("/v1/runs/{run_id}/public-output/{public_schema_id}"),
    )
    .await;
    (
        run_id,
        public_schema_id,
        public_output["data"]["json"].clone(),
    )
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

#[tokio::test]
async fn parity_reth_evm_dcv_public_phase_lifecycle_rest() {
    let _env_guard = EVM_ENV_LOCK.lock().await;
    let rpc_url = required_rpc_url_for_network(NETWORK_ID);
    let control_scope = format!("{CONTROL_SCOPE}.phase.{}", uuid::Uuid::new_v4().simple());

    let wallet = funded_reth_keystore_wallet(&rpc_url, 1).await;
    let signer = wallet.signer_json();

    let chain_id_hex = rpc_call(&rpc_url, "eth_chainId", serde_json::json!([])).await;
    let expected_chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");
    let artifact = contract_artifact_json();

    let (deploy_app, _deploy_tmp) = rest_test_app();
    let (_, _, deploy_public) = run_evm_dcv_phase_to_completion(
        &deploy_app,
        "/v1/evm/dcv/deploy",
        serde_json::json!({
            "kind": "evm_dcv_deploy_start_v1",
            "request": typed_deploy_config(
                artifact.clone(),
                &wallet.from,
                &control_scope,
                &signer,
            ),
            "framework_version": "mfm.integration.evm_dcv.rest.deploy.v1",
            "source_revision": "integration-test",
        }),
    )
    .await;
    let deployed_contract = deploy_public
        .get("deployed_contract")
        .cloned()
        .expect("deployed_contract public output");

    let (configure_app, _configure_tmp) = rest_test_app();
    let (_, _, configure_public) = run_evm_dcv_phase_to_completion(
        &configure_app,
        "/v1/evm/dcv/configure",
        serde_json::json!({
            "kind": "evm_dcv_configure_start_v1",
            "request": typed_configure_config(
                artifact.clone(),
                &wallet.from,
                &control_scope,
                &signer,
            ),
            "deployed_contract": deployed_contract,
            "framework_version": "mfm.integration.evm_dcv.rest.configure.v1",
            "source_revision": "integration-test",
        }),
    )
    .await;
    let configured_contract = configure_public
        .get("configured_contract")
        .cloned()
        .expect("configured_contract public output");

    let (validate_app, _validate_tmp) = rest_test_app();
    let (_, _, validate_public) = run_evm_dcv_phase_to_completion(
        &validate_app,
        "/v1/evm/dcv/validate",
        serde_json::json!({
            "kind": "evm_dcv_validate_start_v1",
            "request": typed_validate_config(artifact, &control_scope, expected_chain_id),
            "configured_contract": configured_contract,
            "framework_version": "mfm.integration.evm_dcv.rest.validate.v1",
            "source_revision": "integration-test",
        }),
    )
    .await;
    let report = validate_public
        .get("validation_report")
        .expect("validation_report public output");

    assert_eq!(report["valid"], serde_json::json!(true));
    assert_eq!(
        report["configuration_read_results"][0]["actual"]["json_text"],
        serde_json::json!("7")
    );
    assert_eq!(
        report["configuration_event_results"][0]["observed_count"],
        serde_json::json!(2)
    );
    assert_eq!(
        report["observed_chain_id"],
        serde_json::json!(expected_chain_id)
    );
    assert!(report["client_version"]
        .as_str()
        .expect("client version")
        .to_ascii_lowercase()
        .contains("reth"));
    let rendered = serde_json::to_string(&validate_public).expect("rendered public output");
    assert!(!wallet.rendered_contains_secret_path(&rendered));
    assert!(!rendered.contains(&rpc_url));
    assert!(!rendered.contains("raw_transaction_hex"));
}

#[tokio::test]
async fn parity_reth_deploy_configure_validate_root_op() {
    let _env_guard = EVM_ENV_LOCK.lock().await;
    let rpc_url = required_rpc_url_for_network(NETWORK_ID);
    let control_scope = format!("{CONTROL_SCOPE}.typed.{}", uuid::Uuid::new_v4().simple());

    let wallet = funded_reth_keystore_wallet(&rpc_url, 0).await;
    let signer = wallet.signer_json();

    let chain_id_hex = rpc_call(&rpc_url, "eth_chainId", serde_json::json!([])).await;
    let expected_chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");

    let canonical =
        decode_deploy_configure_validate_canonical_config(&typed_deploy_configure_validate_config(
            contract_artifact_json(),
            &wallet.from,
            &control_scope,
            &signer,
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
    let registry = mfm_app::production_certification_registry().expect("certification registry");
    let services = mfm_app::make_in_memory_typed_services_with_certification_registry(
        runners,
        tmp.path(),
        registry.clone(),
    );

    let config_inputs = typed_config_inputs(
        dcv_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("typed EVM DCV config artifacts"),
    );

    let bundle = certified.bundle().expect("typed EVM DCV certified bundle");
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::prepare_verified_bundle_launch(
        mfm_app::UntrustedCertifiedBundleLaunchInput {
            spec_bytes: bundle.spec_bytes(),
            certificate_bytes: bundle.certificate_bytes(),
            registry: &registry,
            run_id: run_id.clone(),
            framework_version: "mfm.integration.evm_dcv.typed.v1",
            source_revision: "integration-test",
            drive: DriveMode::AppendOnly,
        },
        config_inputs,
        Vec::new(),
    )
    .expect("typed EVM DCV append-only start request");
    let mut response = services
        .launch_run(request)
        .await
        .expect("start typed EVM DCV run");
    assert_eq!(response.phase, mfm_app::TypedRunPhase::Started);
    assert_eq!(response.scheduler_status, "blocked");

    const MAX_TYPED_DCV_RESUMES: usize = 96;
    const MAX_RESUME_ATTEMPTS_PER_STEP: usize = 4;
    let mut observed_payloads = BTreeSet::new();
    let mut step_payloads = Vec::<BTreeSet<String>>::new();
    let mut previous_head = response.head_seq;
    let mut single_step_resumes = 0usize;
    for _ in 0..MAX_TYPED_DCV_RESUMES {
        if response.phase == mfm_app::TypedRunPhase::Completed {
            break;
        }

        let mut advanced_this_step = false;
        for attempt in 0..MAX_RESUME_ATTEMPTS_PER_STEP {
            response = services
                .resume_stored_run(&run_id, DriveMode::Once)
                .await
                .expect("resume typed EVM DCV run");
            if response.head_seq > previous_head
                || response.phase == mfm_app::TypedRunPhase::Completed
            {
                advanced_this_step = true;
                single_step_resumes += 1;
                break;
            }

            if attempt + 1 < MAX_RESUME_ATTEMPTS_PER_STEP {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }

        assert!(
            advanced_this_step,
            "single-step typed EVM DCV resume made no durable progress after {MAX_RESUME_ATTEMPTS_PER_STEP} retries"
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
    let rendered_public_output = serde_json::to_string(&json).expect("rendered public output");
    assert!(!wallet.rendered_contains_secret_path(&rendered_public_output));
    assert!(!rendered_public_output.contains(&rpc_url));
    assert!(!rendered_public_output.contains("raw_transaction_hex"));
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
    let replay_broker = services
        .replay_broker(&run_id)
        .await
        .expect("typed EVM DCV evidence-only replay broker");
    let replay_verified =
        mfm_transports_evm_dcv::verify_evm_dcv_replay(&replay_broker, services.artifacts())
            .await
            .expect("typed EVM DCV replay verifier");
    assert!(
        replay_verified,
        "typed EVM DCV replay evidence was verified"
    );

    let validate_fact = validate_fact_recorded(&stream);
    let fact_artifact = validate_fact.artifact_id.clone();
    let output_artifact = validate_output_artifact_id(&stream, validate_fact);
    let configured_input_artifact =
        validate_configured_input_artifact_id(&certified.envelope().spec, &stream, validate_fact);

    let fact_err = mfm_transports_evm_dcv::verify_evm_dcv_replay(
        &replay_broker,
        &ReplacementArtifactReader {
            inner: services.artifacts(),
            target: fact_artifact,
            replacement: b"not json".to_vec(),
        },
    )
    .await
    .expect_err("tampered fact artifact rejects replay");
    assert_eq!(fact_err.code(), "MFM_REPLAY_ARTIFACT_MISMATCH");

    let output_err = mfm_transports_evm_dcv::verify_evm_dcv_replay(
        &replay_broker,
        &ReplacementArtifactReader {
            inner: services.artifacts(),
            target: output_artifact,
            replacement: b"not json".to_vec(),
        },
    )
    .await
    .expect_err("tampered validate output artifact rejects replay");
    assert_eq!(output_err.code(), "MFM_REPLAY_ARTIFACT_MISMATCH");

    let configured_bytes =
        tampered_configured_contract_bytes(services.artifacts(), &configured_input_artifact).await;
    let domain_err = mfm_transports_evm_dcv::verify_evm_dcv_replay(
        &replay_broker,
        &ReplacementArtifactReader {
            inner: services.artifacts(),
            target: configured_input_artifact,
            replacement: configured_bytes,
        },
    )
    .await
    .expect_err("tampered configured-contract domain evidence rejects replay");
    assert_eq!(domain_err.code(), "MFM_REPLAY_FACT_MISMATCH");
}

struct ReplacementArtifactReader<'a> {
    inner: &'a FsTypedArtifactStore,
    target: ArtifactId,
    replacement: Vec<u8>,
}

impl EvmDcvArtifactReader for ReplacementArtifactReader<'_> {
    fn get_artifact_by_id<'a>(
        &'a self,
        artifact_id: &'a ArtifactId,
    ) -> mfm_transports_evm_dcv::EvmDcvArtifactReaderFuture<
        'a,
        (Vec<u8>, typed_store::ArtifactEvidenceRef),
    > {
        Box::pin(async move {
            let (bytes, evidence) =
                <FsTypedArtifactStore as EvmDcvArtifactReader>::get_artifact_by_id(
                    self.inner,
                    artifact_id,
                )
                .await?;
            if artifact_id == &self.target {
                Ok((self.replacement.clone(), evidence))
            } else {
                Ok((bytes, evidence))
            }
        })
    }
}

fn validate_fact_recorded(
    stream: &[typed_store::KernelEventEnvelope],
) -> &typed_events::FactRecorded {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            typed_events::KernelEventPayload::FactRecorded(payload) => Some(payload),
            _ => None,
        })
        .expect("typed DCV replay fixture records a validation fact")
}

fn validate_output_artifact_id(
    stream: &[typed_store::KernelEventEnvelope],
    fact: &typed_events::FactRecorded,
) -> ArtifactId {
    stream
        .iter()
        .find_map(|event| match event.payload() {
            typed_events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == fact.node_id =>
            {
                Some(payload.artifact_id.clone())
            }
            _ => None,
        })
        .expect("typed DCV replay fixture produces a validation output")
}

fn validate_configured_input_artifact_id(
    spec: &mfm_spec::v1::TypedExecutionSpec,
    stream: &[typed_store::KernelEventEnvelope],
    fact: &typed_events::FactRecorded,
) -> ArtifactId {
    let validate_node = spec
        .nodes
        .iter()
        .find(|node| node.node_id == fact.node_id)
        .expect("validation node in certified spec");
    let input_cell = match &validate_node.input_bindings.root {
        mfm_spec::v1::InputBindingNodeSpec::Cell(cell) => cell,
        _ => panic!("validation node must consume configured-contract cell"),
    };
    stream
        .iter()
        .find_map(|event| match event.payload() {
            typed_events::KernelEventPayload::CellProduced(payload)
                if payload.cell_id == input_cell.cell_id =>
            {
                Some(payload.artifact_id.clone())
            }
            _ => None,
        })
        .expect("typed DCV replay fixture produces configured-contract input")
}

async fn tampered_configured_contract_bytes(
    artifacts: &FsTypedArtifactStore,
    artifact_id: &ArtifactId,
) -> Vec<u8> {
    let (bytes, _) = artifacts
        .get_artifact_by_id(artifact_id)
        .await
        .expect("configured-contract artifact");
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("configured-contract JSON");
    value["deployed"]["control_scope"] = serde_json::json!("tampered-control-scope");
    serde_json::to_vec(&value).expect("tampered configured-contract JSON")
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
