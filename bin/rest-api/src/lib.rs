#![warn(missing_docs)]
//! REST API wiring for certified typed MFM runs.
//!
//! This crate adapts the opaque [`mfm_app::Application`] facade onto an `axum` router. It is a
//! transport layer only: it decodes HTTP input, applies route admission, invokes app operations,
//! and renders the current public envelopes.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_rest_api::{make_app, make_default_app_state};
//!
//! async fn build_router() -> Result<axum::Router, mfm_rest_api::ApiError> {
//!     let state = make_default_app_state(None).await?;
//!     Ok(make_app(state))
//! }
//! ```

use std::path::PathBuf;
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
    Application, ErrorClass, InvocationKey, ManualResolutionDecision,
    ManualResolutionRecordRequest, PublicError, PublicFactQueryRequest, PublicFactRefId,
    PublicOutputResponse, RunLaunchOutcomeStatus, RunObservation, RunObservationPage, RunResponse,
    RunStreamResponse,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{RunId, SchemaId};
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

fn serialize_response<T: Serialize>(data: T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(data).map_err(|_| {
        ApiError::new(
            ErrorClass::Internal,
            "SerializationError",
            "Failed to serialize response payload",
        )
    })
}

fn json_ok<T: Serialize>(data: T) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(ok(serialize_response(data)?)))
}

/// Thin HTTP adapter around the shared public application error.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ApiError(PublicError);

impl ApiError {
    /// Creates an API error with an explicit HTTP status, code, and message.
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self(PublicError::new(class, code, message))
    }

    /// Creates an API error for a lower-level failure without exposing backend details.
    pub fn backend(class: ErrorClass, code: impl Into<String>, message: &'static str) -> Self {
        Self(PublicError::backend(class, code, message))
    }

    /// Returns the standard invalid-JSON error.
    pub fn invalid_json() -> Self {
        Self::new(
            ErrorClass::BadRequest,
            "InvalidJson",
            "Failed to parse request body as JSON",
        )
    }

    /// Returns the shared public error payload.
    pub const fn public_error(&self) -> &PublicError {
        &self.0
    }

    /// Derives the HTTP status from the shared error classification.
    pub const fn status(&self) -> StatusCode {
        match self.0.class {
            ErrorClass::BadRequest => StatusCode::BAD_REQUEST,
            ErrorClass::NotFound => StatusCode::NOT_FOUND,
            ErrorClass::Conflict => StatusCode::CONFLICT,
            ErrorClass::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorClass::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl From<PublicError> for ApiError {
    fn from(value: PublicError) -> Self {
        Self(value)
    }
}

impl From<JsonRejection> for ApiError {
    fn from(_error: JsonRejection) -> Self {
        Self::invalid_json()
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status();
        (status, Json(json!({ "status": "error", "error": self.0 }))).into_response()
    }
}

/// Environment variable selecting the REST process role (`live` or `read`).
pub const MFM_REST_ROLE: &str = "MFM_REST_ROLE";

/// Process role for REST bootstrap and route admission.
///
/// Read processes serve evidence-only routes and refuse live mutation routes. Live processes
/// additionally serve start, resume, and manual-resolution routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestProcessRole {
    /// Evidence-only status, stream, list, replay, and public-output routes.
    Read,
    /// Live start/resume and manual-resolution execution.
    Live,
}

impl RestProcessRole {
    /// Parses the role from [`MFM_REST_ROLE`], defaulting to [`RestProcessRole::Live`].
    pub fn from_env() -> Result<Self, ApiError> {
        match std::env::var(MFM_REST_ROLE) {
            Err(std::env::VarError::NotPresent) => Ok(Self::Live),
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "live" => Ok(Self::Live),
                "read" => Ok(Self::Read),
                _ => Err(ApiError::new(
                    ErrorClass::Internal,
                    "InvalidRestRole",
                    format!("{MFM_REST_ROLE} must be \"live\" or \"read\""),
                )),
            },
            Err(std::env::VarError::NotUnicode(_)) => Err(ApiError::new(
                ErrorClass::Internal,
                "InvalidRestRole",
                format!("{MFM_REST_ROLE} must be valid UTF-8"),
            )),
        }
    }
}

/// Shared router state injected into request handlers.
#[derive(Clone)]
pub struct AppState {
    /// Process role selecting the route admission policy.
    role: RestProcessRole,
    /// Opaque application services for this process.
    application: Application,
}

impl AppState {
    /// Binds a process role to one opaque application facade.
    pub const fn new(role: RestProcessRole, application: Application) -> Self {
        Self { role, application }
    }

    fn require_live_role(&self) -> Result<(), ApiError> {
        if self.role != RestProcessRole::Live {
            return Err(ApiError::new(
                ErrorClass::ServiceUnavailable,
                "RestRoleReadOnly",
                "this REST process is read-only; live start/resume requires MFM_REST_ROLE=live",
            ));
        }
        Ok(())
    }
}

/// Builds production REST API state for an explicit process role and runtime config path.
pub async fn make_app_state_for_role(
    role: RestProcessRole,
    runtime_config_path: Option<PathBuf>,
) -> Result<AppState, ApiError> {
    let application =
        mfm_app::connect_production_application(None, runtime_config_path.as_deref()).await?;
    Ok(AppState::new(role, application))
}

/// Builds default production REST API state from the environment-selected role and application.
pub async fn make_default_app_state(
    runtime_config_path: Option<PathBuf>,
) -> Result<AppState, ApiError> {
    make_app_state_for_role(RestProcessRole::from_env()?, runtime_config_path).await
}

/// Builds the `axum` router for the public REST API surface.
pub fn make_app(state: AppState) -> Router {
    let request_id_header = HeaderName::from_static("x-request-id");
    let make_span_header = request_id_header.clone();
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/ready", get(ready))
        .route("/v1/facts/kinds", get(facts_kinds))
        .route("/v1/facts/kinds/:kind", get(facts_describe_kind))
        .route("/v1/facts/ref/:public_ref", get(facts_ref))
        .route("/v1/facts/:kind/latest", get(facts_latest))
        .route("/v1/facts/:kind", get(facts_query))
        .route("/v1/runs", get(runs_list))
        .route("/v1/runs/start", post(runs_start))
        .route("/v1/runs/:run_id/resume", post(runs_resume))
        .route(
            "/v1/runs/:run_id/manual-resolution",
            post(runs_manual_resolution),
        )
        .route("/v1/runs/:run_id/status", get(runs_status))
        .route("/v1/runs/:run_id/stream", get(runs_stream))
        .route("/v1/runs/:run_id/replay", post(runs_replay))
        .route(
            "/v1/runs/:run_id/public-output/:schema_id",
            get(runs_public_output),
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
async fn ready(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    state.application.check_ready().await.map_err(|_| {
        ApiError::new(
            ErrorClass::ServiceUnavailable,
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

async fn not_found() -> ApiError {
    ApiError::new(ErrorClass::NotFound, "not_found", "not found")
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManualResolutionKind {
    ManualResolutionV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunStartBody {
    entry_point: String,
    target: String,
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

impl From<RunObservationPage> for RunObservationPageResponse {
    fn from(page: RunObservationPage) -> Self {
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

impl From<RunObservation> for RunObservationResponse {
    fn from(row: RunObservation) -> Self {
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
async fn facts_kinds(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    json_ok(state.application.fact_kinds().await?)
}

#[instrument(level = "debug", skip(state), fields(kind = kind.as_str()))]
async fn facts_describe_kind(
    State(state): State<AppState>,
    Path(kind): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    json_ok(state.application.describe_fact_kind(&kind).await?)
}

#[instrument(level = "debug", skip(state, query), fields(kind = kind.as_str()))]
async fn facts_query(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    query: RawQuery,
) -> Result<Json<serde_json::Value>, ApiError> {
    facts_query_response(&state, kind, query, None).await
}

#[instrument(level = "debug", skip(state, query), fields(kind = kind.as_str()))]
async fn facts_latest(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    query: RawQuery,
) -> Result<Json<serde_json::Value>, ApiError> {
    facts_query_response(&state, kind, query, Some(1)).await
}

async fn facts_query_response(
    state: &AppState,
    kind: String,
    query: RawQuery,
    forced_limit: Option<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let request = public_fact_query_request(kind, query, forced_limit)?;
    json_ok(state.application.query_public_facts(request).await?)
}

#[instrument(level = "debug", skip(state), fields(public_ref = public_ref.as_str()))]
async fn facts_ref(
    State(state): State<AppState>,
    Path(public_ref): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let public_ref = PublicFactRefId::new(public_ref)?;
    json_ok(
        state
            .application
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

fn fact_query_params_api_error(error: PublicError) -> ApiError {
    if mfm_app::is_public_fact_query_parameter_error(&error) {
        ApiError::new(
            ErrorClass::BadRequest,
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
async fn runs_list(
    State(state): State<AppState>,
    query: Result<Query<RunListQuery>, QueryRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Query(query) = query.map_err(|_| {
        ApiError::new(
            ErrorClass::BadRequest,
            "InvalidQuery",
            "Failed to parse run list query",
        )
    })?;
    let page = state
        .application
        .list_runs(
            query.cursor,
            query.limit.unwrap_or(50),
            query.wait_ms.unwrap_or(0),
        )
        .await?;
    json_ok(RunObservationPageResponse::from(page))
}

#[instrument(level = "info", skip(state, body))]
async fn runs_start(
    State(state): State<AppState>,
    body: Result<Json<RunStartBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(req) = body?;
    state.require_live_role()?;
    let invocation_key = req.invocation_key.map(InvocationKey::new).transpose()?;
    let report = state
        .application
        .start_entry_point_run(&req.entry_point, &req.target, invocation_key)
        .await?;

    json_ok(RunStartResponse {
        outcome: report.outcome,
        run: report.run,
        active_run_id: report.active_run_id,
        public_output: report.public_output,
    })
}

#[instrument(level = "info", skip(state, body), fields(run_id = run_id.as_str()))]
async fn runs_resume(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let _req: RunResumeBody = parse_optional_body(body)?;
    state.require_live_role()?;
    let data = state.application.resume_run(&run_id).await?;

    json_ok(data)
}

#[instrument(level = "info", skip(state, body), fields(run_id = run_id.as_str()))]
async fn runs_manual_resolution(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    body: Result<Json<ManualResolutionBody>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let Json(req) = body?;
    match req.kind {
        ManualResolutionKind::ManualResolutionV1 => {}
    }
    let evidence_bytes =
        canonical_json_value_bytes(&req.evidence_json, "ManualResolutionEvidenceInvalid")?;
    let proof_bytes =
        canonical_json_value_bytes(&req.authorization_proof, "ManualResolutionProofInvalid")?;
    state.require_live_role()?;
    let data = state
        .application
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
async fn runs_status(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let data = state.application.run_status(&run_id).await?;

    json_ok(data)
}

#[instrument(
    level = "debug",
    skip(state, query),
    fields(run_id = run_id.as_str())
)]
async fn runs_stream(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    query: Result<Query<RunStreamQuery>, QueryRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Query(query) = query.map_err(|_| {
        ApiError::new(
            ErrorClass::BadRequest,
            "InvalidQuery",
            "Failed to parse stream query",
        )
    })?;
    validate_sequence_range(query.from_seq, query.to_seq)?;

    let run_id = parse_run_id(&run_id)?;
    let response = state.application.run_stream(&run_id).await?;
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
async fn runs_replay(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let data = state.application.verify_replay(&run_id).await?;

    json_ok(data)
}

#[instrument(
    level = "debug",
    skip(state),
    fields(run_id = run_id.as_str(), schema_id = schema_id.as_str())
)]
async fn runs_public_output(
    State(state): State<AppState>,
    Path((run_id, schema_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let run_id = parse_run_id(&run_id)?;
    let schema_id = parse_schema_id(&schema_id)?;
    let data = state.application.public_output(&run_id, &schema_id).await?;

    json_ok(data)
}

fn parse_run_id(value: &str) -> Result<RunId, ApiError> {
    RunId::parse(value).map_err(|_| {
        ApiError::new(
            ErrorClass::BadRequest,
            "InvalidRunId",
            "Run id must use the typed run identity format `run:<algorithm>:<digest>`",
        )
    })
}

fn parse_schema_id(value: &str) -> Result<SchemaId, ApiError> {
    SchemaId::parse(value).map_err(|_| {
        ApiError::new(
            ErrorClass::BadRequest,
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
            ErrorClass::Internal,
            "SerializationError",
            "Failed to serialize request JSON",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|canonical| canonical.to_vec())
        .map_err(|_| {
            ApiError::backend(
                ErrorClass::BadRequest,
                error_code,
                "Request JSON is not canonical JSON",
            )
        })
}

fn validate_sequence_range(from_seq: u64, to_seq: Option<u64>) -> Result<(), ApiError> {
    if from_seq == 0 {
        return Err(ApiError::new(
            ErrorClass::BadRequest,
            "InvalidSequenceRange",
            "from_seq must be greater than zero",
        ));
    }
    if let Some(to_seq) = to_seq {
        if to_seq < from_seq {
            return Err(ApiError::new(
                ErrorClass::BadRequest,
                "InvalidSequenceRange",
                "to_seq must be greater than or equal to from_seq",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
