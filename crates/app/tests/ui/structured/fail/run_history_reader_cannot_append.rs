use mfm_storage_postgres::PostgresStructuredHistoryBackend;
use mfm_store::structured::StructuredRunHistoryReader;

fn unavailable<T>() -> T {
    panic!("compile-fail placeholder")
}

async fn mutate(reader: &StructuredRunHistoryReader<PostgresStructuredHistoryBackend>) {
    reader.admit_run(unavailable()).await;
}

fn main() {}
