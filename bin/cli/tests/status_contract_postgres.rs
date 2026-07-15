#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::process::Output;

use assert_cmd::Command;
use mfm_app::{PostgresSchema, PostgresStore};
use mfm_store::v1 as store;
use sqlx::{AssertSqlSafe, PgPool};

// This path-included shared support also serves other parity suites.
#[allow(dead_code)]
#[path = "../../../tests/integration/src/run_control_support.rs"]
mod run_control_support;

#[path = "support/mod.rs"]
mod support;

#[tokio::test]
async fn run_status_and_stream_preserve_interrupted_chain_head_attempt_history() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_schema();
    create_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);

    PostgresSchema::migrate(&scoped_database_url)
        .await
        .expect("migrate typed postgres schema");
    let store = PostgresStore::connect(&scoped_database_url)
        .await
        .expect("connect typed postgres store");
    let runtime_config_dir = tempfile::tempdir().expect("runtime config directory");
    let runtime_config_path = run_control_support::write_collectors_runtime_config_for_test(
        runtime_config_dir.path(),
        "http://127.0.0.1:8332",
    );
    let (run_id, certified) =
        run_control_support::admit_btc_chain_head_run_without_driving(&store, &runtime_config_path)
            .await;

    let interrupted_node = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("domain node");
    let interrupted_attempt_id = store::test_support::fixed_attempt_id_for_test(0x51);
    store::test_support::append_interrupted_attempt_for_test(
        &store,
        &run_id,
        certified.spec_hash(),
        interrupted_node,
        &interrupted_attempt_id,
    )
    .await;

    let status = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "status".to_owned(),
        run_id.as_str().to_owned(),
        "--database-url".to_owned(),
        scoped_database_url.clone(),
    ]);
    assert_success(&status);
    let status_json = support::parse_success_json(&status.stdout);
    let attempts = status_json["attempt_dispositions"]
        .as_array()
        .expect("attempt dispositions");
    let interrupted = attempts
        .iter()
        .find(|attempt| {
            attempt["attempt_id"] == interrupted_attempt_id.as_str()
                && attempt["disposition"] == "interrupted"
        })
        .expect("interrupted attempt disposition");
    assert!(interrupted["retryable"].is_null());

    let stream = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "stream".to_owned(),
        run_id.as_str().to_owned(),
        "--database-url".to_owned(),
        scoped_database_url,
    ]);
    assert_success(&stream);
    let stream_json = support::parse_success_json(&stream.stdout);
    let stream_events = stream_json["events"].as_array().expect("stream events");
    assert!(
        !stream_events.is_empty(),
        "stream must retain interrupted attempt history"
    );

    drop_schema(&database_url, &schema).await;
}

fn unique_schema() -> String {
    format!("cli_{}", uuid::Uuid::new_v4().simple())
}

async fn create_schema(database_url: &str, schema: &str) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect postgres");
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

fn run_cli(args: &[String]) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    support::sanitize_machine_readable_cli_env(&mut cmd);
    cmd.env_remove("MFM_RUNTIME_CONFIG_FILE");
    cmd.args(args).output().expect("execute mfm_cli")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
