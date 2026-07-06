use mfm_events::v1 as events;
use mfm_ids::{AttemptId, RunId, SideEffectPairId};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::invocation::{ErasedRunCtx, PreInvocationRunCtx};
use crate::{CertifiedRuntimeSpec, Result, RuntimeError};

/// Store-verified side-effect ledger view for one certified node attempt.
///
/// This view is minted only from runtime-owned verified context. It wraps the canonical
/// side-effect projection lookup and validates the projected ledger typestate before exposing it
/// to side-effect protocol helpers.
pub(crate) struct SideEffectAttemptView<'a> {
    projection: Option<&'a store::SideEffectProjection>,
    ledger_state: Option<store::SideEffectLedgerState<'a>>,
}

impl<'a> SideEffectAttemptView<'a> {
    /// Builds a side-effect attempt view from a prepared runner context.
    pub(crate) fn from_erased_context(ctx: &'a ErasedRunCtx<'_>) -> Result<Self> {
        Self::from_verified_parts(
            ctx.runtime_spec(),
            ctx.run_id(),
            ctx.projections(),
            ctx.node(),
            ctx.attempt_id(),
        )
    }

    /// Builds a side-effect attempt view from a pre-invocation resource-lane context.
    pub(crate) fn from_pre_invocation_context(ctx: &'a PreInvocationRunCtx<'_>) -> Result<Self> {
        Self::from_verified_parts(
            ctx.runtime_spec(),
            ctx.run_id(),
            ctx.projections(),
            ctx.node(),
            ctx.attempt_id(),
        )
    }

    fn from_verified_parts(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        projections: &'a store::ProjectionSnapshot,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
    ) -> Result<Self> {
        let projection = side_effect_projection_for_attempt(
            runtime_spec,
            run_id,
            projections,
            node,
            attempt_id,
        )?;
        let ledger_state = projection
            .map(|projection| {
                projection
                    .ledger_state()
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))
            })
            .transpose()?;
        Ok(Self {
            projection,
            ledger_state,
        })
    }

    /// Returns the backing side-effect projection, when this attempt has persisted ledger evidence.
    pub(crate) fn projection(&self) -> Option<&'a store::SideEffectProjection> {
        self.projection
    }

    /// Returns the validated ledger phase, when this attempt has persisted ledger evidence.
    pub(crate) fn phase(&self) -> Option<store::SideEffectLedgerPhase<'a>> {
        self.ledger_state
            .as_ref()
            .map(store::SideEffectLedgerState::phase)
    }
}

/// Recovery disposition for an open side-effect attempt.
pub(crate) enum SideEffectOpenAttemptDisposition {
    /// Continue the same attempt because no side-effect ledger evidence exists yet.
    ContinueBeforeLedger,
    /// Close the open attempt with standalone interruption before invocation preparation.
    InterruptBeforeInvocationPrepared,
    /// Re-enter the runner with the existing attempt id and store-projected ledger state.
    DelegateRecovery,
    /// Recovery cannot safely continue without operational intervention.
    OperationalBlock,
}

/// Validates that a side-effect attempt has certified terminal evidence before producing output.
pub(crate) fn validate_terminal_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let terminal_skipped = matches!(
        projections.cell_terminal_for_run(run_id, &node.output_cell),
        Some(store::CellTerminalProjection::Skipped {
            node_id,
            attempt_id: cell_attempt_id,
            ..
        }) if node_id == &node.node_id && cell_attempt_id == attempt_id
    );
    validate_terminal_batch_evidence(
        runtime_spec,
        run_id,
        projections,
        node,
        attempt_id,
        terminal_skipped,
    )
}

/// Returns whether standalone interruption can close this attempt without ledger recovery.
pub(crate) fn standalone_interruption_allowed(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<bool> {
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
        store::SideEffectLedgerPhase::IntentPersisted { .. }
            | store::SideEffectLedgerPhase::Claimed { .. }
    ))
}

/// Classifies an open side-effect attempt from store-owned ledger typestate.
pub(crate) fn open_attempt_disposition(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<SideEffectOpenAttemptDisposition> {
    let Some(projection) =
        side_effect_projection_for_attempt(runtime_spec, run_id, projections, node, attempt_id)?
    else {
        return Ok(SideEffectOpenAttemptDisposition::ContinueBeforeLedger);
    };
    let state = projection
        .ledger_state()
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    match state.phase() {
        store::SideEffectLedgerPhase::IntentPersisted { .. } => {
            Ok(SideEffectOpenAttemptDisposition::InterruptBeforeInvocationPrepared)
        }
        store::SideEffectLedgerPhase::Claimed { .. } if projection.resource_key.is_some() => {
            Ok(SideEffectOpenAttemptDisposition::DelegateRecovery)
        }
        store::SideEffectLedgerPhase::Claimed { .. } => {
            Ok(SideEffectOpenAttemptDisposition::InterruptBeforeInvocationPrepared)
        }
        store::SideEffectLedgerPhase::Prepared { .. }
        | store::SideEffectLedgerPhase::Started { .. }
        | store::SideEffectLedgerPhase::SubmissionKnown { .. }
        | store::SideEffectLedgerPhase::ReceiptObserved { .. }
        | store::SideEffectLedgerPhase::Confirmed { .. } => {
            Ok(SideEffectOpenAttemptDisposition::DelegateRecovery)
        }
        store::SideEffectLedgerPhase::Ambiguous { .. }
        | store::SideEffectLedgerPhase::Failed { .. } => {
            Ok(SideEffectOpenAttemptDisposition::OperationalBlock)
        }
    }
}

pub(crate) fn side_effect_projection_for_attempt<'a>(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &'a store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<Option<&'a store::SideEffectProjection>> {
    let Some(pair_id) = certified_side_effect_pair_id(runtime_spec, node)? else {
        return Ok(None);
    };
    let Some(projection) = projections.side_effect_for_pair(run_id, pair_id) else {
        return Ok(None);
    };
    validate_side_effect_actor_eligibility(node, attempt_id, pair_id, projection)?;
    Ok(Some(projection))
}

fn certified_side_effect_pair_id<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    node: &'a spec::NodeSpec,
) -> Result<Option<&'a SideEffectPairId>> {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => Ok(Some(&verify.pair_id)),
        _ if node.side_effect.is_some() => runtime_spec
            .side_effect_pair_for_submit_node(&node.node_id)
            .map(Some)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "side-effect node {} is missing certified verify pair",
                    node.node_id
                ))
            }),
        _ => Ok(None),
    }
}

fn validate_side_effect_actor_eligibility(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    pair_id: &SideEffectPairId,
    projection: &store::SideEffectProjection,
) -> Result<()> {
    if projection.pair_id != *pair_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect pair {} projected as pair {}",
            pair_id, projection.pair_id
        )));
    }
    match &node.framework {
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify))
            if projection.intent.node_id != verify.submit_node_id =>
        {
            Err(RuntimeError::InvalidRunStream(format!(
                "side-effect verify node {} pair {} points at submit node {} but projection belongs to node {}",
                node.node_id, pair_id, verify.submit_node_id, projection.intent.node_id
            )))
        }
        _ if node.side_effect.is_some() && projection.intent.node_id != node.node_id => {
            Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} pair {} projection belongs to node {}",
                node.node_id, pair_id, projection.intent.node_id
            )))
        }
        _ if node.side_effect.is_some() && projection.intent.attempt_id != *attempt_id => {
            Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} pair {} projection belongs to attempt {} instead of {}",
                node.node_id, pair_id, projection.intent.attempt_id, attempt_id
            )))
        }
        _ => Ok(()),
    }
}

/// Validates legal terminal evidence for a same-batch side-effect runner output.
pub(crate) fn validate_terminal_batch_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    terminal_skipped: bool,
) -> Result<()> {
    let Some(projection) =
        side_effect_projection_for_attempt(runtime_spec, run_id, projections, node, attempt_id)?
    else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output without ledger evidence",
            node.node_id, attempt_id
        )));
    };
    let state = projection
        .ledger_state()
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    if terminal_skipped {
        if matches!(
            state.phase(),
            store::SideEffectLedgerPhase::SubmissionKnown { .. }
                | store::SideEffectLedgerPhase::ReceiptObserved { .. }
                | store::SideEffectLedgerPhase::Confirmed { .. }
                | store::SideEffectLedgerPhase::Ambiguous { .. }
                | store::SideEffectLedgerPhase::Failed { .. }
        ) {
            return Ok(());
        }
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} skipped output before a submission result",
            node.node_id, attempt_id
        )));
    }
    let pair_id = projection.pair_id.clone();
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())?;
    if terminal_policies
        .require(&pair_id)?
        .is_terminal_phase(&projection.phase)
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before certified terminal evidence",
            node.node_id, attempt_id
        )))
    }
}

/// Validates legal runner evidence when resuming an open side-effect ledger.
pub(crate) fn validate_resume_output(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    let Some(projection) =
        side_effect_projection_for_attempt(runtime_spec, run_id, projections, node, attempt_id)?
    else {
        return Ok(());
    };
    let has_takeover = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectClaimTakenOver(_)
        )
    });
    let has_claim = payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::SideEffectClaimed(_)));
    let has_invocation_prepared = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationPrepared(_)
        )
    });
    let has_invocation_started = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )
    });
    let has_submission_recovery = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectNotSubmittedProven(_)
                | events::KernelEventPayload::SideEffectSubmissionObserved(_)
                | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
                | events::KernelEventPayload::SideEffectAmbiguous(_)
        )
    });
    let closes_submission_boundary = payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::CellSkipped(_)));
    let terminal_failure = payloads.iter().find_map(|payload| match payload {
        events::KernelEventPayload::SideEffectFailed(payload) => Some(payload),
        _ => None,
    });
    let state = projection
        .ledger_state()
        .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let phase = state.phase();
    let closes_projection = validate_side_effect_resume_failure(node, &phase, terminal_failure)?;

    match phase {
        store::SideEffectLedgerPhase::Claimed { .. } => {
            if !closes_projection && !has_takeover && !has_invocation_prepared {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed claimed ledger {} without takeover or prepared invocation",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectLedgerPhase::Prepared { .. } => {
            if !closes_projection && !has_takeover && !has_invocation_started {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed prepared ledger {} without takeover or invocation start",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::NotSubmitted,
            ..
        } => {
            if !closes_projection && !closes_submission_boundary && !has_claim {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed not-submitted ledger {} without next-epoch claim",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Unknown,
            ..
        } => {
            if !closes_submission_boundary && !has_submission_recovery {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed uncertain submission ledger {} without submission recovery evidence",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectLedgerPhase::Started { .. } => {
            if !has_submission_recovery {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed uncertain submission ledger {} without submission recovery evidence",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectLedgerPhase::Ambiguous { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run ambiguous ledger {}",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectLedgerPhase::Failed { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run failed ledger {} on the same attempt",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectLedgerPhase::IntentPersisted { .. }
        | store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Observed { .. },
            ..
        }
        | store::SideEffectLedgerPhase::ReceiptObserved { .. }
        | store::SideEffectLedgerPhase::Confirmed { .. } => {}
    }

    if has_invocation_started
        && matches!(phase, store::SideEffectLedgerPhase::Claimed { .. })
        && !has_takeover
        && !has_invocation_prepared
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect node {} started invocation from stale claim without takeover",
            node.node_id
        )));
    }

    Ok(())
}

fn validate_side_effect_resume_failure(
    node: &spec::NodeSpec,
    phase: &store::SideEffectLedgerPhase<'_>,
    failure: Option<&events::side_effect::Failed>,
) -> Result<bool> {
    let Some(failure) = failure else {
        return Ok(false);
    };
    let failure_matches_phase = matches!(
        (phase, failure.failure_phase),
        (
            store::SideEffectLedgerPhase::IntentPersisted { .. }
                | store::SideEffectLedgerPhase::Claimed { .. }
                | store::SideEffectLedgerPhase::Prepared { .. },
            events::side_effect::FailurePhase::BeforeInvocationStarted
        ) | (
            store::SideEffectLedgerPhase::SubmissionKnown {
                status: store::SideEffectSubmissionState::NotSubmitted,
                ..
            },
            events::side_effect::FailurePhase::AfterNotSubmittedProven
        )
    );
    if failure_matches_phase {
        Ok(true)
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect node {} returned terminal failure outside a failure-admissible ledger phase",
            node.node_id
        )))
    }
}
