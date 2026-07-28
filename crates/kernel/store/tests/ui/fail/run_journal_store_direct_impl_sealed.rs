use mfm_store::v1::{
    AsyncStoreFuture, CommitOutcome, CommittedRunJournal, PreparedCommitBundle, RunJournalStore,
    StoreError,
};

struct DirectStore;

impl RunJournalStore for DirectStore {
    type Error = StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        _bundle: PreparedCommitBundle,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error> {
        todo!()
    }

    fn load_committed_journal<'a>(
        &'a self,
        _run_id: &'a mfm_ids::RunId,
    ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error> {
        todo!()
    }
}

fn main() {}
