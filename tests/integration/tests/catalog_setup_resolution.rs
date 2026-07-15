#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema,
    schema_scoped_database_url, unique_postgres_schema,
};
use mfm_op_btc_collectors::BtcAddressBalanceConfig;
use mfm_storage_postgres::PostgresStore;
use mfm_values::MfmConfig;
use serde_json::{json, Value};
use sqlx::PgPool;

const SETUP_FIXTURE: &[u8] = include_bytes!("../../../examples/setup/organization.toml");

async fn insert_raw(
    database_url: &str,
    name: &str,
    schema_id: &str,
    digest: &str,
    canonical_json: &[u8],
) {
    let pool = PgPool::connect(database_url)
        .await
        .expect("connect raw catalog pool");
    sqlx::query(
        "INSERT INTO catalog_values (name, schema_id, digest, canonical_json) VALUES ($1, $2, $3, $4)",
    )
    .bind(name)
    .bind(schema_id)
    .bind(digest)
    .bind(canonical_json)
    .execute(&pool)
    .await
    .expect("insert raw catalog fixture");
    pool.close().await;
}

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
    assert_eq!(values.len(), 8);

    let cases = [
        (
            "mfm.bitcoin/btc_address_balance@1",
            json!({"config": reference(&values, "acme/bitcoin-balance")}),
            ["acme/bitcoin-balance"].as_slice(),
        ),
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

#[tokio::test]
async fn btc_catalog_resolution_rejects_missing_invalid_and_corrupt_rows_before_admission() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_btc");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_url, 20, 250).await;
    let values = mfm_app::import_setup_toml(&store, SETUP_FIXTURE)
        .await
        .expect("setup import")
        .into_iter()
        .map(|value| (value.name.clone(), value))
        .collect::<BTreeMap<_, _>>();
    let btc = identity(&values, "acme/bitcoin-balance");

    let missing = prepare(
        &store,
        "mfm.bitcoin/btc_address_balance@1",
        &json!({"config": {"name": "acme/missing", "digest": btc.digest.as_str()}}),
    )
    .await
    .expect_err("missing catalog row");
    assert_eq!(missing.code, "CatalogValueNotFound");

    let invalid = PlainCanonicalJsonBytes::from_json_str(
        r#"{"addresses":[],"bitcoin_network":"main","coverage":"configured_only","max_source_reads":1,"network":"bitcoin-mainnet","semantic_source_identity":"public-bitcoin-core"}"#,
    )
    .expect("canonical invalid BTC config");
    let btc_schema = BtcAddressBalanceConfig::schema_id().expect("BTC schema");
    insert_raw(
        &scoped_url,
        "acme/invalid-btc",
        btc_schema.as_str(),
        invalid.content_digest().as_str(),
        invalid.as_bytes(),
    )
    .await;
    let invalid_error = prepare(
        &store,
        "mfm.bitcoin/btc_address_balance@1",
        &json!({
            "config": {
                "name": "acme/invalid-btc",
                "digest": invalid.content_digest().as_str(),
            }
        }),
    )
    .await
    .expect_err("semantically invalid catalog row");
    assert_eq!(invalid_error.code, "CatalogValueValidationFailed");

    insert_raw(
        &scoped_url,
        "acme/corrupt-btc",
        btc.schema_id.as_str(),
        btc.digest.as_str(),
        b"not-canonical-json",
    )
    .await;
    let corrupt = prepare(
        &store,
        "mfm.bitcoin/btc_address_balance@1",
        &json!({
            "config": {
                "name": "acme/corrupt-btc",
                "digest": btc.digest.as_str(),
            }
        }),
    )
    .await
    .expect_err("corrupt catalog row");
    assert_eq!(corrupt.code, "RunStoreCorruption");
    assert!(!corrupt.message.contains("not-canonical-json"));

    let pool = PgPool::connect(&scoped_url)
        .await
        .expect("connect admission assertion pool");
    let run_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_events")
        .fetch_one(&pool)
        .await
        .expect("count run events");
    assert_eq!(run_events, 0, "catalog failures must precede admission");
    pool.close().await;
    drop_postgres_schema(&database_url, &schema).await;
}
