use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, SchemaId};

use super::support::{drop_schema, test_store};
use crate::{CatalogValueKey, CatalogValueRow};

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
}
