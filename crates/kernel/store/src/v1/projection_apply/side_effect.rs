use super::*;

pub(super) fn apply_side_effect_intent_persisted(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::IntentPersisted,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let ledger_ref =
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone());
    if projections.side_effects.contains_key(&ledger_ref) {
        return Err(side_effect_projection_error(
            &payload.ledger_key,
            "intent already persisted",
        ));
    }
    let intent = SideEffectIntentProjection {
        node_id: payload.node_id.clone(),
        attempt_id: payload.attempt_id.clone(),
        scope_id: payload.scope_id.clone(),
        invocation_epoch: payload.invocation_epoch,
        intent_schema_id: payload.intent_schema_id.clone(),
        intent_hash: payload.intent_hash.clone(),
        intent_artifact_id: payload.intent_artifact_id.clone(),
        idempotency_input_schema_id: payload.idempotency_input_schema_id.clone(),
        idempotency_input_hash: payload.idempotency_input_hash.clone(),
        idempotency_key: payload.idempotency_key.clone(),
        capability_kind: payload.capability_kind.clone(),
        capability_version: payload.capability_version.clone(),
        adapter_kind: payload.adapter_kind.clone(),
        adapter_version: payload.adapter_version.clone(),
    };
    projections.side_effects.insert(
        ledger_ref,
        OwnedSideEffectLedgerState::intent_persisted(
            envelope.run_id().clone(),
            payload.ledger_key.clone(),
            payload.ledger_purpose.clone(),
            payload.pair_id.clone(),
            envelope.event_id.clone(),
            intent,
            payload.invocation_epoch,
        )
        .into_projection(),
    );
    Ok(())
}

pub(super) fn apply_side_effect_claimed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::Claimed,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "intent or not-submitted",
        |state| {
            matches!(
                state.phase(),
                SideEffectLedgerPhase::IntentPersisted { .. }
                    | SideEffectLedgerPhase::SubmissionKnown {
                        status: SideEffectSubmissionState::NotSubmitted,
                        ..
                    }
            )
        },
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .claim(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

pub(super) fn apply_side_effect_claim_taken_over(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::ClaimTakenOver,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "claim or prepared",
        |state| {
            matches!(
                state.phase(),
                SideEffectLedgerPhase::Claimed { .. } | SideEffectLedgerPhase::Prepared { .. }
            )
        },
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .take_over(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

pub(super) fn apply_side_effect_invocation_prepared(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::InvocationPrepared,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "claim",
        |state| matches!(state.phase(), SideEffectLedgerPhase::Claimed { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let prepared_invocation = prepared_invocation_projection(
        &payload.prepared_artifact_id,
        &payload.prepared_hash,
        &payload.prepared_artifact_evidence_hash,
        &payload.ledger_key,
    )?
    .or_else(|| previous.prepared_invocation.clone());
    let resource_key = match (&previous.resource_key, &payload.resource_key) {
        (Some(held), Some(prepared)) if held == prepared => Some(held.clone()),
        (Some(_), Some(_)) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "prepared invocation resource key must match the held resource lane"
                    .to_owned(),
            });
        }
        (Some(_), None) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "prepared invocation must echo the held resource lane key".to_owned(),
            });
        }
        (None, Some(_)) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "resource lane must be claimed before invocation prepare".to_owned(),
            });
        }
        (None, None) => None,
    };
    if let Some(resource_key) = &resource_key {
        let lane_key = ResourceLaneKey::from_evidence(resource_key);
        let holder =
            SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone());
        if projections
            .resource_lane(&lane_key)
            .is_none_or(|projection| projection.holder != holder)
        {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "resource lane must be claimed before invocation prepare".to_owned(),
            });
        }
    }
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .prepare(
            envelope.event_id.clone(),
            payload,
            prepared_invocation,
            resource_key,
        )?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

pub(super) fn apply_resource_lane_claimed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::ResourceLaneClaimed,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "claim",
        |state| matches!(state.phase(), SideEffectLedgerPhase::Claimed { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let mut projection = previous.clone();
    if let Some(existing) = &projection.resource_key {
        if existing != &payload.resource_key {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "resource lane claim changed held resource key".to_owned(),
            });
        }
    }
    acquire_resource_lane(projections, envelope.run_id(), &envelope.event_id, payload)?;
    projection.event_id = envelope.event_id.clone();
    projection.resource_key = Some(payload.resource_key.clone());
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

pub(super) fn apply_resource_lane_released(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::ResourceLaneReleased,
) -> Result<()> {
    release_resource_lane(projections, envelope.run_id(), payload)
}

pub(super) fn apply_side_effect_invocation_started(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::InvocationStarted,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "prepared",
        |state| matches!(state.phase(), SideEffectLedgerPhase::Prepared { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .start(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

pub(super) fn apply_side_effect_not_submitted_proven(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::NotSubmittedProven,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::SubmitOrVerify,
            required_previous: "submission_recovery",
        },
        |state| state.mark_not_submitted(envelope.event_id.clone(), payload),
    )?;
    require_no_resource_lane_for_holder(
        projections,
        &SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        "not-submitted proof",
    )?;
    Ok(())
}

pub(super) fn apply_side_effect_submission_observed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::SubmissionObserved,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::SubmitOrVerify,
            required_previous: "submission_recovery",
        },
        |state| state.record_submission(envelope.event_id.clone(), payload),
    )?;
    Ok(())
}

pub(super) fn apply_side_effect_submission_unknown(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::SubmissionUnknown,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Submit),
            required_previous: "submission_recovery",
        },
        |state| state.mark_submission_unknown(envelope.event_id.clone(), payload),
    )?;
    Ok(())
}

pub(super) fn apply_side_effect_receipt_observed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::ReceiptObserved,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Verify),
            required_previous: "submission_observed",
        },
        |state| state.record_receipt(envelope.event_id.clone(), payload),
    )?;
    Ok(())
}

pub(super) fn apply_side_effect_confirmation_observed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::ConfirmationObserved,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Verify),
            required_previous: "receipt",
        },
        |state| state.confirm(envelope.event_id.clone(), payload),
    )?;
    require_no_resource_lane_for_holder(
        projections,
        &SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        "confirmation",
    )?;
    Ok(())
}

pub(super) fn apply_side_effect_ambiguous(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::Ambiguous,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::SubmitOrVerify,
            required_previous: "ambiguity_source",
        },
        |state| state.mark_ambiguous(envelope.event_id.clone(), payload),
    )?;
    if matches!(
        payload.ledger_purpose,
        events::SideEffectLedgerPurpose::Forward
    ) {
        note_saga_engagement(
            projections,
            envelope.run_id(),
            SagaEngagementProjection {
                event_id: envelope.event_id.clone(),
                reason: SagaEngagementReason::ForwardAmbiguous {
                    pair_id: payload.pair_id.clone(),
                },
            },
        );
    }
    Ok(())
}

pub(super) fn apply_side_effect_failed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::Failed,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_failure(
        projections,
        envelope.run_id(),
        payload,
        envelope.event_id.clone(),
    )?;
    require_no_resource_lane_for_holder(
        projections,
        &SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        "side-effect failure",
    )?;
    if !payload.retryable {
        note_saga_engagement(
            projections,
            envelope.run_id(),
            SagaEngagementProjection {
                event_id: envelope.event_id.clone(),
                reason: SagaEngagementReason::NonRetryableFailure {
                    node_id: payload.node_id.clone(),
                    attempt_id: payload.attempt_id.clone(),
                },
            },
        );
    }
    Ok(())
}
