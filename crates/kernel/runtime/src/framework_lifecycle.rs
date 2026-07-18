use mfm_events::v1 as events;
use mfm_ids::{AttemptId, RunId};
use mfm_manual_auth::VerifiedManualResolutionForPrefix;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::RuntimeArtifactStore;
use crate::attempt::{
    async_error_is_stale_expected_next_seq, terminalize_observed_failure, AttemptRunStatus,
    ObservedFailureContext, ObservedFailureRetryabilityPolicy,
};
use crate::binding::BoundRuntimeContext;
use crate::commit::{CommitPlanner, RunnerOutputCommitInput};
use crate::error::async_store_error;
use crate::history::RuntimeRunView;
use crate::invocation::{ErasedRunCtx, InvocationBuilder, InvocationBuilderInput};
use crate::manual_resolution::{
    certified_manual_resolution_spec, verify_manual_resolution_for_prefix,
    ManualResolutionEvidenceArtifact,
};
use crate::transition::TransitionAttempt;
use crate::{attempt_id, CertifiedRuntimeSpec, Result, RuntimeError};

/// Lifecycle for framework nodes that append `StateAttemptStarted` before running.
pub(crate) struct FrameworkAttemptLifecycle<'a> {
    artifact_store: &'a dyn RuntimeArtifactStore,
}

impl<'a> FrameworkAttemptLifecycle<'a> {
    /// Creates a framework attempt lifecycle over runtime-owned artifact storage.
    pub(crate) fn new(artifact_store: &'a dyn RuntimeArtifactStore) -> Self {
        Self { artifact_store }
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
    pub(crate) async fn run<S: store::RunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        bound_context: &BoundRuntimeContext,
        attempt: TransitionAttempt<'_>,
    ) -> Result<AttemptRunStatus> {
        let FrameworkAttempt {
            node,
            selected_attempt_id,
            attempt_no,
            descriptor,
            output_cell,
            binding,
        } = self.prepare(runtime_spec, bound_context, attempt)?;
        let attempt_id = match selected_attempt_id {
            Some(attempt_id) => attempt_id,
            None => {
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                let start_commit = CommitPlanner::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                let bundle = store::PreparedCommitBundle::without_artifacts(start_commit.into())?;
                match store.append_prepared_commit_bundle(bundle).await {
                    Ok(_) => {}
                    Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                        return Ok(AttemptRunStatus::StaleView);
                    }
                    Err(error) => return Err(async_store_error(error)),
                }
                attempt_id
            }
        };
        let latest_committed = store
            .load_committed_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let latest_view = RuntimeRunView::from_committed_stream(runtime_spec, &latest_committed)?;
        let failure_context = ObservedFailureContext {
            runtime_spec,
            run_id,
            node,
            attempt_id: &attempt_id,
            view: &latest_view,
            retryability: ObservedFailureRetryabilityPolicy::for_attempt(runtime_spec, node),
        };
        let terminal_output = match self
            .prepare_terminal_output(FrameworkTerminalOutputInput {
                runtime_spec,
                run_id,
                node,
                descriptor,
                output_cell,
                binding,
                attempt_id: &attempt_id,
                attempt_no,
                latest_view: &latest_view,
            })
            .await
        {
            Ok(output) => output,
            Err(RuntimeError::Blocked(_)) => {
                return Ok(AttemptRunStatus::OperationalBlock);
            }
            Err(error) => {
                return terminalize_observed_failure(store, failure_context, error).await;
            }
        };
        let bundle = terminal_output.into_prepared_commit_bundle()?;
        match store.append_prepared_commit_bundle(bundle).await {
            Ok(_) => Ok(AttemptRunStatus::Advanced),
            Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                Ok(AttemptRunStatus::StaleView)
            }
            Err(error) => Err(async_store_error(error)),
        }
    }

    fn prepare<'b>(
        &self,
        runtime_spec: &'b CertifiedRuntimeSpec,
        bound_context: &BoundRuntimeContext,
        attempt: TransitionAttempt<'b>,
    ) -> Result<FrameworkAttempt<'b>> {
        let TransitionAttempt {
            node,
            attempt_id: selected_attempt_id,
            attempt_no,
        } = attempt;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = bound_context.runner_binding_for(node)?;
        Ok(FrameworkAttempt {
            node,
            selected_attempt_id,
            attempt_no,
            descriptor,
            output_cell,
            binding,
        })
    }

    async fn prepare_terminal_output(
        &self,
        input: FrameworkTerminalOutputInput<'_>,
    ) -> Result<crate::commit::PreparedRunnerOutput> {
        let FrameworkTerminalOutputInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            binding,
            attempt_id,
            attempt_no,
            latest_view,
        } = input;
        let invocation = InvocationBuilder::new(InvocationBuilderInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id,
            attempt_no,
            view: latest_view,
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
            Some(
                self.saga_terminal_proof_for_view(runtime_spec, run_id, latest_view)
                    .await?,
            )
        } else {
            None
        };
        CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id,
            caps: invocation.caps(),
            recorded_facts: invocation.recorded_facts(),
            view: latest_view,
            context_output_extractor: binding.runner.context_output_extractor(),
            saga_terminal_proof: proof,
            output,
        })
    }

    async fn verified_manual_resolution_for_terminal(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
    ) -> Result<Option<VerifiedManualResolutionForPrefix>> {
        let stream = &view.stream;
        let Some((manual_index, manual_payload)) =
            stream
                .iter()
                .enumerate()
                .find_map(|(index, event)| match event.payload() {
                    events::KernelEventPayload::ManualResolutionRecorded(payload)
                        if &payload.run_id == run_id =>
                    {
                        Some((index, payload))
                    }
                    _ => None,
                })
        else {
            return Ok(None);
        };

        let prefix_stream = stream.get(..manual_index).ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "manual resolution prefix index was outside the run stream".to_owned(),
            )
        })?;
        let prefix_projection =
            store::ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(
                prefix_stream,
                &view.artifact_byte_authority,
            )?;
        let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
        let prefix_saga = prefix_projection.derive_saga_projection(
            run_id,
            &runtime_spec.spec().saga,
            &terminal_policies,
        )?;
        if prefix_saga.run_mode != store::RunMode::ManualBlocked {
            return Err(RuntimeError::InvalidRunStream(format!(
                "manual resolution prefix requires ManualBlocked saga mode, found {}",
                prefix_saga.run_mode.as_str()
            )));
        }
        let reason = prefix_saga.manual_block_reason.ok_or_else(|| {
            RuntimeError::InvalidRunStream("manual resolution prefix lacks block reason".to_owned())
        })?;
        let manual = certified_manual_resolution_spec(&runtime_spec.spec().saga)?.clone();
        let prefix = mfm_manual_auth::ManualResolutionPrefixAuthority::new(
            run_id.clone(),
            runtime_spec.spec_hash().clone(),
            stream[manual_index].seq().as_u64(),
            crate::manual_resolution::manual_resolution_stream_prefix_digest(prefix_stream)?,
            crate::manual_resolution::manual_resolution_block_reason(reason),
            crate::manual_resolution::unresolved_manual_obligations_digest(&prefix_saga)?,
            manual,
        )
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;

        let requirements =
            events::KernelEventPayload::ManualResolutionRecorded(manual_payload.clone())
                .artifact_requirements();
        let evidence_requirement = requirements
            .iter()
            .find(|requirement| requirement.artifact_id == manual_payload.evidence_artifact_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::InvalidRunStream(
                    "manual resolution evidence artifact requirement was missing".to_owned(),
                )
            })?;
        let authorization_requirement = requirements
            .iter()
            .find(|requirement| requirement.artifact_id == manual_payload.authorization_artifact_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::InvalidRunStream(
                    "manual resolution authorization artifact requirement was missing".to_owned(),
                )
            })?;
        let evidence = self
            .artifact_store
            .read_retained_artifact(&evidence_requirement)
            .await?;
        let authorization = self
            .artifact_store
            .read_retained_artifact(&authorization_requirement)
            .await?;
        let evidence_media_type = evidence.evidence().media_type.clone();
        let evidence_artifact = ManualResolutionEvidenceArtifact {
            bytes: evidence.into_bytes(),
            media_type: evidence_media_type,
        };
        let verified = verify_manual_resolution_for_prefix(
            prefix,
            manual_payload.outcome,
            &evidence_artifact,
            authorization.into_bytes(),
        )?;
        Ok(Some(verified))
    }

    async fn saga_terminal_proof_for_view(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
    ) -> Result<store::SagaTerminalProof> {
        let manual = self
            .verified_manual_resolution_for_terminal(runtime_spec, run_id, view)
            .await?;
        crate::framework::saga_terminal_proof(
            runtime_spec,
            run_id,
            &view.projections,
            view.next_seq,
            manual,
        )
    }
}

struct FrameworkAttempt<'a> {
    node: &'a spec::NodeSpec,
    selected_attempt_id: Option<AttemptId>,
    attempt_no: u32,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    binding: crate::runners::ErasedRunnerBinding,
}

struct FrameworkTerminalOutputInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    binding: crate::runners::ErasedRunnerBinding,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    latest_view: &'a RuntimeRunView,
}
