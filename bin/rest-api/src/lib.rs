#![warn(missing_docs)]
//! REST API wiring for certified typed MFM runs.
//!
//! This crate adapts typed [`mfm_app`] services onto an `axum` router. It is an assembly layer
//! only: it decodes HTTP input, chooses stores, starts/resumes/replays certified typed runs, and
//! renders typed public outputs.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_rest_api::{make_app, make_default_app_state};
//!
//! async fn build_router() -> Result<axum::Router, mfm_rest_api::ApiError> {
//!     let state = make_default_app_state().await?;
//!     Ok(make_app(state))
//! }
//! ```

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use http::header::HeaderName;
use mfm_app::{
    AppError, AsyncRunServices, DriveMode, ErrorClass, ManualResolutionDecision,
    ManualResolutionRecordRequest, PublicSafeMessage, RunLaunchConfigArtifact,
    RunLaunchSeedArtifact, TypedPublicOutputResponse, TypedRunMode, TypedRunResponse,
    TypedRunStreamResponse,
};
use mfm_artifact_store_fs::{FsTypedArtifactError, FsTypedArtifactStore};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1::ArtifactRole;
use mfm_evm_contract_config::{ConfigurePhaseConfig, DeployPhaseConfig, ValidatePhaseConfig};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, RunId, SchemaId, SeedId};
use mfm_op_evm_contract_lifecycle::{
    compile_contract_configure_program, compile_contract_deploy_program,
    compile_contract_lifecycle_program, compile_contract_validate_program,
    CompiledContractLifecycleProgram, ContractLifecycleCompileError, ContractLifecycleConfig,
    ContractLifecycleConfigArtifact,
};
use mfm_op_portfolio_tracker::{compile_portfolio_snapshot_program, PortfolioConfigArtifact};
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, parse_portfolio_snapshot_authored_config,
    AuthoredConfigFormat, PortfolioSnapshotConfigError,
};
use mfm_spec::v1 as spec;
use mfm_state_portfolio::PortfolioWorkflowConfig;
use mfm_store::v1 as store;
use mfm_store::v1::{
    AsyncStoreFuture, AsyncTypedRunEventStore, TypedProjectionRead, TypedRunEventStore,
};
use mfm_stream_store_postgres::{PostgresTypedRunEventStore, PostgresTypedStoreError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tracing::instrument;

/// Boxed future returned by [`StatusProjectionRead`].
pub type StatusProjectionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<store::ProjectionSnapshot, ApiError>> + Send + 'a>>;

/// Store capability required by REST status rendering.
///
/// Implementations must return a projection for the requested run while preserving any global
/// projection families that status depends on, such as active cross-run resource lanes.
pub trait StatusProjectionRead {
    /// Loads the status projection for `run_id`.
    fn status_projection<'a>(&'a self, run_id: &'a RunId) -> StatusProjectionFuture<'a>;
}

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

fn serialize_response<T: Serialize>(data: T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(data).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SerializationError",
            "Failed to serialize response payload",
        )
    })
}

fn json_ok<T: Serialize>(data: T) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(ok(serialize_response(data)?)))
}

/// Error payload mapped onto HTTP responses.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// HTTP status to return.
    pub status: StatusCode,
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

impl ApiError {
    /// Creates an API error with an explicit HTTP status, code, and message.
    pub fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<PublicSafeMessage>,
    ) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into().into_string(),
        }
    }

    /// Creates an API error for a lower-level failure without exposing backend details.
    pub fn backend(status: StatusCode, code: impl Into<String>, message: &'static str) -> Self {
        Self::new(status, code, PublicSafeMessage::backend(message))
    }

    /// Returns the standard invalid-JSON error.
    pub fn invalid_json() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "InvalidJson",
            "Failed to parse request body as JSON",
        )
    }
}

impl From<AppError> for ApiError {
    fn from(value: AppError) -> Self {
        let status = match value.class {
            ErrorClass::BadRequest => StatusCode::BAD_REQUEST,
            ErrorClass::NotFound => StatusCode::NOT_FOUND,
            ErrorClass::Conflict => StatusCode::CONFLICT,
            ErrorClass::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };

        Self::new(status, value.code, value.message)
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

/// Async in-memory typed run store for tests and single-process development tools.
#[derive(Clone, Default)]
pub struct InMemoryAsyncTypedRunStore {
    inner: Arc<Mutex<store::InMemoryTypedRunStore>>,
}

impl AsyncTypedRunEventStore for InMemoryAsyncTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_plan<'a>(
        &'a self,
        plan: store::PreparedCommitPlan,
    ) -> AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = self
            .inner
            .lock()
            .expect("typed run store lock")
            .append_prepared_commit_plan(plan);
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        let result = Ok(self
            .inner
            .lock()
            .expect("typed run store lock")
            .load_run_stream(run_id));
        Box::pin(std::future::ready(result))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        let result = Ok(self
            .inner
            .lock()
            .expect("typed run store lock")
            .expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        Box::pin(async move {
            let store = self.inner.lock().expect("typed run store lock");
            let stream = store.load_run_stream(run_id);
            let run_projection = store::ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
            projection_with_resource_lanes(
                &run_projection,
                store
                    .projection_snapshot()
                    .resource_lanes()
                    .map(|(lane_key, projection)| (lane_key.clone(), projection.clone()))
                    .collect(),
            )
        })
    }
}

impl StatusProjectionRead for InMemoryAsyncTypedRunStore {
    fn status_projection<'a>(&'a self, run_id: &'a RunId) -> StatusProjectionFuture<'a> {
        Box::pin(async move {
            self.status_projection_snapshot(run_id)
                .await
                .map_err(api_error_from_store_error)
        })
    }
}

impl StatusProjectionRead for PostgresTypedRunEventStore {
    fn status_projection<'a>(&'a self, run_id: &'a RunId) -> StatusProjectionFuture<'a> {
        Box::pin(async move {
            self.status_projection_snapshot(run_id)
                .await
                .map_err(api_error_from_typed_store_error)
        })
    }
}

/// Default production REST API state.
pub type DefaultAppState = AppState<PostgresTypedRunEventStore>;

/// Shared router state injected into request handlers.
#[derive(Clone)]
pub struct AppState<S = PostgresTypedRunEventStore> {
    /// Certified typed run-event store.
    pub store: S,
    /// Certified typed filesystem artifact store.
    pub artifacts: FsTypedArtifactStore,
}

#[derive(Clone)]
struct RouterState<S> {
    app: AppState<S>,
}

impl<S> RouterState<S>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    fn services(&self) -> Result<AsyncRunServices<S>, ApiError> {
        let runners = mfm_app::production_typed_runner_registry(self.app.artifacts.clone())?;
        let certification_registry = mfm_app::production_certification_registry()?;
        Ok(
            mfm_app::make_async_typed_services_with_certification_registry(
                runners,
                self.app.store.clone(),
                self.app.artifacts.clone(),
                certification_registry,
            ),
        )
    }
}

/// Builds the default certified typed filesystem artifact store.
pub fn make_default_typed_artifact_store() -> FsTypedArtifactStore {
    mfm_app::make_default_typed_artifact_store()
}

/// Connects to the default certified typed run-event store.
pub async fn make_default_typed_run_store() -> Result<PostgresTypedRunEventStore, ApiError> {
    PostgresTypedRunEventStore::connect_env()
        .await
        .map_err(api_error_from_typed_store_error)
}

/// Builds default production REST API state from environment-selected stores.
pub async fn make_default_app_state() -> Result<DefaultAppState, ApiError> {
    Ok(AppState {
        store: make_default_typed_run_store().await?,
        artifacts: make_default_typed_artifact_store(),
    })
}

/// Builds in-memory REST API state rooted at `artifact_root`.
pub fn make_in_memory_app_state(
    artifact_root: impl Into<PathBuf>,
) -> AppState<InMemoryAsyncTypedRunStore> {
    AppState {
        store: InMemoryAsyncTypedRunStore::default(),
        artifacts: FsTypedArtifactStore::new(artifact_root),
    }
}

/// Builds the `axum` router for the public REST API surface.
pub fn make_app<S>(state: AppState<S>) -> Router
where
    S: AsyncTypedRunEventStore + StatusProjectionRead + Clone + Send + Sync + 'static,
{
    let request_id_header = HeaderName::from_static("x-request-id");
    let make_span_header = request_id_header.clone();
    let state = RouterState { app: state };

    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/ready", get(ready::<S>))
        .route("/v1/portfolio/snapshot", post(portfolio_snapshot::<S>))
        .route("/v1/evm/contracts/deploy", post(evm_contract_deploy::<S>))
        .route(
            "/v1/evm/contracts/configure",
            post(evm_contract_configure::<S>),
        )
        .route(
            "/v1/evm/contracts/validate",
            post(evm_contract_validate::<S>),
        )
        .route(
            "/v1/evm/contracts/lifecycle",
            post(evm_contract_lifecycle::<S>),
        )
        .route("/v1/runs/start", post(runs_start::<S>))
        .route("/v1/runs/:run_id/resume", post(runs_resume::<S>))
        .route(
            "/v1/runs/:run_id/manual-resolution",
            post(runs_manual_resolution::<S>),
        )
        .route("/v1/runs/:run_id/status", get(runs_status::<S>))
        .route("/v1/runs/:run_id/stream", get(runs_stream::<S>))
        .route("/v1/runs/:run_id/replay", post(runs_replay::<S>))
        .route(
            "/v1/runs/:run_id/public-output/:schema_id",
            get(runs_public_output::<S>),
        )
        .fallback(not_found)
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(move |request: &axum::http::Request<axum::body::Body>| {
                    let request_id = request
                        .headers()
                        .get(&make_span_header)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("missing");
                    tracing::info_span!(
                        "http.request",
                        request_id = %request_id,
                        method = %request.method(),
                        route = %request.uri().path()
                    )
                })
                .on_request(
                    |_request: &axum::http::Request<axum::body::Body>, _span: &tracing::Span| {
                        tracing::info!("request started");
                    },
                )
                .on_response(
                    |response: &axum::http::Response<axum::body::Body>,
                     latency: Duration,
                     _span: &tracing::Span| {
                        tracing::info!(
                            status_code = response.status().as_u16(),
                            latency_ms = latency.as_millis() as u64,
                            "request completed"
                        );
                    },
                ),
        )
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(ok(json!({ "ok": true })))
}

#[instrument(level = "debug", skip(state))]
async fn ready<S>(State(state): State<RouterState<S>>) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    state
        .app
        .store
        .expected_next_seq(&mfm_app::new_run_id())
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "NotReady",
                "typed run store is not ready",
            )
        })?;

    let probe = readiness_artifact_probe()?;
    state
        .app
        .artifacts
        .has_artifact(&probe)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "NotReady",
                "typed artifact store is not ready",
            )
        })?;

    Ok(Json(ok(json!({
      "ok": true,
      "checks": {
        "typed_run_store": "ready",
        "typed_artifact_store": "ready"
      }
    }))))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(err("not_found", "not found")))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TypedRunStartKind {
    TypedRunStartV1,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManualResolutionKind {
    ManualResolutionV1,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PortfolioSnapshotStartKind {
    PortfolioSnapshotStartV1,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmContractDeployStartKind {
    EvmContractDeployStartV1,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmContractConfigureStartKind {
    EvmContractConfigureStartV1,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmContractValidateStartKind {
    EvmContractValidateStartV1,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvmContractLifecycleStartKind {
    EvmContractLifecycleStartV1,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RestDriveMode {
    AppendOnly,
    Once,
    #[default]
    UntilBlocked,
}

impl RestDriveMode {
    fn into_app(self) -> DriveMode {
        match self {
            Self::AppendOnly => DriveMode::AppendOnly,
            Self::Once => DriveMode::Once,
            Self::UntilBlocked => DriveMode::UntilBlocked,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedRunStartBody {
    kind: TypedRunStartKind,
    bundle: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    configs: Vec<TypedConfigBody>,
    #[serde(default)]
    seeds: Vec<TypedSeedBody>,
    #[serde(default = "default_framework_version")]
    framework_version: String,
    #[serde(default = "default_source_revision")]
    source_revision: String,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedConfigBody {
    schema_id: String,
    json: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortfolioSnapshotStartBody {
    kind: PortfolioSnapshotStartKind,
    request: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default = "default_portfolio_framework_version")]
    framework_version: String,
    #[serde(default = "default_source_revision")]
    source_revision: String,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvmContractDeployStartBody {
    kind: EvmContractDeployStartKind,
    config: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default = "default_evm_contract_framework_version")]
    framework_version: String,
    #[serde(default = "default_source_revision")]
    source_revision: String,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvmContractConfigureStartBody {
    kind: EvmContractConfigureStartKind,
    config: serde_json::Value,
    deployed: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default = "default_evm_contract_framework_version")]
    framework_version: String,
    #[serde(default = "default_source_revision")]
    source_revision: String,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvmContractValidateStartBody {
    kind: EvmContractValidateStartKind,
    config: serde_json::Value,
    configured: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default = "default_evm_contract_framework_version")]
    framework_version: String,
    #[serde(default = "default_source_revision")]
    source_revision: String,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvmContractLifecycleStartBody {
    kind: EvmContractLifecycleStartKind,
    config: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default = "default_evm_contract_framework_version")]
    framework_version: String,
    #[serde(default = "default_source_revision")]
    source_revision: String,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Serialize)]
struct PortfolioSnapshotStartResponse {
    run: TypedRunResponse,
    public_output: Option<TypedPublicOutputResponse>,
}

#[derive(Debug, Serialize)]
struct EvmContractStartResponse {
    run: TypedRunResponse,
    public_output: Option<TypedPublicOutputResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedSeedBody {
    seed_id: String,
    json: serde_json::Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedRunResumeBody {
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManualResolutionBody {
    kind: ManualResolutionKind,
    outcome: ManualResolutionDecision,
    evidence_json: serde_json::Value,
    authorization_proof: serde_json::Value,
    #[serde(default = "default_json_media_type_string")]
    evidence_media_type: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunStreamQuery {
    #[serde(default = "default_from_seq")]
    from_seq: u64,
    #[serde(default)]
    to_seq: Option<u64>,
}

fn default_from_seq() -> u64 {
    1
}

fn default_json_media_type_string() -> String {
    "application/json".to_owned()
}

fn default_framework_version() -> String {
    "mfm.rest_api.typed.v1".to_owned()
}

fn default_portfolio_framework_version() -> String {
    "mfm.rest_api.portfolio.typed.v1".to_owned()
}

fn default_evm_contract_framework_version() -> String {
    "mfm.rest_api.evm_contracts.typed.v1".to_owned()
}

#[allow(clippy::disallowed_methods)]
fn default_source_revision() -> String {
    std::env::var("MFM_SOURCE_REVISION").unwrap_or_else(|_| "unknown".to_owned())
}

fn launch_unix_ms() -> Result<u64, ApiError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            ApiError::backend(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LaunchClockUnavailable",
                "System clock is unavailable",
            )
        })?
        .as_millis();
    u64::try_from(millis).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "LaunchClockOverflow",
            "current Unix timestamp in milliseconds does not fit in u64",
        )
    })
}

#[instrument(level = "info", skip(state, body))]
async fn portfolio_snapshot<S>(
    State(state): State<RouterState<S>>,
    body: Result<Json<PortfolioSnapshotStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    match req.kind {
        PortfolioSnapshotStartKind::PortfolioSnapshotStartV1 => {}
    }
    let canonical = parse_portfolio_snapshot_request(&req.request)?;
    let workflow_config = PortfolioWorkflowConfig::from(canonical);
    let compiled = compile_portfolio_snapshot_program(workflow_config).map_err(|error| {
        let _ = error;
        ApiError::backend(
            StatusCode::BAD_REQUEST,
            "PortfolioCompileInvalid",
            "Portfolio snapshot request failed validation",
        )
    })?;
    let public_schema_id = compiled.public_schema_id.clone();

    let run_id = parse_optional_run_id(req.run_id)?;
    let services = state.services()?;
    let config_inputs = run_launch_config_artifacts(compiled.config_artifacts);
    let start = mfm_app::prepare_certified_run_launch(
        mfm_app::CertifiedRunLaunchInput {
            certified_spec: compiled.certified_spec,
            registry: services.certification_registry(),
            run_id: run_id.clone(),
            framework_version: &req.framework_version,
            source_revision: &req.source_revision,
            launched_at_unix_ms: launch_unix_ms()?,
            drive: req.drive.into_app(),
        },
        config_inputs,
        Vec::new(),
    )?;
    let run = services.launch_run(start).await?;
    let public_output = if run.run_mode == TypedRunMode::Completed {
        Some(
            services
                .typed_public_output(&run_id, &public_schema_id)
                .await?,
        )
    } else {
        None
    };

    json_ok(PortfolioSnapshotStartResponse { run, public_output })
}

#[instrument(level = "info", skip(state, body))]
async fn evm_contract_deploy<S>(
    State(state): State<RouterState<S>>,
    body: Result<Json<EvmContractDeployStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    match req.kind {
        EvmContractDeployStartKind::EvmContractDeployStartV1 => {}
    }
    let config = deserialize_request_value::<DeployPhaseConfig>(&req.config)?;
    let compiled =
        compile_contract_deploy_program(config).map_err(api_error_from_contract_compile)?;
    evm_contract_start(
        state,
        req.run_id,
        req.framework_version,
        req.source_revision,
        req.drive,
        compiled,
        Vec::new(),
    )
    .await
}

#[instrument(level = "info", skip(state, body))]
async fn evm_contract_configure<S>(
    State(state): State<RouterState<S>>,
    body: Result<Json<EvmContractConfigureStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    match req.kind {
        EvmContractConfigureStartKind::EvmContractConfigureStartV1 => {}
    }
    let config = deserialize_request_value::<ConfigurePhaseConfig>(&req.config)?;
    let deployed = deserialize_request_value::<DeployedContract>(&req.deployed)?;
    let seed_bytes = canonical_request_value_bytes(&deployed)?;
    let compiled = compile_contract_configure_program(config, deployed)
        .map_err(api_error_from_contract_compile)?;
    let seed = seed_artifact_for_single_contract_seed(&compiled, seed_bytes)?;
    evm_contract_start(
        state,
        req.run_id,
        req.framework_version,
        req.source_revision,
        req.drive,
        compiled,
        vec![seed],
    )
    .await
}

#[instrument(level = "info", skip(state, body))]
async fn evm_contract_validate<S>(
    State(state): State<RouterState<S>>,
    body: Result<Json<EvmContractValidateStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    match req.kind {
        EvmContractValidateStartKind::EvmContractValidateStartV1 => {}
    }
    let config = deserialize_request_value::<ValidatePhaseConfig>(&req.config)?;
    let configured = deserialize_request_value::<ConfiguredContract>(&req.configured)?;
    let seed_bytes = canonical_request_value_bytes(&configured)?;
    let compiled = compile_contract_validate_program(config, configured)
        .map_err(api_error_from_contract_compile)?;
    let seed = seed_artifact_for_single_contract_seed(&compiled, seed_bytes)?;
    evm_contract_start(
        state,
        req.run_id,
        req.framework_version,
        req.source_revision,
        req.drive,
        compiled,
        vec![seed],
    )
    .await
}

#[instrument(level = "info", skip(state, body))]
async fn evm_contract_lifecycle<S>(
    State(state): State<RouterState<S>>,
    body: Result<Json<EvmContractLifecycleStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    match req.kind {
        EvmContractLifecycleStartKind::EvmContractLifecycleStartV1 => {}
    }
    let config = deserialize_request_value::<ContractLifecycleConfig>(&req.config)?;
    let compiled =
        compile_contract_lifecycle_program(config).map_err(api_error_from_contract_compile)?;
    evm_contract_start(
        state,
        req.run_id,
        req.framework_version,
        req.source_revision,
        req.drive,
        compiled,
        Vec::new(),
    )
    .await
}

#[instrument(level = "info", skip(state, body))]
async fn runs_start<S>(
    State(state): State<RouterState<S>>,
    body: Result<Json<TypedRunStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    match req.kind {
        TypedRunStartKind::TypedRunStartV1 => {}
    }
    let run_id = parse_optional_run_id(req.run_id)?;
    let bundle = mfm_app::parse_certified_spec_bundle_json_value(&req.bundle)?;
    let config_media_type = mfm_app::json_media_type()?;
    let mut configs = Vec::with_capacity(req.configs.len());
    for config in req.configs {
        configs.push(RunLaunchConfigArtifact {
            schema_id: SchemaId::parse(&config.schema_id).map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "LaunchConfigSchemaInvalid",
                    "typed config schema id is invalid",
                )
            })?,
            bytes: canonical_json_value_bytes(&config.json, "LaunchConfigInvalid")?,
            media_type: config_media_type.clone(),
        });
    }
    let seed_media_type = mfm_app::json_media_type()?;
    let mut seeds = Vec::with_capacity(req.seeds.len());
    for seed in req.seeds {
        seeds.push(RunLaunchSeedArtifact {
            seed_id: parse_seed_id(&seed.seed_id)?,
            bytes: canonical_json_value_bytes(&seed.json, "LaunchSeedInvalid")?,
            media_type: seed_media_type.clone(),
        });
    }

    let services = state.services()?;
    let start = mfm_app::prepare_verified_bundle_launch(
        mfm_app::UntrustedCertifiedBundleLaunchInput {
            spec_bytes: bundle.spec_bytes(),
            certificate_bytes: bundle.certificate_bytes(),
            registry: services.certification_registry(),
            run_id,
            framework_version: &req.framework_version,
            source_revision: &req.source_revision,
            launched_at_unix_ms: launch_unix_ms()?,
            drive: req.drive.into_app(),
        },
        configs,
        seeds,
    )?;
    let data = services.launch_run(start).await?;

    json_ok(data)
}

#[instrument(level = "info", skip(state, body), fields(run_id = run_id.as_str()))]
async fn runs_resume<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let run_id = parse_run_id(&run_id)?;
    let req: TypedRunResumeBody = parse_optional_body(body)?;
    let data = state
        .services()?
        .resume_stored_run(&run_id, req.drive.into_app())
        .await?;

    json_ok(data)
}

#[instrument(level = "info", skip(state, body), fields(run_id = run_id.as_str()))]
async fn runs_manual_resolution<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
    body: Result<Json<ManualResolutionBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let run_id = parse_run_id(&run_id)?;
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    match req.kind {
        ManualResolutionKind::ManualResolutionV1 => {}
    }
    let evidence_bytes =
        canonical_json_value_bytes(&req.evidence_json, "ManualResolutionEvidenceInvalid")?;
    let proof_bytes =
        canonical_json_value_bytes(&req.authorization_proof, "ManualResolutionProofInvalid")?;
    let data = state
        .services()?
        .record_manual_resolution(ManualResolutionRecordRequest {
            run_id,
            outcome: req.outcome,
            evidence_bytes,
            evidence_media_type: req.evidence_media_type,
            authorization_proof_bytes: proof_bytes,
            note: req.note,
            drive: req.drive.into_app(),
        })
        .await?;

    json_ok(data)
}

#[instrument(level = "debug", skip(state), fields(run_id = run_id.as_str()))]
async fn runs_status<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + StatusProjectionRead + Clone + Send + Sync,
{
    let run_id = parse_run_id(&run_id)?;
    let projection = state.app.store.status_projection(&run_id).await?;
    let data = state
        .services()?
        .run_status_with_projection(&run_id, projection)
        .await?;

    json_ok(data)
}

#[instrument(
    level = "debug",
    skip(state, query),
    fields(run_id = run_id.as_str())
)]
async fn runs_stream<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
    query: Result<Query<RunStreamQuery>, QueryRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let Query(query) = query.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidQuery",
            "Failed to parse stream query",
        )
    })?;
    validate_sequence_range(query.from_seq, query.to_seq)?;

    let run_id = parse_run_id(&run_id)?;
    let response = state.services()?.run_stream(&run_id).await?;
    let data = TypedRunStreamResponse {
        run_id: response.run_id,
        head_seq: response.head_seq,
        events: response
            .events
            .into_iter()
            .filter(|event| {
                event.seq >= query.from_seq
                    && match query.to_seq {
                        Some(to_seq) => event.seq <= to_seq,
                        None => true,
                    }
            })
            .collect(),
    };

    json_ok(data)
}

#[instrument(level = "info", skip(state), fields(run_id = run_id.as_str()))]
async fn runs_replay<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let run_id = parse_run_id(&run_id)?;
    let data = state.services()?.verify_replay_for_run(&run_id).await?;

    json_ok(data)
}

#[instrument(
    level = "debug",
    skip(state),
    fields(run_id = run_id.as_str(), schema_id = schema_id.as_str())
)]
async fn runs_public_output<S>(
    State(state): State<RouterState<S>>,
    Path((run_id, schema_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let run_id = parse_run_id(&run_id)?;
    let schema_id = parse_schema_id(&schema_id)?;
    let data = state
        .services()?
        .typed_public_output(&run_id, &schema_id)
        .await?;

    json_ok(data)
}

async fn evm_contract_start<S>(
    state: RouterState<S>,
    run_id: Option<String>,
    framework_version: String,
    source_revision: String,
    drive: RestDriveMode,
    compiled: CompiledContractLifecycleProgram,
    seed_inputs: Vec<RunLaunchSeedArtifact>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let run_id = parse_optional_run_id(run_id)?;
    let public_schema_id = compiled.public_schema_id.clone();
    let services = state.services()?;
    let start = mfm_app::prepare_certified_run_launch(
        mfm_app::CertifiedRunLaunchInput {
            certified_spec: compiled.certified_spec,
            registry: services.certification_registry(),
            run_id: run_id.clone(),
            framework_version: &framework_version,
            source_revision: &source_revision,
            launched_at_unix_ms: launch_unix_ms()?,
            drive: drive.into_app(),
        },
        evm_contract_config_artifacts(compiled.config_artifacts),
        seed_inputs,
    )?;
    let run = services.launch_run(start).await?;
    let public_output = if run.run_mode == TypedRunMode::Completed {
        Some(
            services
                .typed_public_output(&run_id, &public_schema_id)
                .await?,
        )
    } else {
        None
    };

    json_ok(EvmContractStartResponse { run, public_output })
}

fn evm_contract_config_artifacts(
    configs: Vec<ContractLifecycleConfigArtifact>,
) -> Vec<RunLaunchConfigArtifact> {
    configs
        .into_iter()
        .map(|config| RunLaunchConfigArtifact {
            schema_id: config.schema_id,
            bytes: config.bytes,
            media_type: config.media_type,
        })
        .collect()
}

fn seed_artifact_for_single_contract_seed(
    compiled: &CompiledContractLifecycleProgram,
    bytes: Vec<u8>,
) -> Result<RunLaunchSeedArtifact, ApiError> {
    let seeds = &compiled.certified_spec.envelope().spec.seeds;
    let [seed] = seeds.as_slice() else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "EvmContractCompileInvalid",
            format!(
                "expected exactly one contract launch seed, found {}",
                seeds.len()
            ),
        ));
    };
    Ok(RunLaunchSeedArtifact {
        seed_id: seed.seed_id.clone(),
        bytes,
        media_type: mfm_app::json_media_type()?,
    })
}

fn deserialize_request_value<T>(value: &serde_json::Value) -> Result<T, ApiError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value.clone()).map_err(|_| {
        ApiError::backend(
            StatusCode::BAD_REQUEST,
            "InvalidEvmContractRequest",
            "Invalid EVM contract request",
        )
    })
}

fn canonical_request_value_bytes<T>(value: &T) -> Result<Vec<u8>, ApiError>
where
    T: Serialize,
{
    let json = serde_json::to_string(value).map_err(|_| {
        ApiError::backend(
            StatusCode::BAD_REQUEST,
            "InvalidEvmContractRequest",
            "Failed to serialize EVM contract request",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.to_vec())
        .map_err(|_| {
            ApiError::backend(
                StatusCode::BAD_REQUEST,
                "InvalidEvmContractRequest",
                "Failed to canonicalize EVM contract request",
            )
        })
}

fn api_error_from_contract_compile(error: ContractLifecycleCompileError) -> ApiError {
    let _ = error;
    ApiError::backend(
        StatusCode::BAD_REQUEST,
        "EvmContractCompileInvalid",
        "EVM contract lifecycle request failed validation",
    )
}

fn parse_run_id(value: &str) -> Result<RunId, ApiError> {
    RunId::parse(value).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidRunId",
            "Run id must use the typed run identity format `run:<algorithm>:<digest>`",
        )
    })
}

fn parse_optional_run_id(value: Option<String>) -> Result<RunId, ApiError> {
    match value {
        Some(value) => parse_run_id(&value),
        None => Ok(mfm_app::new_run_id()),
    }
}

fn parse_schema_id(value: &str) -> Result<SchemaId, ApiError> {
    SchemaId::parse(value).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidSchemaId",
            "Schema id must use the typed schema identity format",
        )
    })
}

fn parse_seed_id(value: &str) -> Result<SeedId, ApiError> {
    SeedId::parse(value).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidSeedId",
            "Seed id must use the typed seed identity format",
        )
    })
}

fn parse_optional_body<T>(body: Bytes) -> Result<T, ApiError>
where
    T: DeserializeOwned + Default,
{
    if body.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Ok(T::default());
    }
    serde_json::from_slice(&body).map_err(|_| ApiError::invalid_json())
}

fn canonical_json_value_bytes(
    value: &serde_json::Value,
    error_code: &'static str,
) -> Result<Vec<u8>, ApiError> {
    let json = serde_json::to_string(value).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SerializationError",
            "Failed to serialize request JSON",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.to_vec())
        .map_err(|_| {
            ApiError::backend(
                StatusCode::BAD_REQUEST,
                error_code,
                "Request JSON is not canonical JSON",
            )
        })
}

fn parse_portfolio_snapshot_request(
    value: &serde_json::Value,
) -> Result<mfm_portfolio_config::PortfolioSnapshotCanonicalConfig, ApiError> {
    let raw = serde_json::to_string(value).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SerializationError",
            "Failed to serialize portfolio request JSON",
        )
    })?;
    let authored = parse_portfolio_snapshot_authored_config(&raw, AuthoredConfigFormat::Json)
        .map_err(api_error_from_portfolio_config_error)?;
    canonicalize_portfolio_snapshot_authored_config(authored)
        .map_err(api_error_from_portfolio_config_error)
}

fn run_launch_config_artifacts(
    configs: Vec<PortfolioConfigArtifact>,
) -> Vec<RunLaunchConfigArtifact> {
    configs
        .into_iter()
        .map(|config| RunLaunchConfigArtifact {
            schema_id: config.schema_id,
            bytes: config.bytes,
            media_type: config.media_type,
        })
        .collect()
}

fn api_error_from_portfolio_config_error(error: PortfolioSnapshotConfigError) -> ApiError {
    match error {
        PortfolioSnapshotConfigError::InvalidJson { .. } => ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidJson",
            "Failed to parse request body as JSON",
        ),
        PortfolioSnapshotConfigError::InvalidToml { .. } => ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidToml",
            "Failed to parse request body as TOML",
        ),
        PortfolioSnapshotConfigError::InvalidBundle(_)
        | PortfolioSnapshotConfigError::Decode { .. }
        | PortfolioSnapshotConfigError::Serialize { .. }
        | PortfolioSnapshotConfigError::CanonicalJson { .. } => ApiError::backend(
            StatusCode::BAD_REQUEST,
            "InvalidPortfolioRequest",
            "Portfolio request failed validation",
        ),
    }
}

fn validate_sequence_range(from_seq: u64, to_seq: Option<u64>) -> Result<(), ApiError> {
    if from_seq == 0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidSequenceRange",
            "from_seq must be greater than zero",
        ));
    }
    if let Some(to_seq) = to_seq {
        if to_seq < from_seq {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "InvalidSequenceRange",
                "to_seq must be greater than or equal to from_seq",
            ));
        }
    }
    Ok(())
}

fn readiness_artifact_probe() -> Result<store::ArtifactEvidenceRef, ApiError> {
    let bytes = b"null";
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    Ok(store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").map_err(|_| {
            ApiError::backend(
                StatusCode::INTERNAL_SERVER_ERROR,
                "JsonMediaTypeInvalid",
                "JSON media type is invalid",
            )
        })?,
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::RetentionManifest,
    })
}

fn api_error_from_typed_store_error(error: PostgresTypedStoreError) -> ApiError {
    match error {
        PostgresTypedStoreError::Store(_) => ApiError::backend(
            StatusCode::CONFLICT,
            "RunStoreRejected",
            "Run store rejected the requested operation",
        ),
        PostgresTypedStoreError::Database(_) => ApiError::backend(
            StatusCode::SERVICE_UNAVAILABLE,
            "RunStoreUnavailable",
            "Run store is unavailable",
        ),
        PostgresTypedStoreError::Corruption(_) => ApiError::backend(
            StatusCode::INTERNAL_SERVER_ERROR,
            "RunStoreCorruption",
            "Run store returned invalid data",
        ),
    }
}

fn api_error_from_store_error(_error: store::StoreError) -> ApiError {
    ApiError::backend(
        StatusCode::CONFLICT,
        "RunStoreRejected",
        "Run store rejected the requested operation",
    )
}

fn projection_with_resource_lanes(
    snapshot: &store::ProjectionSnapshot,
    resource_lanes: BTreeMap<store::ResourceLaneKey, store::ResourceLaneProjection>,
) -> Result<store::ProjectionSnapshot, store::StoreError> {
    store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
        run_states: snapshot
            .run_states()
            .map(|(run_id, state)| (run_id.clone(), *state))
            .collect(),
        saga_policy_digests: snapshot
            .saga_policy_digests()
            .map(|(run_id, digest)| (run_id.clone(), digest.clone()))
            .collect(),
        run_completions: snapshot
            .run_completions()
            .map(|(run_id, completion)| (run_id.clone(), completion.clone()))
            .collect(),
        saga_engagements: snapshot
            .saga_engagements()
            .map(|(run_id, engagement)| (run_id.clone(), engagement.clone()))
            .collect(),
        manual_resolutions: snapshot
            .manual_resolutions()
            .map(|(run_id, resolution)| (run_id.clone(), resolution.clone()))
            .collect(),
        attempts: snapshot
            .attempts()
            .map(|((node_id, attempt_id), attempt)| {
                ((node_id.clone(), attempt_id.clone()), attempt.clone())
            })
            .collect(),
        cells: snapshot
            .cells()
            .map(|(cell_id, cell)| (cell_id.clone(), cell.clone()))
            .collect(),
        facts: snapshot
            .facts()
            .map(|((node_id, attempt_id, fact_key), fact)| {
                (
                    (node_id.clone(), attempt_id.clone(), fact_key.clone()),
                    fact.clone(),
                )
            })
            .collect(),
        side_effects: snapshot
            .side_effects()
            .map(|(ledger_ref, side_effect)| (ledger_ref.clone(), side_effect.clone()))
            .collect(),
        resource_lanes,
        public_outputs: snapshot
            .public_outputs()
            .map(|(schema_id, output)| (schema_id.clone(), output.clone()))
            .collect(),
        retentions: snapshot
            .retentions()
            .map(|(run_id, retention)| (run_id.clone(), retention.clone()))
            .collect(),
    })
}

fn api_error_from_typed_artifact_error(error: FsTypedArtifactError) -> ApiError {
    match error {
        FsTypedArtifactError::NotFound { .. } => ApiError::backend(
            StatusCode::NOT_FOUND,
            "ArtifactNotFound",
            "Typed artifact was not found",
        ),
        FsTypedArtifactError::InvalidEvidence { .. }
        | FsTypedArtifactError::InvalidIdentity { .. }
        | FsTypedArtifactError::EvidenceMismatch { .. } => ApiError::backend(
            StatusCode::CONFLICT,
            "ArtifactRejected",
            "Typed artifact evidence was rejected",
        ),
        FsTypedArtifactError::RetainedArtifactRefused { .. } => ApiError::backend(
            StatusCode::CONFLICT,
            "ArtifactRetained",
            "Typed artifact is retained",
        ),
        FsTypedArtifactError::Corruption { .. } => ApiError::backend(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ArtifactCorruption",
            "Typed artifact store returned invalid data",
        ),
        FsTypedArtifactError::Io { .. } => ApiError::backend(
            StatusCode::SERVICE_UNAVAILABLE,
            "ArtifactStoreUnavailable",
            "Typed artifact store is unavailable",
        ),
    }
}

impl From<FsTypedArtifactError> for ApiError {
    fn from(value: FsTypedArtifactError) -> Self {
        api_error_from_typed_artifact_error(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    struct FailingSerialize;

    impl Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("boom"))
        }
    }

    #[test]
    fn serialize_response_returns_api_error_instead_of_panicking() {
        let err = serialize_response(FailingSerialize)
            .expect_err("serialization failures should be returned as api errors");

        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.code, "SerializationError");
        assert_eq!(err.message, "Failed to serialize response payload");
    }

    #[test]
    fn invalid_run_id_is_typed_error() {
        let err = parse_run_id("not-a-uuid").expect_err("dynamic ids are rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "InvalidRunId");
    }

    #[tokio::test]
    async fn evm_contract_routes_accept_append_only_starts() {
        for (route, body) in [
            (
                "/v1/evm/contracts/deploy",
                json!({
                    "kind": "evm_contract_deploy_start_v1",
                    "config": deploy_config_json(),
                    "drive": "append_only"
                }),
            ),
            (
                "/v1/evm/contracts/configure",
                json!({
                    "kind": "evm_contract_configure_start_v1",
                    "config": configure_config_json(),
                    "deployed": deployed_contract_json(),
                    "drive": "append_only"
                }),
            ),
            (
                "/v1/evm/contracts/validate",
                json!({
                    "kind": "evm_contract_validate_start_v1",
                    "config": validate_config_json(),
                    "configured": configured_contract_json(),
                    "drive": "append_only"
                }),
            ),
            (
                "/v1/evm/contracts/lifecycle",
                json!({
                    "kind": "evm_contract_lifecycle_start_v1",
                    "config": {
                        "deploy": deploy_config_json(),
                        "configure": configure_config_json(),
                        "validate": validate_config_json()
                    },
                    "drive": "append_only"
                }),
            ),
        ] {
            let response = test_app()
                .oneshot(json_post(route, body))
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::OK, "route {route}");
            let value = response_json(response).await;
            assert_eq!(value["status"], "success");
            assert_eq!(value["data"]["run"]["run_mode"], "forward");
            assert!(
                value["data"]["run"]["attempt_dispositions"].is_array(),
                "route {route} must expose attempt-level dispositions"
            );
            assert_eq!(value["data"]["public_output"], serde_json::Value::Null);
        }
    }

    #[tokio::test]
    async fn manual_resolution_route_accepts_signed_proof_submission_shape() {
        let run_id = mfm_app::new_run_id();
        let response = test_app()
            .oneshot(json_post(
                &format!("/v1/runs/{run_id}/manual-resolution"),
                json!({
                    "kind": "manual_resolution_v1",
                    "outcome": "confirm_remediated",
                    "evidence_json": {
                        "operator_note": "reviewed"
                    },
                    "authorization_proof": {
                        "proof_version": "mfm.manual_resolution.authorization_proof.v1"
                    },
                    "drive": "append_only"
                }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let value = response_json(response).await;
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "RunNotFound");
    }

    #[tokio::test]
    async fn manual_resolution_route_rejects_request_supplied_prefix_facts() {
        let run_id = mfm_app::new_run_id();
        let response = test_app()
            .oneshot(json_post(
                &format!("/v1/runs/{run_id}/manual-resolution"),
                json!({
                    "kind": "manual_resolution_v1",
                    "outcome": "confirm_remediated",
                    "evidence_json": {},
                    "authorization_proof": {},
                    "expected_next_seq": 2
                }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = response_json(response).await;
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "InvalidJson");
    }

    #[test]
    fn success_response_preserves_run_status_attempt_and_saga_contract() {
        let response = ok(json!({
            "run": {
                "run_id": "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "spec_hash": "spec:sha256-jcs-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "run_mode": "failed_without_acdc_claim",
                "saga": {
                    "policy": {
                        "variant": "compensate_completed",
                        "manual_authorization": null,
                        "on_remediation_unresolved": "manual_resolution"
                    },
                    "obligations": [],
                    "resource_ledgers": [],
                    "resource_lanes": [{
                        "namespace": "mfm.test.account_nonce",
                        "key": "wallet-1",
                        "holding_run_id": "run:sha256-jcs-v1:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                        "holding_ledger_key": "ledger-forward",
                        "holding_ledger_purpose": "forward",
                        "holding_forward_ledger_key": null,
                        "holding_node_id": "node:sha256-jcs-v1:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                        "holding_attempt_id": "attempt:sha256-jcs-v1:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                        "invocation_epoch": 1
                    }],
                    "manual_block_reason": "remediation_ambiguous",
                    "required_manual_authorization": null,
                    "terminal_resolution": {
                        "outcome": "failed_without_acdc_claim",
                        "claim": "none"
                    }
                },
                "attempt_dispositions": [
                    {
                        "node_id": "node:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111",
                        "attempt_id": "attempt:sha256-jcs-v1:2222222222222222222222222222222222222222222222222222222222222222",
                        "disposition": "started",
                        "attempt_no": 1,
                        "retryable": null,
                        "output_cell_id": null
                    },
                    {
                        "node_id": "node:sha256-jcs-v1:3333333333333333333333333333333333333333333333333333333333333333",
                        "attempt_id": "attempt:sha256-jcs-v1:4444444444444444444444444444444444444444444444444444444444444444",
                        "disposition": "interrupted",
                        "attempt_no": null,
                        "retryable": null,
                        "output_cell_id": null
                    },
                    {
                        "node_id": "node:sha256-jcs-v1:5555555555555555555555555555555555555555555555555555555555555555",
                        "attempt_id": "attempt:sha256-jcs-v1:6666666666666666666666666666666666666666666666666666666666666666",
                        "disposition": "failed",
                        "attempt_no": null,
                        "retryable": false,
                        "output_cell_id": null
                    },
                    {
                        "node_id": "node:sha256-jcs-v1:7777777777777777777777777777777777777777777777777777777777777777",
                        "attempt_id": "attempt:sha256-jcs-v1:8888888888888888888888888888888888888888888888888888888888888888",
                        "disposition": "completed",
                        "attempt_no": null,
                        "retryable": null,
                        "output_cell_id": "cell:sha256-jcs-v1:9999999999999999999999999999999999999999999999999999999999999999"
                    }
                ],
                "scheduler_status": "blocked",
                "head_seq": 42
            }
        }));

        assert_eq!(response["status"], "success");
        let dispositions = response["data"]["run"]["attempt_dispositions"]
            .as_array()
            .expect("attempt dispositions");
        assert_eq!(dispositions.len(), 4);
        assert_eq!(dispositions[0]["disposition"], "started");
        assert_eq!(dispositions[1]["disposition"], "interrupted");
        assert_eq!(dispositions[2]["disposition"], "failed");
        assert_eq!(dispositions[3]["disposition"], "completed");
        assert_eq!(
            response["data"]["run"]["saga"]["resource_lanes"][0]["key"],
            "wallet-1"
        );
        assert_eq!(
            response["data"]["run"]["saga"]["manual_block_reason"],
            "remediation_ambiguous"
        );
        assert_eq!(
            response["data"]["run"]["saga"]["terminal_resolution"]["outcome"],
            "failed_without_acdc_claim"
        );
    }

    #[tokio::test]
    async fn removed_legacy_contract_route_is_not_found() {
        let route = ["/v1/evm/", "d", "c", "v", "/deploy"].concat();
        let response = test_app()
            .oneshot(json_post(
                &route,
                json!({
                    "kind": "evm_contract_deploy_start_v1",
                    "config": deploy_config_json(),
                    "drive": "append_only"
                }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn removed_legacy_contract_start_kind_rejects() {
        let removed_kind = ["evm_", "d", "c", "v", "_deploy_start_v1"].concat();
        let response = test_app()
            .oneshot(json_post(
                "/v1/evm/contracts/deploy",
                json!({
                    "kind": removed_kind,
                    "config": deploy_config_json(),
                    "drive": "append_only"
                }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = response_json(response).await;
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "InvalidJson");
    }

    #[test]
    fn run_routes_do_not_import_dynamic_semantic_surfaces() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        )
        .expect("read rest source");
        let forbidden = [
            format!("mfm_{}", "sdk"),
            format!("mfm_{}", "machine"),
            format!("mfm_app_{}", "legacy"),
            format!("Runs{}Request", "Start"),
            format!("{}line", "Pipe"),
            format!("Port{}", "Key"),
            format!("Dyn{}", "Context"),
            format!("State{}", "Graph"),
            format!("Dependency{}", "Edge"),
        ];

        for needle in forbidden {
            assert!(
                !source.contains(&needle),
                "REST typed run surface must not mention dynamic semantic surface `{needle}`"
            );
        }
    }

    fn test_app() -> axum::Router {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("mfm-rest-api-contract-test-{unique}"));
        std::fs::create_dir_all(&root).expect("artifact root");
        make_app(make_in_memory_app_state(root))
    }

    fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    }

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("response json")
    }

    fn deploy_config_json() -> serde_json::Value {
        json!({
            "network": network_json(),
            "signer": signer_json()
        })
    }

    fn configure_config_json() -> serde_json::Value {
        json!({
            "network": network_json(),
            "signer": signer_json(),
            "calls": []
        })
    }

    fn validate_config_json() -> serde_json::Value {
        json!({
            "network": network_json()
        })
    }

    fn network_json() -> serde_json::Value {
        json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1
        })
    }

    fn signer_json() -> serde_json::Value {
        json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead"
        })
    }

    fn deployed_contract_json() -> serde_json::Value {
        json!({
            "lifecycle_version": 1,
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "contract_address": "0x000000000000000000000000000000000000c0de",
            "deploy_tx_hash": "0xabc123",
            "deploy_receipt_evidence": null,
            "deployed_block_number": 100
        })
    }

    fn configured_contract_json() -> serde_json::Value {
        json!({
            "lifecycle_version": 1,
            "deployed": deployed_contract_json(),
            "configure_calls": [],
            "confirmation_read_assertions": [],
            "confirmation_event_assertions": [],
            "configure_tx_hashes": [],
            "configure_receipt_evidence": [],
            "configured_block_number": 101
        })
    }
}
