use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use mfm_app::{
    Application, ConfigDocument, ConfigDocumentError, ConfigName, ConfigSelection, ImportOutcome,
    ItemList, RequestError, RunPageLimit, RunRequestError, SerializableRunView,
    MAX_CONFIG_DOCUMENT_BYTES,
};
use mfm_ids::RunId;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

const RUN_BODY_MAX: usize = 4 * 1024;

pub(crate) fn router(application: Arc<Application>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/entry-points", get(entry_points))
        .route("/v1/bindings", get(bindings))
        .route("/v1/configs", get(list_configs))
        .route(
            "/v1/configs/{name}",
            put(import_config).layer(DefaultBodyLimit::max(MAX_CONFIG_DOCUMENT_BYTES)),
        )
        .route("/v1/runs", get(list_runs))
        .route(
            "/v1/runs/{run_id}/start",
            post(start_run).layer(DefaultBodyLimit::max(RUN_BODY_MAX)),
        )
        .route(
            "/v1/runs/{run_id}/progress",
            post(progress_run).layer(DefaultBodyLimit::max(RUN_BODY_MAX)),
        )
        .route("/v1/runs/{run_id}", get(read_run))
        .fallback(route_not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(application)
}

async fn health() -> Response {
    json_response(StatusCode::OK, &Health { status: "ok" })
}

async fn entry_points() -> Response {
    json_response(StatusCode::OK, &ItemList::new(Application::entry_points()))
}

async fn bindings(State(application): State<Arc<Application>>) -> Response {
    json_response(StatusCode::OK, &ItemList::new(application.bindings()))
}

async fn import_config(
    State(application): State<Arc<Application>>,
    path: Result<Path<ConfigName>, PathRejection>,
    body: Result<Json<Box<RawValue>>, JsonRejection>,
) -> Result<Response, RestError> {
    let Path(name) = path.map_err(|_| RestError(invalid_config_name()))?;
    let Json(raw) = body.map_err(|error| RestError(config_json_rejection(error)))?;
    let document = ConfigDocument::new(raw.get().as_bytes().to_vec()).await?;
    let outcome = application.import_config(name.clone(), document).await?;
    let status = match outcome {
        ImportOutcome::Created { .. } => StatusCode::CREATED,
        ImportOutcome::Unchanged { .. } | ImportOutcome::Updated { .. } => StatusCode::OK,
    };
    Ok(json_response(status, &outcome))
}

async fn list_configs(
    State(application): State<Arc<Application>>,
    query: Result<Query<EmptyQuery>, QueryRejection>,
) -> Result<Response, RestError> {
    query.map_err(|_| RestError(invalid_query()))?;
    let items = application.list_configs().await?;
    Ok(json_response(StatusCode::OK, &ItemList::new(&items)))
}

async fn list_runs(
    State(application): State<Arc<Application>>,
    query: Result<Query<RunQuery>, QueryRejection>,
) -> Result<Response, RestError> {
    let Query(query) = query.map_err(|_| RestError(invalid_query()))?;
    let after = query
        .after
        .as_deref()
        .map(RunId::parse)
        .transpose()
        .map_err(|_| RestError(checked_error("invalid_run_id", "run id is invalid")))?;
    let limit = query
        .limit
        .map(RunPageLimit::new)
        .transpose()
        .map_err(|_| RestError(checked_error("invalid_page_limit", "page limit is invalid")))?
        .unwrap_or_default();
    let page = application.list_runs(after.as_ref(), limit).await?;
    Ok(json_response(StatusCode::OK, &page))
}

async fn start_run(
    State(application): State<Arc<Application>>,
    path: Result<Path<RunId>, PathRejection>,
    body: Result<Json<StartBody>, JsonRejection>,
) -> Result<Response, RestError> {
    let Path(run_id) = path.map_err(|_| RestError(invalid_run_id()))?;
    let Json(body) = body.map_err(|error| RestError(json_rejection(error)))?;
    let result = application.start_run(run_id, &body.config).await?;
    Ok(json_response(StatusCode::OK, &result))
}

async fn progress_run(
    State(application): State<Arc<Application>>,
    path: Result<Path<RunId>, PathRejection>,
    body: Result<Json<EmptyBody>, JsonRejection>,
) -> Result<Response, RestError> {
    let Path(run_id) = path.map_err(|_| RestError(invalid_run_id()))?;
    let Json(_) = body.map_err(|error| RestError(json_rejection(error)))?;
    let view = application.progress_run(&run_id).await?;
    Ok(json_response(
        StatusCode::OK,
        &SerializableRunView::new(&view),
    ))
}

async fn read_run(
    State(application): State<Arc<Application>>,
    path: Result<Path<RunId>, PathRejection>,
) -> Result<Response, RestError> {
    let Path(run_id) = path.map_err(|_| RestError(invalid_run_id()))?;
    let view = application.read_run(&run_id).await?;
    Ok(json_response(
        StatusCode::OK,
        &SerializableRunView::new(&view),
    ))
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

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyQuery {}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunQuery {
    after: Option<String>,
    limit: Option<usize>,
}

struct RestError(Response);

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        self.0
    }
}

impl From<ConfigDocumentError> for RestError {
    fn from(error: ConfigDocumentError) -> Self {
        Self(config_document_error(error))
    }
}

impl From<RequestError> for RestError {
    fn from(error: RequestError) -> Self {
        Self(request_error(error))
    }
}

impl From<RunRequestError> for RestError {
    fn from(error: RunRequestError) -> Self {
        Self(run_request_error(error))
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
        RequestError::RunAdmissionConflict | RequestError::BindingUnbound => StatusCode::CONFLICT,
        RequestError::InvalidConfigDocument | RequestError::RunCapacity => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        RequestError::ConfigMutationIndeterminate | RequestError::DependencyUnavailable => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        RequestError::InvalidRetainedConfig
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

fn config_json_rejection(error: JsonRejection) -> Response {
    if matches!(&error, JsonRejection::MissingJsonContentType(_)) {
        unsupported_media_type()
    } else if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        request_body_too_large()
    } else {
        config_document_error(ConfigDocumentError::Malformed)
    }
}

fn json_rejection(error: JsonRejection) -> Response {
    if matches!(&error, JsonRejection::MissingJsonContentType(_)) {
        unsupported_media_type()
    } else if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        request_body_too_large()
    } else {
        invalid_request_body()
    }
}

fn invalid_request_body() -> Response {
    boundary_error(
        StatusCode::BAD_REQUEST,
        "invalid_request_body",
        "request body is invalid",
    )
}

fn request_body_too_large() -> Response {
    boundary_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "request_body_too_large",
        "request body is too large",
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

fn invalid_config_name() -> Response {
    checked_error("invalid_config_name", "config name is invalid")
}

fn invalid_run_id() -> Response {
    checked_error("invalid_run_id", "run id is invalid")
}

fn invalid_query() -> Response {
    boundary_error(
        StatusCode::BAD_REQUEST,
        "invalid_query",
        "request query is invalid",
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
    (status, Json(value)).into_response()
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use axum::body::{to_bytes, Body};
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{HeaderValue, Method, Request};
    use mfm_app::{Application, BoundCapabilitySet, ComposedRuntime, RunRecovery};
    use mfm_config::MemoryConfigRepository;
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
        Arc::new(Application::from_parts(
            composed,
            Arc::new(MemoryConfigRepository::new()),
        ))
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
                json_request(Method::PUT, "/v1/configs/UPPER", Body::from(DOCUMENT)),
                StatusCode::BAD_REQUEST,
                "invalid_config_name",
            ),
            (
                request(Method::GET, "/v1/configs?limit=201", Body::empty()),
                StatusCode::BAD_REQUEST,
                "invalid_query",
            ),
            (
                request(Method::GET, "/v1/runs?limit=201", Body::empty()),
                StatusCode::BAD_REQUEST,
                "invalid_page_limit",
            ),
            (
                request(Method::GET, "/v1/runs?after=invalid", Body::empty()),
                StatusCode::BAD_REQUEST,
                "invalid_run_id",
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
                json_request(Method::PUT, "/v1/configs/malformed", Body::from("{")),
                StatusCode::BAD_REQUEST,
                "malformed_config_document",
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
            .import_config(
                ConfigName::new("daily").expect("name"),
                ConfigDocument::new(DOCUMENT.to_vec())
                    .await
                    .expect("config document"),
            )
            .await
            .expect("retained config")
            .config()
            .clone();
        let run_id = RunId::parse(RUN_ID).expect("run id");
        for (actual, fixture) in [
            (
                run_request_error(RunRequestError::AppendIndeterminate {
                    recovery: RunRecovery::Start {
                        run_id: run_id.clone(),
                        config,
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
                request(Method::GET, "/v1/configs/daily", Body::empty()),
            )
            .await
            .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
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
