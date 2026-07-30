use mfm_storage_postgres::PostgresRunJournalBackend;

fn raw_pool(backend: &PostgresRunJournalBackend) {
    let _ = backend.writer_pool();
}

fn main() {}
