#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema, response_json,
    schema_scoped_database_url, unique_postgres_schema,
};
const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000033";

#[tokio::test]
async fn parity_rest_postgres_smoke() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_postgres_schema("rest");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_database_url, 20, 250).await;
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        role: mfm_rest_api::RestProcessRole::Live,
        fact_index: mfm_app::production_fact_index_read_provider(store.clone()),
        store,
        runtime_config_path: None,
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
    assert_eq!(ready_v["data"]["checks"]["run_store"], "ready");

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
    drop_postgres_schema(&database_url, &schema).await;
}
