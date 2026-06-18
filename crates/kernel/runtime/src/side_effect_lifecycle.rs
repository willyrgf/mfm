use mfm_events::v1 as events;
use mfm_ids::AttemptId;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::{Result, RuntimeError};

/// Runtime lifecycle guard for side-effect attempt uncertainty and recovery evidence.
pub(crate) struct SideEffectLifecycle;

/// Recovery disposition for an open side-effect attempt.
pub(crate) enum SideEffectOpenAttemptDisposition {
    /// Re-enter the runner with the existing attempt id and store-projected ledger state.
    DelegateRecovery,
}

impl SideEffectLifecycle {
    /// Returns the single durable side-effect projection for a certified node attempt.
    pub(crate) fn projection_for_attempt<'a>(
        projections: &'a store::ProjectionSnapshot,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
    ) -> Result<Option<&'a store::SideEffectProjection>> {
        side_effect_projection_for_attempt(projections, node, attempt_id)
    }

    /// Validates that a side-effect attempt has confirmed terminal evidence before producing output.
    pub(crate) fn validate_terminal_evidence(
        projections: &store::ProjectionSnapshot,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
    ) -> Result<()> {
        validate_side_effect_terminal_evidence(projections, node, attempt_id)
    }

    /// Validates legal runner evidence when resuming an open side-effect ledger.
    pub(crate) fn validate_resume_output(
        projections: &store::ProjectionSnapshot,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        payloads: &[events::KernelEventPayload],
    ) -> Result<()> {
        validate_side_effect_resume_output(projections, node, attempt_id, payloads)
    }

    /// Classifies an open side-effect attempt from store-owned ledger typestate.
    pub(crate) fn open_attempt_disposition(
        projections: &store::ProjectionSnapshot,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
    ) -> Result<SideEffectOpenAttemptDisposition> {
        let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
        else {
            return Ok(SideEffectOpenAttemptDisposition::DelegateRecovery);
        };
        let state = projection
            .ledger_state()
            .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
        if state.is_ambiguous() || state.is_failed() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has terminal ledger evidence while the attempt remains started",
                node.node_id, attempt_id
            )));
        }
        Ok(SideEffectOpenAttemptDisposition::DelegateRecovery)
    }
}

pub(crate) fn side_effect_projection_for_attempt<'a>(
    projections: &'a store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<Option<&'a store::SideEffectProjection>> {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == node.node_id
            && projection.intent.attempt_id == *attempt_id
            && found.replace(projection).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledger projections",
                node.node_id, attempt_id
            )));
        }
    }
    Ok(found)
}

pub(crate) fn validate_side_effect_terminal_evidence(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output without ledger evidence",
            node.node_id, attempt_id
        )));
    };
    let state = projection
        .ledger_state()
        .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
    if state.is_confirmed() {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node.node_id, attempt_id
        )))
    }
}

pub(crate) fn validate_side_effect_resume_output(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
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
            if !closes_projection && !has_claim {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed not-submitted ledger {} without next-epoch claim",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectLedgerPhase::Started { .. }
        | store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Unknown,
            ..
        } => {
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
