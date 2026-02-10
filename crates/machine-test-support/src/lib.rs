//! Test harnesses for `mfm-machine` trait contracts.
//!
//! This crate is intentionally tiny and depends only on `mfm-machine` so storage backends can
//! share correctness tests without creating dependency cycles.

use mfm_machine::errors::StorageError;
use mfm_machine::hashing::artifact_id_for_bytes;
use mfm_machine::ids::ArtifactId;
use mfm_machine::stores::{ArtifactKind, ArtifactStore};

pub async fn artifact_store_contract_tests(store: &dyn ArtifactStore) {
    put_get_roundtrip(store).await;
    content_addressed(store).await;
    exists_and_not_found(store).await;
}

async fn put_get_roundtrip(store: &dyn ArtifactStore) {
    let bytes = b"hello artifact".to_vec();
    let id = store
        .put(ArtifactKind::Other("test".to_string()), bytes.clone())
        .await
        .expect("put must succeed");

    assert_eq!(id, artifact_id_for_bytes(&bytes));

    let got = store.get(&id).await.expect("get must succeed");
    assert_eq!(got, bytes);
}

async fn content_addressed(store: &dyn ArtifactStore) {
    let bytes = b"same bytes".to_vec();

    let id1 = store
        .put(ArtifactKind::Other("k1".to_string()), bytes.clone())
        .await
        .expect("put must succeed");

    let id2 = store
        .put(ArtifactKind::Other("k2".to_string()), bytes.clone())
        .await
        .expect("put must succeed");

    assert_eq!(id1, id2);
    assert_eq!(id1, artifact_id_for_bytes(&bytes));
}

async fn exists_and_not_found(store: &dyn ArtifactStore) {
    let missing = ArtifactId("0".repeat(64));
    assert!(!store.exists(&missing).await.expect("exists must succeed"));

    match store.get(&missing).await {
        Err(StorageError::NotFound(_)) => {}
        other => panic!("expected NotFound for missing artifact, got: {other:?}"),
    }
}
