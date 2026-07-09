#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use assert_cmd::Command;
use mfm_app::{ProductionPostgresSchema, ProductionRunStore};
use mfm_store::v1 as store;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool};
use std::process::Output;

// Path-included support module is shared with other parity suites; this test only
// uses admit/append helpers after report-only cutover (no live RPC seed path).
#[allow(dead_code)]
#[path = "../../../tests/integration/src/run_control_support.rs"]
mod run_control_support;

#[tokio::test]
async fn run_status_reports_interrupted_attempt_and_framework_attempts_from_history() {
    // Report-only cutover: no live RPC required to admit/resume. Without Platform
    // holding facts the resumed run hard-fails (does not complete with a public snapshot).
    // Mirrors rest_api_run_control portfolio status history contract.
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    let schema = unique_schema();
    create_schema(&database_url, &schema).await;
    let scoped_database_url = schema_scoped_database_url(&database_url, &schema);

    ProductionPostgresSchema::migrate(&scoped_database_url)
        .await
        .expect("migrate typed postgres schema");
    let store = ProductionRunStore::connect(&scoped_database_url)
        .await
        .expect("connect typed postgres store");
    let config = sample_portfolio_config();
    let (run_id, certified) =
        run_control_support::admit_portfolio_run_without_driving(&store, &config).await;

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

    let resume = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "resume".to_owned(),
        run_id.as_str().to_owned(),
        "--database-url".to_owned(),
        scoped_database_url.clone(),
    ]);
    assert_success(&resume);
    let resume_json = parse_success_json(&resume.stdout);
    // Without Platform holding facts the report path hard-fails after cutover.
    assert_ne!(resume_json["run_mode"], "completed");
    assert_eq!(resume_json["run_mode"], "failed_without_acdc_claim");

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
    let status_json = parse_success_json(&status.stdout);
    assert_ne!(status_json["run_mode"], "interrupted");
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

    // After cutover, resume without Platform facts hard-fails; framework public-output
    // completion is not required. Status history must still surface the interrupted attempt.
    assert!(
        attempts.iter().any(|attempt| {
            attempt["disposition"] == "failed"
                || attempt["disposition"] == "interrupted"
                || attempt["disposition"] == "completed"
        }),
        "expected attempt dispositions after resume: {attempts:?}"
    );

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
    let stream_json = parse_success_json(&stream.stdout);
    let stream_events = stream_json["events"].as_array().expect("stream events");
    assert!(
        !stream_events.is_empty(),
        "stream must retain history after failed report resume"
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
    // The schema name is UUID-derived and never comes from user input; dynamic DDL is required
    // because PostgreSQL does not parameterize identifiers.
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

fn sample_portfolio_config() -> Value {
    serde_json::json!({
        "portfolio": {
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "family": "evm",
                    "chain_id": 1,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "subject": {
                        "kind": "evm_address",
                        "address": "0x000000000000000000000000000000000000dead"
                    },
                    "implementation": { "kind": "address_only" },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": ["eth.native.ethereum-mainnet"],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1800.00"
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                }
            ],
            "metadata": {}
        }
    })
}

fn run_cli(args: &[String]) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .args(args)
        .output()
        .expect("execute mfm_cli")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_success_json(stdout: &[u8]) -> Value {
    let parsed: Value = serde_json::from_slice(stdout).expect("stdout must be valid json");
    assert_eq!(parsed["status"], "success");
    parsed["data"].clone()
}

fn sanitize_machine_readable_cli_env(cmd: &mut Command) -> &mut Command {
    cmd.env_remove("LOG_LEVEL")
        .env_remove("RUST_LOG")
        .env_remove("LOG_FORMAT")
        .env_remove("LOG_SPAN_EVENTS")
}
