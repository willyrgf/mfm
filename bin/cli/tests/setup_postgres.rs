#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::path::Path;

use assert_cmd::Command;
use axum::body::Body;
use axum::http::Request;
use mfm_app::PostgresSchema;
use mfm_events::v1::KernelEventPayload;
use mfm_ids::RunId;
use mfm_store::v1::{ArtifactEvidenceRef, CommittedRunStream, RunEventStore};
use serde_json::Value;
use sqlx::{AssertSqlSafe, PgPool};
use tempfile::TempDir;
use tower::ServiceExt;

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

const SETUP_FIXTURE: &str = include_str!("../../../examples/setup/organization.toml");
const EMPTY_PORTFOLIO_SETUP: &str = r#"
[[values]]
name = "acme/empty"
kind = "portfolio"

[values.value]
portfolio_id = "portfolio_empty"
quote_codes = ["USD"]
networks = []
wallets = []
symbol_configs = []
metadata = {}
"#;
const PORTFOLIO_ENTRY_POINT: &str = "mfm.portfolio/portfolio_snapshot@1";

fn json_output(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "CLI stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI JSON output")
}

fn json_error(output: std::process::Output) -> Value {
    assert!(!output.status.success(), "CLI unexpectedly succeeded");
    assert!(output.stdout.is_empty());
    serde_json::from_slice(&output.stderr).expect("CLI JSON error output")
}

fn run_cli(database_url: &str, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("mfm_cli")
        .expect("mfm_cli binary")
        .env("DATABASE_URL", database_url)
        .args(args)
        .output()
        .expect("run mfm_cli")
}

fn export_args<'a>(schema_id: &'a str, digest: &'a str, path: &'a Path) -> Vec<&'a str> {
    vec![
        "--output-format",
        "json",
        "setup",
        "export",
        "--name",
        "acme/dual-mainnet",
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
    let setup_path = setup_path.to_str().expect("setup path");

    let imported = json_output(run_cli(
        &scoped_url,
        &[
            "--output-format",
            "json",
            "setup",
            "import",
            "--file",
            setup_path,
        ],
    ));
    assert_eq!(imported["status"], "success");
    let values = imported["data"]["values"]
        .as_array()
        .expect("import values");
    assert_eq!(values.len(), 9);
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
        9
    );

    let first_output = directory.path().join("portfolio-one.json");
    let second_output = directory.path().join("portfolio-two.json");
    let exported = json_output(run_cli(
        &scoped_url,
        &export_args(schema_id, digest, &first_output),
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
        &export_args(schema_id, digest, &second_output),
    ));
    let second_bytes = std::fs::read(&second_output).expect("second export");
    assert_eq!(first_bytes, second_bytes);

    let existing = run_cli(&scoped_url, &export_args(schema_id, digest, &first_output));
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
    assert_eq!(count, 9);
    pool.close().await;
    drop_postgres_schema(&database_url, &schema).await;
}

#[tokio::test]
async fn cli_and_rest_start_have_identical_catalog_semantics() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema();
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    PostgresSchema::migrate(&scoped_url)
        .await
        .expect("migrate parity schema");

    let directory = TempDir::new().expect("temporary parity directory");
    let setup_path = directory.path().join("empty.toml");
    std::fs::write(&setup_path, EMPTY_PORTFOLIO_SETUP).expect("write parity setup");
    let setup_output = json_output(run_cli(
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
    let identity = setup_output["data"]["values"]
        .as_array()
        .and_then(|values| values.first())
        .expect("empty portfolio identity");
    let digest = identity["digest"].as_str().expect("portfolio digest");
    let request = serde_json::json!({
        "portfolio": {"name": "acme/empty", "digest": digest},
    });
    let request_path = directory.path().join("request.json");
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request).expect("request JSON"),
    )
    .expect("write parity request");

    let cli_output = json_output(run_cli(
        &scoped_url,
        &[
            "--output-format",
            "json",
            "run",
            "start",
            "--entry-point",
            PORTFOLIO_ENTRY_POINT,
            "--request",
            request_path.to_str().expect("request path"),
            "--invocation-key",
            "cli-semantic-parity",
        ],
    ));
    assert_eq!(cli_output["data"]["outcome"], "admitted");
    assert_eq!(cli_output["data"]["run"]["run_mode"], "completed");
    let cli_run_id = cli_output["data"]["run"]["run_id"]
        .as_str()
        .expect("CLI run id")
        .parse::<RunId>()
        .expect("CLI run id identity");

    let store = mfm_app::PostgresStore::connect(&scoped_url)
        .await
        .expect("connect parity store");
    let rest_app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        role: mfm_rest_api::RestProcessRole::Live,
        store: store.clone(),
        catalog_store: Some(store.clone()),
        runtime_config_path: None,
        fact_index: mfm_app::production_fact_index_read_provider(store.clone()),
    });
    let rest_response = rest_app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs/start")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "entry_point": PORTFOLIO_ENTRY_POINT,
                        "request": request,
                        "invocation_key": "rest-semantic-parity",
                    }))
                    .expect("REST request JSON"),
                ))
                .expect("REST start request"),
        )
        .await
        .expect("REST start response");
    assert_eq!(rest_response.status(), axum::http::StatusCode::OK);
    let rest_output = axum::body::to_bytes(rest_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("REST response bytes");
    let rest_output: Value = serde_json::from_slice(&rest_output).expect("REST response JSON");
    assert_eq!(rest_output["status"], "success");
    assert_eq!(rest_output["data"]["outcome"], "admitted");
    assert_eq!(rest_output["data"]["run"]["run_mode"], "completed");
    let rest_run_id = rest_output["data"]["run"]["run_id"]
        .as_str()
        .expect("REST run id")
        .parse::<RunId>()
        .expect("REST run id identity");

    let cli_stream = store
        .load_committed_run_stream(&cli_run_id)
        .await
        .expect("CLI committed stream");
    let rest_stream = store
        .load_committed_run_stream(&rest_run_id)
        .await
        .expect("REST committed stream");
    assert_equal_launch_semantics(&cli_stream, &rest_stream);

    let invalid_request = serde_json::json!({
        "portfolio": {"name": "acme/empty"},
        "sentinel_unknown": "must-not-leak",
    });
    let invalid_path = directory.path().join("invalid.json");
    std::fs::write(
        &invalid_path,
        serde_json::to_vec(&invalid_request).expect("invalid request JSON"),
    )
    .expect("write invalid request");
    let cli_error = json_error(run_cli(
        &scoped_url,
        &[
            "--output-format",
            "json",
            "run",
            "start",
            "--entry-point",
            PORTFOLIO_ENTRY_POINT,
            "--request",
            invalid_path.to_str().expect("invalid request path"),
            "--invocation-key",
            "cli-semantic-error-parity",
        ],
    ));
    assert_eq!(cli_error["error"]["code"], "EntryPointRequestInvalid");
    assert!(!cli_error.to_string().contains("must-not-leak"));

    let rest_error_response = mfm_rest_api::make_app(mfm_rest_api::AppState {
        role: mfm_rest_api::RestProcessRole::Live,
        store: store.clone(),
        catalog_store: Some(store.clone()),
        runtime_config_path: None,
        fact_index: mfm_app::production_fact_index_read_provider(store.clone()),
    })
    .oneshot(
        Request::builder()
            .method("POST")
            .uri("/v1/runs/start")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "entry_point": PORTFOLIO_ENTRY_POINT,
                    "request": invalid_request,
                    "invocation_key": "rest-semantic-error-parity",
                }))
                .expect("REST invalid request JSON"),
            ))
            .expect("REST invalid start request"),
    )
    .await
    .expect("REST invalid response");
    assert_eq!(
        rest_error_response.status(),
        axum::http::StatusCode::BAD_REQUEST
    );
    let rest_error_bytes = axum::body::to_bytes(rest_error_response.into_body(), 1024 * 1024)
        .await
        .expect("REST error bytes");
    let rest_error: Value = serde_json::from_slice(&rest_error_bytes).expect("REST error JSON");
    assert_eq!(rest_error["error"]["code"], "EntryPointRequestInvalid");
    assert!(!rest_error.to_string().contains("must-not-leak"));

    drop_postgres_schema(&database_url, &schema).await;
}

fn assert_equal_launch_semantics(cli: &CommittedRunStream, rest: &CommittedRunStream) {
    let cli_admitted = admitted_payload(cli);
    let rest_admitted = admitted_payload(rest);
    assert_eq!(cli_admitted.entry_point, rest_admitted.entry_point);
    assert_eq!(cli_admitted.spec_hash, rest_admitted.spec_hash);
    assert_eq!(cli_admitted.spec_artifact, rest_admitted.spec_artifact);
    assert_eq!(
        cli_admitted.certificate_artifact,
        rest_admitted.certificate_artifact
    );
    assert_eq!(
        cli_admitted.config_artifacts,
        rest_admitted.config_artifacts
    );
    assert_eq!(
        cli_admitted.fact_descriptor_artifacts,
        rest_admitted.fact_descriptor_artifacts
    );
    assert_eq!(cli_admitted.spec_version, rest_admitted.spec_version);
    assert_eq!(
        cli_admitted.lowering_version,
        rest_admitted.lowering_version
    );
    assert_eq!(
        cli_admitted.public_output_schema_id,
        rest_admitted.public_output_schema_id
    );
    assert_eq!(
        cli_admitted.saga_policy_digest,
        rest_admitted.saga_policy_digest
    );
    assert_eq!(
        cli_admitted.descriptor_identities,
        rest_admitted.descriptor_identities
    );
    assert_eq!(
        cli_admitted.runner_executables,
        rest_admitted.runner_executables
    );
    assert_eq!(
        cli_admitted.adapter_executables,
        rest_admitted.adapter_executables
    );
    assert_eq!(
        cli_admitted.admitted_binding_digest,
        rest_admitted.admitted_binding_digest
    );
    assert_eq!(
        cli_admitted.canonicalizer_identity,
        rest_admitted.canonicalizer_identity
    );
    assert_eq!(cli_admitted.seed_cells, rest_admitted.seed_cells);
    assert_eq!(
        cli_admitted.identity_material.certified_spec_hash,
        rest_admitted.identity_material.certified_spec_hash
    );
    assert_eq!(
        cli_admitted.identity_material.store_scope_id,
        rest_admitted.identity_material.store_scope_id
    );
    assert_ne!(
        cli_admitted.identity_material.invocation_key_digest,
        rest_admitted.identity_material.invocation_key_digest
    );
    assert_eq!(
        artifact_bytes_for_refs(cli, &cli_admitted.config_artifacts),
        artifact_bytes_for_refs(rest, &rest_admitted.config_artifacts)
    );
    assert_eq!(
        artifact_bytes_for_refs(cli, std::slice::from_ref(&cli_admitted.spec_artifact)),
        artifact_bytes_for_refs(rest, std::slice::from_ref(&rest_admitted.spec_artifact))
    );
    assert_eq!(
        artifact_bytes_for_refs(
            cli,
            std::slice::from_ref(&cli_admitted.certificate_artifact)
        ),
        artifact_bytes_for_refs(
            rest,
            std::slice::from_ref(&rest_admitted.certificate_artifact)
        )
    );
}

fn admitted_payload(stream: &CommittedRunStream) -> mfm_events::v1::RunAdmitted {
    stream
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            KernelEventPayload::RunAdmitted(payload) => Some((**payload).clone()),
            _ => None,
        })
        .expect("RunAdmitted payload")
}

fn artifact_bytes_for_refs(
    stream: &CommittedRunStream,
    refs: &[mfm_events::v1::RunArtifactEvidenceRef],
) -> Vec<Vec<u8>> {
    refs.iter()
        .map(|reference| {
            let evidence = ArtifactEvidenceRef::from_run_artifact(reference);
            let evidence_hash = evidence.evidence_hash().expect("artifact evidence hash");
            stream
                .artifact_byte_authority()
                .get(&(evidence.artifact_id, evidence_hash))
                .map(|(bytes, _)| bytes.clone())
                .expect("retained artifact bytes")
        })
        .collect()
}
