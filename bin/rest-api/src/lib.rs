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
    AppError, DriveMode, EntryPointRunLaunchInput, ErrorClass, ManualResolutionDecision,
    ManualResolutionRecordRequest, ProductionRunStore, PublicOpName, PublicOutputResponse,
    PublicSafeMessage, RunModeStatus, RunResponse, RunServices, RunStreamResponse,
};
use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{RunId, SchemaId};
use mfm_store::v1 as store;
use mfm_store::v1::{RunEventStore, RunObservationStore};
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
#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}: {message}")]
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

impl From<mfm_app::EntryPointOpResolveError> for ApiError {
    fn from(error: mfm_app::EntryPointOpResolveError) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            error.code().to_owned(),
            PublicSafeMessage::new(error.message().to_owned()),
        )
    }
}

impl From<mfm_app::OpLaunchError> for ApiError {
    fn from(error: mfm_app::OpLaunchError) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            error.code().to_owned(),
            PublicSafeMessage::new(error.message().to_owned()),
        )
    }
}

impl From<mfm_authored_config::AuthoredConfigError> for ApiError {
    fn from(error: mfm_authored_config::AuthoredConfigError) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            error.code().to_owned(),
            PublicSafeMessage::new(error.message().to_owned()),
        )
    }
}

impl From<JsonRejection> for ApiError {
    fn from(_error: JsonRejection) -> Self {
        Self::invalid_json()
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(err(self.code, self.message))).into_response()
    }
}

/// Default production REST API state.
pub type DefaultAppState = AppState<ProductionRunStore>;

/// Shared router state injected into request handlers.
#[derive(Clone)]
pub struct AppState<S = ProductionRunStore> {
    /// Certified typed run-event and artifact authority store.
    pub store: S,
}

#[derive(Clone)]
struct RouterState<S> {
    app: AppState<S>,
}

impl<S> RouterState<S>
where
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    fn services(&self) -> Result<RunServices<S, S>, ApiError> {
        let runners = mfm_app::production_runner_registry(
            mfm_app::artifact_read_provider_from_retained(self.app.store.clone()),
        )?;
        let certification_registry = mfm_app::production_certification_registry()?;
        Ok(mfm_app::make_run_services_with_certification_registry(
            runners,
            self.app.store.clone(),
            self.app.store.clone(),
            certification_registry,
        ))
    }
}

/// Connects to the default certified run store.
pub async fn make_default_run_store() -> Result<ProductionRunStore, ApiError> {
    Ok(mfm_app::connect_production_run_store(None).await?)
}

/// Builds default production REST API state from environment-selected stores.
pub async fn make_default_app_state() -> Result<DefaultAppState, ApiError> {
    Ok(AppState {
        store: make_default_run_store().await?,
    })
}

/// Builds the `axum` router for the public REST API surface.
pub fn make_app<S>(state: AppState<S>) -> Router
where
    S: RunEventStore
        + RunObservationStore<Error = <S as RunEventStore>::Error>
        + store::RetainedArtifactReadProvider
        + Clone
        + Send
        + Sync
        + 'static,
{
    let request_id_header = HeaderName::from_static("x-request-id");
    let make_span_header = request_id_header.clone();
    let state = RouterState { app: state };

    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/ready", get(ready::<S>))
        .route("/v1/runs", get(runs_list::<S>))
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
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
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
                "run store is not ready",
            )
        })?;

    Ok(Json(ok(json!({
      "ok": true,
      "checks": {
        "run_store": "ready"
      }
    }))))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(err("not_found", "not found")))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManualResolutionKind {
    ManualResolutionV1,
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
struct RunStartBody {
    op: String,
    #[serde(default)]
    op_version: Option<u32>,
    #[serde(default)]
    config_format: Option<RestConfigFormat>,
    config: serde_json::Value,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    drive: RestDriveMode,
}

#[derive(Debug, Serialize)]
struct RunStartResponse {
    run: RunResponse,
    public_output: Option<PublicOutputResponse>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RestConfigFormat {
    Toml,
    Json,
}

impl From<RestConfigFormat> for AuthoredConfigFormat {
    fn from(value: RestConfigFormat) -> Self {
        match value {
            RestConfigFormat::Toml => Self::Toml,
            RestConfigFormat::Json => Self::Json,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunResumeBody {
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunListQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    wait_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct RunObservationPageResponse {
    next_cursor: String,
    runs: Vec<RunObservationResponse>,
}

#[derive(Debug, Serialize)]
struct RunObservationResponse {
    run_id: String,
    head_seq: u64,
    observed_status: String,
    started_at: String,
    updated_at: String,
    completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    change_id: Option<String>,
}

impl From<store::RunObservationPage> for RunObservationPageResponse {
    fn from(page: store::RunObservationPage) -> Self {
        Self {
            next_cursor: page.next_cursor,
            runs: page
                .runs
                .into_iter()
                .map(RunObservationResponse::from)
                .collect(),
        }
    }
}

impl From<store::RunObservation> for RunObservationResponse {
    fn from(row: store::RunObservation) -> Self {
        Self {
            run_id: row.run_id.to_string(),
            head_seq: row.head_seq.as_u64(),
            observed_status: row.observed_status.as_str().to_owned(),
            started_at: row.started_at,
            updated_at: row.updated_at,
            completed_at: row.completed_at,
            change_id: row.change_id,
        }
    }
}

fn default_from_seq() -> u64 {
    1
}

fn default_json_media_type_string() -> String {
    "application/json".to_owned()
}

#[instrument(level = "debug", skip(state, query))]
async fn runs_list<S>(
    State(state): State<RouterState<S>>,
    query: Result<Query<RunListQuery>, QueryRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunEventStore
        + RunObservationStore<Error = <S as RunEventStore>::Error>
        + store::RetainedArtifactReadProvider
        + Clone
        + Send
        + Sync
        + 'static,
{
    let Query(query) = query.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidQuery",
            "Failed to parse run list query",
        )
    })?;
    let services = state.services()?;
    let page = services
        .read_run_observations(store::RunObservationQuery::new(
            query.cursor,
            query.limit.unwrap_or(50),
            query.wait_ms.unwrap_or(0),
        ))
        .await?;
    json_ok(RunObservationPageResponse::from(page))
}

#[instrument(level = "info", skip(state, body))]
async fn runs_start<S>(
    State(state): State<RouterState<S>>,
    body: Result<Json<RunStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    let Json(req) = body?;
    let run_id = parse_optional_run_id(req.run_id)?;
    let services = state.services()?;
    let entry_point_registry = mfm_app::production_entry_point_op_registry()?;
    let public_op_name = PublicOpName::new(&req.op)?;
    let op_version = req.op_version.map(mfm_app::OpVersion::new).transpose()?;
    let authored_config = AuthoredConfig::from_json_transport_value(
        req.config_format.map(AuthoredConfigFormat::from),
        &req.config,
    )?;
    let prepared = mfm_app::prepare_entry_point_run_launch(EntryPointRunLaunchInput {
        entry_point_registry: &entry_point_registry,
        public_op_name,
        op_version,
        authored_config,
        certification_registry: services.certification_registry(),
        run_id: run_id.clone(),
        drive: req.drive.into_app(),
    })?;
    let public_output_schema_id = prepared
        .request
        .certified_spec
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone();
    let run = services.launch_run(prepared.request).await?;
    let public_output = if run.run_mode == RunModeStatus::Completed {
        Some(
            services
                .public_output(&run_id, &public_output_schema_id)
                .await?,
        )
    } else {
        None
    };

    json_ok(RunStartResponse { run, public_output })
}

#[instrument(level = "info", skip(state, body), fields(run_id = run_id.as_str()))]
async fn runs_resume<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    let run_id = parse_run_id(&run_id)?;
    let req: RunResumeBody = parse_optional_body(body)?;
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
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    let run_id = parse_run_id(&run_id)?;
    let Json(req) = body?;
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
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
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
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
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
    let data = RunStreamResponse {
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
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
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
    S: RunEventStore + store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    let run_id = parse_run_id(&run_id)?;
    let schema_id = parse_schema_id(&schema_id)?;
    let data = state.services()?.public_output(&run_id, &schema_id).await?;

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
    fn invalid_run_id_has_domain_error() {
        let err = parse_run_id("not-a-uuid").expect_err("dynamic ids are rejected");

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.code, "InvalidRunId");
    }

    #[tokio::test]
    async fn run_start_accepts_entry_point_shape() {
        let response = test_app()
            .oneshot(json_post(
                "/v1/runs/start",
                json!({
                    "op": "missing_entry_point_op",
                    "config": "portfolio_id = \"main\"\n",
                    "drive": "append_only"
                }),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let value = response_json(response).await;
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "EntryPointOpNotFound");
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
        make_app(AppState {
            store: store::AsyncInMemoryRunStore::default(),
        })
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
}
