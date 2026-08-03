use mfm_store::structured::PublicRunReader;
use mfm_storage_postgres::PostgresStructuredHistoryBackend;

fn main() {
    fn forbid(reader: &PublicRunReader<PostgresStructuredHistoryBackend>) {
        // Public-read authority must not expose export load entry points.
        let _ = reader.load_for_export;
    }
    let _ = forbid;
}
