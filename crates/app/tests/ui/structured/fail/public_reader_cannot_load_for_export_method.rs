use mfm_ids::RunId;
use mfm_store::structured::PublicRunReader;
use mfm_storage_postgres::PostgresStructuredHistoryBackend;

fn main() {
    async fn forbid(reader: &PublicRunReader<PostgresStructuredHistoryBackend>, run_id: &RunId) {
        // Purpose-reader methods are not interchangeable: public cannot load export evidence.
        let _ = reader.load_for_export(run_id).await;
    }
    let _ = forbid;
}
