use mfm_storage_postgres::PostgresStructuredHistoryBackend;

fn raw_pool(backend: &PostgresStructuredHistoryBackend) {
    let _ = backend.writer_pool();
}

fn main() {}
