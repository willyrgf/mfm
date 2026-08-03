use mfm_storage_postgres::PostgresStructuredHistoryBackend;
use mfm_store::structured::StructuredRunHistoryWriter;

fn duplicate(
    writer: &StructuredRunHistoryWriter<PostgresStructuredHistoryBackend>,
) -> StructuredRunHistoryWriter<PostgresStructuredHistoryBackend> {
    Clone::clone(writer)
}

fn main() {}
