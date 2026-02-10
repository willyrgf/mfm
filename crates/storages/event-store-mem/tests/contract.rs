use mfm_event_store_mem::MemEventStore;
use mfm_machine_test_support::event_store_contract_tests;

#[tokio::test]
async fn event_store_mem_contract() {
    let store = MemEventStore::new();
    event_store_contract_tests(&store).await;
}
