#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};
use mfm_integration_tests::test_support::{
    connect_postgres_with_retry, create_postgres_schema, drop_postgres_schema,
    schema_scoped_database_url, unique_postgres_schema,
};
use mfm_op_btc_collectors::BtcAddressBalanceConfig;
use mfm_op_portfolio_tracker::PortfolioConfig;
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
        "INSERT INTO catalog_values (name, schema_id, digest, canonical_json) \
         VALUES ($1, $2, $3, $4)",
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
async fn setup_resolves_and_prepares_all_eight_exact_entry_points() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_launch");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_url, 20, 250).await;

    let imported = mfm_app::import_setup_toml(&store, SETUP_FIXTURE)
        .await
        .expect("setup import");
    assert_eq!(imported.len(), 9);
    let values = imported
        .into_iter()
        .map(|value| (value.name.clone(), value))
        .collect::<BTreeMap<_, _>>();

    let cases = [
        (
            "mfm.portfolio/portfolio_snapshot@1",
            json!({"portfolio": reference(&values, "acme/dual-mainnet")}),
            ["acme/dual-mainnet"].as_slice(),
        ),
        (
            "mfm.bitcoin/btc_address_balance@1",
            json!({"config": reference(&values, "acme/bitcoin-balance")}),
            ["acme/bitcoin-balance"].as_slice(),
        ),
        (
            "mfm.evm/evm_native_balance@1",
            json!({"config": reference(&values, "acme/ethereum-balance")}),
            ["acme/ethereum-balance"].as_slice(),
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
        (
            "mfm.portfolio/collect_then_report@1",
            json!({
                "portfolio": reference(&values, "acme/dual-mainnet"),
                "bitcoin_policy": {"coverage": "configured_only", "max_source_reads": 1},
                "evm_policy": {"coverage": "configured_only", "decimals": 18, "max_source_reads": 1},
            }),
            ["acme/dual-mainnet"].as_slice(),
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
        let actual_sources = launch
            .evidence
            .entry_point
            .catalog_sources
            .iter()
            .map(|source| source.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(actual_sources, expected_sources);
        assert!(!launch.evidence.config_artifacts.is_empty());
    }

    drop_postgres_schema(&database_url, &schema).await;
}

#[tokio::test]
async fn resolver_rejects_missing_wrong_type_semantic_and_corrupt_catalog_values_redacted() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_resolver");
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

    let missing = prepare(
        &store,
        "mfm.portfolio/portfolio_snapshot@1",
        &json!({
            "portfolio": {
                "name": "acme/missing",
                "digest": identity(&values, "acme/dual-mainnet").digest.as_str(),
            }
        }),
    )
    .await
    .expect_err("missing catalog row");
    assert_eq!(missing.code, "CatalogValueNotFound");

    let wrong_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([91; 32]),
    );
    let wrong_digest_error = prepare(
        &store,
        "mfm.portfolio/portfolio_snapshot@1",
        &json!({
            "portfolio": {"name": "acme/dual-mainnet", "digest": wrong_digest.as_str()}
        }),
    )
    .await
    .expect_err("wrong digest");
    assert_eq!(wrong_digest_error.code, "CatalogValueNotFound");

    let btc = identity(&values, "acme/bitcoin-balance");
    let btc_bytes = mfm_app::export_catalog_value(&store, btc)
        .await
        .expect("BTC bytes");
    let portfolio_schema = PortfolioConfig::schema_id().expect("portfolio schema");
    insert_raw(
        &scoped_url,
        "acme/wrong-type",
        portfolio_schema.as_str(),
        btc.digest.as_str(),
        &btc_bytes,
    )
    .await;
    let wrong_type = prepare(
        &store,
        "mfm.portfolio/portfolio_snapshot@1",
        &json!({"portfolio": {"name": "acme/wrong-type", "digest": btc.digest.as_str()}}),
    )
    .await
    .expect_err("wrong catalog type");
    assert_eq!(wrong_type.code, "CatalogValueTypeInvalid");

    let invalid_btc = PlainCanonicalJsonBytes::from_json_str(
        r#"{"addresses":[],"bitcoin_network":"main","coverage":"configured_only","max_source_reads":1,"network":"bitcoin-mainnet","semantic_source_identity":"public-bitcoin-core"}"#,
    )
    .expect("invalid BTC canonical bytes");
    let btc_schema = BtcAddressBalanceConfig::schema_id().expect("BTC schema");
    insert_raw(
        &scoped_url,
        "acme/invalid-btc",
        btc_schema.as_str(),
        invalid_btc.content_digest().as_str(),
        invalid_btc.as_bytes(),
    )
    .await;
    let semantic_error = prepare(
        &store,
        "mfm.bitcoin/btc_address_balance@1",
        &json!({
            "config": {"name": "acme/invalid-btc", "digest": invalid_btc.content_digest().as_str()}
        }),
    )
    .await
    .expect_err("semantic validation failure");
    assert_eq!(semantic_error.code, "CatalogValueValidationFailed");

    let portfolio = identity(&values, "acme/dual-mainnet");
    let portfolio_bytes = mfm_app::export_catalog_value(&store, portfolio)
        .await
        .expect("portfolio bytes");
    let noncanonical = String::from_utf8(portfolio_bytes.clone())
        .expect("portfolio JSON")
        .replace('{', "{ ")
        .into_bytes();
    insert_raw(
        &scoped_url,
        "acme/noncanonical",
        portfolio.schema_id.as_str(),
        portfolio.digest.as_str(),
        &noncanonical,
    )
    .await;
    let corruption = prepare(
        &store,
        "mfm.portfolio/portfolio_snapshot@1",
        &json!({"portfolio": {"name": "acme/noncanonical", "digest": portfolio.digest.as_str()}}),
    )
    .await
    .expect_err("noncanonical raw storage");
    assert_eq!(corruption.code, "RunStoreCorruption");
    assert!(!corruption.message.contains("portfolio_id"));

    drop_postgres_schema(&database_url, &schema).await;
}

#[tokio::test]
async fn setup_import_prepares_the_whole_document_before_any_sql_write() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_atomic");
    create_postgres_schema(&database_url, &schema).await;
    let scoped_url = schema_scoped_database_url(&database_url, &schema);
    let store = connect_postgres_with_retry(&scoped_url, 20, 250).await;

    let mut invalid = String::from_utf8(SETUP_FIXTURE.to_vec()).expect("fixture UTF-8");
    invalid.push_str(
        r#"

[[values]]
name = "acme/rejected"
kind = "btc_address_balance"

[values.value]
network = "bitcoin-mainnet"
bitcoin_network = "main"
semantic_source_identity = "public-bitcoin-core"
addresses = []
coverage = "configured_only"
max_source_reads = 1
"#,
    );
    let error = mfm_app::import_setup_toml(&store, invalid.as_bytes())
        .await
        .expect_err("invalid later value");
    assert_eq!(error.code, "SetupValueValidationFailed");
    assert!(store
        .list_catalog_values(None, 100)
        .await
        .expect("empty catalog list")
        .is_empty());

    drop_postgres_schema(&database_url, &schema).await;
}
