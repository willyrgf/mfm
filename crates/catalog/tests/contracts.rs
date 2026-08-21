use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_catalog::{
    CatalogEntry, CatalogError, CatalogPutResult, ConfigCatalog, ConfigDigest, ConfigName,
    MemoryCatalog, RunPageLimit, MAX_CONFIG_DOCUMENT_BYTES, MAX_CONFIG_ENTRIES, MAX_RUN_PAGE_ITEMS,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};
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

#[tokio::test]
async fn memory_catalog_put_is_atomic_and_replaces_complete_content() {
    let catalog = MemoryCatalog::new();
    let first = entry(1);
    assert_eq!(
        catalog.put_config(&first).await.expect("insert"),
        CatalogPutResult::Inserted
    );
    assert_eq!(
        catalog.put_config(&first).await.expect("retry"),
        CatalogPutResult::Unchanged
    );

    let second_bytes = PlainCanonicalJsonBytes::from_json_str(r#"{"value":2}"#).expect("canonical");
    let second = CatalogEntry::new(
        first.name().clone(),
        ConfigDigest::new(second_bytes.content_digest()).expect("digest"),
        second_bytes.to_vec(),
    )
    .expect("entry");
    assert_eq!(
        catalog.put_config(&second).await.expect("update"),
        CatalogPutResult::Updated
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
            catalog.put_config(&entry).await
        });
    }
    let mut inserted = 0;
    let mut capacity = 0;
    while let Some(result) = tasks.join_next().await {
        match result.expect("task") {
            Ok(CatalogPutResult::Inserted) => inserted += 1,
            Err(CatalogError::Capacity) => capacity += 1,
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
    assert_eq!(inserted, MAX_CONFIG_ENTRIES);
    assert_eq!(capacity, callers - MAX_CONFIG_ENTRIES);

    let canonical = PlainCanonicalJsonBytes::from_json_str(r#"{"value":"replacement"}"#)
        .expect("replacement canonical");
    let retained_name = catalog
        .list_configs()
        .await
        .expect("catalog entries")
        .items()[0]
        .name()
        .clone();
    let replacement = CatalogEntry::new(
        retained_name,
        ConfigDigest::new(canonical.content_digest()).expect("replacement digest"),
        canonical.to_vec(),
    )
    .expect("replacement entry");
    assert_eq!(
        catalog
            .put_config(&replacement)
            .await
            .expect("replace at capacity"),
        CatalogPutResult::Updated
    );
}

#[tokio::test]
async fn memory_catalog_lists_every_entry_in_bytewise_name_order() {
    let catalog = MemoryCatalog::new();
    for index in (0..5).rev() {
        assert_eq!(
            catalog.put_config(&entry(index)).await.expect("insert"),
            CatalogPutResult::Inserted
        );
    }
    let entries = catalog.list_configs().await.expect("entries");
    assert_eq!(
        entries
            .items()
            .iter()
            .map(|item| item.name().as_str())
            .collect::<Vec<_>>(),
        [
            "config-000",
            "config-001",
            "config-002",
            "config-003",
            "config-004"
        ]
    );
}

#[test]
fn run_page_limits_are_bounded() {
    assert!(RunPageLimit::new(0).is_err());
    assert!(RunPageLimit::new(MAX_RUN_PAGE_ITEMS + 1).is_err());
}
