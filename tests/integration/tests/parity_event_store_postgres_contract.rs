#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use mfm_event_store_postgres::PostgresEventStore;
use mfm_machine_test_support::stream_store_contract_tests;

#[tokio::test]
async fn event_store_postgres_contract() {
    let store = PostgresEventStore::connect_env()
        .await
        .expect("postgres config");
    stream_store_contract_tests(&store).await;
}
