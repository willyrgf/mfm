#![warn(missing_docs)]
//! Legacy dynamic application-facing orchestration bridge for MFM binaries and transports.
//!
//! `mfm-app-legacy` wires together the old operation registry, transport registry, storage
//! defaults, and higher-level request/response helpers used by the pre-typed CLI and REST API. It
//! is intentionally isolated from the typed `mfm-app` crate and is not a certified typed
//! submit/resume surface.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_app::{
//!     make_default_artifact_store, make_default_stream_store, make_engine_bundle, AppServices,
//! };
//!
//! async fn boot() -> Result<AppServices, mfm_app::AppError> {
//!     let bundle = make_engine_bundle();
//!     let streams = make_default_stream_store().await?;
//!     let artifacts = make_default_artifact_store().await?;
//!     Ok(AppServices::new(bundle, streams, artifacts))
//! }
//! ```
/// Shared observability configuration used by the CLI and REST API.
pub mod observability;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_artifact_store_secret::{is_secret_payload_envelope, SecretArtifactStore, SecretKey};
use mfm_collectors_nix_exec::NixFlakeTransportFactory;
use mfm_evm_deploy_configure_validate_config::{
    canonicalize_deploy_configure_validate_authored_config,
    parse_deploy_configure_validate_authored_config,
    parse_deploy_configure_validate_authored_config_with_hint,
    AuthoredConfigFormat as DeployConfigureValidateAuthoredConfigFormat,
    DeployConfigureValidateConfigError,
};
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
use mfm_machine::errors::{ContextError, ErrorCategory, IoError, RunError, StorageError};
use mfm_machine::events::{
    event_envelopes_from_stream_records, Event, EventEnvelope, KernelEvent, RunStatus,
};
use mfm_machine::ids::{ArtifactId, ContextKey, OpId, RunId};
use mfm_machine::live_io::LiveIoTransportFactory;
use mfm_machine::live_io_registry::{HashMapTransportRegistry, TransportRegistry};
use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
use mfm_machine::runtime::{ChildRunLiveIoTransportFactory, DefaultExecutionEngine, PlanResolver};
use mfm_machine::stores::{ArtifactStore, StreamId, StreamRecord, StreamStore};
use mfm_op_evm_read::EvmReadOp;
use mfm_op_evm_write::{
    EvmConfigureOp, EvmContractFromNixOp, EvmDeployContractSetOp, EvmDeployOp, EvmValidateOp,
};
use mfm_op_keystore::{
    keystore_delete_report_context_key, keystore_import_report_context_key,
    keystore_list_report_context_key, tx_sign_report_context_key, KeystoreDeleteOp,
    KeystoreDeleteOpConfig, KeystoreDeleteReport as OpKeystoreDeleteReport, KeystoreImportOp,
    KeystoreImportOpConfig, KeystoreImportReport as OpKeystoreImportReport,
    KeystoreImportType as OpKeystoreImportType, KeystoreListOp, KeystoreListOpConfig,
    KeystoreListReport as OpKeystoreListReport, KeystoreListSortBy as OpKeystoreListSortBy,
    KeystoreTxSignOp, LocalFileWriteMode as OpLocalFileWriteMode, TxSignOpConfig,
    TxSignReport as OpTxSignReport, KEYSTORE_ADMIN_OP_VERSION, KEYSTORE_DELETE_OP_ID,
    KEYSTORE_IMPORT_OP_ID, KEYSTORE_LIST_OP_ID, TX_OP_VERSION, TX_SIGN_OP_ID,
};
use mfm_op_nix_app::NixAppOp;
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::op::OperationRegistry;
use mfm_sdk::pipeline::{Pipeline, PipelinePlanner};
use mfm_sdk::unstable::{
    execute_single_op_report, single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher,
    HashMapOperationRegistry, SdkPlanResolver, SingleOpReportError, SingleOpReportRequest,
};
use mfm_stream_store_postgres::PostgresStreamStore;
use mfm_transports_exec::ExecProgramTransportFactory;
use mfm_transports_local_evm::LocalEvmIoTransportFactory;
use mfm_transports_local_fs::LocalFsIoTransportFactory;
use mfm_transports_local_keystore::{
    register_local_keystore_resource, LocalKeystoreBip39ExtraSource, LocalKeystoreDeleteResource,
    LocalKeystoreImportResource, LocalKeystoreIoTransportFactory, LocalKeystoreListResource,
    LocalKeystoreResource, LocalKeystoreTxSignResource,
};
use mfm_transports_rpc_control::{RpcControlConfigError, RpcControlTransportFactory};

const ENV_ARTIFACT_BACKEND: &str = "MFM_ARTIFACT_BACKEND";
const ENV_ARTIFACT_ROOT: &str = "MFM_ARTIFACT_ROOT";
const ENV_DATABASE_URL: &str = "DATABASE_URL";
const ENV_S3_ENSURE_BUCKET: &str = "MFM_S3_ENSURE_BUCKET";

fn ensure_single_start_op(
    registry: &dyn OperationRegistry,
    op_id: &OpId,
    op_version: &str,
) -> Result<(), AppError> {
    registry
        .resolve(op_id, op_version)
        .map_err(|err| {
            AppError::new(
                ErrorClass::BadRequest,
                err.info.code.as_str(),
                err.info.message,
            )
        })
        .map(|_| ())
}

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
            AppError::new(ErrorClass::Conflict, info.code.as_str(), info.message)
        }
        StorageError::NotFound(info) => {
            AppError::new(ErrorClass::NotFound, info.code.as_str(), info.message)
        }
        StorageError::Corruption(info) | StorageError::Other(info) => {
            AppError::new(ErrorClass::Internal, info.code.as_str(), info.message)
        }
    }
}

fn run_stream_id(run_id: RunId) -> StreamId {
    StreamId::run(run_id)
}

async fn run_stream_head(store: &dyn StreamStore, run_id: RunId) -> Result<u64, AppError> {
    store
        .head_seq(&run_stream_id(run_id))
        .await
        .map_err(app_error_from_storage_error)
}

async fn read_run_stream_records(
    store: &dyn StreamStore,
    run_id: RunId,
    from_seq: u64,
    to_seq: Option<u64>,
) -> Result<Vec<StreamRecord>, AppError> {
    store
        .read_range(&run_stream_id(run_id), from_seq, to_seq)
        .await
        .map_err(app_error_from_storage_error)
}

async fn read_run_stream_events(
    store: &dyn StreamStore,
    run_id: RunId,
    from_seq: u64,
    to_seq: Option<u64>,
) -> Result<Vec<EventEnvelope>, AppError> {
    let records = read_run_stream_records(store, run_id, from_seq, to_seq).await?;
    event_envelopes_from_stream_records(run_id, records).map_err(app_error_from_storage_error)
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

    let class = if info.code.as_str().starts_with("rpc_control_") {
        ErrorClass::BadGateway
    } else {
        match info.category {
            ErrorCategory::ParsingInput => ErrorClass::BadRequest,
            ErrorCategory::OnChain | ErrorCategory::OffChain | ErrorCategory::Rpc => {
                ErrorClass::BadGateway
            }
            ErrorCategory::Storage | ErrorCategory::Context | ErrorCategory::Unknown => {
                ErrorClass::Internal
            }
        }
    };

    AppError::new(class, info.code.as_str(), info.message)
}

fn app_error_from_single_op_report_error(err: SingleOpReportError) -> AppError {
    let class = match err.code.as_str() {
        "MissingFinalSnapshot"
        | "MissingReport"
        | "InvalidReport"
        | "InvalidSnapshot"
        | "RunFailed" => ErrorClass::Internal,
        _ => ErrorClass::BadRequest,
    };

    AppError::new(class, err.code, err.message)
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
#[allow(clippy::disallowed_methods)]
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
#[allow(clippy::disallowed_methods)]
pub async fn make_default_artifact_store() -> Result<Arc<dyn ArtifactStore>, AppError> {
    let backend = std::env::var(ENV_ARTIFACT_BACKEND).unwrap_or_else(|_| "fs".to_string());
    info!(backend = %backend, "initializing artifact store");
    let store: Arc<dyn ArtifactStore> = match backend.as_str() {
        "fs" => {
            let root = default_artifact_root();
            info!(artifact_root = %root.display(), "using filesystem artifact store");
            Arc::new(FsArtifactStore::new(root))
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
            Arc::new(store)
        }
        other => {
            return Err(AppError::new(
                ErrorClass::Internal,
                "InvalidArtifactBackend",
                format!("invalid {ENV_ARTIFACT_BACKEND}: {other}"),
            ))
        }
    };

    wrap_protected_artifact_store_if_configured(store)
}

/// Wraps an artifact store with protected secret/capability storage when configured.
///
/// If `MFM_SECRET_KEY_HEX` is unset, the original public artifact store is returned unchanged and
/// protected writes will fail closed through the [`ArtifactStore`] default methods.
pub fn wrap_protected_artifact_store_if_configured(
    store: Arc<dyn ArtifactStore>,
) -> Result<Arc<dyn ArtifactStore>, AppError> {
    match SecretKey::from_optional_env().map_err(app_error_from_storage_error)? {
        Some(key) => Ok(Arc::new(SecretArtifactStore::new(store, key))),
        None => Ok(store),
    }
}

/// Builds the default stream store from environment configuration.
#[instrument(level = "info", skip_all)]
#[allow(clippy::disallowed_methods)]
pub async fn make_default_stream_store() -> Result<Arc<dyn StreamStore>, AppError> {
    let database_url = std::env::var(ENV_DATABASE_URL).map_err(|_| {
        AppError::new(
            ErrorClass::Internal,
            "MissingDatabaseUrl",
            format!("Missing {ENV_DATABASE_URL}"),
        )
    })?;

    info!(
        database_url_set = true,
        "initializing postgres stream store"
    );
    let store = PostgresStreamStore::connect(&database_url)
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
        registry.register(Arc::new(KeystoreImportOp));
        registry.register(Arc::new(KeystoreListOp));
        registry.register(Arc::new(KeystoreDeleteOp));
        registry.register(Arc::new(KeystoreTxSignOp));
        registry.register(Arc::new(EvmReadOp));
        registry.register(Arc::new(EvmContractFromNixOp));
        registry.register(Arc::new(EvmDeployContractSetOp));
        registry.register(Arc::new(EvmDeployOp));
        registry.register(Arc::new(EvmConfigureOp));
        registry.register(Arc::new(EvmValidateOp));
        registry.register(Arc::new(NixAppOp));
    }
}

/// Default transport plugin that installs the built-in transport catalog.
#[derive(Clone, Default)]
pub struct DefaultTransportPlugin;

impl TransportPlugin for DefaultTransportPlugin {
    fn register_transports(&self, registry: &mut HashMapTransportRegistry) -> Result<(), AppError> {
        register_transport_factory(registry, Arc::new(ExecProgramTransportFactory::default()))?;
        register_transport_factory(registry, Arc::new(NixFlakeTransportFactory::from_env()))?;
        register_transport_factory(registry, Arc::new(LocalFsIoTransportFactory))?;
        register_transport_factory(registry, Arc::new(LocalEvmIoTransportFactory))?;
        register_transport_factory(registry, Arc::new(LocalKeystoreIoTransportFactory))?;
        let rpc_control_factory = RpcControlTransportFactory::from_env()
            .map_err(app_error_from_rpc_control_config_error)?;
        register_transport_factory(registry, Arc::new(rpc_control_factory))?;
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

fn app_error_from_rpc_control_config_error(err: RpcControlConfigError) -> AppError {
    AppError::new(
        ErrorClass::Internal,
        "RpcControlTransportConfigInvalid",
        err.to_string(),
    )
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
    /// Stream store used for run status and run-stream queries.
    pub streams: Arc<dyn StreamStore>,
    /// Artifact store used for snapshots, facts, and outputs.
    pub artifacts: Arc<dyn ArtifactStore>,
}

impl AppServices {
    /// Creates a new service facade from the supplied engine bundle and stores.
    pub fn new(
        bundle: EngineBundle,
        streams: Arc<dyn StreamStore>,
        artifacts: Arc<dyn ArtifactStore>,
    ) -> Self {
        Self {
            bundle,
            streams,
            artifacts,
        }
    }

    fn stores(&self) -> Stores {
        Stores {
            streams: Arc::clone(&self.streams),
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
            RunsStartRequest::SingleOp {
                op_id,
                op_version,
                op_config,
                input,
            } => {
                tracing::Span::current().record("request_kind", "single");
                tracing::Span::current().record("op_id", op_id.as_str());
                let op_id = OpId::new(op_id).map_err(|_| {
                    AppError::new(
                        ErrorClass::BadRequest,
                        "invalid_op_id",
                        "op_id must match ^[a-z][a-z0-9_]{0,62}$",
                    )
                })?;
                ensure_single_start_op(self.bundle.registry.as_ref(), &op_id, op_version.as_str())?;
                let pipeline = single_op_pipeline(op_id, op_version, op_config).map_err(|e| {
                    AppError::new(ErrorClass::BadRequest, e.info.code.as_str(), e.info.message)
                })?;
                (pipeline, input, default_run_config())
            }
            RunsStartRequest::Pipeline {
                pipeline,
                input,
                run_config,
            } => {
                tracing::Span::current().record("request_kind", "pipeline");
                tracing::Span::current().record("machine_id", pipeline.machine_id.0.as_str());
                for step in &pipeline.steps {
                    ensure_single_start_op(
                        self.bundle.registry.as_ref(),
                        &step.op_id,
                        step.op_version.as_str(),
                    )?;
                }
                (
                    pipeline,
                    input,
                    run_config.unwrap_or_else(default_run_config),
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
            final_snapshot_id: run.final_snapshot_id.map(|id| id.into_string()),
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
            final_snapshot_id: run.final_snapshot_id.map(|id| id.into_string()),
        })
    }

    #[instrument(level = "debug", skip(self), fields(run_id = run_id))]
    /// Returns the current run status by scanning the persisted run stream.
    pub async fn run_status(&self, run_id: &str) -> Result<RunStatusResponse, AppError> {
        let uuid = uuid::Uuid::parse_str(run_id).map_err(|_| AppError::invalid_uuid())?;
        let run_id = RunId(uuid);

        let head = run_stream_head(self.streams.as_ref(), run_id).await?;
        if head == 0 {
            warn!("run not found while reading status");
            return Err(AppError::not_found(
                "run_not_found",
                "run stream was not found",
            ));
        }

        let stream = read_run_stream_events(self.streams.as_ref(), run_id, 1, None).await?;
        debug!(event_count = stream.len(), "loaded run stream events");

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
                    manifest_id = Some(mid.as_str().to_string());
                }
                KernelEvent::RunCompleted {
                    status,
                    final_snapshot_id,
                } => {
                    completed = Some((
                        status.clone(),
                        final_snapshot_id.as_ref().map(|id| id.as_str().to_string()),
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
    /// Returns a range of persisted records for the requested run stream.
    pub async fn run_stream(
        &self,
        run_id: &str,
        query: RunsStreamQuery,
    ) -> Result<RunsStreamResponse, AppError> {
        let uuid = uuid::Uuid::parse_str(run_id).map_err(|_| AppError::invalid_uuid())?;
        let run_id = RunId(uuid);

        let head = run_stream_head(self.streams.as_ref(), run_id).await?;
        if head == 0 {
            warn!("run not found while reading stream");
            return Err(AppError::not_found(
                "run_not_found",
                "run stream was not found",
            ));
        }

        let records =
            read_run_stream_records(self.streams.as_ref(), run_id, query.from_seq, query.to_seq)
                .await?;
        debug!(
            record_count = records.len(),
            head_seq = head,
            "run stream loaded"
        );

        Ok(RunsStreamResponse {
            run_id: run_id.0.to_string(),
            head_seq: head,
            records,
        })
    }

    #[instrument(level = "debug", skip(self), fields(artifact_id = artifact_id))]
    /// Loads an artifact by content address and returns a transport-friendly body.
    pub async fn artifact_get(&self, artifact_id: &str) -> Result<ArtifactGetResponse, AppError> {
        let id = parse_artifact_id(artifact_id)?;
        get_artifact_from_store(Arc::clone(&self.artifacts), &id).await
    }

    async fn run_single_op_report<T>(
        &self,
        op_id: &'static str,
        op_version: &'static str,
        op_name: &'static str,
        report_key: ContextKey,
        op_config: impl Serialize,
    ) -> Result<T, AppError>
    where
        T: serde::de::DeserializeOwned,
    {
        let op_config = serde_json::to_value(op_config).map_err(|_| {
            AppError::new(
                ErrorClass::Internal,
                "SerializationError",
                format!("failed to serialize {op_name} op config"),
            )
        })?;

        execute_single_op_report(
            Arc::clone(&self.bundle.engine),
            self.stores(),
            Arc::clone(&self.bundle.registry),
            Arc::clone(&self.bundle.planner),
            SingleOpReportRequest {
                op_id: op_id.to_string(),
                op_version: op_version.to_string(),
                op_config,
                report_context_key: report_key.0,
            },
        )
        .await
        .map_err(app_error_from_single_op_report_error)
    }

    /// Imports key material through the app-owned keystore operation boundary.
    pub async fn keystore_import(
        &self,
        req: KeystoreImportRequest,
    ) -> Result<KeystoreImportResponse, AppError> {
        let local_resource_handle = local_keystore_resource_handle();
        let bip39_extra = match req.bip39_extra {
            KeystoreBip39ExtraSource::None => LocalKeystoreBip39ExtraSource::None,
            KeystoreBip39ExtraSource::Prompt => LocalKeystoreBip39ExtraSource::Prompt,
            KeystoreBip39ExtraSource::FilePath(path) => {
                LocalKeystoreBip39ExtraSource::FilePath(path)
            }
        };
        register_local_keystore_resource(
            local_resource_handle.clone(),
            LocalKeystoreResource::Import(LocalKeystoreImportResource {
                keystore_path: req.keystore_path,
                label: req.label,
                stdin_mode: req.stdin,
                bip39_extra,
            }),
        );

        let op_config = KeystoreImportOpConfig {
            import_type: match req.import_type {
                KeystoreImportType::PrivateKey => OpKeystoreImportType::PrivateKey,
                KeystoreImportType::Mnemonic => OpKeystoreImportType::Mnemonic,
            },
            derivation_path: req.derivation_path,
            local_resource_handle,
        };

        let report: OpKeystoreImportReport = self
            .run_single_op_report(
                KEYSTORE_IMPORT_OP_ID,
                KEYSTORE_ADMIN_OP_VERSION,
                "keystore import",
                keystore_import_report_context_key(),
                op_config,
            )
            .await?;

        Ok(KeystoreImportResponse {
            id: report.id,
            label: report.label,
            key_type: import_key_type_for_output(&report.key_type),
            address: report.address,
            created_at: report.created_at,
        })
    }

    /// Lists local keystore entries through the app-owned keystore operation boundary.
    pub async fn keystore_list(
        &self,
        req: KeystoreListRequest,
    ) -> Result<KeystoreListResponse, AppError> {
        let local_resource_handle = local_keystore_resource_handle();
        register_local_keystore_resource(
            local_resource_handle.clone(),
            LocalKeystoreResource::List(LocalKeystoreListResource {
                keystore_path: req.keystore_path,
                filter_label: req.filter_label,
            }),
        );

        let op_config = KeystoreListOpConfig {
            local_resource_handle,
            show_addresses: req.show_addresses,
            sort_by: match req.sort_by {
                KeystoreListSortBy::Label => OpKeystoreListSortBy::Label,
                KeystoreListSortBy::Created => OpKeystoreListSortBy::Created,
                KeystoreListSortBy::Type => OpKeystoreListSortBy::Type,
            },
        };

        let report: OpKeystoreListReport = self
            .run_single_op_report(
                KEYSTORE_LIST_OP_ID,
                KEYSTORE_ADMIN_OP_VERSION,
                "keystore list",
                keystore_list_report_context_key(),
                op_config,
            )
            .await?;

        let keys = report
            .keys
            .into_iter()
            .map(|key| KeystoreListEntry {
                id: key.id,
                label: key.label,
                key_type: list_key_type_for_output(&key.key_type),
                address: key.address,
                created: key.created,
            })
            .collect();

        Ok(KeystoreListResponse {
            keys,
            show_addresses: report.show_addresses,
        })
    }

    /// Deletes a key through the app-owned keystore operation boundary.
    pub async fn keystore_delete(
        &self,
        req: KeystoreDeleteRequest,
    ) -> Result<KeystoreDeleteResponse, AppError> {
        let local_resource_handle = local_keystore_resource_handle();
        register_local_keystore_resource(
            local_resource_handle.clone(),
            LocalKeystoreResource::Delete(LocalKeystoreDeleteResource {
                keystore_path: req.keystore_path,
                by_label: req.by_label,
            }),
        );

        let op_config = KeystoreDeleteOpConfig {
            id: req.id,
            yes: req.yes,
            local_resource_handle,
        };

        let report: OpKeystoreDeleteReport = self
            .run_single_op_report(
                KEYSTORE_DELETE_OP_ID,
                KEYSTORE_ADMIN_OP_VERSION,
                "keystore delete",
                keystore_delete_report_context_key(),
                op_config,
            )
            .await?;

        Ok(KeystoreDeleteResponse {
            id: report.id,
            label: report.label,
        })
    }

    /// Signs an EIP-1559 transaction through the app-owned keystore operation boundary.
    pub async fn keystore_tx_sign(
        &self,
        req: KeystoreTxSignRequest,
    ) -> Result<KeystoreTxSignResponse, AppError> {
        let local_resource_handle = local_keystore_resource_handle();
        register_local_keystore_resource(
            local_resource_handle.clone(),
            LocalKeystoreResource::TxSign(LocalKeystoreTxSignResource {
                keystore_path: req.keystore_path,
                by_label: req.by_label,
                out_path: req.out_path,
            }),
        );

        let op_config = TxSignOpConfig {
            id: req.id,
            local_resource_handle,
            to: req.to,
            value_wei: req.value_wei,
            chain_id: req.chain_id,
            nonce: req.nonce,
            max_fee_per_gas: req.max_fee_per_gas,
            max_priority_fee_per_gas: req.max_priority_fee_per_gas,
            gas_limit: req.gas_limit,
            out_write_mode: match req.out_write_mode {
                KeystoreTxOutputWriteMode::CreateNew => OpLocalFileWriteMode::CreateNew,
                KeystoreTxOutputWriteMode::Overwrite => OpLocalFileWriteMode::Overwrite,
            },
            data: req.data,
        };

        let report: OpTxSignReport = self
            .run_single_op_report(
                TX_SIGN_OP_ID,
                TX_OP_VERSION,
                "keystore tx sign",
                tx_sign_report_context_key(),
                op_config,
            )
            .await?;

        Ok(KeystoreTxSignResponse {
            from: report.from,
            to: report.to,
            nonce: report.nonce,
            chain_id: report.chain_id,
            tx_type: report.tx_type,
            payload_hash: report.payload_hash,
        })
    }
}

fn app_error_from_deploy_configure_validate_config_error(
    err: DeployConfigureValidateConfigError,
    format_name: Option<&str>,
) -> AppError {
    match err {
        DeployConfigureValidateConfigError::InvalidJson { .. } => {
            if let Some(format_name) = format_name {
                AppError::new(
                    ErrorClass::BadRequest,
                    "InvalidJson",
                    format!("Failed to parse deploy/configure/validate {format_name}"),
                )
            } else {
                AppError::invalid_json()
            }
        }
        DeployConfigureValidateConfigError::InvalidToml { .. } => AppError::new(
            ErrorClass::BadRequest,
            "InvalidToml",
            "Failed to parse deploy/configure/validate TOML",
        ),
        DeployConfigureValidateConfigError::InvalidJsonText { .. } => AppError::new(
            ErrorClass::BadRequest,
            "DeployConfigureValidateConfigError",
            err.to_string(),
        ),
        DeployConfigureValidateConfigError::Serialize { .. }
        | DeployConfigureValidateConfigError::CanonicalJson { .. } => AppError::new(
            ErrorClass::Internal,
            "DeployConfigureValidateConfigError",
            err.to_string(),
        ),
    }
}

/// Loads an artifact from the supplied store and returns a JSON-or-hex response body.
pub async fn get_artifact_from_store(
    artifacts: Arc<dyn ArtifactStore>,
    id: &ArtifactId,
) -> Result<ArtifactGetResponse, AppError> {
    debug!(artifact_id = %id, "loading artifact from store");

    let bytes = artifacts
        .get(id)
        .await
        .map_err(app_error_from_storage_error)?;
    if is_secret_payload_envelope(&bytes) {
        return Err(AppError::not_found(
            "artifact_not_found",
            "artifact not found",
        ));
    }

    let body = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => ArtifactBody::Json { value },
        Err(_) => ArtifactBody::Hex {
            hex: hex::encode(bytes),
        },
    };

    Ok(ArtifactGetResponse {
        artifact_id: id.as_str().to_string(),
        body,
    })
}

/// Parses a caller-supplied artifact id before any storage lookup.
pub fn parse_artifact_id(artifact_id: &str) -> Result<ArtifactId, AppError> {
    ArtifactId::new(artifact_id).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "InvalidArtifactId",
            "artifact id must be 64 lowercase hex characters",
        )
    })
}

/// Start-run request accepted by app-facing transports.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RunsStartRequest {
    /// Starts a run by wrapping a single operation into the one-step pipeline convention.
    #[serde(rename = "single_op_start_v1")]
    SingleOp {
        /// Operation identifier supplied by the caller.
        op_id: String,
        /// Operation version supplied by the caller.
        op_version: String,
        /// Canonical JSON config passed to the operation.
        #[serde(default = "default_empty_object")]
        op_config: serde_json::Value,
        /// Canonical JSON input embedded in the manifest for the wrapped single-op run.
        #[serde(default = "default_empty_object")]
        input: serde_json::Value,
    },
    /// Starts a run from an explicit pipeline payload.
    #[serde(rename = "pipeline_start_v1")]
    Pipeline {
        /// Pipeline template to launch.
        pipeline: Pipeline,
        /// Canonical JSON input embedded in the manifest.
        #[serde(default = "default_empty_object")]
        input: serde_json::Value,
        /// Optional run configuration override.
        #[serde(default)]
        run_config: Option<RunConfig>,
    },
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
    /// Current head sequence in the run stream.
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
/// Query parameters for fetching a run stream range.
pub struct RunsStreamQuery {
    /// First sequence number to include, defaulting to `1`.
    #[serde(default = "default_from_seq")]
    pub from_seq: u64,

    /// Optional inclusive upper bound for the record range.
    pub to_seq: Option<u64>,
}

/// Response returned by the run-stream query.
#[derive(Clone, Debug, Serialize)]
pub struct RunsStreamResponse {
    /// UUID string of the run.
    pub run_id: String,
    /// Current head sequence in the run stream.
    pub head_seq: u64,
    /// Records in the requested range.
    pub records: Vec<StreamRecord>,
}

impl fmt::Display for RunsStreamResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string_pretty(&self.records).unwrap_or_else(|_| "[]".to_string());
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

/// Format hint for human-authored config bodies accepted by app input adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredConfigFormatHint {
    /// Treat the body as JSON.
    Json,
    /// Treat the body as TOML.
    Toml,
    /// Infer the format from the body contents.
    Infer,
}

impl AuthoredConfigFormatHint {
    /// Returns a format hint from a file extension, falling back to inference.
    pub fn from_path(path: &Path) -> Self {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("json") => Self::Json,
            Some("toml") => Self::Toml,
            _ => Self::Infer,
        }
    }
}

/// Raw authored config body plus a non-domain-specific format hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredConfigInput {
    /// Format hint selected by the transport adapter.
    pub format_hint: AuthoredConfigFormatHint,
    /// Raw config body read by the transport adapter.
    pub body: String,
}

impl AuthoredConfigInput {
    /// Builds an authored config input with an explicit JSON hint.
    pub fn json(body: impl Into<String>) -> Self {
        Self {
            format_hint: AuthoredConfigFormatHint::Json,
            body: body.into(),
        }
    }

    /// Builds an authored config input using a path-derived format hint.
    pub fn with_path_hint(body: impl Into<String>, path: &Path) -> Self {
        Self {
            format_hint: AuthoredConfigFormatHint::from_path(path),
            body: body.into(),
        }
    }
}

/// Import mode accepted by app-level keystore import requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeystoreImportType {
    /// Import a raw EVM private key.
    PrivateKey,
    /// Import a BIP-39 mnemonic phrase.
    Mnemonic,
}

/// Optional BIP-39 extra input source metadata for mnemonic imports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeystoreBip39ExtraSource {
    /// No extra BIP-39 passphrase input is requested.
    None,
    /// Prompt for the extra passphrase through the local keystore transport.
    Prompt,
    /// Read the extra passphrase from a local file or FIFO.
    FilePath(PathBuf),
}

/// App-level request for importing key material into a local keystore.
#[derive(Debug, Clone)]
pub struct KeystoreImportRequest {
    /// Import mode to execute.
    pub import_type: KeystoreImportType,
    /// Optional human-readable key label.
    pub label: Option<String>,
    /// BIP-32 derivation path used for mnemonic imports.
    pub derivation_path: String,
    /// Whether secret key material should be read from stdin.
    pub stdin: bool,
    /// Local keystore path selected by the transport adapter.
    pub keystore_path: PathBuf,
    /// Optional BIP-39 extra input source for mnemonic imports.
    pub bip39_extra: KeystoreBip39ExtraSource,
}

/// App-level response returned after importing key material.
#[derive(Debug, Clone, Serialize)]
pub struct KeystoreImportResponse {
    /// Stable key identifier.
    pub id: String,
    /// Human-readable key label.
    pub label: String,
    /// Normalized key type used by CLI/API output.
    pub key_type: String,
    /// Derived EVM address for the imported key.
    pub address: String,
    /// Creation timestamp string.
    pub created_at: String,
}

/// Sort orders accepted by app-level keystore list requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeystoreListSortBy {
    /// Sort keys lexicographically by label.
    Label,
    /// Sort keys by creation timestamp.
    Created,
    /// Sort keys by stored key type.
    Type,
}

/// App-level request for listing local keystore entries.
#[derive(Debug, Clone)]
pub struct KeystoreListRequest {
    /// Local keystore path selected by the transport adapter.
    pub keystore_path: PathBuf,
    /// Whether derived addresses should be included in output.
    pub show_addresses: bool,
    /// Optional label regex filter.
    pub filter_label: Option<String>,
    /// Sort order for the generated report.
    pub sort_by: KeystoreListSortBy,
}

/// Public key metadata returned by app-level keystore listing.
#[derive(Debug, Clone, Serialize)]
pub struct KeystoreListEntry {
    /// Stable key identifier.
    pub id: String,
    /// Human-readable key label.
    pub label: String,
    /// Normalized key type used by CLI/API output.
    pub key_type: String,
    /// Optional derived EVM address.
    pub address: Option<String>,
    /// Creation timestamp string.
    pub created: String,
}

/// App-level response for local keystore listing.
#[derive(Debug, Clone, Serialize)]
pub struct KeystoreListResponse {
    /// Keys matching the request.
    pub keys: Vec<KeystoreListEntry>,
    /// Whether derived addresses were included in output.
    pub show_addresses: bool,
}

/// App-level request for deleting a key from a local keystore.
#[derive(Debug, Clone)]
pub struct KeystoreDeleteRequest {
    /// Optional exact key identifier to delete.
    pub id: Option<String>,
    /// Optional label selector.
    pub by_label: Option<String>,
    /// Whether destructive confirmation has already been granted.
    pub yes: bool,
    /// Local keystore path selected by the transport adapter.
    pub keystore_path: PathBuf,
}

/// App-level response returned after deleting a key.
#[derive(Debug, Clone, Serialize)]
pub struct KeystoreDeleteResponse {
    /// Stable key identifier that was deleted.
    pub id: String,
    /// Human-readable key label that was deleted.
    pub label: String,
}

/// Output file write policy for app-level keystore transaction signing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeystoreTxOutputWriteMode {
    /// Create a new output file and reject existing paths.
    CreateNew,
    /// Replace an existing regular output file.
    Overwrite,
}

/// App-level request for signing an EIP-1559 transaction with a local keystore key.
#[derive(Debug, Clone)]
pub struct KeystoreTxSignRequest {
    /// Optional exact key identifier.
    pub id: Option<String>,
    /// Optional key label selector.
    pub by_label: Option<String>,
    /// Destination address.
    pub to: String,
    /// Transfer value in wei, encoded as decimal or `0x` quantity.
    pub value_wei: String,
    /// EVM chain id.
    pub chain_id: u64,
    /// Sender nonce.
    pub nonce: u64,
    /// Max fee per gas in wei, encoded as decimal or `0x` quantity.
    pub max_fee_per_gas: String,
    /// Max priority fee per gas in wei, encoded as decimal or `0x` quantity.
    pub max_priority_fee_per_gas: String,
    /// Gas limit.
    pub gas_limit: u64,
    /// Output path for the signed raw transaction.
    pub out_path: PathBuf,
    /// Output file write policy.
    pub out_write_mode: KeystoreTxOutputWriteMode,
    /// Transaction calldata as `0x`-prefixed hex.
    pub data: String,
    /// Local keystore path selected by the transport adapter.
    pub keystore_path: PathBuf,
}

/// App-level response returned after signing and writing an EIP-1559 transaction.
#[derive(Debug, Clone, Serialize)]
pub struct KeystoreTxSignResponse {
    /// Signer address.
    pub from: String,
    /// Destination address.
    pub to: String,
    /// Nonce included in the signed transaction.
    pub nonce: u64,
    /// Chain id included in the signed transaction.
    pub chain_id: u64,
    /// Transaction type label.
    pub tx_type: String,
    /// Hash of the signed transaction payload.
    pub payload_hash: String,
}

/// Input payload for the standard deploy-configure-validate workflow feature.
pub use mfm_evm_deploy_configure_validate_config::DeployConfigureValidateCanonicalConfig as DeployConfigureValidateSpec;

/// Parses and canonicalizes authored deploy/configure/validate config.
pub fn canonicalize_deploy_configure_validate_input(
    input: AuthoredConfigInput,
) -> Result<DeployConfigureValidateSpec, AppError> {
    let authored = match input.format_hint {
        AuthoredConfigFormatHint::Json => parse_deploy_configure_validate_authored_config(
            &input.body,
            DeployConfigureValidateAuthoredConfigFormat::Json,
        )
        .map_err(|err| app_error_from_deploy_configure_validate_config_error(err, Some("JSON")))?,
        AuthoredConfigFormatHint::Toml => parse_deploy_configure_validate_authored_config(
            &input.body,
            DeployConfigureValidateAuthoredConfigFormat::Toml,
        )
        .map_err(|err| app_error_from_deploy_configure_validate_config_error(err, Some("TOML")))?,
        AuthoredConfigFormatHint::Infer => {
            parse_deploy_configure_validate_authored_config_with_hint(&input.body, None)
                .map_err(|err| app_error_from_deploy_configure_validate_config_error(err, None))?
        }
    };

    canonicalize_deploy_configure_validate_authored_config(authored)
        .map_err(|err| app_error_from_deploy_configure_validate_config_error(err, None))
}

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
}

fn local_keystore_resource_handle() -> String {
    format!("local-keystore:{}", uuid::Uuid::new_v4())
}

fn import_key_type_for_output(raw: &str) -> String {
    match raw {
        "raw" => "private key".to_string(),
        "hd_derived" => "hd_derived".to_string(),
        other => other.to_string(),
    }
}

fn list_key_type_for_output(raw: &str) -> String {
    match raw {
        "raw" => "privatekey".to_string(),
        "hd_derived" => "hd_derived".to_string(),
        other => other.to_string(),
    }
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

impl FeatureExecutionResult {
    /// Builds a feature result from any serializable payload.
    pub fn from_serializable(
        feature_id: impl Into<String>,
        result: impl Serialize,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            feature_id: feature_id.into(),
            result: serde_json::to_value(result)?,
        })
    }
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
    RunStream,
    ArtifactGet,
}

impl FeatureCatalog {
    /// Builds the default catalog used by the CLI and REST API.
    pub fn with_builtins() -> Self {
        let mut handlers = HashMap::new();
        handlers.insert("run.start".to_string(), BuiltinFeature::RunStart);
        handlers.insert("run.resume".to_string(), BuiltinFeature::RunResume);
        handlers.insert("run.status".to_string(), BuiltinFeature::RunStatus);
        handlers.insert("run.stream".to_string(), BuiltinFeature::RunStream);
        handlers.insert("artifact.get".to_string(), BuiltinFeature::ArtifactGet);
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
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"const": "single_op_start_v1"},
                                "op_id": {"type": "string"},
                                "op_version": {"type": "string"},
                                "op_config": {"type": "object"},
                                "input": {"type": "object"}
                            },
                            "required": ["kind", "op_id", "op_version"]
                        },
                        {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "kind": {"const": "pipeline_start_v1"},
                                "pipeline": {"type": "object"},
                                "input": {"type": "object"},
                                "run_config": {"type": "object"}
                            },
                            "required": ["kind", "pipeline"]
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
                id: "run.stream".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::RunControl,
                description: "Read run stream records in a sequence range".to_string(),
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
                        "records": {"type": "array"}
                    },
                    "required": ["run_id", "head_seq", "records"]
                }),
            },
            FeatureDescriptor {
                id: "artifact.get".to_string(),
                version: "v1".to_string(),
                kind: FeatureKind::Artifact,
                description: "Fetch an artifact by id".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "artifact_id": {
                            "type": "string",
                            "pattern": "^[0-9a-f]{64}$"
                        }
                    },
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
            BuiltinFeature::RunStream => {
                let parsed: RunStreamInput =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                let query = RunsStreamQuery {
                    from_seq: parsed.from_seq.unwrap_or(1),
                    to_seq: parsed.to_seq,
                };
                serde_json::to_value(services.run_stream(&parsed.run_id, query).await?)
            }
            BuiltinFeature::ArtifactGet => {
                let parsed: ArtifactIdInput =
                    serde_json::from_value(req.payload).map_err(|_| AppError::invalid_json())?;
                serde_json::to_value(
                    get_artifact_from_store(Arc::clone(&services.artifacts), &parsed.artifact_id)
                        .await?,
                )
            }
        }
        .map_err(|_| {
            AppError::new(
                ErrorClass::Internal,
                "SerializationError",
                "Failed to serialize feature result",
            )
        })?;

        FeatureExecutionResult::from_serializable(req.feature_id, result).map_err(|_| {
            AppError::new(
                ErrorClass::Internal,
                "SerializationError",
                "Failed to serialize feature result",
            )
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RunIdInput {
    run_id: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ArtifactIdInput {
    artifact_id: ArtifactId,
}

#[derive(Clone, Debug, Deserialize)]
struct RunStreamInput {
    run_id: String,
    from_seq: Option<u64>,
    to_seq: Option<u64>,
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::config::RunConfig;
    use mfm_machine::context::DynContext;
    use mfm_machine::engine::Stores;
    use mfm_machine::errors::{ErrorCategory, IoError, RunError, StateError, StorageError};
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey, OpId, OpPath, RunId, StateId};
    use mfm_machine::io::IoCall;
    use mfm_machine::io::IoProvider;
    use mfm_machine::live_io::{LiveIoEnv, LiveIoTransportFactory};
    use mfm_machine::live_io_registry::{HashMapTransportRegistry, TransportRegistry};
    use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
    use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta};
    use mfm_machine::recorder::EventRecorder;
    use mfm_machine::runtime::{ChildRunLiveIoTransportFactory, PlanResolver};
    use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
    use mfm_machine::stores::{
        AppendBatchResult, ArtifactKind, ArtifactStore, StreamAppend, StreamId, StreamRecord,
        StreamStore,
    };
    use mfm_sdk::errors::SdkError;
    use mfm_sdk::ids::{MachineId, StepId};
    use mfm_sdk::op::{
        leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
    };
    use mfm_sdk::pipeline::PipelineStep;
    use mfm_sdk::unstable::HashMapOperationRegistry;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn app_canonicalizes_deploy_configure_validate_json_and_toml_equally() {
        let json = canonicalize_deploy_configure_validate_input(AuthoredConfigInput::json(
            deploy_configure_validate_json().to_string(),
        ))
        .expect("json dcv config");
        let toml = canonicalize_deploy_configure_validate_input(AuthoredConfigInput {
            format_hint: AuthoredConfigFormatHint::Toml,
            body: deploy_configure_validate_toml().to_string(),
        })
        .expect("toml dcv config");

        assert_eq!(json, toml);
        assert_eq!(json.machine_id, "evm_deploy_configure_validate");
        assert_eq!(json.pipeline_version, "v1");
    }

    #[test]
    fn authored_config_input_uses_path_extension_only_as_format_hint() {
        let input = AuthoredConfigInput::with_path_hint("{}", Path::new("config.toml"));

        assert_eq!(input.format_hint, AuthoredConfigFormatHint::Toml);
        assert_eq!(input.body, "{}");
    }

    #[derive(Clone)]
    struct NoopStreamStore;

    #[async_trait]
    impl StreamStore for NoopStreamStore {
        async fn head_seq(&self, _stream_id: &StreamId) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn append(&self, _append: StreamAppend) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn append_batch(
            &self,
            _appends: Vec<StreamAppend>,
        ) -> Result<AppendBatchResult, StorageError> {
            Ok(AppendBatchResult {
                stream_heads: Vec::new(),
            })
        }

        async fn read_range(
            &self,
            _stream_id: &StreamId,
            _from_seq: u64,
            _to_seq: Option<u64>,
        ) -> Result<Vec<StreamRecord>, StorageError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone)]
    struct NoopArtifactStore;

    #[derive(Clone, Default)]
    struct InMemoryArtifactStore {
        inner: Arc<tokio::sync::Mutex<HashMap<ArtifactId, Vec<u8>>>>,
    }

    #[async_trait]
    impl ArtifactStore for InMemoryArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            let id = mfm_machine::hashing::artifact_id_for_bytes(&bytes);
            let mut inner = self.inner.lock().await;
            inner.insert(id.clone(), bytes);
            Ok(id)
        }

        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            let inner = self.inner.lock().await;
            inner.get(id).cloned().ok_or_else(|| {
                StorageError::NotFound(mfm_machine::errors::ErrorInfo {
                    code: ErrorCode::must_new("artifact_not_found"),
                    category: ErrorCategory::Storage,
                    retryable: false,
                    message: "artifact not found".to_string(),
                    details: None,
                })
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
            let inner = self.inner.lock().await;
            Ok(inner.contains_key(id))
        }
    }

    #[async_trait]
    impl ArtifactStore for NoopArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            Ok(ArtifactId::must_new("0".repeat(64)))
        }

        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(false)
        }
    }

    struct PanicResolver;

    impl PlanResolver for PanicResolver {
        fn resolve(
            &self,
            _manifest: &mfm_machine::config::RunManifest,
        ) -> Result<mfm_machine::plan::ExecutionPlan, RunError> {
            panic!("child-run resolver should not be used in these transport tests")
        }
    }

    fn default_transport_registry() -> HashMapTransportRegistry {
        let mut registry = HashMapTransportRegistry::new();
        DefaultTransportPlugin
            .register_transports(&mut registry)
            .expect("default transports should register");
        registry
    }

    fn test_live_io_env() -> LiveIoEnv {
        LiveIoEnv {
            stores: Stores {
                streams: Arc::new(NoopStreamStore),
                artifacts: Arc::new(NoopArtifactStore),
            },
            run_id: RunId(uuid::Uuid::new_v4()),
            state_id: StateId::must_new("app.tests.echo".to_string()),
            attempt: 0,
        }
    }

    struct EchoTransportFactory;

    impl LiveIoTransportFactory for EchoTransportFactory {
        fn namespace_group(&self) -> &str {
            "app.test"
        }

        fn make(&self, _env: LiveIoEnv) -> Box<dyn mfm_machine::live_io::LiveIoTransport> {
            Box::new(EchoTransport)
        }
    }

    struct EchoTransport;

    #[async_trait]
    impl mfm_machine::live_io::LiveIoTransport for EchoTransport {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            Ok(serde_json::json!({ "namespace": call.namespace }))
        }
    }

    fn assert_io_error_code(err: IoError, expected: &str) {
        match err {
            IoError::Other(info) => assert_eq!(info.code.as_str(), expected),
            other => panic!("unexpected io error: {other:?}"),
        }
    }

    #[derive(Clone)]
    struct TestPluginSingleState;

    #[async_trait]
    impl State for TestPluginSingleState {
        fn meta(&self) -> StateMeta {
            StateMeta {
                tags: Vec::new(),
                depends_on: Vec::new(),
                depends_on_strategy: DependencyStrategy::Latest,
                side_effects: SideEffectKind::Pure,
                idempotency: Idempotency::None,
            }
        }

        async fn handle(
            &self,
            _ctx: &mut dyn DynContext,
            _io: &mut dyn IoProvider,
            _rec: &mut dyn EventRecorder,
        ) -> Result<StateOutcome, StateError> {
            Ok(StateOutcome {
                snapshot: SnapshotPolicy::Never,
            })
        }
    }

    struct TestPluginSingleOp;

    impl Operation for TestPluginSingleOp {
        fn op_id(&self) -> OpId {
            OpId::must_new("plugin_single_op")
        }

        fn op_version(&self) -> String {
            "v1".to_string()
        }

        fn expand(
            &self,
            op_path: OpPath,
            _op_config: &serde_json::Value,
            _run_config: &RunConfig,
        ) -> Result<PlannedOp, SdkError> {
            Ok(PlannedOp {
                interface: OpInterface {
                    imports: Vec::new(),
                    exports: Vec::new(),
                },
                kind: PlannedOpKind::Leaf(LeafOpSpec {
                    states: vec![leaf_state_node(
                        &op_path,
                        "done",
                        Arc::new(TestPluginSingleState),
                    )?],
                    edges: Vec::new(),
                }),
            })
        }
    }

    struct TestPlugin;

    impl OperationPlugin for TestPlugin {
        fn register_operations(&self, registry: &mut HashMapOperationRegistry) {
            registry.register(Arc::new(TestPluginSingleOp));
        }
    }

    fn test_plugin_single_services() -> AppServices {
        let bundle = AppBuilder::new()
            .with_operation_plugin(Arc::new(TestPlugin))
            .build()
            .expect("builder should include plugin-registered root op");
        AppServices::new(
            bundle,
            Arc::new(NoopStreamStore),
            Arc::new(InMemoryArtifactStore::default()),
        )
    }

    fn plugin_single_pipeline() -> Pipeline {
        Pipeline {
            machine_id: MachineId("plugin_single_op".to_string()),
            pipeline_version: "v1".to_string(),
            steps: vec![PipelineStep {
                step_id: StepId("main".to_string()),
                op_id: OpId::must_new("plugin_single_op".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
            }],
        }
    }

    fn test_services(bundle: EngineBundle) -> AppServices {
        AppServices::new(
            bundle,
            Arc::new(NoopStreamStore),
            Arc::new(NoopArtifactStore),
        )
    }

    #[test]
    fn default_transport_plugin_registers_expected_namespace_groups() {
        let registry = default_transport_registry();

        for group in [
            "exec",
            "nix.exec",
            "local.fs",
            "local.evm",
            "local.keystore",
            "rpc.control",
        ] {
            assert!(
                registry.resolve(group).is_some(),
                "expected namespace group {group} to be registered"
            );
        }
        assert_eq!(registry.all().len(), 6);
    }

    #[tokio::test]
    async fn router_from_default_registry_rejects_unknown_namespace() {
        let registry = default_transport_registry();
        let factory = RouterLiveIoTransportFactory::from_registry(&registry);
        let mut transport = factory.make(test_live_io_env());

        let err = transport
            .call(IoCall {
                namespace: "unknown.namespace".to_string(),
                request: serde_json::json!({}),
                fact_key: None,
            })
            .await
            .expect_err("unknown namespace should fail");
        assert_io_error_code(err, "io_unknown_namespace");
    }

    #[tokio::test]
    async fn child_run_wrapper_forwards_non_child_namespaces() {
        let mut registry = HashMapTransportRegistry::new();
        registry
            .register(Arc::new(EchoTransportFactory))
            .expect("echo transport should register");
        let base_factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(RouterLiveIoTransportFactory::from_registry(&registry));
        let wrapper = ChildRunLiveIoTransportFactory::new(Arc::new(PanicResolver), base_factory);
        let mut transport = wrapper.make(test_live_io_env());

        let response = transport
            .call(IoCall {
                namespace: "app.test.echo".to_string(),
                request: serde_json::json!({}),
                fact_key: None,
            })
            .await
            .expect("non-child route should survive child-run wrapper");
        assert_eq!(
            response,
            serde_json::json!({ "namespace": "app.test.echo" })
        );
    }

    #[tokio::test]
    async fn child_run_wrapper_intercepts_reserved_namespaces() {
        let registry = default_transport_registry();
        let base_factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(RouterLiveIoTransportFactory::from_registry(&registry));
        let wrapper = ChildRunLiveIoTransportFactory::new(Arc::new(PanicResolver), base_factory);
        let mut transport = wrapper.make(test_live_io_env());

        let err = transport
            .call(IoCall {
                namespace: "machine.child_run.spawn".to_string(),
                request: serde_json::json!({}),
                fact_key: Some(FactKey("mfm:child-run:test".to_string())),
            })
            .await
            .expect_err("invalid child-run payload should fail at wrapper boundary");
        assert_io_error_code(err, "child_run_request_invalid");
    }

    #[tokio::test]
    async fn artifact_get_rejects_invalid_artifact_id_before_storage_lookup() {
        let services = test_services(make_engine_bundle());

        let err = services
            .artifact_get("artifact_123")
            .await
            .expect_err("invalid artifact ids must be rejected at the app boundary");

        assert_eq!(err.class, ErrorClass::BadRequest);
        assert_eq!(err.code, "InvalidArtifactId");
    }

    #[tokio::test]
    async fn artifact_get_rejects_protected_artifact_envelopes() {
        let inner: Arc<dyn ArtifactStore> = Arc::new(InMemoryArtifactStore::default());
        let protected = SecretArtifactStore::new(inner, SecretKey::from_bytes([7_u8; 32]));
        let id = protected
            .put_secret_bytes(b"raw signed transaction capability")
            .await
            .expect("protected artifact");
        let artifacts: Arc<dyn ArtifactStore> = Arc::new(protected);

        let err = get_artifact_from_store(artifacts, &id)
            .await
            .expect_err("generic artifact fetch must reject protected artifacts");

        assert_eq!(err.class, ErrorClass::NotFound);
        assert_eq!(err.code, "artifact_not_found");
    }

    #[test]
    fn run_start_rejects_untagged_pipeline_shape() {
        let payload = serde_json::json!({
            "pipeline": plugin_single_pipeline(),
            "input": {}
        });

        let err = serde_json::from_value::<RunsStartRequest>(payload)
            .expect_err("run.start payloads must include an explicit kind tag");

        assert!(
            err.to_string().contains("kind"),
            "unexpected serde error: {err}"
        );
    }

    #[test]
    fn run_start_rejects_pipeline_missing_kind() {
        let payload = serde_json::json!({
            "pipeline": plugin_single_pipeline()
        });

        assert!(
            serde_json::from_value::<RunsStartRequest>(payload).is_err(),
            "pipeline payloads without kind must not match a default single-op request"
        );
    }

    #[test]
    fn run_start_rejects_unknown_fields() {
        let payload = serde_json::json!({
            "kind": "single_op_start_v1",
            "op_id": "plugin_single_op",
            "op_version": "v1",
            "op_config": {},
            "unexpected": true
        });

        let err = serde_json::from_value::<RunsStartRequest>(payload)
            .expect_err("run.start must reject unknown envelope fields");

        assert!(
            err.to_string().contains("unexpected"),
            "unexpected serde error: {err}"
        );
    }

    #[tokio::test]
    async fn feature_run_start_rejects_malformed_kind_as_invalid_json() {
        let services = test_services(make_engine_bundle());
        let catalog = FeatureCatalog::with_builtins();

        let err = catalog
            .execute(
                &services,
                FeatureRequest {
                    feature_id: "run.start".to_string(),
                    payload: serde_json::json!({
                        "kind": "pipeline",
                        "pipeline": plugin_single_pipeline()
                    }),
                },
            )
            .await
            .expect_err("malformed run.start kind must fail before run planning");

        assert_eq!(err.class, ErrorClass::BadRequest);
        assert_eq!(err.code, "InvalidJson");
    }

    #[tokio::test]
    async fn start_run_allows_plugin_registered_root_op() {
        let services = test_plugin_single_services();

        let response = services
            .start_run(RunsStartRequest::SingleOp {
                op_id: "plugin_single_op".to_string(),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({}),
                input: serde_json::json!({}),
            })
            .await
            .expect("plugin-registered root op should be startable");

        assert_ne!(response.phase, "failed");
    }

    #[tokio::test]
    async fn tagged_pipeline_start_uses_requested_pipeline() {
        let services = test_plugin_single_services();

        let response = services
            .start_run(RunsStartRequest::Pipeline {
                pipeline: plugin_single_pipeline(),
                input: serde_json::json!({"source": "tagged_pipeline_test"}),
                run_config: None,
            })
            .await
            .expect("tagged pipeline payload should be startable");

        assert_ne!(response.phase, "failed");
    }

    #[test]
    fn default_registry_does_not_register_portfolio_semantic_ops() {
        let bundle = make_engine_bundle();

        for op_id in [
            "portfolio_tracker",
            "portfolio_execute",
            "portfolio_config_build",
        ] {
            assert!(
                bundle
                    .registry
                    .resolve(&OpId::must_new(op_id.to_string()), "v1")
                    .is_err(),
                "legacy registry must not expose dynamic portfolio op `{op_id}`"
            );
        }
    }

    #[test]
    fn deploy_configure_validate_dynamic_roots_are_not_registered() {
        let bundle = make_engine_bundle();
        for op_id in [
            "evm_deploy_configure_validate",
            "evm_deploy_configure_validate_config_build",
            "evm_deploy_configure_validate_execute",
        ] {
            assert!(
                bundle
                    .registry
                    .resolve(&OpId::must_new(op_id.to_owned()), "v1")
                    .is_err(),
                "legacy registry must not expose dynamic EVM DCV op `{op_id}`"
            );
        }
    }

    fn deploy_configure_validate_json() -> serde_json::Value {
        serde_json::json!({
            "deploy": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead"
            },
            "configure": {
                "network_id": "ethereum-mainnet",
                "from": "0x000000000000000000000000000000000000dead",
                "calls": []
            },
            "validate": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1
            }
        })
    }

    fn deploy_configure_validate_toml() -> &'static str {
        r#"
[deploy]
network_id = "ethereum-mainnet"
from = "0x000000000000000000000000000000000000dead"

[configure]
network_id = "ethereum-mainnet"
from = "0x000000000000000000000000000000000000dead"
calls = []

[validate]
network_id = "ethereum-mainnet"
expected_chain_id = 1
"#
    }
}
