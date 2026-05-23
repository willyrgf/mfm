#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_stream_store_postgres::{PostgresTypedRunEventStore, PostgresTypedStoreError};

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000033";

async fn connect_typed_postgres_with_retry(
    max_attempts: u32,
    delay_ms: u64,
) -> PostgresTypedRunEventStore {
    let mut last_err: Option<PostgresTypedStoreError> = None;
    for _ in 0..max_attempts {
        match PostgresTypedRunEventStore::connect_env().await {
            Ok(store) => return store,
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "<missing>".to_string());
    panic!(
        "typed postgres config after retries (DATABASE_URL={}): {:?}",
        db_url, last_err
    );
}

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
async fn parity_typed_rest_postgres_smoke() {
    let store = connect_typed_postgres_with_retry(20, 250).await;
    let artifact_root =
        std::env::temp_dir().join(format!("mfm-rest-parity-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&artifact_root).expect("typed artifact root");
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        store,
        artifacts: mfm_artifact_store_fs::FsTypedArtifactStore::new(&artifact_root),
    });

    let ready = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/ready")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("ready response");
    assert_eq!(ready.status(), StatusCode::OK);
    let ready_v = response_json(ready).await;
    assert_eq!(ready_v["status"], "success");
    assert_eq!(ready_v["data"]["checks"]["typed_run_store"], "ready");

    let dynamic_artifact = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/artifacts/0000000000000000000000000000000000000000000000000000000000000000")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("artifact response");
    assert_eq!(dynamic_artifact.status(), StatusCode::NOT_FOUND);

    let dynamic_start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "kind": "single_op_start_v1",
                "op_id": "proof",
                "op_version": "v1",
                "op_config": {},
            }),
        ))
        .await
        .expect("dynamic start response");
    assert_eq!(dynamic_start.status(), StatusCode::BAD_REQUEST);

    let absent_status = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{VALID_RUN_ID}/status"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("status response");
    assert_eq!(absent_status.status(), StatusCode::NOT_FOUND);
}
