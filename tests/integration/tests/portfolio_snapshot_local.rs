//! Portfolio snapshot integration after the fact-backed cutover.
//!
//! Report is Platform-fact-only: without admitted holding facts the run hard-fails.
//! Live chain RPC is not report authority (collect-then-report is external multi-run).

#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_app::PublicOpName;
use mfm_events::v1 as events;
use mfm_ids::RunId;
use mfm_store::v1::{self as store, RunEventStore};
use serde_json::json;
use tower::ServiceExt;

mod support;
use support::{empty_post, json_post, response_json};

const NETWORK_ID: &str = "typed-local-eth";

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
async fn rest_portfolio_snapshot_fails_closed_without_facts() {
    let app = rest_test_app();
    let response = app
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": portfolio_payload(),
            }),
        ))
        .await
        .expect("portfolio rest response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "success");
    let run_mode = body["data"]["run"]["run_mode"].as_str().expect("run_mode");
    assert_ne!(run_mode, "completed");
    assert!(body["data"]["run"]["run_id"].as_str().is_some());
}

#[tokio::test]
async fn rest_portfolio_snapshot_defaults_toml_fails_closed_without_facts() {
    let state = support::in_memory_rest_app_state();
    let store = state.store.clone();
    let app = mfm_rest_api::make_app(state);
    let response = app
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "op": "portfolio_snapshot",
                "config": portfolio_payload_toml(),
            }),
        ))
        .await
        .expect("portfolio toml rest response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "success");
    let run_mode = body["data"]["run"]["run_mode"].as_str().expect("run_mode");
    assert_ne!(run_mode, "completed");
    let run_id = RunId::parse(body["data"]["run"]["run_id"].as_str().expect("run id"))
        .expect("typed run id");
    let stream = store.load_run_stream(&run_id).await.expect("run stream");
    assert_portfolio_entry_point_evidence(&stream);
}

#[tokio::test]
async fn portfolio_select_centric_runner_summary_without_facts() {
    let state = support::in_memory_rest_app_state();
    let store = state.store.clone();
    let app = mfm_rest_api::make_app(state);
    let response = portfolio_snapshot_post(&app, &portfolio_payload()).await;
    let run_id = RunId::parse(response["data"]["run"]["run_id"].as_str().expect("run id"))
        .expect("typed run id");
    let stream = store.load_run_stream(&run_id).await.expect("run stream");
    assert_portfolio_entry_point_evidence(&stream);
    let actual = portfolio_runner_output_summary(&stream);
    assert!(
        actual
            .iter()
            .any(|line| line.contains("mfm.portfolio/resolve_subjects")),
        "expected resolve_subjects: {actual:?}"
    );
    assert!(
        actual.iter().all(|line| {
            !line.contains("pin_views")
                && !line.contains("observe_batch")
                && !line.contains("merge_observations")
        }),
        "live pin/observe/merge must not appear: {actual:?}"
    );
    assert!(
        actual.iter().all(|line| {
            !line.contains("assemble_snapshot") && !line.contains("project_report")
        }),
        "assemble/project must not complete without selected holdings: {actual:?}"
    );
}

#[tokio::test]
async fn failed_run_status_stream_and_replay_remain_readable() {
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

    let replay = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/replay")))
        .await
        .expect("replay response");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = response_json(replay).await;
    assert_eq!(replay_body["status"], "success");
    assert_eq!(replay_body["data"]["run_mode"], run_mode);
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
            "portfolio_id": "typed-local",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": NETWORK_ID,
                    "family": "evm",
                    "chain_id": 31337,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_local",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
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

fn portfolio_payload_toml() -> String {
    format!(
        r#"[portfolio]
portfolio_id = "typed-local"
quote_codes = ["USD"]

[portfolio.metadata]

[[portfolio.networks]]
network_id = "{NETWORK_ID}"
family = "evm"
chain_id = 31337

[portfolio.networks.metadata]

[[portfolio.wallets]]
wallet_id = "wallet_local"
network_id = "{NETWORK_ID}"
symbol_ids = ["eth.native.typed-local-eth"]

[portfolio.wallets.subject]
kind = "evm_address"
address = "0x000000000000000000000000000000000000dead"

[portfolio.wallets.implementation]
kind = "address_only"

[portfolio.wallets.metadata]

[[portfolio.symbol_configs]]
symbol_id = "eth.native.typed-local-eth"
display_symbol = "ETH"
kind = "native_balance"
role = "native"
network_id = "{NETWORK_ID}"
decimals = 18

[portfolio.symbol_configs.balance_reader]
kind = "native_balance"

[portfolio.symbol_configs.valuation]

[[portfolio.symbol_configs.valuation.quotes]]
quote = "USD"
priced_symbol_id = "eth.native.typed-local-eth"

[portfolio.symbol_configs.valuation.quotes.reader]
kind = "fixed_unit_price"
unit_price_dec = "2.50"

[portfolio.symbol_configs.metadata]

[valuation_source_registry]
sources = []
"#
    )
}

fn assert_portfolio_entry_point_evidence(stream: &[store::KernelEventEnvelope]) {
    let evidence = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(&payload.entry_point),
            _ => None,
        })
        .expect("RunAdmitted entry-point evidence");
    let registry = mfm_app::production_entry_point_op_registry().expect("entry-point registry");
    let public_name = PublicOpName::new("portfolio_snapshot").expect("public op name");
    let op = registry
        .resolve(&public_name, None)
        .expect("portfolio latest op");

    assert_eq!(evidence.resolved_op_id.as_str(), op.op_id().to_string());
    assert_eq!(
        evidence.entry_point_registry_digest,
        registry.registry_digest().expect("registry digest")
    );
}

fn portfolio_runner_output_summary(stream: &[store::KernelEventEnvelope]) -> Vec<String> {
    attempt_output_commit_summaries(stream, "mfm.portfolio/")
}

fn attempt_output_commit_summaries(
    stream: &[store::KernelEventEnvelope],
    state_kind_prefix: &str,
) -> Vec<String> {
    let state_kinds_by_node = state_kinds_by_node(stream);
    let mut summaries = Vec::new();
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == commit_key
        {
            end += 1;
        }
        if commit_key.as_str().starts_with("attempt-output:") {
            let state_kind = runner_output_node_id(&stream[index..end])
                .and_then(|node_id| state_kinds_by_node.get(node_id.as_str()))
                .map(String::as_str)
                .unwrap_or("unknown");
            if state_kind.starts_with(state_kind_prefix) {
                let payloads = stream[index..end]
                    .iter()
                    .map(|event| runner_payload_summary(event.payload()))
                    .collect::<Vec<_>>()
                    .join("+");
                summaries.push(format!(
                    "{}:{state_kind}:{payloads}",
                    commit_key_class(commit_key.as_str())
                ));
            }
        }
        index = end;
    }
    summaries
}

fn state_kinds_by_node(
    stream: &[store::KernelEventEnvelope],
) -> std::collections::BTreeMap<String, String> {
    stream
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => Some((
                payload.node_id.as_str().to_owned(),
                payload
                    .state_kind
                    .canonical_name()
                    .unwrap_or_else(|| payload.state_kind.as_str())
                    .to_owned(),
            )),
            _ => None,
        })
        .collect()
}

fn runner_output_node_id(events: &[store::KernelEventEnvelope]) -> Option<&mfm_ids::NodeId> {
    events.iter().find_map(|event| match event.payload() {
        events::KernelEventPayload::FactRecorded(payload) => Some(&payload.node_id),
        events::KernelEventPayload::CellProduced(payload) => Some(&payload.node_id),
        events::KernelEventPayload::CellSkipped(payload) => Some(&payload.node_id),
        events::KernelEventPayload::StateAttemptCompleted(payload) => Some(&payload.node_id),
        _ => None,
    })
}

fn runner_payload_summary(payload: &events::KernelEventPayload) -> String {
    match payload {
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            format!(
                "artifact_referenced[role={}]",
                payload.artifact_ref.role.as_str()
            )
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => {
            let roles = payload
                .refs
                .iter()
                .map(|retention| retention.role.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("retention_refs_appended[roles={roles}]")
        }
        _ => payload_schema_name(payload).to_owned(),
    }
}

fn payload_schema_name(payload: &events::KernelEventPayload) -> &str {
    payload
        .schema_descriptor()
        .schema_name
        .strip_prefix("mfm.events.v1.")
        .unwrap_or_else(|| payload.schema_descriptor().schema_name)
}

fn commit_key_class(commit_key: &str) -> &str {
    if commit_key.starts_with("attempt-output:") {
        "attempt-output"
    } else {
        commit_key
    }
}
