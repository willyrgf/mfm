use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use mfm_app::{
    AppError, AppServices, EngineBundle, ErrorClass, FeatureCatalog, FeatureRequest,
    RunsEventsQuery, RunsStartRequest,
};
use mfm_machine::stores::{ArtifactStore, EventStore};
use serde::{Deserialize, Serialize};
use serde_json::json;

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

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid_json() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "InvalidJson",
            "Failed to parse request body as JSON",
        )
    }

    pub fn invalid_uuid() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "InvalidUuid",
            "Invalid UUID format",
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

pub async fn make_default_artifact_store() -> Result<Arc<dyn ArtifactStore>, ApiError> {
    mfm_app::make_default_artifact_store()
        .await
        .map_err(Into::into)
}

pub async fn make_default_event_store() -> Result<Arc<dyn EventStore>, ApiError> {
    mfm_app::make_default_event_store()
        .await
        .map_err(Into::into)
}

pub fn make_engine_bundle() -> EngineBundle {
    mfm_app::make_engine_bundle()
}

#[derive(Clone)]
pub struct AppState {
    pub bundle: EngineBundle,
    pub events: Arc<dyn EventStore>,
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

pub fn make_app(state: AppState) -> Router {
    let state = RouterState {
        app: state,
        catalog: Arc::new(FeatureCatalog::with_builtins()),
    };

    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/features", get(features_list))
        .route("/v1/features/:feature_id/execute", post(features_execute))
        .route("/v1/runs/start", post(runs_start))
        .route("/v1/runs/:run_id/resume", post(runs_resume))
        .route("/v1/runs/:run_id/status", get(runs_status))
        .route("/v1/runs/:run_id/events", get(runs_events))
        .route("/v1/artifacts/:artifact_id", get(artifacts_get))
        .fallback(not_found)
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(ok(json!({ "ok": true })))
}

async fn not_found() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(err("not_found", "not found")))
}

async fn runs_start(
    State(state): State<RouterState>,
    body: Result<Json<RunsStartRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Json(req) = body.map_err(|_| ApiError::invalid_json())?;
    let data = state.services().start_run(req).await?;

    Ok(Json(ok(
        serde_json::to_value(data).expect("run start response must serialize")
    )))
}

async fn runs_resume(
    State(state): State<RouterState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().resume_run(&run_id).await?;

    Ok(Json(ok(
        serde_json::to_value(data).expect("run resume response must serialize")
    )))
}

async fn runs_status(
    State(state): State<RouterState>,
    Path(run_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().run_status(&run_id).await?;

    Ok(Json(ok(
        serde_json::to_value(data).expect("run status response must serialize")
    )))
}

async fn runs_events(
    State(state): State<RouterState>,
    Path(run_id): Path<String>,
    Query(query): Query<RunsEventsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().run_events(&run_id, query).await?;

    Ok(Json(ok(
        serde_json::to_value(data).expect("run events response must serialize")
    )))
}

async fn artifacts_get(
    State(state): State<RouterState>,
    Path(artifact_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state.services().artifact_get(&artifact_id).await?;

    Ok(Json(ok(
        serde_json::to_value(data).expect("artifact response must serialize")
    )))
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

async fn features_list(State(state): State<RouterState>) -> Json<serde_json::Value> {
    Json(ok(serde_json::to_value(FeaturesListResponse {
        features: state.catalog.descriptors().to_vec(),
    })
    .expect("feature list response must serialize")))
}

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

    Ok(Json(ok(
        serde_json::to_value(result).expect("feature execute response must serialize")
    )))
}
