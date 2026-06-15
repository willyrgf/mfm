use super::*;

pub(super) fn apply_projection(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
) -> Result<()> {
    match envelope.payload() {
        KernelEventPayload::RunStarted(payload) => apply_run_started(projections, payload)?,
        KernelEventPayload::RunCompleted(payload) => {
            apply_run_completed(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::StateAttemptStarted(payload) => {
            apply_attempt_started(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::StateAttemptCompleted(payload) => {
            apply_attempt_completed(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            apply_attempt_failed(projections, envelope, payload)?;
        }
        KernelEventPayload::CellProduced(payload) => {
            apply_cell_produced(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::CellSkipped(payload) => {
            apply_cell_skipped(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            apply_side_effect_intent_persisted(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectClaimed(payload) => {
            apply_side_effect_claimed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            apply_side_effect_claim_taken_over(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            apply_side_effect_invocation_prepared(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectInvocationStarted(payload) => {
            apply_side_effect_invocation_started(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            apply_side_effect_not_submitted_proven(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            apply_side_effect_submission_observed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            apply_side_effect_submission_unknown(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectReceiptObserved(payload) => {
            apply_side_effect_receipt_observed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            apply_side_effect_confirmation_observed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectAmbiguous(payload) => {
            apply_side_effect_ambiguous(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectFailed(payload) => {
            apply_side_effect_failed(projections, envelope, payload)?;
        }
        KernelEventPayload::PublicOutputProduced(payload) => {
            apply_public_output_produced(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            apply_public_output_render_failed(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            apply_manual_resolution_recorded(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::RetentionRefsAppended(payload) => {
            apply_retention_refs_appended(projections, payload)?;
        }
        KernelEventPayload::RetentionManifestProjected(payload) => {
            apply_retention_manifest_projected(projections, payload)?;
        }
        KernelEventPayload::FactRecorded(payload) => {
            apply_fact_recorded(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::ArtifactReferenced(_) => {}
    }
    Ok(())
}

fn apply_run_started(
    projections: &mut ProjectionSnapshot,
    payload: &events::RunStarted,
) -> Result<()> {
    let state = projections.run_state(&payload.run_id);
    if state != RunState::Absent {
        return Err(StoreError::ProjectionConflict {
            key: "run:start".to_owned(),
            message: "run already started".to_owned(),
        });
    }
    projections
        .run_states
        .insert(payload.run_id.clone(), RunState::Started);
    Ok(())
}

fn apply_run_completed(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::RunCompleted,
) -> Result<()> {
    let state = projections.run_state(&payload.run_id);
    if state != RunState::Started {
        return Err(StoreError::ProjectionConflict {
            key: "run:complete".to_owned(),
            message: "run must be started and not completed".to_owned(),
        });
    }
    require_forward_quiescence(projections, &payload.run_id)?;
    projections
        .run_states
        .insert(payload.run_id.clone(), RunState::Completed);
    projections.run_completions.insert(
        payload.run_id.clone(),
        RunCompletionProjection {
            event_id: event_id.clone(),
            outcome: payload.outcome.clone(),
        },
    );
    release_resource_lanes_for_run(projections, &payload.run_id);
    Ok(())
}

fn apply_attempt_started(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::StateAttemptStarted,
) -> Result<()> {
    let key = (payload.node_id.clone(), payload.attempt_id.clone());
    if projections.attempts.contains_key(&key) {
        return Err(StoreError::ProjectionConflict {
            key: format!("attempt:{}:{}", payload.node_id, payload.attempt_id),
            message: "attempt already started".to_owned(),
        });
    }
    projections.attempts.insert(
        key,
        AttemptProjection {
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            event_id: event_id.clone(),
            status: AttemptStatus::Started {
                attempt_no: payload.attempt_no,
                state_kind: payload.state_kind.clone(),
                state_version: payload.state_version.clone(),
            },
        },
    );
    Ok(())
}

fn apply_attempt_completed(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::StateAttemptCompleted,
) -> Result<()> {
    update_attempt_terminal_projection(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        event_id.clone(),
        AttemptStatus::Completed {
            output_cell_id: payload.output_cell_id.clone(),
        },
    )
}

fn apply_attempt_failed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::StateAttemptFailed,
) -> Result<()> {
    update_attempt_terminal_projection(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        envelope.event_id.clone(),
        AttemptStatus::Failed {
            retryable: payload.retryable,
            error: Box::new(payload.error.clone()),
        },
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

fn apply_side_effect_intent_persisted(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::IntentPersisted,
) -> Result<()> {
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
    )?;
    require_remediation_intent_admissible(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let ledger_ref =
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone());
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
        SideEffectProjection {
            run_id: envelope.run_id().clone(),
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose: payload.ledger_purpose.clone(),
            event_id: envelope.event_id.clone(),
            intent,
            prepared_invocation: None,
            resource_key: None,
            submission: None,
            receipt: None,
            confirmation: None,
            resource_touched_set: None,
            claim: None,
            phase: SideEffectPhase::IntentPersisted {
                invocation_epoch: payload.invocation_epoch,
            },
        },
    );
    Ok(())
}

fn apply_side_effect_claimed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::Claimed,
) -> Result<()> {
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
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
        "intent or not-submitted",
        |projection| {
            matches!(
                projection.phase,
                SideEffectPhase::IntentPersisted { .. }
                    | SideEffectPhase::NotSubmittedProven { .. }
            )
        },
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    match previous.phase {
        SideEffectPhase::IntentPersisted { invocation_epoch } => {
            if payload.invocation_epoch != invocation_epoch {
                return Err(side_effect_projection_error(
                    &payload.ledger_key,
                    "initial claim invocation epoch does not match intent",
                ));
            }
            require_intent_context(
                previous,
                &payload.node_id,
                &payload.attempt_id,
                payload.invocation_epoch,
            )?;
        }
        SideEffectPhase::NotSubmittedProven { invocation_epoch } => {
            require_intent_attempt_context(previous, &payload.node_id, &payload.attempt_id)?;
            let previous_claim = previous_claim(previous, &payload.ledger_key)?;
            if payload.claim_generation <= previous_claim.claim_generation {
                return Err(side_effect_projection_error(
                    &payload.ledger_key,
                    "retry claim generation must increase",
                ));
            }
            if payload.claim_fencing_token == previous_claim.claim_fencing_token {
                return Err(side_effect_projection_error(
                    &payload.ledger_key,
                    "retry claim fencing token must change",
                ));
            }
            let next_epoch = invocation_epoch.checked_add(1).ok_or_else(|| {
                side_effect_projection_error(&payload.ledger_key, "invocation epoch overflow")
            })?;
            if payload.invocation_epoch != next_epoch {
                return Err(side_effect_projection_error(
                    &payload.ledger_key,
                    "retry claim must advance to the next invocation epoch",
                ));
            }
        }
        _ => unreachable!("phase predicate checked above"),
    }
    let intent = previous.intent.clone();
    let run_id = previous.run_id.clone();
    let ledger_purpose = previous.ledger_purpose.clone();
    let prepared_invocation = previous.prepared_invocation.clone();
    let resource_key = previous.resource_key.clone();
    let submission = previous.submission.clone();
    let receipt = previous.receipt.clone();
    let confirmation = previous.confirmation.clone();
    let resource_touched_set = previous.resource_touched_set.clone();
    let claim = SideEffectClaimProjection {
        node_id: payload.node_id.clone(),
        attempt_id: payload.attempt_id.clone(),
        claim_owner: payload.claim_owner.clone(),
        invocation_epoch: payload.invocation_epoch,
        claim_generation: payload.claim_generation,
        claim_fencing_token: payload.claim_fencing_token.clone(),
    };
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        SideEffectProjection {
            run_id,
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose,
            event_id: envelope.event_id.clone(),
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim: Some(claim),
            phase: SideEffectPhase::Claimed {
                claim_owner: payload.claim_owner.clone(),
                invocation_epoch: payload.invocation_epoch,
                claim_generation: payload.claim_generation,
                claim_fencing_token: payload.claim_fencing_token.clone(),
            },
        },
    );
    Ok(())
}

fn apply_side_effect_claim_taken_over(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::ClaimTakenOver,
) -> Result<()> {
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
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
        "claim or prepared",
        |projection| {
            matches!(
                projection.phase,
                SideEffectPhase::Claimed { .. } | SideEffectPhase::InvocationPrepared { .. }
            )
        },
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    let old_claim = previous_claim(previous, &payload.ledger_key)?;
    require_claim_takeover_matches(&payload.ledger_key, old_claim, payload)?;
    let intent = previous.intent.clone();
    let run_id = previous.run_id.clone();
    let ledger_purpose = previous.ledger_purpose.clone();
    let prepared_invocation = previous.prepared_invocation.clone();
    let resource_key = previous.resource_key.clone();
    let submission = previous.submission.clone();
    let receipt = previous.receipt.clone();
    let confirmation = previous.confirmation.clone();
    let resource_touched_set = previous.resource_touched_set.clone();
    let claim = SideEffectClaimProjection {
        node_id: payload.node_id.clone(),
        attempt_id: payload.attempt_id.clone(),
        claim_owner: payload.new_claim_owner.clone(),
        invocation_epoch: payload.invocation_epoch,
        claim_generation: payload.claim_generation,
        claim_fencing_token: payload.claim_fencing_token.clone(),
    };
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        SideEffectProjection {
            run_id,
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose,
            event_id: envelope.event_id.clone(),
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim: Some(claim),
            phase: SideEffectPhase::Claimed {
                claim_owner: payload.new_claim_owner.clone(),
                invocation_epoch: payload.invocation_epoch,
                claim_generation: payload.claim_generation,
                claim_fencing_token: payload.claim_fencing_token.clone(),
            },
        },
    );
    Ok(())
}

fn apply_side_effect_invocation_prepared(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::InvocationPrepared,
) -> Result<()> {
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
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
        "claim",
        |projection| matches!(projection.phase, SideEffectPhase::Claimed { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    let claim = previous_claim(previous, &payload.ledger_key)?;
    require_claim_context(
        &payload.ledger_key,
        claim,
        ExpectedClaimContext {
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            invocation_epoch: payload.invocation_epoch,
            claim_generation: payload.claim_generation,
            claim_fencing_token: &payload.claim_fencing_token,
            claim_owner: None,
        },
    )?;
    let intent = previous.intent.clone();
    let run_id = previous.run_id.clone();
    let ledger_purpose = previous.ledger_purpose.clone();
    let prepared_invocation = prepared_invocation_projection(
        &payload.prepared_artifact_id,
        &payload.prepared_hash,
        &payload.ledger_key,
    )?
    .or_else(|| previous.prepared_invocation.clone());
    let submission = previous.submission.clone();
    let receipt = previous.receipt.clone();
    let confirmation = previous.confirmation.clone();
    let resource_touched_set = previous.resource_touched_set.clone();
    let claim = claim.clone();
    let resource_key =
        acquire_resource_lane(projections, envelope.run_id(), &envelope.event_id, payload)?;
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        SideEffectProjection {
            run_id,
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose,
            event_id: envelope.event_id.clone(),
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim: Some(claim),
            phase: SideEffectPhase::InvocationPrepared {
                invocation_epoch: payload.invocation_epoch,
                claim_generation: payload.claim_generation,
                claim_fencing_token: payload.claim_fencing_token.clone(),
            },
        },
    );
    Ok(())
}

fn apply_side_effect_invocation_started(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::InvocationStarted,
) -> Result<()> {
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
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
        "prepared",
        |projection| matches!(projection.phase, SideEffectPhase::InvocationPrepared { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    let claim = previous_claim(previous, &payload.ledger_key)?;
    require_claim_context(
        &payload.ledger_key,
        claim,
        ExpectedClaimContext {
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            invocation_epoch: payload.invocation_epoch,
            claim_generation: payload.claim_generation,
            claim_fencing_token: &payload.claim_fencing_token,
            claim_owner: Some(&payload.claim_owner),
        },
    )?;
    let intent = previous.intent.clone();
    let run_id = previous.run_id.clone();
    let ledger_purpose = previous.ledger_purpose.clone();
    let prepared_invocation = previous.prepared_invocation.clone();
    let resource_key = previous.resource_key.clone();
    let submission = previous.submission.clone();
    let receipt = previous.receipt.clone();
    let confirmation = previous.confirmation.clone();
    let resource_touched_set = previous.resource_touched_set.clone();
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        SideEffectProjection {
            run_id,
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose,
            event_id: envelope.event_id.clone(),
            intent,
            prepared_invocation,
            resource_key,
            submission,
            receipt,
            confirmation,
            resource_touched_set,
            claim: Some(claim.clone()),
            phase: SideEffectPhase::InvocationStarted {
                claim_owner: payload.claim_owner.clone(),
                invocation_epoch: payload.invocation_epoch,
                claim_generation: payload.claim_generation,
                claim_fencing_token: payload.claim_fencing_token.clone(),
            },
        },
    );
    Ok(())
}

fn apply_side_effect_not_submitted_proven(
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
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            event_id: envelope.event_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            required_previous: "submission_recovery",
        },
        |epoch| SideEffectPhase::NotSubmittedProven {
            invocation_epoch: epoch,
        },
        |_| Ok(()),
    )?;
    release_resource_lane_for_holder(
        projections,
        &SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
    );
    Ok(())
}

fn apply_side_effect_submission_observed(
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
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            event_id: envelope.event_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            required_previous: "submission_recovery",
        },
        |epoch| SideEffectPhase::SubmissionObserved {
            invocation_epoch: epoch,
        },
        |projection| {
            projection.submission = Some(SideEffectArtifactProjection {
                artifact_id: payload.submission_artifact_id.clone(),
                content_digest: payload.submission_hash.clone(),
                schema_id: Some(payload.submission_schema_id.clone()),
            });
            Ok(())
        },
    )?;
    Ok(())
}

fn apply_side_effect_submission_unknown(
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
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            event_id: envelope.event_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            required_previous: "submission_recovery",
        },
        |epoch| SideEffectPhase::SubmissionUnknown {
            invocation_epoch: epoch,
        },
        |_| Ok(()),
    )?;
    Ok(())
}

fn apply_side_effect_receipt_observed(
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
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            event_id: envelope.event_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            required_previous: "submission_observed",
        },
        |epoch| SideEffectPhase::ReceiptObserved {
            invocation_epoch: epoch,
        },
        |projection| {
            projection.receipt = Some(SideEffectArtifactProjection {
                artifact_id: payload.receipt_artifact_id.clone(),
                content_digest: payload.receipt_hash.clone(),
                schema_id: Some(payload.receipt_schema_id.clone()),
            });
            if let Some(touched_set) = payload.resource_touched_set.clone() {
                projection.resource_touched_set = Some(touched_set);
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn apply_side_effect_confirmation_observed(
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
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            event_id: envelope.event_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            required_previous: "receipt",
        },
        |epoch| SideEffectPhase::ConfirmationObserved {
            invocation_epoch: epoch,
        },
        |projection| {
            projection.confirmation = Some(SideEffectArtifactProjection {
                artifact_id: payload.confirmation_artifact_id.clone(),
                content_digest: payload.confirmation_hash.clone(),
                schema_id: Some(payload.confirmation_schema_id.clone()),
            });
            if let Some(touched_set) = payload.resource_touched_set.clone() {
                projection.resource_touched_set = Some(touched_set);
            }
            Ok(())
        },
    )?;
    release_resource_lane_for_holder(
        projections,
        &SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
    );
    Ok(())
}

fn apply_side_effect_ambiguous(
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
            node_id: &payload.node_id,
            attempt_id: &payload.attempt_id,
            event_id: envelope.event_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            required_previous: "ambiguity_source",
        },
        |epoch| SideEffectPhase::Ambiguous {
            invocation_epoch: epoch,
        },
        |_| Ok(()),
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
                    ledger_key: payload.ledger_key.clone(),
                },
            },
        );
    }
    Ok(())
}

fn apply_side_effect_failed(
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
    release_resource_lane_for_holder(
        projections,
        &SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
    );
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

fn apply_cell_produced(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::CellProduced,
) -> Result<()> {
    if projections.cells.contains_key(&payload.cell_id) {
        return Err(StoreError::ProjectionConflict {
            key: format!("cell:{}:terminal", payload.cell_id),
            message: "cell already terminal".to_owned(),
        });
    }
    projections.cells.insert(
        payload.cell_id.clone(),
        CellTerminalProjection::Produced {
            event_id: event_id.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            schema_id: payload.schema_id.clone(),
            semantic_type_id: payload.semantic_type_id.clone(),
            artifact_id: payload.artifact_id.clone(),
            content_digest: payload.content_digest.clone(),
        },
    );
    Ok(())
}

fn apply_cell_skipped(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::CellSkipped,
) -> Result<()> {
    if projections.cells.contains_key(&payload.cell_id) {
        return Err(StoreError::ProjectionConflict {
            key: format!("cell:{}:terminal", payload.cell_id),
            message: "cell already terminal".to_owned(),
        });
    }
    projections.cells.insert(
        payload.cell_id.clone(),
        CellTerminalProjection::Skipped {
            event_id: event_id.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            schema_id: payload.schema_id.clone(),
            semantic_type_id: payload.semantic_type_id.clone(),
            skip_reason: payload.skip_reason.clone(),
        },
    );
    Ok(())
}

fn apply_public_output_produced(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    if matches!(
        projections.public_outputs.get(&payload.public_schema_id),
        Some(PublicOutputProjection::Produced { .. })
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("public_output:{}", payload.public_schema_id),
            message: "public output already projected".to_owned(),
        });
    }
    projections.public_outputs.insert(
        payload.public_schema_id.clone(),
        PublicOutputProjection::Produced {
            event_id: event_id.clone(),
            rendered_digest: payload.rendered_digest.clone(),
            rendered_artifact_id: payload.rendered_artifact_id.clone(),
        },
    );
    Ok(())
}

fn apply_public_output_render_failed(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    if matches!(
        projections.public_outputs.get(&payload.public_schema_id),
        Some(PublicOutputProjection::Produced { .. })
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("public_output:{}", payload.public_schema_id),
            message: "public output already produced".to_owned(),
        });
    }
    projections.public_outputs.insert(
        payload.public_schema_id.clone(),
        PublicOutputProjection::RenderFailed {
            event_id: event_id.clone(),
            error: Box::new(payload.error.clone()),
        },
    );
    Ok(())
}

fn apply_manual_resolution_recorded(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::ManualResolutionRecorded,
) -> Result<()> {
    if projections.manual_resolutions.contains_key(&payload.run_id) {
        return Err(StoreError::ProjectionConflict {
            key: "run:manual_resolution".to_owned(),
            message: "manual resolution already recorded".to_owned(),
        });
    }
    require_forward_quiescence(projections, &payload.run_id)?;
    projections.manual_resolutions.insert(
        payload.run_id.clone(),
        ManualResolutionProjection {
            event_id: event_id.clone(),
            outcome: payload.outcome,
            evidence_schema_id: payload.evidence_schema_id.clone(),
            evidence_hash: payload.evidence_hash.clone(),
            evidence_artifact_id: payload.evidence_artifact_id.clone(),
            authorization_schema_id: payload.authorization_schema_id.clone(),
            authorization_hash: payload.authorization_hash.clone(),
            authorization_artifact_id: payload.authorization_artifact_id.clone(),
            note: payload.note.clone(),
        },
    );
    release_resource_lanes_for_run(projections, &payload.run_id);
    Ok(())
}

fn apply_retention_refs_appended(
    projections: &mut ProjectionSnapshot,
    payload: &events::RetentionRefsAppended,
) -> Result<()> {
    if projections.run_state(&payload.run_id) == RunState::Absent {
        return Err(StoreError::ProjectionConflict {
            key: format!("retention:{}:refs", payload.run_id),
            message: "retention refs require a started run".to_owned(),
        });
    }
    let retention = projections
        .retentions
        .entry(payload.run_id.clone())
        .or_default();
    for retention_ref in &payload.refs {
        retention
            .refs
            .insert(retention_ref.artifact_id.clone(), retention_ref.clone());
    }
    Ok(())
}

fn apply_retention_manifest_projected(
    projections: &mut ProjectionSnapshot,
    payload: &events::RetentionManifestProjected,
) -> Result<()> {
    if projections.run_state(&payload.run_id) == RunState::Absent {
        return Err(StoreError::ProjectionConflict {
            key: format!("retention:{}:manifest", payload.run_id),
            message: "retention manifest requires a started run".to_owned(),
        });
    }
    let retention = projections
        .retentions
        .entry(payload.run_id.clone())
        .or_default();
    if retention.manifests.contains_key(&payload.manifest_seq) {
        return Err(StoreError::ProjectionConflict {
            key: format!(
                "retention:{}:manifest:{}",
                payload.run_id, payload.manifest_seq
            ),
            message: "manifest sequence already projected".to_owned(),
        });
    }
    match &retention.manifest {
        Some(previous) => {
            let expected_seq = previous.manifest_seq.checked_add(1).ok_or_else(|| {
                StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "manifest sequence overflow".to_owned(),
                }
            })?;
            if payload.manifest_seq != expected_seq {
                return Err(StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "manifest sequence must advance by one".to_owned(),
                });
            }
            if payload.previous_manifest_digest.as_ref() != Some(&previous.manifest_digest) {
                return Err(StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "manifest previous digest does not match latest projection".to_owned(),
                });
            }
        }
        None => {
            if payload.manifest_seq != 1 || payload.previous_manifest_digest.is_some() {
                return Err(StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "first manifest must use sequence 1 and no previous digest".to_owned(),
                });
            }
        }
    }
    let projection = RetentionManifestProjection {
        manifest_seq: payload.manifest_seq,
        manifest_digest: payload.manifest_digest.clone(),
        manifest_artifact_id: payload.manifest_artifact_id.clone(),
        previous_manifest_digest: payload.previous_manifest_digest.clone(),
    };
    retention
        .manifests
        .insert(payload.manifest_seq, projection.clone());
    retention.manifest = Some(projection);
    Ok(())
}

fn apply_fact_recorded(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::FactRecorded,
) -> Result<()> {
    match projections.attempt(&payload.node_id, &payload.attempt_id) {
        Some(AttemptProjection {
            status: AttemptStatus::Started { .. },
            ..
        }) => {}
        Some(_) => {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "fact:{}:{}:{}",
                    payload.node_id, payload.attempt_id, payload.fact_key
                ),
                message: "fact requires an active started attempt".to_owned(),
            });
        }
        None => {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "fact:{}:{}:{}",
                    payload.node_id, payload.attempt_id, payload.fact_key
                ),
                message: "fact requires a started attempt".to_owned(),
            });
        }
    }
    let key = (
        payload.node_id.clone(),
        payload.attempt_id.clone(),
        payload.fact_key.clone(),
    );
    if projections.facts.contains_key(&key) {
        return Err(StoreError::ProjectionConflict {
            key: format!(
                "fact:{}:{}:{}",
                payload.node_id, payload.attempt_id, payload.fact_key
            ),
            message: "fact already recorded for attempt".to_owned(),
        });
    }
    projections.facts.insert(
        key,
        FactProjection {
            event_id: event_id.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            fact_key: payload.fact_key.clone(),
            request_schema_id: payload.request_schema_id.clone(),
            request_hash: payload.request_hash.clone(),
            response_schema_id: payload.response_schema_id.clone(),
            response_hash: payload.response_hash.clone(),
            artifact_id: payload.artifact_id.clone(),
            capability_kind: payload.capability_kind.clone(),
            capability_version: payload.capability_version.clone(),
            adapter_kind: payload.adapter_kind.clone(),
            adapter_version: payload.adapter_version.clone(),
        },
    );
    Ok(())
}

fn update_attempt_terminal_projection(
    projections: &mut ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    event_id: EventId,
    status: AttemptStatus,
) -> Result<()> {
    let key = (node_id.clone(), attempt_id.clone());
    let Some(projection) = projections.attempts.get_mut(&key) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("attempt:{node_id}:{attempt_id}"),
            message: "attempt terminal event requires a started attempt".to_owned(),
        });
    };
    if !matches!(projection.status, AttemptStatus::Started { .. }) {
        return Err(StoreError::ProjectionConflict {
            key: format!("attempt:{node_id}:{attempt_id}"),
            message: "attempt already terminal".to_owned(),
        });
    }
    projection.event_id = event_id;
    projection.status = status;
    Ok(())
}
