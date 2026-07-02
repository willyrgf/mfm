use mfm_events::v1 as events;
use mfm_ids::{RunId, SpecHash};
use mfm_store::v1 as store;

use crate::binding::BoundRuntimeContext;
use crate::commit::{CommitPlanner, PreparedRunLaunch, RunLaunchEvidence};
use crate::history::RuntimeRunView;
use crate::{CertifiedRuntimeSpec, Result};

/// Verified authority returned after admission has appended and reloaded the run root.
#[derive(Clone)]
pub struct RunAdmissionAuthority {
    run_id: RunId,
    spec_hash: SpecHash,
    head_seq: store::StreamSeq,
    bound_context: BoundRuntimeContext,
}

impl RunAdmissionAuthority {
    /// Returns the admitted run id.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the certified spec hash admitted for the run.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Returns the next stream sequence after the verified admission prefix.
    pub fn head_seq(&self) -> store::StreamSeq {
        self.head_seq
    }

    /// Returns the runtime context authority bound at admission.
    pub fn bound_context(&self) -> &BoundRuntimeContext {
        &self.bound_context
    }
}

/// Run-start admission lifecycle.
///
/// Admission is the pre-FSM boundary that turns certified spec authority and verified launch
/// evidence into a prepared append-only admission commit.
pub struct RunAdmissionLifecycle;

impl RunAdmissionLifecycle {
    /// Prepares sealed admission launch authority for a certified and bound run.
    pub fn prepare_run_launch(
        runtime_spec: &CertifiedRuntimeSpec,
        identity_material: events::RunIdentityMaterialV1,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
        bound_context: &BoundRuntimeContext,
    ) -> Result<PreparedRunLaunch> {
        bound_context.validate_admission_authority(runtime_spec)?;
        CommitPlanner::prepare_run_launch(
            runtime_spec,
            identity_material,
            evidence,
            expected_next_seq,
            bound_context,
        )
    }

    /// Verifies the post-append admission stream and returns admitted run/context authority.
    pub fn admitted_run_authority(
        runtime_spec: &CertifiedRuntimeSpec,
        committed: &store::CommittedRunStream,
        bound_context: BoundRuntimeContext,
    ) -> Result<RunAdmissionAuthority> {
        bound_context.validate_admission_authority(runtime_spec)?;
        let view = RuntimeRunView::from_committed_stream(runtime_spec, committed)?;
        bound_context.validate_run_admitted_binding(&view.run_admitted)?;
        Ok(RunAdmissionAuthority {
            run_id: committed.run_id().clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            head_seq: view.next_seq,
            bound_context,
        })
    }
}
