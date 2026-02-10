use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::json;

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory};
use mfm_event_store_postgres::PostgresEventStore;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError, RunError, StorageError};
use mfm_machine::events::EventEnvelope;
use mfm_machine::exec_transport::ExecProgramTransportFactory;
use mfm_machine::ids::{ArtifactId, ContextKey, ErrorCode, RunId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::runtime::DefaultExecutionEngine;
use mfm_machine::stores::{ArtifactStore, EventStore};
use mfm_op_evm_read::EvmReadOp;
use mfm_op_nix_app::NixAppOp;
use mfm_op_proof::ProofOp;
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::op::OperationRegistry;
use mfm_sdk::pipeline::PipelinePlanner;
use mfm_sdk::unstable::{
    single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
    SdkPlanResolver,
};

const ENV_EVM_RPC_URL: &str = "MFM_EVM_RPC_URL";
const ENV_EVM_RPC_AUTHORIZATION: &str = "MFM_EVM_RPC_AUTHORIZATION";

const ENV_ARTIFACT_ROOT: &str = "MFM_ARTIFACT_ROOT";
const ENV_DATABASE_URL: &str = "DATABASE_URL";

fn ok(data: serde_json::Value) -> serde_json::Value {
    json!({ "status": "success", "data": data })
}

fn err(code: impl Into<String>, message: impl Into<String>) -> serde_json::Value {
    json!({
        "status": "error",
        "error": {
            "code": code.into(),
            "message": message.into(),
        }
    })
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid_json() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "InvalidJson",
            "Failed to parse request body as JSON",
        )
    }

    pub fn invalid_uuid() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "InvalidUuid",
            "Invalid UUID format",
        )
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(err(self.code, self.message))).into_response()
    }
}

fn info(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

fn api_error_from_storage_error(err: StorageError) -> ApiError {
    match err {
        StorageError::Concurrency(info) => {
            ApiError::new(StatusCode::CONFLICT, info.code.0, info.message)
        }
        StorageError::NotFound(info) => {
            ApiError::new(StatusCode::NOT_FOUND, info.code.0, info.message)
        }
        StorageError::Corruption(info) | StorageError::Other(info) => {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, info.code.0, info.message)
        }
    }
}

fn api_error_from_run_error(err: RunError) -> ApiError {
    let info = match err {
        RunError::InvalidPlan(info) => info,
        RunError::Storage(se) => match se {
            StorageError::Concurrency(info)
            | StorageError::NotFound(info)
            | StorageError::Corruption(info)
            | StorageError::Other(info) => info,
        },
        RunError::Context(ce) => match ce {
            mfm_machine::errors::ContextError::MissingKey { info, .. }
            | mfm_machine::errors::ContextError::Serialization(info)
            | mfm_machine::errors::ContextError::Other(info) => info,
        },
        RunError::Io(ie) => match ie {
            IoError::MissingFactKey(info)
            | IoError::MissingFact { info, .. }
            | IoError::Transport(info)
            | IoError::RateLimited(info)
            | IoError::Other(info) => info,
        },
        RunError::State(se) => se.info,
        RunError::Other(info) => info,
    };

    let status = match info.category {
        ErrorCategory::ParsingInput => StatusCode::BAD_REQUEST,
        ErrorCategory::OnChain | ErrorCategory::OffChain | ErrorCategory::Rpc => {
            StatusCode::BAD_GATEWAY
        }
        ErrorCategory::Storage | ErrorCategory::Context | ErrorCategory::Unknown => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };

    ApiError::new(status, info.code.0, info.message)
}

fn default_artifact_root() -> PathBuf {
    std::env::var(ENV_ARTIFACT_ROOT)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("run_artifacts")
        })
}

pub fn make_default_artifact_store() -> Arc<dyn ArtifactStore> {
    Arc::new(FsArtifactStore::new(default_artifact_root()))
}

pub async fn make_default_event_store() -> Result<Arc<dyn EventStore>, ApiError> {
    let database_url = std::env::var(ENV_DATABASE_URL).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "MissingDatabaseUrl",
            format!("Missing {ENV_DATABASE_URL}"),
        )
    })?;

    let store = PostgresEventStore::connect(&database_url)
        .await
        .map_err(api_error_from_storage_error)?;

    Ok(Arc::new(store))
}

#[derive(Default)]
struct MapContext {
    inner: HashMap<String, serde_json::Value>,
}

impl DynContext for MapContext {
    fn read(
        &self,
        key: &ContextKey,
    ) -> Result<Option<serde_json::Value>, mfm_machine::errors::ContextError> {
        Ok(self.inner.get(&key.0).cloned())
    }

    fn write(
        &mut self,
        key: ContextKey,
        value: serde_json::Value,
    ) -> Result<(), mfm_machine::errors::ContextError> {
        self.inner.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), mfm_machine::errors::ContextError> {
        self.inner.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, mfm_machine::errors::ContextError> {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.inner {
            m.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(m))
    }
}

fn default_run_config() -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff: BackoffPolicy::Fixed {
                delay: Duration::from_millis(0),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
    }
}

fn phase_str(p: &RunPhase) -> &'static str {
    match p {
        RunPhase::Running => "running",
        RunPhase::Completed => "completed",
        RunPhase::Failed => "failed",
        RunPhase::Cancelled => "cancelled",
    }
}

struct RestApiLiveIoTransportFactory;

impl LiveIoTransportFactory for RestApiLiveIoTransportFactory {
    fn make(&self) -> Box<dyn LiveIoTransport> {
        Box::new(RestApiLiveIoTransport)
    }
}

struct RestApiLiveIoTransport;

#[async_trait]
impl LiveIoTransport for RestApiLiveIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            "proof.read" => Ok(serde_json::json!({ "n": 1 })),
            "proof.side_effect" => {
                let k = call
                    .request
                    .get("idempotency_key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("missing");
                let prefix: String = k.chars().take(8).collect();
                Ok(serde_json::json!({ "tx_hash": format!("0x{prefix}") }))
            }
            "proof.output" => Ok(call.request),
            other => Err(IoError::Other(info(
                "unknown_namespace",
                ErrorCategory::Unknown,
                format!("unknown namespace: {other}"),
            ))),
        }
    }
}

#[derive(Clone)]
pub struct EngineBundle {
    pub engine: Arc<dyn ExecutionEngine>,
    pub registry: Arc<dyn OperationRegistry>,
    pub planner: Arc<dyn PipelinePlanner>,
}

pub fn make_engine_bundle() -> EngineBundle {
    let mut reg = HashMapOperationRegistry::default();
    reg.register(Arc::new(ProofOp::default()));
    reg.register(Arc::new(EvmReadOp));
    reg.register(Arc::new(NixAppOp));
    let registry: Arc<dyn OperationRegistry> = Arc::new(reg);

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let resolver = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));

    let rpc_url = std::env::var(ENV_EVM_RPC_URL).ok();
    let authorization = std::env::var(ENV_EVM_RPC_AUTHORIZATION).ok();
    let evm_factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(EvmJsonRpcHttpTransportFactory::new(EvmJsonRpcHttpConfig {
            rpc_url,
            authorization,
            ..EvmJsonRpcHttpConfig::default()
        }));

    let mut routes: std::collections::HashMap<String, Arc<dyn LiveIoTransportFactory>> =
        std::collections::HashMap::new();
    routes.insert("proof".to_string(), Arc::new(RestApiLiveIoTransportFactory));
    routes.insert(
        "exec".to_string(),
        Arc::new(ExecProgramTransportFactory::default()),
    );
    routes.insert("evm".to_string(), evm_factory);

    let factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(RouterLiveIoTransportFactory::new(routes));
    let engine: Arc<dyn ExecutionEngine> =
        Arc::new(DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory));

    EngineBundle {
        engine,
        registry,
        planner,
    }
}

#[derive(Clone)]
pub struct AppState {
    pub bundle: EngineBundle,
    pub events: Arc<dyn EventStore>,
    pub artifacts: Arc<dyn ArtifactStore>,
}

pub fn make_app(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/runs/start", post(runs_start))
        .route("/v1/runs/:run_id/resume", post(runs_resume))
        .route("/v1/runs/:run_id/status", get(runs_status))
        .route("/v1/runs/:run_id/events", get(runs_events))
        .route("/v1/artifacts/:artifact_id", get(artifacts_get))
        .fallback(not_found)
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(ok(json!({ "ok": true })))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(err("not_found", "not found")))
}

#[derive(Clone, Debug, Deserialize)]
pub struct RunsStartRequest {
    #[serde(default = "default_op_id")]
    pub op_id: String,

    #[serde(default = "default_op_version")]
    pub op_version: String,

    #[serde(default = "default_empty_object")]
    pub op_config: serde_json::Value,
}

fn default_op_id() -> String {
    "proof".to_string()
}

fn default_op_version() -> String {
    "v1".to_string()
}

fn default_empty_object() -> serde_json::Value {
    json!({})
}

#[derive(Clone, Debug, Serialize)]
pub struct RunStartResponse {
    pub run_id: String,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

async fn runs_start(
    State(state): State<AppState>,
    body: Result<Json<RunsStartRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;

    let pipeline = single_op_pipeline(
        mfm_machine::ids::OpId(req.op_id),
        req.op_version,
        req.op_config,
    )
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e.info.code.0, e.info.message))?;

    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            Arc::clone(&state.bundle.engine),
            Stores {
                events: Arc::clone(&state.events),
                artifacts: Arc::clone(&state.artifacts),
            },
            Arc::clone(&state.bundle.registry),
            Arc::clone(&state.bundle.planner),
            LaunchPipeline {
                pipeline,
                input: serde_json::json!({}),
                run_config: default_run_config(),
                build: BuildProvenance {
                    git_commit: None,
                    cargo_lock_hash: None,
                    flake_lock_hash: None,
                    rustc_version: None,
                    target_triple: None,
                    env_allowlist: Vec::new(),
                },
                initial_context: Box::new(MapContext::default()),
            },
        )
        .await
        .map_err(api_error_from_run_error)?;

    Ok(Json(ok(serde_json::to_value(RunStartResponse {
        run_id: run.run_id.0.to_string(),
        phase: phase_str(&run.phase).to_string(),
        final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
    })
    .expect("run start response must serialize"))))
}

#[derive(Clone, Debug, Serialize)]
pub struct RunResumeResponse {
    pub run_id: String,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

async fn runs_resume(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = uuid::Uuid::parse_str(&run_id).map_err(|_| ApiError::invalid_uuid())?;
    let run_id = RunId(uuid);

    let launcher = DefaultRunLauncher;
    let run = launcher
        .resume(
            Arc::clone(&state.bundle.engine),
            Stores {
                events: Arc::clone(&state.events),
                artifacts: Arc::clone(&state.artifacts),
            },
            Arc::clone(&state.bundle.registry),
            Arc::clone(&state.bundle.planner),
            run_id,
        )
        .await
        .map_err(api_error_from_run_error)?;

    Ok(Json(ok(serde_json::to_value(RunResumeResponse {
        run_id: run.run_id.0.to_string(),
        phase: phase_str(&run.phase).to_string(),
        final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
    })
    .expect("run resume response must serialize"))))
}

#[derive(Clone, Debug, Serialize)]
pub struct RunStatusResponse {
    pub run_id: String,
    pub head_seq: u64,
    pub op_id: Option<String>,
    pub manifest_id: Option<String>,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

async fn runs_status(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = uuid::Uuid::parse_str(&run_id).map_err(|_| ApiError::invalid_uuid())?;
    let run_id = RunId(uuid);

    let head = state
        .events
        .head_seq(run_id)
        .await
        .map_err(api_error_from_storage_error)?;
    if head == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "run_not_found",
            "run event stream was not found",
        ));
    }

    let stream = state
        .events
        .read_range(run_id, 1, None)
        .await
        .map_err(api_error_from_storage_error)?;

    let mut op_id = None;
    let mut manifest_id = None;
    let mut completed: Option<(mfm_machine::events::RunStatus, Option<String>)> = None;

    for e in &stream {
        let mfm_machine::events::Event::Kernel(ke) = &e.event else {
            continue;
        };
        match ke {
            mfm_machine::events::KernelEvent::RunStarted {
                op_id: oid,
                manifest_id: mid,
                initial_snapshot_id: _,
            } => {
                op_id = Some(oid.0.clone());
                manifest_id = Some(mid.0.clone());
            }
            mfm_machine::events::KernelEvent::RunCompleted {
                status,
                final_snapshot_id,
            } => {
                completed = Some((
                    status.clone(),
                    final_snapshot_id.as_ref().map(|id| id.0.clone()),
                ));
            }
            _ => {}
        }
    }

    let (phase, final_snapshot_id) = match completed {
        Some((s, id)) => (
            match s {
                mfm_machine::events::RunStatus::Completed => "completed".to_string(),
                mfm_machine::events::RunStatus::Failed => "failed".to_string(),
                mfm_machine::events::RunStatus::Cancelled => "cancelled".to_string(),
            },
            id,
        ),
        None => ("running".to_string(), None),
    };

    Ok(Json(ok(serde_json::to_value(RunStatusResponse {
        run_id: run_id.0.to_string(),
        head_seq: head,
        op_id,
        manifest_id,
        phase,
        final_snapshot_id,
    })
    .expect("run status response must serialize"))))
}

fn default_from_seq() -> u64 {
    1
}

#[derive(Clone, Debug, Deserialize)]
pub struct RunsEventsQuery {
    #[serde(default = "default_from_seq")]
    pub from_seq: u64,

    pub to_seq: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunsEventsResponse {
    pub run_id: String,
    pub head_seq: u64,
    pub events: Vec<EventEnvelope>,
}

async fn runs_events(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Query(q): Query<RunsEventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let uuid = uuid::Uuid::parse_str(&run_id).map_err(|_| ApiError::invalid_uuid())?;
    let run_id = RunId(uuid);

    let head = state
        .events
        .head_seq(run_id)
        .await
        .map_err(api_error_from_storage_error)?;
    if head == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "run_not_found",
            "run event stream was not found",
        ));
    }

    let events = state
        .events
        .read_range(run_id, q.from_seq, q.to_seq)
        .await
        .map_err(api_error_from_storage_error)?;

    Ok(Json(ok(serde_json::to_value(RunsEventsResponse {
        run_id: run_id.0.to_string(),
        head_seq: head,
        events,
    })
    .expect("events response must serialize"))))
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactGetResponse {
    pub artifact_id: String,
    #[serde(flatten)]
    pub body: ArtifactBody,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "encoding", rename_all = "lowercase")]
pub enum ArtifactBody {
    Json { value: serde_json::Value },
    Hex { hex: String },
}

async fn artifacts_get(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = ArtifactId(artifact_id);

    let bytes = state
        .artifacts
        .get(&id)
        .await
        .map_err(api_error_from_storage_error)?;

    let body = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => ArtifactBody::Json { value },
        Err(_) => ArtifactBody::Hex {
            hex: hex::encode(bytes),
        },
    };

    Ok(Json(ok(serde_json::to_value(ArtifactGetResponse {
        artifact_id: id.0,
        body,
    })
    .expect("artifact response must serialize"))))
}
