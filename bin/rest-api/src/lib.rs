#![warn(missing_docs)]
//! REST API wiring for MFM application services.
//!
//! This crate adapts [`mfm_app`] request/response helpers onto an `axum` router while keeping
//! domain execution inside shared app and SDK crates.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_rest_api::{make_app, AppState};
//!
//! async fn build_router() -> Result<axum::Router, mfm_rest_api::ApiError> {
//!     let state = AppState {
//!         bundle: mfm_rest_api::make_engine_bundle(),
//!         events: mfm_rest_api::make_default_event_store().await?,
//!         artifacts: mfm_rest_api::make_default_artifact_store().await?,
//!     };
//!     Ok(make_app(state))
//! }
//! ```
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use http::header::HeaderName;
use mfm_app::{
    AppError, AppServices, EngineBundle, ErrorClass, FeatureCatalog, FeatureRequest,
    RunsEventsQuery, RunsStartRequest,
};
use mfm_machine::ids::{ArtifactId, RunId};
use mfm_machine::stores::{ArtifactStore, StreamId, StreamStore};
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

#[derive(Debug, Clone)]
/// Error payload mapped onto HTTP responses.
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
            ErrorClass::BadGateway => StatusCode::BAD_GATEWAY,
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

/// Builds the default artifact store used by the REST API.
pub async fn make_default_artifact_store() -> Result<Arc<dyn ArtifactStore>, ApiError> {
    mfm_app::make_default_artifact_store()
        .await
        .map_err(Into::into)
}

/// Builds the default event store used by the REST API.
pub async fn make_default_event_store() -> Result<Arc<dyn StreamStore>, ApiError> {
    mfm_app::make_default_event_store()
        .await
        .map_err(Into::into)
}

/// Builds the default engine bundle used by the REST API.
pub fn make_engine_bundle() -> EngineBundle {
    mfm_app::make_engine_bundle()
}

#[derive(Clone)]
/// Shared router state injected into request handlers.
pub struct AppState {
    /// Engine bundle used for planning and execution.
    pub bundle: EngineBundle,
    /// Event store used for run queries.
    pub events: Arc<dyn StreamStore>,
    /// Artifact store used for snapshot and output retrieval.
    pub artifacts: Arc<dyn ArtifactStore>,
}

#[derive(Clone)]
struct RouterState {
    app: AppState,
    catalog: Arc<FeatureCatalog>,
}

impl RouterState {
    fn services(&self) -> AppServices {
        AppServices::new(
            self.app.bundle.clone(),
            Arc::clone(&self.app.events),
            Arc::clone(&self.app.artifacts),
        )
    }
}

/// Builds the `axum` router for the public REST API surface.
pub fn make_app(state: AppState) -> Router {
    let request_id_header = HeaderName::from_static("x-request-id");
    let make_span_header = request_id_header.clone();

    let state = RouterState {
        app: state,
        catalog: Arc::new(FeatureCatalog::with_builtins()),
    };

    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/ready", get(ready))
        .route("/v1/features", get(features_list))
        .route("/v1/features/:feature_id/execute", post(features_execute))
        .route("/v1/runs/start", post(runs_start))
        .route("/v1/runs/:run_id/resume", post(runs_resume))
        .route("/v1/runs/:run_id/status", get(runs_status))
        .route("/v1/runs/:run_id/events", get(runs_events))
        .route("/v1/artifacts/:artifact_id", get(artifacts_get))
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
async fn ready(State(state): State<RouterState>) -> Result<Json<serde_json::Value>, ApiError> {
    // Liveness probe for the event store.
    state
        .app
        .events
        .head_seq(&StreamId::run(RunId(uuid::Uuid::nil())))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "NotReady",
                "event store is not ready",
            )
        })?;

    // Usability probe for the artifact store.
    let probe_artifact_id =
        ArtifactId("0000000000000000000000000000000000000000000000000000000000000000".to_string());
    state
        .app
        .artifacts
        .exists(&probe_artifact_id)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "NotReady",
                "artifact store is not ready",
            )
        })?;

    Ok(Json(ok(json!({
      "ok": true,
      "checks": {
        "event_store": "ready",
        "artifact_store": "ready"
      }
    }))))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(err("not_found", "not found")))
}

#[instrument(level = "info", skip(state, body))]
async fn runs_start(
    State(state): State<RouterState>,
    body: Result<Json<RunsStartRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    let data = state.services().start_run(req).await?;

    json_ok(data)
}

#[instrument(level = "info", skip(state), fields(run_id = run_id.as_str()))]
async fn runs_resume(
    State(state): State<RouterState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().resume_run(&run_id).await?;

    json_ok(data)
}

#[instrument(level = "debug", skip(state), fields(run_id = run_id.as_str()))]
async fn runs_status(
    State(state): State<RouterState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().run_status(&run_id).await?;

    json_ok(data)
}

#[instrument(
    level = "debug",
    skip(state, query),
    fields(run_id = run_id.as_str(), from_seq = query.from_seq, to_seq = ?query.to_seq)
)]
async fn runs_events(
    State(state): State<RouterState>,
    Path(run_id): Path<String>,
    Query(query): Query<RunsEventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().run_events(&run_id, query).await?;

    json_ok(data)
}

#[instrument(level = "debug", skip(state), fields(artifact_id = artifact_id.as_str()))]
async fn artifacts_get(
    State(state): State<RouterState>,
    Path(artifact_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().artifact_get(&artifact_id).await?;

    json_ok(data)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum FeatureExecuteBody {
    Wrapped {
        #[serde(default = "default_empty_object")]
        payload: serde_json::Value,
    },
    Raw(serde_json::Value),
}

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
}

impl FeatureExecuteBody {
    fn into_payload(self) -> serde_json::Value {
        match self {
            Self::Wrapped { payload } => payload,
            Self::Raw(payload) => payload,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct FeaturesListResponse {
    features: Vec<mfm_app::FeatureDescriptor>,
}

#[instrument(level = "debug", skip(state))]
async fn features_list(
    State(state): State<RouterState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    json_ok(FeaturesListResponse {
        features: state.catalog.descriptors().to_vec(),
    })
}

#[instrument(level = "info", skip(state, body), fields(feature_id = feature_id.as_str()))]
async fn features_execute(
    State(state): State<RouterState>,
    Path(feature_id): Path<String>,
    body: Result<Json<FeatureExecuteBody>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid_json())?;

    let result = state
        .catalog
        .execute(
            &state.services(),
            FeatureRequest {
                feature_id,
                payload: body.into_payload(),
            },
        )
        .await?;

    json_ok(result)
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
}
