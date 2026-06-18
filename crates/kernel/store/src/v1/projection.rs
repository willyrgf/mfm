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
            apply_attempt_started(projections, envelope.run_id(), &envelope.event_id, payload)?
        }
        KernelEventPayload::StateAttemptCompleted(payload) => {
            apply_attempt_completed(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            apply_attempt_interrupted(projections, &envelope.event_id, payload)?;
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
    projections
        .saga_policy_digests
        .insert(payload.run_id.clone(), payload.saga_policy_digest.clone());
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
    run_id: &RunId,
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
            run_id: run_id.clone(),
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

fn apply_attempt_interrupted(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::StateAttemptInterrupted,
) -> Result<()> {
    require_no_prepared_side_effect_authority_for_interruption(projections, payload)?;
    update_attempt_terminal_projection(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        event_id.clone(),
        AttemptStatus::Interrupted,
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

fn require_no_prepared_side_effect_authority_for_interruption(
    projections: &ProjectionSnapshot,
    payload: &events::StateAttemptInterrupted,
) -> Result<()> {
    for side_effect in projections.side_effects.values() {
        if side_effect.intent.node_id != payload.node_id
            || side_effect.intent.attempt_id != payload.attempt_id
        {
            continue;
        }
        let ledger_state = side_effect.ledger_state()?;
        if side_effect_phase_blocks_standalone_interruption(ledger_state.phase()) {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{}:{}", payload.node_id, payload.attempt_id),
                message: "interruption is not legal after side-effect invocation was prepared"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

fn side_effect_phase_blocks_standalone_interruption(phase: SideEffectLedgerPhase<'_>) -> bool {
    !matches!(
        phase,
        SideEffectLedgerPhase::IntentPersisted { .. } | SideEffectLedgerPhase::Claimed { .. }
    )
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
        OwnedSideEffectLedgerState::intent_persisted(
            envelope.run_id().clone(),
            payload.ledger_key.clone(),
            payload.ledger_purpose.clone(),
            envelope.event_id.clone(),
            intent,
            payload.invocation_epoch,
        )
        .into_projection(),
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
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .claim(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        projection,
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
        |state| {
            matches!(
                state.phase(),
                SideEffectLedgerPhase::Claimed { .. } | SideEffectLedgerPhase::Prepared { .. }
            )
        },
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .take_over(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        projection,
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
        |state| matches!(state.phase(), SideEffectLedgerPhase::Claimed { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    let prepared_invocation = prepared_invocation_projection(
        &payload.prepared_artifact_id,
        &payload.prepared_hash,
        &payload.ledger_key,
    )?
    .or_else(|| previous.prepared_invocation.clone());
    let previous_projection = previous.clone();
    let resource_key =
        acquire_resource_lane(projections, envelope.run_id(), &envelope.event_id, payload)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous_projection)?
        .prepare(
            envelope.event_id.clone(),
            payload,
            prepared_invocation,
            resource_key,
        )?
        .into_projection();
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        projection,
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
        |state| matches!(state.phase(), SideEffectLedgerPhase::Prepared { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .start(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
        projection,
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
            required_previous: "submission_recovery",
        },
        |state| state.mark_not_submitted(envelope.event_id.clone(), payload),
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
            required_previous: "submission_recovery",
        },
        |state| state.record_submission(envelope.event_id.clone(), payload),
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
            required_previous: "submission_recovery",
        },
        |state| state.mark_submission_unknown(envelope.event_id.clone(), payload),
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
            required_previous: "submission_observed",
        },
        |state| state.record_receipt(envelope.event_id.clone(), payload),
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
            required_previous: "receipt",
        },
        |state| state.confirm(envelope.event_id.clone(), payload),
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
            required_previous: "ambiguity_source",
        },
        |state| state.mark_ambiguous(envelope.event_id.clone(), payload),
    )?;
    release_resource_lane_for_holder(
        projections,
        &SideEffectLedgerRef::new(envelope.run_id().clone(), payload.ledger_key.clone()),
    );
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
    projections.require_no_open_semantic_attempts_for_run(&payload.run_id)?;
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
