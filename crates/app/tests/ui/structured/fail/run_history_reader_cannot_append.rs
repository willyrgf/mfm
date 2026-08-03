use mfm_store::structured::PublicRunReader;
use mfm_storage_postgres::PostgresStructuredHistoryBackend;

fn main() {
    // Purpose readers must not expose semantic mutation entry points.
    fn forbid(reader: &PublicRunReader<PostgresStructuredHistoryBackend>) {
        let _ = reader.admit_run;
    }
    let _ = forbid;
}
