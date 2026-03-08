#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_machine_test_support::artifact_store_contract_tests;

#[tokio::test]
async fn artifact_store_s3_contract() {
    let store = S3ArtifactStore::from_env().expect("s3 config");
    store.ensure_bucket_exists().await.expect("bucket exists");

    artifact_store_contract_tests(&store).await;
}
