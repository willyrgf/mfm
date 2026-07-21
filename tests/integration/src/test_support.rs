#![warn(missing_docs)]
//! Shared helpers for MFM integration tests.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use mfm_events::v1::{ArtifactRole, KernelEventPayload};
use mfm_fact_capabilities::{FactIndexReadCapability, FactIndexReadProvider, FactRecordCapability};
use mfm_store::v1 as store;
use mfm_store::v1::RetainedArtifactReadProvider;
use sqlx::{AssertSqlSafe, PgPool};

#[path = "run_control_support.rs"]
mod run_control_support;

pub use run_control_support::{
    set_evm_runtime_config_env_with_signer_for_test, start_portfolio_rpc_mock,
    write_evm_runtime_config_for_test, write_portfolio_runtime_config_for_test, EnvVarRestore,
    RuntimeConfigSignerBinding, ENV_RUNTIME_CONFIG_FILE,
};

/// Re-export: merge-safe Platform holding seed for store-backed portfolio report tests.
pub use store::test_support::{
    append_platform_holding_facts_for_test, FactRecordFixtureInputForTest,
    PlatformHoldingFactSeedForTest,
};

/// In-memory REST app state used by integration tests.
pub type InMemoryRestAppState = mfm_rest_api::AppState<store::AsyncInMemoryRunStore>;

/// Store-backed projection fact-index (shared with app process assembly tests).
pub use mfm_app::ProjectionFactIndexProvider;

/// Creates a unique schema name for an isolated Postgres parity test.
pub fn unique_postgres_schema(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

/// Creates an isolated Postgres schema for a parity test.
pub async fn create_postgres_schema(database_url: &str, schema: &str) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect postgres");
    // Schema names are generated from UUIDs and never come from user input; dynamic DDL is
    // required because PostgreSQL does not parameterize identifiers.
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&pool)
        .await
        .expect("create schema");
    pool.close().await;
}

/// Drops an isolated Postgres schema after a parity test.
pub async fn drop_postgres_schema(database_url: &str, schema: &str) {
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

/// Adds a Postgres search-path option to an isolated parity-test database URL.
pub fn schema_scoped_database_url(database_url: &str, schema: &str) -> String {
    let separator = if database_url.contains('?') { '&' } else { '?' };
    format!("{database_url}{separator}options=-csearch_path%3D{schema}")
}

/// Migrates and connects a Postgres run store, retrying while the managed service becomes ready.
pub async fn connect_postgres_with_retry(
    database_url: &str,
    max_attempts: u32,
    delay_ms: u64,
) -> mfm_storage_postgres::PostgresStore {
    let mut last_err: Option<mfm_storage_postgres::PostgresStoreError> = None;
    for _ in 0..max_attempts {
        match mfm_storage_postgres::PostgresSchema::migrate(database_url).await {
            Ok(()) => match mfm_storage_postgres::PostgresStore::connect(database_url).await {
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

/// Binds the shared fact capabilities owned by an integration-test process.
pub fn register_process_fact_capabilities(
    registry: &mut mfm_runtime::ErasedRunnerRegistry,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<()> {
    registry.register_capability_spec::<FactIndexReadCapability>(
        mfm_runtime::CapabilityImplementationId::new(fact_index.implementation_id())?,
    )?;
    registry.register_capability_spec::<FactRecordCapability>(
        mfm_runtime::CapabilityImplementationId::new("mfm.integration.managed-fact-record.v1")?,
    )
}

/// Builds in-memory REST app state.
pub fn in_memory_rest_app_state() -> InMemoryRestAppState {
    let store = store::AsyncInMemoryRunStore::default();
    let fact_index = Arc::new(ProjectionFactIndexProvider::new(store.clone()));
    mfm_rest_api::AppState {
        role: mfm_rest_api::RestProcessRole::Live,
        store,
        configured_store: None,
        runtime_config_path: None,
        fact_index,
    }
}

/// Loads all retained fact-query evidence artifacts referenced by `stream`.
pub async fn fact_query_evidences(
    store: &store::AsyncInMemoryRunStore,
    stream: &[store::KernelEventEnvelope],
) -> Vec<mfm_facts::FactQueryEvidence> {
    let mut evidences = Vec::new();
    for event in stream {
        let KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != ArtifactRole::FactQueryEvidence {
            continue;
        }
        let requirement = event
            .payload()
            .artifact_requirements()
            .into_iter()
            .next()
            .expect("query evidence artifact requirement");
        let artifact = store
            .read_retained_artifact(&requirement)
            .await
            .expect("query evidence artifact");
        evidences.push(
            mfm_facts::parse_canonical_fact_query_evidence_bytes(artifact.bytes())
                .expect("query evidence bytes"),
        );
    }
    evidences
}

/// Builds a JSON POST request for REST integration tests.
pub fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let payload = serde_json::to_string(&body).expect("request body serializes");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(payload))
        .expect("request")
}

/// Builds an empty-body POST request for REST integration tests.
pub fn empty_post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// Parses an Axum response body as JSON for REST integration tests.
pub async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("response json")
}
