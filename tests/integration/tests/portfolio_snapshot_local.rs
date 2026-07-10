//! Portfolio snapshot fail-closed contract after the fact-backed cutover.
//!
//! Report is Platform-fact-only: without admitted holding facts the run hard-fails.
//! Success path lives in `portfolio_snapshot_from_admitted_facts` (seeded) and
//! `collect_then_report_native_balances` (collectors then report).

#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_ids::RunId;
use serde_json::json;
use tower::ServiceExt;

mod support;
use support::{json_post, response_json};

#[tokio::test]
async fn portfolio_snapshot_hard_fails_without_platform_holding_facts() {
    let app = rest_test_app();
    let body = portfolio_snapshot_post(&app, &portfolio_payload()).await;
    let run_mode = body["data"]["run"]["run_mode"].as_str().expect("run_mode");
    assert_ne!(
        run_mode, "completed",
        "report-only portfolio must not complete without admitted Platform holding facts"
    );
    assert!(
        body["data"].get("public_output").is_none() || body["data"]["public_output"].is_null(),
        "no public snapshot when required facts are missing"
    );
}

#[tokio::test]
async fn failed_report_status_and_stream_remain_readable() {
    // Replay of hard-failed report-only runs currently returns ReplayDiagnosticInvalid
    // (pre-existing residual). Status + stream cover operator observability after hard-fail.
    let app = rest_test_app();
    let response = portfolio_snapshot_post(&app, &portfolio_payload()).await;
    let run_id = response["data"]["run"]["run_id"]
        .as_str()
        .expect("run id")
        .to_owned();
    let run_mode = response["data"]["run"]["run_mode"]
        .as_str()
        .expect("run_mode")
        .to_owned();
    assert_ne!(run_mode, "completed");

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/status"))
                .body(Body::empty())
                .expect("status request"),
        )
        .await
        .expect("status response");
    assert_eq!(status.status(), StatusCode::OK);
    let status_body = response_json(status).await;
    assert_eq!(status_body["status"], "success");
    assert_eq!(status_body["data"]["run_mode"], run_mode);

    let stream = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/stream"))
                .body(Body::empty())
                .expect("stream request"),
        )
        .await
        .expect("stream response");
    assert_eq!(stream.status(), StatusCode::OK);
    let stream_body = response_json(stream).await;
    assert_eq!(stream_body["status"], "success");
    assert!(
        stream_body["data"]["events"]
            .as_array()
            .is_some_and(|events| !events.is_empty()),
        "{stream_body}"
    );

    // Typed run id remains parseable for operators after hard-fail.
    RunId::parse(&run_id).expect("typed run id");
}

async fn portfolio_snapshot_post(
    app: &axum::Router,
    payload: &serde_json::Value,
) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": payload,
            }),
        ))
        .await
        .expect("portfolio snapshot response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "success");
    body
}

fn rest_test_app() -> axum::Router {
    mfm_rest_api::make_app(support::in_memory_rest_app_state())
}

fn portfolio_payload() -> serde_json::Value {
    json!({
        "portfolio": {
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "typed-local-eth",
                    "family": "evm",
                    "chain_id": 1,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "implementation": { "kind": "address_only" },
                    "network_id": "typed-local-eth",
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
                    "network_id": "typed-local-eth",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.typed-local-eth",
                                "unit_price_dec": "1800.00"
                            }
                        ]
                    },
                        "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }
    })
}
