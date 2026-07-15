#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::path::Path;

use assert_cmd::Command;
use mfm_app::PostgresSchema;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool};
use tempfile::TempDir;

const SETUP_FIXTURE: &str = include_str!("../../../examples/setup/organization.toml");

fn unique_postgres_schema() -> String {
    format!("cli_setup_{}", uuid::Uuid::new_v4().simple())
}

async fn create_postgres_schema(database_url: &str, schema: &str) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect postgres");
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&pool)
        .await
        .expect("create schema");
    pool.close().await;
}

async fn drop_postgres_schema(database_url: &str, schema: &str) {
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

fn json_output(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "CLI stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON output")
}

fn run_cli(database_url: &str, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("mfm_cli")
        .expect("mfm_cli binary")
        .env("DATABASE_URL", database_url)
        .args(args)
        .output()
        .expect("run mfm_cli")
}

fn export_args<'a>(
    name: &'a str,
    schema_id: &'a str,
    digest: &'a str,
    path: &'a Path,
) -> Vec<&'a str> {
    vec![
        "--output-format",
        "json",
        "setup",
        "export",
        "--name",
        name,
        "--schema-id",
        schema_id,
        "--digest",
        digest,
        "--output",
        path.to_str().expect("export path"),
    ]
}

#[tokio::test]
async fn setup_cli_import_list_and_export_preserve_the_catalog_contract() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema();
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    PostgresSchema::migrate(&scoped_url)
        .await
        .expect("migrate CLI schema");

    let directory = TempDir::new().expect("temporary setup directory");
    let setup_path = directory.path().join("organization.toml");
    std::fs::write(&setup_path, SETUP_FIXTURE).expect("write setup fixture");
    let imported = json_output(run_cli(
        &scoped_url,
        &[
            "--output-format",
            "json",
            "setup",
            "import",
            "--file",
            setup_path.to_str().expect("setup path"),
        ],
    ));
    assert_eq!(imported["status"], "success");
    let values = imported["data"]["values"]
        .as_array()
        .expect("import values");
    assert_eq!(values.len(), 7);
    for value in values {
        let object = value.as_object().expect("identity object");
        assert_eq!(object.len(), 3);
        assert!(object.contains_key("name"));
        assert!(object.contains_key("schema_id"));
        assert!(object.contains_key("digest"));
    }
    let identity = values
        .iter()
        .find(|value| value["name"] == "acme/contract-context")
        .expect("contract context identity");
    let schema_id = identity["schema_id"].as_str().expect("schema id");
    let digest = identity["digest"].as_str().expect("digest");

    let listed = json_output(run_cli(
        &scoped_url,
        &["--output-format", "json", "setup", "list", "--limit", "100"],
    ));
    assert_eq!(listed["status"], "success");
    assert_eq!(
        listed["data"]["values"]
            .as_array()
            .expect("listed values")
            .len(),
        7
    );

    let first_output = directory.path().join("context-one.json");
    let second_output = directory.path().join("context-two.json");
    let exported = json_output(run_cli(
        &scoped_url,
        &export_args("acme/contract-context", schema_id, digest, &first_output),
    ));
    assert_eq!(
        exported["data"]["values"]
            .as_array()
            .expect("export identity")
            .len(),
        1
    );
    let first_bytes = std::fs::read(&first_output).expect("first export");
    json_output(run_cli(
        &scoped_url,
        &export_args("acme/contract-context", schema_id, digest, &second_output),
    ));
    let second_bytes = std::fs::read(&second_output).expect("second export");
    assert_eq!(first_bytes, second_bytes);

    let existing = run_cli(
        &scoped_url,
        &export_args("acme/contract-context", schema_id, digest, &first_output),
    );
    assert!(!existing.status.success());
    let error: Value = serde_json::from_slice(&existing.stderr).expect("existing path error JSON");
    assert_eq!(error["error"]["code"], "CatalogExportPathExists");

    let pool = PgPool::connect(&scoped_url)
        .await
        .expect("connect CLI schema");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_values")
        .fetch_one(&pool)
        .await
        .expect("catalog count");
    assert_eq!(count, 7);
    pool.close().await;
    drop_postgres_schema(&database_url, &schema).await;
}
