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

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use http::header::HeaderName;
use mfm_app::{
    AppError, DriveMode, ErrorClass, TypedAsyncAppServices, TypedPublicOutputResponse,
    TypedRunPhase, TypedRunResponse, TypedRunStreamResponse, TypedSeedInput,
};
use mfm_artifact_store_fs::{FsTypedArtifactError, FsTypedArtifactStore, TypedArtifactDescriptor};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1::ArtifactRole;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, RunId, SchemaId, SeedId};
use mfm_op_portfolio_tracker::{
    certified_portfolio_spec, portfolio_config_artifacts_for_spec, portfolio_program_draft,
    PortfolioConfigArtifact,
};
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, parse_portfolio_snapshot_authored_config,
    AuthoredConfigFormat, PortfolioSnapshotConfigError,
};
use mfm_spec::v1 as spec;
use mfm_state_portfolio::PortfolioWorkflowConfig;
use mfm_store::v1 as store;
use mfm_store::v1::{AsyncStoreFuture, AsyncTypedRunEventStore, TypedRunEventStore};
use mfm_stream_store_postgres::{PostgresTypedRunEventStore, PostgresTypedStoreError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;
use tracing::instrument;

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
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
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

    fn record_artifact_evidence<'a>(
        &'a self,
        evidence: store::ArtifactEvidenceRef,
    ) -> AsyncStoreFuture<'a, (), Self::Error> {
        let result = self
            .inner
            .lock()
            .expect("typed run store lock")
            .record_artifact_evidence(evidence);
        Box::pin(std::future::ready(result))
    }

    fn append_typed_run_commit<'a>(
        &'a self,
        request: store::TypedCommitRequest,
    ) -> AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = self
            .inner
            .lock()
            .expect("typed run store lock")
            .append_typed_run_commit(request);
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
    fn services(&self) -> Result<TypedAsyncAppServices<S>, ApiError> {
        let runners = mfm_app::production_typed_runner_registry(self.app.artifacts.clone())?;
        Ok(mfm_app::make_async_typed_services(
            runners,
            self.app.store.clone(),
            self.app.artifacts.clone(),
        ))
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
    S: AsyncTypedRunEventStore + Clone + Send + Sync + 'static,
{
    let request_id_header = HeaderName::from_static("x-request-id");
    let make_span_header = request_id_header.clone();
    let state = RouterState { app: state };

    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/ready", get(ready::<S>))
        .route("/v1/portfolio/snapshot", post(portfolio_snapshot::<S>))
        .route("/v1/runs/start", post(runs_start::<S>))
        .route("/v1/runs/:run_id/resume", post(runs_resume::<S>))
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
enum PortfolioSnapshotStartKind {
    PortfolioSnapshotStartV1,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RestDriveMode {
    AppendOnly,
    #[default]
    UntilBlocked,
}

impl RestDriveMode {
    fn into_app(self) -> DriveMode {
        match self {
            Self::AppendOnly => DriveMode::AppendOnly,
            Self::UntilBlocked => DriveMode::UntilBlocked,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedRunStartBody {
    kind: TypedRunStartKind,
    spec: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
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

#[derive(Debug, Serialize)]
struct PortfolioSnapshotStartResponse {
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
struct RunStreamQuery {
    #[serde(default = "default_from_seq")]
    from_seq: u64,
    #[serde(default)]
    to_seq: Option<u64>,
}

fn default_from_seq() -> u64 {
    1
}

fn default_framework_version() -> String {
    "mfm.rest_api.typed.v1".to_owned()
}

fn default_portfolio_framework_version() -> String {
    "mfm.rest_api.portfolio.typed.v1".to_owned()
}

#[allow(clippy::disallowed_methods)]
fn default_source_revision() -> String {
    std::env::var("MFM_SOURCE_REVISION").unwrap_or_else(|_| "unknown".to_owned())
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
    let draft = portfolio_program_draft(workflow_config.clone()).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "TypedPortfolioPlanInvalid",
            error.to_string(),
        )
    })?;
    let certified = certified_portfolio_spec(workflow_config).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "TypedPortfolioSpecInvalid",
            error.to_string(),
        )
    })?;
    let public_schema_id = certified
        .envelope
        .spec
        .public_outputs
        .public_schema_id
        .clone();
    let configs =
        portfolio_config_artifacts_for_spec(&draft, &certified.envelope.spec).map_err(|error| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "TypedPortfolioConfigInvalid",
                error.to_string(),
            )
        })?;

    let run_id = parse_optional_run_id(req.run_id)?;
    let spec_bytes = certified
        .envelope
        .spec
        .canonical_json()
        .map_err(|error| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "TypedPortfolioSpecInvalid",
                error.to_string(),
            )
        })?
        .to_vec();
    let services = state.services()?;
    persist_portfolio_config_artifacts(services.artifacts(), configs).await?;
    let start = mfm_app::build_typed_run_start_request(
        services.artifacts(),
        &spec_bytes,
        run_id.clone(),
        &req.framework_version,
        &req.source_revision,
        Vec::new(),
        req.drive.into_app(),
    )
    .await?;
    let run = services.start_certified_run(start).await?;
    let public_output = if run.phase == TypedRunPhase::Completed {
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
    let spec_bytes = canonical_json_value_bytes(&req.spec, "TypedSpecInvalid")?;
    let seed_media_type = mfm_app::json_media_type()?;
    let mut seeds = Vec::with_capacity(req.seeds.len());
    for seed in req.seeds {
        seeds.push(TypedSeedInput {
            seed_id: parse_seed_id(&seed.seed_id)?,
            bytes: canonical_json_value_bytes(&seed.json, "TypedSeedInvalid")?,
            media_type: seed_media_type.clone(),
        });
    }

    let services = state.services()?;
    let start = mfm_app::build_typed_run_start_request(
        services.artifacts(),
        &spec_bytes,
        run_id,
        &req.framework_version,
        &req.source_revision,
        seeds,
        req.drive.into_app(),
    )
    .await?;
    let data = services.start_certified_run(start).await?;

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

#[instrument(level = "debug", skip(state), fields(run_id = run_id.as_str()))]
async fn runs_status<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: AsyncTypedRunEventStore + Clone + Send + Sync,
{
    let run_id = parse_run_id(&run_id)?;
    let data = state.services()?.run_status(&run_id).await?;

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
        .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, error_code, error.to_string()))
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

async fn persist_portfolio_config_artifacts(
    artifacts: &FsTypedArtifactStore,
    configs: Vec<PortfolioConfigArtifact>,
) -> Result<(), ApiError> {
    for config in configs {
        artifacts
            .put_artifact(
                config.bytes,
                TypedArtifactDescriptor {
                    media_type: config.media_type,
                    schema_id: Some(config.schema_id),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: ArtifactRole::TypedConfig,
                },
            )
            .await
            .map_err(api_error_from_typed_artifact_error)?;
    }
    Ok(())
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
        | PortfolioSnapshotConfigError::CanonicalJson { .. } => ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidPortfolioRequest",
            error.to_string(),
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
        media_type: spec::MediaType::new("application/json").map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "TypedJsonMediaTypeInvalid",
                error.to_string(),
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
        PostgresTypedStoreError::Store(error) => ApiError::new(
            StatusCode::CONFLICT,
            "TypedStoreRejected",
            error.to_string(),
        ),
        PostgresTypedStoreError::Database(message) => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "TypedStoreUnavailable",
            message,
        ),
        PostgresTypedStoreError::DatabaseSource { context, source } => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "TypedStoreUnavailable",
            format!("{context}: {source}"),
        ),
        PostgresTypedStoreError::Corruption(message) => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "TypedStoreCorruption",
            message,
        ),
    }
}

fn api_error_from_typed_artifact_error(error: FsTypedArtifactError) -> ApiError {
    match error {
        FsTypedArtifactError::NotFound { .. } => ApiError::new(
            StatusCode::NOT_FOUND,
            "TypedArtifactNotFound",
            error.to_string(),
        ),
        FsTypedArtifactError::InvalidEvidence { .. }
        | FsTypedArtifactError::InvalidIdentity { .. }
        | FsTypedArtifactError::EvidenceMismatch { .. } => ApiError::new(
            StatusCode::CONFLICT,
            "TypedArtifactRejected",
            error.to_string(),
        ),
        FsTypedArtifactError::RetainedArtifactRefused { .. } => ApiError::new(
            StatusCode::CONFLICT,
            "TypedArtifactRetained",
            error.to_string(),
        ),
        FsTypedArtifactError::Corruption { .. } => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "TypedArtifactCorruption",
            error.to_string(),
        ),
        FsTypedArtifactError::Io { .. } => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "TypedArtifactStoreUnavailable",
            error.to_string(),
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
}
