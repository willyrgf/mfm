#![warn(missing_docs)]
//! Application-facing orchestration bridge for MFM binaries and transports.
//!
//! `mfm-app` wires together the operation registry, transport registry, storage defaults, and
//! higher-level request/response helpers used by the CLI and REST API. It keeps binary crates thin
//! by exposing transport-safe entrypoints that map requests onto SDK launch/resume flows.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_app::{
//!     make_default_artifact_store, make_default_event_store, make_engine_bundle, AppServices,
//! };
//!
//! async fn boot() -> Result<AppServices, mfm_app::AppError> {
//!     let bundle = make_engine_bundle();
//!     let events = make_default_event_store().await?;
//!     let artifacts = make_default_artifact_store().await?;
//!     Ok(AppServices::new(bundle, events, artifacts))
//! }
//! ```
/// Shared observability configuration used by the CLI and REST API.
pub mod observability;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_collectors_evm_jsonrpc_http::{
    resolve_evm_rpc_sources_from_env, EvmJsonRpcHttpTransportFactory,
};
use mfm_collectors_nix_exec::NixFlakeTransportFactory;
use mfm_event_store_postgres::PostgresEventStore;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
use mfm_machine::errors::{ContextError, ErrorCategory, IoError, RunError, StorageError};
use mfm_machine::events::{Event, EventEnvelope, KernelEvent, RunStatus};
use mfm_machine::exec_transport::ExecProgramTransportFactory;
use mfm_machine::ids::{ArtifactId, ContextKey, OpId, RunId};
use mfm_machine::live_io::LiveIoTransportFactory;
use mfm_machine::live_io_registry::{HashMapTransportRegistry, TransportRegistry};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::runtime::{ChildRunLiveIoTransportFactory, DefaultExecutionEngine, PlanResolver};
use mfm_machine::stores::{ArtifactStore, EventStore};
use mfm_op_aave_v3_origin_adapt::AaveV3OriginAdaptDeployOp;
use mfm_op_evm_deploy_configure_validate::{
    EvmDeployConfigureValidateOp, EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID,
    EVM_DEPLOY_CONFIGURE_VALIDATE_OP_VERSION,
};
use mfm_op_evm_read::EvmReadOp;
use mfm_op_evm_write::{EvmConfigureOp, EvmContractFromNixOp, EvmDeployOp, EvmValidateOp};
use mfm_op_keystore_admin::{KeystoreDeleteOp, KeystoreImportOp, KeystoreListOp};
use mfm_op_keystore_tx::{KeystoreTxSendRawOp, KeystoreTxSignOp};
use mfm_op_nix_app::NixAppOp;
use mfm_op_portfolio_tracker::{
    portfolio_tracker_report_context_key, PortfolioBalanceReport, PortfolioTrackerOp,
    PortfolioTrackerReport,
};
use mfm_op_proof::ProofOp;
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::op::OperationRegistry;
use mfm_sdk::pipeline::{Pipeline, PipelinePlanner, PipelineStep};
use mfm_sdk::unstable::{
    single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
    SdkPlanResolver,
};
use mfm_transports_local_evm::LocalEvmIoTransportFactory;
use mfm_transports_local_fs::LocalFsIoTransportFactory;
use mfm_transports_local_keystore::LocalKeystoreIoTransportFactory;
use mfm_transports_proof::ProofIoTransportFactory;

const ENV_ARTIFACT_BACKEND: &str = "MFM_ARTIFACT_BACKEND";
const ENV_ARTIFACT_ROOT: &str = "MFM_ARTIFACT_ROOT";
const ENV_DATABASE_URL: &str = "DATABASE_URL";
const ENV_PORTFOLIO_TOKENS_JSON: &str = "MFM_PORTFOLIO_TOKENS_JSON";
const ENV_S3_ENSURE_BUCKET: &str = "MFM_S3_ENSURE_BUCKET";

/// High-level error classes used by application-facing APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// The caller provided invalid input.
    BadRequest,
    /// The requested run, artifact, or feature was not found.
    NotFound,
    /// The request conflicted with current persisted state.
    Conflict,
    /// A downstream dependency such as RPC failed.
    BadGateway,
    /// An internal application error occurred.
    Internal,
}

/// Stable error payload returned by application-facing helper APIs.
#[derive(Debug, Clone)]
pub struct AppError {
    /// High-level error classification for HTTP/CLI mapping.
    pub class: ErrorClass,
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable message safe to display to callers.
    pub message: String,
}

impl AppError {
    /// Creates an application error from the supplied classification, code, and message.
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into(),
        }
    }

    /// Returns the standard invalid-JSON error payload.
    pub fn invalid_json() -> Self {
        Self::new(
            ErrorClass::BadRequest,
            "InvalidJson",
            "Failed to parse request body as JSON",
        )
    }

    /// Returns the standard invalid-UUID error payload.
    pub fn invalid_uuid() -> Self {
        Self::new(ErrorClass::BadRequest, "InvalidUuid", "Invalid UUID format")
    }

    /// Returns a generic bad-request error with the supplied message.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorClass::BadRequest, "InvalidRequest", message)
    }

    /// Returns a not-found error with an explicit code and message.
    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::NotFound, code, message)
    }

    /// Returns the canonical error for unknown feature identifiers.
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

/// Maps a storage-layer error into the application error contract.
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

/// Maps an engine `RunError` into the application error contract.
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

/// In-memory context implementation used by default request flows.
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

/// Returns the default empty initial context for new runs.
pub fn default_initial_context() -> Box<dyn DynContext> {
    Box::new(MapContext::default())
}

/// Returns build provenance defaults for application-triggered runs.
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

/// Returns the baseline run configuration used by app-facing helpers.
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

/// Converts an engine phase to the stable lowercase response string.
pub fn phase_str(phase: &RunPhase) -> &'static str {
    match phase {
        RunPhase::Running => "running",
        RunPhase::Completed => "completed",
        RunPhase::Failed => "failed",
        RunPhase::Cancelled => "cancelled",
    }
}

/// Resolves the default local artifact root from `MFM_ARTIFACT_ROOT` or `$HOME/.mfm/run_artifacts`.
pub fn default_artifact_root() -> PathBuf {
    std::env::var(ENV_ARTIFACT_ROOT)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("run_artifacts")
        })
}

/// Builds the default artifact store from environment configuration.
#[instrument(level = "info", skip_all)]
pub async fn make_default_artifact_store() -> Result<Arc<dyn ArtifactStore>, AppError> {
    let backend = std::env::var(ENV_ARTIFACT_BACKEND).unwrap_or_else(|_| "fs".to_string());
    info!(backend = %backend, "initializing artifact store");
    match backend.as_str() {
        "fs" => {
            let root = default_artifact_root();
            info!(artifact_root = %root.display(), "using filesystem artifact store");
            Ok(Arc::new(FsArtifactStore::new(root)))
        }
        "s3" => {
            let store = S3ArtifactStore::from_env().map_err(app_error_from_storage_error)?;
            if std::env::var(ENV_S3_ENSURE_BUCKET).is_ok() {
                info!("ensuring s3 bucket exists");
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

/// Builds the default event store from environment configuration.
#[instrument(level = "info", skip_all)]
pub async fn make_default_event_store() -> Result<Arc<dyn EventStore>, AppError> {
    let database_url = std::env::var(ENV_DATABASE_URL).map_err(|_| {
        AppError::new(
            ErrorClass::Internal,
            "MissingDatabaseUrl",
            format!("Missing {ENV_DATABASE_URL}"),
        )
    })?;

    info!(database_url_set = true, "initializing postgres event store");
    let store = PostgresEventStore::connect(&database_url)
        .await
        .map_err(app_error_from_storage_error)?;

    Ok(Arc::new(store))
}

/// Shared engine wiring used by application-facing services.
#[derive(Clone)]
pub struct EngineBundle {
    /// Execution engine used for start and resume requests.
    pub engine: Arc<dyn ExecutionEngine>,
    /// Operation registry used for planning and resume.
    pub registry: Arc<dyn OperationRegistry>,
    /// Pipeline planner used to build execution plans.
    pub planner: Arc<dyn PipelinePlanner>,
}

/// Extension point for registering operations into the default app bundle.
pub trait OperationPlugin: Send + Sync {
    /// Registers operations on the mutable registry during application boot.
    fn register_operations(&self, registry: &mut HashMapOperationRegistry);
}

/// Extension point for registering live-IO transports into the default app bundle.
pub trait TransportPlugin: Send + Sync {
    /// Registers transports on the mutable registry during application boot.
    fn register_transports(&self, registry: &mut HashMapTransportRegistry) -> Result<(), AppError>;
}

/// Default operation plugin that installs the built-in operation catalog.
#[derive(Clone, Default)]
pub struct DefaultOperationPlugin;

impl OperationPlugin for DefaultOperationPlugin {
    fn register_operations(&self, registry: &mut HashMapOperationRegistry) {
        registry.register(Arc::new(ProofOp::default()));
        registry.register(Arc::new(KeystoreImportOp));
        registry.register(Arc::new(KeystoreListOp));
        registry.register(Arc::new(KeystoreDeleteOp));
        registry.register(Arc::new(KeystoreTxSignOp));
        registry.register(Arc::new(KeystoreTxSendRawOp));
        registry.register(Arc::new(EvmReadOp));
        registry.register(Arc::new(EvmContractFromNixOp));
        registry.register(Arc::new(EvmDeployOp));
        registry.register(Arc::new(EvmConfigureOp));
        registry.register(Arc::new(EvmValidateOp));
        registry.register(Arc::new(EvmDeployConfigureValidateOp));
        registry.register(Arc::new(PortfolioTrackerOp));
        registry.register(Arc::new(NixAppOp));
        registry.register(Arc::new(AaveV3OriginAdaptDeployOp));
    }
}

/// Default transport plugin that installs the built-in transport catalog.
#[derive(Clone, Default)]
pub struct DefaultTransportPlugin;

impl TransportPlugin for DefaultTransportPlugin {
    fn register_transports(&self, registry: &mut HashMapTransportRegistry) -> Result<(), AppError> {
        register_transport_factory(registry, Arc::new(ProofIoTransportFactory))?;
        register_transport_factory(registry, Arc::new(ExecProgramTransportFactory::default()))?;
        register_transport_factory(registry, Arc::new(NixFlakeTransportFactory::from_env()))?;
        register_transport_factory(registry, Arc::new(LocalFsIoTransportFactory))?;
        register_transport_factory(registry, Arc::new(LocalEvmIoTransportFactory))?;
        register_transport_factory(registry, Arc::new(LocalKeystoreIoTransportFactory))?;

        let evm_transport = EvmJsonRpcHttpTransportFactory::from_env().map_err(|err| {
            AppError::new(
                ErrorClass::Internal,
                "EvmTransportConfigInvalid",
                err.to_string(),
            )
        })?;
        register_transport_factory(registry, Arc::new(evm_transport))?;
        Ok(())
    }
}

fn register_transport_factory(
    registry: &mut HashMapTransportRegistry,
    factory: Arc<dyn LiveIoTransportFactory>,
) -> Result<(), AppError> {
    registry.register(factory).map_err(|err| {
        AppError::new(
            ErrorClass::Internal,
            "TransportRegistrationFailed",
            err.to_string(),
        )
    })
}

/// Builder for the default application engine bundle.
///
/// This is the main customization point for callers that want the built-in MFM wiring with a few
/// additional operation or transport plugins layered in before boot.
///
/// # Examples
///
/// ```rust
/// use mfm_app::{AppBuilder, DefaultOperationPlugin};
/// use std::sync::Arc;
///
/// let bundle = AppBuilder::new()
///     .with_operation_plugin(Arc::new(DefaultOperationPlugin))
///     .build()
///     .expect("builder should assemble the default engine");
///
/// let _engine = bundle.engine.clone();
/// ```
pub struct AppBuilder {
    operation_plugins: Vec<Arc<dyn OperationPlugin>>,
    transport_plugins: Vec<Arc<dyn TransportPlugin>>,
    planner: Arc<dyn PipelinePlanner>,
}

impl Default for AppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AppBuilder {
    /// Creates an application builder with the default plugins installed.
    pub fn new() -> Self {
        Self {
            operation_plugins: vec![Arc::new(DefaultOperationPlugin)],
            transport_plugins: vec![Arc::new(DefaultTransportPlugin)],
            planner: Arc::new(DefaultPipelinePlanner),
        }
    }

    /// Adds an operation plugin to the application builder.
    pub fn with_operation_plugin(mut self, plugin: Arc<dyn OperationPlugin>) -> Self {
        self.operation_plugins.push(plugin);
        self
    }

    /// Adds a transport plugin to the application builder.
    pub fn with_transport_plugin(mut self, plugin: Arc<dyn TransportPlugin>) -> Self {
        self.transport_plugins.push(plugin);
        self
    }

    /// Builds the engine bundle used by `AppServices`.
    pub fn build(self) -> Result<EngineBundle, AppError> {
        let mut reg = HashMapOperationRegistry::default();
        for plugin in &self.operation_plugins {
            plugin.register_operations(&mut reg);
        }
        let registry: Arc<dyn OperationRegistry> = Arc::new(reg);

        let planner = self.planner;
        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));

        let mut transports = HashMapTransportRegistry::new();
        for plugin in &self.transport_plugins {
            plugin.register_transports(&mut transports)?;
        }

        let base_factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(RouterLiveIoTransportFactory::from_registry(&transports));
        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(
            ChildRunLiveIoTransportFactory::new(Arc::clone(&resolver), Arc::clone(&base_factory)),
        );
        let engine: Arc<dyn ExecutionEngine> =
            Arc::new(DefaultExecutionEngine::new(resolver).with_live_transport_factory(factory));

        Ok(EngineBundle {
            engine,
            registry,
            planner,
        })
    }
}

/// Builds the default engine bundle and panics only if the built-in wiring is invalid.
pub fn make_engine_bundle() -> EngineBundle {
    AppBuilder::new()
        .build()
        .expect("default app builder must build an engine bundle")
}

/// High-level service facade used by the CLI and REST API.
#[derive(Clone)]
pub struct AppServices {
    /// Engine bundle used for planning and execution.
    pub bundle: EngineBundle,
    /// Event store used for run status and event queries.
    pub events: Arc<dyn EventStore>,
    /// Artifact store used for snapshots, facts, and outputs.
    pub artifacts: Arc<dyn ArtifactStore>,
}

impl AppServices {
    /// Creates a new service facade from the supplied engine bundle and stores.
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

    #[instrument(
        level = "info",
        skip(self, req),
        fields(op_id, machine_id, request_kind)
    )]
    /// Starts a new run from either a single-op request or a full pipeline request.
    pub async fn start_run(&self, req: RunsStartRequest) -> Result<RunStartResponse, AppError> {
        let (pipeline, input, run_config) = match req {
            RunsStartRequest::Single(req) => {
                tracing::Span::current().record("request_kind", "single");
                tracing::Span::current().record("op_id", req.op_id.as_str());
                let op_id = OpId::new(req.op_id).map_err(|_| {
                    AppError::new(
                        ErrorClass::BadRequest,
                        "invalid_op_id",
                        "op_id must match ^[a-z][a-z0-9_]{0,62}$",
                    )
                })?;
                let pipeline =
                    single_op_pipeline(op_id, req.op_version, req.op_config).map_err(|e| {
                        AppError::new(ErrorClass::BadRequest, e.info.code.0, e.info.message)
                    })?;
                (pipeline, serde_json::json!({}), default_run_config())
            }
            RunsStartRequest::Pipeline(req) => {
                tracing::Span::current().record("request_kind", "pipeline");
                tracing::Span::current().record("machine_id", req.pipeline.machine_id.0.as_str());
                (
                    req.pipeline,
                    req.input,
                    req.run_config.unwrap_or_else(default_run_config),
                )
            }
        };

        debug!(
            step_count = pipeline.steps.len(),
            "launching run for pipeline"
        );
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
        info!(
            run_id = %run.run_id.0,
            phase = %phase_str(&run.phase),
            "run start finished"
        );

        Ok(RunStartResponse {
            run_id: run.run_id.0.to_string(),
            phase: phase_str(&run.phase).to_string(),
            final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
        })
    }

    #[instrument(level = "info", skip(self), fields(run_id = run_id))]
    /// Resumes a previously started run by UUID string.
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
        info!(
            run_id = %run.run_id.0,
            phase = %phase_str(&run.phase),
            "run resume finished"
        );

        Ok(RunResumeResponse {
            run_id: run.run_id.0.to_string(),
            phase: phase_str(&run.phase).to_string(),
            final_snapshot_id: run.final_snapshot_id.map(|id| id.0),
        })
    }

    #[instrument(level = "debug", skip(self), fields(run_id = run_id))]
    /// Returns the current run status by scanning the persisted event stream.
    pub async fn run_status(&self, run_id: &str) -> Result<RunStatusResponse, AppError> {
        let uuid = uuid::Uuid::parse_str(run_id).map_err(|_| AppError::invalid_uuid())?;
        let run_id = RunId(uuid);

        let head = self
            .events
            .head_seq(run_id)
            .await
            .map_err(app_error_from_storage_error)?;
        if head == 0 {
            warn!("run not found while reading status");
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
        debug!(event_count = stream.len(), "loaded run event stream");

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
                    op_id = Some(oid.to_string());
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

    #[instrument(
        level = "debug",
        skip(self, query),
        fields(run_id = run_id, from_seq = query.from_seq, to_seq = ?query.to_seq)
    )]
    /// Returns a range of persisted events for the requested run.
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
            warn!("run not found while reading events");
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
        debug!(
            event_count = events.len(),
            head_seq = head,
            "run events loaded"
        );

        Ok(RunsEventsResponse {
            run_id: run_id.0.to_string(),
            head_seq: head,
            events,
        })
    }

    #[instrument(level = "debug", skip(self), fields(artifact_id = artifact_id))]
    /// Loads an artifact by content address and returns a transport-friendly body.
    pub async fn artifact_get(&self, artifact_id: &str) -> Result<ArtifactGetResponse, AppError> {
        get_artifact_from_store(Arc::clone(&self.artifacts), artifact_id).await
    }

    /// Starts the standard deploy-configure-validate pipeline template.
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

    /// Starts the deploy-configure-validate template from raw JSON or a JSON file.
    pub async fn start_deploy_configure_validate_from_spec_input(
        &self,
        spec_json: Option<String>,
        spec_file: Option<PathBuf>,
    ) -> Result<RunStartResponse, AppError> {
        let raw_spec = match (spec_json, spec_file) {
            (Some(_), Some(_)) => {
                return Err(AppError::new(
                    ErrorClass::BadRequest,
                    "InvalidArguments",
                    "Pass only one of --spec-json or --spec-file",
                ));
            }
            (None, None) => {
                return Err(AppError::new(
                    ErrorClass::BadRequest,
                    "MissingArgument",
                    "Pass one of --spec-json or --spec-file",
                ));
            }
            (Some(s), None) => s,
            (None, Some(path)) => std::fs::read_to_string(path).map_err(|_| {
                AppError::new(
                    ErrorClass::BadRequest,
                    "InvalidSpecFile",
                    "Failed to read --spec-file contents",
                )
            })?,
        };

        let spec: DeployConfigureValidateSpec = serde_json::from_str(&raw_spec).map_err(|_| {
            AppError::new(
                ErrorClass::BadRequest,
                "InvalidJson",
                "Failed to parse deploy/configure/validate spec JSON",
            )
        })?;
        self.start_deploy_configure_validate(spec).await
    }

    /// Starts a portfolio snapshot run and extracts the final report when available.
    pub async fn start_portfolio_snapshot(
        &self,
        req: PortfolioSnapshotRequest,
    ) -> Result<PortfolioSnapshotResponse, AppError> {
        // Fail fast for the higher-level feature surface when RPC is not configured.
        //
        // Note: the underlying op can still be started via `run.start` and may end in `phase=failed`,
        // but the feature is intended to behave like a request-level RPC dependency.
        if resolve_evm_rpc_sources_from_env().is_empty() {
            return Err(AppError::new(
                ErrorClass::BadGateway,
                "evm_rpc_url_missing",
                "evm rpc url is not configured",
            ));
        }

        const OP_ID: &str = "portfolio_tracker";
        const OP_VERSION: &str = "v1";

        let mut req = req;
        let mut tokens = load_portfolio_tokens_from_env()?;
        tokens.extend(req.tokens);
        req.tokens = tokens;

        let op_config = serde_json::to_value(&req).map_err(|_| {
            AppError::invalid_request("failed to encode portfolio snapshot request")
        })?;

        let run = self
            .start_run(RunsStartRequest::Single(SingleOpStartRequest {
                op_id: OP_ID.to_string(),
                op_version: OP_VERSION.to_string(),
                op_config,
            }))
            .await?;

        let mut snapshot_artifact_id = None;
        let mut chain_id = None;
        let mut block_number = None;
        let mut native_balance = None;

        if let Some(final_snapshot_id) = &run.final_snapshot_id {
            let report_key = portfolio_tracker_report_context_key();
            let bytes = self
                .artifacts
                .get(&ArtifactId(final_snapshot_id.clone()))
                .await
                .map_err(app_error_from_storage_error)?;

            let v = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
                AppError::new(
                    ErrorClass::Internal,
                    "ContextSnapshotDecodeFailed",
                    "failed to decode context snapshot json",
                )
            })?;

            if let Some(report_value) = v.get(&report_key.0).cloned() {
                let report: PortfolioTrackerReport =
                    serde_json::from_value(report_value).map_err(|_| {
                        AppError::new(
                            ErrorClass::Internal,
                            "PortfolioSnapshotReportDecodeFailed",
                            "failed to decode portfolio snapshot report",
                        )
                    })?;
                snapshot_artifact_id = Some(report.snapshot_artifact_id);
                chain_id = Some(report.chain_id);
                block_number = Some(report.block_number);
                native_balance = report.native_balance;
            }
        }

        Ok(PortfolioSnapshotResponse {
            run_id: run.run_id,
            phase: run.phase,
            final_snapshot_id: run.final_snapshot_id,
            snapshot_artifact_id,
            chain_id,
            block_number,
            native_balance,
        })
    }

    /// Starts a portfolio snapshot run using a CLI-style `--tokens-json` string.
    pub async fn start_portfolio_snapshot_from_tokens_json(
        &self,
        address: String,
        chain_id: u64,
        tokens_json: String,
    ) -> Result<PortfolioSnapshotResponse, AppError> {
        let tokens = parse_portfolio_tokens_json(&tokens_json)?;

        self.start_portfolio_snapshot(PortfolioSnapshotRequest {
            address,
            chain_id: Some(chain_id),
            tokens,
        })
        .await
    }
}

/// Loads an artifact from the supplied store and returns a JSON-or-hex response body.
pub async fn get_artifact_from_store(
    artifacts: Arc<dyn ArtifactStore>,
    artifact_id: &str,
) -> Result<ArtifactGetResponse, AppError> {
    let id = ArtifactId(artifact_id.to_string());
    debug!(artifact_id = %id.0, "loading artifact from store");

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
/// Start-run request accepted by app-facing transports.
pub enum RunsStartRequest {
    /// Starts a run by wrapping a single operation into the one-step pipeline convention.
    Single(SingleOpStartRequest),
    /// Starts a run from an explicit pipeline payload.
    Pipeline(PipelineStartRequest),
}

/// Request payload for starting a single operation run.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SingleOpStartRequest {
    /// Operation identifier, defaulting to `proof`.
    #[serde(default = "default_op_id")]
    pub op_id: String,

    /// Operation version, defaulting to `v1`.
    #[serde(default = "default_op_version")]
    pub op_version: String,

    /// Canonical JSON config passed to the operation.
    #[serde(default = "default_empty_object")]
    pub op_config: serde_json::Value,
}

/// Request payload for starting an explicit pipeline run.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PipelineStartRequest {
    /// Pipeline template to launch.
    pub pipeline: Pipeline,

    /// Canonical JSON input embedded in the manifest.
    #[serde(default = "default_empty_object")]
    pub input: serde_json::Value,

    /// Optional run configuration override.
    #[serde(default)]
    pub run_config: Option<RunConfig>,
}

/// Response returned after starting a new run.
#[derive(Clone, Debug, Serialize)]
pub struct RunStartResponse {
    /// UUID string of the started run.
    pub run_id: String,
    /// Current run phase string.
    pub phase: String,
    /// Final snapshot id if the run finished immediately.
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

/// Response returned after resuming a run.
#[derive(Clone, Debug, Serialize)]
pub struct RunResumeResponse {
    /// UUID string of the resumed run.
    pub run_id: String,
    /// Current run phase string.
    pub phase: String,
    /// Final snapshot id if the run completed.
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

/// Current status projection for a run.
#[derive(Clone, Debug, Serialize)]
pub struct RunStatusResponse {
    /// UUID string of the run.
    pub run_id: String,
    /// Current head sequence in the run event stream.
    pub head_seq: u64,
    /// Operation id recorded at run start, when available.
    pub op_id: Option<String>,
    /// Manifest artifact id recorded at run start, when available.
    pub manifest_id: Option<String>,
    /// Current phase string.
    pub phase: String,
    /// Final snapshot id when the run has completed.
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
/// Query parameters for fetching a run event range.
pub struct RunsEventsQuery {
    /// First sequence number to include, defaulting to `1`.
    #[serde(default = "default_from_seq")]
    pub from_seq: u64,

    /// Optional inclusive upper bound for the event range.
    pub to_seq: Option<u64>,
}

/// Response returned by the run-events query.
#[derive(Clone, Debug, Serialize)]
pub struct RunsEventsResponse {
    /// UUID string of the run.
    pub run_id: String,
    /// Current head sequence in the run event stream.
    pub head_seq: u64,
    /// Events in the requested range.
    pub events: Vec<EventEnvelope>,
}

impl fmt::Display for RunsEventsResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string_pretty(&self.events).unwrap_or_else(|_| "[]".to_string());
        write!(f, "{s}")
    }
}

/// Artifact fetch response returned by app-facing transports.
#[derive(Debug, Clone, Serialize)]
pub struct ArtifactGetResponse {
    /// Content-addressed artifact identifier.
    pub artifact_id: String,
    /// Decoded body representation.
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
/// Artifact body encoded either as structured JSON or hex bytes.
pub enum ArtifactBody {
    /// Artifact body successfully decoded as JSON.
    Json {
        /// Structured JSON value decoded from the artifact bytes.
        value: serde_json::Value,
    },
    /// Artifact body returned as lowercase hex.
    Hex {
        /// Lowercase hex encoding of the raw artifact bytes.
        hex: String,
    },
}

/// Input payload for the standard deploy-configure-validate pipeline feature.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployConfigureValidateSpec {
    /// Machine id to assign to the generated pipeline.
    #[serde(default = "default_machine_id")]
    pub machine_id: String,

    /// Version string to assign to the generated pipeline.
    #[serde(default = "default_pipeline_version")]
    pub pipeline_version: String,

    /// Additional pipeline input forwarded into the run manifest.
    #[serde(default = "default_empty_object")]
    pub input: serde_json::Value,

    /// Operation config for the deploy phase.
    pub deploy: serde_json::Value,
    /// Operation config for the configure phase.
    pub configure: serde_json::Value,
    /// Operation config for the validate phase.
    pub validate: serde_json::Value,
}

/// Converts a deploy-configure-validate spec into the canonical single-step pipeline template.
pub fn pipeline_from_deploy_configure_validate_spec(spec: DeployConfigureValidateSpec) -> Pipeline {
    Pipeline {
        machine_id: MachineId(spec.machine_id),
        pipeline_version: spec.pipeline_version,
        steps: vec![PipelineStep {
            step_id: StepId("main".to_string()),
            op_id: OpId::must_new(EVM_DEPLOY_CONFIGURE_VALIDATE_OP_ID.to_string()),
            op_version: EVM_DEPLOY_CONFIGURE_VALIDATE_OP_VERSION.to_string(),
            op_config: serde_json::json!({
                "deploy": spec.deploy,
                "configure": spec.configure,
                "validate": spec.validate,
            }),
        }],
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
/// Category of a built-in application feature.
pub enum FeatureKind {
    /// Feature that directly starts an operation.
    Operation,
    /// Feature that expands into a reusable pipeline template.
    PipelineTemplate,
    /// Feature that controls or inspects existing runs.
    RunControl,
    /// Feature that fetches artifacts.
    Artifact,
}

/// Metadata describing a built-in feature surface.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureDescriptor {
    /// Stable feature identifier.
    pub id: String,
    /// Feature version string.
    pub version: String,
    /// Feature category.
    pub kind: FeatureKind,
    /// Human-readable feature summary.
    pub description: String,
    /// JSON Schema-like input description.
    pub input_schema: serde_json::Value,
    /// JSON Schema-like output description.
    pub output_schema: serde_json::Value,
}

/// Request payload for executing a built-in feature.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureRequest {
    /// Stable feature identifier to execute.
    pub feature_id: String,
    /// Canonical JSON payload passed to that feature.
    #[serde(default = "default_empty_object")]
    pub payload: serde_json::Value,
}

/// Result returned after a feature executes successfully.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureExecutionResult {
    /// Executed feature identifier.
    pub feature_id: String,
    /// Feature-specific result payload.
    pub result: serde_json::Value,
}

impl fmt::Display for FeatureExecutionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "feature_id: {}", self.feature_id)?;
        writeln!(f, "result:")?;
        let s =
            serde_json::to_string_pretty(&self.result).unwrap_or_else(|_| self.result.to_string());
        write!(f, "{s}")
    }
}

/// Registry of built-in feature descriptors and dispatch handlers.
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
    PortfolioSnapshot,
}

impl FeatureCatalog {
    /// Builds the default catalog used by the CLI and REST API.
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
        handlers.insert(
            "portfolio.snapshot".to_string(),
            BuiltinFeature::PortfolioSnapshot,
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
            FeatureDescriptor {
                id: "portfolio.snapshot".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::Operation,
                description: "Start a portfolio snapshot run from a single wallet address (default chain_id: 1)".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "address": {"type": "string"},
                        "chain_id": {"type": "integer"},
                        "tokens": {"type": "array"}
                    },
                    "required": ["address"]
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "run_id": {"type": "string"},
                        "phase": {"type": "string"},
                        "final_snapshot_id": {"type": ["string", "null"]},
                        "snapshot_artifact_id": {"type": ["string", "null"]},
                        "chain_id": {"type": ["integer", "null"]},
                        "block_number": {"type": ["integer", "null"]},
                        "native_balance": {
                            "type": ["object", "null"],
                            "properties": {
                                "symbol": {"type": "string"},
                                "raw_u256_dec": {"type": "string"},
                                "decimals": {"type": "integer"},
                                "amount_dec": {"type": "string"}
                            },
                            "required": ["symbol", "raw_u256_dec", "decimals", "amount_dec"]
                        }
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

    /// Returns the static descriptors for all registered built-in features.
    pub fn descriptors(&self) -> &[FeatureDescriptor] {
        &self.descriptors
    }

    /// Executes a built-in feature request against the provided services.
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
            BuiltinFeature::PortfolioSnapshot => {
                let parsed: PortfolioSnapshotRequest =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                serde_json::to_value(services.start_portfolio_snapshot(parsed).await?)
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

#[derive(Clone, Debug, Deserialize, Serialize)]
/// Request payload for the portfolio snapshot feature.
pub struct PortfolioSnapshotRequest {
    /// Wallet address to inspect.
    pub address: String,
    /// Optional chain id override. Defaults are handled by the feature itself.
    pub chain_id: Option<u64>,
    /// Additional token descriptors to include in the snapshot.
    #[serde(default)]
    pub tokens: Vec<PortfolioTokenSpec>,
}

/// Parses the CLI/REST `tokens_json` string into token descriptors.
pub fn parse_portfolio_tokens_json(tokens_json: &str) -> Result<Vec<PortfolioTokenSpec>, AppError> {
    let tokens_value: serde_json::Value = serde_json::from_str(tokens_json).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "InvalidJson",
            "Failed to parse --tokens-json as JSON",
        )
    })?;
    if !tokens_value.is_array() {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "InvalidJson",
            "--tokens-json must be a JSON array",
        ));
    }

    serde_json::from_value(tokens_value).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "InvalidJson",
            "--tokens-json entries must be valid token objects",
        )
    })
}

fn normalize_eth_address(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if rest.len() != 40 {
        return None;
    }
    if !rest.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", rest.to_ascii_lowercase()))
}

fn parse_portfolio_tokens_from_env(raw: &str) -> Result<Vec<PortfolioTokenSpec>, AppError> {
    let tokens_value: serde_json::Value = serde_json::from_str(raw).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "InvalidPortfolioTokensJson",
            format!("invalid {ENV_PORTFOLIO_TOKENS_JSON}"),
        )
    })?;

    if !tokens_value.is_array() {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "InvalidPortfolioTokensJson",
            format!("invalid {ENV_PORTFOLIO_TOKENS_JSON}"),
        ));
    }

    let tokens: Vec<PortfolioTokenSpec> = serde_json::from_value(tokens_value).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "InvalidPortfolioTokensJson",
            format!("invalid {ENV_PORTFOLIO_TOKENS_JSON}"),
        )
    })?;

    if tokens
        .iter()
        .any(|token| normalize_eth_address(&token.address).is_none())
    {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "InvalidPortfolioTokensJson",
            format!("invalid token address in {ENV_PORTFOLIO_TOKENS_JSON}"),
        ));
    }

    Ok(tokens)
}

fn load_portfolio_tokens_from_env() -> Result<Vec<PortfolioTokenSpec>, AppError> {
    let Ok(raw) = std::env::var(ENV_PORTFOLIO_TOKENS_JSON) else {
        return Ok(Vec::new());
    };
    parse_portfolio_tokens_from_env(&raw)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
/// Token descriptor used by the portfolio snapshot feature.
pub struct PortfolioTokenSpec {
    /// ERC-20 token contract address.
    pub address: String,
    /// Optional symbol hint for output formatting.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Optional decimals hint for output formatting.
    #[serde(default)]
    pub decimals: Option<u8>,
}

/// Response returned after starting a portfolio snapshot feature run.
#[derive(Clone, Debug, Serialize)]
pub struct PortfolioSnapshotResponse {
    /// UUID string of the run.
    pub run_id: String,
    /// Current run phase string.
    pub phase: String,
    /// Final snapshot id when the run completed.
    pub final_snapshot_id: Option<String>,
    /// Report artifact id containing the portfolio snapshot, when available.
    pub snapshot_artifact_id: Option<String>,
    /// Resolved chain id from the final report.
    pub chain_id: Option<u64>,
    /// Resolved block number from the final report.
    pub block_number: Option<u64>,
    /// Native balance from the final report, when present.
    pub native_balance: Option<PortfolioBalanceReport>,
}
