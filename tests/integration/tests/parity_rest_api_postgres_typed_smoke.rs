#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use sqlx::{AssertSqlSafe, PgPool};
use tower::ServiceExt;

use mfm_stream_store_postgres::{
    PostgresSchema, PostgresTypedRunEventStore, PostgresTypedStoreError,
};

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000033";

async fn connect_typed_postgres_with_retry(
    database_url: &str,
    max_attempts: u32,
    delay_ms: u64,
) -> PostgresTypedRunEventStore {
    let mut last_err: Option<PostgresTypedStoreError> = None;
    for _ in 0..max_attempts {
        match PostgresSchema::migrate(database_url).await {
            Ok(()) => match PostgresTypedRunEventStore::connect(database_url).await {
                Ok(store) => return store,
                Err(err) => {
                    last_err = Some(err);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
            },
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    panic!(
        "typed postgres config after retries (DATABASE_URL redacted): {:?}",
        last_err
    );
}

fn unique_schema() -> String {
    format!("rest_{}", uuid::Uuid::new_v4().simple())
}

async fn create_schema(database_url: &str, schema: &str) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect postgres");
    // The schema name is generated from a UUID and never comes from user input; dynamic DDL is
    // required because PostgreSQL does not parameterize identifiers.
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&pool)
        .await
        .expect("create schema");
    pool.close().await;
}

async fn drop_schema(database_url: &str, schema: &str) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect postgres");
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {schema} CASCADE"
    )))
    .execute(&pool)
    .await
    .expect("drop schema");
    pool.close().await;
}

fn schema_scoped_database_url(database_url: &str, schema: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}options=-csearch_path%3D{schema}")
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
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_schema();
    create_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_typed_postgres_with_retry(&scoped_database_url, 20, 250).await;
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
    drop_schema(&database_url, &schema).await;
}
