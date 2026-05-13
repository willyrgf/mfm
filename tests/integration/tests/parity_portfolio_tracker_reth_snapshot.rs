#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_integration_tests::rpc_control;
use mfm_machine::stores::{ArtifactStore, StreamStore};
use mfm_stream_store_mem::MemStreamStore;
use mfm_transports_rpc_control::RpcControlBootstrapSource;

const NETWORK_ID: &str = "ethereum-mainnet";
const CONTROL_SCOPE: &str = "parity.portfolio_snapshot.eth_only";

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let s = serde_json::to_string(&body).expect("json request must serialize");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(s))
        .expect("request")
}

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
                    "network_id": "ethereum-mainnet",
                    "chain_id": chain_id,
                    "control_scope": control_scope,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_mainnet",
                    "address": wallet_address,
                    "implementation": {
                        "kind": "address_only"
                    },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": [
                        "eth.native.ethereum-mainnet"
                    ],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "native_balance"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
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

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
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

async fn rpc_call(
    rpc_sources: &[RpcControlBootstrapSource],
    control_scope: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(tmp.path()));
    let streams: Arc<dyn StreamStore> = Arc::new(MemStreamStore::new());
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

#[tokio::test]
async fn parity_portfolio_snapshot_feature_against_reth_eth_only() {
    let rpc_sources = rpc_control::required_bootstrap_sources_from_env_for_network(NETWORK_ID);
    let control_scope = format!("{CONTROL_SCOPE}.{}", uuid::Uuid::new_v4().simple());
    let bootstrap_control_scope = format!("{control_scope}.bootstrap");

    let chain_id_hex = rpc_call(
        &rpc_sources,
        &bootstrap_control_scope,
        "eth_chainId",
        serde_json::json!([]),
    )
    .await;
    let chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");

    let streams: Arc<dyn StreamStore> = Arc::new(MemStreamStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(tmp.path()));

    // Intentionally use a fixed address. This keeps the test independent of `eth_accounts`
    // support/configuration in the node.
    let wallet_address = "0x000000000000000000000000000000000000dead";

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        streams: Arc::clone(&streams),
        artifacts: Arc::clone(&artifacts),
    });

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/features/portfolio.snapshot/execute",
            canonical_portfolio_snapshot_payload(wallet_address, chain_id, &control_scope),
        ))
        .await
        .expect("feature execute response");

    assert_eq!(resp.status(), StatusCode::OK);

    let v = response_json(resp).await;
    assert_eq!(v["status"], "success");
    assert_eq!(v["data"]["feature_id"], "portfolio.snapshot");
    assert_eq!(v["data"]["result"]["phase"], "completed");
    assert_eq!(
        v["data"]["result"]["report"]["portfolio_id"],
        "reth-eth-only"
    );
    assert_eq!(v["data"]["result"]["report"]["error_count"], 0);
    let report_wallet = &v["data"]["result"]["report"]["wallet_summaries"][0];
    let report_wallet_usd = find_quote_total(&report_wallet["totals_by_quote"], "USD");
    let report_portfolio_usd =
        find_quote_total(&v["data"]["result"]["report"]["totals_by_quote"], "USD");
    assert_eq!(report_wallet_usd["collateral_value_dec"], "0");
    assert_eq!(report_wallet_usd["debt_value_dec"], "0");
    assert_eq!(report_wallet_usd["staked_value_dec"], "0");
    assert_eq!(report_wallet_usd, report_portfolio_usd);

    let snapshot_artifact_id = v["data"]["result"]["snapshot_artifact_id"]
        .as_str()
        .expect("snapshot_artifact_id")
        .to_string();

    let artifact_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/artifacts/{snapshot_artifact_id}"))
                .body(Body::empty())
                .expect("artifact request"),
        )
        .await
        .expect("artifact get response");

    assert_eq!(artifact_resp.status(), StatusCode::OK);
    let a = response_json(artifact_resp).await;

    assert_eq!(a["status"], "success");
    assert_eq!(a["data"]["artifact_id"], snapshot_artifact_id);
    assert_eq!(a["data"]["encoding"], "json");
    let out = a["data"]["value"].clone();
    assert_eq!(out["portfolio_id"], "reth-eth-only");
    assert_eq!(out["network_pins"][0]["anchor"]["chain_id"], chain_id);
    assert_eq!(out["wallets"][0]["address"], wallet_address);
    assert_eq!(
        out["wallets"][0]["observations"]
            .as_array()
            .map(|v| v.len()),
        Some(1)
    );
    assert_eq!(
        out["wallets"][0]["observations"][0]["symbol_id"],
        "eth.native.ethereum-mainnet"
    );
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
