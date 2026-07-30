use mfm_storage_postgres::PostgresRunJournalBackend;
use mfm_store::RunHistoryWriter;

fn duplicate(
    writer: &RunHistoryWriter<PostgresRunJournalBackend>,
) -> RunHistoryWriter<PostgresRunJournalBackend> {
    Clone::clone(writer)
}

fn main() {}
