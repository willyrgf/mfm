#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use sqlx::{AssertSqlSafe, PgPool};
use tower::ServiceExt;

use mfm_integration_tests::test_support::response_json;
use mfm_stream_store_postgres::{PostgresRunStore, PostgresSchema, PostgresStoreError};

const VALID_RUN_ID: &str =
    "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000033";

async fn connect_postgres_with_retry(
    database_url: &str,
    max_attempts: u32,
    delay_ms: u64,
) -> PostgresRunStore {
    let mut last_err: Option<PostgresStoreError> = None;
    for _ in 0..max_attempts {
        match PostgresSchema::migrate(database_url).await {
            Ok(()) => match PostgresRunStore::connect(database_url).await {
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

#[tokio::test]
async fn parity_rest_postgres_smoke() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_schema();
    create_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_database_url, 20, 250).await;
    // Ready/smoke only (no SelectHoldings). Live portfolio assembly uses
    // production_fact_index_read_provider; empty projection is enough for readiness.
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        store,
        runtime_config_path: None,
        fact_query_receipt_trust_root: None,
        fact_index: mfm_app::ProjectionFactIndexProvider::empty_arc(),
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
    drop_schema(&database_url, &schema).await;
}
