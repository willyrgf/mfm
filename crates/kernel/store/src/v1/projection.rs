use super::*;

pub(super) fn apply_projection(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    match envelope.payload() {
        KernelEventPayload::RunAdmitted(payload) => apply_run_admitted(
            projections,
            envelope.run_id(),
            &envelope.event_id,
            payload,
            artifact_bytes,
        )?,
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
            apply_cell_produced(projections, envelope.run_id(), &envelope.event_id, payload)?;
        }
        KernelEventPayload::CellSkipped(payload) => {
            apply_cell_skipped(projections, envelope.run_id(), &envelope.event_id, payload)?;
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
        KernelEventPayload::ResourceLaneClaimed(payload) => {
            apply_resource_lane_claimed(projections, envelope, payload)?;
        }
        KernelEventPayload::ResourceLaneClaimIntent(_)
        | KernelEventPayload::ResourceLaneReleaseIntent(_) => {
            return Err(StoreError::ProjectionConflict {
                key: "resource_lane:intent".to_owned(),
                message: "resource-lane intents must be store-filled before projection".to_owned(),
            });
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
        KernelEventPayload::ResourceLaneReleased(payload) => {
            apply_resource_lane_released(projections, envelope, payload)?;
        }
        KernelEventPayload::PublicOutputProduced(payload) => {
            apply_public_output_produced(
                projections,
                envelope.run_id(),
                &envelope.event_id,
                payload,
            )?;
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            apply_public_output_render_failed(
                projections,
                envelope.run_id(),
                &envelope.event_id,
                payload,
            )?;
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
            apply_fact_recorded(projections, envelope, payload, artifact_bytes)?;
        }
        KernelEventPayload::ArtifactReferenced(_) => {}
    }
    Ok(())
}

pub(super) fn apply_projection_for_external_fact_indexes(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
) -> Result<()> {
    match envelope.payload() {
        KernelEventPayload::RunAdmitted(payload) => {
            apply_run_admitted_base(projections, envelope.run_id(), &envelope.event_id, payload)?;
        }
        KernelEventPayload::FactRecorded(payload) => {
            apply_fact_recorded_record_only(projections, envelope, payload)?;
        }
        _ => apply_projection(projections, envelope, &ArtifactByteAuthorityMap::new())?,
    }
    Ok(())
}

fn apply_run_admitted(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::RunAdmitted,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    apply_run_admitted_base(projections, run_id, event_id, payload)?;
    for artifact in &payload.fact_descriptor_artifacts {
        apply_fact_descriptor_artifact(projections, artifact, artifact_bytes)?;
    }
    Ok(())
}

fn apply_run_admitted_base(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    _event_id: &EventId,
    payload: &events::RunAdmitted,
) -> Result<()> {
    validate_run_admitted_identity(run_id, payload)?;
    let state = projections.run_state(&payload.run_id);
    if state != RunState::Absent {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "run already admitted".to_owned(),
        });
    }
    projections
        .run_states
        .insert(payload.run_id.clone(), RunState::Started);
    projections
        .run_spec_hashes
        .insert(payload.run_id.clone(), payload.spec_hash.clone());
    projections
        .saga_policy_digests
        .insert(payload.run_id.clone(), payload.saga_policy_digest.clone());
    let retention = projections
        .retentions
        .entry(payload.run_id.clone())
        .or_default();
    insert_run_admission_retention(retention, &payload.spec_artifact);
    insert_run_admission_retention(retention, &payload.certificate_artifact);
    for artifact in &payload.config_artifacts {
        insert_run_admission_retention(retention, artifact);
    }
    for seed in &payload.seed_cells {
        retention.refs.insert(
            seed.seed_artifact.artifact_id.clone(),
            events::RetentionRef {
                artifact_id: seed.seed_artifact.artifact_id.clone(),
                role: seed.seed_artifact.role,
                content_digest: seed.seed_artifact.content_digest.clone(),
            },
        );
    }
    Ok(())
}

fn validate_run_admitted_identity(run_id: &RunId, payload: &events::RunAdmitted) -> Result<()> {
    if payload.run_id != *run_id {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "RunAdmitted run id does not match stream run id".to_owned(),
        });
    }
    if payload.identity_material.certified_spec_hash != payload.spec_hash {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "RunAdmitted identity material spec hash does not match event spec hash"
                .to_owned(),
        });
    }
    let derived =
        payload
            .identity_material
            .derive_run_id()
            .map_err(|_| StoreError::ProjectionConflict {
                key: "run:admission".to_owned(),
                message: "RunAdmitted identity material is invalid".to_owned(),
            })?;
    if derived != payload.run_id {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "RunAdmitted run id does not match identity material".to_owned(),
        });
    }
    Ok(())
}

fn insert_run_admission_retention(
    retention: &mut RetentionProjection,
    artifact: &events::RunArtifactEvidenceRef,
) {
    retention.refs.insert(
        artifact.artifact_id.clone(),
        events::RetentionRef {
            artifact_id: artifact.artifact_id.clone(),
            role: artifact.role,
            content_digest: artifact.content_digest.clone(),
        },
    );
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
    require_no_resource_lanes_for_run(projections, &payload.run_id, "run completion")?;
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

fn apply_side_effect_claimed(
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

fn apply_side_effect_claim_taken_over(
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

fn apply_side_effect_invocation_prepared(
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

fn apply_resource_lane_claimed(
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

fn apply_resource_lane_released(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::ResourceLaneReleased,
) -> Result<()> {
    release_resource_lane(projections, envelope.run_id(), payload)
}

fn apply_side_effect_invocation_started(
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
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::SubmitOrVerify,
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
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Submit),
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
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Verify),
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

fn apply_cell_produced(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::CellProduced,
) -> Result<()> {
    let key = (run_id.clone(), payload.cell_id.clone());
    if projections.cells.contains_key(&key) {
        return Err(StoreError::ProjectionConflict {
            key: format!("cell:{run_id}:{}:terminal", payload.cell_id),
            message: "cell already terminal".to_owned(),
        });
    }
    projections.cells.insert(
        key,
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
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::CellSkipped,
) -> Result<()> {
    let key = (run_id.clone(), payload.cell_id.clone());
    if projections.cells.contains_key(&key) {
        return Err(StoreError::ProjectionConflict {
            key: format!("cell:{run_id}:{}:terminal", payload.cell_id),
            message: "cell already terminal".to_owned(),
        });
    }
    projections.cells.insert(
        key,
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
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    let key = (run_id.clone(), payload.public_schema_id.clone());
    if matches!(
        projections.public_outputs.get(&key),
        Some(PublicOutputProjection::Produced { .. })
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("public_output:{run_id}:{}", payload.public_schema_id),
            message: "public output already projected".to_owned(),
        });
    }
    projections.public_outputs.insert(
        key,
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
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    let key = (run_id.clone(), payload.public_schema_id.clone());
    if matches!(
        projections.public_outputs.get(&key),
        Some(PublicOutputProjection::Produced { .. })
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("public_output:{run_id}:{}", payload.public_schema_id),
            message: "public output already produced".to_owned(),
        });
    }
    projections.public_outputs.insert(
        key,
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
    require_no_resource_lanes_for_run(projections, &payload.run_id, "manual resolution")?;
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
    envelope: &KernelEventEnvelope,
    payload: &events::FactRecorded,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    let claim = &payload.claim;
    let mut record = FactRecordProjection::from_recorded_event(envelope, payload, None)?;
    let claim_id = record.fact_claim_id.clone();
    require_started_fact_attempt(projections, payload, &claim_id)?;

    let descriptor_projection = projections
        .fact_descriptor(claim.fact_descriptor_hash())
        .cloned()
        .ok_or_else(|| StoreError::ProjectionConflict {
            key: format!("fact_descriptor:{}", claim.fact_descriptor_hash()),
            message: "fact descriptor must be admitted before recording a fact".to_owned(),
        })?;
    let descriptor = load_projected_fact_descriptor(&descriptor_projection, artifact_bytes)?;
    validate_fact_claim_against_descriptor(claim, &descriptor_projection)?;
    let subject_material = mfm_facts::parse_canonical_fact_subject_material_bytes(
        claim.subject().subject_material().as_bytes(),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;

    let response = claim.response();
    let (response_bytes, response_evidence) = require_artifact_bytes_by_key(
        artifact_bytes,
        response.artifact_id(),
        response.artifact_evidence_hash(),
    )?;
    validate_fact_response_evidence(response, response_evidence)?;
    record.response_artifact_evidence = Some(response_evidence.clone());

    insert_fact_record_projection(projections, record.clone())?;
    if !matches!(
        claim.visibility(),
        mfm_facts::FactVisibility::Indexed { .. }
    ) {
        return Ok(());
    }

    let recorded_at = fact_recorded_at(envelope);
    let store_commit_order = envelope.seq().as_u64();
    let index = FactIndexProjection::from_record_projection(
        &record,
        envelope.commit_key().clone(),
        store_commit_order,
        recorded_at.clone(),
    )?
    .ok_or_else(|| StoreError::ProjectionConflict {
        key: fact_claim_projection_key("fact_index", &claim_id),
        message: "indexed fact record did not produce an index projection".to_owned(),
    })?;
    let metadata = mfm_facts::FactExtractionMetadata::new(
        recorded_at.clone(),
        claim.observed_at().map(str::to_owned),
        store_commit_order,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;
    let response_value =
        mfm_facts::parse_canonical_fact_response_bytes(&descriptor, response_bytes)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
    let terms = mfm_facts::extract_terms_from_material(
        &descriptor,
        &subject_material,
        &response_value,
        &metadata,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;

    projections
        .fact_index_entries
        .insert(claim_id.clone(), index);
    for term in terms {
        let key = (claim_id.clone(), term.field_id().clone());
        if projections
            .fact_term_entries
            .insert(
                key.clone(),
                FactIndexTermProjection::from_extracted_term(
                    &claim_id,
                    claim.fact_descriptor_hash(),
                    &term,
                ),
            )
            .is_some()
        {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "{}:{}",
                    fact_claim_projection_key("fact_term", &claim_id),
                    key.1
                ),
                message: "duplicate fact term projection".to_owned(),
            });
        }
    }
    Ok(())
}

fn apply_fact_recorded_record_only(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::FactRecorded,
) -> Result<()> {
    let record = FactRecordProjection::from_recorded_event(envelope, payload, None)?;
    require_started_fact_attempt(projections, payload, &record.fact_claim_id)?;
    insert_fact_record_projection(projections, record)
}

fn require_started_fact_attempt(
    projections: &ProjectionSnapshot,
    payload: &events::FactRecorded,
    claim_id: &mfm_facts::FactClaimId,
) -> Result<()> {
    match projections.attempt(&payload.node_id, &payload.attempt_id) {
        Some(AttemptProjection {
            status: AttemptStatus::Started { .. },
            ..
        }) => Ok(()),
        Some(_) => Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact", claim_id),
            message: "fact requires an active started attempt".to_owned(),
        }),
        None => Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact", claim_id),
            message: "fact requires a started attempt".to_owned(),
        }),
    }
}

fn insert_fact_record_projection(
    projections: &mut ProjectionSnapshot,
    record: FactRecordProjection,
) -> Result<()> {
    let claim = &record.claim;
    let claim_id = &record.fact_claim_id;
    if projections.fact_records.values().any(|record| {
        record.claim.response().artifact_id() == claim.response().artifact_id()
            && record.claim.response().artifact_evidence_hash()
                == claim.response().artifact_evidence_hash()
    }) {
        return Err(StoreError::ProjectionConflict {
            key: format!(
                "fact_response:{}:{}",
                claim.response().artifact_id(),
                claim.response().artifact_evidence_hash()
            ),
            message: "response artifact is already bound to a fact claim".to_owned(),
        });
    }
    if projections.fact_records.contains_key(claim_id) {
        return Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact_record", claim_id),
            message: "fact claim id is already projected".to_owned(),
        });
    }
    projections
        .fact_records
        .insert(record.fact_claim_id.clone(), record);
    Ok(())
}

fn apply_fact_descriptor_artifact(
    projections: &mut ProjectionSnapshot,
    artifact: &events::RunArtifactEvidenceRef,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let evidence = ArtifactEvidenceRef::from_run_artifact(artifact);
    if evidence.artifact_role != ArtifactRole::FactDescriptor
        || evidence.schema_id.as_ref() != Some(&expected_schema)
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "fact_descriptor",
        });
    }
    let bytes = require_artifact_bytes_exact(artifact_bytes, &evidence)?;
    let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    if descriptor_hash != evidence.digest {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "digest",
        });
    }
    let namespace_hash = mfm_facts::fact_subject_namespace_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let projection = FactDescriptorProjection {
        descriptor_hash: descriptor_hash.clone(),
        descriptor_artifact_id: artifact.artifact_id.clone(),
        descriptor_artifact_evidence: evidence.clone(),
        fact_kind: descriptor.fact_kind().clone(),
        descriptor_schema_id: descriptor.descriptor_schema_id().clone(),
        subject_schema_id: descriptor.subject_schema_id().clone(),
        response_schema_id: descriptor.response_schema_id().clone(),
        fact_subject_namespace_hash: namespace_hash,
    };
    match projections
        .fact_descriptors
        .insert(descriptor_hash.clone(), projection.clone())
    {
        Some(existing) if equivalent_fact_descriptor_projection(&existing, &projection) => {
            projections
                .fact_descriptors
                .insert(descriptor_hash, existing);
        }
        Some(_) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("fact_descriptor:{descriptor_hash}"),
                message: "conflicting fact descriptor projection".to_owned(),
            });
        }
        None => {}
    }
    Ok(())
}

fn equivalent_fact_descriptor_projection(
    left: &FactDescriptorProjection,
    right: &FactDescriptorProjection,
) -> bool {
    left.descriptor_hash == right.descriptor_hash
        && left.descriptor_artifact_id == right.descriptor_artifact_id
        && left.descriptor_artifact_evidence == right.descriptor_artifact_evidence
        && left.fact_kind == right.fact_kind
        && left.descriptor_schema_id == right.descriptor_schema_id
        && left.subject_schema_id == right.subject_schema_id
        && left.response_schema_id == right.response_schema_id
        && left.fact_subject_namespace_hash == right.fact_subject_namespace_hash
}

fn load_projected_fact_descriptor(
    projection: &FactDescriptorProjection,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<mfm_facts::FactDescriptor> {
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    for ((artifact_id, _), (bytes, evidence)) in artifact_bytes {
        if artifact_id != &projection.descriptor_artifact_id {
            continue;
        }
        if evidence.digest != projection.descriptor_hash {
            continue;
        }
        if evidence.artifact_role != ArtifactRole::FactDescriptor
            || evidence.schema_id.as_ref() != Some(&expected_schema)
        {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "fact_descriptor",
            });
        }
        let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        if descriptor_hash != projection.descriptor_hash {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "digest",
            });
        }
        return Ok(descriptor);
    }
    Err(StoreError::MissingArtifact {
        artifact_id: projection.descriptor_artifact_id.clone(),
    })
}

fn validate_fact_claim_against_descriptor(
    claim: &mfm_facts::FactClaim,
    descriptor: &FactDescriptorProjection,
) -> Result<()> {
    if claim.fact_descriptor_hash() != &descriptor.descriptor_hash
        || claim.fact_kind() != &descriptor.fact_kind
        || claim.subject().fact_subject_namespace_hash() != &descriptor.fact_subject_namespace_hash
        || claim.response().response_schema_id() != &descriptor.response_schema_id
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("fact_descriptor:{}", claim.fact_descriptor_hash()),
            message: "fact claim does not match admitted descriptor".to_owned(),
        });
    }
    Ok(())
}

fn validate_fact_response_evidence(
    response: &mfm_facts::FactResponseEvidence,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if evidence.artifact_role != ArtifactRole::FactResponse
        || evidence.schema_id.as_ref() != Some(response.response_schema_id())
        || &evidence.digest != response.response_hash()
        || &evidence.artifact_id != response.artifact_id()
        || evidence.evidence_hash()? != *response.artifact_evidence_hash()
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: response.artifact_id().clone(),
            field: "fact_response",
        });
    }
    Ok(())
}

fn require_artifact_bytes_exact<'a>(
    artifact_bytes: &'a ArtifactByteAuthorityMap,
    evidence: &ArtifactEvidenceRef,
) -> Result<&'a [u8]> {
    let evidence_hash = evidence.evidence_hash()?;
    let Some((bytes, stored_evidence)) =
        artifact_bytes.get(&(evidence.artifact_id.clone(), evidence_hash))
    else {
        return Err(StoreError::MissingArtifact {
            artifact_id: evidence.artifact_id.clone(),
        });
    };
    if stored_evidence != evidence {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "artifact",
        });
    }
    super::verify_retained_artifact_bytes(bytes, evidence)?;
    Ok(bytes.as_slice())
}

fn require_artifact_bytes_by_key<'a>(
    artifact_bytes: &'a ArtifactByteAuthorityMap,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<(&'a [u8], &'a ArtifactEvidenceRef)> {
    let Some((bytes, evidence)) = artifact_bytes.get(&(artifact_id.clone(), evidence_hash.clone()))
    else {
        return Err(StoreError::MissingArtifact {
            artifact_id: artifact_id.clone(),
        });
    };
    super::verify_retained_artifact_bytes(bytes, evidence)?;
    Ok((bytes.as_slice(), evidence))
}

fn fact_recorded_at(_envelope: &KernelEventEnvelope) -> String {
    "1970-01-01T00:00:00Z".to_owned()
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
