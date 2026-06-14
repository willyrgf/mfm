use super::*;

pub(super) fn note_saga_engagement(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    engagement: SagaEngagementProjection,
) {
    projections
        .saga_engagements
        .entry(run_id.clone())
        .or_insert(engagement);
}

pub(super) fn require_forward_fence_open(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
) -> Result<()> {
    if matches!(purpose, events::SideEffectLedgerPurpose::Forward)
        && projections.saga_engagement(run_id).is_some()
    {
        Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{ledger_key}"),
            message: "forward side-effect boundary event rejected after saga engagement".to_owned(),
        })
    } else {
        Ok(())
    }
}

pub(super) fn require_forward_quiescence(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
) -> Result<()> {
    if forward_ledgers_quiescent(projections, run_id) {
        Ok(())
    } else {
        Err(StoreError::ProjectionConflict {
            key: format!("run:{run_id}:quiescence"),
            message: "past-boundary forward side-effect ledgers must be quiescent".to_owned(),
        })
    }
}

pub(super) fn require_remediation_intent_admissible(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    remediation_ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
) -> Result<()> {
    let events::SideEffectLedgerPurpose::Remediation { forward_ledger_key } = purpose else {
        return Ok(());
    };
    if projections.saga_engagement(run_id).is_none() {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger requires prior saga engagement".to_owned(),
        });
    }
    let Some(forward) = projections.side_effect_for_run(run_id, forward_ledger_key) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger references missing forward ledger".to_owned(),
        });
    };
    if !matches!(
        forward.ledger_purpose,
        events::SideEffectLedgerPurpose::Forward
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger references a non-forward ledger".to_owned(),
        });
    }
    if !matches!(forward.phase, SideEffectPhase::ConfirmationObserved { .. }) {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger requires confirmed forward ledger".to_owned(),
        });
    }
    if projections.side_effects.values().any(|projection| {
        projection.run_id == *run_id
            && matches!(
                &projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Remediation {
                    forward_ledger_key: linked
                } if linked == forward_ledger_key
            )
    }) {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger already exists for forward ledger".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn side_effect_projection_error(
    ledger_key: &events::SideEffectLedgerKey,
    message: impl Into<String>,
) -> StoreError {
    StoreError::ProjectionConflict {
        key: format!("sidefx:{ledger_key}"),
        message: message.into(),
    }
}

pub(super) fn require_side_effect_phase<'a>(
    projections: &'a ProjectionSnapshot,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
    expected: &'static str,
    predicate: impl FnOnce(&SideEffectProjection) -> bool,
) -> Result<&'a SideEffectProjection> {
    let Some(projection) = projections.side_effect_for_run(run_id, ledger_key) else {
        return Err(side_effect_projection_error(
            ledger_key,
            format!("missing side-effect projection; expected {expected}"),
        ));
    };
    if predicate(projection) {
        Ok(projection)
    } else {
        Err(side_effect_projection_error(
            ledger_key,
            format!("illegal side-effect transition; expected {expected}"),
        ))
    }
}

pub(super) fn require_side_effect_purpose(
    projection: &SideEffectProjection,
    ledger_key: &events::SideEffectLedgerKey,
    expected: &events::SideEffectLedgerPurpose,
) -> Result<()> {
    if projection.ledger_purpose == *expected {
        Ok(())
    } else {
        Err(side_effect_projection_error(
            ledger_key,
            "side-effect ledger purpose changed",
        ))
    }
}

pub(super) fn previous_claim<'a>(
    projection: &'a SideEffectProjection,
    ledger_key: &events::SideEffectLedgerKey,
) -> Result<&'a SideEffectClaimProjection> {
    projection.claim.as_ref().ok_or_else(|| {
        side_effect_projection_error(ledger_key, "side-effect transition requires active claim")
    })
}

pub(super) fn require_active_attempt_for_side_effect(
    projections: &ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    ledger_key: &events::SideEffectLedgerKey,
) -> Result<()> {
    match projections.attempt(node_id, attempt_id) {
        Some(AttemptProjection {
            status: AttemptStatus::Started { .. },
            ..
        }) => Ok(()),
        Some(_) => Err(side_effect_projection_error(
            ledger_key,
            "side-effect event requires an active started attempt",
        )),
        None => Err(side_effect_projection_error(
            ledger_key,
            "side-effect event requires a started attempt",
        )),
    }
}

pub(super) fn require_intent_context(
    projection: &SideEffectProjection,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) -> Result<()> {
    let ledger_key = &projection.ledger_key;
    if projection.intent.node_id != *node_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "node id does not match intent projection",
        ));
    }
    if projection.intent.attempt_id != *attempt_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "attempt id does not match intent projection",
        ));
    }
    if projection.intent.invocation_epoch != invocation_epoch {
        return Err(side_effect_projection_error(
            ledger_key,
            "invocation epoch does not match intent projection",
        ));
    }
    Ok(())
}

pub(super) fn require_intent_attempt_context(
    projection: &SideEffectProjection,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger_key = &projection.ledger_key;
    if projection.intent.node_id != *node_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "node id does not match intent projection",
        ));
    }
    if projection.intent.attempt_id != *attempt_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "attempt id does not match intent projection",
        ));
    }
    Ok(())
}

pub(super) struct ExpectedClaimContext<'a> {
    pub(super) node_id: &'a NodeId,
    pub(super) attempt_id: &'a AttemptId,
    pub(super) invocation_epoch: u32,
    pub(super) claim_generation: u32,
    pub(super) claim_fencing_token: &'a side_effect::ClaimFencingToken,
    pub(super) claim_owner: Option<&'a events::RunnerInvocationId>,
}

pub(super) fn require_claim_context(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    expected: ExpectedClaimContext<'_>,
) -> Result<()> {
    if claim.node_id != *expected.node_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "node id does not match active claim",
        ));
    }
    if claim.attempt_id != *expected.attempt_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "attempt id does not match active claim",
        ));
    }
    if claim.invocation_epoch != expected.invocation_epoch {
        return Err(side_effect_projection_error(
            ledger_key,
            "invocation epoch does not match active claim",
        ));
    }
    if claim.claim_generation != expected.claim_generation {
        return Err(side_effect_projection_error(
            ledger_key,
            "claim generation does not match active claim",
        ));
    }
    if claim.claim_fencing_token != *expected.claim_fencing_token {
        return Err(side_effect_projection_error(
            ledger_key,
            "claim fencing token does not match active claim",
        ));
    }
    if let Some(claim_owner) = expected.claim_owner {
        if claim.claim_owner != *claim_owner {
            return Err(side_effect_projection_error(
                ledger_key,
                "claim owner does not match active claim",
            ));
        }
    }
    Ok(())
}

pub(super) fn require_claim_takeover_matches(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    payload: &side_effect::ClaimTakenOver,
) -> Result<()> {
    if claim.node_id != payload.node_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "node id does not match active claim",
        ));
    }
    if claim.attempt_id != payload.attempt_id {
        return Err(side_effect_projection_error(
            ledger_key,
            "attempt id does not match active claim",
        ));
    }
    if claim.claim_owner != payload.previous_claim_owner {
        return Err(side_effect_projection_error(
            ledger_key,
            "previous claim owner does not match active claim",
        ));
    }
    if claim.claim_generation != payload.previous_claim_generation {
        return Err(side_effect_projection_error(
            ledger_key,
            "previous claim generation does not match active claim",
        ));
    }
    if payload.claim_generation <= payload.previous_claim_generation {
        return Err(side_effect_projection_error(
            ledger_key,
            "takeover claim generation must increase",
        ));
    }
    if payload.claim_fencing_token == claim.claim_fencing_token {
        return Err(side_effect_projection_error(
            ledger_key,
            "takeover claim fencing token must change",
        ));
    }
    if claim.invocation_epoch != payload.invocation_epoch {
        return Err(side_effect_projection_error(
            ledger_key,
            "invocation epoch does not match active claim",
        ));
    }
    Ok(())
}

pub(super) fn prepared_invocation_projection(
    artifact_id: &Option<ArtifactId>,
    content_digest: &Option<ContentDigest>,
    ledger_key: &events::SideEffectLedgerKey,
) -> Result<Option<SideEffectArtifactProjection>> {
    match (artifact_id, content_digest) {
        (Some(artifact_id), Some(content_digest)) => Ok(Some(SideEffectArtifactProjection {
            artifact_id: artifact_id.clone(),
            content_digest: content_digest.clone(),
            schema_id: None,
        })),
        (None, None) => Ok(None),
        _ => Err(side_effect_projection_error(
            ledger_key,
            "prepared invocation artifact id and hash must be recorded together",
        )),
    }
}

fn phase_matches_expected(phase: &SideEffectPhase, expected: &'static str) -> bool {
    match expected {
        "started" => matches!(phase, SideEffectPhase::InvocationStarted { .. }),
        "submission_recovery" => matches!(
            phase,
            SideEffectPhase::InvocationStarted { .. } | SideEffectPhase::SubmissionUnknown { .. }
        ),
        "submission_observed" => {
            matches!(phase, SideEffectPhase::SubmissionObserved { .. })
        }
        "receipt" => matches!(phase, SideEffectPhase::ReceiptObserved { .. }),
        "not_submitted" => matches!(phase, SideEffectPhase::NotSubmittedProven { .. }),
        "ambiguity_source" => matches!(
            phase,
            SideEffectPhase::InvocationStarted { .. }
                | SideEffectPhase::SubmissionUnknown { .. }
                | SideEffectPhase::SubmissionObserved { .. }
                | SideEffectPhase::ReceiptObserved { .. }
        ),
        _ => false,
    }
}

pub(super) struct EpochOnlyTransition<'a> {
    pub(super) run_id: &'a RunId,
    pub(super) ledger_key: &'a events::SideEffectLedgerKey,
    pub(super) ledger_purpose: &'a events::SideEffectLedgerPurpose,
    pub(super) node_id: &'a NodeId,
    pub(super) attempt_id: &'a AttemptId,
    pub(super) event_id: EventId,
    pub(super) invocation_epoch: u32,
    pub(super) required_previous: &'static str,
}

pub(super) fn transition_side_effect_epoch_only(
    projections: &mut ProjectionSnapshot,
    transition: EpochOnlyTransition<'_>,
    next_phase: impl FnOnce(u32) -> SideEffectPhase,
    update_projection: impl FnOnce(&mut SideEffectProjection) -> Result<()>,
) -> Result<()> {
    let (
        run_id,
        ledger_purpose,
        intent,
        prepared_invocation,
        resource_key,
        submission,
        receipt,
        confirmation,
        resource_touched_set,
        claim,
    ) = {
        let previous = require_side_effect_phase(
            projections,
            transition.run_id,
            transition.ledger_key,
            transition.required_previous,
            |projection| phase_matches_expected(&projection.phase, transition.required_previous),
        )?;
        require_side_effect_purpose(previous, transition.ledger_key, transition.ledger_purpose)?;
        let claim = previous_claim(previous, transition.ledger_key)?;
        require_claim_context(
            transition.ledger_key,
            claim,
            ExpectedClaimContext {
                node_id: transition.node_id,
                attempt_id: transition.attempt_id,
                invocation_epoch: transition.invocation_epoch,
                claim_generation: claim.claim_generation,
                claim_fencing_token: &claim.claim_fencing_token,
                claim_owner: Some(&claim.claim_owner),
            },
        )?;
        (
            previous.run_id.clone(),
            previous.ledger_purpose.clone(),
            previous.intent.clone(),
            previous.prepared_invocation.clone(),
            previous.resource_key.clone(),
            previous.submission.clone(),
            previous.receipt.clone(),
            previous.confirmation.clone(),
            previous.resource_touched_set.clone(),
            claim.clone(),
        )
    };
    let mut projection = SideEffectProjection {
        run_id,
        ledger_key: transition.ledger_key.clone(),
        ledger_purpose,
        event_id: transition.event_id,
        intent,
        prepared_invocation,
        resource_key,
        submission,
        receipt,
        confirmation,
        resource_touched_set,
        claim: Some(claim),
        phase: next_phase(transition.invocation_epoch),
    };
    update_projection(&mut projection)?;
    projections.side_effects.insert(
        SideEffectLedgerRef::new((*transition.run_id).clone(), transition.ledger_key.clone()),
        projection,
    );
    Ok(())
}

pub(super) fn transition_side_effect_failure(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    payload: &side_effect::Failed,
    event_id: EventId,
) -> Result<()> {
    let (
        run_id,
        ledger_purpose,
        intent,
        prepared_invocation,
        resource_key,
        submission,
        receipt,
        confirmation,
        resource_touched_set,
        claim,
    ) = {
        let Some(previous) = projections.side_effect_for_run(run_id, &payload.ledger_key) else {
            return Err(side_effect_projection_error(
                &payload.ledger_key,
                "missing side-effect projection",
            ));
        };
        require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
        match payload.failure_phase {
            side_effect::FailurePhase::BeforeInvocationStarted => match previous.phase {
                SideEffectPhase::IntentPersisted { invocation_epoch } => {
                    if payload.invocation_epoch != invocation_epoch {
                        return Err(side_effect_projection_error(
                            &payload.ledger_key,
                            "failure invocation epoch does not match intent",
                        ));
                    }
                    require_intent_context(
                        previous,
                        &payload.node_id,
                        &payload.attempt_id,
                        payload.invocation_epoch,
                    )?;
                }
                SideEffectPhase::Claimed { .. } | SideEffectPhase::InvocationPrepared { .. } => {
                    let claim = previous_claim(previous, &payload.ledger_key)?;
                    require_claim_context(
                        &payload.ledger_key,
                        claim,
                        ExpectedClaimContext {
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                            claim_generation: claim.claim_generation,
                            claim_fencing_token: &claim.claim_fencing_token,
                            claim_owner: None,
                        },
                    )?;
                }
                _ => {
                    return Err(side_effect_projection_error(
                        &payload.ledger_key,
                        "before-start failure requires intent, claim, or prepared phase",
                    ));
                }
            },
            side_effect::FailurePhase::AfterNotSubmittedProven => {
                if !phase_matches_expected(&previous.phase, "not_submitted") {
                    return Err(side_effect_projection_error(
                        &payload.ledger_key,
                        "after-not-submitted failure requires not-submitted phase",
                    ));
                }
                let claim = previous_claim(previous, &payload.ledger_key)?;
                require_claim_context(
                    &payload.ledger_key,
                    claim,
                    ExpectedClaimContext {
                        node_id: &payload.node_id,
                        attempt_id: &payload.attempt_id,
                        invocation_epoch: payload.invocation_epoch,
                        claim_generation: claim.claim_generation,
                        claim_fencing_token: &claim.claim_fencing_token,
                        claim_owner: Some(&claim.claim_owner),
                    },
                )?;
            }
        }
        (
            previous.run_id.clone(),
            previous.ledger_purpose.clone(),
            previous.intent.clone(),
            previous.prepared_invocation.clone(),
            previous.resource_key.clone(),
            previous.submission.clone(),
            previous.receipt.clone(),
            previous.confirmation.clone(),
            previous.resource_touched_set.clone(),
            previous.claim.clone(),
        )
    };
    projections.side_effects.insert(
        SideEffectLedgerRef::new(run_id.clone(), payload.ledger_key.clone()),
        SideEffectProjection {
            run_id,
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose,
            event_id,
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim,
            phase: SideEffectPhase::Failed {
                invocation_epoch: payload.invocation_epoch,
                failure_phase: payload.failure_phase,
            },
        },
    );
    Ok(())
}
