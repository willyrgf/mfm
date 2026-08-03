use mfm_store::structured::{PublicRunReader, TraceRunReader};
use mfm_storage_postgres::PostgresStructuredHistoryBackend;

fn require_trace(_reader: &TraceRunReader<PostgresStructuredHistoryBackend>) {}

fn main() {
    fn check(reader: &PublicRunReader<PostgresStructuredHistoryBackend>) {
        require_trace(reader);
    }
    let _ = check;
}
