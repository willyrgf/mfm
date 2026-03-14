#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{
    EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory, EvmJsonRpcSource, EvmSourceKind,
};
use mfm_machine::engine::Stores;
use mfm_machine::ids::{RunId, StateId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransportFactory};
use mfm_machine::stores::{ArtifactStore, StreamStore};
use mfm_stream_store_mem::MemStreamStore;

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let s = serde_json::to_string(&body).expect("json request must serialize");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(s))
        .expect("request")
}

fn canonical_portfolio_snapshot_payload(wallet_address: &str, chain_id: u64) -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "reth-eth-only",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "chain_id": chain_id,
                    "rpc_source_id": null,
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

async fn rpc_call(rpc_url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let streams: Arc<dyn StreamStore> = Arc::new(MemStreamStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(tmp.path()));

    let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
        sources: vec![EvmJsonRpcSource {
            id: "helper_primary".to_string(),
            rpc_url: rpc_url.to_string(),
            authorization: None,
            kind: EvmSourceKind::RemoteUser,
            require_get_proof_probe: false,
        }],
        preferred_order: vec!["helper_primary".to_string()],
        ..EvmJsonRpcHttpConfig::default()
    });
    let mut t = factory.make(LiveIoEnv {
        stores: Stores { streams, artifacts },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId::must_new("parity.main.rpc".to_string()),
        attempt: 0,
    });

    let v = t
        .call(IoCall {
            namespace: "evm".to_string(),
            request: serde_json::json!({
                "method": method,
                "params": params,
            }),
            fact_key: None,
        })
        .await
        .expect("rpc call");

    v
}

#[tokio::test]
async fn parity_portfolio_snapshot_feature_against_reth_eth_only() {
    let rpc_url = std::env::var("MFM_EVM_RPC_URL").expect("MFM_EVM_RPC_URL is required");

    let chain_id_hex = rpc_call(&rpc_url, "eth_chainId", serde_json::json!([])).await;
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
            serde_json::json!({
                "payload": canonical_portfolio_snapshot_payload(wallet_address, chain_id)
            }),
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
    assert_eq!(out["network_pins"][0]["chain_id"], chain_id);
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
