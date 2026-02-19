#![cfg(feature = "parity-tests")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{
    EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory, EvmJsonRpcSource, EvmRoutingStrategy,
    EvmSourceKind,
};
use mfm_event_store_mem::MemEventStore;
use mfm_machine::engine::Stores;
use mfm_machine::errors::IoError;
use mfm_machine::ids::{FactKey, RunId, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::live_io::{
    FactIndex, LiveIo, LiveIoEnv, LiveIoTransportFactory, NoopFactRecorder,
};
use mfm_machine::replay_io::ReplayIo;
use mfm_machine::stores::{ArtifactStore, EventStore};

#[derive(Clone)]
enum StubBehavior {
    JsonResult(Value),
    HttpStatus(u16),
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

async fn stub_handler(
    State(state): State<StubState>,
    Json(request): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.hits.fetch_add(1, Ordering::SeqCst);
    let id = request.get("id").cloned().unwrap_or_else(|| json!(1));

    match state.behavior {
        StubBehavior::JsonResult(ref result) => (
            StatusCode::OK,
            Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            })),
        ),
        StubBehavior::HttpStatus(code) => (
            StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32000,
                    "message": "stubbed http status",
                }
            })),
        ),
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

fn source(id: &str, rpc_url: &str) -> EvmJsonRpcSource {
    EvmJsonRpcSource {
        id: id.to_string(),
        rpc_url: rpc_url.to_string(),
        authorization: None,
        kind: EvmSourceKind::RemoteUser,
        require_get_proof_probe: false,
    }
}

fn build_transport(
    primary: &StubServer,
    secondary: &StubServer,
) -> Box<dyn mfm_machine::live_io::LiveIoTransport> {
    let factory = EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
        sources: vec![
            source("primary", &primary.url),
            source("secondary", &secondary.url),
        ],
        preferred_order: vec!["primary".to_string(), "secondary".to_string()],
        strategy: EvmRoutingStrategy::Failover,
        ..EvmJsonRpcHttpConfig::default()
    });

    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));

    factory.make(LiveIoEnv {
        stores: Stores { events, artifacts },
        run_id: RunId(uuid::Uuid::new_v4()),
        state_id: StateId("parity.evm_pool.transport".to_string()),
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
async fn evm_rpc_pool_failover_live_then_replay_keeps_network_quiet() {
    let primary = start_stub_server(StubBehavior::HttpStatus(500)).await;
    let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

    let run_id = RunId(uuid::Uuid::new_v4());
    let state_id = StateId("parity.evm_pool.failover".to_string());
    let fact_key = FactKey("parity:evm_pool:failover".to_string());
    let facts = FactIndex::default();

    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));

    let mut live = new_live_io(
        run_id,
        state_id.clone(),
        build_transport(&primary, &secondary),
        Arc::clone(&artifacts),
        facts.clone(),
    );
    let request = json!({
        "method": "eth_getLogs",
        "params": [],
    });

    let live_result = live
        .call(IoCall {
            namespace: "evm".to_string(),
            request: request.clone(),
            fact_key: Some(fact_key.clone()),
        })
        .await
        .expect("live call should fail over and succeed");
    assert_eq!(live_result.response, json!("0x2"));
    assert!(live_result.recorded_payload_id.is_some());

    let primary_hits_after_live = primary.hit_count();
    let secondary_hits_after_live = secondary.hit_count();
    assert!(primary_hits_after_live >= 1);
    assert!(secondary_hits_after_live >= 1);

    let mut replay = ReplayIo::new(run_id, state_id, 0, artifacts, facts, false);
    let replay_result = replay
        .call(IoCall {
            namespace: "evm".to_string(),
            request,
            fact_key: Some(fact_key),
        })
        .await
        .expect("replay should read recorded fact without network");
    assert_eq!(replay_result.response, json!("0x2"));

    assert_eq!(primary.hit_count(), primary_hits_after_live);
    assert_eq!(secondary.hit_count(), secondary_hits_after_live);
}

#[tokio::test]
async fn evm_rpc_pool_failover_uses_secondary_when_primary_is_429() {
    let primary = start_stub_server(StubBehavior::HttpStatus(429)).await;
    let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

    let run_id = RunId(uuid::Uuid::new_v4());
    let state_id = StateId("parity.evm_pool.rate_limit".to_string());
    let fact_key = FactKey("parity:evm_pool:rate_limit".to_string());
    let facts = FactIndex::default();

    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));

    let mut live = new_live_io(
        run_id,
        state_id,
        build_transport(&primary, &secondary),
        artifacts,
        facts,
    );

    let live_result = live
        .call(IoCall {
            namespace: "evm".to_string(),
            request: json!({
                "method": "eth_getLogs",
                "params": [],
            }),
            fact_key: Some(fact_key),
        })
        .await
        .expect("live call should fail over on 429");

    assert_eq!(live_result.response, json!("0x2"));
    assert!(primary.hit_count() >= 1);
    assert!(secondary.hit_count() >= 1);
}

#[tokio::test]
async fn evm_replay_missing_fact_key_behavior_is_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(temp.path()));
    let facts = FactIndex::default();
    let run_id = RunId(uuid::Uuid::new_v4());
    let state_id = StateId("parity.evm_pool.missing_fact_key".to_string());

    let mut replay = ReplayIo::new(run_id, state_id, 0, artifacts, facts, false);
    let err = replay
        .call(IoCall {
            namespace: "evm".to_string(),
            request: json!({
                "method": "eth_chainId",
                "params": [],
            }),
            fact_key: None,
        })
        .await
        .expect_err("replay without fact key should fail");

    match err {
        IoError::MissingFactKey(info) => assert_eq!(info.code.0, "missing_fact_key"),
        other => panic!("expected MissingFactKey, got {other:?}"),
    }
}
