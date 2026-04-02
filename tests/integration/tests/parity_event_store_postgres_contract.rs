#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::time::Duration;

use mfm_machine::errors::StorageError;
use mfm_machine_test_support::stream_store_contract_tests;
use mfm_stream_store_postgres::PostgresStreamStore;

async fn connect_postgres_with_retry(max_attempts: u32, delay_ms: u64) -> PostgresStreamStore {
    let mut last_err: Option<StorageError> = None;
    for _ in 0..max_attempts {
        match PostgresStreamStore::connect_env().await {
            Ok(store) => return store,
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "<missing>".to_string());
    panic!(
        "postgres config after retries (DATABASE_URL={}): {:?}",
        db_url, last_err
    );
}

#[tokio::test]
async fn stream_store_postgres_contract() {
    let store = connect_postgres_with_retry(20, 250).await;
    stream_store_contract_tests(&store).await;
}
