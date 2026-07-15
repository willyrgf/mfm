#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema, json_post,
    response_json, schema_scoped_database_url, unique_postgres_schema,
};
use serde_json::{json, Value};
use tower::ServiceExt;

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000033";
const PORTFOLIO_ENTRY_POINT: &str = "mfm.portfolio/portfolio_snapshot@1";

async fn assert_start_error(
    app: &axum::Router,
    body: Value,
    expected_code: &str,
    forbidden: &[&str],
) {
    let response = app
        .clone()
        .oneshot(json_post("/v1/runs/start", body))
        .await
        .expect("start response");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = response_json(response).await;
    assert_eq!(response["status"], "error");
    assert_eq!(response["error"]["code"], expected_code);
    let serialized = response.to_string();
    for value in forbidden {
        assert!(!serialized.contains(value), "REST error leaked {value}");
    }
}

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
        catalog_store: Some(store.clone()),
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
        .clone()
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

    for old_field in ["op", "op_version", "config_format", "config"] {
        assert_start_error(
            &app,
            json!({
                "entry_point": PORTFOLIO_ENTRY_POINT,
                "request": {old_field: "legacy-value"},
            }),
            "EntryPointRequestInvalid",
            &["legacy-value"],
        )
        .await;
    }
    assert_start_error(
        &app,
        json!({
            "entry_point": PORTFOLIO_ENTRY_POINT,
            "request": "portfolio = 'legacy TOML'",
        }),
        "EntryPointRequestInvalid",
        &["legacy TOML"],
    )
    .await;
    assert_start_error(
        &app,
        json!({
            "entry_point": PORTFOLIO_ENTRY_POINT,
            "request": {"portfolio": {"name": "acme/dual-mainnet"}},
        }),
        "EntryPointRequestInvalid",
        &["acme/dual-mainnet"],
    )
    .await;
    assert_start_error(
        &app,
        json!({
            "entry_point": PORTFOLIO_ENTRY_POINT,
            "request": {
                "portfolio": {"name": "acme/dual-mainnet", "digest": "not-a-digest"},
                "unknown": true,
            },
        }),
        "EntryPointRequestInvalid",
        &["acme/dual-mainnet", "not-a-digest"],
    )
    .await;
    assert_start_error(
        &app,
        json!({
            "entry_point": "mfm.portfolio/portfolio_snapshot",
            "request": {},
        }),
        "EntryPointNotFound",
        &[],
    )
    .await;

    drop_postgres_schema(&database_url, &schema).await;
}
