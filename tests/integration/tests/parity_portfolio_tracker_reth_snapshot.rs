#![cfg(feature = "parity-tests")]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory};
use mfm_event_store_mem::MemEventStore;
use mfm_machine::engine::Stores;
use mfm_machine::ids::{RunId, StateId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransportFactory};
use mfm_machine::stores::{ArtifactStore, EventStore};

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
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(tmp.path()));

    let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
        rpc_url: Some(rpc_url.to_string()),
        authorization: None,
        ..EvmJsonRpcHttpConfig::default()
    });
    let mut t = factory.make(LiveIoEnv {
        stores: Stores { events, artifacts },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId("parity.main.rpc".to_string()),
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

    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(tmp.path()));

    // Intentionally use a fixed address. This keeps the test independent of `eth_accounts`
    // support/configuration in the node.
    let wallet_address = "0x000000000000000000000000000000000000dead";

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events: Arc::clone(&events),
        artifacts: Arc::clone(&artifacts),
    });

    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/features/portfolio.snapshot/execute",
            serde_json::json!({
                "payload": {
                    "address": wallet_address,
                    "chain_id": chain_id,
                    "tokens": [],
                }
            }),
        ))
        .await
        .expect("feature execute response");

    assert_eq!(resp.status(), StatusCode::OK);

    let v = response_json(resp).await;
    assert_eq!(v["status"], "success");
    assert_eq!(v["data"]["feature_id"], "portfolio.snapshot");
    assert_eq!(v["data"]["result"]["phase"], "completed");
    assert_eq!(v["data"]["result"]["chain_id"], chain_id);
    assert!(v["data"]["result"]["block_number"].as_u64().is_some());

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
    assert_eq!(
        a["data"]["value"]["wallet_address"].as_str(),
        Some(wallet_address)
    );
    assert_eq!(a["data"]["value"]["chain_id"].as_u64(), Some(chain_id));
    assert_eq!(
        a["data"]["value"]["tokens"].as_array().map(|a| a.len()),
        Some(0)
    );
}
