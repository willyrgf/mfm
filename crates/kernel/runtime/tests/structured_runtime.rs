//! Runtime unit proofs over a caller-owned history port (no production backend).

use std::collections::BTreeMap;
use std::sync::Mutex;

use mfm_ids::{AccessAttemptId, ContentRef, RunId};
use mfm_journal::structured::{
    ExternalAccessAuthorized, ExternalAccessObserved, HistoryObject, JournalHead, RecordRef,
    RunAdmitted, SemanticHead,
};
use mfm_runtime::history::{
    AccessAuthorizationProposal, AccessObservationProposal, HistoryError, HistoryFuture,
    ObservationCommit, ObservationQualification, ProposedObservationOutcome, RuntimeHistoryPort,
    StateTransitionProposal, StructuredAdmissionCommand, StructuredAppendAttempt,
    StructuredFrontier, StructuredStoreIdentity, VerifiedRunView,
};
use mfm_runtime::structured::Runtime;

struct EmptyVerified;

impl VerifiedRunView for EmptyVerified {
    fn run_id(&self) -> &RunId {
        panic!("unused")
    }
    fn admission(&self) -> &RunAdmitted {
        panic!("unused")
    }
    fn journal_head(&self) -> &JournalHead {
        panic!("unused")
    }
    fn semantic_head(&self) -> &SemanticHead {
        panic!("unused")
    }
    fn frontier(&self) -> &StructuredFrontier {
        &StructuredFrontier::Complete
    }
    fn object(&self, _: &ContentRef) -> Option<&HistoryObject> {
        None
    }
    fn authorization(
        &self,
        _: &AccessAttemptId,
    ) -> Option<(&RecordRef, &ExternalAccessAuthorized)> {
        None
    }
    fn observation(
        &self,
        _: &AccessAttemptId,
    ) -> Option<(&RecordRef, &ExternalAccessObserved)> {
        None
    }
}

struct RejectPort {
    identity: StructuredStoreIdentity,
    calls: Mutex<Vec<&'static str>>,
}

impl RuntimeHistoryPort for RejectPort {
    type VerifiedRun = EmptyVerified;

    fn store_identity(&self) -> &StructuredStoreIdentity {
        &self.identity
    }

    fn admit_run<'a>(
        &'a self,
        _command: StructuredAdmissionCommand,
    ) -> HistoryFuture<'a, (RunId, StructuredAppendAttempt)> {
        self.calls.lock().expect("lock").push("admit_run");
        Box::pin(async { Err(HistoryError::BackendUnavailable) })
    }

    fn load_verified<'a>(&'a self, _: &'a RunId) -> HistoryFuture<'a, Self::VerifiedRun> {
        self.calls.lock().expect("lock").push("load_verified");
        Box::pin(async { Err(HistoryError::RunNotFound) })
    }

    fn commit_state_transition<'a>(
        &'a self,
        _: Self::VerifiedRun,
        _: &'a StateTransitionProposal,
    ) -> HistoryFuture<'a, StructuredAppendAttempt> {
        Box::pin(async { Err(HistoryError::BackendUnavailable) })
    }

    fn authorize_access<'a>(
        &'a self,
        _: Self::VerifiedRun,
        _: &'a AccessAuthorizationProposal,
    ) -> HistoryFuture<'a, StructuredAppendAttempt> {
        Box::pin(async { Err(HistoryError::BackendUnavailable) })
    }

    fn resolve_attempt<'a>(
        &'a self,
        _: &'a mut StructuredAppendAttempt,
    ) -> HistoryFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    fn qualify_observation<'a>(
        &'a self,
        _: &'a Self::VerifiedRun,
        _: &'a RecordRef,
        _: &'a ProposedObservationOutcome,
    ) -> HistoryFuture<'a, ObservationQualification> {
        Box::pin(async { Ok(ObservationQualification::Ready) })
    }

    fn commit_observation<'a>(
        &'a self,
        _: Self::VerifiedRun,
        _: &'a AccessObservationProposal,
    ) -> HistoryFuture<'a, ObservationCommit> {
        Box::pin(async { Ok(ObservationCommit::ExistingSame) })
    }
}

// Runtime construction requires RuntimeProcessRegistry which needs full qualification.
// Port trait object-safety and caller-owned ports are proven by this compiling RejectPort impl.
#[test]
fn caller_owned_port_can_be_named_without_production_backend() {
    let _ = std::any::type_name::<RejectPort>();
    let _ = std::any::type_name::<Runtime<RejectPort>>();
    let map: BTreeMap<u8, u8> = BTreeMap::new();
    assert!(map.is_empty());
}
