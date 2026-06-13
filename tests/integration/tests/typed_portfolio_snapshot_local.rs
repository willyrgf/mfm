#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{routing::post, Json, Router};
use mfm_app::TypedRunMode;
use serde_json::json;
use tower::ServiceExt;

mod support;

const NETWORK_ID: &str = "typed-local-eth";
static RPC_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn typed_portfolio_snapshot_resumes_from_append_only_start() {
    let _env_guard = RPC_ENV_LOCK.lock().await;
    let rpc_url = start_rpc_mock().await;
    set_rpc_env(rpc_url);

    let result = support::resume_typed_portfolio_snapshot(portfolio_payload()).await;
    assert_eq!(result.started.run_mode, TypedRunMode::Forward);
    assert_eq!(result.resumed.run_mode, TypedRunMode::Completed);
    assert_eq!(result.started.spec_hash, result.resumed.spec_hash);
    assert_eq!(result.authority.spec_hash, result.resumed.spec_hash);
    assert!(!result.authority.certificate_hash.is_empty());
    assert!(result.authority.retained_artifacts > 0);
    assert_eq!(
        result.authority.public_output_event_id,
        result.public_output.event_id
    );
    assert_eq!(
        result.authority.public_output_rendered_digest,
        result.public_output.rendered_digest
    );

    let public_output = result
        .public_output
        .json
        .as_ref()
        .expect("public output json");
    let snapshot = &public_output["snapshot"];
    let report = &public_output["report"];
    assert_eq!(snapshot["portfolio_id"], "typed-local");
    assert_eq!(snapshot["network_pins"][0]["anchor"]["block_number"], 100);
    assert_eq!(
        snapshot["wallets"][0]["observations"][0]["quantity"]["amount_dec"],
        "1.000000000000000000"
    );
    assert_eq!(
        snapshot["wallets"][0]["observations"][0]["values"][0]["value_dec"],
        "2.500000000000000000"
    );
    assert_eq!(report["error_count"], 0);
    assert_eq!(
        report["totals_by_quote"][0]["assets_value_dec"],
        "2.500000000000000000"
    );
}

#[tokio::test]
async fn rest_portfolio_snapshot_matches_typed_public_output() {
    let _env_guard = RPC_ENV_LOCK.lock().await;
    let rpc_url = start_rpc_mock().await;
    set_rpc_env(rpc_url);

    let expected = support::run_typed_portfolio_snapshot(portfolio_payload()).await;
    let expected_public_output = expected
        .public_output
        .json
        .as_ref()
        .expect("expected public output json");
    let app = rest_test_app();
    let response = app
        .oneshot(json_post(
            "/v1/portfolio/snapshot",
            json!({
                "kind": "portfolio_snapshot_start_v1",
                "request": portfolio_payload(),
                "framework_version": "mfm.integration.rest.portfolio.typed.v1",
                "source_revision": "integration-test",
                "drive": "until_blocked"
            }),
        ))
        .await
        .expect("portfolio rest response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = response_json(response).await;
    assert_eq!(body["status"], "success");
    assert_eq!(body["data"]["run"]["run_mode"], "completed");
    assert_eq!(body["data"]["run"]["spec_hash"], expected.run.spec_hash);
    assert!(
        body["data"]["public_output"]["event_id"]
            .as_str()
            .is_some_and(|event_id| event_id.starts_with("event:sha256-jcs-v1:")),
        "REST public-output response must expose typed event evidence"
    );
    assert_eq!(
        body["data"]["public_output"]["rendered_digest"],
        expected.authority.public_output_rendered_digest
    );
    assert_eq!(
        &body["data"]["public_output"]["json"], expected_public_output,
        "REST portfolio route must render the same public JSON as the typed workflow helper"
    );
}

async fn start_rpc_mock() -> String {
    let app = Router::new().route("/", post(rpc_handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc mock");
    let addr = listener.local_addr().expect("rpc mock addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("rpc mock serve");
    });
    format!("http://{addr}")
}

fn set_rpc_env(rpc_url: String) {
    std::env::set_var(
        "MFM_EVM_RPC_SOURCES_JSON",
        json!({
            "sources": [
                {
                    "id": NETWORK_ID,
                    "expected_chain_id": 31337,
                    "rpc_url": rpc_url,
                    "authorization": null
                }
            ],
            "policies": [
                {
                    "id": NETWORK_ID,
                    "ordered_sources": [NETWORK_ID]
                }
            ]
        })
        .to_string(),
    );
}

fn rest_test_app() -> axum::Router {
    let root = std::env::temp_dir().join(format!("mfm-rest-portfolio-{}", uuid::Uuid::new_v4()));
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

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
}

async fn rpc_handler(Json(request): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let id = request.get("id").cloned().unwrap_or_else(|| json!(1));
    let method = request
        .get("method")
        .and_then(|value| value.as_str())
        .expect("json-rpc method");
    let result = match method {
        "eth_chainId" => json!("0x7a69"),
        "eth_getBlockByNumber" => json!({
            "number": "0x64",
            "hash": "0x1111111111111111111111111111111111111111111111111111111111111111"
        }),
        "eth_getBalance" => json!("0xde0b6b3a7640000"),
        other => panic!("unexpected rpc method {other}"),
    };
    Json(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
}

fn portfolio_payload() -> serde_json::Value {
    json!({
        "portfolio": {
            "portfolio_id": "typed-local",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": NETWORK_ID,
                    "family": "evm",
                    "chain_id": 31337,
                    "control_scope": "typed-local",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_local",
                    "address": "0x000000000000000000000000000000000000dead",
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": NETWORK_ID,
                    "symbol_ids": ["eth.native.typed-local-eth"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.typed-local-eth",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": NETWORK_ID,
                    "protocol": null,
                    "balance_reader": {
                        "kind": "native_balance"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.typed-local-eth",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "2.50"
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        },
        "valuation_source_registry": {
            "sources": []
        }
    })
}
