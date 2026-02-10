use mfm_artifact_store_fs::FsArtifactStore;
use mfm_machine_test_support::artifact_store_contract_tests;

#[tokio::test]
async fn artifact_store_fs_contract() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsArtifactStore::new(dir.path());

    artifact_store_contract_tests(&store).await;
}
