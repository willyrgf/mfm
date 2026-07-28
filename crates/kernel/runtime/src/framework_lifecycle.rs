use mfm_ids::{AttemptId, NodeId, RunId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::attempt::{
    async_error_is_stale_expected_next_seq, terminalize_observed_failure, AttemptRunResult,
    AttemptRunStatus, ObservedFailureContext, ObservedFailureRetryabilityPolicy,
};
use crate::binding::BoundRuntimeContext;
use crate::commit::{CommitPlanner, RunnerOutputCommitInput};
use crate::error::async_store_error;
use crate::history::{
    refresh_current_after_commit, refresh_current_after_stale_append, VerifiedCurrentRun,
};
use crate::invocation::{ErasedRunCtx, InvocationBuilder, InvocationBuilderInput};
use crate::spec_authority::CurrentRuntimeSpecRef;
use crate::transition::TransitionAttempt;
use crate::{attempt_id, Result, RuntimeError};

/// Lifecycle for framework nodes that append `StateAttemptStarted` before running.
pub(crate) struct FrameworkAttemptLifecycle {}

impl FrameworkAttemptLifecycle {
    /// Creates a framework attempt lifecycle.
    pub(crate) fn new() -> Self {
        Self {}
    }

    /// Returns whether this certified node uses the framework started-before-run lifecycle.
    pub(crate) fn owns_node(node: &spec::NodeSpec) -> bool {
        matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::PublicOutputRender(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
                    | spec::FrameworkNodeSpec::ResolveSagaTerminal(_)
                    | spec::FrameworkNodeSpec::SideEffectVerify(_)
            )
        )
    }

    /// Runs one framework attempt against an async typed store.
    pub(crate) async fn run<S: store::RunJournalStore + ?Sized>(
        &self,
        store: &S,
        mut current: VerifiedCurrentRun,
        bound_context: &BoundRuntimeContext,
        attempt: TransitionAttempt,
    ) -> Result<AttemptRunResult> {
        let TransitionAttempt {
            node_id,
            attempt_id: selected_attempt_id,
            attempt_no,
        } = attempt;
        let run_id = current.view().run_id().clone();
        let binding = {
            let runtime_spec = current.runtime_spec();
            let node = certified_node(&runtime_spec, &node_id)?;
            bound_context.runner_binding_for(node)?
        };
        let attempt_id = match selected_attempt_id {
            Some(attempt_id) => attempt_id,
            None => {
                let (attempt_id, start_commit) = {
                    let runtime_spec = current.runtime_spec();
                    let lifecycle = current.lifecycle();
                    let node = certified_node(&runtime_spec, &node_id)?;
                    let attempt_id =
                        attempt_id(&run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                    let start_commit = CommitPlanner::prepare_attempt_start(
                        &runtime_spec,
                        &run_id,
                        node,
                        &attempt_id,
                        attempt_no,
                        &lifecycle,
                    )?;
                    (attempt_id, start_commit)
                };
                let bundle = store::PreparedCommitBundle::without_artifacts(start_commit.into())?;
                let outcome = match store.append_prepared_commit_bundle(bundle).await {
                    Ok(outcome) => outcome,
                    Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                        current = refresh_current_after_stale_append(store, current).await?;
                        return Ok(AttemptRunResult::new(current, AttemptRunStatus::StaleView));
                    }
                    Err(error) => return Err(async_store_error(error)),
                };
                match &outcome {
                    store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_) => {}
                    store::CommitOutcome::AdmissionBlocked(block) => {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "framework attempt start for node {node_id} was blocked by lane {}:{}",
                            block.resource_lane_key.namespace, block.resource_lane_key.key
                        )));
                    }
                    store::CommitOutcome::ExecutionClaimBusy(_) => {
                        return Err(unexpected_execution_claim_busy("framework attempt start"));
                    }
                }
                current = refresh_current_after_commit(store, current, &outcome).await?;
                attempt_id
            }
        };

        let retryability = {
            let runtime_spec = current.runtime_spec();
            let node = certified_node(&runtime_spec, &node_id)?;
            ObservedFailureRetryabilityPolicy::for_attempt(&runtime_spec, node)
        };
        let failure_context = ObservedFailureContext {
            run_id: &run_id,
            node_id: &node_id,
            attempt_id: &attempt_id,
            retryability,
        };
        let terminal_output = match self
            .prepare_terminal_output(
                &current,
                &run_id,
                &node_id,
                &binding,
                &attempt_id,
                attempt_no,
            )
            .await
        {
            Ok(output) => output,
            Err(RuntimeError::Blocked(_)) => {
                return Ok(AttemptRunResult::new(
                    current,
                    AttemptRunStatus::OperationalBlock,
                ));
            }
            Err(error) => {
                return terminalize_observed_failure(store, current, failure_context, error).await;
            }
        };
        let (bundle, settlement) = terminal_output.into_prepared_commit_bundle()?;
        let outcome = match store.append_prepared_commit_bundle(bundle).await {
            Ok(outcome) => outcome,
            Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                current = refresh_current_after_stale_append(store, current).await?;
                return Ok(AttemptRunResult::new(current, AttemptRunStatus::StaleView));
            }
            Err(error) => return Err(async_store_error(error)),
        };
        if matches!(&outcome, store::CommitOutcome::Appended(_)) {
            if let Some(settlement) = settlement {
                settlement.settle_appended();
            }
        }
        match &outcome {
            store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_) => {}
            store::CommitOutcome::AdmissionBlocked(block) => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "framework terminal commit for node {node_id} was blocked by lane {}:{}",
                    block.resource_lane_key.namespace, block.resource_lane_key.key
                )));
            }
            store::CommitOutcome::ExecutionClaimBusy(_) => {
                return Err(unexpected_execution_claim_busy(
                    "framework terminal attempt commit",
                ));
            }
        }
        current = refresh_current_after_commit(store, current, &outcome).await?;
        Ok(AttemptRunResult::new(current, AttemptRunStatus::Advanced))
    }

    async fn prepare_terminal_output(
        &self,
        current: &VerifiedCurrentRun,
        run_id: &RunId,
        node_id: &NodeId,
        binding: &crate::runners::ErasedRunnerBinding,
        attempt_id: &AttemptId,
        attempt_no: u32,
    ) -> Result<crate::commit::PreparedRunnerOutput> {
        let runtime_spec = current.runtime_spec();
        let lifecycle = current.lifecycle();
        let node = certified_node(&runtime_spec, node_id)?;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let invocation = InvocationBuilder::new(InvocationBuilderInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id,
            attempt_no,
            lifecycle,
        })
        .build()?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let proof = if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) {
            Some(self.saga_terminal_proof_for_current(current)?)
        } else {
            None
        };
        CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec: invocation.runtime_spec(),
            run_id,
            node,
            attempt_id,
            caps: invocation.caps(),
            lifecycle: current.lifecycle(),
            context_output_extractor: binding.runner.context_output_extractor(),
            saga_terminal_proof: proof,
            output,
        })
    }

    fn saga_terminal_proof_for_current(
        &self,
        current: &VerifiedCurrentRun,
    ) -> Result<store::SagaTerminalProof> {
        let lifecycle = current.lifecycle();
        lifecycle
            .saga_terminal_proof(lifecycle.next_sequence()?)
            .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))
    }
}

fn certified_node<'a>(
    runtime_spec: &CurrentRuntimeSpecRef<'a>,
    node_id: &NodeId,
) -> Result<&'a spec::NodeSpec> {
    runtime_spec.node(node_id).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!("selected certified node {node_id} is missing"))
    })
}

fn unexpected_execution_claim_busy(context: &str) -> RuntimeError {
    RuntimeError::InvalidRunStream(format!(
        "{context} requested a run execution claim outside run admission"
    ))
}
