use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory};
use mfm_event_store_postgres::PostgresEventStore;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
use mfm_machine::errors::{
    ContextError, ErrorCategory, ErrorInfo, IoError, RunError, StorageError,
};
use mfm_machine::events::{Event, EventEnvelope, KernelEvent, RunStatus};
use mfm_machine::exec_transport::ExecProgramTransportFactory;
use mfm_machine::ids::{ArtifactId, ContextKey, ErrorCode, OpId, RunId};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::runtime::{ChildRunLiveIoTransportFactory, DefaultExecutionEngine, PlanResolver};
use mfm_machine::stores::{ArtifactStore, EventStore};
use mfm_op_evm_read::EvmReadOp;
use mfm_op_evm_write::{EvmConfigureOp, EvmContractFromNixOp, EvmDeployOp, EvmValidateOp};
use mfm_op_nix_app::nix_exec_transport::NixFlakeTransportFactory;
use mfm_op_nix_app::NixAppOp;
use mfm_op_proof::ProofOp;
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::op::OperationRegistry;
use mfm_sdk::pipeline::{Pipeline, PipelinePlanner, PipelineStep};
use mfm_sdk::unstable::{
    single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
    SdkPlanResolver,
};

const ENV_EVM_RPC_URL: &str = "MFM_EVM_RPC_URL";
const ENV_EVM_RPC_AUTHORIZATION: &str = "MFM_EVM_RPC_AUTHORIZATION";

const ENV_ARTIFACT_BACKEND: &str = "MFM_ARTIFACT_BACKEND";
const ENV_ARTIFACT_ROOT: &str = "MFM_ARTIFACT_ROOT";
const ENV_DATABASE_URL: &str = "DATABASE_URL";
const ENV_S3_ENSURE_BUCKET: &str = "MFM_S3_ENSURE_BUCKET";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    BadRequest,
    NotFound,
    Conflict,
    BadGateway,
    Internal,
}

#[derive(Debug, Clone)]
pub struct AppError {
    pub class: ErrorClass,
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid_json() -> Self {
        Self::new(
            ErrorClass::BadRequest,
            "InvalidJson",
            "Failed to parse request body as JSON",
        )
    }

    pub fn invalid_uuid() -> Self {
        Self::new(ErrorClass::BadRequest, "InvalidUuid", "Invalid UUID format")
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::BadRequest, "InvalidRequest", message)
    }

    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::NotFound, code, message)
    }

    pub fn feature_not_found(feature_id: &str) -> Self {
        Self::new(
            ErrorClass::NotFound,
            "FeatureNotFound",
            format!("feature not found: {feature_id}"),
        )
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

fn info(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

pub fn app_error_from_storage_error(err: StorageError) -> AppError {
    match err {
        StorageError::Concurrency(info) => {
            AppError::new(ErrorClass::Conflict, info.code.0, info.message)
        }
        StorageError::NotFound(info) => {
            AppError::new(ErrorClass::NotFound, info.code.0, info.message)
        }
        StorageError::Corruption(info) | StorageError::Other(info) => {
            AppError::new(ErrorClass::Internal, info.code.0, info.message)
        }
    }
}

pub fn app_error_from_run_error(err: RunError) -> AppError {
    let info = match err {
        RunError::InvalidPlan(info) => info,
        RunError::Storage(se) => match se {
            StorageError::Concurrency(info)
            | StorageError::NotFound(info)
            | StorageError::Corruption(info)
            | StorageError::Other(info) => info,
        },
        RunError::Context(ce) => match ce {
            ContextError::MissingKey { info, .. }
            | ContextError::Serialization(info)
            | ContextError::Other(info) => info,
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

    let class = match info.category {
        ErrorCategory::ParsingInput => ErrorClass::BadRequest,
        ErrorCategory::OnChain | ErrorCategory::OffChain | ErrorCategory::Rpc => {
            ErrorClass::BadGateway
        }
        ErrorCategory::Storage | ErrorCategory::Context | ErrorCategory::Unknown => {
            ErrorClass::Internal
        }
    };

    AppError::new(class, info.code.0, info.message)
}

#[derive(Default)]
pub struct MapContext {
    inner: HashMap<String, serde_json::Value>,
}

impl DynContext for MapContext {
    fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
        Ok(self.inner.get(&key.0).cloned())
    }

    fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
        self.inner.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
        self.inner.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, ContextError> {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.inner {
            m.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(m))
    }
}

pub fn default_initial_context() -> Box<dyn DynContext> {
    Box::new(MapContext::default())
}

pub fn default_build_provenance() -> BuildProvenance {
    BuildProvenance {
        git_commit: None,
        cargo_lock_hash: None,
        flake_lock_hash: None,
        rustc_version: None,
        target_triple: None,
        env_allowlist: Vec::new(),
    }
}

pub fn default_run_config() -> RunConfig {
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
        nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
    }
}

pub fn phase_str(phase: &RunPhase) -> &'static str {
    match phase {
        RunPhase::Running => "running",
        RunPhase::Completed => "completed",
        RunPhase::Failed => "failed",
        RunPhase::Cancelled => "cancelled",
    }
}

pub fn default_artifact_root() -> PathBuf {
    std::env::var(ENV_ARTIFACT_ROOT)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("run_artifacts")
        })
}

pub async fn make_default_artifact_store() -> Result<Arc<dyn ArtifactStore>, AppError> {
    let backend = std::env::var(ENV_ARTIFACT_BACKEND).unwrap_or_else(|_| "fs".to_string());
    match backend.as_str() {
        "fs" => Ok(Arc::new(FsArtifactStore::new(default_artifact_root()))),
        "s3" => {
            let store = S3ArtifactStore::from_env().map_err(app_error_from_storage_error)?;
            if std::env::var(ENV_S3_ENSURE_BUCKET).is_ok() {
                store
                    .ensure_bucket_exists()
                    .await
                    .map_err(app_error_from_storage_error)?;
            }
            Ok(Arc::new(store))
        }
        other => Err(AppError::new(
            ErrorClass::Internal,
            "InvalidArtifactBackend",
            format!("invalid {ENV_ARTIFACT_BACKEND}: {other}"),
        )),
    }
}

pub async fn make_default_event_store() -> Result<Arc<dyn EventStore>, AppError> {
    let database_url = std::env::var(ENV_DATABASE_URL).map_err(|_| {
        AppError::new(
            ErrorClass::Internal,
            "MissingDatabaseUrl",
            format!("Missing {ENV_DATABASE_URL}"),
        )
    })?;

    let store = PostgresEventStore::connect(&database_url)
        .await
        .map_err(app_error_from_storage_error)?;

    Ok(Arc::new(store))
}

struct AppLiveIoTransportFactory;

impl LiveIoTransportFactory for AppLiveIoTransportFactory {
    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(AppLiveIoTransport)
    }
}

struct AppLiveIoTransport;

#[async_trait]
impl LiveIoTransport for AppLiveIoTransport {
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
    reg.register(Arc::new(EvmContractFromNixOp));
    reg.register(Arc::new(EvmDeployOp));
    reg.register(Arc::new(EvmConfigureOp));
    reg.register(Arc::new(EvmValidateOp));
    reg.register(Arc::new(NixAppOp));
    let registry: Arc<dyn OperationRegistry> = Arc::new(reg);

    let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
    let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
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

    let mut routes: HashMap<String, Arc<dyn LiveIoTransportFactory>> = HashMap::new();
    routes.insert("proof".to_string(), Arc::new(AppLiveIoTransportFactory));
    routes.insert(
        "exec".to_string(),
        Arc::new(ExecProgramTransportFactory::default()),
    );
    routes.insert(
        "nix".to_string(),
        Arc::new(NixFlakeTransportFactory::default()),
    );
    routes.insert("evm".to_string(), evm_factory);

    let base_factory: Arc<dyn LiveIoTransportFactory> =
        Arc::new(RouterLiveIoTransportFactory::new(routes));
    let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(ChildRunLiveIoTransportFactory::new(
        Arc::clone(&resolver),
        Arc::clone(&base_factory),
    ));
    let engine: Arc<dyn ExecutionEngine> =
        Arc::new(DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory));

    EngineBundle {
        engine,
        registry,
        planner,
    }
}

#[derive(Clone)]
pub struct AppServices {
    pub bundle: EngineBundle,
    pub events: Arc<dyn EventStore>,
    pub artifacts: Arc<dyn ArtifactStore>,
}

impl AppServices {
    pub fn new(
        bundle: EngineBundle,
        events: Arc<dyn EventStore>,
        artifacts: Arc<dyn ArtifactStore>,
    ) -> Self {
        Self {
            bundle,
            events,
            artifacts,
        }
    }

    fn stores(&self) -> Stores {
        Stores {
            events: Arc::clone(&self.events),
            artifacts: Arc::clone(&self.artifacts),
        }
    }

    pub async fn start_run(&self, req: RunsStartRequest) -> Result<RunStartResponse, AppError> {
        let (pipeline, input, run_config) = match req {
            RunsStartRequest::Single(req) => {
                let pipeline = single_op_pipeline(OpId(req.op_id), req.op_version, req.op_config)
                    .map_err(|e| {
                    AppError::new(ErrorClass::BadRequest, e.info.code.0, e.info.message)
                })?;
                (pipeline, serde_json::json!({}), default_run_config())
            }
            RunsStartRequest::Pipeline(req) => (
                req.pipeline,
                req.input,
                req.run_config.unwrap_or_else(default_run_config),
            ),
        };

        let launcher = DefaultRunLauncher;
        let run = launcher
            .start_pipeline(
                Arc::clone(&self.bundle.engine),
                self.stores(),
                Arc::clone(&self.bundle.registry),
                Arc::clone(&self.bundle.planner),
                LaunchPipeline {
                    pipeline,
                    input,
                    run_config,
                    build: default_build_provenance(),
                    initial_context: default_initial_context(),
                },
            )
            .await
            .map_err(app_error_from_run_error)?;

        Ok(RunStartResponse {
            run_id: run.run_id.0.to_string(),
            phase: phase_str(&run.phase).to_string(),
            final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
        })
    }

    pub async fn resume_run(&self, run_id: &str) -> Result<RunResumeResponse, AppError> {
        let uuid = uuid::Uuid::parse_str(run_id).map_err(|_| AppError::invalid_uuid())?;
        let run_id = RunId(uuid);

        let launcher = DefaultRunLauncher;
        let run = launcher
            .resume(
                Arc::clone(&self.bundle.engine),
                self.stores(),
                Arc::clone(&self.bundle.registry),
                Arc::clone(&self.bundle.planner),
                run_id,
            )
            .await
            .map_err(app_error_from_run_error)?;

        Ok(RunResumeResponse {
            run_id: run.run_id.0.to_string(),
            phase: phase_str(&run.phase).to_string(),
            final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
        })
    }

    pub async fn run_status(&self, run_id: &str) -> Result<RunStatusResponse, AppError> {
        let uuid = uuid::Uuid::parse_str(run_id).map_err(|_| AppError::invalid_uuid())?;
        let run_id = RunId(uuid);

        let head = self
            .events
            .head_seq(run_id)
            .await
            .map_err(app_error_from_storage_error)?;
        if head == 0 {
            return Err(AppError::not_found(
                "run_not_found",
                "run event stream was not found",
            ));
        }

        let stream = self
            .events
            .read_range(run_id, 1, None)
            .await
            .map_err(app_error_from_storage_error)?;

        let mut op_id = None;
        let mut manifest_id = None;
        let mut completed: Option<(RunStatus, Option<String>)> = None;

        for e in &stream {
            let Event::Kernel(ke) = &e.event else {
                continue;
            };
            match ke {
                KernelEvent::RunStarted {
                    op_id: oid,
                    manifest_id: mid,
                    initial_snapshot_id: _,
                } => {
                    op_id = Some(oid.0.clone());
                    manifest_id = Some(mid.0.clone());
                }
                KernelEvent::RunCompleted {
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
                    RunStatus::Completed => "completed".to_string(),
                    RunStatus::Failed => "failed".to_string(),
                    RunStatus::Cancelled => "cancelled".to_string(),
                },
                id,
            ),
            None => ("running".to_string(), None),
        };

        Ok(RunStatusResponse {
            run_id: run_id.0.to_string(),
            head_seq: head,
            op_id,
            manifest_id,
            phase,
            final_snapshot_id,
        })
    }

    pub async fn run_events(
        &self,
        run_id: &str,
        query: RunsEventsQuery,
    ) -> Result<RunsEventsResponse, AppError> {
        let uuid = uuid::Uuid::parse_str(run_id).map_err(|_| AppError::invalid_uuid())?;
        let run_id = RunId(uuid);

        let head = self
            .events
            .head_seq(run_id)
            .await
            .map_err(app_error_from_storage_error)?;
        if head == 0 {
            return Err(AppError::not_found(
                "run_not_found",
                "run event stream was not found",
            ));
        }

        let events = self
            .events
            .read_range(run_id, query.from_seq, query.to_seq)
            .await
            .map_err(app_error_from_storage_error)?;

        Ok(RunsEventsResponse {
            run_id: run_id.0.to_string(),
            head_seq: head,
            events,
        })
    }

    pub async fn artifact_get(&self, artifact_id: &str) -> Result<ArtifactGetResponse, AppError> {
        get_artifact_from_store(Arc::clone(&self.artifacts), artifact_id).await
    }

    pub async fn start_deploy_configure_validate(
        &self,
        spec: DeployConfigureValidateSpec,
    ) -> Result<RunStartResponse, AppError> {
        let pipeline = pipeline_from_deploy_configure_validate_spec(spec.clone());
        self.start_run(RunsStartRequest::Pipeline(PipelineStartRequest {
            pipeline,
            input: spec.input,
            run_config: Some(default_run_config()),
        }))
        .await
    }
}

pub async fn get_artifact_from_store(
    artifacts: Arc<dyn ArtifactStore>,
    artifact_id: &str,
) -> Result<ArtifactGetResponse, AppError> {
    let id = ArtifactId(artifact_id.to_string());

    let bytes = artifacts
        .get(&id)
        .await
        .map_err(app_error_from_storage_error)?;

    let body = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => ArtifactBody::Json { value },
        Err(_) => ArtifactBody::Hex {
            hex: hex::encode(bytes),
        },
    };

    Ok(ArtifactGetResponse {
        artifact_id: id.0,
        body,
    })
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum RunsStartRequest {
    Single(SingleOpStartRequest),
    Pipeline(PipelineStartRequest),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SingleOpStartRequest {
    #[serde(default = "default_op_id")]
    pub op_id: String,

    #[serde(default = "default_op_version")]
    pub op_version: String,

    #[serde(default = "default_empty_object")]
    pub op_config: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PipelineStartRequest {
    pub pipeline: Pipeline,

    #[serde(default = "default_empty_object")]
    pub input: serde_json::Value,

    #[serde(default)]
    pub run_config: Option<RunConfig>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunStartResponse {
    pub run_id: String,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

impl fmt::Display for RunStartResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "run_id: {}", self.run_id)?;
        writeln!(f, "phase: {}", self.phase)?;
        if let Some(id) = &self.final_snapshot_id {
            writeln!(f, "final_snapshot_id: {id}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RunResumeResponse {
    pub run_id: String,
    pub phase: String,
    pub final_snapshot_id: Option<String>,
}

impl fmt::Display for RunResumeResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "run_id: {}", self.run_id)?;
        writeln!(f, "phase: {}", self.phase)?;
        if let Some(id) = &self.final_snapshot_id {
            writeln!(f, "final_snapshot_id: {id}")?;
        }
        Ok(())
    }
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

impl fmt::Display for RunStatusResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "run_id: {}", self.run_id)?;
        writeln!(f, "head_seq: {}", self.head_seq)?;
        if let Some(op_id) = &self.op_id {
            writeln!(f, "op_id: {op_id}")?;
        }
        if let Some(manifest_id) = &self.manifest_id {
            writeln!(f, "manifest_id: {manifest_id}")?;
        }
        writeln!(f, "phase: {}", self.phase)?;
        if let Some(id) = &self.final_snapshot_id {
            writeln!(f, "final_snapshot_id: {id}")?;
        }
        Ok(())
    }
}

fn default_from_seq() -> u64 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

impl fmt::Display for RunsEventsResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string_pretty(&self.events).unwrap_or_else(|_| "[]".to_string());
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactGetResponse {
    pub artifact_id: String,
    #[serde(flatten)]
    pub body: ArtifactBody,
}

impl fmt::Display for ArtifactGetResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.body {
            ArtifactBody::Json { value } => {
                let s =
                    serde_json::to_string_pretty(value).unwrap_or_else(|_| "<invalid json>".into());
                write!(f, "{s}")
            }
            ArtifactBody::Hex { hex } => write!(f, "{hex}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "encoding", rename_all = "lowercase")]
pub enum ArtifactBody {
    Json { value: serde_json::Value },
    Hex { hex: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployConfigureValidateSpec {
    #[serde(default = "default_machine_id")]
    pub machine_id: String,

    #[serde(default = "default_pipeline_version")]
    pub pipeline_version: String,

    #[serde(default = "default_empty_object")]
    pub input: serde_json::Value,

    pub deploy: serde_json::Value,
    pub configure: serde_json::Value,
    pub validate: serde_json::Value,
}

pub fn pipeline_from_deploy_configure_validate_spec(spec: DeployConfigureValidateSpec) -> Pipeline {
    Pipeline {
        machine_id: MachineId(spec.machine_id),
        pipeline_version: spec.pipeline_version,
        steps: vec![
            PipelineStep {
                step_id: StepId("deploy".to_string()),
                op_id: OpId("evm_deploy".to_string()),
                op_version: "v1".to_string(),
                op_config: spec.deploy,
            },
            PipelineStep {
                step_id: StepId("configure".to_string()),
                op_id: OpId("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: spec.configure,
            },
            PipelineStep {
                step_id: StepId("validate".to_string()),
                op_id: OpId("evm_validate".to_string()),
                op_version: "v1".to_string(),
                op_config: spec.validate,
            },
        ],
    }
}

fn default_op_id() -> String {
    "proof".to_string()
}

fn default_op_version() -> String {
    "v1".to_string()
}

fn default_machine_id() -> String {
    "evm_deploy_configure_validate".to_string()
}

fn default_pipeline_version() -> String {
    "v1".to_string()
}

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    Operation,
    PipelineTemplate,
    RunControl,
    Artifact,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureDescriptor {
    pub id: String,
    pub version: String,
    pub kind: FeatureKind,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureRequest {
    pub feature_id: String,
    #[serde(default = "default_empty_object")]
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureExecutionResult {
    pub feature_id: String,
    pub result: serde_json::Value,
}

#[derive(Clone, Default)]
pub struct FeatureCatalog {
    handlers: HashMap<String, BuiltinFeature>,
    descriptors: Vec<FeatureDescriptor>,
}

#[derive(Clone, Copy)]
enum BuiltinFeature {
    RunStart,
    RunResume,
    RunStatus,
    RunEvents,
    ArtifactGet,
    PipelineDeployConfigureValidateStart,
}

impl FeatureCatalog {
    pub fn with_builtins() -> Self {
        let mut handlers = HashMap::new();
        handlers.insert("run.start".to_string(), BuiltinFeature::RunStart);
        handlers.insert("run.resume".to_string(), BuiltinFeature::RunResume);
        handlers.insert("run.status".to_string(), BuiltinFeature::RunStatus);
        handlers.insert("run.events".to_string(), BuiltinFeature::RunEvents);
        handlers.insert("artifact.get".to_string(), BuiltinFeature::ArtifactGet);
        handlers.insert(
            "pipeline.deploy_configure_validate.start".to_string(),
            BuiltinFeature::PipelineDeployConfigureValidateStart,
        );

        let descriptors = vec![
            FeatureDescriptor {
                id: "run.start".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::RunControl,
                description: "Start a run from either a single op or a pipeline payload"
                    .to_string(),
                input_schema: serde_json::json!({
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "op_id": {"type": "string"},
                                "op_version": {"type": "string"},
                                "op_config": {"type": "object"}
                            }
                        },
                        {
                            "type": "object",
                            "properties": {
                                "pipeline": {"type": "object"},
                                "input": {"type": "object"},
                                "run_config": {"type": "object"}
                            },
                            "required": ["pipeline"]
                        }
                    ]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {"type": "string"},
                        "phase": {"type": "string"},
                        "final_snapshot_id": {"type": ["string", "null"]}
                    },
                    "required": ["run_id", "phase"]
                }),
            },
            FeatureDescriptor {
                id: "run.resume".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::RunControl,
                description: "Resume a run by id".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"run_id": {"type": "string"}},
                    "required": ["run_id"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {"type": "string"},
                        "phase": {"type": "string"},
                        "final_snapshot_id": {"type": ["string", "null"]}
                    },
                    "required": ["run_id", "phase"]
                }),
            },
            FeatureDescriptor {
                id: "run.status".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::RunControl,
                description: "Read run status without executing states".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"run_id": {"type": "string"}},
                    "required": ["run_id"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {"type": "string"},
                        "head_seq": {"type": "integer"},
                        "op_id": {"type": ["string", "null"]},
                        "manifest_id": {"type": ["string", "null"]},
                        "phase": {"type": "string"},
                        "final_snapshot_id": {"type": ["string", "null"]}
                    },
                    "required": ["run_id", "head_seq", "phase"]
                }),
            },
            FeatureDescriptor {
                id: "run.events".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::RunControl,
                description: "Read run events in a sequence range".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {"type": "string"},
                        "from_seq": {"type": "integer"},
                        "to_seq": {"type": ["integer", "null"]}
                    },
                    "required": ["run_id"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {"type": "string"},
                        "head_seq": {"type": "integer"},
                        "events": {"type": "array"}
                    },
                    "required": ["run_id", "head_seq", "events"]
                }),
            },
            FeatureDescriptor {
                id: "artifact.get".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::Artifact,
                description: "Fetch an artifact by id".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"artifact_id": {"type": "string"}},
                    "required": ["artifact_id"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "artifact_id": {"type": "string"},
                        "encoding": {"type": "string"}
                    },
                    "required": ["artifact_id", "encoding"]
                }),
            },
            FeatureDescriptor {
                id: "pipeline.deploy_configure_validate.start".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::PipelineTemplate,
                description: "Start the standard deploy->configure->validate pipeline template"
                    .to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "machine_id": {"type": "string"},
                        "pipeline_version": {"type": "string"},
                        "input": {"type": "object"},
                        "deploy": {"type": "object"},
                        "configure": {"type": "object"},
                        "validate": {"type": "object"}
                    },
                    "required": ["deploy", "configure", "validate"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {"type": "string"},
                        "phase": {"type": "string"},
                        "final_snapshot_id": {"type": ["string", "null"]}
                    },
                    "required": ["run_id", "phase"]
                }),
            },
        ];

        Self {
            handlers,
            descriptors,
        }
    }

    pub fn descriptors(&self) -> &[FeatureDescriptor] {
        &self.descriptors
    }

    pub async fn execute(
        &self,
        services: &AppServices,
        req: FeatureRequest,
    ) -> Result<FeatureExecutionResult, AppError> {
        let feature = self
            .handlers
            .get(&req.feature_id)
            .copied()
            .ok_or_else(|| AppError::feature_not_found(&req.feature_id))?;

        let result = match feature {
            BuiltinFeature::RunStart => {
                let parsed: RunsStartRequest =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                serde_json::to_value(services.start_run(parsed).await?)
            }
            BuiltinFeature::RunResume => {
                let parsed: RunIdInput =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                serde_json::to_value(services.resume_run(&parsed.run_id).await?)
            }
            BuiltinFeature::RunStatus => {
                let parsed: RunIdInput =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                serde_json::to_value(services.run_status(&parsed.run_id).await?)
            }
            BuiltinFeature::RunEvents => {
                let parsed: RunEventsInput =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                let query = RunsEventsQuery {
                    from_seq: parsed.from_seq.unwrap_or(1),
                    to_seq: parsed.to_seq,
                };
                serde_json::to_value(services.run_events(&parsed.run_id, query).await?)
            }
            BuiltinFeature::ArtifactGet => {
                let parsed: ArtifactIdInput =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                serde_json::to_value(services.artifact_get(&parsed.artifact_id).await?)
            }
            BuiltinFeature::PipelineDeployConfigureValidateStart => {
                let parsed: DeployConfigureValidateSpec =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                serde_json::to_value(services.start_deploy_configure_validate(parsed).await?)
            }
        }
        .map_err(|_| {
            AppError::new(
                ErrorClass::Internal,
                "SerializationError",
                "Failed to serialize feature result",
            )
        })?;

        Ok(FeatureExecutionResult {
            feature_id: req.feature_id,
            result,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RunIdInput {
    run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ArtifactIdInput {
    artifact_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct RunEventsInput {
    run_id: String,
    from_seq: Option<u64>,
    to_seq: Option<u64>,
}
