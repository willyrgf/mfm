use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_artifact_store_fs::FsArtifactStore;
use mfm_event_store_mem::MemEventStore;
use mfm_machine::stores::{ArtifactStore, EventStore};

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
async fn start_status_resume_happy_path() {
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> =
        Arc::new(FsArtifactStore::new(tmp.path().to_path_buf()));

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events,
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
    let start_phase = start_v["data"]["phase"].as_str().expect("phase");
    assert_eq!(start_phase, "completed");
    let final_snapshot_id = start_v["data"]["final_snapshot_id"]
        .as_str()
        .expect("final_snapshot_id")
        .to_string();

    let status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/status"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("status response");

    assert_eq!(status.status(), StatusCode::OK);
    let status_v = response_json(status).await;
    assert_eq!(status_v["status"], "success");
    assert_eq!(status_v["data"]["run_id"], run_id);
    assert!(status_v["data"]["head_seq"].as_u64().expect("head_seq") > 0);
    assert_eq!(status_v["data"]["phase"], "completed");
    assert_eq!(status_v["data"]["final_snapshot_id"], final_snapshot_id);

    let events = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/runs/{run_id}/events?from_seq=1"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("events response");

    assert_eq!(events.status(), StatusCode::OK);
    let events_v = response_json(events).await;
    assert_eq!(events_v["status"], "success");
    assert_eq!(events_v["data"]["run_id"], run_id);
    assert!(events_v["data"]["head_seq"].as_u64().expect("head_seq") > 0);
    assert!(!events_v["data"]["events"]
        .as_array()
        .expect("events")
        .is_empty());

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

    let resume = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/runs/{run_id}/resume"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("resume response");

    assert_eq!(resume.status(), StatusCode::OK);
    let resume_v = response_json(resume).await;
    assert_eq!(resume_v["status"], "success");
    assert_eq!(resume_v["data"]["run_id"], run_id);
    assert_eq!(resume_v["data"]["phase"], "completed");
    assert_eq!(resume_v["data"]["final_snapshot_id"], final_snapshot_id);
}

#[tokio::test]
async fn start_pipeline_payload_happy_path() {
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> =
        Arc::new(FsArtifactStore::new(tmp.path().to_path_buf()));

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events,
        artifacts,
    });

    let start = app
        .clone()
        .oneshot(json_post(
            "/v1/runs/start",
            serde_json::json!({
                "pipeline": {
                    "machine_id": "proof",
                    "pipeline_version": "v1",
                    "steps": [
                        {
                            "step_id": "main",
                            "op_id": "proof",
                            "op_version": "v1",
                            "op_config": {}
                        }
                    ]
                },
                "input": {}
            }),
        ))
        .await
        .expect("start response");

    assert_eq!(start.status(), StatusCode::OK);
    let start_v = response_json(start).await;
    assert_eq!(start_v["status"], "success");
    assert_eq!(start_v["data"]["phase"], "completed");
}

#[tokio::test]
async fn artifacts_not_found_is_404() {
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> =
        Arc::new(FsArtifactStore::new(tmp.path().to_path_buf()));

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events,
        artifacts,
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/artifacts/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
}

#[tokio::test]
async fn status_invalid_uuid_is_stable_error() {
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> =
        Arc::new(FsArtifactStore::new(tmp.path().to_path_buf()));

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events,
        artifacts,
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/runs/not-a-uuid/status")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "InvalidUuid");
}

#[tokio::test]
async fn start_invalid_json_is_stable_error() {
    let events: Arc<dyn EventStore> = Arc::new(MemEventStore::new());
    let tmp = tempfile::tempdir().expect("tempdir");
    let artifacts: Arc<dyn ArtifactStore> =
        Arc::new(FsArtifactStore::new(tmp.path().to_path_buf()));

    let bundle = mfm_rest_api::make_engine_bundle();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle,
        events,
        artifacts,
    });

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs/start")
                .header("content-type", "application/json")
                .body(Body::from("not json"))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = response_json(resp).await;
    assert_eq!(v["status"], "error");
    assert_eq!(v["error"]["code"], "InvalidJson");
}
