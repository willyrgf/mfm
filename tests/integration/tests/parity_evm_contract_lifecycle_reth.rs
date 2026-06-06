#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_integration_tests::test_support;

const NETWORK_ID: &str = "reth-local";
const DEFAULT_PARITY_RETH_HTTP_PORT: &str = "8565";
const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";
const ENV_EVM_SIGNERS_JSON: &str = "MFM_EVM_SIGNERS_JSON";
const ENV_EVM_CONTRACT_SOURCE_REF: &str = "MFM_EVM_CONTRACT_SOURCE_REF";
const ENV_EVM_CONTRACT_SOURCE_POLICY_ID: &str = "MFM_EVM_CONTRACT_SOURCE_POLICY_ID";
static EVM_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn parity_reth_contract_lifecycle_rest_route_completes_and_replays() {
    let _env_guard = EVM_ENV_LOCK.lock().await;
    let rpc_url = required_rpc_url_for_source(NETWORK_ID);
    let chain_id = rpc_chain_id(&rpc_url).await;
    let wallet = test_support::funded_reth_keystore_wallet(&rpc_url, 0).await;
    let _restore = EnvRestore::set([
        (
            ENV_EVM_RPC_SOURCES_JSON,
            wallet
                .runtime_source_registry_json(NETWORK_ID, chain_id, &rpc_url)
                .to_string(),
        ),
        (
            ENV_EVM_SIGNERS_JSON,
            wallet.runtime_signer_registry_json().to_string(),
        ),
        (ENV_EVM_CONTRACT_SOURCE_REF, NETWORK_ID.to_owned()),
        (ENV_EVM_CONTRACT_SOURCE_POLICY_ID, NETWORK_ID.to_owned()),
    ]);

    let app = rest_test_app();
    let config = lifecycle_config(chain_id, wallet.signer_json());
    let compiled_config: mfm_op_evm_contract_lifecycle::ContractLifecycleConfig =
        serde_json::from_value(config.clone()).expect("contract lifecycle config");
    let public_schema_id =
        mfm_op_evm_contract_lifecycle::compile_contract_lifecycle_program(compiled_config)
            .expect("compiled lifecycle")
            .public_schema_id
            .to_string();

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/evm/contracts/lifecycle",
            serde_json::json!({
                "kind": "evm_contract_lifecycle_start_v1",
                "config": config,
                "framework_version": "mfm.integration.rest.evm_contracts.typed.v1",
                "source_revision": "integration-test",
                "drive": "until_blocked"
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
    assert_eq!(replay_body["data"]["phase"], "completed");
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

fn lifecycle_config(chain_id: u64, signer: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "lifecycle_version": 1,
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

fn network_json(chain_id: u64) -> serde_json::Value {
    serde_json::json!({
        "network_id": NETWORK_ID,
        "expected_chain_id": chain_id,
    })
}

fn required_rpc_url_for_source(source_id: &str) -> String {
    if let Ok(raw) = std::env::var(ENV_EVM_RPC_SOURCES_JSON) {
        let registry: serde_json::Value =
            serde_json::from_str(&raw).expect("MFM_EVM_RPC_SOURCES_JSON must decode");
        if let Some(source) = registry
            .get("sources")
            .and_then(|sources| sources.as_array())
            .and_then(|sources| {
                sources.iter().find(|source| {
                    source
                        .get("id")
                        .and_then(|value| value.as_str())
                        .is_some_and(|id| id == source_id)
                })
            })
            .and_then(|source| source.get("rpc_url"))
            .and_then(|value| value.as_str())
        {
            return source.to_owned();
        }
    }

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
    let root = std::env::temp_dir().join(format!(
        "mfm-rest-contract-lifecycle-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("typed artifact root");
    mfm_rest_api::make_app(mfm_rest_api::make_in_memory_app_state(root))
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

fn empty_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
}

fn response_phase(body: &serde_json::Value) -> Option<&str> {
    body["data"]["run"]["phase"]
        .as_str()
        .or_else(|| body["data"]["phase"].as_str())
}

struct EnvRestore {
    previous: Vec<(&'static str, Option<String>)>,
}

impl EnvRestore {
    fn set<const N: usize>(values: [(&'static str, String); N]) -> Self {
        let previous = values
            .iter()
            .map(|(name, _)| (*name, std::env::var(name).ok()))
            .collect::<Vec<_>>();
        for (name, value) in values {
            std::env::set_var(name, value);
        }
        Self { previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..) {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}
