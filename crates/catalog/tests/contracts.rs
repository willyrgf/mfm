use std::sync::Arc;

use mfm_canonical::{CanonicalBytes, PlainCanonicalJsonBytes};
use mfm_catalog::{
    CatalogDeleteResult, CatalogEntry, CatalogError, CatalogInsertResult, ConfigCatalog,
    ConfigCursor, ConfigDigest, ConfigName, MemoryCatalog, PageLimit, RunCursor,
    MAX_CONFIG_DOCUMENT_BYTES, MAX_CONFIG_ENTRIES, MAX_CURSOR_ENCODED_BYTES, MAX_PAGE_ITEMS,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes, RunId};
use tokio::sync::Barrier;

fn config_name(index: usize) -> ConfigName {
    ConfigName::new(format!("config-{index:03}")).expect("checked name")
}

fn entry(index: usize) -> CatalogEntry {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":{index}}}"#))
        .expect("canonical JSON");
    CatalogEntry::new(
        config_name(index),
        ConfigDigest::new(canonical.content_digest()).expect("JCS digest"),
        canonical.to_vec(),
    )
    .expect("entry")
}

#[test]
fn config_name_and_digest_grammars_are_exact() {
    for accepted in ["a", "0", "daily", "daily-main-2", &"a".repeat(64)] {
        assert_eq!(
            ConfigName::new(accepted).expect("accepted").as_str(),
            accepted
        );
    }
    for rejected in [
        "",
        "-daily",
        "daily-",
        "Daily",
        "daily_main",
        "daily/main",
        &"a".repeat(65),
    ] {
        assert!(ConfigName::new(rejected).is_err(), "accepted {rejected:?}");
    }

    let jcs = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([1; 32]),
    );
    let raw =
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([1; 32]));
    assert_eq!(
        ConfigDigest::new(jcs.clone()).expect("JCS").as_str(),
        jcs.as_str()
    );
    assert!(ConfigDigest::new(raw).is_err());
    assert_eq!(
        serde_json::from_str::<ConfigDigest>(
            &serde_json::to_string(&ConfigDigest::new(jcs).expect("digest")).expect("serialize")
        )
        .expect("deserialize")
        .content_digest()
        .algorithm(),
        DigestAlgorithm::Sha256JcsV1
    );
}

#[test]
fn custody_records_enforce_only_the_shared_mechanical_bound() {
    let digest = ConfigDigest::new(
        PlainCanonicalJsonBytes::from_json_str("{}")
            .expect("canonical")
            .content_digest(),
    )
    .expect("digest");
    assert!(CatalogEntry::new(config_name(0), digest.clone(), Vec::new()).is_err());
    assert!(CatalogEntry::new(
        config_name(0),
        digest.clone(),
        vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES + 1]
    )
    .is_err());
    let opaque = vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES];
    assert_eq!(
        CatalogEntry::new(config_name(0), digest, opaque)
            .expect("mechanically bounded")
            .canonical_bytes()
            .len(),
        MAX_CONFIG_DOCUMENT_BYTES
    );
}

#[test]
fn cursor_wires_are_exact_resource_specific_and_bounded() {
    let config = ConfigCursor::after_name(ConfigName::new("alpha").expect("name"));
    let decoded = CanonicalBytes::from_base64url_no_pad(config.as_str().to_owned())
        .expect("base64url")
        .into_bytes();
    assert_eq!(
        decoded,
        br#"{"after":"alpha","order":"name-asc","resource":"configs","v":1}"#
    );
    assert_eq!(ConfigCursor::parse(config.as_str()).expect("parse"), config);
    assert!(RunCursor::parse(config.as_str()).is_err());

    let run_id = RunId::from_digest(DigestBytes::from_array([9; 32]));
    let run = RunCursor::after_run(run_id.clone());
    assert_eq!(
        RunCursor::parse(run.as_str()).expect("parse").after(),
        &run_id
    );
    assert!(ConfigCursor::parse(run.as_str()).is_err());

    for decoded in [
        br#"{"after":"alpha","order":"name-asc","resource":"configs","v":2}"#.as_slice(),
        br#"{"after":"alpha","extra":0,"order":"name-asc","resource":"configs","v":1}"#,
        br#"{"after":"alpha","after":"beta","order":"name-asc","resource":"configs","v":1}"#,
        br#"{"resource":"configs","order":"name-asc","after":"alpha","v":1}"#,
    ] {
        let encoded = CanonicalBytes::new(decoded.to_vec());
        assert!(ConfigCursor::parse(encoded.encoded()).is_err());
    }
    assert!(ConfigCursor::parse("a".repeat(MAX_CURSOR_ENCODED_BYTES + 1)).is_err());
    assert!(ConfigCursor::parse("====").is_err());
}

#[tokio::test]
async fn memory_catalog_lifecycle_is_atomic_and_aba_safe() {
    let catalog = MemoryCatalog::new();
    let first = entry(1);
    assert_eq!(
        catalog.insert_config(&first).await.expect("insert"),
        CatalogInsertResult::Inserted
    );
    assert_eq!(
        catalog.insert_config(&first).await.expect("retry"),
        CatalogInsertResult::Unchanged
    );

    let second_bytes = PlainCanonicalJsonBytes::from_json_str(r#"{"value":2}"#).expect("canonical");
    let second = CatalogEntry::new(
        first.name().clone(),
        ConfigDigest::new(second_bytes.content_digest()).expect("digest"),
        second_bytes.to_vec(),
    )
    .expect("entry");
    assert_eq!(
        catalog.insert_config(&second).await.expect("conflict"),
        CatalogInsertResult::Conflict
    );
    assert_eq!(
        catalog
            .delete_config(first.name(), second.digest())
            .await
            .expect("mismatch"),
        CatalogDeleteResult::DigestMismatch
    );
    assert_eq!(
        catalog
            .delete_config(first.name(), first.digest())
            .await
            .expect("delete"),
        CatalogDeleteResult::Deleted
    );
    assert_eq!(
        catalog.insert_config(&second).await.expect("reuse"),
        CatalogInsertResult::Inserted
    );
    assert_eq!(
        catalog
            .delete_config(first.name(), first.digest())
            .await
            .expect("stale delete"),
        CatalogDeleteResult::DigestMismatch
    );
    let retained = catalog
        .load_config(first.name())
        .await
        .expect("load")
        .expect("present");
    assert_eq!(retained.digest(), second.digest());
    assert_eq!(retained.canonical_bytes(), second.canonical_bytes());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_absent_inserts_cannot_exceed_capacity() {
    let callers = MAX_CONFIG_ENTRIES + 32;
    let catalog = Arc::new(MemoryCatalog::new());
    let barrier = Arc::new(Barrier::new(callers));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..callers {
        let catalog = Arc::clone(&catalog);
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            let entry = entry(index);
            barrier.wait().await;
            catalog.insert_config(&entry).await
        });
    }
    let mut inserted = 0;
    let mut capacity = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("task") {
            Ok(CatalogInsertResult::Inserted) => inserted += 1,
            Err(CatalogError::Capacity) => capacity += 1,
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
    assert_eq!(inserted, MAX_CONFIG_ENTRIES);
    assert_eq!(capacity, callers - MAX_CONFIG_ENTRIES);
}

#[tokio::test]
async fn memory_catalog_pages_in_bytewise_name_order() {
    let catalog = MemoryCatalog::new();
    for index in (0..5).rev() {
        assert_eq!(
            catalog.insert_config(&entry(index)).await.expect("insert"),
            CatalogInsertResult::Inserted
        );
    }
    let limit = PageLimit::new(2).expect("limit");
    let first = catalog.list_configs(None, limit).await.expect("first");
    assert_eq!(
        first
            .items()
            .iter()
            .map(|item| item.name().as_str())
            .collect::<Vec<_>>(),
        ["config-000", "config-001"]
    );
    let second = catalog
        .list_configs(first.next_cursor(), limit)
        .await
        .expect("second");
    assert_eq!(
        second
            .items()
            .iter()
            .map(|item| item.name().as_str())
            .collect::<Vec<_>>(),
        ["config-002", "config-003"]
    );
    let third = catalog
        .list_configs(second.next_cursor(), limit)
        .await
        .expect("third");
    assert_eq!(third.items()[0].name().as_str(), "config-004");
    assert!(third.next_cursor().is_none());
    assert!(PageLimit::new(0).is_err());
    assert!(PageLimit::new(MAX_PAGE_ITEMS + 1).is_err());
}
