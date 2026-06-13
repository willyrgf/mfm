use std::sync::Arc;

use mfm_ids::{AttemptId, RunId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::RuntimeArtifactStager;
use crate::commit::{
    CommitPlanner, PreparedRunLaunch, PreparedStagedArtifact, RunLaunchEvidence,
    RunnerOutputCommitInput,
};
use crate::error::async_store_error;
use crate::frontier::{scheduler_decision, AttemptPlan, RunnableNode, SchedulerDecision};
use crate::history::{
    committed_config_artifact, materialize_inputs, recorded_facts_for_attempt, RuntimeRunView,
};
use crate::invocation::{CertifiedRuntimeCapabilities, ErasedRunCtx, PreparedRunnerInvocation};
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

    /// Runs one deterministic runnable node, if any.
    pub async fn drive_once<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let view = RuntimeRunView::from_store(runtime_spec, run_id, store)?;
        match scheduler_decision(runtime_spec, run_id, &view)? {
            SchedulerDecision::Run(runnable) => {
                self.run_node_attempt(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                Ok(SchedulerStatus::Advanced)
            }
            SchedulerDecision::Blocked => Ok(SchedulerStatus::Blocked),
            SchedulerDecision::Completed => Ok(SchedulerStatus::PublicOutputProjected),
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
        loop {
            match self.drive_once(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
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
        let stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let view = RuntimeRunView::from_stream(runtime_spec, run_id, &stream)?;
        match scheduler_decision(runtime_spec, run_id, &view)? {
            SchedulerDecision::Run(runnable) => {
                self.run_node_attempt_async(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                Ok(SchedulerStatus::Advanced)
            }
            SchedulerDecision::Blocked => Ok(SchedulerStatus::Blocked),
            SchedulerDecision::Completed => Ok(SchedulerStatus::PublicOutputProjected),
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
        loop {
            match self.drive_once_async(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    async fn run_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
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
            self.run_started_framework_node_attempt(
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
            .await?;
            return Ok(());
        }
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
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<()> {
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
        Ok(())
    }

    async fn run_node_attempt_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
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
            self.run_started_framework_node_attempt_async(
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
            .await?;
            return Ok(());
        }
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
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt_async<
        S: store::AsyncTypedRunEventStore + ?Sized,
    >(
        &self,
        store: &S,
        input: FrameworkNodeAttemptInput<'_>,
    ) -> Result<()> {
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
        Ok(())
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
