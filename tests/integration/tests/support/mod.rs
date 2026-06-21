use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_app::{TypedPublicOutputResponse, TypedRunMode, TypedRunResponse};
use mfm_certify::certify_program_draft;
use mfm_op_portfolio_tracker::portfolio_program_draft;
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, parse_portfolio_snapshot_authored_config,
    AuthoredConfigFormat,
};
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

#[allow(unused_imports)]
pub use mfm_integration_tests::test_support::*;

/// Authority summary for a typed portfolio snapshot run.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TypedPortfolioSnapshotAuthority {
    /// Certified spec hash for the run.
    pub spec_hash: String,
    /// Certificate artifact digest rendered as text.
    pub certificate_hash: String,
    /// Number of retained artifacts in replay proof projection.
    pub retained_artifacts: usize,
    /// Public-output event id emitted by the typed run.
    pub public_output_event_id: String,
    /// Canonical rendered digest for the typed public output.
    pub public_output_rendered_digest: String,
}

/// Full snapshot result used by typed-portfolio tests.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TypedPortfolioSnapshotResponse {
    /// Run returned by the typed snapshot request.
    pub started: TypedRunResponse,
    /// Run returned by resume.
    pub resumed: TypedRunResponse,
    /// Alias for the terminal run response used by parity-style assertions.
    pub run: TypedRunResponse,
    /// Rendered public output for the portfolio workflow.
    pub public_output: TypedPublicOutputResponse,
    /// Replay-derived authority for output validation.
    pub authority: TypedPortfolioSnapshotAuthority,
}

/// Starts a typed portfolio snapshot run and lets it execute until blocked.
#[allow(dead_code)]
pub async fn run_typed_portfolio_snapshot(
    payload: serde_json::Value,
) -> TypedPortfolioSnapshotResponse {
    let app = rest_test_app();
    let response = portfolio_snapshot_post(&app, &payload, "until_blocked").await;
    let run = parse_typed_run_response(&response["data"]["run"]);
    let public_output = parse_typed_public_output_response(
        response["data"]
            .get("public_output")
            .expect("portfolio snapshot should return public output"),
    );
    assert_eq!(run.run_mode, TypedRunMode::Completed);

    let authority = replay_and_public_output_authority(&app, &run, &payload).await;
    TypedPortfolioSnapshotResponse {
        started: run.clone(),
        resumed: run.clone(),
        run,
        public_output,
        authority,
    }
}

/// Starts a typed portfolio snapshot run with `append_only` and then resumes it to completion.
#[allow(dead_code)]
pub async fn resume_typed_portfolio_snapshot(
    payload: serde_json::Value,
) -> TypedPortfolioSnapshotResponse {
    let app = rest_test_app();
    let response = portfolio_snapshot_post(&app, &payload, "append_only").await;
    let started = parse_typed_run_response(&response["data"]["run"]);
    assert_eq!(started.run_mode, TypedRunMode::Forward);

    let run_id = &started.run_id;
    let resume = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{run_id}/resume")))
        .await
        .expect("resume typed portfolio run");
    assert_eq!(resume.status(), StatusCode::OK);
    let resumed_payload = response_json(resume).await;
    assert_eq!(resumed_payload["status"], "success");
    let resumed = parse_typed_run_response(&resumed_payload["data"]);

    let authority = replay_and_public_output_authority(&app, &resumed, &payload).await;
    let public_output =
        fetch_public_output(&app, &resumed.run_id, &parse_public_schema_id(&payload)).await;

    TypedPortfolioSnapshotResponse {
        started,
        resumed: resumed.clone(),
        run: resumed,
        public_output,
        authority,
    }
}

fn rest_test_app() -> axum::Router {
    let root = std::env::temp_dir().join(format!("mfm-rest-portfolio-tests-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("typed artifact root");
    mfm_rest_api::make_app(in_memory_rest_app_state(root))
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let payload = serde_json::to_string(&body).expect("request body serializes");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .expect("request")
}

fn empty_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

async fn portfolio_snapshot_post(
    app: &axum::Router,
    payload: &serde_json::Value,
    drive: &str,
) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            json!({
                "op": "portfolio_snapshot",
                "config_format": "json",
                "config": payload,
                "drive": drive,
            }),
        ))
        .await
        .expect("portfolio snapshot response");
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "portfolio snapshot error response: {body}"
    );
    assert_eq!(body["status"], "success");
    body
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("response json")
}

fn parse_typed_run_response(value: &serde_json::Value) -> TypedRunResponse {
    serde_json::from_value(value.clone()).expect("typed run response")
}

fn parse_typed_public_output_response(value: &serde_json::Value) -> TypedPublicOutputResponse {
    TypedPublicOutputResponse {
        run_id: value["run_id"]
            .as_str()
            .expect("typed public-output run id")
            .to_owned(),
        public_schema_id: value["public_schema_id"]
            .as_str()
            .expect("typed public-output schema id")
            .to_owned(),
        event_id: value["event_id"]
            .as_str()
            .expect("typed public-output event id")
            .to_owned(),
        rendered_digest: value["rendered_digest"]
            .as_str()
            .expect("typed public-output rendered digest")
            .to_owned(),
        rendered_artifact_id: value["rendered_artifact_id"]
            .as_str()
            .map(|value| value.to_owned()),
        json: value.get("json").cloned(),
    }
}

fn parse_public_schema_id(value: &serde_json::Value) -> String {
    let raw = serde_json::to_string(value).expect("request json");
    let authored = parse_portfolio_snapshot_authored_config(&raw, AuthoredConfigFormat::Json)
        .expect("portfolio snapshot authored config");
    let canonical = canonicalize_portfolio_snapshot_authored_config(authored)
        .expect("portfolio snapshot canonical config");
    let workflow = canonical.into();
    let draft = portfolio_program_draft(workflow).expect("portfolio draft");
    let certified = certify_program_draft(&draft).expect("certified portfolio spec");
    certified
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .as_str()
        .to_owned()
}

async fn replay_and_public_output_authority(
    app: &axum::Router,
    run: &TypedRunResponse,
    payload: &serde_json::Value,
) -> TypedPortfolioSnapshotAuthority {
    let replay = app
        .clone()
        .oneshot(empty_post(&format!("/v1/runs/{}/replay", run.run_id)))
        .await
        .expect("run replay");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_data = response_json(replay).await;
    assert_eq!(replay_data["status"], "success");
    let retained_artifacts = replay_data["data"]["retained_artifacts"]
        .as_u64()
        .expect("retained artifact count") as usize;
    let public_output =
        fetch_public_output(app, &run.run_id, &parse_public_schema_id(payload)).await;
    TypedPortfolioSnapshotAuthority {
        spec_hash: replay_data["data"]["spec_hash"]
            .as_str()
            .expect("replay spec hash")
            .to_owned(),
        certificate_hash: replay_data["data"]["spec_hash"]
            .as_str()
            .expect("spec hash")
            .to_owned(),
        retained_artifacts,
        public_output_event_id: public_output.event_id.clone(),
        public_output_rendered_digest: public_output.rendered_digest.clone(),
    }
}

async fn fetch_public_output(
    app: &axum::Router,
    run_id: &str,
    public_schema_id: &str,
) -> TypedPublicOutputResponse {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/v1/runs/{run_id}/public-output/{public_schema_id}"
                ))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("public output response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "success");
    parse_typed_public_output_response(&body["data"])
}
