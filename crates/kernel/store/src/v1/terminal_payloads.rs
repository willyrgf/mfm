use super::*;

pub(super) fn validate_attempt_terminal_resource_lane_release_batch(
    request: &CommitRequest,
) -> Result<()> {
    reject_terminal_resource_lane_claims(AttemptTerminal::NAME, request)?;
    for (release_index, release) in request
        .payloads
        .iter()
        .enumerate()
        .filter(|(_, payload)| is_resource_lane_release_payload(payload))
    {
        let Some(release_ref) = release.resource_lane_authority_ref() else {
            continue;
        };
        let matched_terminal =
            request
                .payloads
                .iter()
                .enumerate()
                .any(|(terminal_index, terminal)| {
                    terminal_index > release_index
                        && terminal_payload_matches_resource_lane_release(
                            terminal,
                            &release_ref,
                            TerminalReleaseMatchKind::AttemptTerminal,
                            None,
                        )
                });
        if !matched_terminal {
            return Err(invalid_prepared_commit_purpose(
                AttemptTerminal::NAME,
                "resource lane release must precede a matching terminal attempt payload",
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_side_effect_terminal_resource_lane_release_batch(
    request: &CommitRequest,
) -> Result<()> {
    reject_terminal_resource_lane_claims(SideEffectTerminal::NAME, request)?;
    let authority = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                SideEffectTerminal::NAME,
                "side-effect terminal resource-lane release requires certified run authority",
            )
        })?;
    for (release_index, release) in request
        .payloads
        .iter()
        .enumerate()
        .filter(|(_, payload)| is_resource_lane_release_payload(payload))
    {
        let Some(release_ref) = release.resource_lane_authority_ref() else {
            continue;
        };
        let matched_terminal =
            request
                .payloads
                .iter()
                .enumerate()
                .any(|(terminal_index, terminal)| {
                    terminal_index > release_index
                        && terminal_payload_matches_resource_lane_release(
                            terminal,
                            &release_ref,
                            TerminalReleaseMatchKind::SideEffectTerminal,
                            Some(authority),
                        )
                });
        if !matched_terminal {
            return Err(invalid_prepared_commit_purpose(
                SideEffectTerminal::NAME,
                "resource lane release must precede a matching side-effect terminal payload",
            ));
        }
    }
    Ok(())
}

fn reject_terminal_resource_lane_claims(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    if request.payloads.iter().any(is_resource_lane_claim_payload) {
        return Err(invalid_prepared_commit_purpose(
            purpose,
            "terminal commits cannot acquire resource lanes",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalReleaseMatchKind {
    AttemptTerminal,
    SideEffectTerminal,
}

fn terminal_payload_matches_resource_lane_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
    kind: TerminalReleaseMatchKind,
    authority: Option<&CertifiedRunStoreAuthority>,
) -> bool {
    match kind {
        TerminalReleaseMatchKind::AttemptTerminal => {
            attempt_terminal_payload_matches_release(terminal, release)
        }
        TerminalReleaseMatchKind::SideEffectTerminal => {
            let Some(authority) = authority else {
                return false;
            };
            side_effect_terminal_payload_matches_release(terminal, release, authority)
        }
    }
}

fn attempt_terminal_payload_matches_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
) -> bool {
    let Some(emitter) = release.emitter else {
        return false;
    };
    match terminal {
        KernelEventPayload::StateAttemptCompleted(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        _ => false,
    }
}

fn side_effect_terminal_payload_matches_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
    authority: &CertifiedRunStoreAuthority,
) -> bool {
    if release.release_authority != Some(events::ResourceLaneReleaseAuthority::VerifyTerminal) {
        return false;
    }
    if !terminal.is_side_effect_terminal_disposition() {
        return false;
    }
    let Some(terminal) = terminal.side_effect_ledger_ref() else {
        return false;
    };
    terminal.ledger_key == release.ledger.ledger_key
        && terminal.ledger_purpose == release.ledger.ledger_purpose
        && terminal.pair_id == release.ledger.pair_id
        && terminal.invocation_epoch == release.ledger.invocation_epoch
        && side_effect_terminal_release_role_allowed(
            terminal.kind,
            terminal.pair_role,
            release.ledger.pair_role,
        )
        && side_effect_terminal_release_policy_allowed(terminal.kind, terminal.pair_id, authority)
}

fn side_effect_terminal_release_policy_allowed(
    terminal_kind: events::SideEffectEventKind,
    pair_id: &SideEffectPairId,
    authority: &CertifiedRunStoreAuthority,
) -> bool {
    let Ok(pair) = authority.side_effect_pair(pair_id) else {
        return false;
    };
    match terminal_kind {
        events::SideEffectEventKind::ReceiptObserved => {
            pair.terminal_policy == SideEffectTerminalPolicy::Receipt
        }
        events::SideEffectEventKind::ConfirmationObserved => true,
        events::SideEffectEventKind::NotSubmittedProven | events::SideEffectEventKind::Failed => {
            true
        }
        _ => false,
    }
}

fn side_effect_terminal_release_role_allowed(
    terminal_kind: events::SideEffectEventKind,
    terminal_role: events::SideEffectPairRole,
    release_role: events::SideEffectPairRole,
) -> bool {
    if release_role != events::SideEffectPairRole::Verify {
        return false;
    }
    match terminal_kind {
        events::SideEffectEventKind::NotSubmittedProven => {
            matches!(
                terminal_role,
                events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
            )
        }
        events::SideEffectEventKind::ReceiptObserved
        | events::SideEffectEventKind::ConfirmationObserved => {
            terminal_role == events::SideEffectPairRole::Verify
        }
        events::SideEffectEventKind::Failed => matches!(
            terminal_role,
            events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
        ),
        _ => false,
    }
}

pub(super) fn is_attempt_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    payload.is_attempt_terminal()
        || matches!(
            payload,
            KernelEventPayload::ResourceLaneReleased(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_)
        )
        || is_retention_ref_payload(payload)
}

pub(super) fn is_side_effect_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_side_effect_payload(payload)
        || payload.is_attempt_terminal()
        || is_retention_ref_payload(payload)
}

pub(super) fn is_side_effect_progress_commit_payload(payload: &KernelEventPayload) -> bool {
    (is_side_effect_payload(payload) && !payload.is_side_effect_terminal())
        || is_retention_ref_payload(payload)
}

pub(super) fn is_side_effect_payload(payload: &KernelEventPayload) -> bool {
    payload.side_effect_ledger_ref().is_some()
}

fn is_resource_lane_claim_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::ResourceLaneClaimed(_) | KernelEventPayload::ResourceLaneClaimIntent(_)
    )
}

fn is_resource_lane_release_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::ResourceLaneReleased(_)
            | KernelEventPayload::ResourceLaneReleaseIntent(_)
    )
}

pub(crate) fn is_retention_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RetentionRefsAppended(_)
            | KernelEventPayload::RetentionManifestProjected(_)
    )
}

fn is_retention_ref_payload(payload: &KernelEventPayload) -> bool {
    matches!(payload, KernelEventPayload::RetentionRefsAppended(_))
}

pub(crate) fn is_run_completed_payload(payload: &KernelEventPayload) -> bool {
    matches!(payload, KernelEventPayload::RunCompleted(_))
}

fn is_completed_run_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RunCompleted(events::RunCompleted {
            outcome: events::RunCompletionOutcome::Completed(_),
            ..
        })
    )
}

fn is_non_run_completed_attempt_terminal_payload(payload: &KernelEventPayload) -> bool {
    payload.is_attempt_terminal() && !is_run_completed_payload(payload)
}

pub(super) fn is_retention_commit_payload(payload: &KernelEventPayload) -> bool {
    is_retention_payload(payload)
        || is_non_run_completed_attempt_terminal_payload(payload)
        || is_completed_run_payload(payload)
}

pub(super) fn is_saga_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_run_completed_payload(payload)
        || is_non_run_completed_attempt_terminal_payload(payload)
        || is_retention_ref_payload(payload)
}

pub(crate) fn request_contains_saga_terminal_outcome(request: &CommitRequest) -> bool {
    request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::RunCompleted(events::RunCompleted {
                outcome: events::RunCompletionOutcome::Compensated
                    | events::RunCompletionOutcome::ManuallyResolved
                    | events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                ..
            })
        )
    })
}

pub(crate) fn request_contains_manual_resolution(request: &CommitRequest) -> bool {
    request
        .payloads
        .iter()
        .any(|payload| matches!(payload, KernelEventPayload::ManualResolutionRecorded(_)))
}
