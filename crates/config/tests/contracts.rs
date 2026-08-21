use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_config::{
    ConfigDigest, ConfigImportResult, ConfigName, ConfigRepository, ConfigRepositoryError,
    ConfigRevision, ConfigRevisions, MemoryConfigRepository, RetainedConfigRevision,
    MAX_CONFIG_DOCUMENT_BYTES,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};
use tokio::sync::Barrier;

fn name(value: &str) -> ConfigName {
    ConfigName::new(value).expect("checked name")
}

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
fn revisions_enforce_only_the_shared_document_bound() {
    let digest = ConfigDigest::new(
        PlainCanonicalJsonBytes::from_json_str("{}")
            .expect("canonical")
            .content_digest(),
    )
    .expect("digest");
    assert!(ConfigRevision::new(name("config-0"), digest.clone(), Vec::new()).is_err());
    assert!(ConfigRevision::new(
        name("config-0"),
        digest.clone(),
        vec![b'x'; MAX_CONFIG_DOCUMENT_BYTES + 1]
    )
    .is_err());
    assert_eq!(
        ConfigRevision::new(
            name("config-0"),
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
async fn memory_repository_retains_history_and_repoints_current() {
    let repository = MemoryConfigRepository::new();
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
        repository.import_config(&second).await.expect("update"),
        ConfigImportResult::Updated
    );
    assert_eq!(
        repository
            .load_config(first.name(), Some(first.digest()))
            .await
            .expect("load exact")
            .expect("first retained")
            .canonical_bytes(),
        first.canonical_bytes()
    );
    assert_eq!(
        repository.import_config(&first).await.expect("reactivate"),
        ConfigImportResult::Updated
    );
    assert_eq!(
        repository
            .load_config(first.name(), None)
            .await
            .expect("load current")
            .expect("current")
            .digest(),
        first.digest()
    );
    let listed = repository.list_configs().await.expect("list");
    assert_eq!(listed.items().len(), 2);
    assert!(listed.items()[0].is_current());
    assert!(!listed.items()[1].is_current());
}

#[tokio::test]
async fn repository_rejects_identity_collision_and_invalid_list_markers() {
    let repository = MemoryConfigRepository::new();
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
            .load_config(first.name(), None)
            .await
            .expect("load")
            .expect("current")
            .canonical_bytes(),
        first.canonical_bytes()
    );

    assert!(matches!(
        ConfigRevisions::new(vec![RetainedConfigRevision::new(first.clone(), false)]),
        Err(ConfigRepositoryError::Corrupt)
    ));
    let second = revision("daily", 2);
    assert!(matches!(
        ConfigRevisions::new(vec![
            RetainedConfigRevision::new(first, true),
            RetainedConfigRevision::new(second, true),
        ]),
        Err(ConfigRepositoryError::Corrupt)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_imports_retain_every_revision_with_one_current() {
    let repository = Arc::new(MemoryConfigRepository::new());
    let callers = 300;
    let barrier = Arc::new(Barrier::new(callers));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..callers {
        let repository = Arc::clone(&repository);
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            let revision = revision("daily", index);
            barrier.wait().await;
            repository.import_config(&revision).await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.expect("task").expect("import");
    }
    let listed = repository.list_configs().await.expect("list");
    assert_eq!(listed.items().len(), callers);
    assert_eq!(
        listed
            .items()
            .iter()
            .filter(|item| item.is_current())
            .count(),
        1
    );
}
