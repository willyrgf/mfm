#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use mfm_app::RunModeStatus;

mod support;

const NETWORK_ID: &str = "reth-local";
const SYMBOL_ID: &str = "eth.native.reth-local";
const CONTROL_SCOPE: &str = "parity/portfolio-snapshot/eth-only";
const DEFAULT_PARITY_RETH_HTTP_PORT: &str = "8565";

fn canonical_portfolio_snapshot_payload(
    wallet_address: &str,
    chain_id: u64,
    control_scope: &str,
) -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "reth-eth-only",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": NETWORK_ID,
                    "family": "evm",
                    "chain_id": chain_id,
                    "control_scope": control_scope,
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

fn find_quote_total<'a>(totals: &'a serde_json::Value, quote: &str) -> &'a serde_json::Value {
    totals
        .as_array()
        .expect("quote totals array")
        .iter()
        .find(|total| total["quote"] == quote)
        .unwrap_or_else(|| panic!("missing quote total for {quote}"))
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

#[tokio::test]
async fn parity_portfolio_snapshot_feature_against_reth_eth_only() {
    let rpc_url = required_rpc_url();
    let control_scope = format!("{CONTROL_SCOPE}.{}", uuid::Uuid::new_v4().simple());

    let chain_id_hex = rpc_call(&rpc_url, "eth_chainId", serde_json::json!([])).await;
    let chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");
    let _runtime_config = set_runtime_source_registry(&rpc_url);

    // Intentionally use a fixed address. This keeps the test independent of `eth_accounts`
    // support/configuration in the node.
    let wallet_address = "0x000000000000000000000000000000000000dead";

    let response = support::run_portfolio_snapshot(canonical_portfolio_snapshot_payload(
        wallet_address,
        chain_id,
        &control_scope,
    ))
    .await;
    assert_eq!(response.run.run_mode, RunModeStatus::Completed);
    let public_output = response
        .public_output
        .json
        .as_ref()
        .expect("public output json");
    let report = &public_output["report"];
    assert_eq!(report["portfolio_id"], "reth-eth-only");
    assert_eq!(report["error_count"], 0);
    let report_wallet = &report["wallet_summaries"][0];
    let report_wallet_usd = find_quote_total(&report_wallet["totals_by_quote"], "USD");
    let report_portfolio_usd = find_quote_total(&report["totals_by_quote"], "USD");
    assert_eq!(report_wallet_usd["collateral_value_dec"], "0");
    assert_eq!(report_wallet_usd["debt_value_dec"], "0");
    assert_eq!(report_wallet_usd["staked_value_dec"], "0");
    assert_eq!(report_wallet_usd, report_portfolio_usd);

    let out = &public_output["snapshot"];
    assert_eq!(out["portfolio_id"], "reth-eth-only");
    assert_eq!(out["network_pins"][0]["anchor"]["chain_id"], chain_id);
    assert!(out["network_pins"][0]["anchor"]["block_hash"]
        .as_str()
        .is_some_and(|value| value.starts_with("0x") && value.len() == 66));
    assert_eq!(out["wallets"][0]["address"], wallet_address);
    assert_eq!(
        out["wallets"][0]["observations"]
            .as_array()
            .map(|v: &Vec<serde_json::Value>| v.len()),
        Some(1)
    );
    assert_eq!(out["wallets"][0]["observations"][0]["symbol_id"], SYMBOL_ID);
    assert_eq!(
        out["wallets"][0]["observations"][0]["values"][0]["quote"],
        "USD"
    );
    assert_eq!(
        out["wallets"][0]["observations"][0]["values"][0]["unit_price_dec"],
        "1800.00"
    );
    let observation_value = &out["wallets"][0]["observations"][0]["values"][0]["value_dec"];
    assert_eq!(report_wallet_usd["assets_value_dec"], *observation_value);
    assert_eq!(report_wallet_usd["net_value_dec"], *observation_value);
}

fn set_runtime_source_registry(rpc_url: &str) -> support::EnvVarRestore {
    support::set_evm_runtime_config_env_for_test(NETWORK_ID, rpc_url)
}
