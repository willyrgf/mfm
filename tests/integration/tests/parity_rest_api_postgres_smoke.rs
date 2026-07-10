#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::ffi::OsString;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_events::v1 as events;
use mfm_integration_tests::test_support::prepare_portfolio_launch_for_store;
use sqlx::{AssertSqlSafe, PgPool};
use tower::ServiceExt;

use mfm_integration_tests::test_support::response_json;
use mfm_store::v1::RunEventStore;
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
    let fact_query_receipt_trust_root = store.store_authority().fact_receipt_trust_root().cloned();
    let fact_query_authority_ready = store.fact_receipt_queries_ready();
    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        fact_index: mfm_app::production_fact_index_read_provider(store.clone()),
        store,
        runtime_config_path: None,
        fact_query_receipt_trust_root,
        fact_query_authority_ready,
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

struct FactReceiptSigningKeyEnvRestore {
    previous: Option<OsString>,
}

impl FactReceiptSigningKeyEnvRestore {
    fn clear() -> Self {
        let previous = std::env::var_os(mfm_app::MFM_FACT_RECEIPT_SIGNING_KEY_FILE);
        std::env::remove_var(mfm_app::MFM_FACT_RECEIPT_SIGNING_KEY_FILE);
        Self { previous }
    }

    fn set(&self, path: &std::path::Path) {
        std::env::set_var(mfm_app::MFM_FACT_RECEIPT_SIGNING_KEY_FILE, path);
    }

    fn remove(&self) {
        std::env::remove_var(mfm_app::MFM_FACT_RECEIPT_SIGNING_KEY_FILE);
    }
}

impl Drop for FactReceiptSigningKeyEnvRestore {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(path) => std::env::set_var(mfm_app::MFM_FACT_RECEIPT_SIGNING_KEY_FILE, path),
            None => std::env::remove_var(mfm_app::MFM_FACT_RECEIPT_SIGNING_KEY_FILE),
        }
    }
}

#[tokio::test]
async fn postgres_fact_receipt_authority_covers_provisioning_and_admission() {
    let _signing_key_env = FactReceiptSigningKeyEnvRestore::clear();
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_schema();
    create_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);
    let key_dir = tempfile::tempdir().expect("fact receipt key tempdir");
    let key_a_path = key_dir.path().join("key-a");
    let key_b_path = key_dir.path().join("key-b");
    std::fs::write(&key_a_path, [0x11_u8; 32]).expect("write key a");
    std::fs::write(&key_b_path, [0x22_u8; 32]).expect("write key b");
    PostgresSchema::migrate(&scoped_database_url)
        .await
        .expect("migrate authority lifecycle schema");

    let no_authority_store = mfm_app::connect_production_run_store(Some(&scoped_database_url))
        .await
        .expect("production store without receipt authority");
    assert!(no_authority_store
        .store_authority()
        .fact_receipt_trust_root()
        .is_none());
    assert!(!no_authority_store.fact_receipt_queries_ready());
    let no_authority_services =
        mfm_app::connect_production_run_services(Some(&scoped_database_url), None)
            .await
            .expect("unrelated production services do not need receipt authority");
    let prepared = prepare_portfolio_launch_for_store(
        &no_authority_store,
        &portfolio_fact_authority_config(),
        None,
    )
    .await;
    let run_id = prepared.request.run_id.clone();
    let error = no_authority_services
        .launch_prepared_entry_point_run(prepared)
        .await
        .expect_err("fact-reading launch must fail before admission without authority");
    assert_eq!(error.code, "FactReceiptAuthorityUnavailable");
    assert!(no_authority_store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .is_empty());

    _signing_key_env.set(&key_a_path);
    let no_root_with_signer =
        match mfm_app::connect_production_run_store(Some(&scoped_database_url)).await {
            Ok(_) => panic!("signer without a persisted root must fail at connection"),
            Err(error) => error,
        };
    assert_eq!(no_root_with_signer.code, "FactReceiptSignerInvalid");

    mfm_app::provision_production_fact_receipt_authority(Some(&scoped_database_url))
        .await
        .expect("provision public receipt authority");
    _signing_key_env.remove();
    let root_only = mfm_app::connect_production_run_store(Some(&scoped_database_url))
        .await
        .expect("connect with root but no signer");
    assert!(root_only
        .store_authority()
        .fact_receipt_trust_root()
        .is_some());
    assert!(!root_only.fact_receipt_queries_ready());

    _signing_key_env.set(&key_b_path);
    let mismatched = match mfm_app::connect_production_run_store(Some(&scoped_database_url)).await {
        Ok(_) => panic!("mismatched signer must fail at connection"),
        Err(error) => error,
    };
    assert_eq!(mismatched.code, "FactReceiptSignerInvalid");
    assert!(!mismatched.message.contains("222222"));

    _signing_key_env.set(&key_a_path);
    let matching_store = mfm_app::connect_production_run_store(Some(&scoped_database_url))
        .await
        .expect("connect with matching signer");
    assert!(matching_store.fact_receipt_queries_ready());
    let matching_services =
        mfm_app::connect_production_run_services(Some(&scoped_database_url), None)
            .await
            .expect("production services with matching receipt authority");
    let prepared = prepare_portfolio_launch_for_store(
        &matching_store,
        &portfolio_fact_authority_config(),
        Some("matching-authority"),
    )
    .await;
    let run_id = prepared.request.run_id.clone();
    let report = matching_services
        .launch_prepared_entry_point_run(prepared)
        .await
        .expect("signed empty fact query reaches the portfolio runner");
    assert_eq!(
        report.run.expect("admitted run").run_mode,
        mfm_app::RunModeStatus::FailedWithoutAcdcClaim
    );
    let stream = matching_store
        .load_run_stream(&run_id)
        .await
        .expect("matching-authority stream");
    assert!(stream
        .iter()
        .any(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_))));
    assert!(stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptFailed(payload)
                if payload.error.code.as_str() == "FactReceiptAuthorityUnavailable"
        )
    }));

    drop_schema(&database_url, &schema).await;
}

fn portfolio_fact_authority_config() -> serde_json::Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "postgres-authority",
            "quote_codes": ["USD"],
            "networks": [{
                "network_id": "postgres-authority-eth",
                "family": "evm",
                "chain_id": 31337,
                "metadata": {}
            }],
            "wallets": [{
                "wallet_id": "wallet_postgres_authority",
                "subject": {
                    "kind": "evm_address",
                    "address": "0x000000000000000000000000000000000000dead"
                },
                "implementation": {"kind": "address_only"},
                "network_id": "postgres-authority-eth",
                "symbol_ids": ["eth.native.postgres-authority-eth"],
                "metadata": {}
            }],
            "symbol_configs": [{
                "symbol_id": "eth.native.postgres-authority-eth",
                "display_symbol": "ETH",
                "kind": "native_balance",
                "role": "native",
                "network_id": "postgres-authority-eth",
                "protocol": null,
                "balance_reader": {"kind": "native_balance"},
                "valuation": {
                    "quotes": [{
                        "quote": "USD",
                        "priced_symbol_id": "eth.native.postgres-authority-eth",
                        "unit_price_dec": "2.50"
                    }]
                },
                "underlying_symbol_id": null,
                "metadata": {}
            }],
            "metadata": {}
        }
    })
}
