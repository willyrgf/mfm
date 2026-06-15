use std::collections::BTreeSet;
use std::sync::Arc;

use mfm_events::v1 as events;
use mfm_ids::{AttemptId, NodeId, RunId};
use mfm_manual_auth::VerifiedManualResolution;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::RuntimeArtifactStager;
use crate::commit::{
    CommitPlanner, PreparedRunLaunch, PreparedStagedArtifact, RunLaunchEvidence,
    RunnerOutputCommitInput,
};
use crate::error::async_store_error;
use crate::frontier::{
    scheduler_decision_with_blocked_nodes, AttemptPlan, RunnableNode, SchedulerDecision,
};
use crate::history::{
    committed_config_artifact, materialize_inputs, recorded_facts_for_attempt, RuntimeRunView,
};
use crate::invocation::{CertifiedRuntimeCapabilities, ErasedRunCtx, PreparedRunnerInvocation};
use crate::manual_resolution::{
    prepare_manual_resolution_commit, ManualResolutionEvidenceArtifact,
};
use crate::runners::{ErasedRunnerBinding, ErasedRunnerRegistry};
use crate::{attempt_id, CertifiedRuntimeSpec, Result, RuntimeError};

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

struct RunnerInvocationInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    view: &'a RuntimeRunView,
}

struct FrameworkNodeAttemptInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    view: &'a RuntimeRunView,
    runnable: RunnableNode<'a>,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    binding: ErasedRunnerBinding,
}

enum DriveStepStatus {
    Advanced,
    Blocked,
    PublicOutputProjected,
    BlockedOnResourceLane { node_id: NodeId, advanced: bool },
}

enum NodeRunStatus {
    Advanced,
    BlockedOnResourceLane { node_id: NodeId, advanced: bool },
}

fn prepare_runner_invocation<'a>(
    input: RunnerInvocationInput<'a>,
) -> Result<PreparedRunnerInvocation<'a>> {
    let RunnerInvocationInput {
        runtime_spec,
        run_id,
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no,
        view,
    } = input;
    let config_artifact = committed_config_artifact(node, view)?;
    let inputs = materialize_inputs(runtime_spec, node, view)?;
    let caps =
        CertifiedRuntimeCapabilities::new(node.node_id.clone(), node.capability_bindings.clone());
    let recorded_facts = recorded_facts_for_attempt(&view.projections, &node.node_id, attempt_id)?;
    Ok(PreparedRunnerInvocation {
        runtime_spec,
        run_id,
        spec_hash: runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no,
        config_artifact,
        inputs,
        caps,
        recorded_facts,
        projections: &view.projections,
        run_stream: &view.stream,
    })
}

fn resource_lane_block_for_request(
    projections: &store::ProjectionSnapshot,
    request: &store::TypedCommitRequest,
) -> Option<store::ResourceLaneKey> {
    request.payloads.iter().find_map(|payload| {
        let events::KernelEventPayload::SideEffectInvocationPrepared(payload) = payload else {
            return None;
        };
        let resource_key = payload.resource_key.as_ref()?;
        let lane_key = store::ResourceLaneKey::from_evidence(resource_key);
        let holder =
            store::SideEffectLedgerRef::new(request.run_id.clone(), payload.ledger_key.clone());
        projections
            .resource_lane(&lane_key)
            .filter(|projection| projection.holder != holder)
            .map(|_| lane_key)
    })
}

fn request_has_resource_lane_prepare(request: &store::TypedCommitRequest) -> bool {
    request.payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationPrepared(payload)
                if payload.resource_key.is_some()
        )
    })
}

fn store_error_is_resource_lane_block(error: &store::StoreError) -> bool {
    matches!(error, store::StoreError::ResourceLaneBlocked { .. })
}

fn async_error_is_resource_lane_block(error: &impl store::StoreErrorInspection) -> bool {
    error
        .as_store_error()
        .is_some_and(store_error_is_resource_lane_block)
}

/// Serial typed scheduler.
#[derive(Clone)]
pub struct SerialTypedScheduler {
    runners: ErasedRunnerRegistry,
    artifact_stager: Arc<dyn RuntimeArtifactStager>,
}

impl SerialTypedScheduler {
    /// Creates a scheduler using a certified runner registry and runtime artifact stager.
    pub fn new(
        runners: ErasedRunnerRegistry,
        artifact_stager: Arc<dyn RuntimeArtifactStager>,
    ) -> Self {
        Self {
            runners,
            artifact_stager,
        }
    }

    /// Prepares sealed genesis launch authority for a certified run.
    pub fn prepare_run_launch(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunLaunchEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<PreparedRunLaunch> {
        CommitPlanner::prepare_run_launch(
            &self.runners,
            runtime_spec,
            run_id,
            evidence,
            expected_next_seq,
        )
    }

    /// Appends the prepared typed genesis commit after staging middleware-owned artifacts.
    pub async fn start_run<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        Ok(store.append_prepared_typed_commit(launch.commit)?)
    }

    /// Appends the prepared typed genesis commit through an async typed store.
    pub async fn start_run_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        launch: PreparedRunLaunch,
    ) -> Result<store::CommitOutcome> {
        self.stage_prepared_artifacts(&launch.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(launch.commit)
            .await
            .map_err(async_store_error)
    }

    /// Appends a verified manual resolution after staging evidence and authorization artifacts.
    pub async fn record_manual_resolution<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        verified: VerifiedManualResolution,
        evidence_artifact: ManualResolutionEvidenceArtifact,
        note: Option<events::ManualResolutionNote>,
    ) -> Result<store::CommitOutcome> {
        let run_id = verified.claim().run_id.clone();
        let stream = store.load_run_stream(&run_id);
        let saga = store
            .projection_snapshot()
            .derive_saga_projection(&run_id, &runtime_spec.spec().saga);
        let expected_next_seq = store.expected_next_seq(&run_id);
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
        Ok(store.append_prepared_typed_commit(commit)?)
    }

    /// Runs one deterministic runnable node, if any.
    pub async fn drive_once<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut blocked_nodes = BTreeSet::new();
        loop {
            match self
                .drive_once_with_blocked_nodes(store, runtime_spec, run_id, &blocked_nodes)
                .await?
            {
                DriveStepStatus::Advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerStatus::PublicOutputProjected);
                }
                DriveStepStatus::BlockedOnResourceLane { node_id, advanced } => {
                    if advanced {
                        return Ok(SchedulerStatus::Advanced);
                    }
                    blocked_nodes.insert(node_id);
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
        let mut blocked_nodes = BTreeSet::new();
        loop {
            match self
                .drive_once_with_blocked_nodes(store, runtime_spec, run_id, &blocked_nodes)
                .await?
            {
                DriveStepStatus::Advanced => advanced = true,
                DriveStepStatus::BlockedOnResourceLane {
                    node_id,
                    advanced: step_advanced,
                } => {
                    advanced |= step_advanced;
                    blocked_nodes.insert(node_id);
                }
                DriveStepStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
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
        let mut blocked_nodes = BTreeSet::new();
        loop {
            match self
                .drive_once_async_with_blocked_nodes(store, runtime_spec, run_id, &blocked_nodes)
                .await?
            {
                DriveStepStatus::Advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerStatus::PublicOutputProjected);
                }
                DriveStepStatus::BlockedOnResourceLane { node_id, advanced } => {
                    if advanced {
                        return Ok(SchedulerStatus::Advanced);
                    }
                    blocked_nodes.insert(node_id);
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
        let mut blocked_nodes = BTreeSet::new();
        loop {
            match self
                .drive_once_async_with_blocked_nodes(store, runtime_spec, run_id, &blocked_nodes)
                .await?
            {
                DriveStepStatus::Advanced => advanced = true,
                DriveStepStatus::BlockedOnResourceLane {
                    node_id,
                    advanced: step_advanced,
                } => {
                    advanced |= step_advanced;
                    blocked_nodes.insert(node_id);
                }
                DriveStepStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                DriveStepStatus::Blocked => return Ok(SchedulerStatus::Blocked),
                DriveStepStatus::PublicOutputProjected => {
                    return Ok(SchedulerStatus::PublicOutputProjected);
                }
            }
        }
    }

    async fn drive_once_with_blocked_nodes<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        blocked_nodes: &BTreeSet<NodeId>,
    ) -> Result<DriveStepStatus> {
        let view = RuntimeRunView::from_store(runtime_spec, run_id, store)?;
        match scheduler_decision_with_blocked_nodes(runtime_spec, run_id, &view, blocked_nodes)? {
            SchedulerDecision::Run(runnable) => {
                match self
                    .run_node_attempt(store, runtime_spec, run_id, &view, runnable)
                    .await?
                {
                    NodeRunStatus::Advanced => Ok(DriveStepStatus::Advanced),
                    NodeRunStatus::BlockedOnResourceLane { node_id, advanced } => {
                        Ok(DriveStepStatus::BlockedOnResourceLane { node_id, advanced })
                    }
                }
            }
            SchedulerDecision::Blocked => Ok(DriveStepStatus::Blocked),
            SchedulerDecision::Completed => Ok(DriveStepStatus::PublicOutputProjected),
        }
    }

    async fn drive_once_async_with_blocked_nodes<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        blocked_nodes: &BTreeSet<NodeId>,
    ) -> Result<DriveStepStatus> {
        let stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let view = RuntimeRunView::from_stream(runtime_spec, run_id, &stream)?;
        match scheduler_decision_with_blocked_nodes(runtime_spec, run_id, &view, blocked_nodes)? {
            SchedulerDecision::Run(runnable) => {
                match self
                    .run_node_attempt_async(store, runtime_spec, run_id, &view, runnable)
                    .await?
                {
                    NodeRunStatus::Advanced => Ok(DriveStepStatus::Advanced),
                    NodeRunStatus::BlockedOnResourceLane { node_id, advanced } => {
                        Ok(DriveStepStatus::BlockedOnResourceLane { node_id, advanced })
                    }
                }
            }
            SchedulerDecision::Blocked => Ok(DriveStepStatus::Blocked),
            SchedulerDecision::Completed => Ok(DriveStepStatus::PublicOutputProjected),
        }
    }

    async fn run_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<NodeRunStatus> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
                    | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
            )
        ) {
            return self
                .run_started_framework_node_attempt(
                    store,
                    FrameworkNodeAttemptInput {
                        runtime_spec,
                        run_id,
                        view,
                        runnable,
                        descriptor,
                        output_cell,
                        binding,
                    },
                )
                .await;
        }
        let mut advanced = false;
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew { attempt_no } => {
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(RunnerInvocationInput {
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    attempt_id: &attempt_id,
                    attempt_no,
                    view,
                })?;
                let start_commit = CommitPlanner::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store.append_prepared_typed_commit(start_commit)?;
                advanced = true;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store.load_run_stream(run_id);
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view: &latest_view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id: &attempt_id,
            caps: invocation.caps(),
            recorded_facts: invocation.recorded_facts(),
            view: &latest_view,
            output,
        })?;
        if resource_lane_block_for_request(
            store.projection_snapshot(),
            terminal_output.commit.request(),
        )
        .is_some()
        {
            return Ok(NodeRunStatus::BlockedOnResourceLane {
                node_id: node.node_id.clone(),
                advanced,
            });
        }
        let has_resource_lane_prepare =
            request_has_resource_lane_prepare(terminal_output.commit.request());
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        match store.append_prepared_typed_commit(terminal_output.commit) {
            Ok(_) => Ok(NodeRunStatus::Advanced),
            Err(error)
                if has_resource_lane_prepare && store_error_is_resource_lane_block(&error) =>
            {
                Ok(NodeRunStatus::BlockedOnResourceLane {
                    node_id: node.node_id.clone(),
                    advanced,
                })
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn run_started_framework_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<NodeRunStatus> {
        let FrameworkNodeAttemptInput {
            runtime_spec,
            run_id,
            view,
            runnable,
            descriptor,
            output_cell,
            binding,
        } = input;
        let node = runnable.node;
        let AttemptPlan::StartNew { attempt_no } = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(NodeRunStatus::Advanced)
    }

    async fn run_node_attempt_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<NodeRunStatus> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
                    | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
            )
        ) {
            return self
                .run_started_framework_node_attempt_async(
                    store,
                    FrameworkNodeAttemptInput {
                        runtime_spec,
                        run_id,
                        view,
                        runnable,
                        descriptor,
                        output_cell,
                        binding,
                    },
                )
                .await;
        }
        let mut advanced = false;
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew { attempt_no } => {
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(RunnerInvocationInput {
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    attempt_id: &attempt_id,
                    attempt_no,
                    view,
                })?;
                let start_commit = CommitPlanner::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store
                    .append_prepared_typed_commit(start_commit)
                    .await
                    .map_err(async_store_error)?;
                advanced = true;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view: &latest_view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id: &attempt_id,
            caps: invocation.caps(),
            recorded_facts: invocation.recorded_facts(),
            view: &latest_view,
            output,
        })?;
        let has_resource_lane_prepare =
            request_has_resource_lane_prepare(terminal_output.commit.request());
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        match store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
        {
            Ok(_) => Ok(NodeRunStatus::Advanced),
            Err(error)
                if has_resource_lane_prepare && async_error_is_resource_lane_block(&error) =>
            {
                Ok(NodeRunStatus::BlockedOnResourceLane {
                    node_id: node.node_id.clone(),
                    advanced,
                })
            }
            Err(error) => Err(async_store_error(error)),
        }
    }

    async fn run_started_framework_node_attempt_async<
        S: store::AsyncTypedRunEventStore + ?Sized,
    >(
        &self,
        store: &S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<NodeRunStatus> {
        let FrameworkNodeAttemptInput {
            runtime_spec,
            run_id,
            view,
            runnable,
            descriptor,
            output_cell,
            binding,
        } = input;
        let node = runnable.node;
        let AttemptPlan::StartNew { attempt_no } = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(RunnerInvocationInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view,
        })?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = CommitPlanner::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(NodeRunStatus::Advanced)
    }

    async fn stage_prepared_artifacts(&self, artifacts: &[PreparedStagedArtifact]) -> Result<()> {
        for artifact in artifacts {
            self.artifact_stager
                .stage_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_ids::{DigestAlgorithm, DigestBytes};
    use mfm_spec::v1::ResourceNamespace;

    #[test]
    fn resource_lane_block_detection_uses_typed_store_error() {
        let lane_key = store::ResourceLaneKey {
            namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
            key: events::ResourceKey::new("wallet-1").expect("resource key"),
        };
        let holder = store::SideEffectLedgerRef::new(
            RunId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x7a; 32]),
            ),
            events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key"),
        );

        let typed = store::StoreError::ResourceLaneBlocked {
            lane_key: Box::new(lane_key),
            holder: Box::new(holder),
        };
        assert!(store_error_is_resource_lane_block(&typed));
        assert!(async_error_is_resource_lane_block(&typed));

        let prose = store::StoreError::ProjectionConflict {
            key: "resource_lane:mfm.test.account_nonce:wallet-1".to_owned(),
            message: "resource lane already held by a different display message".to_owned(),
        };
        assert!(!store_error_is_resource_lane_block(&prose));
        assert!(!async_error_is_resource_lane_block(&prose));
    }
}
