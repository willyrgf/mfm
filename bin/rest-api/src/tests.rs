use super::*;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use axum::http::{Method, Request};
use mfm_app::{
    application_for_test, application_with_export_for_test, AccessPolicyError, AccessTarget,
    AuthorizedTenant, RunAccessGrant, RunAccessPolicy, SecretCredential, TestApplicationMode,
};
use mfm_ids::TenantScopeId;
use tokio::io::{AsyncRead, ReadBuf};
use tower::ServiceExt as _;

const RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000";

struct RecordingPolicy {
    result: Result<AuthorizedTenant, AccessPolicyError>,
    calls: Mutex<Vec<(RunAccessGrant, AccessTarget)>>,
}

impl RecordingPolicy {
    fn allowing(tenant_hex: char) -> Arc<Self> {
        Arc::new(Self {
            result: Ok(AuthorizedTenant::new(
                TenantScopeId::new(format!(
                    "mfm.tenant_scope.v1:{}",
                    tenant_hex.to_string().repeat(32)
                ))
                .expect("tenant scope"),
            )),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn denying() -> Arc<Self> {
        Arc::new(Self {
            result: Err(AccessPolicyError::GrantDenied),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<(RunAccessGrant, AccessTarget)> {
        self.calls.lock().expect("policy calls").clone()
    }
}

#[async_trait]
impl RunAccessPolicy for RecordingPolicy {
    async fn authorize(
        &self,
        credential: &SecretCredential,
        grant: RunAccessGrant,
        target: &AccessTarget,
    ) -> Result<AuthorizedTenant, AccessPolicyError> {
        assert_eq!(credential.expose_to_policy(), b"opaque");
        self.calls
            .lock()
            .expect("policy calls")
            .push((grant, target.clone()));
        self.result.clone()
    }
}

struct ExportProbe {
    bytes: &'static [u8],
    offset: usize,
    polls: Arc<AtomicUsize>,
    fail: bool,
}

impl AsyncRead for ExportProbe {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Poll::Ready(Err(std::io::Error::other("private export-reader sentinel")));
        }
        let remaining = &self.bytes[self.offset..];
        let count = remaining.len().min(buffer.remaining());
        buffer.put_slice(&remaining[..count]);
        self.offset += count;
        Poll::Ready(Ok(()))
    }
}

#[test]
fn error_status_mapping_includes_authentication_and_grant_denial() {
    assert_eq!(
        ApiError::from(PublicError::authentication_required()).status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ApiError::from(PublicError::grant_denied()).status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ApiError::from(PublicError::run_not_found()).status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        ApiError::from(PublicError::source_run_export_denied()).status(),
        StatusCode::FORBIDDEN
    );
}

#[test]
fn bearer_parser_accepts_only_one_exact_nonempty_bearer_header() {
    let mut headers = HeaderMap::new();
    assert!(bearer_credential(&headers).is_err());

    headers.insert(AUTHORIZATION, HeaderValue::from_static("Basic value"));
    assert!(bearer_credential(&headers).is_err());

    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer opaque"));
    let _credential = bearer_credential(&headers).expect("exact bearer credential");

    headers.append(AUTHORIZATION, HeaderValue::from_static("Bearer second"));
    assert!(bearer_credential(&headers).is_err());
}

#[test]
fn replay_query_requires_one_exact_mode() {
    assert_eq!(
        decode_replay_query(Some("mode=verify")).expect("verify mode"),
        ReplayMode::Verify
    );
    assert_eq!(
        decode_replay_query(Some("mode=reproduce")).expect("reproduce mode"),
        ReplayMode::Reproduce
    );
    assert_eq!(
        decode_replay_query(Some("mode=compare_current")).expect("compare mode"),
        ReplayMode::CompareCurrent
    );
    for query in [
        None,
        Some(""),
        Some("tenant=forbidden"),
        Some("mode=verify&mode=reproduce"),
        Some("mode=unknown"),
    ] {
        assert!(decode_replay_query(query).is_err(), "{query:?}");
    }
}

#[test]
fn json_request_bodies_reject_unknown_and_duplicate_fields() {
    assert!(decode_body::<ExportBody>(br#"{"kind":"audit","token":"forbidden"}"#).is_err());
    assert!(decode_body::<ExportBody>(br#"{"kind":"audit","kind":"semantic"}"#).is_err());
}

#[tokio::test]
async fn replay_stream_request_requires_exact_headers_and_preserves_raw_bytes() {
    let bytes = b"\x1e{\"kind\":\"end\"}\n";
    let mut headers = replay_stream_headers(bytes);
    let reproduce = replay_stream_request(
        ReplayMode::Reproduce,
        &headers,
        Body::from(bytes.as_slice()),
    )
    .expect("reproduce stream");
    let ReplayRequest::Reproduce(input) = reproduce else {
        panic!("reproduce variant");
    };
    assert_eq!(input.content_ref(), &replay_export_ref(bytes));

    headers.remove(CONTENT_TYPE);
    assert_eq!(
        replay_stream_request(ReplayMode::Reproduce, &headers, Body::empty())
            .expect_err("missing content type")
            .public_error()
            .code,
        "ReplayArtifactInvalid"
    );

    let mut headers = replay_stream_headers(bytes);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    assert!(replay_stream_request(ReplayMode::Reproduce, &headers, Body::empty()).is_err());

    let mut headers = replay_stream_headers(bytes);
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(
            "application/vnd.mfm.run-export-stream.v2+json-seq; charset=utf-8",
        ),
    );
    assert!(replay_stream_request(ReplayMode::Reproduce, &headers, Body::empty()).is_err());

    let mut headers = replay_stream_headers(bytes);
    headers.append(
        CONTENT_TYPE,
        HeaderValue::from_static(mfm_app::PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE),
    );
    assert!(replay_stream_request(ReplayMode::Reproduce, &headers, Body::empty()).is_err());

    let mut headers = replay_stream_headers(bytes);
    headers.append(
        MFM_CONTENT_DIGEST,
        HeaderValue::from_static(
            "content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000",
        ),
    );
    assert!(replay_stream_request(ReplayMode::Reproduce, &headers, Body::empty()).is_err());
}

#[test]
fn page_query_rejects_unknown_repeated_and_out_of_range_values() {
    assert!(decode_page_query(Some("tenant=forbidden")).is_err());
    assert!(decode_page_query(Some("cursor=one&cursor=two")).is_err());
    assert!(decode_page_query(Some("limit=10&limit=11")).is_err());
    assert!(decode_page_query(Some("limit=not-a-number")).is_err());
    assert!(decode_page_query(Some("limit=0"))
        .and_then(PageQuery::into_request)
        .is_err());
    assert!(decode_page_query(Some("limit=501"))
        .and_then(PageQuery::into_request)
        .is_err());

    let request = decode_page_query(Some("cursor=opaque&limit=500"))
        .and_then(PageQuery::into_request)
        .expect("valid bounded page query");
    assert_eq!(request.cursor(), Some("opaque"));
    assert_eq!(request.effective_limit(), 500);
}

#[tokio::test]
async fn request_body_limit_returns_a_reviewed_public_error() {
    let error = request_body(
        Body::from(vec![b'x'; MAX_REQUEST_BODY_BYTES + 1]),
        MAX_REQUEST_BODY_BYTES,
    )
    .await
    .expect_err("oversized body");
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error.public_error().code, "RequestBodyTooLarge");
}

#[tokio::test]
async fn standalone_composition_fails_closed_without_a_writer_fence() {
    let error = match make_default_app_state(None).await {
        Ok(_) => panic!("standalone composition must not mint writer authority"),
        Err(error) => error,
    };
    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error.public_error().code,
        "AuthoritativeWriterFenceUnavailable"
    );
}

#[tokio::test]
async fn readiness_failure_has_one_fixed_service_unavailable_contract() {
    assert_public_error(
        readiness_failed().into_response(),
        StatusCode::SERVICE_UNAVAILABLE,
        "NotReady",
        "The service is not ready",
    )
    .await;
}

#[tokio::test]
async fn readiness_is_unauthenticated_and_performs_no_policy_work() {
    let policy = RecordingPolicy::allowing('1');
    let response = test_router(policy.clone(), TestApplicationMode::Sentinel)
        .oneshot(request(Method::GET, "/v1/ready", Body::empty(), None))
        .await
        .expect("readiness response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1_024)
        .await
        .expect("readiness body");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).expect("readiness JSON"),
        json!({
            "status": "success",
            "data": { "ok": true },
        })
    );
    assert!(policy.calls().is_empty());
}

#[tokio::test]
async fn method_not_allowed_uses_the_error_envelope() {
    let response = get_method_not_allowed().await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get(ALLOW),
        Some(&HeaderValue::from_static("GET"))
    );
    let bytes = to_bytes(response.into_body(), 1024)
        .await
        .expect("bounded method error body");
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).expect("method error JSON"),
        json!({
            "status": "error",
            "error": {
                "code": "MethodNotAllowed",
                "message": "The method is not allowed for this route",
            },
        })
    );

    let response = post_method_not_allowed().await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get(ALLOW),
        Some(&HeaderValue::from_static("POST"))
    );
}

#[tokio::test]
async fn explicit_head_rejection_keeps_reviewed_headers_and_strips_the_body() {
    let router = Router::new().route("/v1/health", exact_get(health));
    let response = router
        .oneshot(
            Request::builder()
                .method(Method::HEAD)
                .uri("/v1/health")
                .body(Body::empty())
                .expect("HEAD request"),
        )
        .await
        .expect("HEAD response");

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
    assert_eq!(
        response.headers().get(ALLOW),
        Some(&HeaderValue::from_static("GET"))
    );
    let body = to_bytes(response.into_body(), 1)
        .await
        .expect("empty HEAD body");
    assert!(body.is_empty());
}

#[tokio::test]
async fn real_router_rejects_head_on_every_path_without_policy_or_backend_work() {
    let policy = RecordingPolicy::allowing('1');
    let router = test_router(policy.clone(), TestApplicationMode::Sentinel);
    for (path, allowed) in [
        ("/v1/health", "GET"),
        ("/v1/ready", "GET"),
        ("/v1/entry-points", "GET"),
        ("/v1/runs", "POST"),
        (
            "/v1/runs/run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000",
            "GET",
        ),
        (
            "/v1/runs/run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000/drive",
            "POST",
        ),
        (
            "/v1/runs/run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000/replay",
            "POST",
        ),
        (
            "/v1/runs/run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000/trace",
            "GET",
        ),
        (
            "/v1/runs/run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000/audit",
            "GET",
        ),
        (
            "/v1/runs/run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000/exports",
            "POST",
        ),
    ] {
        let response = router
            .clone()
            .oneshot(request(Method::HEAD, path, Body::from("not read"), None))
            .await
            .expect("HEAD response");
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{path}"
        );
        assert_eq!(
            response.headers().get(ALLOW),
            Some(&HeaderValue::from_static(allowed)),
            "{path}"
        );
        assert_eq!(
            response.headers().get(CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/json")),
            "{path}"
        );
        assert!(
            to_bytes(response.into_body(), 1)
                .await
                .expect("empty HEAD body")
                .is_empty(),
            "{path}"
        );
    }
    assert!(
        policy.calls().is_empty(),
        "HEAD must not invoke authentication policy"
    );
}

#[tokio::test]
async fn bearer_authentication_precedes_replay_body_reads() {
    let policy = RecordingPolicy::allowing('1');
    let router = test_router(policy.clone(), TestApplicationMode::Sentinel);
    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/v1/runs/{RUN_ID}/replay?mode=reproduce"),
            panic_body(),
            None,
        ))
        .await
        .expect("missing bearer response");
    assert_public_error(
        response,
        StatusCode::UNAUTHORIZED,
        "AuthenticationRequired",
        "Authentication is required",
    )
    .await;

    let response = router
        .oneshot(request(
            Method::POST,
            &format!("/v1/runs/{RUN_ID}/replay?mode=reproduce"),
            panic_body(),
            Some("Basic opaque"),
        ))
        .await
        .expect("malformed bearer response");
    assert_public_error(
        response,
        StatusCode::UNAUTHORIZED,
        "AuthenticationRequired",
        "Authentication is required",
    )
    .await;
    assert!(
        policy.calls().is_empty(),
        "malformed credentials must not reach policy"
    );
}

#[tokio::test]
async fn exact_grant_denial_is_forbidden_before_backend_access() {
    let policy = RecordingPolicy::denying();
    let response = test_router(policy.clone(), TestApplicationMode::Sentinel)
        .oneshot(request(
            Method::GET,
            &format!("/v1/runs/{RUN_ID}"),
            Body::empty(),
            Some("Bearer opaque"),
        ))
        .await
        .expect("grant denial response");
    assert_public_error(
        response,
        StatusCode::FORBIDDEN,
        "GrantDenied",
        "The credential does not grant this operation",
    )
    .await;
    assert_eq!(policy.calls().len(), 1);
    assert_eq!(policy.calls()[0].0, RunAccessGrant::ReadPublic);
}

#[tokio::test]
async fn replay_grant_denial_does_not_poll_a_valid_raw_stream() {
    let policy = RecordingPolicy::denying();
    let response = test_router(policy.clone(), TestApplicationMode::Sentinel)
        .oneshot(replay_stream_http_request(
            &format!("/v1/runs/{RUN_ID}/replay?mode=reproduce"),
            panic_body(),
            b"unpolled",
            Some("Bearer opaque"),
        ))
        .await
        .expect("grant denial response");
    assert_public_error(
        response,
        StatusCode::FORBIDDEN,
        "GrantDenied",
        "The credential does not grant this operation",
    )
    .await;
    assert_eq!(policy.calls().len(), 1);
    assert_eq!(policy.calls()[0].0, RunAccessGrant::Replay);
}

#[tokio::test]
async fn missing_and_cross_tenant_runs_have_one_indistinguishable_response() {
    let mut responses = Vec::new();
    for tenant in ['1', '2'] {
        let policy = RecordingPolicy::allowing(tenant);
        let response = test_router(policy, TestApplicationMode::RunNotFound)
            .oneshot(request(
                Method::GET,
                &format!("/v1/runs/{RUN_ID}"),
                Body::empty(),
                Some("Bearer opaque"),
            ))
            .await
            .expect("not-found response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        responses.push(
            to_bytes(response.into_body(), 1_024)
                .await
                .expect("not-found body"),
        );
    }
    assert_eq!(responses[0], responses[1]);
    assert_eq!(
        serde_json::from_slice::<Value>(&responses[0]).expect("not-found JSON"),
        json!({
            "status": "error",
            "error": {
                "code": "RunNotFound",
                "message": "The requested run was not found",
            },
        })
    );
}

#[tokio::test]
async fn replay_reauthorizes_export_only_for_non_verify_artifacts() {
    let verify_policy = RecordingPolicy::allowing('1');
    let response = test_router(
        verify_policy.clone(),
        TestApplicationMode::Replay(replay_response()),
    )
    .oneshot(request(
        Method::POST,
        &format!("/v1/runs/{RUN_ID}/replay?mode=verify"),
        Body::empty(),
        Some("Bearer opaque"),
    ))
    .await
    .expect("verify response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        verify_policy
            .calls()
            .iter()
            .map(|(grant, _)| *grant)
            .collect::<Vec<_>>(),
        vec![RunAccessGrant::Replay]
    );

    let artifact = b"\x1e{\"kind\":\"end\"}\n";
    let non_verify_policy = RecordingPolicy::allowing('1');
    let response = test_router(
        non_verify_policy.clone(),
        TestApplicationMode::Replay(replay_response()),
    )
    .oneshot(replay_stream_http_request(
        &format!("/v1/runs/{RUN_ID}/replay?mode=compare_current"),
        Body::from(artifact.as_slice()),
        artifact,
        Some("Bearer opaque"),
    ))
    .await
    .expect("non-verify response");
    assert_eq!(response.status(), StatusCode::OK);
    let calls = non_verify_policy.calls();
    assert_eq!(
        calls.iter().map(|(grant, _)| *grant).collect::<Vec<_>>(),
        vec![RunAccessGrant::Replay, RunAccessGrant::Export]
    );
    assert_eq!(
        calls[0].1, calls[1].1,
        "the export reauthorization must target the same run"
    );
}

#[tokio::test]
async fn replay_raw_stream_has_no_old_rest_total_body_cap() {
    let bytes = vec![b'x'; 16_777_216 + 1];
    let policy = RecordingPolicy::allowing('1');
    let response = test_router(policy, TestApplicationMode::Replay(replay_response()))
        .oneshot(replay_stream_http_request(
            &format!("/v1/runs/{RUN_ID}/replay?mode=compare_current"),
            Body::from(bytes.clone()),
            &bytes,
            Some("Bearer opaque"),
        ))
        .await
        .expect("large raw replay response");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn export_response_is_lazy_raw_stream_with_exact_headers() {
    let bytes = b"\x1e{\"kind\":\"end\"}\n";
    let content_ref = replay_export_ref(bytes);
    let polls = Arc::new(AtomicUsize::new(0));
    let policy = RecordingPolicy::allowing('1');
    let application = application_with_export_for_test(
        policy.clone(),
        content_ref.clone(),
        Box::pin(ExportProbe {
            bytes,
            offset: 0,
            polls: Arc::clone(&polls),
            fail: false,
        }),
    );
    let response = make_app(AppState::new(application))
        .oneshot(request(
            Method::POST,
            &format!("/v1/runs/{RUN_ID}/exports"),
            Body::from(r#"{"kind":"semantic"}"#),
            Some("Bearer opaque"),
        ))
        .await
        .expect("export response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static(
            mfm_app::PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE
        ))
    );
    assert_eq!(
        response.headers().get(MFM_CONTENT_DIGEST),
        Some(&HeaderValue::from_str(content_ref.content_digest().as_str()).expect("digest header"))
    );
    let streamed = to_bytes(response.into_body(), 1_024)
        .await
        .expect("streamed body");
    assert_eq!(streamed.as_ref(), bytes);
    assert!(polls.load(Ordering::SeqCst) > 0);
    assert_eq!(
        policy
            .calls()
            .iter()
            .map(|(grant, _)| *grant)
            .collect::<Vec<_>>(),
        vec![RunAccessGrant::Export]
    );
}

#[tokio::test]
async fn export_body_reader_errors_hide_private_diagnostics() {
    let content_ref = replay_export_ref(b"unavailable");
    let policy = RecordingPolicy::allowing('1');
    let application = application_with_export_for_test(
        policy,
        content_ref,
        Box::pin(ExportProbe {
            bytes: b"",
            offset: 0,
            polls: Arc::new(AtomicUsize::new(0)),
            fail: true,
        }),
    );
    let response = make_app(AppState::new(application))
        .oneshot(request(
            Method::POST,
            &format!("/v1/runs/{RUN_ID}/exports"),
            Body::from(r#"{"kind":"semantic"}"#),
            Some("Bearer opaque"),
        ))
        .await
        .expect("export response");
    assert_eq!(response.status(), StatusCode::OK);
    let error = to_bytes(response.into_body(), 1_024)
        .await
        .expect_err("reader failure");
    let rendered = error.to_string();
    assert!(!rendered.contains("private export-reader sentinel"));
    assert!(rendered.contains("export stream unavailable"));
}

#[tokio::test]
async fn invalid_replay_artifacts_fail_before_policy_or_backend_replay() {
    let policy = RecordingPolicy::allowing('1');
    let router = test_router(
        policy.clone(),
        TestApplicationMode::Replay(replay_response()),
    );
    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/v1/runs/{RUN_ID}/replay?mode=reproduce"),
            panic_body(),
            Some("Bearer opaque"),
        ))
        .await
        .expect("invalid artifact response");
    assert_public_error(
        response,
        StatusCode::BAD_REQUEST,
        "ReplayArtifactInvalid",
        "The replay artifact is invalid.",
    )
    .await;
    assert!(policy.calls().is_empty());

    let mut invalid_digest = request(
        Method::POST,
        &format!("/v1/runs/{RUN_ID}/replay?mode=reproduce"),
        panic_body(),
        Some("Bearer opaque"),
    );
    invalid_digest.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(mfm_app::PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE),
    );
    invalid_digest.headers_mut().insert(
        MFM_CONTENT_DIGEST,
        HeaderValue::from_static("not-a-content-digest"),
    );
    let response = router
        .oneshot(invalid_digest)
        .await
        .expect("invalid digest response");
    assert_public_error(
        response,
        StatusCode::BAD_REQUEST,
        "ReplayArtifactInvalid",
        "The replay artifact is invalid.",
    )
    .await;
    assert!(policy.calls().is_empty());
}

#[tokio::test]
async fn real_router_rejects_removed_routes_and_wrong_methods_without_app_work() {
    let policy = RecordingPolicy::allowing('1');
    let router = test_router(policy.clone(), TestApplicationMode::Sentinel);

    let removed = router
        .clone()
        .oneshot(request(Method::GET, "/v1/facts/kinds", Body::empty(), None))
        .await
        .expect("removed route response");
    assert_public_error(
        removed,
        StatusCode::NOT_FOUND,
        "NotFound",
        "The route was not found",
    )
    .await;

    for (method, path, allowed) in [
        (Method::GET, format!("/v1/runs/{RUN_ID}/drive"), "POST"),
        (Method::POST, format!("/v1/runs/{RUN_ID}"), "GET"),
    ] {
        let response = router
            .clone()
            .oneshot(request(method, &path, Body::empty(), None))
            .await
            .expect("wrong-method response");
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            response.headers().get(ALLOW),
            Some(&HeaderValue::from_static(allowed))
        );
    }
    assert!(policy.calls().is_empty());
}

fn test_router(policy: Arc<RecordingPolicy>, mode: TestApplicationMode) -> Router {
    make_app(AppState::new(application_for_test(
        policy,
        Vec::new(),
        mode,
    )))
}

fn request(method: Method, uri: &str, body: Body, authorization: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(authorization) = authorization {
        builder = builder.header(AUTHORIZATION, authorization);
    }
    builder.body(body).expect("request")
}

fn replay_stream_http_request(
    uri: &str,
    body: Body,
    digest_bytes: &[u8],
    authorization: Option<&str>,
) -> Request<Body> {
    let mut request = request(Method::POST, uri, body, authorization);
    *request.headers_mut() = replay_stream_headers(digest_bytes);
    if let Some(authorization) = authorization {
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(authorization).expect("authorization header"),
        );
    }
    request
}

fn replay_stream_headers(bytes: &[u8]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(mfm_app::PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE),
    );
    headers.insert(
        MFM_CONTENT_DIGEST,
        HeaderValue::from_str(replay_export_ref(bytes).content_digest().as_str())
            .expect("content digest header"),
    );
    headers
}

fn panic_body() -> Body {
    Body::from_stream(futures_util::stream::once(async {
        panic!("the replay body must remain unpolled");
        #[allow(unreachable_code)]
        Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::new())
    }))
}

async fn assert_public_error(response: Response, status: StatusCode, code: &str, message: &str) {
    assert_eq!(response.status(), status);
    let body = to_bytes(response.into_body(), 1_024)
        .await
        .expect("public error body");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).expect("public error JSON"),
        json!({
            "status": "error",
            "error": {
                "code": code,
                "message": message,
            },
        })
    );
}

fn replay_response() -> mfm_app::ReplayResponse {
    mfm_app::ReplayResponse::strict_decode(
        format!("{{\"kind\":\"reproduced\",\"result\":\"unavailable\",\"run_id\":\"{RUN_ID}\"}}")
            .as_bytes(),
    )
    .expect("replay response")
}

fn replay_export_ref(bytes: &[u8]) -> ContentRef {
    let contract = mfm_canonical::RecoverabilityContract::embedded().expect("recoverability annex");
    ContentRef::new(
        contract
            .schema_id("mfm.portable-run-export-stream.v2")
            .expect("portable export stream schema")
            .clone(),
        contract.raw_content_digest(bytes),
    )
    .expect("portable export content ref")
}
