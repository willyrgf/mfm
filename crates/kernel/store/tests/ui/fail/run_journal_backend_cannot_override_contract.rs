use mfm_store::v1::{
    AsyncStoreFuture, CommitOutcome, CommittedRunJournal, JournalLoadVerifier,
    PreparedCommitBundle, RunJournalBackend, RunJournalStore, StoreError,
};

struct Backend;

impl RunJournalBackend for Backend {
    type Error = StoreError;

    fn backend_append<'a>(
        &'a self,
        _bundle: PreparedCommitBundle,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error> {
        todo!()
    }

    fn backend_load<'a>(
        &'a self,
        _verifier: JournalLoadVerifier,
    ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error> {
        todo!()
    }
}

impl RunJournalStore for Backend {
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
