#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_integration_tests::test_support::{self, empty_post, json_post, response_json};

const NETWORK_ID: &str = "reth-local";
const DEFAULT_PARITY_RETH_HTTP_PORT: &str = "8565";
static EVM_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn parity_reth_contract_lifecycle_rest_route_completes_and_replays() {
    let _env_guard = EVM_ENV_LOCK.lock().await;
    let rpc_url = required_rpc_url_for_source(NETWORK_ID);
    let chain_id = rpc_chain_id(&rpc_url).await;
    let wallet = test_support::funded_reth_keystore_wallet(&rpc_url, 0).await;
    let _runtime_config = wallet.set_runtime_config_env_for_test(NETWORK_ID, &rpc_url);

    let app = rest_test_app();
    let config = lifecycle_config(chain_id, wallet.signer_json());
    let compiled_config: mfm_op_evm_contract_lifecycle::ContractLifecycleConfig =
        serde_json::from_value(config.clone()).expect("contract lifecycle config");
    let public_schema_id = public_schema_id_from_plan(
        mfm_op_evm_contract_lifecycle::plan_contract_lifecycle_entry_point(compiled_config)
            .expect("planned lifecycle"),
    );

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "evm_contract_lifecycle",
                "config_format": "json",
                "config": config,
            }),
        ))
        .await
        .expect("contract lifecycle start response");
    let start_status = start.status();
    let mut body = response_json(start).await;
    assert_eq!(start_status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "success");
    let run_id = body["data"]["run"]["run_id"]
        .as_str()
        .expect("run id")
        .to_owned();

    for _ in 0..12 {
        if response_phase(&body) == Some("completed") {
            break;
        }
        let resume = app
            .clone()
            .oneshot(empty_post(&format!("/v1/runs/{run_id}/resume")))
            .await
            .expect("contract lifecycle resume response");
        assert_eq!(resume.status(), StatusCode::OK);
        body = response_json(resume).await;
        assert_eq!(body["status"], "success");
    }

    assert_eq!(response_phase(&body), Some("completed"), "{body}");
    assert!(
        !body.to_string().contains(&wallet.from),
        "runtime responses must not leak signer routing details"
    );
    assert!(
        !wallet.rendered_contains_secret_path(&body.to_string())
            && !wallet.rendered_contains_runtime_signer_config(&body.to_string()),
        "runtime responses must not leak signer provider config"
    );

    let replay = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/replay")))
        .await
        .expect("contract lifecycle replay response");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = response_json(replay).await;
    assert_eq!(replay_body["status"], "success");
    assert_eq!(replay_body["data"]["run_mode"], "completed");
    assert!(
        replay_body["data"]["retained_artifacts"]
            .as_u64()
            .expect("retained artifact count")
            > 0
    );

    let public_output = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{run_id}/public-output/{public_schema_id}"
                ))
                .body(Body::empty())
                .expect("public output request"),
        )
        .await
        .expect("contract lifecycle public output response");
    assert_eq!(public_output.status(), StatusCode::OK);
    let public_output_body = response_json(public_output).await;
    assert_eq!(public_output_body["status"], "success");
    assert_eq!(
        public_output_body["data"]["json"]["validation_report"]["valid"], true,
        "{public_output_body}"
    );
    assert_eq!(
        public_output_body["data"]["json"]["validation_report"]["observed_chain_id"],
        chain_id
    );
}

#[tokio::test]
async fn parity_reth_contract_phase_routes_deploy_configure_and_validate_contract() {
    let _env_guard = EVM_ENV_LOCK.lock().await;
    let rpc_url = required_rpc_url_for_source(NETWORK_ID);
    let chain_id = rpc_chain_id(&rpc_url).await;
    let wallet = test_support::funded_reth_keystore_wallet(&rpc_url, 1).await;
    let _runtime_config = wallet.set_runtime_config_env_for_test(NETWORK_ID, &rpc_url);

    let artifact = configurable_contract_artifact();
    let deploy_app = rest_test_app();
    let deploy_config = deploy_config(chain_id, wallet.signer_json(), artifact.clone());
    let deploy_schema_id = deploy_public_schema_id(deploy_config.clone());
    let (_, deploy_output) = start_contract_phase(
        &deploy_app,
        "evm_contract_deploy",
        deploy_config,
        &deploy_schema_id,
        &wallet,
    )
    .await;
    let deployed = deploy_output["data"]["json"]["deployed"].clone();
    assert_eq!(deployed["network_id"], NETWORK_ID);
    assert_eq!(deployed["expected_chain_id"], chain_id);
    assert_ne!(
        deployed["contract_address"],
        serde_json::Value::Null,
        "{deploy_output}"
    );
    assert_ne!(
        deployed["deploy_tx_hash"],
        serde_json::Value::Null,
        "{deploy_output}"
    );
    assert_ne!(
        deployed["deployed_block_number"],
        serde_json::Value::Null,
        "{deploy_output}"
    );
    assert!(
        deployed
            .as_object()
            .expect("deployed object")
            .contains_key("deploy_receipt_evidence"),
        "{deploy_output}"
    );

    let configure_app = rest_test_app();
    let configure_config = configure_config(chain_id, wallet.signer_json(), artifact.clone());
    let configure_schema_id =
        configure_public_schema_id(configure_config.clone(), deployed.clone());
    let (_, configure_output) = start_contract_phase(
        &configure_app,
        "evm_contract_configure",
        serde_json::json!({
            "config": configure_config,
            "deployed": deployed
        }),
        &configure_schema_id,
        &wallet,
    )
    .await;
    let configured = configure_output["data"]["json"]["configured"].clone();
    assert_eq!(
        configured["configure_calls"]
            .as_array()
            .expect("configure calls")
            .len(),
        1,
        "{configure_output}"
    );
    assert_eq!(
        configured["configure_tx_hashes"]
            .as_array()
            .expect("configure tx hashes")
            .len(),
        1,
        "{configure_output}"
    );
    assert_ne!(
        configured["configured_block_number"],
        serde_json::Value::Null,
        "{configure_output}"
    );
    assert!(
        configured["configure_receipt_evidence"]
            .as_array()
            .expect("configure receipt evidence")
            .is_empty(),
        "{configure_output}"
    );

    let validate_app = rest_test_app();
    let validate_config = validate_config(chain_id, artifact);
    let validate_schema_id = validate_public_schema_id(validate_config.clone(), configured.clone());
    let (_, validate_output) = start_contract_phase(
        &validate_app,
        "evm_contract_validate",
        serde_json::json!({
            "config": validate_config,
            "configured": configured
        }),
        &validate_schema_id,
        &wallet,
    )
    .await;
    let report = &validate_output["data"]["json"]["validation_report"];
    assert_eq!(report["valid"], true, "{validate_output}");
    assert_eq!(report["observed_chain_id"], chain_id);
    assert_eq!(report["configuration_read_results"][0]["function"], "ready");
    assert_eq!(report["configuration_read_results"][0]["passed"], true);
    assert_eq!(
        report["configuration_event_results"][0]["event"],
        "Configured"
    );
    assert_eq!(
        report["configuration_event_results"][0]["passed"], true,
        "{validate_output}"
    );
    assert_eq!(report["read_results"][0]["function"], "ready");
    assert_eq!(report["read_results"][0]["passed"], true);
    assert_eq!(report["event_results"][0]["event"], "Configured");
    assert_eq!(report["event_results"][0]["passed"], true);
}

fn lifecycle_config(chain_id: u64, signer: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "deploy": {
            "artifact": empty_contract_artifact(),
            "network": network_json(chain_id),
            "signer": signer,
            "receipt": {
                "poll_interval_ms": 25,
                "max_receipt_polls": 80
            }
        },
        "configure": {
            "network": network_json(chain_id),
            "signer": signer,
            "calls": [],
            "receipt": {
                "poll_interval_ms": 25,
                "max_receipt_polls": 80
            }
        },
        "validate": {
            "network": network_json(chain_id),
            "validation": {
                "read_assertions": [],
                "event_assertions": []
            }
        }
    })
}

fn deploy_config(
    chain_id: u64,
    signer: serde_json::Value,
    artifact: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "artifact": artifact,
        "network": network_json(chain_id),
        "signer": signer,
        "receipt": {
            "poll_interval_ms": 25,
            "max_receipt_polls": 80
        }
    })
}

fn configure_config(
    chain_id: u64,
    signer: serde_json::Value,
    artifact: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "artifact": artifact,
        "network": network_json(chain_id),
        "signer": signer,
        "calls": [
            {
                "function": "configure",
                "args": []
            }
        ],
        "confirmation_read_assertions": read_assertions(),
        "confirmation_event_assertions": event_assertions(),
        "receipt": {
            "poll_interval_ms": 25,
            "max_receipt_polls": 80
        }
    })
}

fn validate_config(chain_id: u64, artifact: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "artifact": artifact,
        "network": network_json(chain_id),
        "validation": {
            "read_assertions": read_assertions(),
            "event_assertions": event_assertions()
        }
    })
}

fn read_assertions() -> serde_json::Value {
    serde_json::json!([
        {
            "function": "ready",
            "args": [],
            "expected": {
                "json_text": "true"
            }
        }
    ])
}

fn event_assertions() -> serde_json::Value {
    serde_json::json!([
        {
            "event": "Configured",
            "min_count": 1
        }
    ])
}

fn empty_contract_artifact() -> serde_json::Value {
    serde_json::json!({
        "abi": {
            "json_text": "[]"
        },
        "bytecode": {
            "json_text": "{\"object\":\"0x60006000f3\"}"
        }
    })
}

fn configurable_contract_artifact() -> serde_json::Value {
    let abi = serde_json::json!([
        {
            "type": "function",
            "name": "configure",
            "inputs": [],
            "outputs": [],
            "stateMutability": "nonpayable"
        },
        {
            "type": "function",
            "name": "ready",
            "inputs": [],
            "outputs": [
                {
                    "name": "",
                    "type": "bool"
                }
            ],
            "stateMutability": "view"
        },
        {
            "type": "event",
            "name": "Configured",
            "inputs": [],
            "anonymous": false
        }
    ]);
    let bytecode = serde_json::json!({
        "object": configurable_contract_creation_bytecode()
    });
    serde_json::json!({
        "abi": {
            "json_text": abi.to_string()
        },
        "bytecode": {
            "json_text": bytecode.to_string()
        }
    })
}

fn configurable_contract_creation_bytecode() -> String {
    let ready_selector = selector("ready()");
    let configure_selector = selector("configure()");
    let configured_topic = alloy_primitives::keccak256("Configured()");

    let mut runtime = Vec::new();
    runtime.extend_from_slice(&[0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c]);
    runtime.extend_from_slice(&[0x80, 0x63]);
    runtime.extend_from_slice(&ready_selector);
    runtime.extend_from_slice(&[0x14, 0x60, 0x1f, 0x57]);
    runtime.extend_from_slice(&[0x80, 0x63]);
    runtime.extend_from_slice(&configure_selector);
    runtime.extend_from_slice(&[0x14, 0x60, 0x2b, 0x57]);
    runtime.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0xfd]);
    runtime.extend_from_slice(&[0x5b, 0x60, 0x00, 0x54, 0x60, 0x00, 0x52]);
    runtime.extend_from_slice(&[0x60, 0x20, 0x60, 0x00, 0xf3]);
    runtime.extend_from_slice(&[0x5b, 0x60, 0x01, 0x60, 0x00, 0x55, 0x7f]);
    runtime.extend_from_slice(configured_topic.as_slice());
    runtime.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0xa1]);
    runtime.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0xf3]);
    assert_eq!(runtime.len(), 92);

    let runtime_len = u8::try_from(runtime.len()).expect("runtime fits in PUSH1");
    let mut creation = vec![
        0x60,
        runtime_len,
        0x60,
        0x0c,
        0x60,
        0x00,
        0x39,
        0x60,
        runtime_len,
        0x60,
        0x00,
        0xf3,
    ];
    creation.extend_from_slice(&runtime);
    format!("0x{}", hex_encode(&creation))
}

fn selector(signature: &str) -> [u8; 4] {
    let digest = alloy_primitives::keccak256(signature);
    let mut selector = [0_u8; 4];
    selector.copy_from_slice(&digest.as_slice()[..4]);
    selector
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn deploy_public_schema_id(config: serde_json::Value) -> String {
    let config: mfm_evm_contract_config::DeployPhaseConfig =
        serde_json::from_value(config).expect("deploy phase config");
    public_schema_id_from_plan(
        mfm_op_evm_contract_lifecycle::plan_contract_deploy_entry_point(config)
            .expect("planned deploy"),
    )
}

fn configure_public_schema_id(config: serde_json::Value, deployed: serde_json::Value) -> String {
    let config: mfm_evm_contract_config::ConfigurePhaseConfig =
        serde_json::from_value(config).expect("configure phase config");
    let deployed: mfm_evm_contract_model::DeployedContract =
        serde_json::from_value(deployed).expect("deployed contract");
    public_schema_id_from_plan(
        mfm_op_evm_contract_lifecycle::plan_contract_configure_entry_point(
            mfm_op_evm_contract_lifecycle::ContractConfigureEntryPointConfig { config, deployed },
        )
        .expect("planned configure"),
    )
}

fn validate_public_schema_id(config: serde_json::Value, configured: serde_json::Value) -> String {
    let config: mfm_evm_contract_config::ValidatePhaseConfig =
        serde_json::from_value(config).expect("validate phase config");
    let configured: mfm_evm_contract_model::ConfiguredContract =
        serde_json::from_value(configured).expect("configured contract");
    public_schema_id_from_plan(
        mfm_op_evm_contract_lifecycle::plan_contract_validate_entry_point(
            mfm_op_evm_contract_lifecycle::ContractValidateEntryPointConfig { config, configured },
        )
        .expect("planned validate"),
    )
}

fn public_schema_id_from_plan(plan: mfm_program::TypedProgramLaunchPlan) -> String {
    mfm_certify::certify_program_draft(&plan.draft)
        .expect("certified entry-point plan")
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .to_string()
}

async fn start_contract_phase(
    app: &axum::Router,
    op: &str,
    config: serde_json::Value,
    public_schema_id: &str,
    wallet: &test_support::FundedRethKeystoreWallet,
) -> (String, serde_json::Value) {
    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": op,
                "config_format": "json",
                "config": config,
            }),
        ))
        .await
        .expect("contract phase start response");
    let start_status = start.status();
    let mut body = response_json(start).await;
    assert_eq!(start_status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "success");
    let run_id = body["data"]["run"]["run_id"]
        .as_str()
        .expect("run id")
        .to_owned();

    for _ in 0..12 {
        if response_phase(&body) == Some("completed") {
            break;
        }
        let resume = app
            .clone()
            .oneshot(empty_post(&format!("/v1/runs/{run_id}/resume")))
            .await
            .expect("contract phase resume response");
        assert_eq!(resume.status(), StatusCode::OK);
        body = response_json(resume).await;
        assert_eq!(body["status"], "success");
    }

    assert_eq!(response_phase(&body), Some("completed"), "{body}");
    assert_no_runtime_signer_leak(&body, wallet);
    let public_output = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{run_id}/public-output/{public_schema_id}"
                ))
                .body(Body::empty())
                .expect("public output request"),
        )
        .await
        .expect("contract phase public output response");
    assert_eq!(public_output.status(), StatusCode::OK);
    let public_output = response_json(public_output).await;
    assert_eq!(public_output["status"], "success");
    assert_no_runtime_signer_leak(&public_output, wallet);
    (run_id, public_output)
}

fn assert_no_runtime_signer_leak(
    body: &serde_json::Value,
    wallet: &test_support::FundedRethKeystoreWallet,
) {
    let rendered = body.to_string();
    assert!(
        !rendered.contains(&wallet.from),
        "runtime responses must not leak signer routing details"
    );
    assert!(
        !wallet.rendered_contains_secret_path(&rendered)
            && !wallet.rendered_contains_runtime_signer_config(&rendered),
        "runtime responses must not leak signer provider config"
    );
}

fn network_json(chain_id: u64) -> serde_json::Value {
    serde_json::json!({
        "network_id": NETWORK_ID,
        "expected_chain_id": chain_id,
    })
}

fn required_rpc_url_for_source(source_id: &str) -> String {
    assert_eq!(source_id, NETWORK_ID);
    let port = std::env::var("RETH_HTTP_PORT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PARITY_RETH_HTTP_PORT.to_string());
    format!("http://127.0.0.1:{port}")
}

async fn rpc_chain_id(rpc_url: &str) -> u64 {
    let chain_id_hex = test_support::rpc_call(rpc_url, "eth_chainId", serde_json::json!([])).await;
    let raw = chain_id_hex.as_str().expect("eth_chainId hex");
    u64::from_str_radix(raw.strip_prefix("0x").expect("chain id hex"), 16).expect("chain id parses")
}

fn rest_test_app() -> axum::Router {
    mfm_rest_api::make_app(test_support::in_memory_rest_app_state())
}

fn response_phase(body: &serde_json::Value) -> Option<&str> {
    body["data"]["run"]["run_mode"]
        .as_str()
        .or_else(|| body["data"]["run_mode"].as_str())
}
