//! Private production adapter implementing [`RuntimeHistoryPort`].

use mfm_ids::{ContentRef, RunId};
use mfm_journal::structured::RecordRef;
use mfm_runtime::history::{
    HistoryFuture, QualifiedRuntimeIntent, RuntimeHistoryPort, StructuredAppendAttempt,
    StructuredStoreIdentity, VerifiedRunView,
};

use super::backend::{StructuredHistoryBackend, StructuredRunHistoryWriter};
use super::VerifiedStructuredRun;

impl VerifiedRunView for VerifiedStructuredRun {
    fn run_id(&self) -> &RunId {
        VerifiedStructuredRun::run_id(self)
    }

    fn admission(&self) -> &mfm_journal::structured::RunAdmitted {
        VerifiedStructuredRun::admission(self)
    }

    fn journal_head(&self) -> &mfm_journal::structured::JournalHead {
        VerifiedStructuredRun::journal_head(self)
    }

    fn semantic_head(&self) -> &mfm_journal::structured::SemanticHead {
        VerifiedStructuredRun::semantic_head(self)
    }

    fn frontier(&self) -> &mfm_runtime::history::StructuredFrontier {
        VerifiedStructuredRun::frontier(self)
    }

    fn object(&self, content_ref: &ContentRef) -> Option<&mfm_journal::structured::HistoryObject> {
        VerifiedStructuredRun::object(self, content_ref)
    }

    fn authorization(
        &self,
        access_attempt_id: &mfm_ids::AccessAttemptId,
    ) -> Option<(
        &RecordRef,
        &mfm_journal::structured::ExternalAccessAuthorized,
    )> {
        VerifiedStructuredRun::authorization(self, access_attempt_id)
    }

    fn observation(
        &self,
        access_attempt_id: &mfm_ids::AccessAttemptId,
    ) -> Option<(&RecordRef, &mfm_journal::structured::ExternalAccessObserved)> {
        VerifiedStructuredRun::observation(self, access_attempt_id)
    }
}

/// Runtime-facing holder of the sole store mutation authority.
pub(super) struct StoreHistoryAdapter<B: StructuredHistoryBackend> {
    writer: StructuredRunHistoryWriter<B>,
}

impl<B: StructuredHistoryBackend> mfm_authority_seal::RuntimeHistoryPortSeal
    for StoreHistoryAdapter<B>
{
}

impl<B: StructuredHistoryBackend> StoreHistoryAdapter<B> {
    pub(super) fn from_writer(writer: StructuredRunHistoryWriter<B>) -> Self {
        Self { writer }
    }
}

impl<B: StructuredHistoryBackend> RuntimeHistoryPort for StoreHistoryAdapter<B> {
    type VerifiedRun = VerifiedStructuredRun;

    fn store_identity(&self) -> &StructuredStoreIdentity {
        self.writer.store_identity()
    }

    fn load_verified<'a>(&'a self, run_id: &'a RunId) -> HistoryFuture<'a, Self::VerifiedRun> {
        Box::pin(async move { self.writer.load_verified(run_id).await })
    }

    fn commit_event<'a>(
        &'a self,
        previous: Option<Self::VerifiedRun>,
        intent: QualifiedRuntimeIntent,
    ) -> HistoryFuture<'a, (RunId, StructuredAppendAttempt)> {
        Box::pin(async move { self.writer.commit_event(previous, intent).await })
    }
}
