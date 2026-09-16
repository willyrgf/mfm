use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_config::{
    ConfigDigest, ConfigImportResult, ConfigRepository, ConfigRepositoryError, ConfigRevision,
    MemoryConfigRepository, MAX_CONFIG_DOCUMENT_BYTES,
};
use mfm_ids::ConfigName;
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

fn revision(name: &str, value: usize) -> ConfigRevision {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":{value}}}"#))
        .expect("canonical JSON");
    ConfigRevision::new(
        ConfigName::new(name).expect("config name"),
        ConfigDigest::new(canonical.content_digest()).expect("JCS digest"),
        canonical.to_vec(),
    )
    .expect("revision")
}

#[test]
fn config_digest_and_document_bounds_are_exact() {
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
            &serde_json::to_string(&ConfigDigest::new(jcs.clone()).expect("digest"))
                .expect("serialize")
        )
        .expect("deserialize")
        .as_str(),
        jcs.as_str()
    );

    let digest = revision("bounds", 1).digest().clone();
    assert!(ConfigRevision::new(
        ConfigName::new("bounds").expect("name"),
        digest.clone(),
        vec![]
    )
    .is_err());
    assert!(ConfigRevision::new(
        ConfigName::new("bounds").expect("name"),
        digest.clone(),
        vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES + 1]
    )
    .is_err());
    assert_eq!(
        ConfigRevision::new(
            ConfigName::new("bounds").expect("name"),
            digest,
            vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES]
        )
        .expect("mechanically bounded")
        .canonical_bytes()
        .len(),
        MAX_CONFIG_DOCUMENT_BYTES
    );
}

#[tokio::test]
async fn memory_repository_retains_lists_loads_and_deletes_exact_revisions() {
    let repository = MemoryConfigRepository::default();
    let first = revision("daily", 1);
    let second = revision("daily", 2);
    assert_eq!(
        repository.import_config(&first).await.expect("create"),
        ConfigImportResult::Created
    );
    assert_eq!(
        repository.import_config(&first).await.expect("retry"),
        ConfigImportResult::Unchanged
    );
    assert_eq!(
        repository.import_config(&second).await.expect("create"),
        ConfigImportResult::Created
    );
    let listed = repository.list_configs().await.expect("list");
    assert_eq!(listed.len(), 2);
    assert!(listed
        .windows(2)
        .all(|pair| { (pair[0].name(), pair[0].digest()) < (pair[1].name(), pair[1].digest()) }));
    assert_eq!(
        repository
            .load_config(first.name(), first.digest())
            .await
            .expect("load")
            .expect("first retained")
            .canonical_bytes(),
        first.canonical_bytes()
    );

    repository
        .delete_config(first.name(), first.digest())
        .await
        .expect("delete");
    repository
        .delete_config(first.name(), first.digest())
        .await
        .expect("idempotent delete");
    assert!(repository
        .load_config(first.name(), first.digest())
        .await
        .expect("load deleted")
        .is_none());
    assert!(repository
        .load_config(second.name(), second.digest())
        .await
        .expect("load retained")
        .is_some());
}

#[tokio::test]
async fn repository_rejects_identity_collision_without_losing_the_revision() {
    let repository = MemoryConfigRepository::default();
    let first = revision("daily", 1);
    repository.import_config(&first).await.expect("create");
    let collision = ConfigRevision::new(
        first.name().clone(),
        first.digest().clone(),
        br#"{"value":"different"}"#.to_vec(),
    )
    .expect("bounded collision");
    assert_eq!(
        repository.import_config(&collision).await,
        Err(ConfigRepositoryError::Corrupt)
    );
    assert_eq!(
        repository
            .load_config(first.name(), first.digest())
            .await
            .expect("load")
            .expect("retained")
            .canonical_bytes(),
        first.canonical_bytes()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_identical_imports_retain_one_revision() {
    let repository = Arc::new(MemoryConfigRepository::default());
    let expected = revision("daily", 1);
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..300 {
        let repository = Arc::clone(&repository);
        let revision = expected.clone();
        tasks.spawn(async move { repository.import_config(&revision).await });
    }
    let mut created = 0;
    while let Some(result) = tasks.join_next().await {
        created +=
            usize::from(result.expect("task").expect("import") == ConfigImportResult::Created);
    }
    assert_eq!(created, 1);
    assert_eq!(repository.list_configs().await.expect("list").len(), 1);

    for value in 0..300 {
        assert_eq!(
            repository
                .import_config(&revision("unbounded", value))
                .await
                .expect("unbounded import"),
            ConfigImportResult::Created
        );
    }
    assert_eq!(repository.list_configs().await.expect("list").len(), 301);
}
