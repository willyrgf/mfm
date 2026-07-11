use super::*;

pub(super) fn derive_saga_projection(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    policy: &SagaPolicySpec,
    terminal_policies: &SideEffectTerminalPolicies,
) -> Result<SagaProjection> {
    let obligations = derive_saga_obligations(projections, run_id, terminal_policies)?;
    let run_completion = projections.run_completion(run_id).cloned();
    let manual_resolution = projections.manual_resolution(run_id).cloned();
    let engagement = projections.saga_engagement(run_id).cloned();
    let forward_quiescent = forward_ledgers_quiescent(projections, run_id, terminal_policies)?;
    let has_forward_boundary = obligations
        .values()
        .any(|obligation| forward_ledger_crossed_boundary(&obligation.forward_phase));
    let owed_count = obligations
        .values()
        .filter(|obligation| obligation.classification == ForwardLedgerClassification::Owed)
        .count();
    let all_owed_closed = owed_count > 0
        && obligations.values().all(|obligation| {
            obligation.classification != ForwardLedgerClassification::Owed
                || obligation
                    .remediation
                    .as_ref()
                    .map(|remediation| remediation.closed)
                    .unwrap_or(false)
        });
    let unresolved_reason = obligations.values().find_map(obligation_unresolved_reason);

    let (run_mode, manual_block_reason) = if let Some(completion) = run_completion.as_ref() {
        (RunMode::from_completion_outcome(&completion.outcome), None)
    } else if engagement.is_none() || !forward_quiescent {
        (RunMode::Forward, None)
    } else if !has_forward_boundary || (owed_count == 0 && unresolved_reason.is_none()) {
        (RunMode::FailedWithoutAcdcClaim, None)
    } else {
        run_mode_for_uncompleted_quiescent_saga(
            policy,
            manual_resolution.as_ref(),
            owed_count,
            all_owed_closed,
            unresolved_reason,
        )
    };

    Ok(SagaProjection {
        run_id: run_id.clone(),
        run_mode,
        engagement,
        forward_quiescent,
        manual_block_reason,
        obligations,
        manual_resolution,
        run_completion,
    })
}

fn derive_saga_obligations(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    terminal_policies: &SideEffectTerminalPolicies,
) -> Result<BTreeMap<SideEffectPairId, SagaObligationProjection>> {
    projections
        .side_effects
        .values()
        .filter(|projection| {
            projection.run_id == *run_id
                && matches!(
                    projection.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                )
        })
        .map(|forward| {
            let remediation =
                remediation_for_forward(projections, run_id, &forward.pair_id, terminal_policies)?;
            Ok((
                forward.pair_id.clone(),
                SagaObligationProjection {
                    forward_ledger_key: forward.ledger_key.clone(),
                    forward_pair_id: forward.pair_id.clone(),
                    forward_phase: forward.phase.clone(),
                    classification: forward_ledger_classification(
                        &forward.phase,
                        terminal_policies.require(&forward.pair_id)?,
                    ),
                    remediation,
                },
            ))
        })
        .collect()
}

fn remediation_for_forward(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    forward_pair_id: &SideEffectPairId,
    terminal_policies: &SideEffectTerminalPolicies,
) -> Result<Option<RemediationLedgerProjection>> {
    projections
        .side_effects
        .values()
        .find(|projection| {
            projection.run_id == *run_id
                && matches!(
                    &projection.ledger_purpose,
                    events::SideEffectLedgerPurpose::Remediation {
                        forward_pair_id: linked,
                        ..
                    } if *linked == *forward_pair_id
                )
        })
        .map(|projection| {
            let unresolved = remediation_unresolved_reason(projections, projection);
            Ok(RemediationLedgerProjection {
                ledger_key: projection.ledger_key.clone(),
                pair_id: projection.pair_id.clone(),
                phase: projection.phase.clone(),
                closed: terminal_policies
                    .require(&projection.pair_id)?
                    .is_terminal_phase(&projection.phase),
                unresolved,
            })
        })
        .transpose()
}

fn run_mode_for_uncompleted_quiescent_saga(
    policy: &SagaPolicySpec,
    manual_resolution: Option<&ManualResolutionProjection>,
    owed_count: usize,
    all_owed_closed: bool,
    unresolved_reason: Option<ManualBlockReason>,
) -> (RunMode, Option<ManualBlockReason>) {
    if let Some(mode) = manual_resolution.map(manual_resolution_run_mode) {
        return (mode, None);
    }

    match policy {
        SagaPolicySpec::NoSideEffects | SagaPolicySpec::FailWithoutAcdcClaim => {
            (RunMode::FailedWithoutAcdcClaim, None)
        }
        SagaPolicySpec::ManualResolution { .. } => (
            RunMode::ManualBlocked,
            Some(ManualBlockReason::PolicyManualResolution),
        ),
        SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved,
        } => {
            if let Some(reason) = unresolved_reason {
                match on_remediation_unresolved {
                    RemediationUnresolvedSpec::ManualResolution { .. } => {
                        (RunMode::ManualBlocked, Some(reason))
                    }
                    RemediationUnresolvedSpec::FailWithoutAcdcClaim => {
                        (RunMode::FailedWithoutAcdcClaim, None)
                    }
                }
            } else if owed_count > 0 && all_owed_closed {
                (RunMode::Compensated, None)
            } else if owed_count > 0 {
                (RunMode::Remediating, None)
            } else {
                (RunMode::FailedWithoutAcdcClaim, None)
            }
        }
    }
}

fn manual_resolution_run_mode(manual: &ManualResolutionProjection) -> RunMode {
    match manual.outcome {
        events::ManualResolutionOutcome::ConfirmRemediated => RunMode::ManuallyResolved,
        events::ManualResolutionOutcome::FailWithoutAcdcClaim => RunMode::FailedWithoutAcdcClaim,
    }
}

fn obligation_unresolved_reason(
    obligation: &SagaObligationProjection,
) -> Option<ManualBlockReason> {
    if obligation.classification == ForwardLedgerClassification::Unresolvable {
        Some(ManualBlockReason::ForwardAmbiguous)
    } else {
        obligation
            .remediation
            .as_ref()
            .and_then(|remediation| remediation.unresolved)
    }
}

fn remediation_unresolved_reason(
    projections: &ProjectionSnapshot,
    projection: &SideEffectProjection,
) -> Option<ManualBlockReason> {
    match projection.phase {
        SideEffectPhase::Ambiguous { .. } => Some(ManualBlockReason::RemediationAmbiguous),
        SideEffectPhase::Failed { .. } => {
            if side_effect_failure_retryable(projections, projection) == Some(false) {
                Some(ManualBlockReason::RemediationFailed)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn side_effect_failure_retryable(
    projections: &ProjectionSnapshot,
    projection: &SideEffectProjection,
) -> Option<bool> {
    match &projections
        .attempt(&projection.intent.node_id, &projection.intent.attempt_id)?
        .status
    {
        AttemptStatus::Failed { retryable, .. } => Some(*retryable),
        _ => None,
    }
}

fn forward_ledger_classification(
    phase: &SideEffectPhase,
    terminal_policy: SideEffectTerminalPolicy,
) -> ForwardLedgerClassification {
    match phase {
        SideEffectPhase::IntentPersisted { .. }
        | SideEffectPhase::Claimed { .. }
        | SideEffectPhase::InvocationPrepared { .. }
        | SideEffectPhase::NotSubmittedProven { .. }
        | SideEffectPhase::Failed { .. } => ForwardLedgerClassification::NothingOwed,
        SideEffectPhase::InvocationStarted { .. }
        | SideEffectPhase::SubmissionObserved { .. }
        | SideEffectPhase::SubmissionUnknown { .. } => ForwardLedgerClassification::Pending,
        SideEffectPhase::ReceiptObserved { .. } | SideEffectPhase::ConfirmationObserved { .. } => {
            if terminal_policy.is_terminal_phase(phase) {
                ForwardLedgerClassification::Owed
            } else {
                ForwardLedgerClassification::Pending
            }
        }
        SideEffectPhase::Ambiguous { .. } => ForwardLedgerClassification::Unresolvable,
    }
}

pub(super) fn forward_ledgers_quiescent(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    terminal_policies: &SideEffectTerminalPolicies,
) -> Result<bool> {
    for projection in projections.side_effects.values().filter(|projection| {
        projection.run_id == *run_id
            && matches!(
                projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            )
    }) {
        if forward_ledger_crossed_boundary(&projection.phase)
            && !forward_ledger_phase_is_quiescent(
                &projection.phase,
                terminal_policies.require(&projection.pair_id)?,
            )
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn forward_ledger_crossed_boundary(phase: &SideEffectPhase) -> bool {
    matches!(
        phase,
        SideEffectPhase::InvocationStarted { .. }
            | SideEffectPhase::SubmissionObserved { .. }
            | SideEffectPhase::SubmissionUnknown { .. }
            | SideEffectPhase::ReceiptObserved { .. }
            | SideEffectPhase::ConfirmationObserved { .. }
            | SideEffectPhase::Ambiguous { .. }
            | SideEffectPhase::NotSubmittedProven { .. }
            | SideEffectPhase::Failed {
                failure_phase: side_effect::FailurePhase::AfterNotSubmittedProven,
                ..
            }
    )
}

fn forward_ledger_phase_is_quiescent(
    phase: &SideEffectPhase,
    terminal_policy: SideEffectTerminalPolicy,
) -> bool {
    match phase {
        SideEffectPhase::NotSubmittedProven { .. }
        | SideEffectPhase::ConfirmationObserved { .. }
        | SideEffectPhase::Ambiguous { .. }
        | SideEffectPhase::Failed { .. } => true,
        SideEffectPhase::ReceiptObserved { .. } => terminal_policy.is_terminal_phase(phase),
        SideEffectPhase::IntentPersisted { .. }
        | SideEffectPhase::Claimed { .. }
        | SideEffectPhase::InvocationPrepared { .. }
        | SideEffectPhase::InvocationStarted { .. }
        | SideEffectPhase::SubmissionObserved { .. }
        | SideEffectPhase::SubmissionUnknown { .. } => false,
    }
}
