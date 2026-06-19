use std::collections::BTreeSet;
use std::sync::Arc;

use mfm_events::v1 as events;
use mfm_ids::{NodeId, RunId};
use mfm_store::v1 as store;

use crate::admission::{RunAdmissionAuthority, RunAdmissionLifecycle};
use crate::artifacts::RuntimeArtifactStore;
use crate::attempt::{AttemptLifecycle, AttemptRunStatus, ResourceLaneBlockWitness};
use crate::binding::{BoundRuntimeContext, BoundRuntimeContextLoader};
use crate::commit::{PreparedRunLaunch, PreparedStagedArtifact, RunLaunchEvidence};
use crate::error::async_store_error;
use crate::framework_lifecycle::FrameworkAttemptLifecycle;
use crate::history::{RuntimeRunView, VerifiedRunContextLoader};
use crate::manual_resolution::{
    build_manual_resolution_prefix_authority, build_manual_resolution_prefix_authority_from_parts,
    certified_manual_resolution_spec, prepare_manual_resolution_commit,
    verify_manual_resolution_for_prefix, ManualResolutionEvidenceArtifact,
};
use crate::recovery::AttemptRecoveryLifecycle;
use crate::runners::ErasedRunnerRegistry;
use crate::transition::{TransitionAttempt, TransitionDecision, TransitionLifecycle};
use crate::{CertifiedRuntimeSpec, Result};

/// Result of one serial scheduler drive call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerStatus {
    /// At least one node attempt ran and committed.
    Advanced,
    /// No node is currently runnable.
    Blocked,
    /// Public output has already been projected.
    PublicOutputProjected,
}

/// Manual resolution input verified against the certified saga policy and current stream prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionRequest {
    /// Operator-selected terminal resolution outcome.
    pub outcome: events::ManualResolutionOutcome,
    /// Artifact containing the retained manual evidence bytes.
    pub evidence_artifact: ManualResolutionEvidenceArtifact,
    /// Canonical proof bytes for the manual resolution claim.
    pub proof_bytes: Vec<u8>,
    /// Optional retained note attached to the resolution.
    pub note: Option<events::ManualResolutionNote>,
}

enum DriveStepStatus {
    Advanced,
    StaleView,
    Blocked,
    PublicOutputProjected,
    BlockedOnResourceLane {
        witness: ResourceLaneBlockWitness,
        advanced: bool,
    },
    OperationalBlock {
        node_id: NodeId,
    },
}

/// Serial typed scheduler.
#[derive(Clone)]
pub struct SerialTypedScheduler {
    run_contexts: VerifiedRunContextLoader,
    artifact_store: Arc<dyn RuntimeArtifactStore>,
}

impl SerialTypedScheduler {
    /// Creates a scheduler using a certified runner registry and runtime artifact store.
    pub fn new(
        runners: ErasedRunnerRegistry,
        artifact_store: Arc<dyn RuntimeArtifactStore>,
    ) -> Self {
        Self {
            run_contexts: VerifiedRunContextLoader::new(BoundRuntimeContextLoader::new(runners)),
            artifact_store,
        }
    }

    /// Prepares sealed admission launch authority for a certified run.
    pub fn prepare_run_launch(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<PreparedRunLaunch> {
        let bound_context = self.run_contexts.load_bound_context(runtime_spec)?;
        RunAdmissionLifecycle::prepare_run_launch(
            runtime_spec,
            run_id,
            evidence,
            expected_next_seq,
            &bound_context,
        )
    }

    /// Appends the prepared typed admission commit after staging middleware-owned artifacts.
    pub async fn start_run<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        Ok(store.append_prepared_commit_plan(launch.commit.into())?)
    }

    /// Appends admission and reloads verified admission authority for the run.
    pub async fn start_run_admitted<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        launch: PreparedRunLaunch,
    ) -> Result<RunAdmissionAuthority> {
        let run_id = launch.commit.request().run_id().clone();
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        store.append_prepared_commit_plan(launch.commit.into())?;
        let stream = store.load_run_stream(&run_id);
        let bound_context = self.run_contexts.load_bound_context(runtime_spec)?;
        RunAdmissionLifecycle::admitted_run_authority(runtime_spec, &run_id, &stream, bound_context)
    }

    /// Appends the prepared typed admission commit through an async typed store.
    pub async fn start_run_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        store
            .append_prepared_commit_plan(launch.commit.into())
            .await
            .map_err(async_store_error)
    }

    /// Appends async admission and reloads verified admission authority for the run.
    pub async fn start_run_admitted_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        launch: PreparedRunLaunch,
    ) -> Result<RunAdmissionAuthority> {
        let run_id = launch.commit.request().run_id().clone();
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        store
            .append_prepared_commit_plan(launch.commit.into())
            .await
            .map_err(async_store_error)?;
        let stream = store
            .load_run_stream(&run_id)
            .await
            .map_err(async_store_error)?;
        let bound_context = self.run_contexts.load_bound_context(runtime_spec)?;
        RunAdmissionLifecycle::admitted_run_authority(runtime_spec, &run_id, &stream, bound_context)
    }

    /// Appends a verified manual resolution after staging evidence and authorization artifacts.
    pub async fn record_manual_resolution<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        request: ManualResolutionRequest,
    ) -> Result<store::CommitOutcome> {
        let ManualResolutionRequest {
            outcome,
            evidence_artifact,
            proof_bytes,
            note,
        } = request;
        let manual = certified_manual_resolution_spec(&runtime_spec.spec().saga)?;
        let prefix =
            build_manual_resolution_prefix_authority(store, runtime_spec, run_id, manual.clone())?;
        let verified =
            verify_manual_resolution_for_prefix(prefix, outcome, &evidence_artifact, proof_bytes)?;
        let stream = store.load_run_stream(run_id);
        let saga = store
            .projection_snapshot()
            .derive_saga_projection(run_id, &runtime_spec.spec().saga);
        let expected_next_seq = store.expected_next_seq(run_id);
        let (commit, artifacts_to_stage) = prepare_manual_resolution_commit(
            runtime_spec,
            &stream,
            &saga,
            expected_next_seq,
            verified,
            evidence_artifact,
            note,
        )?;
        self.stage_prepared_artifacts(&artifacts_to_stage).await?;
        Ok(store.append_prepared_commit_plan(commit.into())?)
    }

    /// Appends a verified manual resolution through an async durable typed store.
    pub async fn record_manual_resolution_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        request: ManualResolutionRequest,
    ) -> Result<store::CommitOutcome> {
        let ManualResolutionRequest {
            outcome,
            evidence_artifact,
            proof_bytes,
            note,
        } = request;
        let manual = certified_manual_resolution_spec(&runtime_spec.spec().saga)?;
        let stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let expected_next_seq = expected_next_seq_from_stream(&stream)?;
        let projection = store::ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
        let prefix = build_manual_resolution_prefix_authority_from_parts(
            runtime_spec,
            run_id,
            manual.clone(),
            &stream,
            expected_next_seq,
            &projection,
        )?;
        let verified =
            verify_manual_resolution_for_prefix(prefix, outcome, &evidence_artifact, proof_bytes)?;
        let saga = projection.derive_saga_projection(run_id, &runtime_spec.spec().saga);
        let (commit, artifacts_to_stage) = prepare_manual_resolution_commit(
            runtime_spec,
            &stream,
            &saga,
            expected_next_seq,
            verified,
            evidence_artifact,
            note,
        )?;
        self.stage_prepared_artifacts(&artifacts_to_stage).await?;
        store
            .append_prepared_commit_plan(commit.into())
            .await
            .map_err(async_store_error)
    }

    /// Runs one deterministic runnable node, if any.
    pub async fn drive_once<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut blocked_lanes = BTreeSet::new();
        loop {
            match self
                .drive_once_with_blocked_lanes(store, runtime_spec, run_id, &blocked_lanes)
                .await?
            {
                DriveStepStatus::Advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::StaleView => continue,
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
                DriveStepStatus::OperationalBlock { node_id } => {
                    let _ = node_id;
                    return Ok(SchedulerStatus::Blocked);
                }
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerStatus::PublicOutputProjected);
                }
                DriveStepStatus::BlockedOnResourceLane { witness, advanced } => {
                    if advanced {
                        return Ok(SchedulerStatus::Advanced);
                    }
                    blocked_lanes.insert(witness);
                }
            }
        }
    }

    /// Runs deterministic runnable nodes until no node is runnable or public output is projected.
    pub async fn drive_until_blocked<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        let mut blocked_lanes = BTreeSet::new();
        loop {
            match self
                .drive_once_with_blocked_lanes(store, runtime_spec, run_id, &blocked_lanes)
                .await?
            {
                DriveStepStatus::Advanced => advanced = true,
                DriveStepStatus::StaleView => continue,
                DriveStepStatus::BlockedOnResourceLane {
                    witness,
                    advanced: step_advanced,
                } => {
                    advanced |= step_advanced;
                    blocked_lanes.insert(witness);
                }
                DriveStepStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
                DriveStepStatus::OperationalBlock { node_id } if advanced => {
                    let _ = node_id;
                    return Ok(SchedulerStatus::Advanced);
                }
                DriveStepStatus::OperationalBlock { node_id } => {
                    let _ = node_id;
                    return Ok(SchedulerStatus::Blocked);
                }
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerStatus::PublicOutputProjected);
                }
            }
        }
    }

    /// Runs one deterministic runnable node against an async durable typed store, if any.
    pub async fn drive_once_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut blocked_lanes = BTreeSet::new();
        loop {
            match self
                .drive_once_async_with_blocked_lanes(store, runtime_spec, run_id, &blocked_lanes)
                .await?
            {
                DriveStepStatus::Advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::StaleView => continue,
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
                DriveStepStatus::OperationalBlock { node_id } => {
                    let _ = node_id;
                    return Ok(SchedulerStatus::Blocked);
                }
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerStatus::PublicOutputProjected);
                }
                DriveStepStatus::BlockedOnResourceLane { witness, advanced } => {
                    if advanced {
                        return Ok(SchedulerStatus::Advanced);
                    }
                    blocked_lanes.insert(witness);
                }
            }
        }
    }

    /// Runs deterministic runnable nodes against an async durable typed store until blocked.
    pub async fn drive_until_blocked_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        let mut blocked_lanes = BTreeSet::new();
        loop {
            match self
                .drive_once_async_with_blocked_lanes(store, runtime_spec, run_id, &blocked_lanes)
                .await?
            {
                DriveStepStatus::Advanced => advanced = true,
                DriveStepStatus::StaleView => continue,
                DriveStepStatus::BlockedOnResourceLane {
                    witness,
                    advanced: step_advanced,
                } => {
                    advanced |= step_advanced;
                    blocked_lanes.insert(witness);
                }
                DriveStepStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
                DriveStepStatus::OperationalBlock { node_id } if advanced => {
                    let _ = node_id;
                    return Ok(SchedulerStatus::Advanced);
                }
                DriveStepStatus::OperationalBlock { node_id } => {
                    let _ = node_id;
                    return Ok(SchedulerStatus::Blocked);
                }
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerStatus::PublicOutputProjected);
                }
            }
        }
    }

    async fn drive_once_with_blocked_lanes<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<DriveStepStatus> {
        let context = self.run_contexts.load(runtime_spec, run_id, store)?;
        let bound_context = context.bound_context();
        let view = context.view();
        match TransitionLifecycle::decide(runtime_spec, run_id, view, blocked_lanes)? {
            TransitionDecision::StartNode(attempt)
            | TransitionDecision::StartRemediation(attempt)
            | TransitionDecision::ResolveSagaTerminal(attempt)
            | TransitionDecision::ContinueAttempt(attempt) => {
                match self
                    .run_node_attempt(store, runtime_spec, run_id, view, bound_context, attempt)
                    .await?
                {
                    AttemptRunStatus::Advanced => Ok(DriveStepStatus::Advanced),
                    AttemptRunStatus::StaleView => Ok(DriveStepStatus::StaleView),
                    AttemptRunStatus::BlockedOnResourceLane { witness, advanced } => {
                        Ok(DriveStepStatus::BlockedOnResourceLane { witness, advanced })
                    }
                    AttemptRunStatus::OperationalBlock { node_id } => {
                        Ok(DriveStepStatus::OperationalBlock { node_id })
                    }
                }
            }
            TransitionDecision::AwaitManualResolution => Ok(DriveStepStatus::Blocked),
            TransitionDecision::Blocked
                if TransitionLifecycle::public_output_projected(
                    runtime_spec,
                    run_id,
                    view,
                    blocked_lanes,
                )? =>
            {
                Ok(DriveStepStatus::PublicOutputProjected)
            }
            TransitionDecision::Blocked => Ok(DriveStepStatus::Blocked),
        }
    }

    async fn drive_once_async_with_blocked_lanes<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        blocked_lanes: &BTreeSet<ResourceLaneBlockWitness>,
    ) -> Result<DriveStepStatus> {
        let context = self
            .run_contexts
            .load_async(runtime_spec, run_id, store)
            .await?;
        let bound_context = context.bound_context();
        let view = context.view();
        match TransitionLifecycle::decide(runtime_spec, run_id, view, blocked_lanes)? {
            TransitionDecision::StartNode(attempt)
            | TransitionDecision::StartRemediation(attempt)
            | TransitionDecision::ResolveSagaTerminal(attempt)
            | TransitionDecision::ContinueAttempt(attempt) => {
                match self
                    .run_node_attempt_async(
                        store,
                        runtime_spec,
                        run_id,
                        view,
                        bound_context,
                        attempt,
                    )
                    .await?
                {
                    AttemptRunStatus::Advanced => Ok(DriveStepStatus::Advanced),
                    AttemptRunStatus::StaleView => Ok(DriveStepStatus::StaleView),
                    AttemptRunStatus::BlockedOnResourceLane { witness, advanced } => {
                        Ok(DriveStepStatus::BlockedOnResourceLane { witness, advanced })
                    }
                    AttemptRunStatus::OperationalBlock { node_id } => {
                        Ok(DriveStepStatus::OperationalBlock { node_id })
                    }
                }
            }
            TransitionDecision::AwaitManualResolution => Ok(DriveStepStatus::Blocked),
            TransitionDecision::Blocked
                if TransitionLifecycle::public_output_projected(
                    runtime_spec,
                    run_id,
                    view,
                    blocked_lanes,
                )? =>
            {
                Ok(DriveStepStatus::PublicOutputProjected)
            }
            TransitionDecision::Blocked => Ok(DriveStepStatus::Blocked),
        }
    }

    async fn run_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        bound_context: &BoundRuntimeContext,
        attempt: TransitionAttempt<'_>,
    ) -> Result<AttemptRunStatus> {
        if let Some(status) = AttemptRecoveryLifecycle::dispatch_open_attempt_for_attempt(
            store,
            runtime_spec,
            run_id,
            view,
            &attempt,
        )? {
            return Ok(status);
        }
        if FrameworkAttemptLifecycle::owns_node(attempt.node) {
            return FrameworkAttemptLifecycle::new(self.artifact_store.as_ref())
                .run(store, runtime_spec, run_id, view, bound_context, attempt)
                .await;
        }
        AttemptLifecycle::new(self.artifact_store.as_ref())
            .run(store, runtime_spec, run_id, view, bound_context, attempt)
            .await
    }

    async fn run_node_attempt_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        bound_context: &BoundRuntimeContext,
        attempt: TransitionAttempt<'_>,
    ) -> Result<AttemptRunStatus> {
        if let Some(status) = AttemptRecoveryLifecycle::dispatch_open_attempt_for_attempt_async(
            store,
            runtime_spec,
            run_id,
            view,
            &attempt,
        )
        .await?
        {
            return Ok(status);
        }
        if FrameworkAttemptLifecycle::owns_node(attempt.node) {
            return FrameworkAttemptLifecycle::new(self.artifact_store.as_ref())
                .run_async(store, runtime_spec, run_id, view, bound_context, attempt)
                .await;
        }
        AttemptLifecycle::new(self.artifact_store.as_ref())
            .run_async(store, runtime_spec, run_id, view, bound_context, attempt)
            .await
    }

    async fn stage_prepared_artifacts(&self, artifacts: &[PreparedStagedArtifact]) -> Result<()> {
        for artifact in artifacts {
            self.artifact_store
                .stage_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
                .await?;
        }
        Ok(())
    }
}

fn expected_next_seq_from_stream(
    stream: &[store::KernelEventEnvelope],
) -> Result<store::StreamSeq> {
    let next = stream
        .last()
        .map(|event| {
            event.seq().as_u64().checked_add(1).ok_or_else(|| {
                crate::RuntimeError::InvalidRunStream(
                    "manual resolution stream sequence overflow".to_owned(),
                )
            })
        })
        .transpose()?
        .unwrap_or(store::StreamSeq::FIRST.as_u64());
    Ok(store::StreamSeq::new(next)?)
}
