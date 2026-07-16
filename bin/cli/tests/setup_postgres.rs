#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::path::Path;

use assert_cmd::Command;
use mfm_app::{PostgresSchema, PostgresStore};
use mfm_events::v1::KernelEventPayload;
use mfm_store::v1::RunEventStore;
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool};
use tempfile::TempDir;

// This path-included shared support also serves the integration parity suites.
#[allow(dead_code)]
#[path = "../../../tests/integration/src/run_control_support.rs"]
mod run_control_support;

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
async fn setup_cli_catalog_and_snapshot_start_preserve_the_public_contract() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema();
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    PostgresSchema::migrate(&scoped_url)
        .await
        .expect("migrate CLI schema");

    let entry_points = json_output(run_cli(
        &scoped_url,
        &["--output-format", "json", "ops", "list"],
    ));
    assert_eq!(entry_points["status"], "success");
    let entry_points = entry_points["data"]["entry_points"]
        .as_array()
        .expect("CLI entry-point list");
    assert_eq!(entry_points.len(), 1);
    assert_eq!(
        entry_points[0]["entry_point_id"], "mfm.portfolio/snapshot@1",
        "CLI discovery exposes exactly the one public portfolio objective"
    );
    assert!(entry_points[0]["request_schema_id"].is_string());

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
    assert_eq!(values.len(), 1);
    for value in values {
        let object = value.as_object().expect("identity object");
        assert_eq!(object.len(), 3);
        assert!(object.contains_key("name"));
        assert!(object.contains_key("schema_id"));
        assert!(object.contains_key("digest"));
    }
    let identity = values
        .iter()
        .find(|value| value["name"] == "acme/dual-mainnet")
        .expect("portfolio identity");
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
        1
    );

    let first_output = directory.path().join("portfolio-one.json");
    let second_output = directory.path().join("portfolio-two.json");
    let exported = json_output(run_cli(
        &scoped_url,
        &export_args("acme/dual-mainnet", schema_id, digest, &first_output),
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
        &export_args("acme/dual-mainnet", schema_id, digest, &second_output),
    ));
    let second_bytes = std::fs::read(&second_output).expect("second export");
    assert_eq!(first_bytes, second_bytes);

    let existing = run_cli(
        &scoped_url,
        &export_args("acme/dual-mainnet", schema_id, digest, &first_output),
    );
    assert!(!existing.status.success());
    let error: Value = serde_json::from_slice(&existing.stderr).expect("existing path error JSON");
    assert_eq!(error["error"]["code"], "CatalogExportPathExists");

    let portfolio_name = identity["name"].as_str().expect("portfolio name");
    let portfolio_digest = identity["digest"].as_str().expect("portfolio digest");
    let request_path = directory.path().join("snapshot-request.json");
    let request = serde_json::json!({
        "portfolio": {
            "name": portfolio_name,
            "digest": portfolio_digest,
        }
    });
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("serialize snapshot request"),
    )
    .expect("write snapshot request");
    let rpc_url = run_control_support::start_collectors_rpc_mock().await;
    let runtime_config_path =
        run_control_support::write_collectors_runtime_config_for_test(directory.path(), &rpc_url);
    let started = json_output(run_cli(
        &scoped_url,
        &[
            "--output-format",
            "json",
            "run",
            "start",
            "--entry-point",
            "mfm.portfolio/snapshot@1",
            "--request",
            request_path.to_str().expect("snapshot request path"),
            "--invocation-key",
            "postgres-cli-portfolio-snapshot",
            "--runtime-config",
            runtime_config_path.to_str().expect("runtime config path"),
        ],
    ));
    assert_eq!(started["status"], "success");
    assert_eq!(started["data"]["outcome"], "admitted");
    assert_eq!(started["data"]["run"]["run_mode"], "completed");
    let run_id: mfm_ids::RunId = started["data"]["run"]["run_id"]
        .as_str()
        .expect("started run id")
        .parse()
        .expect("typed started run id");
    let store = PostgresStore::connect(&scoped_url)
        .await
        .expect("connect catalog-backed run store");
    let stream = store
        .load_run_stream(&run_id)
        .await
        .expect("started run stream");
    let admitted = stream
        .iter()
        .find_map(|event| match event.payload() {
            KernelEventPayload::RunAdmitted(payload) => Some(payload.as_ref()),
            _ => None,
        })
        .expect("run admission evidence");
    assert_eq!(
        admitted.entry_point.entry_point_id.as_str(),
        "mfm.portfolio/snapshot@1"
    );
    assert_eq!(admitted.entry_point.catalog_sources.len(), 1);
    let source = &admitted.entry_point.catalog_sources[0];
    assert_eq!(source.name.as_str(), portfolio_name);
    assert_eq!(source.schema_id.as_str(), schema_id);
    assert_eq!(source.digest.as_str(), portfolio_digest);

    std::fs::remove_file(&runtime_config_path).expect("remove live runtime config");
    let replay = json_output(run_cli(
        &scoped_url,
        &["--output-format", "json", "run", "replay", run_id.as_str()],
    ));
    assert_eq!(replay["status"], "success");
    assert_eq!(replay["data"]["run_mode"], "completed");

    let pool = PgPool::connect(&scoped_url)
        .await
        .expect("connect CLI schema");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM catalog_values")
        .fetch_one(&pool)
        .await
        .expect("catalog count");
    assert_eq!(count, 1);
    pool.close().await;
    drop_postgres_schema(&database_url, &schema).await;
}
