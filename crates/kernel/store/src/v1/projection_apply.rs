use super::*;

pub(crate) fn fact_claim_projection_key(prefix: &str, claim_id: &mfm_facts::FactClaimId) -> String {
    format!(
        "{}:{}:{}:{}",
        prefix,
        claim_id.source_run_id(),
        claim_id.source_seq(),
        claim_id.source_ordinal()
    )
}

pub(crate) fn apply_projection(
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

pub(crate) fn apply_projection_for_external_fact_indexes(
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
    insert_run_admission_retention(retention, &payload.spec_artifact)?;
    insert_run_admission_retention(retention, &payload.certificate_artifact)?;
    for artifact in &payload.config_artifacts {
        insert_run_admission_retention(retention, artifact)?;
    }
    for seed in &payload.seed_cells {
        let evidence = ArtifactEvidenceRef {
            artifact_id: seed.seed_artifact.artifact_id.clone(),
            digest: seed.seed_artifact.content_digest.clone(),
            byte_len: seed.seed_artifact.byte_len,
            media_type: seed.seed_artifact.media_type.clone(),
            schema_id: Some(seed.seed_artifact.schema_id.clone()),
            semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
            producer_node_id: None,
            producer_seed_id: Some(seed.seed_id.clone()),
            artifact_role: seed.seed_artifact.role,
        };
        insert_retention_evidence(retention, evidence)?;
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
) -> Result<()> {
    insert_retention_evidence(retention, ArtifactEvidenceRef::from_run_artifact(artifact))
}

fn insert_retention_evidence(
    retention: &mut RetentionProjection,
    evidence: ArtifactEvidenceRef,
) -> Result<()> {
    let retention_ref = evidence.retention_ref()?;
    let key = (
        retention_ref.artifact_id.clone(),
        retention_ref.evidence_hash.clone(),
    );
    retention.refs.insert(key, retention_ref);
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

#[path = "projection_apply/side_effect.rs"]
mod side_effect_handlers;
use self::side_effect_handlers::{
    apply_resource_lane_claimed, apply_resource_lane_released, apply_side_effect_ambiguous,
    apply_side_effect_claim_taken_over, apply_side_effect_claimed,
    apply_side_effect_confirmation_observed, apply_side_effect_failed,
    apply_side_effect_intent_persisted, apply_side_effect_invocation_prepared,
    apply_side_effect_invocation_started, apply_side_effect_not_submitted_proven,
    apply_side_effect_receipt_observed, apply_side_effect_submission_observed,
    apply_side_effect_submission_unknown,
};

#[path = "projection_apply/fact.rs"]
mod fact_handlers;
use self::fact_handlers::{
    apply_fact_descriptor_artifact, apply_fact_recorded, apply_fact_recorded_record_only,
};

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
            evidence_hash: payload.evidence_hash.clone(),
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
        retention.refs.insert(
            (
                retention_ref.artifact_id.clone(),
                retention_ref.evidence_hash.clone(),
            ),
            retention_ref.clone(),
        );
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
