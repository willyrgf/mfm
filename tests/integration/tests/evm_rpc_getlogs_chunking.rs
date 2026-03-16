#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_collectors_rpc_control::{rpc_control_io_call, JsonRpcCall, RpcControlRequest};
use mfm_integration_tests::rpc_control;
use mfm_machine::engine::Stores;
use mfm_machine::errors::IoError;
use mfm_machine::ids::{FactKey, RunId, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::live_io::{
    FactIndex, LiveIo, LiveIoEnv, LiveIoTransportFactory, NoopFactRecorder,
};
use mfm_machine::replay_io::ReplayIo;
use mfm_machine::stores::{ArtifactStore, StreamStore};
use mfm_stream_store_mem::MemStreamStore;
use mfm_transports_rpc_control::{
    RpcControlExecutorTuning, RpcControlPlaneStorageMode, RpcControlTransportFactory,
};

const NETWORK_ID: &str = "ethereum-mainnet";

#[derive(Clone)]
enum StubBehavior {
    AlwaysStatus(u16),
    LogsSpanGate { max_ok_span: u64, fail_status: u16 },
}

#[derive(Clone)]
struct StubState {
    behavior: StubBehavior,
    hits: Arc<AtomicUsize>,
}

struct StubServer {
    url: String,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl StubServer {
    fn hit_count(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn parse_hex_u64(raw: &str) -> Option<u64> {
    let trimmed = raw.strip_prefix("0x")?;
    if trimmed.is_empty() {
        return None;
    }
    u64::from_str_radix(trimmed, 16).ok()
}

fn parse_logs_span(request: &Value) -> Option<u64> {
    let method = request.get("method")?.as_str()?;
    if !method.eq_ignore_ascii_case("eth_getLogs") {
        return None;
    }

    let filter = request.get("params")?.as_array()?.first()?.as_object()?;

    let from = filter.get("fromBlock")?.as_str().and_then(parse_hex_u64)?;
    let to = filter.get("toBlock")?.as_str().and_then(parse_hex_u64)?;
    if from > to {
        return None;
    }
    Some(to.saturating_sub(from).saturating_add(1))
}

fn logs_from_block(request: &Value) -> String {
    request
        .get("params")
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
        .and_then(|v| v.get("fromBlock"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| "0x0".to_string())
}

async fn stub_handler(
    State(state): State<StubState>,
    Json(request): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.hits.fetch_add(1, Ordering::SeqCst);
    let id = request.get("id").cloned().unwrap_or_else(|| json!(1));
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .map(str::to_ascii_lowercase);

    match state.behavior {
        StubBehavior::AlwaysStatus(code) => (
            StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32000, "message": "always fail"},
            })),
        ),
        StubBehavior::LogsSpanGate {
            max_ok_span,
            fail_status,
        } => {
            let span = parse_logs_span(&request);
            if span.is_some_and(|s| s > max_ok_span) {
                (
                    StatusCode::from_u16(fail_status).unwrap_or(StatusCode::TOO_MANY_REQUESTS),
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32000, "message": "range too large"},
                    })),
                )
            } else {
                let result = match method.as_deref() {
                    Some("eth_getlogs") => json!([{
                        "blockNumber": logs_from_block(&request),
                        "logIndex": "0x0",
                        "transactionHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
                    }]),
                    Some("eth_chainid") => json!("0x1"),
                    Some("eth_blocknumber") => json!("0x400"),
                    _ => json!("0x1"),
                };
                (
                    StatusCode::OK,
                    Json(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result,
                    })),
                )
            }
        }
    }
}

async fn start_stub_server(behavior: StubBehavior) -> StubServer {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/", post(stub_handler))
        .with_state(StubState {
            behavior,
            hits: Arc::clone(&hits),
        });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local addr");
    let task = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            panic!("stub server failed: {err}");
        }
    });

    StubServer {
        url: format!("http://{}", addr),
        hits,
        task,
    }
}

fn build_transport(
    primary: &StubServer,
    secondary: &StubServer,
    max_span: u64,
    min_span: u64,
    max_chunks: u64,
) -> Box<dyn mfm_machine::live_io::LiveIoTransport> {
    let sources = vec![
        rpc_control::single_remote_user_source("primary", NETWORK_ID, &primary.url),
        rpc_control::single_remote_user_source("secondary", NETWORK_ID, &secondary.url),
    ];

    let streams: Arc<dyn StreamStore> = Arc::new(MemStreamStore::new());
    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));

    RpcControlTransportFactory::new(sources)
        .with_control_plane_storage_mode(RpcControlPlaneStorageMode::StreamStore)
        .with_executor_tuning(RpcControlExecutorTuning::new(
            max_span, min_span, max_chunks,
        ))
        .make(LiveIoEnv {
            stores: Stores { streams, artifacts },
            run_id: RunId(uuid::Uuid::new_v4()),
            state_id: StateId::must_new("parity.evm_logs_chunking.transport".to_string()),
            attempt: 0,
        })
}

fn new_live_io(
    run_id: RunId,
    state_id: StateId,
    transport: Box<dyn mfm_machine::live_io::LiveIoTransport>,
    artifacts: Arc<dyn ArtifactStore>,
    facts: FactIndex,
) -> LiveIo {
    LiveIo::new(
        run_id,
        state_id,
        0,
        artifacts,
        facts,
        Arc::new(NoopFactRecorder),
        transport,
    )
}

#[tokio::test]
async fn evm_getlogs_chunking_succeeds_with_adaptive_split_and_failover() {
    let primary = start_stub_server(StubBehavior::AlwaysStatus(429)).await;
    let secondary = start_stub_server(StubBehavior::LogsSpanGate {
        max_ok_span: 32,
        fail_status: 429,
    })
    .await;

    let run_id = RunId(uuid::Uuid::new_v4());
    let state_id = StateId::must_new("parity.evm_logs_chunking.success".to_string());
    let facts = FactIndex::default();
    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));

    let mut live = new_live_io(
        run_id,
        state_id,
        build_transport(&primary, &secondary, 64, 8, 64),
        artifacts,
        facts,
    );

    let response = live
        .call(rpc_control_io_call(
            RpcControlRequest::EvmCall {
                call: JsonRpcCall::for_network(
                    NETWORK_ID,
                    "eth_getLogs",
                    serde_json::json!([{
                        "address": "0x0000000000000000000000000000000000000000",
                        "fromBlock": "0x1",
                        "toBlock": "0x80",
                        "topics": [],
                    }]),
                ),
            },
            FactKey("parity:evm_logs_chunking:success".to_string()),
        ))
        .await
        .expect("chunking should succeed");

    let logs = response.response.as_array().expect("array response");
    let from_blocks = logs
        .iter()
        .filter_map(|v| v.get("blockNumber"))
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        from_blocks,
        vec![
            "0x1".to_string(),
            "0x21".to_string(),
            "0x41".to_string(),
            "0x61".to_string(),
        ]
    );
}

#[tokio::test]
async fn evm_getlogs_chunking_returns_exhausted_when_retryable_failures_persist() {
    let primary = start_stub_server(StubBehavior::AlwaysStatus(429)).await;
    let secondary = start_stub_server(StubBehavior::LogsSpanGate {
        max_ok_span: 0,
        fail_status: 429,
    })
    .await;

    let run_id = RunId(uuid::Uuid::new_v4());
    let state_id = StateId::must_new("parity.evm_logs_chunking.exhausted".to_string());
    let facts = FactIndex::default();
    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));

    let mut live = new_live_io(
        run_id,
        state_id,
        build_transport(&primary, &secondary, 16, 8, 16),
        artifacts,
        facts,
    );

    let err = live
        .call(rpc_control_io_call(
            RpcControlRequest::EvmCall {
                call: JsonRpcCall::for_network(
                    NETWORK_ID,
                    "eth_getLogs",
                    serde_json::json!([{
                        "address": "0x0000000000000000000000000000000000000000",
                        "fromBlock": "0x1",
                        "toBlock": "0x40",
                        "topics": [],
                    }]),
                ),
            },
            FactKey("parity:evm_logs_chunking:exhausted".to_string()),
        ))
        .await
        .expect_err("chunking should exhaust");

    match err {
        IoError::Transport(info) => assert_eq!(info.code.0, "evm_logs_chunking_exhausted"),
        other => panic!("expected Transport, got {other:?}"),
    }
}

#[tokio::test]
async fn evm_getlogs_chunking_live_then_replay_is_network_quiet() {
    let primary = start_stub_server(StubBehavior::AlwaysStatus(429)).await;
    let secondary = start_stub_server(StubBehavior::LogsSpanGate {
        max_ok_span: 32,
        fail_status: 429,
    })
    .await;

    let run_id = RunId(uuid::Uuid::new_v4());
    let state_id = StateId::must_new("parity.evm_logs_chunking.replay".to_string());
    let fact_key = FactKey("parity:evm_logs_chunking:replay".to_string());
    let facts = FactIndex::default();

    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));

    let mut live = new_live_io(
        run_id,
        state_id.clone(),
        build_transport(&primary, &secondary, 64, 8, 64),
        Arc::clone(&artifacts),
        facts.clone(),
    );
    let request = json!({
        "method": "eth_getLogs",
        "params": [{
            "address": "0x0000000000000000000000000000000000000000",
            "fromBlock": "0x1",
            "toBlock": "0x80",
            "topics": [],
        }],
    });

    let live_result = live
        .call(rpc_control_io_call(
            RpcControlRequest::EvmCall {
                call: JsonRpcCall::for_network(
                    NETWORK_ID,
                    "eth_getLogs",
                    serde_json::Value::Array(
                        request["params"].as_array().cloned().unwrap_or_default(),
                    ),
                ),
            },
            fact_key.clone(),
        ))
        .await
        .expect("live chunking call should succeed");
    let live_len = live_result
        .response
        .as_array()
        .map(|v| v.len())
        .expect("array response");
    assert_eq!(live_len, 4);

    let primary_hits_after_live = primary.hit_count();
    let secondary_hits_after_live = secondary.hit_count();

    let mut replay = ReplayIo::new(run_id, state_id, 0, artifacts, facts, false);
    let replay_result = replay
        .call(IoCall {
            namespace: "rpc.control".to_string(),
            request: serde_json::to_value(RpcControlRequest::EvmCall {
                call: JsonRpcCall::for_network(
                    NETWORK_ID,
                    "eth_getLogs",
                    serde_json::Value::Array(
                        request["params"].as_array().cloned().unwrap_or_default(),
                    ),
                ),
            })
            .expect("rpc.control request"),
            fact_key: Some(fact_key),
        })
        .await
        .expect("replay should reuse recorded fact payload");
    let replay_len = replay_result
        .response
        .as_array()
        .map(|v| v.len())
        .expect("array response");
    assert_eq!(replay_len, 4);

    assert_eq!(primary.hit_count(), primary_hits_after_live);
    assert_eq!(secondary.hit_count(), secondary_hits_after_live);
}
