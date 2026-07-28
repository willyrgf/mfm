use mfm_events::v1 as events;
use mfm_store::v1 as store;

use crate::binding::BoundRuntimeContext;
use crate::commit::{CommitPlanner, PreparedRunLaunch, RunLaunchEvidence};
use crate::{CertifiedRuntimeSpec, Result};

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
}
