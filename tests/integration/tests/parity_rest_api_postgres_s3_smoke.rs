#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_machine::stores::{ArtifactStore, StreamStore};
use mfm_stream_store_postgres::PostgresStreamStore;

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let s = serde_json::to_string(&body).expect("json request must serialize");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(s))
        .expect("request")
}

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
}

#[tokio::test]
async fn parity_postgres_s3_smoke() {
    let pg = PostgresStreamStore::connect_env()
        .await
        .expect("postgres config");
    let streams: Arc<dyn StreamStore> = Arc::new(pg);

    let s3 = S3ArtifactStore::from_env().expect("s3 config");
    s3.ensure_bucket_exists().await.expect("bucket exists");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(s3);

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        streams,
        artifacts,
    });

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({"op_id":"proof","op_version":"v1","op_config":{}}),
        ))
        .await
        .expect("start response");
    assert_eq!(start.status(), StatusCode::OK);

    let start_v = response_json(start).await;
    assert_eq!(start_v["status"], "success");

    let run_id = start_v["data"]["run_id"]
        .as_str()
        .expect("run_id")
        .to_string();
    let final_snapshot_id = start_v["data"]["final_snapshot_id"]
        .as_str()
        .expect("final_snapshot_id")
        .to_string();

    let artifact = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/artifacts/{final_snapshot_id}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("artifact response");
    assert_eq!(artifact.status(), StatusCode::OK);

    let artifact_v = response_json(artifact).await;
    assert_eq!(artifact_v["status"], "success");
    assert_eq!(artifact_v["data"]["artifact_id"], final_snapshot_id);
    assert_eq!(artifact_v["data"]["encoding"], "json");
    assert!(artifact_v["data"]["value"].is_object());

    let stream_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/stream?from_seq=1"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("stream response");
    assert_eq!(stream_resp.status(), StatusCode::OK);
}
