use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, SchemaId};

use super::support::{drop_schema, test_store};
use crate::{CatalogValueKey, CatalogValueRow, PostgresStore, MAX_CATALOG_VALUE_BYTES};

fn schema_id(byte: u8) -> SchemaId {
    SchemaId::new(
        "mfm.storage.catalog.test",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
    .expect("schema id")
}

fn row(name: &str, schema: SchemaId, value: u64) -> CatalogValueRow {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":{value}}}"#))
        .expect("canonical value");
    CatalogValueRow::new(
        name,
        schema,
        canonical.content_digest(),
        canonical.as_bytes().to_vec(),
    )
    .expect("catalog row")
}

async fn insert_raw(
    store: &PostgresStore,
    name: &str,
    schema_id: &str,
    digest: &str,
    canonical_json: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO catalog_values (name, schema_id, digest, canonical_json) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(name)
    .bind(schema_id)
    .bind(digest)
    .bind(canonical_json)
    .execute(&store.pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn catalog_append_load_list_export_and_immutability_are_atomic() {
    let (store, schema) = test_store().await;
    let catalog_schema_id = schema_id(1);
    let first = row("acme/first", catalog_schema_id.clone(), 1);
    let second = row("acme/second", catalog_schema_id, 2);

    store
        .append_catalog_values(&[first.clone(), second.clone()])
        .await
        .expect("catalog append");
    store
        .append_catalog_values(&[first.clone(), second.clone()])
        .await
        .expect("same catalog append is idempotent");

    let loaded = store
        .load_catalog_value(&first.key)
        .await
        .expect("catalog load")
        .expect("first value");
    assert_eq!(loaded, first);
    assert_eq!(
        store
            .export_catalog_value(&first.key)
            .await
            .expect("catalog export")
            .expect("exported value")
            .canonical_json,
        first.canonical_json
    );

    let page = store
        .list_catalog_values(None, 1)
        .await
        .expect("catalog first page");
    assert_eq!(page, vec![first.key.clone()]);
    let next_page = store
        .list_catalog_values(Some(&page[0]), 100)
        .await
        .expect("catalog second page");
    assert_eq!(next_page, vec![second.key.clone()]);
    assert!(store.list_catalog_values(None, 101).await.is_err());

    let wrong_schema = CatalogValueKey::new(
        first.key.name.clone(),
        schema_id(2),
        first.key.digest.clone(),
    )
    .expect("wrong schema key");
    assert!(store
        .load_catalog_value(&wrong_schema)
        .await
        .expect("wrong schema lookup")
        .is_none());

    assert!(sqlx::query("UPDATE catalog_values SET canonical_json = $1")
        .bind(first.canonical_json.clone())
        .execute(&store.pool)
        .await
        .is_err());
    assert!(sqlx::query("DELETE FROM catalog_values")
        .execute(&store.pool)
        .await
        .is_err());
    assert!(sqlx::query("TRUNCATE catalog_values")
        .execute(&store.pool)
        .await
        .is_err());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn catalog_changed_content_under_one_name_creates_a_new_digest_row() {
    let (store, schema) = test_store().await;
    let schema_id = schema_id(3);
    let first = row("acme/revisioned", schema_id.clone(), 1);
    let second = row("acme/revisioned", schema_id, 2);
    assert_ne!(first.key.digest, second.key.digest);

    store
        .append_catalog_values(&[first.clone(), second.clone()])
        .await
        .expect("append changed catalog value");

    let mut identities = store
        .list_catalog_values(None, 100)
        .await
        .expect("list revisions");
    identities.sort();
    let mut expected = vec![first.key, second.key];
    expected.sort();
    assert_eq!(identities, expected);

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn catalog_invalid_later_batch_row_leaves_earlier_rows_uncommitted() {
    let (store, schema) = test_store().await;
    let earlier = row("acme/earlier", schema_id(4), 1);
    let invalid = CatalogValueRow {
        key: CatalogValueKey::new("acme/later", schema_id(5), earlier.key.digest.clone())
            .expect("invalid row key"),
        canonical_json: b"not-canonical-json".to_vec(),
    };
    let error = store
        .append_catalog_values(&[earlier.clone(), invalid])
        .await
        .expect_err("invalid later row must reject the batch");
    assert!(!error.to_string().contains("not-canonical-json"));
    assert!(store
        .load_catalog_value(&earlier.key)
        .await
        .expect("earlier lookup")
        .is_none());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn catalog_exact_key_conflict_with_different_bytes_does_not_commit_other_rows() {
    let (store, schema) = test_store().await;
    let existing = row("acme/existing", schema_id(6), 1);
    insert_raw(
        &store,
        &existing.key.name,
        existing.key.schema_id.as_str(),
        existing.key.digest.as_str(),
        br#"{"value": 2}"#,
    )
    .await
    .expect("insert conflicting stored bytes");

    let unrelated = row("acme/unrelated", schema_id(7), 2);
    let error = store
        .append_catalog_values(&[unrelated.clone(), existing])
        .await
        .expect_err("exact-key conflict must reject the batch");
    assert!(!error.to_string().contains("{\"value\": 2}"));
    assert!(store
        .load_catalog_value(&unrelated.key)
        .await
        .expect("unrelated lookup")
        .is_none());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn catalog_duplicate_exact_items_are_idempotent_within_one_batch() {
    let (store, schema) = test_store().await;
    let value = row("acme/duplicate", schema_id(8), 1);
    store
        .append_catalog_values(&[value.clone(), value.clone()])
        .await
        .expect("duplicate exact items are idempotent");
    assert_eq!(
        store
            .list_catalog_values(None, 100)
            .await
            .expect("list duplicate item")
            .len(),
        1
    );

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn catalog_keyset_pagination_covers_page_sizes_one_and_one_hundred() {
    let (store, schema) = test_store().await;
    let schema_id = schema_id(9);
    let values = (0..101_u64)
        .map(|value| row(&format!("acme/value-{value:03}"), schema_id.clone(), value))
        .collect::<Vec<_>>();
    store
        .append_catalog_values(&values)
        .await
        .expect("append pagination values");

    let first = store
        .list_catalog_values(None, 1)
        .await
        .expect("one-item page");
    assert_eq!(first.len(), 1);
    let second = store
        .list_catalog_values(Some(&first[0]), 100)
        .await
        .expect("one-hundred-item page");
    assert_eq!(second.len(), 100);
    let third = store
        .list_catalog_values(Some(second.last().expect("second page identity")), 100)
        .await
        .expect("final page");
    assert_eq!(third.len(), 0);

    let all = store
        .list_catalog_values(None, 100)
        .await
        .expect("first hundred identities");
    assert_eq!(all.len(), 100);
    assert!(store.list_catalog_values(None, 0).await.is_err());
    assert!(store.list_catalog_values(None, 101).await.is_err());

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn catalog_corrupt_rows_are_rejected_by_load_list_and_export() {
    let (store, schema) = test_store().await;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(r#"{"value":1}"#).expect("canonical bytes");
    let canonical_digest = canonical.content_digest();
    let noncanonical_key =
        CatalogValueKey::new("acme/noncanonical", schema_id(10), canonical_digest.clone())
            .expect("noncanonical key");
    insert_raw(
        &store,
        &noncanonical_key.name,
        noncanonical_key.schema_id.as_str(),
        noncanonical_key.digest.as_str(),
        br#"{"value": 1}"#,
    )
    .await
    .expect("insert noncanonical corruption");

    let mismatched_key = CatalogValueKey::new(
        "acme/mismatched",
        schema_id(11),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([11; 32]),
        ),
    )
    .expect("mismatched key");
    insert_raw(
        &store,
        &mismatched_key.name,
        mismatched_key.schema_id.as_str(),
        mismatched_key.digest.as_str(),
        canonical.as_bytes(),
    )
    .await
    .expect("insert digest corruption");

    let malformed_key = CatalogValueKey::new("acme/malformed", schema_id(12), canonical_digest)
        .expect("malformed key");
    insert_raw(
        &store,
        &malformed_key.name,
        malformed_key.schema_id.as_str(),
        malformed_key.digest.as_str(),
        &[0xff, 0x00, 0x01],
    )
    .await
    .expect("insert malformed corruption");

    for key in [&noncanonical_key, &mismatched_key, &malformed_key] {
        let load_error = store
            .load_catalog_value(key)
            .await
            .expect_err("corrupt load must fail");
        assert!(!load_error.to_string().contains("value"));
        let export_error = store
            .export_catalog_value(key)
            .await
            .expect_err("corrupt export must fail");
        assert!(!export_error.to_string().contains("value"));
    }
    let list_error = store
        .list_catalog_values(None, 100)
        .await
        .expect_err("corrupt list must fail");
    assert!(!list_error.to_string().contains("value"));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn catalog_database_bounds_reject_invalid_raw_rows() {
    let (store, schema) = test_store().await;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(r#"{"value":1}"#).expect("canonical bytes");
    let digest = canonical.content_digest();
    let valid_schema = schema_id(13).to_string();

    for (name, schema_id, digest, bytes) in [
        (
            "Acme/value",
            valid_schema.as_str(),
            digest.as_str(),
            canonical.as_bytes().to_vec(),
        ),
        (
            "acme/value",
            "schema:Bad:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000",
            digest.as_str(),
            canonical.as_bytes().to_vec(),
        ),
        (
            "acme/value",
            valid_schema.as_str(),
            "content:sha256-jcs-v1:BAD",
            canonical.as_bytes().to_vec(),
        ),
        (
            "acme/value",
            valid_schema.as_str(),
            digest.as_str(),
            vec![b'0'; MAX_CATALOG_VALUE_BYTES + 1],
        ),
    ] {
        assert!(insert_raw(&store, name, schema_id, digest, &bytes)
            .await
            .is_err());
    }

    drop_schema(&store, &schema).await;
}

#[test]
fn raw_catalog_rows_reject_invalid_canonical_bytes_and_digest_mismatches() {
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([7; 32]),
    );
    assert!(CatalogValueRow::new(
        "acme/value",
        schema_id(1),
        digest.clone(),
        br#"{"value":1}"#.to_vec(),
    )
    .is_err());
    assert!(
        CatalogValueRow::new("acme/value", schema_id(1), digest, b"not-json".to_vec(),).is_err()
    );
    assert!(CatalogValueRow::new(
        "Acme/value",
        schema_id(1),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([1; 32]),
        ),
        vec![b'0'; MAX_CATALOG_VALUE_BYTES + 1],
    )
    .is_err());
}
