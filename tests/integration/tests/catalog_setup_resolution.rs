#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;

use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema,
    schema_scoped_database_url, unique_postgres_schema,
};
use mfm_storage_postgres::PostgresStore;
use serde_json::{json, Value};

const SETUP_FIXTURE: &[u8] = include_bytes!("../../../examples/setup/organization.toml");

fn identity<'a>(
    values: &'a BTreeMap<String, mfm_app::CatalogValueIdentity>,
    name: &str,
) -> &'a mfm_app::CatalogValueIdentity {
    values.get(name).expect("catalog identity")
}

fn reference(values: &BTreeMap<String, mfm_app::CatalogValueIdentity>, name: &str) -> Value {
    json!({
        "name": name,
        "digest": identity(values, name).digest.as_str(),
    })
}

async fn prepare(
    store: &PostgresStore,
    entry_point: &str,
    request: &Value,
) -> Result<mfm_app::RunLaunchRequest, mfm_app::AppError> {
    let registry = mfm_app::production_certification_registry()?;
    mfm_app::prepare_entry_point_run_launch(
        store,
        entry_point,
        request,
        &registry,
        store.store_authority().store_scope_id().clone(),
        None,
    )
    .await
}

#[tokio::test]
async fn setup_resolves_and_prepares_all_current_public_entry_points() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_launch");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_url, 20, 250).await;

    let values = mfm_app::import_setup_toml(&store, SETUP_FIXTURE)
        .await
        .expect("setup import")
        .into_iter()
        .map(|value| (value.name.clone(), value))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(values.len(), 7);

    let cases = [
        (
            "mfm.evm.contract/deploy@1",
            json!({
                "context": reference(&values, "acme/contract-context"),
                "deploy_action": reference(&values, "acme/deploy-action"),
            }),
            ["acme/contract-context", "acme/deploy-action"].as_slice(),
        ),
        (
            "mfm.evm.contract/configure@1",
            json!({
                "context": reference(&values, "acme/contract-context"),
                "import_deployed": reference(&values, "acme/import-deployed"),
                "configure_action": reference(&values, "acme/configure-action"),
            }),
            [
                "acme/configure-action",
                "acme/contract-context",
                "acme/import-deployed",
            ]
            .as_slice(),
        ),
        (
            "mfm.evm.contract/validate@1",
            json!({
                "context": reference(&values, "acme/contract-context"),
                "import_configured": reference(&values, "acme/import-configured"),
                "validate_action": reference(&values, "acme/validate-action"),
            }),
            [
                "acme/contract-context",
                "acme/import-configured",
                "acme/validate-action",
            ]
            .as_slice(),
        ),
        (
            "mfm.evm.contract/lifecycle@1",
            json!({
                "context": reference(&values, "acme/contract-context"),
                "deploy_action": reference(&values, "acme/deploy-action"),
                "configure_action": reference(&values, "acme/configure-action"),
                "validate_action": reference(&values, "acme/validate-action"),
            }),
            [
                "acme/configure-action",
                "acme/contract-context",
                "acme/deploy-action",
                "acme/validate-action",
            ]
            .as_slice(),
        ),
    ];

    for (entry_point, request, expected_sources) in cases {
        let launch = prepare(&store, entry_point, &request)
            .await
            .unwrap_or_else(|error| panic!("{entry_point} preparation failed: {error}"));
        assert_eq!(
            launch.evidence.entry_point.entry_point_id.as_str(),
            entry_point
        );
        assert_eq!(
            launch
                .evidence
                .entry_point
                .catalog_sources
                .iter()
                .map(|source| source.name.as_str())
                .collect::<Vec<_>>(),
            expected_sources
        );
        assert!(!launch.evidence.config_artifacts.is_empty());
    }

    drop_postgres_schema(&database_url, &schema).await;
}
