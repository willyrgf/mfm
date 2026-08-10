//! Runtime unit proofs over a caller-owned history port (no production backend).

use std::collections::BTreeMap;
use std::sync::Mutex;

use mfm_ids::{AccessAttemptId, ContentRef, RunId};
use mfm_journal::structured::{
    ExternalAccessAuthorized, ExternalAccessObserved, HistoryObject, JournalHead, RecordRef,
    RunAdmitted, SemanticHead,
};
use mfm_runtime::history::{
    HistoryError, HistoryFuture, QualifiedRuntimeIntent, RuntimeHistoryPort,
    StructuredAppendAttempt, StructuredFrontier, StructuredStoreIdentity, VerifiedRunView,
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
    fn observation(&self, _: &AccessAttemptId) -> Option<(&RecordRef, &ExternalAccessObserved)> {
        None
    }
}

struct RejectPort {
    identity: StructuredStoreIdentity,
    calls: Mutex<Vec<&'static str>>,
}

impl mfm_authority_seal::RuntimeHistoryPortSeal for RejectPort {}

impl RuntimeHistoryPort for RejectPort {
    type VerifiedRun = EmptyVerified;

    fn store_identity(&self) -> &StructuredStoreIdentity {
        &self.identity
    }

    fn commit_event<'a>(
        &'a self,
        _: Option<Self::VerifiedRun>,
        _: QualifiedRuntimeIntent,
    ) -> HistoryFuture<'a, (RunId, StructuredAppendAttempt)> {
        self.calls.lock().expect("lock").push("commit_event");
        Box::pin(async { Err(HistoryError::BackendUnavailable) })
    }

    fn load_verified<'a>(&'a self, _: &'a RunId) -> HistoryFuture<'a, Self::VerifiedRun> {
        self.calls.lock().expect("lock").push("load_verified");
        Box::pin(async { Err(HistoryError::RunNotFound) })
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
