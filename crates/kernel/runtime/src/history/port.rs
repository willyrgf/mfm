//! Consumer-side Runtime history port.

use std::future::Future;
use std::pin::Pin;

use mfm_ids::{AccessAttemptId, ContentRef, RunId};
use mfm_journal::structured::{
    ExternalAccessAuthorized, ExternalAccessObserved, HistoryObject, JournalHead, RecordRef,
    RunAdmitted, SemanticHead,
};

use super::commands::QualifiedRuntimeIntent;
use super::cursor::StructuredFrontier;
use super::identity::StructuredStoreIdentity;
use super::proofs::StructuredAppendAttempt;
use super::Result;

/// Boxed asynchronous history-port operation.
pub type HistoryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Read view required by Runtime over one verified structured run.
pub trait VerifiedRunView: Send {
    /// Returns the exact run identity.
    fn run_id(&self) -> &RunId;

    /// Returns the exact verified admission root.
    fn admission(&self) -> &RunAdmitted;

    /// Returns the exact physical journal head.
    fn journal_head(&self) -> &JournalHead;

    /// Returns the exact semantic head.
    fn semantic_head(&self) -> &SemanticHead;

    /// Returns the closed action frontier.
    fn frontier(&self) -> &StructuredFrontier;

    /// Resolves one exact verified content-addressed history object.
    fn object(&self, content_ref: &ContentRef) -> Option<&HistoryObject>;

    /// Resolves one exact verified access authorization and its record identity.
    fn authorization(
        &self,
        access_attempt_id: &AccessAttemptId,
    ) -> Option<(&RecordRef, &ExternalAccessAuthorized)>;

    /// Resolves one exact verified access observation and its record identity.
    fn observation(
        &self,
        access_attempt_id: &AccessAttemptId,
    ) -> Option<(&RecordRef, &ExternalAccessObserved)>;
}

/// Consumer-side port through which Runtime requests semantic history mutation.
///
/// Production adapters are private to semantic store open. Every event enters
/// through the same consuming operation; Runtime cannot author or correct a
/// persisted candidate itself.
pub trait RuntimeHistoryPort: mfm_authority_seal::RuntimeHistoryPortSeal + Send + Sync {
    /// Verified run view loaded for drive and mutation basing.
    type VerifiedRun: VerifiedRunView;

    /// Returns the immutable qualified store identity.
    fn store_identity(&self) -> &StructuredStoreIdentity;

    /// Loads and callback-free verifies one exact current run.
    fn load_verified<'a>(&'a self, run_id: &'a RunId) -> HistoryFuture<'a, Self::VerifiedRun>;

    /// Qualifies and commits one normalized semantic intent.
    ///
    /// Admission supplies no predecessor. Every other intent consumes the
    /// exact verified prefix on which Runtime selected the action.
    fn commit_event<'a>(
        &'a self,
        previous: Option<Self::VerifiedRun>,
        intent: QualifiedRuntimeIntent,
    ) -> HistoryFuture<'a, (RunId, StructuredAppendAttempt)>;
}
