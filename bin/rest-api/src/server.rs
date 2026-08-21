use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::rejection::PathRejection;
use axum::extract::{OriginalUri, Path, Request, State};
use axum::http::header::{CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::Response;
use axum::routing::{get, post, put};
use axum::Router;
use mfm_app::{
    Application, ConfigDocument, ConfigDocumentError, ConfigName, ConfigSelection, ImportOutcome,
    ItemList, RequestError, RunPageLimit, RunRequestError, SerializableRunView,
    MAX_CONFIG_DOCUMENT_BYTES,
};
use mfm_ids::RunId;
use serde::{Deserialize, Serialize};

const CONFIG_PATH_ENCODED_MAX: usize = 192;
const RUN_PATH_ENCODED_MAX: usize = 300;
const QUERY_ENCODED_MAX: usize = 1024;
const RUN_BODY_MAX: usize = 4 * 1024;

pub(crate) fn router(application: Arc<Application>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/entry-points", get(entry_points))
        .route("/v1/bindings", get(bindings))
        .route("/v1/configs", get(list_configs))
        .route("/v1/configs/{name}", put(import_config).get(read_config))
        .route("/v1/runs", get(list_runs))
        .route("/v1/runs/{run_id}/start", post(start_run))
        .route("/v1/runs/{run_id}/progress", post(progress_run))
        .route("/v1/runs/{run_id}", get(read_run))
        .fallback(route_not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(application)
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
    json_response(StatusCode::OK, &ItemList::new(Application::entry_points()))
}

async fn bindings(State(application): State<Arc<Application>>, request: Request) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    json_response(StatusCode::OK, &ItemList::new(application.bindings()))
}

async fn import_config(
    State(application): State<Arc<Application>>,
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
    match application.import_config(name.clone(), document).await {
        Ok(outcome) => {
            let status = match outcome {
                ImportOutcome::Created { .. } => StatusCode::CREATED,
                ImportOutcome::Unchanged { .. } | ImportOutcome::Updated { .. } => StatusCode::OK,
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
    State(application): State<Arc<Application>>,
    OriginalUri(uri): OriginalUri,
    request: Request,
) -> Response {
    if let Err(error) = require_empty_body(request).await {
        return error;
    }
    if uri.query().is_some() {
        return invalid_query_failure().response();
    }
    match application.list_configs().await {
        Ok(items) => json_response(StatusCode::OK, &ItemList::new(&items)),
        Err(error) => request_error(error),
    }
}

async fn read_config(
    State(application): State<Arc<Application>>,
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
    match application.read_config(&name).await {
        Ok(config) => json_response(StatusCode::OK, &config),
        Err(error) => request_error(error),
    }
}

async fn list_runs(
    State(application): State<Arc<Application>>,
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
    let after = match query.after.as_deref().map(RunId::parse).transpose() {
        Ok(after) => after,
        Err(_) => return checked_error("invalid_run_id", "run id is invalid"),
    };
    match application.list_runs(after.as_ref(), query.limit).await {
        Ok(page) => json_response(StatusCode::OK, &page),
        Err(error) => request_error(error),
    }
}

async fn start_run(
    State(application): State<Arc<Application>>,
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
    match application.start_run(run_id, &body.config).await {
        Ok(result) => json_response(StatusCode::OK, &result),
        Err(error) => run_request_error(error),
    }
}

async fn progress_run(
    State(application): State<Arc<Application>>,
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
    match application.progress_run(&run_id).await {
        Ok(view) => json_response(StatusCode::OK, &SerializableRunView::new(&view)),
        Err(error) => run_request_error(error),
    }
}

async fn read_run(
    State(application): State<Arc<Application>>,
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
    match application.read_run(&run_id).await {
        Ok(view) => json_response(StatusCode::OK, &SerializableRunView::new(&view)),
        Err(error) => request_error(error),
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
    after: Option<String>,
    limit: RunPageLimit,
}

fn query(uri: &Uri) -> Result<Query, RestFailure> {
    let encoded = uri.query().unwrap_or_default();
    if encoded.len() > QUERY_ENCODED_MAX {
        return Err(invalid_query_failure());
    }
    let mut after = None;
    let mut limit = None;
    if !encoded.is_empty() {
        for field in encoded.split('&') {
            let (key, value) = field.split_once('=').ok_or_else(invalid_query_failure)?;
            let key = decode_query_component(key).ok_or_else(invalid_query_failure)?;
            let value = decode_query_component(value).ok_or_else(invalid_query_failure)?;
            match key.as_str() {
                "after" if after.is_none() => after = Some(value),
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
                .and_then(|value| RunPageLimit::new(value).ok())
                .ok_or_else(|| checked_failure("invalid_page_limit", "page limit is invalid"))?
        }
        None => RunPageLimit::default(),
    };
    Ok(Query { after, limit })
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
        RequestError::ConfigDigestMismatch
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
    const RUN_ID: &str =
        "run:sha256-jcs-v1:1111111111111111111111111111111111111111111111111111111111111111";
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
                    _ => return Err(ReadAdapterError::Internal),
                };
                Ok(EvmProviderResponse::Read(value))
            })
        }
    }

    fn application() -> Arc<Application> {
        let store = Arc::new(MemoryStore::new());
        let bindings = BoundCapabilitySet::new(vec![(
            1,
            EvmEndpoint::new("alpha").expect("endpoint"),
            Arc::new(Provider) as Arc<dyn EvmProvider>,
        )])
        .expect("bindings");
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

    fn json_request(method: Method, uri: &str, body: impl Into<Body>) -> Request<Body> {
        let mut request = request(method, uri, body);
        request
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        request
    }

    async fn send(service: &Router, request: Request<Body>) -> Response {
        service.clone().oneshot(request).await.expect("response")
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("JSON response")
    }

    async fn assert_error(
        service: &Router,
        request: Request<Body>,
        status: StatusCode,
        code: &str,
    ) {
        let response = send(service, request).await;
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }

    #[tokio::test]
    async fn router_contract_covers_transport_models_and_recovery() {
        let application = application();
        let service = router(Arc::clone(&application));

        let mut health = request(Method::GET, "/healthz", Body::empty());
        health
            .headers_mut()
            .insert("origin", HeaderValue::from_static("https://example.com"));
        let response = send(&service, health).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["status"], "ok");

        let head = send(
            &service,
            request(Method::HEAD, "/v1/entry-points", Body::empty()),
        )
        .await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[CONTENT_TYPE], "application/json");
        assert!(to_bytes(head.into_body(), usize::MAX)
            .await
            .expect("HEAD body")
            .is_empty());

        for (request, status, code) in [
            (
                request(Method::GET, "/missing", Body::empty()),
                StatusCode::NOT_FOUND,
                "route_not_found",
            ),
            (
                request(Method::POST, "/healthz", Body::empty()),
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
            ),
            (
                request(Method::GET, "/healthz", Body::from("x")),
                StatusCode::BAD_REQUEST,
                "invalid_request_body",
            ),
            (
                request(Method::GET, "/v1/configs/UPPER", Body::empty()),
                StatusCode::BAD_REQUEST,
                "invalid_config_name",
            ),
            (
                request(Method::GET, "/v1/configs?limit=201", Body::empty()),
                StatusCode::BAD_REQUEST,
                "invalid_query",
            ),
            (
                request(
                    Method::POST,
                    &format!("/v1/runs/{RUN_ID}/start"),
                    Body::from("{}"),
                ),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
            ),
            (
                json_request(
                    Method::POST,
                    &format!("/v1/runs/{RUN_ID}/progress"),
                    Body::from("{}"),
                ),
                StatusCode::NOT_FOUND,
                "run_absent",
            ),
            (
                json_request(
                    Method::PUT,
                    "/v1/configs/invalid",
                    Body::from(r#"{"entry_point":"unknown","input":{}}"#),
                ),
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_config_document",
            ),
            (
                json_request(
                    Method::PUT,
                    "/v1/configs/large",
                    Body::from(vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES + 1]),
                ),
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_body_too_large",
            ),
        ] {
            assert_error(&service, request, status, code).await;
        }

        let response = send(
            &service,
            json_request(Method::PUT, "/v1/configs/daily", Body::from(DOCUMENT)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()[LOCATION], "/v1/configs/daily");
        let imported = response_json(response).await;
        let digest = imported["config"]["digest"]
            .as_str()
            .expect("digest")
            .to_owned();

        let response = send(
            &service,
            json_request(
                Method::POST,
                &format!("/v1/runs/{RUN_ID}/start"),
                Body::from(r#"{"config":{"kind":"current","name":"daily"}}"#),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        assert_eq!(started["config"]["digest"], digest);
        assert_eq!(started["run"]["run_id"], RUN_ID);
        assert_eq!(started["run"]["state"]["kind"], "succeeded");
        assert!(started["run"]["state"]["value"].is_object());

        let shown = response_json(
            send(
                &service,
                request(Method::GET, &format!("/v1/runs/{RUN_ID}"), Body::empty()),
            )
            .await,
        )
        .await;
        assert_eq!(shown, started["run"]);

        let progressed = response_json(
            send(
                &service,
                json_request(
                    Method::POST,
                    &format!("/v1/runs/{RUN_ID}/progress"),
                    Body::from("{}"),
                ),
            )
            .await,
        )
        .await;
        assert_eq!(progressed, started["run"]);

        let runs = response_json(
            send(
                &service,
                request(Method::GET, "/v1/runs?limit=1", Body::empty()),
            )
            .await,
        )
        .await;
        assert_eq!(runs["items"][0]["run_id"], RUN_ID);

        let config = application
            .read_config(&ConfigName::new("daily").expect("name"))
            .await
            .expect("stored config");
        let run_id = RunId::parse(RUN_ID).expect("run id");
        for (actual, fixture) in [
            (
                run_request_error(RunRequestError::AppendIndeterminate {
                    recovery: RunRecovery::Start {
                        run_id: run_id.clone(),
                        config: config.config().clone(),
                    },
                }),
                include_str!("../../../docs/contracts/client-surface/run-recovery-start.json"),
            ),
            (
                run_request_error(RunRequestError::AppendIndeterminate {
                    recovery: RunRecovery::Progress { run_id },
                }),
                include_str!("../../../docs/contracts/client-surface/run-recovery-progress.json"),
            ),
        ] {
            assert_eq!(actual.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(
                response_json(actual).await,
                serde_json::from_str::<serde_json::Value>(fixture).expect("recovery fixture")
            );
        }

        assert_eq!(
            send(
                &service,
                request(Method::DELETE, "/v1/configs/daily", Body::empty()),
            )
            .await
            .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}
