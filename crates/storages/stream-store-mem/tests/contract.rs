use mfm_machine_test_support::stream_store_contract_tests;
use mfm_stream_store_mem::MemStreamStore;

#[tokio::test]
async fn stream_store_mem_contract() {
    let store = MemStreamStore::new();
    stream_store_contract_tests(&store).await;
}
