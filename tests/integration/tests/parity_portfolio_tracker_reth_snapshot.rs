//! Portfolio + live Reth: report is not a crawler.
//!
//! After cutover, even with a live Reth node, `portfolio_snapshot` hard-fails without
//! admitted Platform holding facts. Collector↔Reth parity is future work; this parity
//! gate only locks "live RPC is not report authority."

#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use tower::ServiceExt;

mod support;
use support::{json_post, response_json};

const NETWORK_ID: &str = "reth-local";
const SYMBOL_ID: &str = "eth.native.reth-local";

#[tokio::test]
async fn parity_portfolio_snapshot_hard_fails_without_platform_holding_facts() {
    // Presence of Reth/env must not turn report into a live crawl.
    let _rpc = std::env::var("RETH_HTTP_PORT").ok();
    let app = mfm_rest_api::make_app(support::in_memory_rest_app_state());
    let response = app
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": {
                    "portfolio": {
                        "portfolio_id": "reth-eth-only",
                        "quote_codes": ["USD"],
                        "networks": [{
                            "network_id": NETWORK_ID,
                            "family": "evm",
                            "chain_id": 1,
                            "metadata": {}
                        }],
                        "wallets": [{
                            "wallet_id": "wallet_mainnet",
                            "subject": {
                                "kind": "evm_address",
                                "address": "0x0000000000000000000000000000000000000001"
                            },
                            "implementation": { "kind": "address_only" },
                            "network_id": NETWORK_ID,
                            "symbol_ids": [SYMBOL_ID],
                            "metadata": {}
                        }],
                        "symbol_configs": [{
                            "symbol_id": SYMBOL_ID,
                            "display_symbol": "ETH",
                            "kind": "native_balance",
                            "role": "native",
                            "network_id": NETWORK_ID,
                            "protocol": null,
                            "balance_reader": { "kind": "native_balance" },
                            "valuation": {
                                "quotes": [{
                                    "quote": "USD",
                                    "priced_symbol_id": SYMBOL_ID,
                                    "unit_price_dec": "1800.00"
                                }]
                            },
                            "underlying_symbol_id": null,
                            "metadata": {}
                        }],
                        "metadata": {}
                    }
                }
            }),
        ))
        .await
        .expect("start");
    let body = response_json(response).await;
    assert_eq!(body["status"], "success");
    let run_mode = body["data"]["run"]["run_mode"].as_str().expect("run_mode");
    assert_ne!(
        run_mode, "completed",
        "live Reth must not complete portfolio_snapshot without Platform holding facts"
    );
}
