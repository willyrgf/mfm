use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, DigestAlgorithm, NodeId, RunId, SchemaId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::RuntimeArtifactStager;
use crate::binding::BoundRuntimeContext;
use crate::commit::{
    AttemptFailureCommitInput, CommitPlanner, PreparedRunnerOutput, PreparedStagedArtifact,
    RunnerOutputCommitInput,
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
        projected_started_attempt_lane_key(projections, node)
            .map(|lane_key| lane_key == self.lane_key)
            .unwrap_or(true)
    }
}

/// Ordinary node-attempt lifecycle.
///
/// This owns selected/started/invoked/terminal-planned/terminal-committed orchestration for
/// non-framework-special attempts. New attempts append `StateAttemptStarted` before sealed runner
/// invocation construction, so observed materialization and runner-output validation failures can
/// terminalize through runtime-owned failure-safe evidence.
pub(crate) struct AttemptLifecycle<'a> {
    artifact_store: &'a dyn RuntimeArtifactStager,
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
    /// Creates a lifecycle bound to runtime-owned artifact staging.
    pub(crate) fn new(artifact_store: &'a dyn RuntimeArtifactStager) -> Self {
        Self { artifact_store }
    }

    /// Runs one ordinary attempt against a sync typed store.
    pub(crate) async fn run<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
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
                match store.append_prepared_commit_plan(start_commit.into()) {
                    Ok(_) => {}
                    Err(error) if store_error_is_stale_expected_next_seq(&error) => {
                        return Ok(AttemptRunStatus::StaleView);
                    }
                    Err(error) => return Err(error.into()),
                }
                advanced = true;
                (attempt_id, selected_attempt.phase.attempt_no)
            }
            Some(attempt_id) => (attempt_id, selected_attempt.phase.attempt_no),
        };

        let latest_stream = store.load_run_stream(run_id);
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
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
                return terminalize_observed_failure(
                    self.artifact_store,
                    store,
                    failure_context,
                    error,
                )
                .await;
            }
        };
        let output = match binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await
        {
            Ok(output) => output,
            Err(error) => {
                return terminalize_observed_failure(
                    self.artifact_store,
                    store,
                    failure_context,
                    error,
                )
                .await;
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
                return terminalize_observed_failure(
                    self.artifact_store,
                    store,
                    failure_context,
                    error,
                )
                .await;
            }
        };
        let terminal_planned_attempt = Attempt {
            phase: TerminalPlanned { terminal_output },
        };
        if let Some(witness) = resource_lane_block_for_request(
            store.projection_snapshot(),
            terminal_planned_attempt
                .phase
                .terminal_output
                .commit
                .request(),
        ) {
            return Ok(AttemptRunStatus::BlockedOnResourceLane { witness, advanced });
        }
        let has_resource_lane_prepare = request_has_resource_lane_prepare(
            terminal_planned_attempt
                .phase
                .terminal_output
                .commit
                .request(),
        );
        let terminal_output = terminal_planned_attempt.phase.terminal_output;
        stage_prepared_artifacts(self.artifact_store, &terminal_output.artifacts_to_stage).await?;
        match store.append_prepared_commit_plan(terminal_output.commit) {
            Ok(_) => {
                let _terminal_committed_attempt = Attempt {
                    phase: TerminalCommitted,
                };
                Ok(AttemptRunStatus::Advanced)
            }
            Err(error) if store_error_is_stale_expected_next_seq(&error) => {
                Ok(AttemptRunStatus::StaleView)
            }
            Err(error)
                if has_resource_lane_prepare
                    && resource_lane_block_witness_from_store_error(&node.node_id, &error)
                        .is_some() =>
            {
                let witness = resource_lane_block_witness_from_store_error(&node.node_id, &error)
                    .expect("resource lane block witness");
                Ok(AttemptRunStatus::BlockedOnResourceLane { witness, advanced })
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Runs one ordinary attempt against an async typed store.
    pub(crate) async fn run_async<S: store::AsyncTypedRunEventStore + ?Sized>(
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
                match store.append_prepared_commit_plan(start_commit.into()).await {
                    Ok(_) => {}
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

        let latest_stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
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
                return terminalize_observed_failure_async(
                    self.artifact_store,
                    store,
                    failure_context,
                    error,
                )
                .await;
            }
        };
        let output = match binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await
        {
            Ok(output) => output,
            Err(error) => {
                return terminalize_observed_failure_async(
                    self.artifact_store,
                    store,
                    failure_context,
                    error,
                )
                .await;
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
                return terminalize_observed_failure_async(
                    self.artifact_store,
                    store,
                    failure_context,
                    error,
                )
                .await;
            }
        };
        let terminal_planned_attempt = Attempt {
            phase: TerminalPlanned { terminal_output },
        };
        if let Some(witness) = resource_lane_block_for_request(
            &latest_view.projections,
            terminal_planned_attempt
                .phase
                .terminal_output
                .commit
                .request(),
        ) {
            return Ok(AttemptRunStatus::BlockedOnResourceLane { witness, advanced });
        }
        let has_resource_lane_prepare = request_has_resource_lane_prepare(
            terminal_planned_attempt
                .phase
                .terminal_output
                .commit
                .request(),
        );
        let terminal_output = terminal_planned_attempt.phase.terminal_output;
        stage_prepared_artifacts(self.artifact_store, &terminal_output.artifacts_to_stage).await?;
        match store
            .append_prepared_commit_plan(terminal_output.commit)
            .await
        {
            Ok(_) => {
                let _terminal_committed_attempt = Attempt {
                    phase: TerminalCommitted,
                };
                Ok(AttemptRunStatus::Advanced)
            }
            Err(error) if async_error_is_stale_expected_next_seq(&error) => {
                Ok(AttemptRunStatus::StaleView)
            }
            Err(error)
                if has_resource_lane_prepare
                    && async_resource_lane_block_witness(&node.node_id, &error).is_some() =>
            {
                let witness = async_resource_lane_block_witness(&node.node_id, &error)
                    .expect("resource lane block witness");
                Ok(AttemptRunStatus::BlockedOnResourceLane { witness, advanced })
            }
            Err(error) => Err(async_store_error(error)),
        }
    }
}

pub(crate) async fn terminalize_observed_failure<S: store::TypedRunEventStore + ?Sized>(
    artifact_store: &dyn RuntimeArtifactStager,
    store: &mut S,
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
    stage_prepared_artifacts(artifact_store, &failure.artifacts_to_stage).await?;
    match store.append_prepared_commit_plan(failure.commit) {
        Ok(_) => Ok(AttemptRunStatus::Advanced),
        Err(error) if store_error_is_stale_expected_next_seq(&error) => {
            Ok(AttemptRunStatus::StaleView)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn terminalize_observed_failure_async<
    S: store::AsyncTypedRunEventStore + ?Sized,
>(
    artifact_store: &dyn RuntimeArtifactStager,
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
    stage_prepared_artifacts(artifact_store, &failure.artifacts_to_stage).await?;
    match store.append_prepared_commit_plan(failure.commit).await {
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
    if node.side_effect.is_some()
        && SideEffectLifecycle::projection_for_attempt(&view.projections, node, attempt_id)?
            .is_some()
    {
        return Ok(false);
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
        ObservedFailureClass::InputMaterialization => events::MfmErrorInfo {
            code: events::ErrorCode::new("input_materialization_failed")?,
            category: events::ErrorCategory::Validation,
            retryable,
            safe_message: "attempt input materialization failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
        ObservedFailureClass::InvalidRunnerOutput => events::MfmErrorInfo {
            code: events::ErrorCode::new("runner_output_invalid")?,
            category: events::ErrorCategory::Validation,
            retryable,
            safe_message: "runner output failed validation".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
        ObservedFailureClass::RuntimeValidation => events::MfmErrorInfo {
            code: events::ErrorCode::new("runtime_validation_failed")?,
            category: events::ErrorCategory::Validation,
            retryable,
            safe_message: "runtime validation failed while handling attempt".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
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
    request: &store::TypedCommitRequest,
) -> Option<ResourceLaneBlockWitness> {
    request.payloads().iter().find_map(|payload| {
        let events::KernelEventPayload::SideEffectInvocationPrepared(payload) = payload else {
            return None;
        };
        let resource_key = payload.resource_key.as_ref()?;
        let lane_key = store::ResourceLaneKey::from_evidence(resource_key);
        let holder =
            store::SideEffectLedgerRef::new(request.run_id().clone(), payload.ledger_key.clone());
        projections
            .resource_lane(&lane_key)
            .filter(|projection| projection.holder != holder)
            .map(|_| ResourceLaneBlockWitness {
                node_id: payload.node_id.clone(),
                lane_key,
            })
    })
}

fn request_has_resource_lane_prepare(request: &store::TypedCommitRequest) -> bool {
    request.payloads().iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationPrepared(payload)
                if payload.resource_key.is_some()
        )
    })
}

#[cfg(test)]
fn store_error_is_resource_lane_block(error: &store::StoreError) -> bool {
    matches!(error, store::StoreError::ResourceLaneBlocked { .. })
}

fn resource_lane_block_witness_from_store_error(
    node_id: &NodeId,
    error: &store::StoreError,
) -> Option<ResourceLaneBlockWitness> {
    match error {
        store::StoreError::ResourceLaneBlocked { lane_key, .. } => Some(ResourceLaneBlockWitness {
            node_id: node_id.clone(),
            lane_key: (**lane_key).clone(),
        }),
        _ => None,
    }
}

#[cfg(test)]
fn async_error_is_resource_lane_block(error: &impl store::StoreErrorInspection) -> bool {
    error
        .as_store_error()
        .is_some_and(store_error_is_resource_lane_block)
}

fn async_resource_lane_block_witness(
    node_id: &NodeId,
    error: &impl store::StoreErrorInspection,
) -> Option<ResourceLaneBlockWitness> {
    error
        .as_store_error()
        .and_then(|error| resource_lane_block_witness_from_store_error(node_id, error))
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

fn projected_started_attempt_lane_key(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
) -> Option<store::ResourceLaneKey> {
    for ((attempt_node_id, attempt_id), attempt) in projections.attempts() {
        if attempt_node_id != &node.node_id
            || !matches!(attempt.status, store::AttemptStatus::Started { .. })
        {
            continue;
        }
        let Some(resource_key) = projections.side_effects().find_map(|(_, projection)| {
            (projection.intent.node_id == node.node_id
                && projection.intent.attempt_id == *attempt_id)
                .then_some(projection.resource_key.as_ref())
                .flatten()
        }) else {
            continue;
        };
        return Some(store::ResourceLaneKey::from_evidence(resource_key));
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

async fn stage_prepared_artifacts(
    artifact_store: &dyn RuntimeArtifactStager,
    artifacts: &[PreparedStagedArtifact],
) -> Result<()> {
    for artifact in artifacts {
        artifact_store
            .stage_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
            .await?;
    }
    Ok(())
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
