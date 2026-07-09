//! Portfolio Reth parity feature tests after the fact-backed cutover.
//!
//! Report is Platform-fact-only. Live Reth crawl is not report authority.
//! Without admitted holding facts the run hard-fails even if Reth is up.
//! Collect-then-report (external multi-run) is the operator path for success.

#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use tower::ServiceExt;

mod support;

const NETWORK_ID: &str = "reth-local";
const SYMBOL_ID: &str = "eth.native.reth-local";
const DEFAULT_PARITY_RETH_HTTP_PORT: &str = "8565";

fn canonical_portfolio_snapshot_payload(wallet_address: &str, chain_id: u64) -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "reth-eth-only",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": NETWORK_ID,
                    "family": "evm",
                    "chain_id": chain_id,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_mainnet",
                    "subject": {
                        "kind": "evm_address",
                        "address": wallet_address
                    },
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": NETWORK_ID,
                    "symbol_ids": [
                        SYMBOL_ID
                    ],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": SYMBOL_ID,
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
                                "priced_symbol_id": SYMBOL_ID,
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1800.00"
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

fn parse_u64_hex(s: &str) -> u64 {
    let Some(rest) = s.strip_prefix("0x") else {
        panic!("missing 0x prefix: {s}");
    };
    if rest.is_empty() || rest.len() > 16 {
        panic!("invalid hex: {s}");
    }
    u64::from_str_radix(rest, 16).expect("hex u64")
}

fn required_rpc_url() -> String {
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

/// Report-only cutover: even with live Reth + runtime config, portfolio_snapshot
/// must not complete without admitted Platform holding facts. Live crawl is not
/// report authority.
#[tokio::test]
async fn parity_portfolio_snapshot_hard_fails_without_platform_holding_facts() {
    let rpc_url = required_rpc_url();

    let chain_id_hex = rpc_call(&rpc_url, "eth_chainId", serde_json::json!([])).await;
    let chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");
    let _runtime_config = set_runtime_source_registry(&rpc_url);

    let wallet_address = "0x000000000000000000000000000000000000dead";

    // support::run_portfolio_snapshot asserts Completed — call REST helper path instead.
    let app = mfm_rest_api::make_app(support::in_memory_rest_app_state());
    let response = app
        .oneshot(support::json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": canonical_portfolio_snapshot_payload(wallet_address, chain_id),
            }),
        ))
        .await
        .expect("portfolio rest response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = support::response_json(response).await;
    assert_eq!(body["status"], "success");
    let run_mode = body["data"]["run"]["run_mode"].as_str().expect("run_mode");
    assert_ne!(
        run_mode, "completed",
        "live Reth must not make report succeed without Platform holding facts"
    );
    assert!(
        body["data"].get("public_output").is_none() || body["data"]["public_output"].is_null(),
        "no public snapshot without admitted holding facts"
    );
}

fn set_runtime_source_registry(rpc_url: &str) -> support::EnvVarRestore {
    support::set_evm_runtime_config_env_for_test(NETWORK_ID, rpc_url)
}
