#![warn(missing_docs)]
//! HTTP transport for the recoverability-v1 application facade.
//!
//! Every protected route accepts only one `Authorization: Bearer` credential and delegates a
//! purpose-specific call to [`mfm_app::Application`]. Entry-point discovery and health/readiness
//! are unauthenticated. Portable export returns raw canonical bytes rather than a JSON envelope.

use std::path::PathBuf;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::rejection::PathRejection;
use axum::extract::{MatchedPath, Path, RawQuery, State};
use axum::handler::Handler;
use axum::http::header::{ALLOW, AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, MethodRouter};
use axum::{Json, Router};
use futures_util::TryStreamExt;
use mfm_app::{
    AdmissionStatus, Application, ErrorClass, ExportKind, ExportRequest, PublicError,
    PublicJsonResponse, ReplayMode, ReplayRequest, SecretCredential,
};
use mfm_ids::{ContentDigest, ContentRef, RunId};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::io::{ReaderStream, StreamReader};
use tower_http::trace::TraceLayer;
use tracing::instrument;

const MAX_BEARER_BYTES: usize = mfm_app::MAX_SECRET_CREDENTIAL_BYTES;
const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024;
const MFM_CONTENT_DIGEST: HeaderName = HeaderName::from_static("mfm-content-digest");

/// Thin HTTP adapter around the shared redaction-safe application error.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ApiError(PublicError);

impl ApiError {
    /// Creates an API error with an explicit public classification.
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self(PublicError::new(class, code, message))
    }

    /// Returns the shared public error payload.
    pub const fn public_error(&self) -> &PublicError {
        &self.0
    }

    /// Derives the exact HTTP status from the shared classification.
    pub const fn status(&self) -> StatusCode {
        match self.0.class {
            ErrorClass::BadRequest => StatusCode::BAD_REQUEST,
            ErrorClass::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorClass::Forbidden => StatusCode::FORBIDDEN,
            ErrorClass::NotFound => StatusCode::NOT_FOUND,
            ErrorClass::Conflict => StatusCode::CONFLICT,
            ErrorClass::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorClass::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl From<PublicError> for ApiError {
    fn from(error: PublicError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(json!({ "status": "error", "error": self.0 })),
        )
            .into_response()
    }
}

/// Shared router state containing only the opaque application facade.
#[derive(Clone)]
pub struct AppState {
    application: Application,
}

impl AppState {
    /// Creates transport state around an already deployment-composed application.
    pub const fn new(application: Application) -> Self {
        Self { application }
    }
}

/// Standalone REST bootstrap.
///
/// The repository binary has no deployment-owned authoritative-writer fence, so authority-bearing
/// construction fails closed. Deployments embed this library and inject a fully composed
/// [`AppState`] into [`make_app`].
pub async fn make_default_app_state(
    _runtime_config_path: Option<PathBuf>,
) -> Result<AppState, ApiError> {
    Err(ApiError::new(
        ErrorClass::ServiceUnavailable,
        "AuthoritativeWriterFenceUnavailable",
        "A deployment-owned authoritative-writer fence is required",
    ))
}

/// Builds the exact public REST router.
pub fn make_app(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", exact_get(health))
        .route("/v1/ready", exact_get(ready))
        .route("/v1/entry-points", exact_get(entry_points))
        .route("/v1/runs", exact_post(admit_run))
        .route("/v1/runs/:run_id", exact_get(read_public_run))
        .route("/v1/runs/:run_id/drive", exact_post(drive_once))
        .route("/v1/runs/:run_id/replay", exact_post(replay_run))
        .route("/v1/runs/:run_id/trace", exact_get(read_transition_trace))
        .route("/v1/runs/:run_id/audit", exact_get(read_access_audit))
        .route("/v1/runs/:run_id/exports", exact_post(export_run))
        .fallback(not_found)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<Body>| {
                    let route = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str)
                        .unwrap_or("unmatched");
                    tracing::info_span!(
                        "http.request",
                        method = %request.method(),
                        route,
                    )
                })
                .on_response(
                    |response: &axum::http::Response<Body>,
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

fn exact_get<H, T, S>(handler: H) -> MethodRouter<S>
where
    H: Handler<T, S>,
    T: 'static,
    S: Clone + Send + Sync + 'static,
{
    get(handler)
        .head(get_method_not_allowed)
        .fallback(get_method_not_allowed)
}

fn exact_post<H, T, S>(handler: H) -> MethodRouter<S>
where
    H: Handler<T, S>,
    T: 'static,
    S: Clone + Send + Sync + 'static,
{
    post(handler)
        .head(post_method_not_allowed)
        .fallback(post_method_not_allowed)
}

async fn health() -> Json<Value> {
    Json(success(json!({ "ok": true })))
}

#[instrument(level = "debug", skip(state))]
async fn ready(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    state
        .application
        .check_ready()
        .await
        .map_err(|_| readiness_failed())?;
    Ok(Json(success(json!({ "ok": true }))))
}

fn readiness_failed() -> ApiError {
    ApiError::new(
        ErrorClass::ServiceUnavailable,
        "NotReady",
        "The service is not ready",
    )
}

async fn entry_points(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    public_json_ok(state.application.entry_points())
}

async fn admit_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let credential = bearer_credential(&headers)?;
    let body = request_body(body, MAX_REQUEST_BODY_BYTES).await?;
    let request = mfm_app::AdmitRunRequest::decode_json(&body)?;
    let response = state.application.admit_run(credential, request).await?;
    let status = match response.admission()? {
        AdmissionStatus::NewlyAdmitted => StatusCode::CREATED,
        AdmissionStatus::Attached => StatusCode::OK,
        AdmissionStatus::OutcomeUnknown => StatusCode::ACCEPTED,
    };
    public_json_response(status, &response)
}

async fn drive_once(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let credential = bearer_credential(&headers)?;
    let run_id = run_id_path(path)?;
    let body = request_body(body, MAX_REQUEST_BODY_BYTES).await?;
    require_empty_body(&body)?;
    let response = state.application.drive_once(credential, run_id).await?;
    public_json_response(StatusCode::OK, &response)
}

async fn read_public_run(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let credential = bearer_credential(&headers)?;
    let run_id = run_id_path(path)?;
    let body = request_body(body, MAX_REQUEST_BODY_BYTES).await?;
    require_empty_body(&body)?;
    let response = state
        .application
        .read_public_run(credential, run_id)
        .await?;
    public_json_response(StatusCode::OK, &response)
}

async fn replay_run(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let credential = bearer_credential(&headers)?;
    let run_id = run_id_path(path)?;
    let mode = decode_replay_query(query.as_deref())?;
    let request = match mode {
        ReplayMode::Verify => {
            let body = request_body(body, MAX_REQUEST_BODY_BYTES).await?;
            require_empty_body(&body)?;
            ReplayRequest::Verify
        }
        ReplayMode::Reproduce | ReplayMode::CompareCurrent => {
            replay_stream_request(mode, &headers, body)?
        }
    };
    let response = state
        .application
        .replay_run(credential, run_id, request)
        .await?;
    public_json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    cursor: Option<String>,
    limit: Option<u16>,
}

impl PageQuery {
    fn into_request(self) -> Result<mfm_app::PageRequest, ApiError> {
        mfm_app::PageRequest::new(self.cursor, self.limit)
            .map_err(|_| {
                PublicError::bad_request("PageLimitInvalid", "Page limit must be between 1 and 500")
            })
            .map_err(Into::into)
    }
}

async fn read_transition_trace(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let credential = bearer_credential(&headers)?;
    let run_id = run_id_path(path)?;
    let body = request_body(body, MAX_REQUEST_BODY_BYTES).await?;
    require_empty_body(&body)?;
    let page = decode_page_query(query.as_deref())?.into_request()?;
    let response = state
        .application
        .read_transition_trace(credential, run_id, page)
        .await?;
    public_json_response(StatusCode::OK, &response)
}

async fn read_access_audit(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let credential = bearer_credential(&headers)?;
    let run_id = run_id_path(path)?;
    let body = request_body(body, MAX_REQUEST_BODY_BYTES).await?;
    require_empty_body(&body)?;
    let page = decode_page_query(query.as_deref())?.into_request()?;
    let response = state
        .application
        .read_access_audit(credential, run_id, page)
        .await?;
    public_json_response(StatusCode::OK, &response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportBody {
    kind: String,
}

async fn export_run(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let credential = bearer_credential(&headers)?;
    let run_id = run_id_path(path)?;
    let body = request_body(body, MAX_REQUEST_BODY_BYTES).await?;
    let body: ExportBody = decode_body(&body)?;
    let kind = match body.kind.as_str() {
        "semantic" => ExportKind::Semantic,
        "audit" => ExportKind::Audit,
        _ => {
            return Err(PublicError::bad_request(
                "ExportKindInvalid",
                "Export kind must be semantic or audit",
            )
            .into())
        }
    };
    let export = state
        .application
        .export_run(credential, run_id, ExportRequest::new(kind))
        .await?;
    let content_type = HeaderValue::from_str(export.media_type()).map_err(|_| {
        PublicError::internal(
            "ExportMediaTypeInvalid",
            "Export media type could not be rendered",
        )
    })?;
    let digest = HeaderValue::from_str(export.digest().as_str()).map_err(|_| {
        PublicError::internal("ExportDigestInvalid", "Export digest could not be rendered")
    })?;
    let body = ReaderStream::new(export.into_reader())
        .map_err(|_| std::io::Error::other("export stream unavailable"));
    let mut response = Response::new(Body::from_stream(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(CONTENT_TYPE, content_type);
    response.headers_mut().insert(MFM_CONTENT_DIGEST, digest);
    Ok(response)
}

async fn not_found() -> ApiError {
    ApiError::new(ErrorClass::NotFound, "NotFound", "The route was not found")
}

async fn get_method_not_allowed() -> Response {
    method_not_allowed("GET")
}

async fn post_method_not_allowed() -> Response {
    method_not_allowed("POST")
}

fn method_not_allowed(allowed: &'static str) -> Response {
    let mut response = (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({
            "status": "error",
            "error": {
                "code": "MethodNotAllowed",
                "message": "The method is not allowed for this route",
            },
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(ALLOW, HeaderValue::from_static(allowed));
    response
}

fn bearer_credential(headers: &HeaderMap) -> Result<SecretCredential, ApiError> {
    let mut values = headers.get_all(AUTHORIZATION).iter();
    let value = values
        .next()
        .filter(|_| values.next().is_none())
        .ok_or_else(PublicError::authentication_required)?;
    let bytes = value.as_bytes();
    let token = bytes
        .strip_prefix(b"Bearer ")
        .filter(|token| !token.is_empty() && token.len() <= MAX_BEARER_BYTES)
        .ok_or_else(PublicError::authentication_required)?;
    SecretCredential::new(token.to_vec())
        .map_err(|_| PublicError::authentication_required())
        .map_err(Into::into)
}

async fn request_body(body: Body, max_bytes: usize) -> Result<axum::body::Bytes, ApiError> {
    request_body_with_limit(body, max_bytes, request_body_too_large).await
}

async fn request_body_with_limit(
    body: Body,
    max_bytes: usize,
    limit_error: fn() -> PublicError,
) -> Result<axum::body::Bytes, ApiError> {
    to_bytes(body, max_bytes)
        .await
        .map_err(|_| limit_error())
        .map_err(Into::into)
}

fn request_body_too_large() -> PublicError {
    PublicError::bad_request(
        "RequestBodyTooLarge",
        "Request body exceeds the permitted size",
    )
}

fn parse_run_id(value: &str) -> Result<RunId, ApiError> {
    RunId::parse(value)
        .map_err(|_| invalid_run_id())
        .map_err(Into::into)
}

fn run_id_path(path: Result<Path<String>, PathRejection>) -> Result<RunId, ApiError> {
    let Path(value) = path.map_err(|_| invalid_run_id())?;
    parse_run_id(&value)
}

fn invalid_run_id() -> PublicError {
    PublicError::bad_request("InvalidRunId", "Run id is invalid")
}

fn require_empty_body(body: &[u8]) -> Result<(), ApiError> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(PublicError::bad_request(
            "UnexpectedRequestBody",
            "This operation accepts no request body",
        )
        .into())
    }
}

fn decode_body<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ApiError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| PublicError::bad_request("InvalidJson", "Request JSON is invalid"))?;
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(text)
        .map_err(|_| PublicError::bad_request("InvalidJson", "Request JSON is invalid"))?;
    serde_json::from_slice(canonical.as_bytes())
        .map_err(|_| PublicError::bad_request("InvalidJson", "Request JSON is invalid"))
        .map_err(Into::into)
}

fn decode_replay_query(query: Option<&str>) -> Result<ReplayMode, ApiError> {
    let mut mode = None;
    for (name, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            "mode" if mode.is_none() => mode = Some(ReplayMode::parse(&value)?),
            "mode" => {
                return Err(PublicError::bad_request(
                    "InvalidQuery",
                    "Query parameters must not be repeated",
                )
                .into())
            }
            _ => {
                return Err(PublicError::bad_request(
                    "InvalidQuery",
                    "The query contains an unknown parameter",
                )
                .into())
            }
        }
    }
    mode.ok_or_else(|| {
        PublicError::bad_request(
            "ReplayModeInvalid",
            "Replay mode query parameter is required",
        )
        .into()
    })
}

fn replay_stream_request(
    mode: ReplayMode,
    headers: &HeaderMap,
    body: Body,
) -> Result<ReplayRequest, ApiError> {
    if exactly_one_header(headers, CONTENT_TYPE)
        .filter(|value| value.as_bytes() == mfm_replay_media_type().as_bytes())
        .is_none()
    {
        return Err(PublicError::replay_artifact_invalid().into());
    }
    let digest = exactly_one_header(headers, MFM_CONTENT_DIGEST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| ContentDigest::parse(value).ok())
        .ok_or_else(PublicError::replay_artifact_invalid)?;
    let schema_id = mfm_canonical::RecoverabilityContract::embedded()
        .and_then(|contract| contract.schema_id("mfm.portable-run-export-stream.v1"))
        .map_err(|_| {
            PublicError::internal(
                "RecoverabilityContractUnavailable",
                "The recoverability contract is unavailable",
            )
        })?
        .clone();
    let content_ref =
        ContentRef::new(schema_id, digest).map_err(|_| PublicError::replay_artifact_invalid())?;
    let stream = body.into_data_stream().map_err(std::io::Error::other);
    let input =
        mfm_app::ExportStreamInput::from_reader(content_ref, Box::pin(StreamReader::new(stream)))?;
    match mode {
        ReplayMode::Reproduce => Ok(ReplayRequest::Reproduce(input)),
        ReplayMode::CompareCurrent => Ok(ReplayRequest::CompareCurrent(input)),
        ReplayMode::Verify => Err(PublicError::replay_artifact_invalid().into()),
    }
}

fn exactly_one_header(headers: &HeaderMap, name: HeaderName) -> Option<&HeaderValue> {
    let mut values = headers.get_all(name).iter();
    values.next().filter(|_| values.next().is_none())
}

fn mfm_replay_media_type() -> &'static str {
    mfm_app::PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE
}

fn decode_page_query(query: Option<&str>) -> Result<PageQuery, ApiError> {
    let mut cursor = None;
    let mut limit = None;
    for (name, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            "cursor" if cursor.is_none() => cursor = Some(value.into_owned()),
            "limit" if limit.is_none() => {
                limit = Some(value.parse::<u16>().map_err(|_| {
                    PublicError::bad_request(
                        "PageLimitInvalid",
                        "Page limit must be between 1 and 500",
                    )
                })?);
            }
            "cursor" | "limit" => {
                return Err(PublicError::bad_request(
                    "InvalidQuery",
                    "Query parameters must not be repeated",
                )
                .into());
            }
            _ => {
                return Err(PublicError::bad_request(
                    "InvalidQuery",
                    "The query contains an unknown parameter",
                )
                .into());
            }
        }
    }
    Ok(PageQuery { cursor, limit })
}

fn success(data: Value) -> Value {
    json!({ "status": "success", "data": data })
}

fn public_json_ok<T: PublicJsonResponse + ?Sized>(value: &T) -> Result<Json<Value>, ApiError> {
    value
        .public_json()
        .map(success)
        .map(Json)
        .map_err(Into::into)
}

fn public_json_response<T: PublicJsonResponse>(
    status: StatusCode,
    value: &T,
) -> Result<Response, ApiError> {
    let body = public_json_ok(value)?;
    Ok((status, body).into_response())
}

#[cfg(test)]
mod tests;
