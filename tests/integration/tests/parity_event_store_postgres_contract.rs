#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use mfm_machine_test_support::stream_store_contract_tests;
use mfm_stream_store_postgres::PostgresStreamStore;

#[tokio::test]
async fn stream_store_postgres_contract() {
    let store = PostgresStreamStore::connect_env()
        .await
        .expect("postgres config");
    stream_store_contract_tests(&store).await;
}
