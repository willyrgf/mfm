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
    pair_id: Option<&SideEffectPairId>,
    pair_role: Option<events::SideEffectPairRole>,
) -> Result<()> {
    if matches!(purpose, events::SideEffectLedgerPurpose::Forward)
        && projections.saga_engagement(run_id).is_some()
    {
        if pair_role == Some(events::SideEffectPairRole::Verify)
            && pair_id.is_some_and(|pair_id| {
                projections
                    .side_effect_for_run(run_id, ledger_key)
                    .is_some_and(|projection| {
                        projection.pair_id.as_ref() == Some(pair_id)
                            && matches!(
                                projection.ledger_purpose,
                                events::SideEffectLedgerPurpose::Forward
                            )
                    })
            })
        {
            return Ok(());
        }
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{ledger_key}"),
            message: "forward side-effect boundary event rejected after saga engagement".to_owned(),
        });
    }
    Ok(())
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
    let events::SideEffectLedgerPurpose::Remediation {
        forward_ledger_key,
        forward_pair_id,
    } = purpose
    else {
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
    if let Some(forward_pair_id) = forward_pair_id {
        let Some(pair_forward) = projections.side_effect_for_pair(run_id, forward_pair_id)? else {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{remediation_ledger_key}"),
                message: "remediation ledger references missing forward pair".to_owned(),
            });
        };
        if pair_forward.ledger_key != *forward_ledger_key {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{remediation_ledger_key}"),
                message: "remediation forward pair does not match forward ledger key".to_owned(),
            });
        }
    }
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
                    forward_ledger_key: linked,
                    ..
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

pub(super) fn require_side_effect_pair_event(
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_id: Option<&SideEffectPairId>,
    pair_role: Option<events::SideEffectPairRole>,
    expected_role: events::SideEffectPairRole,
) -> Result<()> {
    if pair_id.is_some() != pair_role.is_some() {
        return Err(side_effect_projection_error(
            ledger_key,
            "side-effect pair id and role must be recorded together",
        ));
    }
    match purpose {
        events::SideEffectLedgerPurpose::Forward => {
            if pair_id.is_none() {
                return Err(side_effect_projection_error(
                    ledger_key,
                    "forward side-effect event requires certified pair id",
                ));
            }
            if pair_role != Some(expected_role) {
                return Err(side_effect_projection_error(
                    ledger_key,
                    "side-effect pair role does not match event phase",
                ));
            }
        }
        events::SideEffectLedgerPurpose::Remediation { .. } => {
            if pair_id.is_some() && pair_role != Some(expected_role) {
                return Err(side_effect_projection_error(
                    ledger_key,
                    "side-effect pair role does not match event phase",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn require_side_effect_pair_consistent(
    projection: &SideEffectProjection,
    ledger_key: &events::SideEffectLedgerKey,
    pair_id: Option<&SideEffectPairId>,
) -> Result<()> {
    if projection.pair_id.as_ref() == pair_id {
        Ok(())
    } else {
        Err(side_effect_projection_error(
            ledger_key,
            "side-effect pair id changed",
        ))
    }
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
    predicate: impl FnOnce(&SideEffectLedgerState<'_>) -> bool,
) -> Result<&'a SideEffectProjection> {
    let Some(projection) = projections.side_effect_for_run(run_id, ledger_key) else {
        return Err(side_effect_projection_error(
            ledger_key,
            format!("missing side-effect projection; expected {expected}"),
        ));
    };
    let state = projection.ledger_state()?;
    if predicate(&state) {
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

fn phase_matches_expected(phase: &SideEffectLedgerPhase<'_>, expected: &'static str) -> bool {
    match expected {
        "started" => matches!(phase, SideEffectLedgerPhase::Started { .. }),
        "submission_recovery" => matches!(
            phase,
            SideEffectLedgerPhase::Started { .. }
                | SideEffectLedgerPhase::SubmissionKnown {
                    status: SideEffectSubmissionState::Unknown,
                    ..
                }
        ),
        "submission_observed" => {
            matches!(
                phase,
                SideEffectLedgerPhase::SubmissionKnown {
                    status: SideEffectSubmissionState::Observed { .. },
                    ..
                }
            )
        }
        "receipt" => matches!(phase, SideEffectLedgerPhase::ReceiptObserved { .. }),
        "not_submitted" => matches!(
            phase,
            SideEffectLedgerPhase::SubmissionKnown {
                status: SideEffectSubmissionState::NotSubmitted,
                ..
            }
        ),
        "ambiguity_source" => matches!(
            phase,
            SideEffectLedgerPhase::Started { .. }
                | SideEffectLedgerPhase::SubmissionKnown { .. }
                | SideEffectLedgerPhase::ReceiptObserved { .. }
        ),
        _ => false,
    }
}

pub(super) struct EpochOnlyTransition<'a> {
    pub(super) run_id: &'a RunId,
    pub(super) ledger_key: &'a events::SideEffectLedgerKey,
    pub(super) ledger_purpose: &'a events::SideEffectLedgerPurpose,
    pub(super) pair_id: Option<&'a SideEffectPairId>,
    pub(super) pair_role: Option<events::SideEffectPairRole>,
    pub(super) expected_pair_role: events::SideEffectPairRole,
    pub(super) required_previous: &'static str,
}

pub(super) fn transition_side_effect_epoch_only(
    projections: &mut ProjectionSnapshot,
    transition: EpochOnlyTransition<'_>,
    transition_state: impl FnOnce(OwnedSideEffectLedgerState) -> Result<OwnedSideEffectLedgerState>,
) -> Result<()> {
    require_side_effect_pair_event(
        transition.ledger_key,
        transition.ledger_purpose,
        transition.pair_id,
        transition.pair_role,
        transition.expected_pair_role,
    )?;
    require_forward_fence_open(
        projections,
        transition.run_id,
        transition.ledger_key,
        transition.ledger_purpose,
        transition.pair_id,
        transition.pair_role,
    )?;
    let previous = require_side_effect_phase(
        projections,
        transition.run_id,
        transition.ledger_key,
        transition.required_previous,
        |state| phase_matches_expected(&state.phase(), transition.required_previous),
    )?;
    require_side_effect_purpose(previous, transition.ledger_key, transition.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, transition.ledger_key, transition.pair_id)?;
    let projection = transition_state(OwnedSideEffectLedgerState::from_projection(
        previous.clone(),
    )?)?
    .into_projection();
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
    require_side_effect_pair_event(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_id.as_ref(),
        payload.pair_role,
        events::SideEffectPairRole::Verify,
    )?;
    require_forward_fence_open(
        projections,
        run_id,
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_id.as_ref(),
        payload.pair_role,
    )?;
    let Some(previous) = projections.side_effect_for_run(run_id, &payload.ledger_key) else {
        return Err(side_effect_projection_error(
            &payload.ledger_key,
            "missing side-effect projection",
        ));
    };
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, payload.pair_id.as_ref())?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .fail(event_id, payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectLedgerRef::new(run_id.clone(), payload.ledger_key.clone()),
        projection,
    );
    Ok(())
}
