#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema,
    schema_scoped_database_url, start_collectors_rpc_mock, unique_postgres_schema,
    write_collectors_runtime_config_for_test,
};
use serde_json::json;
use sqlx::{AssertSqlSafe, PgPool};

const SETUP_FIXTURE: &[u8] = include_bytes!("../../../examples/setup/organization.toml");
const ENTRY_POINT: &str = "mfm.portfolio/collect_then_report@1";

#[tokio::test]
async fn admitted_catalog_launch_remains_operable_after_catalog_removal() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_replay");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_url, 20, 250).await;
    let imported = mfm_app::import_setup_toml(&store, SETUP_FIXTURE)
        .await
        .expect("setup import");
    let values = imported
        .into_iter()
        .map(|value| (value.name.clone(), value))
        .collect::<BTreeMap<_, _>>();
    let portfolio = values.get("acme/dual-mainnet").expect("portfolio identity");
    let request = json!({
        "portfolio": {
            "name": portfolio.name,
            "digest": portfolio.digest.as_str(),
        },
        "bitcoin_policy": {"coverage": "configured_only", "max_source_reads": 1},
        "evm_policy": {
            "coverage": "configured_only",
            "decimals": 18,
            "max_source_reads": 1,
        },
    });
    let certification_registry = mfm_app::production_certification_registry().expect("cert");
    let prepared = mfm_app::prepare_entry_point_run_launch(
        &store,
        ENTRY_POINT,
        &request,
        &certification_registry,
        store.store_authority().store_scope_id().clone(),
        Some(mfm_app::InvocationKey::new("catalog-replay-independence").expect("invocation key")),
    )
    .await
    .expect("catalog-backed launch preparation");
    assert_eq!(
        prepared.evidence.entry_point.entry_point_id.as_str(),
        ENTRY_POINT
    );
    assert_eq!(prepared.evidence.entry_point.catalog_sources.len(), 1);
    let run_id = prepared.run_id.clone();
    let public_schema_id = prepared
        .certified_spec
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone();

    let rpc_url = start_collectors_rpc_mock().await;
    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path =
        write_collectors_runtime_config_for_test(runtime_config_dir.path(), &rpc_url);
    let fact_index = mfm_app::production_fact_index_read_provider(store.clone());
    let runners = mfm_app::production_runner_registry(
        Arc::new(store.clone()),
        fact_index,
        Some(runtime_config_path.as_path()),
    )
    .expect("production runners");
    let services = mfm_app::make_run_services(
        runners,
        store.clone(),
        store.clone(),
        certification_registry.clone(),
    );

    // Preparation is the only catalog-dependent phase. Renaming the table before launch makes
    // any accidental catalog lookup during admission or execution fail loudly.
    let pool = PgPool::connect(&scoped_url)
        .await
        .expect("connect catalog replay schema");
    sqlx::raw_sql(AssertSqlSafe(
        "ALTER TABLE catalog_values RENAME TO catalog_values_revoked",
    ))
    .execute(&pool)
    .await
    .expect("revoke catalog table");
    let report = services
        .launch_run_and_render(prepared)
        .await
        .expect("launch without catalog access");
    assert_eq!(report.outcome, mfm_app::RunLaunchOutcomeStatus::Admitted);
    assert_eq!(
        report.run.as_ref().expect("run response").run_mode,
        mfm_app::RunModeStatus::Completed
    );
    sqlx::raw_sql(AssertSqlSafe("DROP TABLE catalog_values_revoked"))
        .execute(&pool)
        .await
        .expect("delete catalog after admission");
    pool.close().await;

    let resumed = services
        .resume_stored_run(&run_id)
        .await
        .expect("resume without catalog");
    assert_eq!(resumed.run_mode, mfm_app::RunModeStatus::Completed);

    let read_services =
        mfm_app::make_run_read_services(store.clone(), store.clone(), certification_registry);
    assert_eq!(
        read_services
            .run_status(&run_id)
            .await
            .expect("status without catalog")
            .run_mode,
        mfm_app::RunModeStatus::Completed
    );
    assert!(!read_services
        .run_stream(&run_id)
        .await
        .expect("stream without catalog")
        .events
        .is_empty());
    let public_output = read_services
        .public_output(&run_id, &public_schema_id)
        .await
        .expect("public output without catalog");
    assert!(public_output.json.is_some());
    let replay = read_services
        .verify_replay_for_run(&run_id)
        .await
        .expect("replay without catalog");
    assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Completed);

    drop_postgres_schema(&database_url, &schema).await;
}
