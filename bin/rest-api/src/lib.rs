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
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use http::header::HeaderName;
use mfm_app::{
    AppError, EntryPointRunLaunchInput, ErrorClass, InvocationKey, ManualResolutionDecision,
    ManualResolutionRecordRequest, ProductionRunStore, PublicFactQueryRequest, PublicFactRefId,
    PublicOpName, PublicOutputResponse, PublicSafeMessage, RunLaunchOutcomeStatus, RunReadServices,
    RunResponse, RunServices, RunStreamResponse,
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
use url::form_urlencoded;

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
    /// Optional runtime configuration file path for live capability-backed runs.
    pub runtime_config_path: Option<PathBuf>,
    /// Optional fact-query receipt trust root used for replay verification.
    pub fact_query_receipt_trust_root: Option<store::FactQueryReceiptTrustRoot>,
}

#[derive(Clone)]
struct RouterState<S> {
    app: AppState<S>,
}

trait RunCommandStore:
    RunEventStore
    + store::StoreScopeStore
    + store::ExecutionClaimStore
    + store::RetainedArtifactReadProvider
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<S> RunCommandStore for S where
    S: RunEventStore
        + store::StoreScopeStore
        + store::ExecutionClaimStore
        + store::RetainedArtifactReadProvider
        + Clone
        + Send
        + Sync
        + 'static
{
}

trait RunObservationCommandStore:
    RunCommandStore + RunObservationStore<Error = <Self as RunEventStore>::Error>
{
}

impl<S> RunObservationCommandStore for S where
    S: RunCommandStore + RunObservationStore<Error = <S as RunEventStore>::Error>
{
}

impl<S> RouterState<S>
where
    S: RunCommandStore,
{
    fn live_services(&self) -> Result<RunServices<S, S>, ApiError> {
        let runners = mfm_app::production_runner_registry(
            Arc::new(self.app.store.clone()),
            self.app.runtime_config_path.as_deref(),
        )?;
        let certification_registry = mfm_app::production_certification_registry()?;
        let fact_query_receipt_trust_root = self.app.fact_query_receipt_trust_root.clone();
        Ok(
            mfm_app::make_run_services_with_certification_registry_and_fact_query_trust_root(
                runners,
                self.app.store.clone(),
                self.app.store.clone(),
                certification_registry,
                fact_query_receipt_trust_root,
            ),
        )
    }

    fn read_services(&self) -> Result<RunReadServices<S, S>, ApiError> {
        let certification_registry = mfm_app::production_certification_registry()?;
        let fact_query_receipt_trust_root = self.app.fact_query_receipt_trust_root.clone();
        Ok(
            mfm_app::make_run_read_services_with_certification_registry_and_fact_query_trust_root(
                self.app.store.clone(),
                self.app.store.clone(),
                certification_registry,
                fact_query_receipt_trust_root,
            ),
        )
    }
}

/// Connects to the default certified run store.
pub async fn make_default_run_store() -> Result<ProductionRunStore, ApiError> {
    Ok(mfm_app::connect_production_run_store_with_optional_fact_query_signer(None).await?)
}

/// Builds default production REST API state from environment-selected stores.
pub async fn make_default_app_state() -> Result<DefaultAppState, ApiError> {
    let store = make_default_run_store().await?;
    let fact_query_receipt_trust_root = store.store_authority().fact_receipt_trust_root().cloned();
    Ok(AppState {
        store,
        runtime_config_path: std::env::var_os(mfm_app::MFM_RUNTIME_CONFIG_FILE).map(PathBuf::from),
        fact_query_receipt_trust_root,
    })
}

/// Builds the `axum` router for the public REST API surface.
pub fn make_app<S>(state: AppState<S>) -> Router
where
    S: RunEventStore
        + store::StoreScopeStore
        + store::ExecutionClaimStore
        + RunObservationStore<Error = <S as RunEventStore>::Error>
        + store::RetainedArtifactReadProvider
        + mfm_app::PublicFactQueryExecutor
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
        .route("/v1/facts/kinds", get(facts_kinds::<S>))
        .route("/v1/facts/kinds/:kind", get(facts_describe_kind::<S>))
        .route("/v1/facts/ref/:public_ref", get(facts_ref::<S>))
        .route("/v1/facts/:kind/latest", get(facts_latest::<S>))
        .route("/v1/facts/:kind", get(facts_query::<S>))
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
    S: RunCommandStore,
{
    state.app.store.load_store_scope_id().await.map_err(|_| {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunStartBody {
    op: String,
    #[serde(default)]
    op_version: Option<u32>,
    #[serde(default)]
    config_format: Option<RestConfigFormat>,
    config: serde_json::Value,
    #[serde(default)]
    invocation_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunStartResponse {
    outcome: RunLaunchOutcomeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<RunResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
struct RunResumeBody {}

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

#[instrument(level = "debug", skip(state))]
async fn facts_kinds<S>(
    State(state): State<RouterState<S>>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore,
{
    json_ok(state.read_services()?.fact_kinds().await?)
}

#[instrument(level = "debug", skip(state), fields(kind = kind.as_str()))]
async fn facts_describe_kind<S>(
    State(state): State<RouterState<S>>,
    Path(kind): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore,
{
    json_ok(state.read_services()?.describe_fact_kind(&kind).await?)
}

#[instrument(level = "debug", skip(state, query), fields(kind = kind.as_str()))]
async fn facts_query<S>(
    State(state): State<RouterState<S>>,
    Path(kind): Path<String>,
    query: RawQuery,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore + mfm_app::PublicFactQueryExecutor,
{
    facts_query_response(&state, kind, query, None).await
}

#[instrument(level = "debug", skip(state, query), fields(kind = kind.as_str()))]
async fn facts_latest<S>(
    State(state): State<RouterState<S>>,
    Path(kind): Path<String>,
    query: RawQuery,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore + mfm_app::PublicFactQueryExecutor,
{
    facts_query_response(&state, kind, query, Some(1)).await
}

async fn facts_query_response<S>(
    state: &RouterState<S>,
    kind: String,
    query: RawQuery,
    forced_limit: Option<u64>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore + mfm_app::PublicFactQueryExecutor,
{
    let request = public_fact_query_request(kind, query, forced_limit)?;
    json_ok(state.read_services()?.query_public_facts(request).await?)
}

#[instrument(level = "debug", skip(state), fields(public_ref = public_ref.as_str()))]
async fn facts_ref<S>(
    State(state): State<RouterState<S>>,
    Path(public_ref): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore,
{
    let public_ref = PublicFactRefId::new(public_ref)?;
    json_ok(
        state
            .read_services()?
            .resolve_public_fact_ref(&public_ref)
            .await?,
    )
}

fn public_fact_query_request(
    kind: String,
    query: RawQuery,
    forced_limit: Option<u64>,
) -> Result<PublicFactQueryRequest, ApiError> {
    let pairs = query
        .0
        .as_deref()
        .map(|raw| form_urlencoded::parse(raw.as_bytes()))
        .into_iter()
        .flatten();
    PublicFactQueryRequest::from_query_pairs(kind, pairs, forced_limit)
        .map_err(fact_query_params_api_error)
}

fn fact_query_params_api_error(error: AppError) -> ApiError {
    if mfm_app::is_public_fact_query_parameter_error(&error) {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidQuery",
            "Failed to parse fact query parameters",
        )
    } else {
        ApiError::from(error)
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
    S: RunObservationCommandStore,
{
    let Query(query) = query.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidQuery",
            "Failed to parse run list query",
        )
    })?;
    let services = state.read_services()?;
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
    S: RunCommandStore,
{
    let Json(req) = body?;
    let services = state.live_services()?;
    let store_scope_id = services.load_store_scope_id().await?;
    let entry_point_registry = mfm_app::production_entry_point_op_registry()?;
    let public_op_name = PublicOpName::new(&req.op)?;
    let op_version = req.op_version.map(mfm_app::OpVersion::new).transpose()?;
    let invocation_key = req.invocation_key.map(InvocationKey::new).transpose()?;
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
        store_scope_id,
        invocation_key,
    })?;
    let report = services.launch_prepared_entry_point_run(prepared).await?;

    json_ok(RunStartResponse {
        outcome: report.outcome,
        run: report.run,
        active_run_id: report.active_run_id,
        public_output: report.public_output,
    })
}

#[instrument(level = "info", skip(state, body), fields(run_id = run_id.as_str()))]
async fn runs_resume<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore,
{
    let run_id = parse_run_id(&run_id)?;
    let _req: RunResumeBody = parse_optional_body(body)?;
    let data = state.live_services()?.resume_stored_run(&run_id).await?;

    json_ok(data)
}

#[instrument(level = "info", skip(state, body), fields(run_id = run_id.as_str()))]
async fn runs_manual_resolution<S>(
    State(state): State<RouterState<S>>,
    Path(run_id): Path<String>,
    body: Result<Json<ManualResolutionBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError>
where
    S: RunCommandStore,
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
        .live_services()?
        .record_manual_resolution(ManualResolutionRecordRequest {
            run_id,
            outcome: req.outcome,
            evidence_bytes,
            evidence_media_type: req.evidence_media_type,
            authorization_proof_bytes: proof_bytes,
            note: req.note,
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
    S: RunCommandStore,
{
    let run_id = parse_run_id(&run_id)?;
    let data = state.read_services()?.run_status(&run_id).await?;

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
    S: RunCommandStore,
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
    let response = state.read_services()?.run_stream(&run_id).await?;
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
    S: RunCommandStore,
{
    let run_id = parse_run_id(&run_id)?;
    let data = state
        .read_services()?
        .verify_replay_for_run(&run_id)
        .await?;

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
    S: RunCommandStore,
{
    let run_id = parse_run_id(&run_id)?;
    let schema_id = parse_schema_id(&schema_id)?;
    let data = state
        .read_services()?
        .public_output(&run_id, &schema_id)
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
mod tests;
