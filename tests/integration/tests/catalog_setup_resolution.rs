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
use mfm_state_evm_contracts::{ImportConfiguredSpec, ImportDeployedSpec};
use mfm_storage_postgres::PostgresStore;
use mfm_values::{MfmConfig, ValidatedConfig};
use serde::de::DeserializeOwned;
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

async fn export_json(
    store: &PostgresStore,
    values: &BTreeMap<String, mfm_app::CatalogValueIdentity>,
    name: &str,
) -> Value {
    let identity = identity(values, name);
    serde_json::from_slice(
        &mfm_app::export_catalog_value(store, identity)
            .await
            .expect("catalog value bytes"),
    )
    .expect("catalog value JSON")
}

async fn insert_json(
    database_url: &str,
    name: &str,
    schema_id: &str,
    value: &Value,
) -> ContentDigest {
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(value).expect("catalog value JSON"),
    )
    .expect("canonical catalog value JSON");
    let digest = canonical.content_digest();
    insert_raw(
        database_url,
        name,
        schema_id,
        digest.as_str(),
        canonical.as_bytes(),
    )
    .await;
    digest
}

async fn insert_validated_json<T>(
    database_url: &str,
    name: &str,
    schema_id: &str,
    value: Value,
) -> ContentDigest
where
    T: DeserializeOwned + MfmConfig,
{
    let config: T = serde_json::from_value(value).expect("typed catalog value");
    let canonical = ValidatedConfig::new(config)
        .expect("validated catalog value")
        .canonical_json()
        .expect("canonical typed catalog value");
    let digest = canonical.content_digest();
    insert_raw(
        database_url,
        name,
        schema_id,
        digest.as_str(),
        canonical.as_bytes(),
    )
    .await;
    digest
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

fn request_for_entry_point(
    values: &BTreeMap<String, mfm_app::CatalogValueIdentity>,
    entry_point: &str,
) -> Value {
    match entry_point {
        "mfm.portfolio/portfolio_snapshot@1" => {
            json!({"portfolio": reference(values, "acme/dual-mainnet")})
        }
        "mfm.bitcoin/btc_address_balance@1" => {
            json!({"config": reference(values, "acme/bitcoin-balance")})
        }
        "mfm.evm/evm_native_balance@1" => {
            json!({"config": reference(values, "acme/ethereum-balance")})
        }
        "mfm.evm.contract/deploy@1" => json!({
            "context": reference(values, "acme/contract-context"),
            "deploy_action": reference(values, "acme/deploy-action"),
        }),
        "mfm.evm.contract/configure@1" => json!({
            "context": reference(values, "acme/contract-context"),
            "import_deployed": reference(values, "acme/import-deployed"),
            "configure_action": reference(values, "acme/configure-action"),
        }),
        "mfm.evm.contract/validate@1" => json!({
            "context": reference(values, "acme/contract-context"),
            "import_configured": reference(values, "acme/import-configured"),
            "validate_action": reference(values, "acme/validate-action"),
        }),
        "mfm.evm.contract/lifecycle@1" => json!({
            "context": reference(values, "acme/contract-context"),
            "deploy_action": reference(values, "acme/deploy-action"),
            "configure_action": reference(values, "acme/configure-action"),
            "validate_action": reference(values, "acme/validate-action"),
        }),
        "mfm.portfolio/collect_then_report@1" => json!({
            "portfolio": reference(values, "acme/dual-mainnet"),
            "bitcoin_policy": {"coverage": "configured_only", "max_source_reads": 1},
            "evm_policy": {"coverage": "configured_only", "decimals": 18, "max_source_reads": 1},
        }),
        _ => panic!("unknown test entry point {entry_point}"),
    }
}

fn first_reference_field(entry_point: &str) -> &'static str {
    match entry_point {
        "mfm.portfolio/portfolio_snapshot@1" | "mfm.portfolio/collect_then_report@1" => "portfolio",
        "mfm.bitcoin/btc_address_balance@1" | "mfm.evm/evm_native_balance@1" => "config",
        "mfm.evm.contract/deploy@1"
        | "mfm.evm.contract/configure@1"
        | "mfm.evm.contract/validate@1"
        | "mfm.evm.contract/lifecycle@1" => "context",
        _ => panic!("unknown test entry point {entry_point}"),
    }
}

fn replace_first_reference(request: &mut Value, entry_point: &str, reference: Value) {
    let field = first_reference_field(entry_point);
    request
        .get_mut(field)
        .expect("entry-point first reference")
        .clone_from(&reference);
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
async fn every_exact_entry_point_rejects_catalog_failures_before_admission() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_entry_failures");
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
    let cases = [
        ("mfm.portfolio/portfolio_snapshot@1", "acme/dual-mainnet"),
        ("mfm.bitcoin/btc_address_balance@1", "acme/bitcoin-balance"),
        ("mfm.evm/evm_native_balance@1", "acme/ethereum-balance"),
        ("mfm.evm.contract/deploy@1", "acme/contract-context"),
        ("mfm.evm.contract/configure@1", "acme/contract-context"),
        ("mfm.evm.contract/validate@1", "acme/contract-context"),
        ("mfm.evm.contract/lifecycle@1", "acme/contract-context"),
        ("mfm.portfolio/collect_then_report@1", "acme/dual-mainnet"),
    ];

    for (index, (entry_point, source_name)) in cases.into_iter().enumerate() {
        let base = request_for_entry_point(&values, entry_point);

        let mut missing = base.clone();
        replace_first_reference(
            &mut missing,
            entry_point,
            json!({
                "name": format!("acme/missing-{index}"),
                "digest": identity(&values, source_name).digest.as_str(),
            }),
        );
        assert_eq!(
            prepare(&store, entry_point, &missing)
                .await
                .expect_err("missing catalog value")
                .code,
            "CatalogValueNotFound",
            "missing reference for {entry_point}"
        );

        let mut wrong_digest = base.clone();
        replace_first_reference(
            &mut wrong_digest,
            entry_point,
            json!({
                "name": source_name,
                "digest": format!("content:sha256-jcs-v1:{}", "ab".repeat(32)),
            }),
        );
        assert_eq!(
            prepare(&store, entry_point, &wrong_digest)
                .await
                .expect_err("wrong catalog digest")
                .code,
            "CatalogValueNotFound",
            "wrong digest for {entry_point}"
        );

        let wrong_type_name = if source_name == "acme/dual-mainnet" {
            "acme/bitcoin-balance"
        } else {
            "acme/dual-mainnet"
        };
        let wrong_type_identity = identity(&values, wrong_type_name);
        let wrong_type_bytes = mfm_app::export_catalog_value(&store, wrong_type_identity)
            .await
            .expect("wrong-type catalog bytes");
        let wrong_type_row_name = format!("acme/wrong-type-{index}");
        insert_raw(
            &scoped_url,
            &wrong_type_row_name,
            identity(&values, source_name).schema_id.as_str(),
            wrong_type_identity.digest.as_str(),
            &wrong_type_bytes,
        )
        .await;
        let mut wrong_type = base.clone();
        replace_first_reference(
            &mut wrong_type,
            entry_point,
            json!({
                "name": wrong_type_row_name,
                "digest": wrong_type_identity.digest.as_str(),
            }),
        );
        assert_eq!(
            prepare(&store, entry_point, &wrong_type)
                .await
                .expect_err("wrong catalog type")
                .code,
            "CatalogValueTypeInvalid",
            "wrong type for {entry_point}"
        );

        let source = identity(&values, source_name);
        let corrupt_name = format!("acme/corrupt-{index}");
        insert_raw(
            &scoped_url,
            &corrupt_name,
            source.schema_id.as_str(),
            source.digest.as_str(),
            b"not-canonical-json",
        )
        .await;
        let mut corrupt = base;
        replace_first_reference(
            &mut corrupt,
            entry_point,
            json!({"name": corrupt_name, "digest": source.digest.as_str()}),
        );
        let corruption = prepare(&store, entry_point, &corrupt)
            .await
            .expect_err("corrupt catalog row");
        assert_eq!(corruption.code, "RunStoreCorruption");
        assert!(!corruption.message.contains("not-canonical-json"));
    }

    let pool = sqlx::PgPool::connect(&scoped_url)
        .await
        .expect("connect admission assertion pool");
    let run_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_events")
        .fetch_one(&pool)
        .await
        .expect("count run events");
    assert_eq!(run_events, 0, "preparation failures must precede admission");
    pool.close().await;
    drop_postgres_schema(&database_url, &schema).await;
}

#[tokio::test]
async fn resolver_validates_every_concrete_catalog_family() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_family_failures");
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
    let cases = [
        ("mfm.portfolio/portfolio_snapshot@1", "acme/dual-mainnet"),
        ("mfm.bitcoin/btc_address_balance@1", "acme/bitcoin-balance"),
        ("mfm.evm/evm_native_balance@1", "acme/ethereum-balance"),
        ("mfm.evm.contract/deploy@1", "acme/contract-context"),
        ("mfm.evm.contract/configure@1", "acme/contract-context"),
        ("mfm.evm.contract/validate@1", "acme/contract-context"),
        ("mfm.evm.contract/lifecycle@1", "acme/contract-context"),
        ("mfm.portfolio/collect_then_report@1", "acme/dual-mainnet"),
    ];

    for (index, (entry_point, source_name)) in cases.into_iter().enumerate() {
        let source = identity(&values, source_name);
        let base_request = request_for_entry_point(&values, entry_point);

        let shape_name = format!("acme/shape-{index}");
        let shape_digest = insert_json(
            &scoped_url,
            &shape_name,
            source.schema_id.as_str(),
            &json!({"wrong_shape": "secret-shape-sentinel"}),
        )
        .await;
        let mut wrong_shape = base_request.clone();
        replace_first_reference(
            &mut wrong_shape,
            entry_point,
            json!({"name": shape_name, "digest": shape_digest.as_str()}),
        );
        let shape_error = prepare(&store, entry_point, &wrong_shape)
            .await
            .expect_err("wrong catalog JSON shape");
        assert_eq!(shape_error.code, "CatalogValueTypeInvalid");
        assert!(!shape_error.message.contains("secret-shape-sentinel"));

        let mut unknown_value = export_json(&store, &values, source_name).await;
        unknown_value["unknown_typed_field"] = json!("secret-field-sentinel");
        let unknown_name = format!("acme/unknown-{index}");
        let unknown_digest = insert_json(
            &scoped_url,
            &unknown_name,
            source.schema_id.as_str(),
            &unknown_value,
        )
        .await;
        let mut unknown = base_request.clone();
        replace_first_reference(
            &mut unknown,
            entry_point,
            json!({"name": unknown_name, "digest": unknown_digest.as_str()}),
        );
        let unknown_error = prepare(&store, entry_point, &unknown)
            .await
            .expect_err("unknown typed catalog field");
        assert_eq!(unknown_error.code, "CatalogValueTypeInvalid");
        assert!(!unknown_error.message.contains("secret-field-sentinel"));

        let Some(semantic_value) = (match entry_point {
            "mfm.portfolio/portfolio_snapshot@1" | "mfm.portfolio/collect_then_report@1" => {
                let mut value = export_json(&store, &values, source_name).await;
                value["quote_codes"] = json!(["USD", "USD"]);
                Some(value)
            }
            "mfm.bitcoin/btc_address_balance@1" => {
                let mut value = export_json(&store, &values, source_name).await;
                value["addresses"] = json!([]);
                Some(value)
            }
            "mfm.evm/evm_native_balance@1" => {
                let mut value = export_json(&store, &values, source_name).await;
                value["accounts"] = json!([]);
                Some(value)
            }
            _ => None,
        }) else {
            continue;
        };
        let semantic_name = format!("acme/semantic-{index}");
        let semantic_digest = insert_json(
            &scoped_url,
            &semantic_name,
            source.schema_id.as_str(),
            &semantic_value,
        )
        .await;
        let mut semantic = base_request;
        replace_first_reference(
            &mut semantic,
            entry_point,
            json!({"name": semantic_name, "digest": semantic_digest.as_str()}),
        );
        let semantic_error = prepare(&store, entry_point, &semantic)
            .await
            .expect_err("semantic catalog value failure");
        assert_eq!(semantic_error.code, "CatalogValueValidationFailed");
        assert!(!semantic_error.message.contains("USD"));
    }

    drop_postgres_schema(&database_url, &schema).await;
}

#[tokio::test]
async fn application_preparation_rejects_relational_builder_failures_before_certification() {
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let schema = unique_postgres_schema("catalog_builder_failures");
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

    let mut invalid_deployed_import = export_json(&store, &values, "acme/import-deployed").await;
    invalid_deployed_import["adoption"]["evidence_policy"]["initial_event_assertions"] = json!([
        {
            "event": "Configured",
            "min_count": 1,
            "from_block": {"kind": "number", "number": 0}
        }
    ]);
    let deployed_import_name = "acme/invalid-builder-import-deployed";
    let deployed_import_digest = insert_validated_json::<ImportDeployedSpec>(
        &scoped_url,
        deployed_import_name,
        identity(&values, "acme/import-deployed").schema_id.as_str(),
        invalid_deployed_import,
    )
    .await;
    let mut configure_request = request_for_entry_point(&values, "mfm.evm.contract/configure@1");
    configure_request["import_deployed"] = json!({
        "name": deployed_import_name,
        "digest": deployed_import_digest.as_str(),
    });
    let configure_error = prepare(&store, "mfm.evm.contract/configure@1", &configure_request)
        .await
        .expect_err("invalid configure join");
    assert_eq!(configure_error.code, "EvmContractPlanFailed");
    assert_eq!(configure_error.message, "Entry-point planning failed");

    let mut invalid_configured_import =
        export_json(&store, &values, "acme/import-configured").await;
    invalid_configured_import["adoption"]["evidence_policy"]["initial_event_assertions"] = json!([
        {
            "event": "Validated",
            "min_count": 1,
            "to_block": {"kind": "number", "number": 0}
        }
    ]);
    let configured_import_name = "acme/invalid-builder-import-configured";
    let configured_import_digest = insert_validated_json::<ImportConfiguredSpec>(
        &scoped_url,
        configured_import_name,
        identity(&values, "acme/import-configured")
            .schema_id
            .as_str(),
        invalid_configured_import,
    )
    .await;
    let mut validate_request = request_for_entry_point(&values, "mfm.evm.contract/validate@1");
    validate_request["import_configured"] = json!({
        "name": configured_import_name,
        "digest": configured_import_digest.as_str(),
    });
    let validate_error = prepare(&store, "mfm.evm.contract/validate@1", &validate_request)
        .await
        .expect_err("invalid validate join");
    assert_eq!(validate_error.code, "EvmContractPlanFailed");
    assert_eq!(validate_error.message, "Entry-point planning failed");

    let mut empty_portfolio = export_json(&store, &values, "acme/dual-mainnet").await;
    empty_portfolio["wallets"] = json!([]);
    let empty_portfolio_name = "acme/invalid-builder-portfolio";
    let empty_portfolio_digest = insert_validated_json::<PortfolioConfig>(
        &scoped_url,
        empty_portfolio_name,
        identity(&values, "acme/dual-mainnet").schema_id.as_str(),
        empty_portfolio,
    )
    .await;
    let mut collect_request =
        request_for_entry_point(&values, "mfm.portfolio/collect_then_report@1");
    collect_request["portfolio"] = json!({
        "name": empty_portfolio_name,
        "digest": empty_portfolio_digest.as_str(),
    });
    let collect_error = prepare(
        &store,
        "mfm.portfolio/collect_then_report@1",
        &collect_request,
    )
    .await
    .expect_err("invalid collect/report relationship");
    assert_eq!(collect_error.code, "CollectThenReportConfigInvalid");
    assert_eq!(collect_error.message, "Entry-point planning failed");

    let pool = sqlx::PgPool::connect(&scoped_url)
        .await
        .expect("connect admission assertion pool");
    let run_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_events")
        .fetch_one(&pool)
        .await
        .expect("count run events");
    assert_eq!(run_events, 0, "builder failures must precede admission");
    pool.close().await;
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
