use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, SchemaId, StableAuthorKey};
use sqlx::Row;

use super::support::{drop_schema, test_store};
use crate::{
    ConfiguredValuePublicationStatus, ConfiguredValueRow, PostgresStore, MAX_CONFIGURED_VALUE_BYTES,
};

fn schema_id(byte: u8) -> SchemaId {
    SchemaId::new(
        "mfm.storage.configured-value.test",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
    .expect("schema id")
}

fn target(value: &str) -> StableAuthorKey {
    StableAuthorKey::new(value).expect("stable target")
}

fn row(target_value: &str, schema: SchemaId, value: u64) -> ConfiguredValueRow {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":{value}}}"#))
        .expect("canonical value");
    ConfiguredValueRow::new(
        target(target_value),
        schema,
        canonical.content_digest(),
        canonical.as_bytes().to_vec(),
    )
    .expect("configured row")
}

async fn insert_raw(
    store: &PostgresStore,
    target: &str,
    schema_id: &str,
    digest: &str,
    canonical_json: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO catalog_values (target, schema_id, digest, canonical_json) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(target)
    .bind(schema_id)
    .bind(digest)
    .bind(canonical_json)
    .execute(&store.pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn configured_values_publish_atomically_by_target_and_report_each_outcome() {
    let (store, schema) = test_store().await;
    let first = row("acme/first", schema_id(1), 1);
    let second = row("acme/second", schema_id(2), 2);

    let created = store
        .publish_configured_values(&[second.clone(), first.clone()])
        .await
        .expect("initial publication");
    assert_eq!(created.len(), 2);
    assert_eq!(created[0].row, first);
    assert_eq!(created[0].status, ConfiguredValuePublicationStatus::Created);
    assert_eq!(created[1].row, second);
    assert_eq!(created[1].status, ConfiguredValuePublicationStatus::Created);

    let unchanged = store
        .publish_configured_values(&[first.clone(), second.clone()])
        .await
        .expect("identical publication");
    assert!(unchanged
        .iter()
        .all(|publication| publication.status == ConfiguredValuePublicationStatus::Unchanged));

    let replacement = row("acme/first", schema_id(3), 3);
    let updated = store
        .publish_configured_values(std::slice::from_ref(&replacement))
        .await
        .expect("replacement publication");
    assert_eq!(updated[0].status, ConfiguredValuePublicationStatus::Updated);
    assert_eq!(
        store
            .load_configured_value(&target("acme/first"))
            .await
            .expect("load replacement"),
        Some(replacement)
    );
    assert_eq!(
        store.list_configured_targets().await.expect("list targets"),
        vec![target("acme/first"), target("acme/second")]
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn configured_values_use_target_only_for_selection() {
    let (store, schema) = test_store().await;
    let first = row("acme/current", schema_id(4), 1);
    let replacement = row("acme/current", schema_id(5), 2);
    store
        .publish_configured_values(&[first])
        .await
        .expect("initial publication");
    store
        .publish_configured_values(std::slice::from_ref(&replacement))
        .await
        .expect("replacement publication");

    let loaded = store
        .load_configured_value(&target("acme/current"))
        .await
        .expect("target lookup")
        .expect("current row");
    assert_eq!(loaded, replacement);

    let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM catalog_values")
        .fetch_one(&store.pool)
        .await
        .expect("count current values")
        .get("count");
    assert_eq!(count, 1);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn configured_value_invalid_or_duplicate_batch_leaves_every_target_unchanged() {
    let (store, schema) = test_store().await;
    let earlier = row("acme/earlier", schema_id(6), 1);
    let invalid = ConfiguredValueRow {
        target: target("acme/later"),
        schema_id: schema_id(7),
        digest: earlier.digest.clone(),
        canonical_json: b"not-canonical-json".to_vec(),
    };
    let error = store
        .publish_configured_values(&[earlier.clone(), invalid])
        .await
        .expect_err("invalid later row must reject the batch");
    assert!(!error.to_string().contains("not-canonical-json"));
    assert!(store
        .load_configured_value(&earlier.target)
        .await
        .expect("earlier lookup")
        .is_none());

    let duplicate = row("acme/duplicate", schema_id(8), 1);
    let error = store
        .publish_configured_values(&[duplicate.clone(), duplicate])
        .await
        .expect_err("duplicate target must reject the batch");
    assert!(!error.to_string().contains("acme/duplicate"));
    assert!(store
        .list_configured_targets()
        .await
        .expect("list empty targets")
        .is_empty());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn configured_value_load_rejects_noncanonical_bytes_digest_mismatches_and_invalid_targets() {
    let (store, schema) = test_store().await;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(r#"{"value":1}"#).expect("canonical bytes");
    let canonical_digest = canonical.content_digest();
    insert_raw(
        &store,
        "acme/noncanonical",
        schema_id(9).as_str(),
        canonical_digest.as_str(),
        br#"{"value": 1}"#,
    )
    .await
    .expect("insert noncanonical corruption");
    insert_raw(
        &store,
        "acme/mismatched",
        schema_id(10).as_str(),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([10; 32]),
        )
        .as_str(),
        canonical.as_bytes(),
    )
    .await
    .expect("insert digest corruption");
    insert_raw(
        &store,
        "mfm.reserved",
        schema_id(11).as_str(),
        canonical_digest.as_str(),
        canonical.as_bytes(),
    )
    .await
    .expect("insert invalid target corruption");

    for value in ["acme/noncanonical", "acme/mismatched"] {
        let error = store
            .load_configured_value(&target(value))
            .await
            .expect_err("corrupt configured value must fail");
        assert!(!error.to_string().contains("value"));
    }
    let list_error = store
        .list_configured_targets()
        .await
        .expect_err("invalid stored target must fail");
    assert!(!list_error.to_string().contains("mfm.reserved"));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn configured_value_database_bounds_reject_oversized_raw_rows() {
    let (store, schema) = test_store().await;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(r#"{"value":1}"#).expect("canonical bytes");
    assert!(insert_raw(
        &store,
        "acme/value",
        schema_id(12).as_str(),
        canonical.content_digest().as_str(),
        &vec![b'0'; MAX_CONFIGURED_VALUE_BYTES + 1],
    )
    .await
    .is_err());

    drop_schema(&store, &schema).await;
}

#[test]
fn configured_value_rows_reject_invalid_canonical_bytes_and_digest_mismatches() {
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([7; 32]),
    );
    assert!(ConfiguredValueRow::new(
        target("acme/value"),
        schema_id(13),
        digest.clone(),
        br#"{"value":1}"#.to_vec(),
    )
    .is_err());
    assert!(ConfiguredValueRow::new(
        target("acme/value"),
        schema_id(13),
        digest,
        b"not-json".to_vec(),
    )
    .is_err());
}
