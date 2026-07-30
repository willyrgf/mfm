use mfm_storage_postgres::PostgresRunJournalBackend;
use mfm_store::QualifiedRunStore;

fn duplicate(
    assembly: &QualifiedRunStore<PostgresRunJournalBackend>,
) -> QualifiedRunStore<PostgresRunJournalBackend> {
    Clone::clone(assembly)
}

fn main() {}
