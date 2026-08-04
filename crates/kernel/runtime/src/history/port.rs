//! Consumer-side Runtime history port.

use std::future::Future;
use std::pin::Pin;

use mfm_ids::{AccessAttemptId, ContentRef, RunId};
use mfm_journal::structured::{
    ExternalAccessAuthorized, ExternalAccessObserved, HistoryObject, JournalHead, RecordRef,
    RunAdmitted, SemanticHead,
};

use super::commands::{
    AccessAuthorizationProposal, AccessObservationProposal, StateTransitionProposal,
    StructuredAdmissionCommand,
};
use super::cursor::{ObservationQualification, StructuredFrontier};
use super::identity::StructuredStoreIdentity;
use super::proofs::{ObservationCommit, StructuredAppendAttempt};
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

/// Append-attempt operations required by Runtime ambiguity handling.
pub trait AppendAttemptApi: Send {
    /// Returns the exact backend disposition.
    fn outcome(&self) -> &super::HistoryAppendOutcome;

    /// Returns whether root closure was part of a committed batch.
    fn closed(&self) -> bool;

    /// Consumes a newly committed authorization into its one-use proof.
    fn into_committed_access_authorization(self) -> Option<super::CommittedAccessAuthorization>
    where
        Self: Sized;

    /// Confirms resolve of acknowledgement-unknown into existing same content.
    fn confirm_existing_same(&mut self);
}

/// Authorization proof operations required by Runtime access brackets.
pub trait AuthorizationApi: Send {
    /// Returns the exact assigned authorization record reference.
    fn authorization_ref(&self) -> &RecordRef;

    /// Returns the kernel-derived access-attempt identity.
    fn access_attempt_id(&self) -> &AccessAttemptId;

    /// Returns the complete immutable committed authorization.
    fn authorization(&self) -> &ExternalAccessAuthorized;
}

/// Consumer-side port through which Runtime requests semantic history mutation.
///
/// Production adapters are private to store assembly and own the sole fold and
/// backend. A caller-implemented test port conveys authority only over resources
/// that port already owns and cannot be attached to MFM production backends.
pub trait RuntimeHistoryPort: Send + Sync {
    /// Verified run view loaded for drive and mutation basing.
    type VerifiedRun: VerifiedRunView;

    /// Returns the immutable qualified store identity.
    fn store_identity(&self) -> &StructuredStoreIdentity;

    /// Verifies and atomically admits one exact structured run.
    ///
    /// Run identity is derived inside the adapter from the annex preimage. The
    /// derived identifier is returned with the append attempt outcome via the
    /// committed admission record; callers never supply a trusted run digest.
    fn admit_run<'a>(
        &'a self,
        command: StructuredAdmissionCommand,
    ) -> HistoryFuture<'a, (RunId, StructuredAppendAttempt)>;

    /// Loads and callback-free verifies one exact run for a Runtime action.
    fn load_verified<'a>(&'a self, run_id: &'a RunId) -> HistoryFuture<'a, Self::VerifiedRun>;

    /// Returns a verified run that performed no append to its owning bounded store cache.
    fn retain_verified<'a>(&'a self, verified: Self::VerifiedRun) -> HistoryFuture<'a, ()>;

    /// Verifies and atomically commits one exact current callback result.
    fn commit_state_transition<'a>(
        &'a self,
        verified: Self::VerifiedRun,
        proposal: &'a StateTransitionProposal,
    ) -> HistoryFuture<'a, StructuredAppendAttempt>;

    /// Verifies and atomically authorizes one exact current Read or Effect.
    fn authorize_access<'a>(
        &'a self,
        verified: Self::VerifiedRun,
        proposal: &'a AccessAuthorizationProposal,
    ) -> HistoryFuture<'a, StructuredAppendAttempt>;

    /// Resolves one unchanged attempt and accepts only its exact retained
    /// store-validated candidate.
    fn resolve_attempt<'a>(
        &'a self,
        attempt: &'a mut StructuredAppendAttempt,
    ) -> HistoryFuture<'a, bool>;

    /// Validates one invoked completion before Runtime freezes pending content.
    fn qualify_observation<'a>(
        &'a self,
        verified: &'a Self::VerifiedRun,
        authorization_ref: &'a RecordRef,
        outcome: &'a super::ProposedObservationOutcome,
    ) -> HistoryFuture<'a, ObservationQualification>;

    /// Verifies and atomically records one exact consumed access completion.
    fn commit_observation<'a>(
        &'a self,
        verified: Self::VerifiedRun,
        proposal: &'a AccessObservationProposal,
    ) -> HistoryFuture<'a, ObservationCommit>;
}

impl AppendAttemptApi for StructuredAppendAttempt {
    fn outcome(&self) -> &super::HistoryAppendOutcome {
        self.outcome()
    }

    fn closed(&self) -> bool {
        self.closed()
    }

    fn into_committed_access_authorization(self) -> Option<super::CommittedAccessAuthorization> {
        StructuredAppendAttempt::into_committed_access_authorization(self)
    }

    fn confirm_existing_same(&mut self) {
        StructuredAppendAttempt::confirm_existing_same(self);
    }
}

impl AuthorizationApi for super::CommittedAccessAuthorization {
    fn authorization_ref(&self) -> &RecordRef {
        self.authorization_ref()
    }

    fn access_attempt_id(&self) -> &AccessAttemptId {
        self.access_attempt_id()
    }

    fn authorization(&self) -> &ExternalAccessAuthorized {
        self.authorization()
    }
}
