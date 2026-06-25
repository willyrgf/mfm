use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, DigestAlgorithm, NodeId, RunId, SchemaId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::binding::BoundRuntimeContext;
use crate::commit::{
    prepared_commit_bundle, AttemptFailureCommitInput, CommitPlanner, PreparedRunnerOutput,
    PreparedStagedArtifact, RunnerOutputCommitInput,
};
use crate::error::async_store_error;
use crate::history::RuntimeRunView;
use crate::invocation::{
    ErasedRunCtx, InvocationBuilder, InvocationBuilderInput, PreparedRunnerInvocation,
};
use crate::side_effect_lifecycle::SideEffectLifecycle;
use crate::transition::TransitionAttempt;
use crate::{
    attempt_id, canonical_json, CertifiedRuntimeSpec, ErasedRunnerOutput, Result, RuntimeError,
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
        witness: ResourceLaneBlockWitness,
        /// Whether the attempt-start commit advanced before the lane block.
        advanced: bool,
    },
    /// Recovery cannot safely continue without operational intervention.
    OperationalBlock {
        /// Node whose open attempt is operationally blocked.
        node_id: NodeId,
    },
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
pub(crate) struct AttemptLifecycle<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

struct Attempt<P> {
    phase: P,
}

struct Selected<'a> {
    node: &'a spec::NodeSpec,
    selected_attempt_id: Option<AttemptId>,
    attempt_no: u32,
}

struct Started<'a> {
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    view: &'a RuntimeRunView,
}

struct Invoked<'a> {
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
    view: &'a RuntimeRunView,
    invocation: PreparedRunnerInvocation<'a>,
    output: ErasedRunnerOutput,
}

struct TerminalPlanned {
    terminal_output: PreparedRunnerOutput,
}

struct TerminalCommitted;

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

impl<'a> AttemptLifecycle<'a> {
    /// Creates a lifecycle bound to runtime-owned append planning.
    pub(crate) fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
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
        let selected_attempt = Attempt {
            phase: Selected {
                node,
                selected_attempt_id,
                attempt_no,
            },
        };
        let descriptor = runtime_spec.state_descriptor_for_node(selected_attempt.phase.node)?;
        let output_cell = runtime_spec
            .cell(&selected_attempt.phase.node.output_cell)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "node {} output cell {} is missing",
                    selected_attempt.phase.node.node_id, selected_attempt.phase.node.output_cell
                ))
            })?;
        let binding = bound_context.runner_binding_for(selected_attempt.phase.node)?;
        let mut advanced = false;
        let (attempt_id, attempt_no) = match selected_attempt.phase.selected_attempt_id {
            None => {
                let attempt_id = attempt_id(
                    run_id,
                    runtime_spec.spec_hash(),
                    &selected_attempt.phase.node.node_id,
                    selected_attempt.phase.attempt_no,
                )?;
                let start_commit = CommitPlanner::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    selected_attempt.phase.node,
                    &attempt_id,
                    selected_attempt.phase.attempt_no,
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
                                format!(
                                    " held by run {} ledger {}",
                                    holder.run_id, holder.ledger_key
                                )
                            })
                            .unwrap_or_else(|| " with no active holder".to_owned());
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "attempt start for node {} was blocked by resource lane {}:{}{}",
                            selected_attempt.phase.node.node_id,
                            block.resource_lane_key.namespace,
                            block.resource_lane_key.key,
                            holder
                        )));
                    }
                    Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                        return Ok(AttemptRunStatus::StaleView);
                    }
                    Err(error) => return Err(async_store_error(error)),
                }
                advanced = true;
                (attempt_id, selected_attempt.phase.attempt_no)
            }
            Some(attempt_id) => (attempt_id, selected_attempt.phase.attempt_no),
        };

        let mut latest_stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let mut latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        if node_needs_pre_invocation_lane_claim(
            &latest_view.projections,
            selected_attempt.phase.node,
            &attempt_id,
        )? {
            {
                let pre_invocation = InvocationBuilder::new(InvocationBuilderInput {
                    runtime_spec,
                    run_id,
                    node: selected_attempt.phase.node,
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
                        selected_attempt.phase.node.node_id
                    ))
                })?;
                let output = binding
                    .runner
                    .preclaim_resource_lane(&pre_invocation)
                    .await?;
                let preclaim = CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
                    runtime_spec,
                    run_id,
                    node: selected_attempt.phase.node,
                    attempt_id: &attempt_id,
                    caps: pre_invocation.caps(),
                    recorded_facts: pre_invocation.recorded_facts(),
                    view: &latest_view,
                    saga_terminal_proof: None,
                    output,
                })?;
                if !request_has_resource_lane_claim(preclaim.commit.request()) {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "exclusive side-effect node {} did not emit pre-invocation resource-lane claim",
                        selected_attempt.phase.node.node_id
                    )));
                }
                let bundle = prepared_commit_bundle(preclaim.commit, preclaim.artifact_admissions)?;
                match store.append_prepared_commit_bundle(bundle).await {
                    Ok(store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_)) => {
                        advanced = true;
                    }
                    Ok(store::CommitOutcome::AdmissionBlocked(block)) => {
                        return Ok(AttemptRunStatus::BlockedOnResourceLane {
                            witness: resource_lane_block_witness_from_outcome(
                                &selected_attempt.phase.node.node_id,
                                *block,
                            ),
                            advanced,
                        });
                    }
                    Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                        return Ok(AttemptRunStatus::StaleView);
                    }
                    Err(error) => return Err(async_store_error(error)),
                }
            }
            latest_stream = store
                .load_run_stream(run_id)
                .await
                .map_err(async_store_error)?;
            latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        }
        let started_attempt = Attempt {
            phase: Started {
                node: selected_attempt.phase.node,
                attempt_id: &attempt_id,
                attempt_no,
                view: &latest_view,
            },
        };
        let failure_context = ObservedFailureContext {
            runtime_spec,
            run_id,
            node: started_attempt.phase.node,
            attempt_id: started_attempt.phase.attempt_id,
            view: started_attempt.phase.view,
            retryability: ObservedFailureRetryabilityPolicy::for_attempt(
                runtime_spec,
                started_attempt.phase.node,
            ),
        };
        let invocation = match InvocationBuilder::new(InvocationBuilderInput {
            runtime_spec,
            run_id,
            node: started_attempt.phase.node,
            descriptor,
            output_cell,
            attempt_id: started_attempt.phase.attempt_id,
            attempt_no: started_attempt.phase.attempt_no,
            view: started_attempt.phase.view,
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
            Err(error) => {
                return terminalize_observed_failure(store, failure_context, error).await;
            }
        };
        let invoked_attempt = Attempt {
            phase: Invoked {
                node: started_attempt.phase.node,
                attempt_id: started_attempt.phase.attempt_id,
                view: started_attempt.phase.view,
                invocation,
                output,
            },
        };
        let Invoked {
            node,
            attempt_id,
            view,
            invocation,
            output,
        } = invoked_attempt.phase;
        let terminal_output = match CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
            runtime_spec,
            run_id,
            node,
            attempt_id,
            caps: invocation.caps(),
            recorded_facts: invocation.recorded_facts(),
            view,
            saga_terminal_proof: None,
            output,
        }) {
            Ok(output) => output,
            Err(error) => {
                return terminalize_observed_failure(store, failure_context, error).await;
            }
        };
        let terminal_planned_attempt = Attempt {
            phase: TerminalPlanned { terminal_output },
        };
        let lane_projection = store
            .status_projection_snapshot(run_id)
            .await
            .map_err(async_store_error)?;
        if let Some(witness) = resource_lane_block_for_request(
            &lane_projection,
            terminal_planned_attempt
                .phase
                .terminal_output
                .commit
                .request(),
        ) {
            return Ok(AttemptRunStatus::BlockedOnResourceLane { witness, advanced });
        }
        let has_resource_lane_claim = request_has_resource_lane_claim(
            terminal_planned_attempt
                .phase
                .terminal_output
                .commit
                .request(),
        );
        let terminal_output = terminal_planned_attempt.phase.terminal_output;
        let bundle =
            prepared_commit_bundle(terminal_output.commit, terminal_output.artifact_admissions)?;
        match store.append_prepared_commit_bundle(bundle).await {
            Ok(store::CommitOutcome::Appended(_) | store::CommitOutcome::Idempotent(_)) => {
                let _terminal_committed_attempt = Attempt {
                    phase: TerminalCommitted,
                };
                Ok(AttemptRunStatus::Advanced)
            }
            Ok(store::CommitOutcome::AdmissionBlocked(block)) if has_resource_lane_claim => {
                Ok(AttemptRunStatus::BlockedOnResourceLane {
                    witness: resource_lane_block_witness_from_outcome(&node.node_id, *block),
                    advanced,
                })
            }
            Ok(store::CommitOutcome::AdmissionBlocked(block)) => {
                Err(RuntimeError::InvalidRunStream(format!(
                    "commit without resource-lane claim was blocked by lane {}:{}",
                    block.resource_lane_key.namespace, block.resource_lane_key.key
                )))
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
    let Some(error_info) = observed_attempt_failure_info(&error, retryability)? else {
        return Err(error);
    };
    if !can_terminalize_observed_failure(node, attempt_id, view)? {
        return Err(error);
    }
    let diagnostic_artifact = redacted_attempt_failure_diagnostic_artifact(
        runtime_spec,
        run_id,
        node,
        attempt_id,
        &error_info,
    )?;
    let failure = CommitPlanner::prepare_attempt_failure(AttemptFailureCommitInput {
        runtime_spec,
        run_id,
        node,
        attempt_id,
        view,
        error: error_info,
        diagnostic_artifact: Some(diagnostic_artifact),
    })?;
    let bundle = prepared_commit_bundle(failure.commit, failure.artifact_admissions)?;
    match store.append_prepared_commit_bundle(bundle).await {
        Ok(_) => Ok(AttemptRunStatus::Advanced),
        Err(error) if async_error_is_stale_expected_next_seq(&error) => {
            Ok(AttemptRunStatus::StaleView)
        }
        Err(error) => Err(async_store_error(error)),
    }
}

fn can_terminalize_observed_failure(
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
        let Some(side_effect) =
            SideEffectLifecycle::projection_for_attempt(&view.projections, node, attempt_id)?
        else {
            return Ok(true);
        };
        return Ok(matches!(
            side_effect.phase,
            store::SideEffectPhase::Claimed { .. }
        ));
    }
    Ok(true)
}

fn observed_attempt_failure_info(
    error: &RuntimeError,
    retryability: ObservedFailureRetryabilityPolicy,
) -> Result<Option<events::MfmErrorInfo>> {
    let Some(failure_class) = observed_failure_class(error) else {
        return Ok(None);
    };
    let retryable = retryability.retryable_for(failure_class);
    let failure = match failure_class {
        ObservedFailureClass::InputMaterialization => events::MfmErrorInfo::new(
            events::ErrorCode::new("input_materialization_failed")?,
            events::ErrorCategory::Validation,
            retryable,
            "attempt input materialization failed",
        )?,
        ObservedFailureClass::InvalidRunnerOutput => events::MfmErrorInfo::new(
            events::ErrorCode::new("runner_output_invalid")?,
            events::ErrorCategory::Validation,
            retryable,
            "runner output failed validation",
        )?,
        ObservedFailureClass::RuntimeValidation => events::MfmErrorInfo::new(
            events::ErrorCode::new("runtime_validation_failed")?,
            events::ErrorCategory::Validation,
            retryable,
            "runtime validation failed while handling attempt",
        )?,
    };
    Ok(Some(failure))
}

fn observed_failure_class(error: &RuntimeError) -> Option<ObservedFailureClass> {
    match error {
        RuntimeError::InputMaterialization(_) => Some(ObservedFailureClass::InputMaterialization),
        RuntimeError::InvalidRunnerOutput(_) => Some(ObservedFailureClass::InvalidRunnerOutput),
        RuntimeError::RuntimeValidation(_) => Some(ObservedFailureClass::RuntimeValidation),
        _ => None,
    }
}

fn redacted_attempt_failure_diagnostic_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    error: &events::MfmErrorInfo,
) -> Result<PreparedStagedArtifact> {
    let bytes = canonical_json(serde_json::json!({
        "attempt_id": attempt_id.as_str(),
        "category": store::error_category_str(error.category),
        "code": error.code.as_str(),
        "diagnostic_schema": "mfm.runtime.redacted_attempt_failure_diagnostic",
        "diagnostic_schema_version": "1",
        "node_id": node.node_id.as_str(),
        "retryable": error.retryable,
        "run_id": run_id.as_str(),
        "safe_message": &error.safe_message,
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
            "category",
            "code",
            "diagnostic_schema",
            "diagnostic_schema_version",
            "node_id",
            "retryable",
            "run_id",
            "safe_message",
            "spec_hash"
        ],
        "name": "mfm.runtime.redacted_attempt_failure_diagnostic",
        "version": "1",
    }))?
    .content_digest();
    Ok(SchemaId::new(
        "mfm.runtime.redacted_attempt_failure_diagnostic",
        "1",
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
            store::SideEffectLedgerRef::new(request.run_id().clone(), payload.ledger_key.clone());
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

fn node_requires_pre_invocation_lane_claim(node: &spec::NodeSpec) -> bool {
    matches!(
        node.side_effect
            .as_ref()
            .map(|side_effect| &side_effect.resource_claim),
        Some(spec::ResourceClaimSpec::Exclusive { .. })
    )
}

fn node_needs_pre_invocation_lane_claim(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<bool> {
    if !node_requires_pre_invocation_lane_claim(node) {
        return Ok(false);
    }
    let Some(projection) =
        SideEffectLifecycle::projection_for_attempt(projections, node, attempt_id)?
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

pub(crate) fn store_error_is_stale_expected_next_seq(error: &store::StoreError) -> bool {
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
mod tests {
    use super::*;
    use mfm_capabilities::CapabilitySetDescriptor;
    use mfm_ids::{
        ArtifactId, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EffectKind,
        NodeId, SchemaId, ScopeId, StateKind, StateVersion,
    };
    use mfm_spec::v1::ResourceNamespace;

    #[test]
    fn resource_lane_block_detection_uses_typed_commit_outcome() {
        let lane_key = store::ResourceLaneKey {
            namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
            key_schema_id: SchemaId::new(
                "mfm.test.resource_key",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x7b; 32]),
            )
            .expect("schema id"),
            key: events::ResourceKey::new("wallet-1").expect("resource key"),
        };
        let holder = store::SideEffectLedgerRef::new(
            RunId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x7a; 32]),
            ),
            events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key"),
        );
        let node_id = NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([7; 32]),
        );

        let witness = resource_lane_block_witness_from_outcome(
            &node_id,
            store::WaitFifoAdmissionBlock {
                lane: store::ResourceAdmissionLane::from_resource_lane_key(&lane_key)
                    .expect("resource admission lane"),
                resource_lane_key: lane_key,
                holder: Some(holder),
                waiter: None,
            },
        );
        assert_eq!(witness.node_id, node_id);
        assert_eq!(
            witness.lane_key.namespace,
            ResourceNamespace::new("mfm.test.account_nonce").expect("namespace")
        );
        assert_eq!(
            witness.lane_key.key,
            events::ResourceKey::new("wallet-1").expect("resource key")
        );
    }

    #[test]
    fn resource_lane_block_witness_does_not_block_unstarted_same_namespace_node() {
        let witness = ResourceLaneBlockWitness {
            node_id: node_id(7),
            lane_key: resource_lane_key("wallet-1"),
            waiter_id: None,
            lane_ticket: None,
        };
        let unrelated = exclusive_resource_node(node_id(8));

        assert!(!witness.blocks_node(&store::ProjectionSnapshot::default(), &unrelated));
    }

    fn resource_lane_key(key: &str) -> store::ResourceLaneKey {
        store::ResourceLaneKey {
            namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
            key_schema_id: schema_id(0x7b),
            key: events::ResourceKey::new(key).expect("resource key"),
        }
    }

    fn exclusive_resource_node(node_id: NodeId) -> spec::NodeSpec {
        let schema = schema_id(0x31);
        spec::NodeSpec {
            node_id,
            stable_key: spec::StableAuthorKey::new("exclusive-node").expect("stable key"),
            scope_id: ScopeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x32; 32]),
            ),
            state_kind: StateKind::new(
                "mfm.test",
                "exclusive",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x33; 32]),
            )
            .expect("state kind"),
            state_version: StateVersion::new("mfm.test.exclusive.v1").expect("state version"),
            descriptor_id: DescriptorId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x34; 32]),
            ),
            config_ref: spec::ConfigRef {
                schema_id: schema.clone(),
                artifact_id: ArtifactId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0x35; 32]),
                ),
                digest: content_digest(0x36),
                byte_len: 2,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            input_bindings: spec::InputBindingSpec {
                input_schema_id: schema.clone(),
                input_descriptor_id: DescriptorId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0x37; 32]),
                ),
                root: spec::InputBindingNodeSpec::Unit,
                digest: content_digest(0x38),
            },
            output_cell: CellId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x39; 32]),
            ),
            effect_kind: EffectKind::new(
                "mfm.test",
                "side-effect",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x3a; 32]),
            )
            .expect("effect kind"),
            capability_bindings: CapabilitySetDescriptor::new(Vec::new()).expect("capabilities"),
            adapter_bindings: Vec::new(),
            side_effect: Some(spec::SideEffectContractSpec {
                contract_digest: content_digest(0x3b),
                resource_claim: spec::ResourceClaimSpec::Exclusive {
                    namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
                    key_schema: schema,
                },
            }),
            framework: None,
            planning_lineage: spec::PlanningLineage {
                active_operation_instances: Vec::new(),
                completed_operation_frames: Vec::new(),
                lineage_digest: content_digest(0x3c),
            },
            deterministic_predecessors: Vec::new(),
        }
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn schema_id(byte: u8) -> SchemaId {
        SchemaId::new(
            "mfm.test.schema",
            &format!("{byte}"),
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
        .expect("schema id")
    }

    fn content_digest(byte: u8) -> mfm_ids::ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    #[test]
    fn observed_failure_retryability_follows_policy_and_failure_class() {
        let retry_materialization = ObservedFailureRetryabilityPolicy {
            input_materialization_retryable: true,
        };
        let terminal_materialization = ObservedFailureRetryabilityPolicy {
            input_materialization_retryable: false,
        };

        assert!(retry_materialization.retryable_for(ObservedFailureClass::InputMaterialization));
        assert!(!terminal_materialization.retryable_for(ObservedFailureClass::InputMaterialization));
        assert!(!retry_materialization.retryable_for(ObservedFailureClass::InvalidRunnerOutput));
        assert!(!retry_materialization.retryable_for(ObservedFailureClass::RuntimeValidation));
    }
}
