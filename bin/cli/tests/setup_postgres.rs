#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::sync::Arc;

use assert_cmd::Command;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use mfm_app::{PostgresSchema, PostgresStore};
use mfm_events::v1::KernelEventPayload;
use mfm_store::v1::{CommitOutcome, RunEventStore, StoreScopeStore};
use serde_json::{json, Value};
use sqlx::{AssertSqlSafe, PgPool};
use tempfile::TempDir;
use tower::ServiceExt;

// This path-included shared support also serves the integration parity suites.
#[allow(dead_code)]
#[path = "../../../tests/integration/src/run_control_support.rs"]
mod run_control_support;

const SETUP_FIXTURE: &str = include_str!("../../../examples/setup/organization.toml");
const TARGET: &str = "acme/primary";
const STABLE_INVOCATION_KEY: &str = "postgres-cli-portfolio";
const UNFINISHED_TARGET: &str = "acme/resume";

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

fn json_error(output: std::process::Output) -> Value {
    assert!(!output.status.success(), "CLI unexpectedly succeeded");
    serde_json::from_slice(&output.stderr).expect("CLI JSON error")
}

fn run_cli(database_url: &str, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("mfm_cli")
        .expect("mfm_cli binary")
        .env("DATABASE_URL", database_url)
        .args(args)
        .output()
        .expect("run mfm_cli")
}

fn import_args(path: &Path) -> Vec<&str> {
    vec![
        "--output-format",
        "json",
        "setup",
        "import",
        path.to_str().expect("setup path"),
    ]
}

fn export_args<'a>(target: &'a str, path: &'a Path) -> Vec<&'a str> {
    vec![
        "--output-format",
        "json",
        "setup",
        "export",
        target,
        "--output",
        path.to_str().expect("export path"),
    ]
}

fn start_args<'a>(
    target: &'a str,
    invocation_key: &'a str,
    runtime_config_path: &'a Path,
) -> Vec<&'a str> {
    vec![
        "--output-format",
        "json",
        "run",
        "start",
        "mfm.portfolio/snapshot@1",
        target,
        "--invocation-key",
        invocation_key,
        "--runtime-config",
        runtime_config_path.to_str().expect("runtime config path"),
    ]
}

async fn admission_evidence(store: &PostgresStore, run_id: &mfm_ids::RunId) -> AdmissionEvidence {
    let stream = store
        .load_run_stream(run_id)
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
    assert_eq!(admitted.entry_point.configured_targets.len(), 1);
    let source = &admitted.entry_point.configured_targets[0];
    AdmissionEvidence {
        target: source.target.as_str().to_owned(),
        schema_id: source.schema_id.as_str().to_owned(),
        digest: source.digest.as_str().to_owned(),
        certified_spec_hash: admitted
            .identity_material
            .certified_spec_hash
            .as_str()
            .to_owned(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct AdmissionEvidence {
    target: String,
    schema_id: String,
    digest: String,
    certified_spec_hash: String,
}

async fn admit_configured_target_without_driving(
    store: &PostgresStore,
    target: &str,
    runtime_config_path: &Path,
    invocation_key: &str,
) -> mfm_ids::RunId {
    let request = mfm_app::prepare_entry_point_run_launch(
        store,
        "mfm.portfolio/snapshot@1",
        target,
        &mfm_app::production_certification_registry().expect("production certification registry"),
        store.load_store_scope_id().await.expect("store scope"),
        Some(mfm_app::InvocationKey::new(invocation_key).expect("invocation key")),
    )
    .await
    .expect("prepare configured target launch");
    let run_id = request.run_id.clone();
    let runners = mfm_app::production_runner_registry(
        Arc::new(store.clone()),
        mfm_app::production_fact_index_read_provider(store.clone()),
        Some(runtime_config_path),
    )
    .expect("production runners");
    let scheduler = mfm_runtime::SerialTypedScheduler::new(runners, Arc::new(store.clone()));
    let runtime_spec =
        mfm_runtime::CertifiedRuntimeSpec::new(request.certified_spec).expect("runtime spec");
    let launch = scheduler
        .prepare_run_launch(
            &runtime_spec,
            request.identity_material,
            request.evidence,
            store
                .expected_next_seq(&run_id)
                .await
                .expect("expected next sequence"),
        )
        .expect("prepare admission");
    assert!(matches!(
        scheduler
            .start_run(store, launch)
            .await
            .expect("admit configured target without driving"),
        CommitOutcome::Appended(_)
    ));
    run_id
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read REST body");
    serde_json::from_slice(&body).expect("REST JSON response")
}

#[tokio::test]
async fn configured_target_cli_and_rest_replace_current_config_with_stable_invocation_and_nonterminal_resume(
) {
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
    let entry_points = entry_points["data"]["entry_points"]
        .as_array()
        .expect("CLI entry-point list");
    assert_eq!(entry_points, &[json!("mfm.portfolio/snapshot@1")]);

    let directory = TempDir::new().expect("temporary setup directory");
    let setup_a_path = directory.path().join("organization-a.toml");
    std::fs::write(&setup_a_path, SETUP_FIXTURE).expect("write setup A");
    let imported_a = json_output(run_cli(&scoped_url, &import_args(&setup_a_path)));
    let configs_a = imported_a["data"]["configs"]
        .as_array()
        .expect("import A configs");
    assert_eq!(configs_a.len(), 1);
    let config_a = &configs_a[0];
    assert_eq!(config_a["target"], TARGET);
    assert_eq!(config_a["status"], "created");
    let digest_a = config_a["digest"].as_str().expect("A digest").to_owned();
    let schema_id = config_a["schema_id"]
        .as_str()
        .expect("schema id")
        .to_owned();

    let listed = json_output(run_cli(
        &scoped_url,
        &["--output-format", "json", "setup", "list"],
    ));
    assert_eq!(listed["data"]["targets"], json!([TARGET]));

    let first_output = directory.path().join("portfolio-one.json");
    let second_output = directory.path().join("portfolio-two.json");
    let exported = json_output(run_cli(&scoped_url, &export_args(TARGET, &first_output)));
    assert_eq!(exported["data"], json!({"target": TARGET}));
    let first_bytes = std::fs::read(&first_output).expect("first export");
    json_output(run_cli(&scoped_url, &export_args(TARGET, &second_output)));
    assert_eq!(
        first_bytes,
        std::fs::read(&second_output).expect("second export")
    );
    let existing = json_error(run_cli(&scoped_url, &export_args(TARGET, &first_output)));
    assert_eq!(existing["error"]["code"], "SetupExportPathExists");

    let rpc_url = run_control_support::start_collectors_rpc_mock().await;
    let runtime_config_path =
        run_control_support::write_collectors_runtime_config_for_test(directory.path(), &rpc_url);
    let started_a = json_output(run_cli(
        &scoped_url,
        &start_args(TARGET, STABLE_INVOCATION_KEY, &runtime_config_path),
    ));
    assert_eq!(started_a["data"]["outcome"], "admitted");
    assert_eq!(started_a["data"]["run"]["run_mode"], "completed");
    let run_a: mfm_ids::RunId = started_a["data"]["run"]["run_id"]
        .as_str()
        .expect("A run id")
        .parse()
        .expect("typed A run id");
    let store = PostgresStore::connect(&scoped_url)
        .await
        .expect("connect configured run store");
    let evidence_a = admission_evidence(&store, &run_a).await;
    assert_eq!(evidence_a.target, TARGET);
    assert_eq!(evidence_a.schema_id, schema_id);
    assert_eq!(evidence_a.digest, digest_a);

    let setup_b = SETUP_FIXTURE.replacen(
        "metadata = {}",
        "metadata = { revision = \"replacement\" }",
        1,
    );
    let setup_b_path = directory.path().join("organization-b.toml");
    std::fs::write(&setup_b_path, setup_b).expect("write setup B");
    let imported_b = json_output(run_cli(&scoped_url, &import_args(&setup_b_path)));
    let config_b = imported_b["data"]["configs"]
        .as_array()
        .and_then(|configs| configs.first())
        .expect("import B config");
    assert_eq!(config_b["target"], TARGET);
    assert_eq!(config_b["status"], "updated");
    let digest_b = config_b["digest"].as_str().expect("B digest").to_owned();
    assert_ne!(digest_b, digest_a);

    let started_b = json_output(run_cli(
        &scoped_url,
        &start_args(TARGET, STABLE_INVOCATION_KEY, &runtime_config_path),
    ));
    assert_eq!(started_b["data"]["outcome"], "admitted");
    let run_b: mfm_ids::RunId = started_b["data"]["run"]["run_id"]
        .as_str()
        .expect("B run id")
        .parse()
        .expect("typed B run id");
    let cli_evidence_b = admission_evidence(&store, &run_b).await;
    assert_eq!(cli_evidence_b.target, TARGET);
    assert_eq!(cli_evidence_b.schema_id, schema_id);
    assert_eq!(cli_evidence_b.digest, digest_b);
    assert_ne!(run_a, run_b);
    assert_ne!(
        evidence_a.certified_spec_hash,
        cli_evidence_b.certified_spec_hash,
        "replacing the current configuration must change the certified spec under one invocation key"
    );

    let attached_b = json_output(run_cli(
        &scoped_url,
        &start_args(TARGET, STABLE_INVOCATION_KEY, &runtime_config_path),
    ));
    assert_eq!(attached_b["data"]["outcome"], "attached");
    assert_eq!(attached_b["data"]["run"]["run_id"], run_b.as_str());

    let rest = mfm_rest_api::make_app(mfm_rest_api::AppState {
        role: mfm_rest_api::RestProcessRole::Live,
        fact_index: mfm_app::production_fact_index_read_provider(store.clone()),
        configured_store: Some(store.clone()),
        store: store.clone(),
        runtime_config_path: Some(runtime_config_path.clone()),
    });
    let rest_response = rest
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/runs/start")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "entry_point": "mfm.portfolio/snapshot@1",
                        "target": TARGET,
                        "invocation_key": STABLE_INVOCATION_KEY,
                    })
                    .to_string(),
                ))
                .expect("REST request"),
        )
        .await
        .expect("REST response");
    assert_eq!(rest_response.status(), StatusCode::OK);
    let rest_response = response_json(rest_response).await;
    assert_eq!(rest_response["data"]["outcome"], "attached");
    let rest_run: mfm_ids::RunId = rest_response["data"]["run"]["run_id"]
        .as_str()
        .expect("REST run id")
        .parse()
        .expect("typed REST run id");
    assert_eq!(rest_run, run_b);
    assert_eq!(admission_evidence(&store, &rest_run).await, cli_evidence_b);

    let duplicate_path = directory.path().join("duplicate-targets.toml");
    std::fs::write(&duplicate_path, format!("{SETUP_FIXTURE}\n{SETUP_FIXTURE}"))
        .expect("write duplicate targets");
    let duplicate = json_error(run_cli(&scoped_url, &import_args(&duplicate_path)));
    assert_eq!(duplicate["error"]["code"], "SetupDuplicateTarget");

    let setup_c = SETUP_FIXTURE.replacen(
        "portfolio_id = \"acme/primary\"",
        "portfolio_id = \"acme/resume\"",
        1,
    );
    let setup_c_path = directory.path().join("organization-c.toml");
    std::fs::write(&setup_c_path, setup_c).expect("write setup C");
    let imported_c = json_output(run_cli(&scoped_url, &import_args(&setup_c_path)));
    let config_c = imported_c["data"]["configs"]
        .as_array()
        .and_then(|configs| configs.first())
        .expect("import C config");
    assert_eq!(config_c["target"], UNFINISHED_TARGET);
    assert_eq!(config_c["status"], "created");
    let unfinished_run = admit_configured_target_without_driving(
        &store,
        UNFINISHED_TARGET,
        &runtime_config_path,
        "postgres-cli-portfolio-resume",
    )
    .await;
    let admitted_stream = store
        .load_run_stream(&unfinished_run)
        .await
        .expect("admitted unfinished run stream");
    assert_eq!(admitted_stream.len(), 1);
    assert!(matches!(
        admitted_stream[0].payload(),
        KernelEventPayload::RunAdmitted(_)
    ));

    let configured_pool = PgPool::connect(&scoped_url)
        .await
        .expect("connect for current configuration removal");
    sqlx::query("DELETE FROM configured_values")
        .execute(&configured_pool)
        .await
        .expect("remove mutable current configuration");
    configured_pool.close().await;
    let resumed_unfinished = json_output(run_cli(
        &scoped_url,
        &[
            "--output-format",
            "json",
            "run",
            "resume",
            unfinished_run.as_str(),
            "--runtime-config",
            runtime_config_path.to_str().expect("runtime config path"),
        ],
    ));
    assert_eq!(
        resumed_unfinished["data"]["run_id"],
        unfinished_run.as_str()
    );
    assert_eq!(resumed_unfinished["data"]["run_mode"], "completed");
    let resumed_stream = store
        .load_run_stream(&unfinished_run)
        .await
        .expect("resumed unfinished run stream");
    assert!(resumed_stream.len() > 1);
    assert!(resumed_stream
        .iter()
        .any(|event| matches!(event.payload(), KernelEventPayload::RunCompleted(_))));

    std::fs::remove_file(&runtime_config_path).expect("remove live runtime config");
    let replay = json_output(run_cli(
        &scoped_url,
        &[
            "--output-format",
            "json",
            "run",
            "replay",
            unfinished_run.as_str(),
        ],
    ));
    assert_eq!(replay["data"]["run_mode"], "completed");

    let listed_after_deletion = json_output(run_cli(
        &scoped_url,
        &["--output-format", "json", "setup", "list"],
    ));
    assert_eq!(listed_after_deletion["data"]["targets"], json!([]));
    drop_postgres_schema(&database_url, &schema).await;
}
