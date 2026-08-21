use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::rejection::PathRejection;
use axum::extract::{OriginalUri, Path, Request, State};
use axum::http::header::{CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::Response;
use axum::routing::{get, post, put};
use axum::Router;
use mfm_app::{
    Application, BindingList, ConfigDocument, ConfigDocumentError, ConfigPageRequest,
    ConfigSelection, EntryPointList, ImportOutcome, RequestError, RunPageRequest, RunRequestError,
    SerializableRunView,
};
use mfm_catalog::{
    ConfigCursor, ConfigDigest, ConfigName, PageLimit, RunCursor, MAX_CONFIG_DOCUMENT_BYTES,
};
use mfm_ids::RunId;
use serde::{Deserialize, Serialize};

const CONFIG_PATH_ENCODED_MAX: usize = 192;
const RUN_PATH_ENCODED_MAX: usize = 300;
const QUERY_ENCODED_MAX: usize = 1024;
const RUN_BODY_MAX: usize = 4 * 1024;
const CONFIG_DIGEST_HEADER: HeaderName = HeaderName::from_static("mfm-config-digest");

#[derive(Clone)]
struct ServerState {
    application: Arc<Application>,
    controls: Arc<RunControls>,
}

pub(crate) fn router(
    application: Arc<Application>,
    max_in_flight_runs: usize,
    run_timeout: Duration,
) -> Router {
    let state = ServerState {
        application,
        controls: Arc::new(RunControls::new(max_in_flight_runs, run_timeout)),
    };
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/entry-points", get(entry_points))
        .route("/v1/bindings", get(bindings))
        .route("/v1/configs", get(list_configs))
        .route(
            "/v1/configs/{name}",
            put(import_config).get(read_config).delete(delete_config),
        )
        .route("/v1/runs", get(list_runs))
        .route("/v1/runs/{run_id}/start", post(start_run))
        .route("/v1/runs/{run_id}/progress", post(progress_run))
        .route("/v1/runs/{run_id}", get(read_run))
        .fallback(route_not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
}

async fn health(request: Request) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    json_response(StatusCode::OK, &Health { status: "ok" })
}

async fn entry_points(request: Request) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    json_response(
        StatusCode::OK,
        &EntryPointList::new(Application::entry_points()),
    )
}

async fn bindings(State(state): State<ServerState>, request: Request) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    json_response(
        StatusCode::OK,
        &BindingList::new(state.application.bindings()),
    )
}

async fn import_config(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Response {
    if !is_json(request.headers()) {
        return unsupported_media_type();
    }
    let name = match config_name(&uri, path) {
        Ok(name) => name,
        Err(error) => return error.response(),
    };
    let bytes = match bounded_body(request, MAX_CONFIG_DOCUMENT_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };
    let document = match ConfigDocument::new(bytes).await {
        Ok(document) => document,
        Err(error) => return config_document_error(error),
    };
    match state
        .application
        .import_config(name.clone(), document)
        .await
    {
        Ok(outcome) => {
            let status = match outcome {
                ImportOutcome::Created { .. } => StatusCode::CREATED,
                ImportOutcome::Unchanged { .. } => StatusCode::OK,
            };
            let mut response = json_response(status, &outcome);
            let location = format!("/v1/configs/{}", name.as_str());
            match HeaderValue::from_str(&location) {
                Ok(location) => {
                    response.headers_mut().insert(LOCATION, location);
                    response
                }
                Err(_) => internal_error(),
            }
        }
        Err(error) => request_error(error),
    }
}

async fn list_configs(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    let query = match query(&uri) {
        Ok(query) => query,
        Err(error) => return error.response(),
    };
    let cursor = match query.cursor.as_deref().map(ConfigCursor::parse).transpose() {
        Ok(cursor) => cursor,
        Err(_) => return checked_error("invalid_cursor", "cursor is invalid"),
    };
    let page = ConfigPageRequest::new(cursor, query.limit);
    match state.application.list_configs(&page).await {
        Ok(page) => json_response(StatusCode::OK, &page),
        Err(error) => request_error(error),
    }
}

async fn read_config(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    let name = match config_name(&uri, path) {
        Ok(name) => name,
        Err(error) => return error.response(),
    };
    match state.application.read_config(&name).await {
        Ok(config) => json_response(StatusCode::OK, &config),
        Err(error) => request_error(error),
    }
}

async fn delete_config(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Response {
    let name = match config_name(&uri, path) {
        Ok(name) => name,
        Err(error) => return error.response(),
    };
    let digest = match digest_header(request.headers()) {
        Ok(digest) => digest,
        Err(error) => return error.response(),
    };
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    match state.application.delete_config(&name, &digest).await {
        Ok(()) => empty_response(StatusCode::NO_CONTENT),
        Err(error) => request_error(error),
    }
}

async fn list_runs(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    let query = match query(&uri) {
        Ok(query) => query,
        Err(error) => return error.response(),
    };
    let cursor = match query.cursor.as_deref().map(RunCursor::parse).transpose() {
        Ok(cursor) => cursor,
        Err(_) => return checked_error("invalid_cursor", "cursor is invalid"),
    };
    let page = RunPageRequest::new(cursor, query.limit);
    match state.application.list_runs(&page).await {
        Ok(page) => json_response(StatusCode::OK, &page),
        Err(error) => request_error(error),
    }
}

async fn start_run(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Response {
    if !is_json(request.headers()) {
        return unsupported_media_type();
    }
    let run_id = match run_id(&uri, path) {
        Ok(run_id) => run_id,
        Err(error) => return error.response(),
    };
    let bytes = match bounded_body(request, RUN_BODY_MAX).await {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };
    let body: StartBody = match serde_json::from_slice(&bytes) {
        Ok(body) => body,
        Err(_) => return invalid_request_body(),
    };
    let result = state
        .controls
        .execute(
            &run_id,
            true,
            state.application.start_run(run_id.clone(), &body.config),
        )
        .await;
    match result {
        Ok(Ok(result)) => json_response(StatusCode::OK, &result),
        Ok(Err(error)) => run_request_error(error),
        Err(error) => control_error(error),
    }
}

async fn progress_run(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Response {
    if !is_json(request.headers()) {
        return unsupported_media_type();
    }
    let run_id = match run_id(&uri, path) {
        Ok(run_id) => run_id,
        Err(error) => return error.response(),
    };
    let bytes = match bounded_body(request, RUN_BODY_MAX).await {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };
    if serde_json::from_slice::<EmptyBody>(&bytes).is_err() {
        return invalid_request_body();
    }
    let result = state
        .controls
        .execute(&run_id, true, state.application.progress_run(&run_id))
        .await;
    match result {
        Ok(Ok(view)) => json_response(StatusCode::OK, &SerializableRunView::new(&view)),
        Ok(Err(error)) => run_request_error(error),
        Err(error) => control_error(error),
    }
}

async fn read_run(
    State(state): State<ServerState>,
    OriginalUri(uri): OriginalUri,
    path: Result<Path<String>, PathRejection>,
    request: Request,
) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    let run_id = match run_id(&uri, path) {
        Ok(run_id) => run_id,
        Err(error) => return error.response(),
    };
    let result = state
        .controls
        .execute(&run_id, false, state.application.read_run(&run_id))
        .await;
    match result {
        Ok(Ok(view)) => json_response(StatusCode::OK, &SerializableRunView::new(&view)),
        Ok(Err(error)) => request_error(error),
        Err(error) => control_error(error),
    }
}

async fn route_not_found() -> Response {
    boundary_error(
        StatusCode::NOT_FOUND,
        "route_not_found",
        "route is not found",
    )
}

async fn method_not_allowed() -> Response {
    boundary_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "method is not allowed",
    )
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartBody {
    config: ConfigSelection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyBody {}

struct Query {
    cursor: Option<String>,
    limit: PageLimit,
}

fn query(uri: &Uri) -> Result<Query, RestFailure> {
    let encoded = uri.query().unwrap_or_default();
    if encoded.len() > QUERY_ENCODED_MAX {
        return Err(invalid_query_failure());
    }
    let mut cursor = None;
    let mut limit = None;
    if !encoded.is_empty() {
        for field in encoded.split('&') {
            let (key, value) = field.split_once('=').ok_or_else(invalid_query_failure)?;
            let key = decode_query_component(key).ok_or_else(invalid_query_failure)?;
            let value = decode_query_component(value).ok_or_else(invalid_query_failure)?;
            match key.as_str() {
                "cursor" if cursor.is_none() => cursor = Some(value),
                "limit" if limit.is_none() => limit = Some(value),
                _ => return Err(invalid_query_failure()),
            }
        }
    }
    let limit = match limit {
        Some(value) => {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid_query_failure());
            }
            value
                .parse::<usize>()
                .ok()
                .and_then(|value| PageLimit::new(value).ok())
                .ok_or_else(|| checked_failure("invalid_page_limit", "page limit is invalid"))?
        }
        None => PageLimit::default(),
    };
    Ok(Query { cursor, limit })
}

fn decode_query_component(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex(bytes[index + 1])?;
                let low = hex(bytes[index + 2])?;
                decoded.push(high << 4 | low);
                index += 3;
            }
            b'%' => return None,
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

const fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn config_name(
    uri: &Uri,
    path: Result<Path<String>, PathRejection>,
) -> Result<ConfigName, RestFailure> {
    if raw_tail(uri).len() > CONFIG_PATH_ENCODED_MAX {
        return Err(checked_failure(
            "invalid_config_name",
            "config name is invalid",
        ));
    }
    let Path(value) =
        path.map_err(|_| checked_failure("invalid_config_name", "config name is invalid"))?;
    ConfigName::new(value)
        .map_err(|_| checked_failure("invalid_config_name", "config name is invalid"))
}

fn run_id(uri: &Uri, path: Result<Path<String>, PathRejection>) -> Result<RunId, RestFailure> {
    let encoded = uri
        .path()
        .split('/')
        .rev()
        .find(|part| !part.is_empty() && *part != "start" && *part != "progress")
        .unwrap_or_default();
    if encoded.len() > RUN_PATH_ENCODED_MAX {
        return Err(checked_failure("invalid_run_id", "run id is invalid"));
    }
    let Path(value) = path.map_err(|_| checked_failure("invalid_run_id", "run id is invalid"))?;
    RunId::parse(value).map_err(|_| checked_failure("invalid_run_id", "run id is invalid"))
}

fn raw_tail(uri: &Uri) -> &str {
    uri.path().rsplit('/').next().unwrap_or_default()
}

fn digest_header(headers: &HeaderMap) -> Result<ConfigDigest, RestFailure> {
    let mut values = headers.get_all(&CONFIG_DIGEST_HEADER).iter();
    let Some(value) = values.next() else {
        return Err(RestFailure::new(
            StatusCode::PRECONDITION_REQUIRED,
            "precondition_required",
            "config digest header is required",
        ));
    };
    if values.next().is_some() {
        return Err(checked_failure(
            "invalid_config_digest",
            "config digest is invalid",
        ));
    }
    let value = value
        .to_str()
        .map_err(|_| checked_failure("invalid_config_digest", "config digest is invalid"))?;
    ConfigDigest::parse(value)
        .map_err(|_| checked_failure("invalid_config_digest", "config digest is invalid"))
}

fn is_json(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(CONTENT_TYPE).iter();
    let Some(value) = values.next().and_then(|value| value.to_str().ok()) else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    let mut parts = value.split(';');
    if !parts
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
    {
        return false;
    }
    match (parts.next(), parts.next()) {
        (None, None) => true,
        (Some(parameter), None) => parameter.split_once('=').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("charset")
                && value.trim().eq_ignore_ascii_case("utf-8")
        }),
        _ => false,
    }
}

async fn bounded_body(request: Request, limit: usize) -> Result<Vec<u8>, Response> {
    to_bytes(request.into_body(), limit)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|_| {
            boundary_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_body_too_large",
                "request body is too large",
            )
        })
}

async fn require_empty_body(request: Request) -> Result<(), Response> {
    match to_bytes(request.into_body(), 1).await {
        Ok(bytes) if bytes.is_empty() => Ok(()),
        Ok(_) | Err(_) => Err(invalid_request_body()),
    }
}

struct RunControls {
    permits: Arc<tokio::sync::Semaphore>,
    active: Arc<Mutex<HashSet<RunId>>>,
    timeout: Duration,
}

impl RunControls {
    fn new(max_in_flight: usize, timeout: Duration) -> Self {
        Self {
            permits: Arc::new(tokio::sync::Semaphore::new(max_in_flight)),
            active: Arc::new(Mutex::new(HashSet::new())),
            timeout,
        }
    }

    async fn execute<F, T, E>(
        &self,
        run_id: &RunId,
        exclusive: bool,
        future: F,
    ) -> Result<Result<T, E>, ControlError>
    where
        F: Future<Output = Result<T, E>>,
    {
        let _permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| ControlError::Capacity)?;
        let _active = exclusive
            .then(|| ActiveRun::enter(self.active.clone(), run_id.clone()))
            .transpose()?;
        tokio::time::timeout(self.timeout, future)
            .await
            .map_err(|_| ControlError::Deadline)
    }
}

struct ActiveRun {
    runs: Arc<Mutex<HashSet<RunId>>>,
    run_id: RunId,
}

impl ActiveRun {
    fn enter(runs: Arc<Mutex<HashSet<RunId>>>, run_id: RunId) -> Result<Self, ControlError> {
        let inserted = runs
            .lock()
            .map_err(|_| ControlError::Internal)?
            .insert(run_id.clone());
        if !inserted {
            return Err(ControlError::Busy);
        }
        Ok(Self { runs, run_id })
    }
}

impl Drop for ActiveRun {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(&self.run_id);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ControlError {
    Busy,
    Capacity,
    Deadline,
    Internal,
}

fn control_error(error: ControlError) -> Response {
    match error {
        ControlError::Busy => boundary_error(
            StatusCode::CONFLICT,
            "run_busy",
            "run already has an active request",
        ),
        ControlError::Capacity => boundary_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            "run request limit is reached",
        ),
        ControlError::Deadline => boundary_error(
            StatusCode::GATEWAY_TIMEOUT,
            "deadline_exceeded",
            "run request deadline exceeded",
        ),
        ControlError::Internal => internal_error(),
    }
}

fn config_document_error(error: ConfigDocumentError) -> Response {
    let status = match error {
        ConfigDocumentError::Malformed => StatusCode::BAD_REQUEST,
        ConfigDocumentError::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
        ConfigDocumentError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        ConfigDocumentError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    boundary_error(status, error.code(), &error.to_string())
}

fn request_error(error: RequestError) -> Response {
    let status = match error {
        RequestError::ConfigAbsent | RequestError::RunAbsent => StatusCode::NOT_FOUND,
        RequestError::ConfigConflict
        | RequestError::ConfigDigestMismatch
        | RequestError::RunAdmissionConflict
        | RequestError::BindingUnbound => StatusCode::CONFLICT,
        RequestError::InvalidConfigDocument
        | RequestError::CatalogCapacity
        | RequestError::RunCapacity => StatusCode::UNPROCESSABLE_ENTITY,
        RequestError::CatalogIndeterminate | RequestError::DependencyUnavailable => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        RequestError::InvalidCatalog
        | RequestError::InvalidRunHistory
        | RequestError::IncompatibleAssembly
        | RequestError::InvalidRunIndex
        | RequestError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    boundary_error(status, error.code(), &error.to_string())
}

fn run_request_error(error: RunRequestError) -> Response {
    match error {
        RunRequestError::Request(error) => request_error(error),
        RunRequestError::AppendIndeterminate { recovery } => json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            &RecoveryError {
                code: "run_append_indeterminate",
                message: "run append outcome is indeterminate",
                recovery,
            },
        ),
    }
}

fn invalid_request_body() -> Response {
    boundary_error(
        StatusCode::BAD_REQUEST,
        "invalid_request_body",
        "request body is invalid",
    )
}

fn unsupported_media_type() -> Response {
    boundary_error(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported_media_type",
        "content type must be application/json",
    )
}

fn checked_error(code: &'static str, message: &'static str) -> Response {
    boundary_error(StatusCode::BAD_REQUEST, code, message)
}

#[derive(Clone, Copy)]
struct RestFailure {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl RestFailure {
    const fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }

    fn response(self) -> Response {
        boundary_error(self.status, self.code, self.message)
    }
}

const fn invalid_query_failure() -> RestFailure {
    RestFailure::new(
        StatusCode::BAD_REQUEST,
        "invalid_query",
        "request query is invalid",
    )
}

const fn checked_failure(code: &'static str, message: &'static str) -> RestFailure {
    RestFailure::new(StatusCode::BAD_REQUEST, code, message)
}

fn internal_error() -> Response {
    boundary_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        "application internal failure",
    )
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

#[derive(Serialize)]
struct RecoveryError {
    code: &'static str,
    message: &'static str,
    recovery: mfm_app::RunRecovery,
}

fn boundary_error(status: StatusCode, code: &'static str, message: &str) -> Response {
    json_response(status, &ErrorBody { code, message })
}

fn json_response(status: StatusCode, value: &impl Serialize) -> Response {
    let bytes = match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(_) if status != StatusCode::INTERNAL_SERVER_ERROR => return internal_error(),
        Err(_) => b"{\"code\":\"internal\",\"message\":\"application internal failure\"}".to_vec(),
    };
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn empty_response(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use axum::http::{Method, Request};
    use mfm_app::{Application, BoundCapabilitySet, ComposedRuntime, RunRecovery};
    use mfm_catalog::MemoryCatalog;
    use mfm_evm::{EvmEndpoint, EvmReadValue};
    use mfm_evm_live::{EvmProvider, EvmProviderResponse};
    use mfm_ids::StableId;
    use mfm_runtime::ReadAdapterError;
    use mfm_store::MemoryStore;
    use tower::ServiceExt;

    use super::*;

    const DOCUMENT: &[u8] = br#"{
      "input":{"portfolio":{"quotes":["usd"],"portfolio_id":"portfolio-example","collections":[{"request":{"sources":[{"token":null,"source_id":"wallet-0.native","chain_id":1,"address":"0x1111111111111111111111111111111111111111"}],"decimals":18},"correlation":"native-0"}]},"selector":{"quote":"usd","target":"portfolio-example"},"routes":[{"endpoint_id":"alpha","chain_id":1}]},
      "entry_point":"mfm.portfolio/snapshot@1"}"#;

    const ANCHOR: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct Provider;

    impl EvmProvider for Provider {
        fn request<'a>(
            &'a self,
            operation: StableId,
            _request_bytes: Vec<u8>,
        ) -> Pin<Box<dyn Future<Output = Result<EvmProviderResponse, ReadAdapterError>> + Send + 'a>>
        {
            Box::pin(async move {
                let value = match operation.as_str() {
                    "mfm.evm.read-chain-identity@1" => EvmReadValue::ChainId(1),
                    "mfm.evm.read-initial-anchor@1" | "mfm.evm.confirm-balance-anchor@1" => {
                        EvmReadValue::Anchor {
                            number: "100".to_owned(),
                            hash: ANCHOR.to_owned(),
                        }
                    }
                    "mfm.evm.read-native-balance@1" => {
                        EvmReadValue::RawUnits("1000000000000000000".to_owned())
                    }
                    "mfm.evm.read-token-decimals@1" => EvmReadValue::TokenDecimals(6),
                    "mfm.evm.read-token-balance@1" => EvmReadValue::RawUnits("2500000".to_owned()),
                    _ => return Err(ReadAdapterError::Internal),
                };
                Ok(EvmProviderResponse::Read(value))
            })
        }
    }

    fn application() -> Arc<Application> {
        application_with_routes(false)
    }

    fn bound_application() -> Arc<Application> {
        application_with_routes(true)
    }

    fn application_with_routes(bound: bool) -> Arc<Application> {
        let store = Arc::new(MemoryStore::new());
        let routes = if bound {
            vec![(
                1,
                EvmEndpoint::new("alpha").expect("endpoint"),
                Arc::new(Provider) as Arc<dyn EvmProvider>,
            )]
        } else {
            Vec::new()
        };
        let bindings = BoundCapabilitySet::new(routes).expect("bindings");
        let composed = ComposedRuntime::compose(store, bindings).expect("composition");
        Arc::new(
            Application::from_parts(composed, Arc::new(MemoryCatalog::new())).expect("application"),
        )
    }

    fn request(method: Method, uri: &str, body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(body.into())
            .expect("request")
    }

    async fn body(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("JSON response")
    }

    #[tokio::test]
    async fn host_and_origin_are_not_admission_inputs() {
        let service = router(application(), 2, Duration::from_secs(1));
        let response = service
            .clone()
            .oneshot(request(Method::GET, "/healthz", Body::empty()))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        let mut origin = request(Method::GET, "/healthz", Body::empty());
        origin
            .headers_mut()
            .insert("origin", HeaderValue::from_static("https://example.com"));
        let response = service.clone().oneshot(origin).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn fallback_method_and_media_errors_are_normalized() {
        let service = router(application(), 2, Duration::from_secs(1));
        let response = service
            .clone()
            .oneshot(request(Method::GET, "/missing", Body::empty()))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body(response).await["code"], "route_not_found");

        let response = service
            .clone()
            .oneshot(request(Method::POST, "/healthz", Body::empty()))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body(response).await["code"], "method_not_allowed");

        let response = service
            .oneshot(request(
                Method::POST,
                "/v1/runs/not-a-run/start",
                Body::from("{}"),
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(body(response).await["code"], "unsupported_media_type");
    }

    #[tokio::test]
    async fn config_routes_share_exact_models_and_conditional_delete() {
        let service = router(application(), 2, Duration::from_secs(1));
        let mut import = request(Method::PUT, "/v1/configs/daily", Body::from(DOCUMENT));
        import
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.clone().oneshot(import).await.expect("response");
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()[LOCATION], "/v1/configs/daily");
        let imported = body(response).await;
        assert_eq!(imported["outcome"], "created");
        let digest = imported["config"]["digest"]
            .as_str()
            .expect("digest")
            .to_owned();

        let response = service
            .clone()
            .oneshot(request(Method::GET, "/v1/configs/daily", Body::empty()))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let shown = body(response).await;
        assert!(shown["document"].is_object());
        assert_eq!(shown["config"]["digest"], digest);

        let response = service
            .clone()
            .oneshot(request(Method::GET, "/v1/configs?limit=1", Body::empty()))
            .await
            .expect("response");
        assert_eq!(body(response).await["items"][0]["name"], "daily");

        let run_id =
            "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111";
        let mut start = request(
            Method::POST,
            &format!("/v1/runs/{run_id}/start"),
            Body::from(r#"{"config":{"kind":"current","name":"daily"}}"#),
        );
        start
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.clone().oneshot(start).await.expect("response");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(body(response).await["code"], "binding_unbound");

        let response = service
            .clone()
            .oneshot(request(Method::DELETE, "/v1/configs/daily", Body::empty()))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);

        let mut delete = request(Method::DELETE, "/v1/configs/daily", Body::empty());
        delete.headers_mut().insert(
            &CONFIG_DIGEST_HEADER,
            HeaderValue::from_str(&digest).expect("digest header"),
        );
        let response = service.oneshot(delete).await.expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(!response.headers().contains_key(CONTENT_TYPE));
    }

    #[tokio::test]
    async fn checked_paths_queries_bodies_and_shared_errors_are_exact() {
        let service = router(application(), 2, Duration::from_secs(1));
        let response = service
            .clone()
            .oneshot(request(Method::GET, "/v1/configs/UPPER", Body::empty()))
            .await
            .expect("response");
        assert_eq!(body(response).await["code"], "invalid_config_name");

        let response = service
            .clone()
            .oneshot(request(
                Method::GET,
                "/v1/configs?limit=0&limit=1",
                Body::empty(),
            ))
            .await
            .expect("response");
        assert_eq!(body(response).await["code"], "invalid_query");

        let response = service
            .clone()
            .oneshot(request(
                Method::GET,
                "/v1/configs?limit=many",
                Body::empty(),
            ))
            .await
            .expect("response");
        assert_eq!(body(response).await["code"], "invalid_query");

        let response = service
            .clone()
            .oneshot(request(Method::GET, "/v1/configs?limit=201", Body::empty()))
            .await
            .expect("response");
        assert_eq!(body(response).await["code"], "invalid_page_limit");

        let run_id =
            "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111";
        let mut start = request(
            Method::POST,
            &format!("/v1/runs/{run_id}/start"),
            Body::from(r#"{"config":{"kind":"current","name":"daily"},"entry_point":"x"}"#),
        );
        start
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.clone().oneshot(start).await.expect("response");
        assert_eq!(body(response).await["code"], "invalid_request_body");

        let mut progress = request(
            Method::POST,
            &format!("/v1/runs/{run_id}/progress"),
            Body::from(" { } \n"),
        );
        progress.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=UTF-8"),
        );
        let response = service.oneshot(progress).await.expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body(response).await["code"], "run_absent");
    }

    #[tokio::test]
    async fn current_and_exact_start_use_shared_results_and_terminal_raw_values() {
        let service = router(bound_application(), 2, Duration::from_secs(10));
        let mut import = request(Method::PUT, "/v1/configs/daily", Body::from(DOCUMENT));
        import
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.clone().oneshot(import).await.expect("response");
        let imported = body(response).await;
        let digest = imported["config"]["digest"]
            .as_str()
            .expect("digest")
            .to_owned();

        let current_id =
            "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111";
        let mut start = request(
            Method::POST,
            &format!("/v1/runs/{current_id}/start"),
            Body::from(r#"{"config":{"kind":"current","name":"daily"}}"#),
        );
        start
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.clone().oneshot(start).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let started = body(response).await;
        assert_eq!(started["config"]["digest"], digest);
        assert_eq!(started["run"]["run_id"], current_id);

        let mut terminal = started["run"].clone();
        for _ in 0..20 {
            if terminal["state"]["kind"] == "succeeded" {
                break;
            }
            let mut progress = request(
                Method::POST,
                &format!("/v1/runs/{current_id}/progress"),
                Body::from("{}"),
            );
            progress
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            let response = service.clone().oneshot(progress).await.expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            terminal = body(response).await;
        }
        assert_eq!(terminal["state"]["kind"], "succeeded");
        assert!(terminal["state"]["value"].is_object());

        let exact_id =
            "run:sha256-jcs-v1:2222222222222222222222222222222222222222222222222222222222222222";
        let mut exact = request(
            Method::POST,
            &format!("/v1/runs/{exact_id}/start"),
            Body::from(format!(
                r#"{{"config":{{"kind":"exact","name":"daily","digest":"{digest}"}}}}"#
            )),
        );
        exact
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.oneshot(exact).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let exact = body(response).await;
        assert_eq!(exact["config"]["digest"], digest);
        assert_eq!(exact["run"]["run_id"], exact_id);
    }

    #[tokio::test]
    async fn head_body_size_and_bodyless_admission_are_normalized() {
        let service = router(application(), 2, Duration::from_secs(1));
        let response = service
            .clone()
            .oneshot(request(Method::HEAD, "/v1/entry-points", Body::empty()))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert!(to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .is_empty());

        let response = service
            .clone()
            .oneshot(request(Method::GET, "/healthz", Body::from("x")))
            .await
            .expect("response");
        assert_eq!(body(response).await["code"], "invalid_request_body");

        let invalid_run = request(Method::HEAD, "/v1/runs/not-a-run", Body::empty());
        let response = service
            .clone()
            .oneshot(invalid_run)
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body")
            .is_empty());

        let mut oversized = request(
            Method::PUT,
            "/v1/configs/daily",
            Body::from(vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES + 1]),
        );
        oversized
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.clone().oneshot(oversized).await.expect("response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body(response).await["code"], "request_body_too_large");

        let mut invalid = request(
            Method::PUT,
            "/v1/configs/daily",
            Body::from(r#"{"entry_point":"unknown","input":{}}"#),
        );
        invalid
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = service.oneshot(invalid).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body(response).await["code"], "invalid_config_document");
    }

    #[tokio::test]
    async fn both_indeterminate_recovery_sums_use_the_shared_envelope() {
        let application = bound_application();
        let document = ConfigDocument::new(DOCUMENT.to_vec())
            .await
            .expect("document");
        let outcome = application
            .import_config(ConfigName::new("daily").expect("name"), document)
            .await
            .expect("import");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .expect("run id");
        let start = run_request_error(RunRequestError::AppendIndeterminate {
            recovery: RunRecovery::Start {
                run_id: run_id.clone(),
                config: outcome.config().clone(),
            },
        });
        assert_eq!(start.status(), StatusCode::SERVICE_UNAVAILABLE);
        let start = body(start).await;
        assert_eq!(start["code"], "run_append_indeterminate");
        assert_eq!(start["recovery"]["kind"], "start");
        assert_eq!(start["recovery"]["config"]["name"], "daily");
        assert_eq!(
            start,
            serde_json::from_str::<serde_json::Value>(include_str!(
                "../../../docs/contracts/client-surface/run-recovery-start.json"
            ))
            .expect("start recovery fixture")
        );

        let progress = run_request_error(RunRequestError::AppendIndeterminate {
            recovery: RunRecovery::Progress { run_id },
        });
        let progress = body(progress).await;
        assert_eq!(progress["recovery"]["kind"], "progress");
        assert!(progress["recovery"].get("config").is_none());
        assert_eq!(
            progress,
            serde_json::from_str::<serde_json::Value>(include_str!(
                "../../../docs/contracts/client-surface/run-recovery-progress.json"
            ))
            .expect("progress recovery fixture")
        );
    }

    #[tokio::test]
    async fn run_controls_drop_guards_on_deadline_and_report_pressure() {
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .expect("run id");
        let controls = RunControls::new(1, Duration::from_millis(10));
        let deadline = controls
            .execute(&run_id, true, std::future::pending::<Result<(), ()>>())
            .await;
        assert!(matches!(deadline, Err(ControlError::Deadline)));
        assert!(controls.active.lock().expect("active set").is_empty());

        let permit = controls
            .permits
            .clone()
            .try_acquire_owned()
            .expect("permit");
        let capacity = controls
            .execute(&run_id, false, async { Ok::<_, ()>(()) })
            .await;
        assert!(matches!(capacity, Err(ControlError::Capacity)));
        drop(permit);

        let active = ActiveRun::enter(controls.active.clone(), run_id.clone()).expect("active");
        let busy = controls
            .execute(&run_id, true, async { Ok::<_, ()>(()) })
            .await;
        assert!(matches!(busy, Err(ControlError::Busy)));
        drop(active);

        let capacity = control_error(ControlError::Capacity);
        assert_eq!(capacity.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body(capacity).await["code"], "too_many_requests");
        let deadline = control_error(ControlError::Deadline);
        assert_eq!(deadline.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body(deadline).await["code"], "deadline_exceeded");
    }

    #[test]
    fn request_error_status_table_is_exhaustive() {
        let cases = [
            (RequestError::ConfigAbsent, StatusCode::NOT_FOUND),
            (RequestError::ConfigConflict, StatusCode::CONFLICT),
            (RequestError::ConfigDigestMismatch, StatusCode::CONFLICT),
            (
                RequestError::InvalidConfigDocument,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                RequestError::CatalogCapacity,
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                RequestError::CatalogIndeterminate,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (
                RequestError::InvalidCatalog,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (RequestError::RunAbsent, StatusCode::NOT_FOUND),
            (RequestError::RunAdmissionConflict, StatusCode::CONFLICT),
            (
                RequestError::InvalidRunHistory,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                RequestError::IncompatibleAssembly,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (RequestError::RunCapacity, StatusCode::UNPROCESSABLE_ENTITY),
            (RequestError::BindingUnbound, StatusCode::CONFLICT),
            (
                RequestError::InvalidRunIndex,
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                RequestError::DependencyUnavailable,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            (RequestError::Internal, StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (error, status) in cases {
            assert_eq!(request_error(error).status(), status);
        }
    }
}
