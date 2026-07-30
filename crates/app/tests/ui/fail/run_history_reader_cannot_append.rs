use mfm_storage_postgres::PostgresRunJournalBackend;
use mfm_store::RunHistoryReader;

fn unavailable<T>() -> T {
    panic!("compile-fail placeholder")
}

fn mutate(reader: &RunHistoryReader<PostgresRunJournalBackend>) {
    reader.append_admission(unavailable(), unavailable());
}

fn main() {}
