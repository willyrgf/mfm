use std::collections::BTreeMap;

use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, CellId, ContentDigest, SchemaId, SpecHash};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::spec_authority::{CurrentRuntimeIndex, CurrentRuntimeSpecRef};
use crate::{
    config_ref_key, CertifiedRuntimeSpec, MaterializedCell, MaterializedCellTerminal,
    MaterializedInputNode, MaterializedInputs, NamedMaterializedInput, Result, RuntimeError,
};

#[path = "history_artifacts.rs"]
mod history_artifacts;
#[path = "history_inputs.rs"]
mod history_inputs;

pub(crate) use self::history_artifacts::{
    committed_config_artifact, committed_input_artifact, event_artifact_ref_from_store,
    payload_spec_hash, run_artifact_ref_from_store, store_seed_artifact,
    validate_certificate_artifact, validate_config_artifacts, validate_spec_artifact,
};
pub(crate) use self::history_inputs::{materialize_inputs, validate_seed_cells};

/// One verified current run, containing the sole certified run view and non-authoritative runtime
/// indexes.
#[derive(Debug)]
pub struct VerifiedCurrentRun {
    view: Box<store::VerifiedRunView>,
    index: CurrentRuntimeIndex,
}

impl VerifiedCurrentRun {
    fn new(view: store::VerifiedRunView, index: CurrentRuntimeIndex) -> Result<Self> {
        let view = Box::new(view);
        index.validate_against(store::current_lifecycle::read(view.as_ref()).certified_spec())?;
        Ok(Self { view, index })
    }

    /// Returns the opaque store-owned verified run view.
    pub fn view(&self) -> &store::VerifiedRunView {
        self.view.as_ref()
    }

    pub(crate) fn runtime_spec(&self) -> CurrentRuntimeSpecRef<'_> {
        let lifecycle = store::current_lifecycle::read(self.view.as_ref());
        self.index.runtime_spec(lifecycle.certified_spec())
    }

    pub(crate) fn lifecycle(&self) -> store::current_lifecycle::CurrentLifecycleReader<'_> {
        store::current_lifecycle::read(self.view.as_ref())
    }

    pub(crate) fn verify_successor(self, successor: store::CommittedRunJournal) -> Result<Self> {
        let Self { view, index } = self;
        let view = (*view).verify_successor(successor)?;
        Self::new(view, index)
    }
}

/// Consumes pre-admission certification authority and binds it to one committed journal.
pub fn verify_current_run(
    journal: store::CommittedRunJournal,
    runtime_spec: CertifiedRuntimeSpec,
) -> Result<VerifiedCurrentRun> {
    let (certified, index) = runtime_spec.into_parts();
    index.validate_against(&certified)?;
    let view = journal.verify(certified)?;
    VerifiedCurrentRun::new(view, index)
}

/// Reloads and advances current-run authority after one store append outcome.
///
/// An appended batch must produce a strict successor. An idempotent result may observe the same
/// journal or a strict successor when another already-committed batch is also visible.
pub(crate) async fn refresh_current_after_commit<S>(
    store: &S,
    current: VerifiedCurrentRun,
    outcome: &store::CommitOutcome,
) -> Result<VerifiedCurrentRun>
where
    S: store::RunJournalStore + ?Sized,
{
    match outcome {
        store::CommitOutcome::Appended(_) => {
            reload_current(store, current, ReloadExpectation::StrictSuccessor).await
        }
        store::CommitOutcome::Idempotent(_) => {
            reload_current(store, current, ReloadExpectation::EqualOrSuccessor).await
        }
        store::CommitOutcome::ExecutionClaimBusy(_) | store::CommitOutcome::AdmissionBlocked(_) => {
            Ok(current)
        }
    }
}

/// Reloads a strict successor after an append lost its expected-sequence race.
pub(crate) async fn refresh_current_after_stale_append<S>(
    store: &S,
    current: VerifiedCurrentRun,
) -> Result<VerifiedCurrentRun>
where
    S: store::RunJournalStore + ?Sized,
{
    reload_current(store, current, ReloadExpectation::StrictSuccessor).await
}

#[derive(Debug, Clone, Copy)]
enum ReloadExpectation {
    EqualOrSuccessor,
    StrictSuccessor,
}

async fn reload_current<S>(
    store: &S,
    current: VerifiedCurrentRun,
    expectation: ReloadExpectation,
) -> Result<VerifiedCurrentRun>
where
    S: store::RunJournalStore + ?Sized,
{
    let run_id = current.view().run_id().clone();
    let current_sequence = current.view().current_run_sequence().ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "verified current run is missing its persisted sequence".to_owned(),
        )
    })?;
    let successor = store
        .load_committed_journal(&run_id)
        .await
        .map_err(crate::error::async_store_error)?;
    let successor_sequence = successor.current_run_sequence().ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "reloaded committed journal is missing its persisted sequence".to_owned(),
        )
    })?;
    match successor_sequence.cmp(&current_sequence) {
        std::cmp::Ordering::Greater => current.verify_successor(successor),
        std::cmp::Ordering::Equal
            if matches!(expectation, ReloadExpectation::EqualOrSuccessor) =>
        {
            Ok(current)
        }
        std::cmp::Ordering::Equal => Err(RuntimeError::InvalidRunStream(format!(
            "append reported progress for run {run_id} but reload remained at sequence {current_sequence}"
        ))),
        std::cmp::Ordering::Less => Err(RuntimeError::InvalidRunStream(format!(
            "reloaded journal for run {run_id} regressed from sequence {current_sequence} to {successor_sequence}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::VerifiedCurrentRun;

    trait AmbiguousIfClone<Marker> {
        fn marker() {}
    }

    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    impl<T: Clone> AmbiguousIfClone<u8> for T {}

    #[test]
    fn verified_current_run_stays_affine_and_stack_bounded() {
        let _ = <VerifiedCurrentRun as AmbiguousIfClone<_>>::marker;
        assert!(
            std::mem::size_of::<VerifiedCurrentRun>() <= 64,
            "verified current-run carrier grew beyond its conservative stack budget"
        );
    }
}
