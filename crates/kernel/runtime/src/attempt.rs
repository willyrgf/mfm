use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, DigestAlgorithm, NodeId, RunId, SchemaId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::binding::BoundRuntimeContext;
use crate::commit::{
    AttemptFailureCommitInput, CommitPlanner, PreparedStagedArtifact, RunnerOutputCommitInput,
};
use crate::error::async_store_error;
use crate::history::RuntimeRunView;
use crate::invocation::{ErasedRunCtx, InvocationBuilder, InvocationBuilderInput};
use crate::side_effect_lifecycle::side_effect_projection_for_attempt;
use crate::transition::TransitionAttempt;
use crate::{
    attempt_id, canonical_json, CertifiedRuntimeSpec, Result, RuntimeDiagnostic, RuntimeError,
    REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_SCHEMA, REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_VERSION,
};

/// Result of running one ordinary attempt lifecycle.
pub(crate) enum AttemptRunStatus {
    /// Attempt advanced and committed terminal state.
    Advanced,
    /// The loaded view was stale; reload verified history and re-decide.
    StaleView,
    /// Attempt was blocked by an exclusive resource lane.
    BlockedOnResourceLane {
        /// Scoped witness proving which resource lane blocked the node.
        witness: Box<ResourceLaneBlockWitness>,
        /// Whether the attempt-start commit advanced before the lane block.
        advanced: bool,
    },
    /// Recovery cannot safely continue without operational intervention.
    OperationalBlock,
}

/// Scoped resource-lane block carried between scheduler decisions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ResourceLaneBlockWitness {
    /// Node whose attempt observed the lane block.
    pub(crate) node_id: NodeId,
    /// Resource lane that blocked the attempt.
    pub(crate) lane_key: store::ResourceLaneKey,
    /// Non-authoritative waiter id, when the store exposed one.
    pub(crate) waiter_id: Option<String>,
    /// Lane-local waiter ticket, when the store exposed one.
    pub(crate) lane_ticket: Option<u64>,
}

impl ResourceLaneBlockWitness {
    /// Returns true when this witness prevents starting or recovering the node.
    pub(crate) fn blocks_node(
        &self,
        projections: &store::ProjectionSnapshot,
        node: &spec::NodeSpec,
    ) -> bool {
        if node.node_id.as_str() == self.node_id.as_str() {
            return true;
        }
        if !node_resource_claims_lane_namespace(node, &self.lane_key) {
            return false;
        }
        projected_started_attempt_active_lane_key(projections, node)
            .map(|lane_key| lane_key == self.lane_key)
            .unwrap_or(false)
    }
}

/// Ordinary node-attempt lifecycle.
///
/// This owns selected/started/invoked/terminal-planned/terminal-committed orchestration for
/// non-framework-special attempts. New attempts append `StateAttemptStarted` before sealed runner
/// invocation construction, so observed materialization and runner-output validation failures can
/// terminalize through runtime-owned failure-safe evidence.
pub(crate) struct AttemptLifecycle;

/// Trusted context for terminalizing an observed post-start attempt failure.
#[derive(Clone, Copy)]
pub(crate) struct ObservedFailureContext<'a> {
    /// Certified runtime spec for the attempt.
    pub(crate) runtime_spec: &'a CertifiedRuntimeSpec,
    /// Run id for the attempt.
    pub(crate) run_id: &'a RunId,
    /// Attempted node.
    pub(crate) node: &'a spec::NodeSpec,
    /// Started attempt id.
    pub(crate) attempt_id: &'a AttemptId,
    /// Verified view after the attempt start commit.
    pub(crate) view: &'a RuntimeRunView,
    /// Trusted retryability policy derived from certified runtime authority.
    pub(crate) retryability: ObservedFailureRetryabilityPolicy,
}

/// Trusted retryability policy for runtime-owned failure-safe terminalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ObservedFailureRetryabilityPolicy {
    input_materialization_retryable: bool,
}

impl ObservedFailureRetryabilityPolicy {
    /// Derives retryability policy from certified saga and node authority.
    pub(crate) fn for_attempt(runtime_spec: &CertifiedRuntimeSpec, node: &spec::NodeSpec) -> Self {
        Self {
            input_materialization_retryable: matches!(
                &runtime_spec.spec().saga,
                spec::SagaPolicySpec::NoSideEffects
            ) && node.side_effect.is_none(),
        }
    }

    fn retryable_for(self, failure_class: ObservedFailureClass) -> bool {
        match failure_class {
            ObservedFailureClass::InputMaterialization => self.input_materialization_retryable,
            ObservedFailureClass::InvalidRunnerOutput | ObservedFailureClass::RuntimeValidation => {
                false
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservedFailureClass {
    InputMaterialization,
    InvalidRunnerOutput,
    RuntimeValidation,
}

impl AttemptLifecycle {
    /// Creates a lifecycle bound to runtime-owned append planning.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Runs one ordinary attempt against an async typed store.
    pub(crate) async fn run<S: store::RunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        bound_context: &BoundRuntimeContext,
        attempt: TransitionAttempt<'_>,
    ) -> Result<AttemptRunStatus> {
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
        let mut advanced = false;
        let (attempt_id, attempt_no) = match selected_attempt_id {
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
                    Ok(store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_)) => {
                    }
                    Ok(store::CommitOutcome::AdmissionBlocked(block)) => {
                        let holder = block
                            .holder
                            .as_ref()
                            .map(|holder| {
                                format!(" held by run {} pair {}", holder.run_id, holder.pair_id)
                            })
                            .unwrap_or_else(|| " with no active holder".to_owned());
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "attempt start for node {} was blocked by resource lane {}:{}{}",
                            node.node_id,
                            block.resource_lane_key.namespace,
                            block.resource_lane_key.key,
                            holder
                        )));
                    }
                    Ok(store::CommitOutcome::ExecutionClaimBusy(_)) => {
                        return Err(unexpected_execution_claim_busy("attempt start"));
                    }
                    Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                        return Ok(AttemptRunStatus::StaleView);
                    }
                    Err(error) => return Err(async_store_error(error)),
                }
                advanced = true;
                (attempt_id, attempt_no)
            }
            Some(attempt_id) => (attempt_id, attempt_no),
        };

        let mut latest_view = load_runtime_run_view(runtime_spec, store, run_id).await?;
        if node_needs_pre_invocation_lane_claim(
            runtime_spec,
            run_id,
            &latest_view.projections,
            node,
            &attempt_id,
        )? {
            {
                let pre_invocation = InvocationBuilder::new(InvocationBuilderInput {
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    attempt_id: &attempt_id,
                    attempt_no,
                    view: &latest_view,
                })
                .build_pre_invocation()
                .map_err(|error| {
                    RuntimeError::InvalidRunStream(format!(
                        "failed to build resource-lane preflight context for node {}: {error}",
                        node.node_id
                    ))
                })?;
                let output = binding
                    .runner
                    .preclaim_resource_lane(&pre_invocation)
                    .await?;
                let preclaim = CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
                    runtime_spec,
                    run_id,
                    node,
                    attempt_id: &attempt_id,
                    caps: pre_invocation.caps(),
                    recorded_facts: pre_invocation.recorded_facts(),
                    view: &latest_view,
                    context_output_extractor: None,
                    saga_terminal_proof: None,
                    output,
                })?;
                if !request_has_resource_lane_claim(preclaim.request()) {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "exclusive side-effect node {} did not emit pre-invocation resource-lane claim",
                        node.node_id
                    )));
                }
                let bundle = preclaim.into_prepared_commit_bundle()?;
                match store.append_prepared_commit_bundle(bundle).await {
                    Ok(store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_)) => {
                        advanced = true;
                    }
                    Ok(store::CommitOutcome::AdmissionBlocked(block)) => {
                        return Ok(AttemptRunStatus::BlockedOnResourceLane {
                            witness: Box::new(resource_lane_block_witness_from_outcome(
                                &node.node_id,
                                *block,
                            )),
                            advanced,
                        });
                    }
                    Ok(store::CommitOutcome::ExecutionClaimBusy(_)) => {
                        return Err(unexpected_execution_claim_busy(
                            "resource-lane pre-invocation commit",
                        ));
                    }
                    Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                        return Ok(AttemptRunStatus::StaleView);
                    }
                    Err(error) => return Err(async_store_error(error)),
                }
            }
            latest_view = load_runtime_run_view(runtime_spec, store, run_id).await?;
        }
        let failure_context = ObservedFailureContext {
            runtime_spec,
            run_id,
            node,
            attempt_id: &attempt_id,
            view: &latest_view,
            retryability: ObservedFailureRetryabilityPolicy::for_attempt(runtime_spec, node),
        };
        let invocation = match InvocationBuilder::new(InvocationBuilderInput {
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no,
            view: &latest_view,
        })
        .build()
        {
            Ok(invocation) => invocation,
            Err(error) => {
                return terminalize_observed_failure(store, failure_context, error).await;
            }
        };
        let output = match binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
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
        let terminal_output = match CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id: &attempt_id,
            caps: invocation.caps(),
            recorded_facts: invocation.recorded_facts(),
            view: &latest_view,
            context_output_extractor: binding.runner.context_output_extractor(),
            saga_terminal_proof: None,
            output,
        }) {
            Ok(output) => output,
            Err(error) => {
                return terminalize_observed_failure(store, failure_context, error).await;
            }
        };
        let lane_projection = store
            .status_projection_snapshot(run_id)
            .await
            .map_err(async_store_error)?;
        if let Some(witness) =
            resource_lane_block_for_request(&lane_projection, terminal_output.request())
        {
            return Ok(AttemptRunStatus::BlockedOnResourceLane {
                witness: Box::new(witness),
                advanced,
            });
        }
        let has_resource_lane_claim = request_has_resource_lane_claim(terminal_output.request());
        let bundle = terminal_output.into_prepared_commit_bundle()?;
        match store.append_prepared_commit_bundle(bundle).await {
            Ok(store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_)) => {
                Ok(AttemptRunStatus::Advanced)
            }
            Ok(store::CommitOutcome::AdmissionBlocked(block)) if has_resource_lane_claim => {
                Ok(AttemptRunStatus::BlockedOnResourceLane {
                    witness: Box::new(resource_lane_block_witness_from_outcome(
                        &node.node_id,
                        *block,
                    )),
                    advanced,
                })
            }
            Ok(store::CommitOutcome::AdmissionBlocked(block)) => {
                Err(RuntimeError::InvalidRunStream(format!(
                    "commit without resource-lane claim was blocked by lane {}:{}",
                    block.resource_lane_key.namespace, block.resource_lane_key.key
                )))
            }
            Ok(store::CommitOutcome::ExecutionClaimBusy(_)) => {
                Err(unexpected_execution_claim_busy("terminal attempt commit"))
            }
            Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                Ok(AttemptRunStatus::StaleView)
            }
            Err(error) => Err(async_store_error(error)),
        }
    }
}

pub(crate) async fn terminalize_observed_failure<S: store::RunEventStore + ?Sized>(
    store: &S,
    context: ObservedFailureContext<'_>,
    error: RuntimeError,
) -> Result<AttemptRunStatus> {
    let ObservedFailureContext {
        runtime_spec,
        run_id,
        node,
        attempt_id,
        view,
        retryability,
    } = context;
    let Some(failure_info) = observed_attempt_failure_info(&error, retryability)? else {
        return Err(error);
    };
    if !can_terminalize_observed_failure(runtime_spec, run_id, node, attempt_id, view)? {
        return Err(error);
    }
    let diagnostic_artifact = redacted_attempt_failure_diagnostic_artifact(
        runtime_spec,
        run_id,
        node,
        attempt_id,
        failure_info.diagnostic.as_ref(),
    )?;
    let failure = CommitPlanner::prepare_attempt_failure(AttemptFailureCommitInput {
        runtime_spec,
        run_id,
        node,
        attempt_id,
        view,
        error: failure_info.error,
        diagnostic_artifact: Some(diagnostic_artifact),
    })?;
    let bundle = failure.into_prepared_commit_bundle()?;
    match store.append_prepared_commit_bundle(bundle).await {
        Ok(store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_)) => {
            Ok(AttemptRunStatus::Advanced)
        }
        Ok(store::CommitOutcome::AdmissionBlocked(block)) => {
            Err(RuntimeError::InvalidRunStream(format!(
                "attempt failure commit was blocked by lane {}:{}",
                block.resource_lane_key.namespace, block.resource_lane_key.key
            )))
        }
        Ok(store::CommitOutcome::ExecutionClaimBusy(_)) => {
            Err(unexpected_execution_claim_busy("attempt failure commit"))
        }
        Err(error) if async_error_is_stale_expected_next_seq(&error) => {
            Ok(AttemptRunStatus::StaleView)
        }
        Err(error) => Err(async_store_error(error)),
    }
}

fn unexpected_execution_claim_busy(context: &str) -> RuntimeError {
    RuntimeError::InvalidRunStream(format!(
        "{context} requested a run execution claim outside run admission"
    ))
}

async fn load_runtime_run_view<S: store::RunEventStore + ?Sized>(
    runtime_spec: &CertifiedRuntimeSpec,
    store: &S,
    run_id: &RunId,
) -> Result<RuntimeRunView> {
    let committed = store
        .load_committed_run_stream(run_id)
        .await
        .map_err(async_store_error)?;
    let authority = store
        .status_projection_snapshot(run_id)
        .await
        .map_err(async_store_error)?;
    RuntimeRunView::from_committed_stream(runtime_spec, &committed)?
        .with_status_authority(&authority)
}

fn can_terminalize_observed_failure(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    view: &RuntimeRunView,
) -> Result<bool> {
    let Some(attempt) = view.projections.attempt(&node.node_id, attempt_id) else {
        return Ok(false);
    };
    if !matches!(attempt.status, store::AttemptStatus::Started { .. }) {
        return Ok(false);
    }
    if node.side_effect.is_some() {
        let Some(side_effect) = side_effect_projection_for_attempt(
            runtime_spec,
            run_id,
            &view.projections,
            node,
            attempt_id,
        )?
        else {
            return Ok(true);
        };
        return Ok(matches!(
            side_effect.phase,
            store::SideEffectPhase::Claimed { .. }
        ));
    }
    if matches!(
        node.framework,
        Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
    ) {
        return Ok(false);
    }
    Ok(true)
}

struct ObservedAttemptFailureInfo {
    error: events::MfmErrorInfo,
    diagnostic: Option<RuntimeDiagnostic>,
}

fn observed_attempt_failure_info(
    error: &RuntimeError,
    retryability: ObservedFailureRetryabilityPolicy,
) -> Result<Option<ObservedAttemptFailureInfo>> {
    let Some(failure_class) = observed_failure_class(error) else {
        return Ok(None);
    };
    let retryable = retryability.retryable_for(failure_class);
    let diagnostic = observed_failure_diagnostic(error).cloned();
    let failure = match failure_class {
        ObservedFailureClass::InputMaterialization => events::MfmErrorInfo::new(
            events::ErrorCode::new("input_materialization_failed")?,
            events::ErrorCategory::Validation,
            retryable,
            "attempt input materialization failed",
        )?,
        ObservedFailureClass::InvalidRunnerOutput => match error {
            RuntimeError::InvalidRunnerOutputFailure { failure, .. } => {
                failure.public_error(retryable)?
            }
            RuntimeError::InvalidRunnerOutput(_) => events::MfmErrorInfo::new(
                events::ErrorCode::new("runner_output_invalid")?,
                events::ErrorCategory::Validation,
                retryable,
                "runner output failed validation",
            )?,
            _ => unreachable!("failure class must match runtime error"),
        },
        ObservedFailureClass::RuntimeValidation => events::MfmErrorInfo::new(
            events::ErrorCode::new("runtime_validation_failed")?,
            events::ErrorCategory::Validation,
            retryable,
            "runtime validation failed while handling attempt",
        )?,
    };
    let error = if let Some(diagnostic) = &diagnostic {
        let details_digest = canonical_json(diagnostic.public_details_json())?.content_digest();
        failure.with_public_details(events::RedactedJson::new(details_digest))?
    } else {
        failure
    };
    Ok(Some(ObservedAttemptFailureInfo { error, diagnostic }))
}

fn observed_failure_class(error: &RuntimeError) -> Option<ObservedFailureClass> {
    match error {
        RuntimeError::InputMaterialization(_) => Some(ObservedFailureClass::InputMaterialization),
        RuntimeError::InvalidRunnerOutput(_) | RuntimeError::InvalidRunnerOutputFailure { .. } => {
            Some(ObservedFailureClass::InvalidRunnerOutput)
        }
        RuntimeError::RuntimeValidation(_) => Some(ObservedFailureClass::RuntimeValidation),
        _ => None,
    }
}

fn observed_failure_diagnostic(error: &RuntimeError) -> Option<&RuntimeDiagnostic> {
    match error {
        RuntimeError::InvalidRunnerOutputFailure { diagnostic, .. } => diagnostic.as_deref(),
        _ => None,
    }
}

fn redacted_attempt_failure_diagnostic_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    diagnostic: Option<&RuntimeDiagnostic>,
) -> Result<PreparedStagedArtifact> {
    let bytes = canonical_json(serde_json::json!({
        "attempt_id": attempt_id.as_str(),
        "diagnostic": diagnostic.map(RuntimeDiagnostic::to_json),
        "diagnostic_schema": REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_SCHEMA,
        "diagnostic_schema_version": REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_VERSION,
        "node_id": node.node_id.as_str(),
        "run_id": run_id.as_str(),
        "spec_hash": runtime_spec.spec_hash().as_str(),
    }))?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(redacted_attempt_failure_diagnostic_schema_id()?),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::RedactedDiagnostic,
    };
    Ok(PreparedStagedArtifact {
        bytes: bytes.as_bytes().to_vec(),
        evidence,
    })
}

fn redacted_attempt_failure_diagnostic_schema_id() -> Result<SchemaId> {
    let digest = canonical_json(serde_json::json!({
        "fields": [
            "attempt_id",
            "diagnostic",
            "diagnostic_schema",
            "diagnostic_schema_version",
            "node_id",
            "run_id",
            "spec_hash"
        ],
        "name": REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_SCHEMA,
        "version": REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_VERSION,
    }))?
    .content_digest();
    Ok(SchemaId::new(
        REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_SCHEMA,
        REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_VERSION,
        DigestAlgorithm::Sha256JcsV1,
        *digest.digest(),
    )?)
}

fn resource_lane_block_for_request(
    projections: &store::ProjectionSnapshot,
    request: &store::CommitRequest,
) -> Option<ResourceLaneBlockWitness> {
    request.payloads().iter().find_map(|payload| {
        let events::KernelEventPayload::ResourceLaneClaimIntent(payload) = payload else {
            return None;
        };
        let lane_key = store::ResourceLaneKey::from_evidence(&payload.resource_key);
        let holder =
            store::SideEffectPairLedgerRef::new(request.run_id().clone(), payload.pair_id.clone());
        projections
            .resource_lane(&lane_key)
            .filter(|projection| projection.holder != holder)
            .map(|_| ResourceLaneBlockWitness {
                node_id: payload.node_id.clone(),
                lane_key,
                waiter_id: None,
                lane_ticket: None,
            })
    })
}

fn request_has_resource_lane_claim(request: &store::CommitRequest) -> bool {
    request.payloads().iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::ResourceLaneClaimIntent(_)
        )
    })
}

fn resource_lane_block_witness_from_outcome(
    node_id: &NodeId,
    block: store::WaitFifoAdmissionBlock,
) -> ResourceLaneBlockWitness {
    ResourceLaneBlockWitness {
        node_id: node_id.clone(),
        lane_key: block.resource_lane_key,
        waiter_id: block
            .waiter
            .as_ref()
            .map(|waiter| waiter.waiter_id.as_str().to_owned()),
        lane_ticket: block.waiter.as_ref().map(|waiter| waiter.lane_ticket),
    }
}

fn node_resource_claims_lane_namespace(
    node: &spec::NodeSpec,
    lane_key: &store::ResourceLaneKey,
) -> bool {
    let Some(side_effect) = &node.side_effect else {
        return false;
    };
    match &side_effect.resource_claim {
        spec::ResourceClaimSpec::Exclusive { namespace, .. }
        | spec::ResourceClaimSpec::ExactTouchedSet { namespace, .. } => {
            namespace == &lane_key.namespace
        }
        spec::ResourceClaimSpec::ManualOnly => false,
    }
}

fn node_needs_pre_invocation_lane_claim(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<bool> {
    if !matches!(
        node.side_effect
            .as_ref()
            .map(|side_effect| &side_effect.resource_claim),
        Some(spec::ResourceClaimSpec::Exclusive { .. })
    ) {
        return Ok(false);
    }
    let Some(projection) =
        side_effect_projection_for_attempt(runtime_spec, run_id, projections, node, attempt_id)?
    else {
        return Ok(true);
    };
    let state = projection
        .ledger_state()
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    Ok(matches!(
        state.phase(),
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::NotSubmitted,
            ..
        }
    ))
}

fn projected_started_attempt_active_lane_key(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
) -> Option<store::ResourceLaneKey> {
    for (lane_key, lane) in projections.resource_lanes() {
        if lane.node_id != node.node_id {
            continue;
        }
        let Some(attempt) = projections.attempt(&node.node_id, &lane.attempt_id) else {
            continue;
        };
        if matches!(attempt.status, store::AttemptStatus::Started { .. }) {
            return Some(lane_key.clone());
        }
    }
    None
}

fn store_error_is_stale_expected_next_seq(error: &store::StoreError) -> bool {
    matches!(error, store::StoreError::StaleExpectedNextSeq { .. })
}

pub(crate) fn async_error_is_stale_expected_next_seq(
    error: &impl store::StoreErrorInspection,
) -> bool {
    error
        .as_store_error()
        .is_some_and(store_error_is_stale_expected_next_seq)
}

#[cfg(test)]
mod tests;
