use super::*;

/// Returns canonical JSON bytes for a typed event payload.
pub fn payload_canonical_json(payload: &KernelEventPayload) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(payload_json(payload))
}

pub(super) fn kernel_event_envelope_json(envelope: &KernelEventEnvelope) -> serde_json::Value {
    serde_json::json!({
        "commit_key": envelope.commit_key().as_str(),
        "event_id": envelope.event_id().as_str(),
        "event_schema_id": envelope.event_schema_id().as_str(),
        "logical_key": envelope.logical_key().as_str(),
        "ordinal": envelope.ordinal().as_u32(),
        "payload": payload_json(envelope.payload()),
        "payload_hash": envelope.payload_hash().as_str(),
        "run_id": envelope.run_id().as_str(),
        "seq": envelope.seq().as_u64(),
        "store_commit_order": envelope.store_commit_order().as_u64(),
        "spec_hash": envelope.spec_hash().as_str(),
    })
}

pub(super) fn parse_kernel_event_envelope(json: &serde_json::Value) -> Result<KernelEventEnvelope> {
    let seq = StreamSeq::new(required_u64(json, "seq")?)?;
    let store_commit_order = StoreCommitOrder::new(required_u64(json, "store_commit_order")?);
    let ordinal = CommitOrdinal::new(required_u32(json, "ordinal")?);
    let payload = payload_from_json_value(required_obj(json, "payload")?)?;
    KernelEventEnvelope::from_persisted_record(PersistedKernelEventRecord {
        event_id: parse_identity(required_str(json, "event_id")?)?,
        event_schema_id: parse_identity(required_str(json, "event_schema_id")?)?,
        run_id: parse_identity(required_str(json, "run_id")?)?,
        seq,
        store_commit_order,
        ordinal,
        spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
        commit_key: CommitKey::new(required_str(json, "commit_key")?)?,
        logical_key: LogicalEventKey::new(required_str(json, "logical_key")?)?,
        payload_hash: parse_identity(required_str(json, "payload_hash")?)?,
        payload,
    })
}

/// Computes the canonical idempotency fingerprint for a purpose-specific prepared commit plan.
///
/// The fingerprint intentionally excludes `expected_next_seq`, so idempotent retries can be
/// recognized before stale sequence checks as required by the store contract. It covers the
/// artifact evidence admitted atomically with the commit.
pub fn prepared_commit_plan_fingerprint(plan: &PreparedCommitPlan) -> Result<CommitFingerprint> {
    let request = plan.request();
    let canonical = canonical_json(serde_json::json!({
        "admitted_artifacts": sorted_store_artifacts_json(plan.admitted_artifacts()),
        "commit_key": request.commit_key.as_str(),
        "payloads": request.payloads.iter().map(payload_json).collect::<Vec<_>>(),
        "preconditions": preconditions_json(&request.preconditions),
        "required_artifacts": sorted_store_artifacts_json(&request.required_artifacts),
        "run_id": request.run_id.as_str(),
    }))?;
    Ok(CommitFingerprint(canonical.content_digest()))
}

fn sorted_store_artifacts_json(artifacts: &[ArtifactEvidenceRef]) -> Vec<serde_json::Value> {
    let mut artifacts = artifacts
        .iter()
        .map(store_artifact_json)
        .collect::<Vec<_>>();
    artifacts.sort_by_key(serde_json::Value::to_string);
    artifacts
}

fn payload_json(payload: &KernelEventPayload) -> serde_json::Value {
    match payload {
        KernelEventPayload::RunAdmitted(payload) => serde_json::json!({
            "admitted_binding_digest": payload.admitted_binding_digest.as_str(),
            "canonicalizer_identity": payload.canonicalizer_identity.as_str(),
            "certificate_artifact": run_artifact_json(&payload.certificate_artifact),
            "config_artifacts": payload.config_artifacts.iter().map(run_artifact_json).collect::<Vec<_>>(),
            "descriptor_identities": payload.descriptor_identities.iter().map(descriptor_identity_json).collect::<Vec<_>>(),
            "fact_descriptor_artifacts": payload.fact_descriptor_artifacts.iter().map(run_artifact_json).collect::<Vec<_>>(),
            "adapter_executables": payload.adapter_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
            "entry_point": entry_point_launch_evidence_json(&payload.entry_point),
            "identity_material": run_identity_material_json(&payload.identity_material),
            "lowering_version": payload.lowering_version.as_str(),
            "public_output_schema_id": payload.public_output_schema_id.as_str(),
            "run_id": payload.run_id.as_str(),
            "runner_executables": payload.runner_executables.iter().map(executable_identity_json).collect::<Vec<_>>(),
            "saga_policy_digest": payload.saga_policy_digest.as_str(),
            "seed_cells": payload.seed_cells.iter().map(seed_cell_ref_json).collect::<Vec<_>>(),
            "spec_artifact": run_artifact_json(&payload.spec_artifact),
            "spec_hash": payload.spec_hash.as_str(),
            "spec_version": payload.spec_version.as_str(),
            "variant": "RunAdmitted",
        }),
        KernelEventPayload::StateAttemptStarted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "attempt_no": payload.attempt_no,
            "node_id": payload.node_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "state_kind": payload.state_kind.as_str(),
            "state_version": payload.state_version.as_str(),
            "variant": "StateAttemptStarted",
        }),
        KernelEventPayload::FactRecorded(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim": fact_claim_json(&payload.claim),
            "node_id": payload.node_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "FactRecorded",
        }),
        KernelEventPayload::ArtifactReferenced(payload) => serde_json::json!({
            "artifact_ref": event_artifact_json(&payload.artifact_ref),
            "attempt_id": payload.attempt_id.as_ref().map(AttemptId::as_str),
            "node_id": payload.node_id.as_ref().map(NodeId::as_str),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ArtifactReferenced",
        }),
        KernelEventPayload::CellProduced(payload) => serde_json::json!({
            "artifact_id": payload.artifact_id.as_str(),
            "attempt_id": payload.attempt_id.as_str(),
            "cell_id": payload.cell_id.as_str(),
            "context": cell_context_json(&payload.context),
            "content_digest": payload.content_digest.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "node_id": payload.node_id.as_str(),
            "producer_state_kind": payload.producer_state_kind.as_ref().map(|value| value.as_str()),
            "producer_state_version": payload.producer_state_version.as_ref().map(|value| value.as_str()),
            "schema_id": payload.schema_id.as_str(),
            "scope_id": payload.scope_id.as_str(),
            "semantic_type_id": payload.semantic_type_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "value_lineage": value_lineage_json(&payload.value_lineage),
            "variant": "CellProduced",
        }),
        KernelEventPayload::CellSkipped(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "cell_id": payload.cell_id.as_str(),
            "context": cell_context_json(&payload.context),
            "node_id": payload.node_id.as_str(),
            "schema_id": payload.schema_id.as_str(),
            "scope_id": payload.scope_id.as_str(),
            "semantic_type_id": payload.semantic_type_id.as_str(),
            "skip_reason": skip_reason_json(&payload.skip_reason),
            "spec_hash": payload.spec_hash.as_str(),
            "value_lineage": value_lineage_json(&payload.value_lineage),
            "variant": "CellSkipped",
        }),
        KernelEventPayload::SideEffectIntentPersisted(payload) => serde_json::json!({
            "adapter_kind": payload.adapter_kind.as_str(),
            "adapter_version": payload.adapter_version.as_str(),
            "attempt_id": payload.attempt_id.as_str(),
            "capability_kind": payload.capability_kind.as_str(),
            "capability_version": payload.capability_version.as_str(),
            "idempotency_input_hash": payload.idempotency_input_hash.as_str(),
            "idempotency_input_schema_id": payload.idempotency_input_schema_id.as_str(),
            "idempotency_key": payload.idempotency_key.as_str(),
            "intent_artifact_evidence_hash": payload.intent_artifact_evidence_hash.as_str(),
            "intent_artifact_id": payload.intent_artifact_id.as_str(),
            "intent_hash": payload.intent_hash.as_str(),
            "intent_schema_id": payload.intent_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "scope_id": payload.scope_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectIntentPersisted",
        }),
        KernelEventPayload::SideEffectClaimed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "claim_owner": payload.claim_owner.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectClaimed",
        }),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "new_claim_owner": payload.new_claim_owner.as_str(),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "previous_claim_generation": payload.previous_claim_generation,
            "previous_claim_owner": payload.previous_claim_owner.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectClaimTakenOver",
        }),
        KernelEventPayload::ResourceLaneClaimed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token,
            "claim_id": payload.claim_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "lane_transition_seq": payload.lane_transition_seq,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "requirement_digest": payload.requirement_digest.as_str(),
            "resolved_by_capability_impl": payload.resolved_by_capability_impl.as_str(),
            "resource_key": resource_key_evidence_json(&payload.resource_key),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneClaimed",
        }),
        KernelEventPayload::ResourceLaneClaimIntent(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "requirement_digest": payload.requirement_digest.as_str(),
            "resolved_by_capability_impl": payload.resolved_by_capability_impl.as_str(),
            "resource_key": resource_key_evidence_json(&payload.resource_key),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneClaimIntent",
        }),
        KernelEventPayload::SideEffectInvocationPrepared(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "resource_key": payload.resource_key.as_ref().map(resource_key_evidence_json),
            "prepared_artifact_evidence_hash": payload
                .prepared_artifact_evidence_hash
                .as_ref()
                .map(ContentDigest::as_str),
            "prepared_artifact_id": payload.prepared_artifact_id.as_ref().map(ArtifactId::as_str),
            "prepared_hash": payload.prepared_hash.as_ref().map(ContentDigest::as_str),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectInvocationPrepared",
        }),
        KernelEventPayload::SideEffectInvocationStarted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "claim_fencing_token": payload.claim_fencing_token.as_str(),
            "claim_generation": payload.claim_generation,
            "claim_owner": payload.claim_owner.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectInvocationStarted",
        }),
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "proof_artifact_evidence_hash": payload.proof_artifact_evidence_hash.as_str(),
            "proof_artifact_id": payload.proof_artifact_id.as_str(),
            "proof_hash": payload.proof_hash.as_str(),
            "proof_schema_id": payload.proof_schema_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectNotSubmittedProven",
        }),
        KernelEventPayload::SideEffectSubmissionObserved(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "submission_artifact_evidence_hash": payload.submission_artifact_evidence_hash.as_str(),
            "submission_artifact_id": payload.submission_artifact_id.as_str(),
            "submission_hash": payload.submission_hash.as_str(),
            "submission_schema_id": payload.submission_schema_id.as_str(),
            "variant": "SideEffectSubmissionObserved",
        }),
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "evidence_artifact_evidence_hash": payload.evidence_artifact_evidence_hash.as_str(),
            "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "evidence_schema_id": payload.evidence_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectSubmissionUnknown",
        }),
        KernelEventPayload::SideEffectReceiptObserved(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "receipt_artifact_evidence_hash": payload.receipt_artifact_evidence_hash.as_str(),
            "receipt_artifact_id": payload.receipt_artifact_id.as_str(),
            "receipt_hash": payload.receipt_hash.as_str(),
            "receipt_schema_id": payload.receipt_schema_id.as_str(),
            "replay_verifier_id": payload.replay_verifier_id.as_str(),
            "resource_touched_set": payload.resource_touched_set.as_ref().map(resource_touched_set_evidence_json),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectReceiptObserved",
        }),
        KernelEventPayload::SideEffectConfirmationObserved(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "confirmation_artifact_evidence_hash": payload
                .confirmation_artifact_evidence_hash
                .as_str(),
            "confirmation_artifact_id": payload.confirmation_artifact_id.as_str(),
            "confirmation_hash": payload.confirmation_hash.as_str(),
            "confirmation_schema_id": payload.confirmation_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "replay_verifier_id": payload.replay_verifier_id.as_str(),
            "resource_touched_set": payload.resource_touched_set.as_ref().map(resource_touched_set_evidence_json),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectConfirmationObserved",
        }),
        KernelEventPayload::SideEffectAmbiguous(payload) => serde_json::json!({
            "ambiguity_code": payload.ambiguity_code.as_str(),
            "attempt_id": payload.attempt_id.as_str(),
            "evidence_artifact_evidence_hash": payload.evidence_artifact_evidence_hash.as_str(),
            "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "evidence_schema_id": payload.evidence_schema_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectAmbiguous",
        }),
        KernelEventPayload::SideEffectFailed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "error": error_info_json(&payload.error),
            "failure_phase": failure_phase_str(payload.failure_phase),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "node_id": payload.node_id.as_str(),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "retryable": payload.retryable,
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "SideEffectFailed",
        }),
        KernelEventPayload::ResourceLaneReleased(payload) => serde_json::json!({
            "claim_fencing_token": payload.claim_fencing_token,
            "claim_id": payload.claim_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "lane_transition_seq": payload.lane_transition_seq,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "release_authority": payload.release_authority.as_str(),
            "release_id": payload.release_id.as_str(),
            "release_reason": payload.release_reason.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneReleased",
        }),
        KernelEventPayload::ResourceLaneReleaseIntent(payload) => serde_json::json!({
            "claim_id": payload.claim_id.as_str(),
            "invocation_epoch": payload.invocation_epoch,
            "ledger_key": payload.ledger_key.as_str(),
            "ledger_purpose": side_effect_ledger_purpose_json(&payload.ledger_purpose),
            "pair_id": payload.pair_id.as_str(),
            "pair_role": payload.pair_role.as_str(),
            "release_authority": payload.release_authority.as_str(),
            "release_reason": payload.release_reason.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ResourceLaneReleaseIntent",
        }),
        KernelEventPayload::PublicOutputProduced(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "cells": payload.cells.iter().map(named_cell_ref_json).collect::<Vec<_>>(),
            "node_id": payload.node_id.as_str(),
            "output_spec_digest": payload.output_spec_digest.as_str(),
            "public_schema_id": payload.public_schema_id.as_str(),
            "receipt_cell_id": payload.receipt_cell_id.as_str(),
            "rendered_artifact_evidence_hash": payload
                .rendered_artifact_evidence_hash
                .as_ref()
                .map(ContentDigest::as_str),
            "rendered_artifact_id": payload.rendered_artifact_id.as_ref().map(ArtifactId::as_str),
            "rendered_digest": payload.rendered_digest.as_str(),
            "renderer_descriptor_id": payload.renderer_descriptor_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "PublicOutputProduced",
        }),
        KernelEventPayload::PublicOutputRenderFailed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "error": error_info_json(&payload.error),
            "node_id": payload.node_id.as_str(),
            "public_schema_id": payload.public_schema_id.as_str(),
            "renderer_descriptor_id": payload.renderer_descriptor_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "PublicOutputRenderFailed",
        }),
        KernelEventPayload::StateAttemptCompleted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "node_id": payload.node_id.as_str(),
            "output_cell_id": payload.output_cell_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "StateAttemptCompleted",
        }),
        KernelEventPayload::StateAttemptInterrupted(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "node_id": payload.node_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "StateAttemptInterrupted",
        }),
        KernelEventPayload::StateAttemptFailed(payload) => serde_json::json!({
            "attempt_id": payload.attempt_id.as_str(),
            "error": error_info_json(&payload.error),
            "node_id": payload.node_id.as_str(),
            "retryable": payload.retryable,
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "StateAttemptFailed",
        }),
        KernelEventPayload::ManualResolutionRecorded(payload) => serde_json::json!({
            "authorization_artifact_evidence_hash": payload
                .authorization_artifact_evidence_hash
                .as_str(),
            "authorization_artifact_id": payload.authorization_artifact_id.as_str(),
            "authorization_hash": payload.authorization_hash.as_str(),
            "authorization_schema_id": payload.authorization_schema_id.as_str(),
            "evidence_artifact_evidence_hash": payload.evidence_artifact_evidence_hash.as_str(),
            "evidence_artifact_id": payload.evidence_artifact_id.as_str(),
            "evidence_hash": payload.evidence_hash.as_str(),
            "evidence_schema_id": payload.evidence_schema_id.as_str(),
            "note": payload.note.as_ref().map(manual_resolution_note_json),
            "outcome": manual_resolution_outcome_str(payload.outcome),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "ManualResolutionRecorded",
        }),
        KernelEventPayload::RunCompleted(payload) => serde_json::json!({
            "outcome": run_completion_outcome_json(&payload.outcome),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "RunCompleted",
        }),
        KernelEventPayload::RetentionRefsAppended(payload) => serde_json::json!({
            "reason": retention_reason_str(payload.reason),
            "refs": payload.refs.iter().map(retention_ref_json).collect::<Vec<_>>(),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "RetentionRefsAppended",
        }),
        KernelEventPayload::RetentionManifestProjected(payload) => serde_json::json!({
            "manifest_artifact_evidence_hash": payload.manifest_artifact_evidence_hash.as_str(),
            "manifest_artifact_id": payload.manifest_artifact_id.as_str(),
            "manifest_digest": payload.manifest_digest.as_str(),
            "manifest_seq": payload.manifest_seq,
            "previous_manifest_digest": payload.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
            "run_id": payload.run_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": "RetentionManifestProjected",
        }),
    }
}

/// Parses a typed kernel event payload from its store canonical JSON shape.
pub fn payload_from_json_value(json: &serde_json::Value) -> Result<KernelEventPayload> {
    match required_str(json, "variant")? {
        "RunAdmitted" => Ok(KernelEventPayload::RunAdmitted(Box::new(
            events::RunAdmitted {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                identity_material: parse_run_identity_material(required_obj(
                    json,
                    "identity_material",
                )?)?,
                entry_point: parse_entry_point_launch_evidence(required_obj(json, "entry_point")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                spec_artifact: parse_run_artifact(required_obj(json, "spec_artifact")?)?,
                certificate_artifact: parse_run_artifact(required_obj(
                    json,
                    "certificate_artifact",
                )?)?,
                config_artifacts: parse_vec(json, "config_artifacts", parse_run_artifact)?,
                fact_descriptor_artifacts: parse_vec(
                    json,
                    "fact_descriptor_artifacts",
                    parse_run_artifact,
                )?,
                spec_version: parse_identity(required_str(json, "spec_version")?)?,
                lowering_version: parse_identity(required_str(json, "lowering_version")?)?,
                public_output_schema_id: parse_identity(required_str(
                    json,
                    "public_output_schema_id",
                )?)?,
                saga_policy_digest: parse_identity(required_str(json, "saga_policy_digest")?)?,
                descriptor_identities: parse_vec(json, "descriptor_identities", |item| {
                    parse_descriptor_identity(item)
                })?,
                runner_executables: parse_vec(json, "runner_executables", parse_executable)?,
                adapter_executables: parse_vec(json, "adapter_executables", parse_executable)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
                admitted_binding_digest: parse_identity(required_str(
                    json,
                    "admitted_binding_digest",
                )?)?,
                seed_cells: parse_vec(json, "seed_cells", parse_seed_cell_ref)?,
            },
        ))),
        "StateAttemptStarted" => Ok(KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                attempt_no: required_u32(json, "attempt_no")?,
                state_kind: parse_identity(required_str(json, "state_kind")?)?,
                state_version: parse_identity(required_str(json, "state_version")?)?,
            },
        )),
        "FactRecorded" => Ok(KernelEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            claim: parse_fact_claim(required_obj(json, "claim")?)?,
        })),
        "ArtifactReferenced" => Ok(KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: optional_str(json, "node_id")?
                    .map(parse_identity)
                    .transpose()?,
                attempt_id: optional_str(json, "attempt_id")?
                    .map(parse_identity)
                    .transpose()?,
                artifact_ref: parse_event_artifact(required_obj(json, "artifact_ref")?)?,
            },
        )),
        "CellProduced" => Ok(KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            cell_id: parse_identity(required_str(json, "cell_id")?)?,
            scope_id: parse_identity(required_str(json, "scope_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
            context: parse_cell_context(required_obj(json, "context")?)?,
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
            producer_state_kind: optional_str(json, "producer_state_kind")?
                .map(parse_identity)
                .transpose()?,
            producer_state_version: optional_str(json, "producer_state_version")?
                .map(parse_identity)
                .transpose()?,
        })),
        "CellSkipped" => Ok(KernelEventPayload::CellSkipped(events::CellSkipped {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            cell_id: parse_identity(required_str(json, "cell_id")?)?,
            scope_id: parse_identity(required_str(json, "scope_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
            context: parse_cell_context(required_obj(json, "context")?)?,
            skip_reason: parse_skip_reason(required_obj(json, "skip_reason")?)?,
        })),
        "SideEffectIntentPersisted" => Ok(KernelEventPayload::SideEffectIntentPersisted(
            side_effect::IntentPersisted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                scope_id: parse_identity(required_str(json, "scope_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                intent_schema_id: parse_identity(required_str(json, "intent_schema_id")?)?,
                intent_hash: parse_identity(required_str(json, "intent_hash")?)?,
                intent_artifact_id: parse_identity(required_str(json, "intent_artifact_id")?)?,
                intent_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "intent_artifact_evidence_hash",
                )?)?,
                idempotency_input_schema_id: parse_identity(required_str(
                    json,
                    "idempotency_input_schema_id",
                )?)?,
                idempotency_input_hash: parse_identity(required_str(
                    json,
                    "idempotency_input_hash",
                )?)?,
                idempotency_key: events::IdempotencyKeyRef::new(required_str(
                    json,
                    "idempotency_key",
                )?)?,
                capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
                capability_version: parse_identity(required_str(json, "capability_version")?)?,
                adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
                adapter_version: parse_identity(required_str(json, "adapter_version")?)?,
            },
        )),
        "SideEffectClaimed" => Ok(KernelEventPayload::SideEffectClaimed(
            side_effect::Claimed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "SideEffectClaimTakenOver" => Ok(KernelEventPayload::SideEffectClaimTakenOver(
            side_effect::ClaimTakenOver {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                previous_claim_owner: events::RunnerInvocationId::new(required_str(
                    json,
                    "previous_claim_owner",
                )?)?,
                new_claim_owner: events::RunnerInvocationId::new(required_str(
                    json,
                    "new_claim_owner",
                )?)?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                previous_claim_generation: required_u32(json, "previous_claim_generation")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "ResourceLaneClaimed" => Ok(KernelEventPayload::ResourceLaneClaimed(
            events::ResourceLaneClaimed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                resource_key: parse_resource_key_evidence(required_obj(json, "resource_key")?)?,
                requirement_digest: parse_identity(required_str(json, "requirement_digest")?)?,
                resolved_by_capability_impl: events::RunnerFactoryId::new(required_str(
                    json,
                    "resolved_by_capability_impl",
                )?)?,
                claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
                claim_fencing_token: required_u64(json, "claim_fencing_token")?,
                lane_transition_seq: required_u64(json, "lane_transition_seq")?,
            },
        )),
        "SideEffectInvocationPrepared" => Ok(KernelEventPayload::SideEffectInvocationPrepared(
            side_effect::InvocationPrepared {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
                resource_key: optional_obj(json, "resource_key")?
                    .map(parse_resource_key_evidence)
                    .transpose()?,
                prepared_artifact_id: optional_str(json, "prepared_artifact_id")?
                    .map(parse_identity)
                    .transpose()?,
                prepared_hash: optional_str(json, "prepared_hash")?
                    .map(parse_identity)
                    .transpose()?,
                prepared_artifact_evidence_hash: optional_str(
                    json,
                    "prepared_artifact_evidence_hash",
                )?
                .map(parse_identity)
                .transpose()?,
            },
        )),
        "SideEffectInvocationStarted" => Ok(KernelEventPayload::SideEffectInvocationStarted(
            side_effect::InvocationStarted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
                claim_generation: required_u32(json, "claim_generation")?,
                claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                    json,
                    "claim_fencing_token",
                )?)?,
            },
        )),
        "SideEffectNotSubmittedProven" => Ok(KernelEventPayload::SideEffectNotSubmittedProven(
            side_effect::NotSubmittedProven {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                proof_schema_id: parse_identity(required_str(json, "proof_schema_id")?)?,
                proof_hash: parse_identity(required_str(json, "proof_hash")?)?,
                proof_artifact_id: parse_identity(required_str(json, "proof_artifact_id")?)?,
                proof_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "proof_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectSubmissionObserved" => Ok(KernelEventPayload::SideEffectSubmissionObserved(
            side_effect::SubmissionObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                submission_schema_id: parse_identity(required_str(json, "submission_schema_id")?)?,
                submission_hash: parse_identity(required_str(json, "submission_hash")?)?,
                submission_artifact_id: parse_identity(required_str(
                    json,
                    "submission_artifact_id",
                )?)?,
                submission_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "submission_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectSubmissionUnknown" => Ok(KernelEventPayload::SideEffectSubmissionUnknown(
            side_effect::SubmissionUnknown {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectReceiptObserved" => Ok(KernelEventPayload::SideEffectReceiptObserved(
            side_effect::ReceiptObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                receipt_schema_id: parse_identity(required_str(json, "receipt_schema_id")?)?,
                receipt_hash: parse_identity(required_str(json, "receipt_hash")?)?,
                receipt_artifact_id: parse_identity(required_str(json, "receipt_artifact_id")?)?,
                receipt_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "receipt_artifact_evidence_hash",
                )?)?,
                replay_verifier_id: events::ReplayVerifierId::new(required_str(
                    json,
                    "replay_verifier_id",
                )?)?,
                resource_touched_set: optional_obj(json, "resource_touched_set")?
                    .map(parse_resource_touched_set_evidence)
                    .transpose()?,
            },
        )),
        "SideEffectConfirmationObserved" => Ok(KernelEventPayload::SideEffectConfirmationObserved(
            side_effect::ConfirmationObserved {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                confirmation_schema_id: parse_identity(required_str(
                    json,
                    "confirmation_schema_id",
                )?)?,
                confirmation_hash: parse_identity(required_str(json, "confirmation_hash")?)?,
                confirmation_artifact_id: parse_identity(required_str(
                    json,
                    "confirmation_artifact_id",
                )?)?,
                confirmation_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "confirmation_artifact_evidence_hash",
                )?)?,
                replay_verifier_id: events::ReplayVerifierId::new(required_str(
                    json,
                    "replay_verifier_id",
                )?)?,
                resource_touched_set: optional_obj(json, "resource_touched_set")?
                    .map(parse_resource_touched_set_evidence)
                    .transpose()?,
            },
        )),
        "SideEffectAmbiguous" => Ok(KernelEventPayload::SideEffectAmbiguous(
            side_effect::Ambiguous {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                ambiguity_code: events::AmbiguityCode::new(required_str(json, "ambiguity_code")?)?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
            },
        )),
        "SideEffectFailed" => Ok(KernelEventPayload::SideEffectFailed(side_effect::Failed {
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
            ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                json,
                "ledger_purpose",
            )?)?,
            pair_id: parse_identity(required_str(json, "pair_id")?)?,
            pair_role: parse_side_effect_pair_role(json, "pair_role")?,
            invocation_epoch: required_u32(json, "invocation_epoch")?,
            failure_phase: parse_failure_phase(required_str(json, "failure_phase")?)?,
            retryable: required_bool(json, "retryable")?,
            error: parse_error_info(required_obj(json, "error")?)?,
        })),
        "ResourceLaneReleased" => Ok(KernelEventPayload::ResourceLaneReleased(
            events::ResourceLaneReleased {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
                ledger_purpose: parse_side_effect_ledger_purpose(required_obj(
                    json,
                    "ledger_purpose",
                )?)?,
                pair_id: parse_identity(required_str(json, "pair_id")?)?,
                pair_role: parse_side_effect_pair_role(json, "pair_role")?,
                invocation_epoch: required_u32(json, "invocation_epoch")?,
                claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
                release_id: events::ResourceLaneReleaseId::new(required_str(json, "release_id")?)?,
                claim_fencing_token: required_u64(json, "claim_fencing_token")?,
                release_authority: parse_resource_lane_release_authority(required_str(
                    json,
                    "release_authority",
                )?)?,
                release_reason: events::ResourceLaneReleaseReason::new(required_str(
                    json,
                    "release_reason",
                )?)?,
                lane_transition_seq: required_u64(json, "lane_transition_seq")?,
            },
        )),
        "PublicOutputProduced" => Ok(KernelEventPayload::PublicOutputProduced(
            events::PublicOutputProduced {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                receipt_cell_id: parse_identity(required_str(json, "receipt_cell_id")?)?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                output_spec_digest: parse_identity(required_str(json, "output_spec_digest")?)?,
                cells: parse_vec(json, "cells", parse_named_cell_ref)?,
                rendered_digest: parse_identity(required_str(json, "rendered_digest")?)?,
                rendered_artifact_id: optional_str(json, "rendered_artifact_id")?
                    .map(parse_identity)
                    .transpose()?,
                rendered_artifact_evidence_hash: optional_str(
                    json,
                    "rendered_artifact_evidence_hash",
                )?
                .map(parse_identity)
                .transpose()?,
                renderer_descriptor_id: parse_identity(required_str(
                    json,
                    "renderer_descriptor_id",
                )?)?,
            },
        )),
        "PublicOutputRenderFailed" => Ok(KernelEventPayload::PublicOutputRenderFailed(
            events::PublicOutputRenderFailed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                renderer_descriptor_id: parse_identity(required_str(
                    json,
                    "renderer_descriptor_id",
                )?)?,
                error: parse_error_info(required_obj(json, "error")?)?,
            },
        )),
        "StateAttemptCompleted" => Ok(KernelEventPayload::StateAttemptCompleted(
            events::StateAttemptCompleted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                output_cell_id: parse_identity(required_str(json, "output_cell_id")?)?,
            },
        )),
        "StateAttemptInterrupted" => Ok(KernelEventPayload::StateAttemptInterrupted(
            events::StateAttemptInterrupted {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            },
        )),
        "StateAttemptFailed" => Ok(KernelEventPayload::StateAttemptFailed(
            events::StateAttemptFailed {
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                node_id: parse_identity(required_str(json, "node_id")?)?,
                attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
                retryable: required_bool(json, "retryable")?,
                error: parse_error_info(required_obj(json, "error")?)?,
            },
        )),
        "ManualResolutionRecorded" => Ok(KernelEventPayload::ManualResolutionRecorded(
            events::ManualResolutionRecorded {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                outcome: parse_manual_resolution_outcome(required_str(json, "outcome")?)?,
                evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
                evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
                evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
                evidence_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "evidence_artifact_evidence_hash",
                )?)?,
                authorization_schema_id: parse_identity(required_str(
                    json,
                    "authorization_schema_id",
                )?)?,
                authorization_hash: parse_identity(required_str(json, "authorization_hash")?)?,
                authorization_artifact_id: parse_identity(required_str(
                    json,
                    "authorization_artifact_id",
                )?)?,
                authorization_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "authorization_artifact_evidence_hash",
                )?)?,
                note: optional_obj(json, "note")?
                    .map(parse_manual_resolution_note)
                    .transpose()?,
            },
        )),
        "RunCompleted" => Ok(KernelEventPayload::RunCompleted(events::RunCompleted {
            run_id: parse_identity(required_str(json, "run_id")?)?,
            spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
            outcome: parse_run_completion_outcome(required_obj(json, "outcome")?)?,
        })),
        "RetentionRefsAppended" => Ok(KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                refs: parse_vec(json, "refs", parse_retention_ref)?,
                reason: parse_retention_reason(required_str(json, "reason")?)?,
            },
        )),
        "RetentionManifestProjected" => Ok(KernelEventPayload::RetentionManifestProjected(
            events::RetentionManifestProjected {
                run_id: parse_identity(required_str(json, "run_id")?)?,
                spec_hash: parse_identity(required_str(json, "spec_hash")?)?,
                manifest_seq: required_u64(json, "manifest_seq")?,
                manifest_digest: parse_identity(required_str(json, "manifest_digest")?)?,
                previous_manifest_digest: optional_str(json, "previous_manifest_digest")?
                    .map(parse_identity)
                    .transpose()?,
                manifest_artifact_id: parse_identity(required_str(json, "manifest_artifact_id")?)?,
                manifest_artifact_evidence_hash: parse_identity(required_str(
                    json,
                    "manifest_artifact_evidence_hash",
                )?)?,
            },
        )),
        other => Err(StoreError::Event(format!(
            "unknown event payload variant {other}"
        ))),
    }
}

fn parse_descriptor_identity(json: &serde_json::Value) -> Result<DescriptorIdentity> {
    match required_str(json, "descriptor_family")? {
        "state" => Ok(DescriptorIdentity::State(Box::new(
            StateDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                name: required_str(json, "name")?.to_owned(),
                state_kind: parse_identity(required_str(json, "state_kind")?)?,
                state_version: parse_identity(required_str(json, "state_version")?)?,
                context: parse_state_context_descriptor(required_obj(json, "context")?)?,
                input_context: parse_state_input_context_contract(required_obj(
                    json,
                    "input_context",
                )?)?,
                output_context: parse_state_output_context_contract(required_obj(
                    json,
                    "output_context",
                )?)?,
                config_schema_id: parse_identity(required_str(json, "config_schema_id")?)?,
                input_schema_id: parse_identity(required_str(json, "input_schema_id")?)?,
                output_schema_id: parse_identity(required_str(json, "output_schema_id")?)?,
                output_semantic_type_id: parse_identity(required_str(
                    json,
                    "output_semantic_type_id",
                )?)?,
                effect_kind: parse_identity(required_str(json, "effect_kind")?)?,
                effect_class: required_str(json, "effect_class")?.to_owned(),
                effect_name: required_str(json, "effect_name")?.to_owned(),
                effect_version: parse_identity(required_str(json, "effect_version")?)?,
                capabilities: parse_capability_set(required_obj(json, "capabilities")?)?,
                emitted_fact_descriptors: parse_vec(
                    json,
                    "emitted_fact_descriptors",
                    parse_fact_descriptor_ref,
                )?,
                runner: required_str(json, "runner")?.to_owned(),
                side_effect_contract_digest: optional_str(json, "side_effect_contract_digest")?
                    .map(parse_identity)
                    .transpose()?,
            },
        ))),
        "operation" => Ok(DescriptorIdentity::Operation(Box::new(
            OperationDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                name: required_str(json, "name")?.to_owned(),
                operation_kind: parse_identity(required_str(json, "operation_kind")?)?,
                operation_version: parse_identity(required_str(json, "operation_version")?)?,
                config_schema_id: parse_identity(required_str(json, "config_schema_id")?)?,
                input_schema_id: parse_identity(required_str(json, "input_schema_id")?)?,
                output_schema_id: parse_identity(required_str(json, "output_schema_id")?)?,
                expansion_abi: required_str(json, "expansion_abi")?.to_owned(),
            },
        ))),
        "renderer" => Ok(DescriptorIdentity::Renderer(Box::new(
            RendererDescriptorIdentity {
                descriptor_id: parse_identity(required_str(json, "descriptor_id")?)?,
                renderer_kind: RendererKind::new(required_str(json, "renderer_kind")?)
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                renderer_version: RendererVersion::new(required_str(json, "renderer_version")?)
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                public_schema_id: parse_identity(required_str(json, "public_schema_id")?)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            },
        ))),
        other => Err(StoreError::Identity(format!(
            "unknown descriptor identity family {other}"
        ))),
    }
}

fn parse_state_context_descriptor(
    json: &serde_json::Value,
) -> Result<spec::StateContextDescriptorSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateContextDescriptorSpec::no_context()),
        "required" => Ok(spec::StateContextDescriptorSpec::Required(Box::new(
            spec::StateContextDescriptorRequirementSpec {
                context_descriptor_id: parse_identity(required_str(
                    json,
                    "context_descriptor_id",
                )?)?,
                schema_id: parse_identity(required_str(json, "schema_id")?)?,
                semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
                canonicalizer_identity: CanonicalizerIdentity::new(required_str(
                    json,
                    "canonicalizer_identity",
                )?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            },
        ))),
        kind => Err(StoreError::Event(format!(
            "unknown state context descriptor kind {kind}"
        ))),
    }
}

fn parse_context_producer(json: &serde_json::Value) -> Result<spec::ContextProducerSpec> {
    Ok(spec::ContextProducerSpec {
        producer_descriptor_ids: required_array(json, "producer_descriptor_ids")?
            .iter()
            .map(|value| {
                let raw = value.as_str().ok_or_else(|| {
                    StoreError::Event("producer_descriptor_ids entries must be strings".to_owned())
                })?;
                parse_identity(raw).map_err(StoreError::from)
            })
            .collect::<Result<Vec<_>>>()?,
        seed_producers_allowed: required_bool(json, "seed_producers_allowed")?,
    })
}

fn parse_state_input_context_contract(
    json: &serde_json::Value,
) -> Result<spec::StateInputContextContractSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateInputContextContractSpec::no_context()),
        "required" => Ok(spec::StateInputContextContractSpec::Required {
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required_obj(json, "producer")?)?),
        }),
        kind => Err(StoreError::Event(format!(
            "unknown state input context contract kind {kind}"
        ))),
    }
}

fn parse_state_output_context_contract(
    json: &serde_json::Value,
) -> Result<spec::StateOutputContextContractSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::StateOutputContextContractSpec::no_context()),
        "produces" => Ok(spec::StateOutputContextContractSpec::Produces {
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
        }),
        kind => Err(StoreError::Event(format!(
            "unknown state output context contract kind {kind}"
        ))),
    }
}

fn parse_cell_context(json: &serde_json::Value) -> Result<spec::CellContextSpec> {
    match required_str(json, "kind")? {
        "no_context" => Ok(spec::CellContextSpec::no_context()),
        "bound" => Ok(spec::CellContextSpec::Bound {
            context_ref: parse_identity(required_str(json, "context_ref")?)?,
            resource_kind: ContextResourceKind::new(required_str(json, "resource_kind")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            stage: ContextStage::new(required_str(json, "stage")?)
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            producer: Box::new(parse_context_producer(required_obj(json, "producer")?)?),
        }),
        kind => Err(StoreError::Event(format!(
            "unknown cell context kind {kind}"
        ))),
    }
}

fn parse_capability_set(json: &serde_json::Value) -> Result<CapabilitySetDescriptor> {
    let capabilities = required_array(json, "capabilities")?
        .iter()
        .map(parse_capability)
        .collect::<Result<Vec<_>>>()?;
    Ok(CapabilitySetDescriptor::new(capabilities)?)
}

fn parse_capability(json: &serde_json::Value) -> Result<CapabilityDescriptor> {
    Ok(CapabilityDescriptor::new(
        parse_identity::<CapabilityKind>(required_str(json, "kind")?)?,
        parse_identity::<CapabilityVersion>(required_str(json, "version")?)?,
        parse_capability_role(required_str(json, "role")?)?,
        required_str(json, "name")?.to_owned(),
    )?)
}

fn parse_executable(json: &serde_json::Value) -> Result<events::ExecutableIdentity> {
    Ok(events::ExecutableIdentity {
        factory_id: events::RunnerFactoryId::new(required_str(json, "factory_id")?)?,
        cargo_package_digest: parse_identity(required_str(json, "cargo_package_digest")?)?,
        binary_digest: parse_identity(required_str(json, "binary_digest")?)?,
        nix_derivation_hash: optional_str(json, "nix_derivation_hash")?
            .map(events::NixDerivationHash::new)
            .transpose()?,
        nix_output_hash: optional_str(json, "nix_output_hash")?
            .map(events::NixOutputHash::new)
            .transpose()?,
    })
}

fn parse_seed_cell_ref(json: &serde_json::Value) -> Result<events::SeedCellRef> {
    Ok(events::SeedCellRef {
        seed_id: parse_identity(required_str(json, "seed_id")?)?,
        cell_id: parse_identity(required_str(json, "cell_id")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        digest: parse_identity(required_str(json, "digest")?)?,
        seed_artifact: parse_event_artifact(required_obj(json, "seed_artifact")?)?,
    })
}

fn parse_named_cell_ref(json: &serde_json::Value) -> Result<events::NamedTypedCellRef> {
    Ok(events::NamedTypedCellRef {
        public_field_path: PublicFieldPath::new(required_str(json, "public_field_path")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
        cell_id: parse_identity(required_str(json, "cell_id")?)?,
        producer: parse_cell_producer(required_obj(json, "producer")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        value_lineage: parse_value_lineage(required_obj(json, "value_lineage")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
    })
}

fn parse_cell_producer(json: &serde_json::Value) -> Result<CellProducer> {
    match required_str(json, "kind")? {
        "node" => Ok(CellProducer::Node(parse_identity(required_str(
            json, "node_id",
        )?)?)),
        "seed" => Ok(CellProducer::Seed(parse_identity(required_str(
            json, "seed_id",
        )?)?)),
        other => Err(StoreError::Identity(format!(
            "unknown cell producer {other}"
        ))),
    }
}

fn parse_value_lineage(json: &serde_json::Value) -> Result<ValueLineageRef> {
    Ok(ValueLineageRef {
        lineage_digest: parse_identity(required_str(json, "lineage_digest")?)?,
    })
}

/// Parses an artifact evidence reference from canonical JSON.
pub fn parse_event_artifact(json: &serde_json::Value) -> CodecResult<events::ArtifactEvidenceRef> {
    Ok(events::ArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        semantic_type_id: optional_str(json, "semantic_type_id")?
            .map(parse_identity)
            .transpose()?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
    })
}

fn parse_fact_claim(json: &serde_json::Value) -> Result<mfm_facts::FactClaim> {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: parse_fact_visibility(required_obj(json, "visibility")?)?,
        fact_kind: mfm_facts::FactKind::new(required_str(json, "fact_kind")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
        fact_descriptor_hash: parse_identity(required_str(json, "fact_descriptor_hash")?)?,
        subject: parse_fact_subject_evidence(required_obj(json, "subject")?)?,
        observed_at: optional_str(json, "observed_at")?.map(str::to_owned),
        request: optional_obj(json, "request")?
            .map(parse_fact_request_evidence)
            .transpose()?,
        response: parse_fact_response_evidence(required_obj(json, "response")?)?,
        producer: parse_fact_producer_provenance(required_obj(json, "producer")?)?,
    })
    .map_err(|error| StoreError::Identity(error.to_string()))
}

fn parse_fact_visibility(json: &serde_json::Value) -> Result<mfm_facts::FactVisibility> {
    match required_str(json, "kind")? {
        "run_private" => Ok(mfm_facts::FactVisibility::RunPrivate),
        "indexed" => Ok(mfm_facts::FactVisibility::Indexed {
            audience: parse_fact_audience(required_str(json, "audience")?)?,
            scope: parse_fact_visibility_scope(required_str(json, "scope")?)?,
        }),
        other => Err(StoreError::Identity(format!(
            "unknown fact visibility {other}"
        ))),
    }
}

fn parse_fact_audience(value: &str) -> Result<mfm_facts::FactAudience> {
    value
        .parse::<mfm_facts::FactAudience>()
        .map_err(|_| StoreError::Identity(format!("unknown fact audience {value}")))
}

fn parse_fact_visibility_scope(value: &str) -> Result<mfm_facts::FactVisibilityScope> {
    value
        .parse::<mfm_facts::FactVisibilityScope>()
        .map_err(|_| StoreError::Identity(format!("unknown fact visibility scope {value}")))
}

fn parse_fact_subject_evidence(json: &serde_json::Value) -> Result<mfm_facts::FactSubjectEvidence> {
    mfm_facts::FactSubjectEvidence::new(
        parse_identity(required_str(json, "fact_subject_namespace_hash")?)?,
        PlainCanonicalJsonBytes::from_canonical_json_slice(
            required_str(json, "subject_material")?.as_bytes(),
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?,
        parse_identity(required_str(json, "subject_material_hash")?)?,
        mfm_facts::FactKey::from_digest(parse_identity(required_str(json, "fact_key")?)?),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))
}

fn parse_fact_request_evidence(json: &serde_json::Value) -> Result<mfm_facts::FactRequestEvidence> {
    Ok(mfm_facts::FactRequestEvidence::new(
        parse_identity(required_str(json, "request_schema_id")?)?,
        parse_identity(required_str(json, "request_hash")?)?,
    ))
}

fn parse_fact_response_evidence(
    json: &serde_json::Value,
) -> Result<mfm_facts::FactResponseEvidence> {
    Ok(mfm_facts::FactResponseEvidence::new(
        parse_identity(required_str(json, "response_schema_id")?)?,
        parse_identity(required_str(json, "response_hash")?)?,
        parse_identity(required_str(json, "artifact_id")?)?,
        parse_identity(required_str(json, "artifact_evidence_hash")?)?,
    ))
}

fn parse_fact_producer_provenance(
    json: &serde_json::Value,
) -> Result<mfm_facts::FactProducerProvenance> {
    Ok(mfm_facts::FactProducerProvenance::new(
        parse_identity(required_str(json, "capability_kind")?)?,
        required_str(json, "capability_version")?.parse()?,
        parse_identity(required_str(json, "adapter_kind")?)?,
        required_str(json, "adapter_version")?.parse()?,
    ))
}

fn parse_run_artifact(json: &serde_json::Value) -> Result<events::RunArtifactEvidenceRef> {
    Ok(events::RunArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        schema_id: optional_str(json, "schema_id")?
            .map(parse_identity)
            .transpose()?,
        semantic_type_id: optional_str(json, "semantic_type_id")?
            .map(parse_identity)
            .transpose()?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)
            .map_err(|error| StoreError::Identity(error.to_string()))?,
    })
}

fn parse_entry_point_launch_evidence(
    json: &serde_json::Value,
) -> Result<events::EntryPointLaunchEvidence> {
    Ok(events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new(required_str(json, "resolved_op_id")?)?,
        entry_point_registry_digest: parse_identity(required_str(
            json,
            "entry_point_registry_digest",
        )?)?,
    })
}

fn parse_run_identity_material(json: &serde_json::Value) -> Result<events::RunIdentityMaterialV1> {
    Ok(events::RunIdentityMaterialV1 {
        certified_spec_hash: parse_identity(required_str(json, "certified_spec_hash")?)?,
        store_scope_id: StoreScopeId::new(required_str(json, "store_scope_id")?)?,
        invocation_key_digest: parse_identity(required_str(json, "invocation_key_digest")?)?,
    })
}

/// Parses a cell skip reason from canonical JSON.
pub fn parse_skip_reason(json: &serde_json::Value) -> CodecResult<events::SkipReason> {
    Ok(events::SkipReason {
        code: events::ErrorCode::new(required_str(json, "code")?)?,
        safe_message: required_str(json, "safe_message")?.to_owned(),
    })
}

/// Parses a side-effect ledger purpose from canonical JSON.
pub fn parse_side_effect_ledger_purpose(
    json: &serde_json::Value,
) -> CodecResult<events::SideEffectLedgerPurpose> {
    match required_str(json, "kind")? {
        "forward" => Ok(events::SideEffectLedgerPurpose::Forward),
        "remediation" => Ok(events::SideEffectLedgerPurpose::Remediation {
            forward_pair_id: parse_identity(required_str(json, "forward_pair_id")?)?,
        }),
        other => Err(CodecError::Identity(format!(
            "unknown side-effect ledger purpose {other}"
        ))),
    }
}

fn parse_side_effect_pair_role(
    json: &serde_json::Value,
    field: &'static str,
) -> CodecResult<events::SideEffectPairRole> {
    let value = required_str(json, field)?;
    events::SideEffectPairRole::parse(value)
        .ok_or_else(|| CodecError::Identity(format!("unknown side-effect pair role {value}")))
}

/// Parses exclusive resource-key evidence from canonical JSON.
pub fn parse_resource_key_evidence(
    json: &serde_json::Value,
) -> CodecResult<events::ResourceKeyEvidence> {
    Ok(events::ResourceKeyEvidence {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        key_schema_id: parse_identity(required_str(json, "key_schema_id")?)?,
        key: events::ResourceKey::new(required_str(json, "key")?)?,
    })
}

/// Parses touched-set resource evidence from canonical JSON.
pub fn parse_resource_touched_set_evidence(
    json: &serde_json::Value,
) -> CodecResult<events::ResourceTouchedSetEvidence> {
    Ok(events::ResourceTouchedSetEvidence {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
        evidence_artifact_evidence_hash: parse_identity(required_str(
            json,
            "evidence_artifact_evidence_hash",
        )?)?,
    })
}

/// Parses a manual resolution outcome tag.
pub fn parse_manual_resolution_outcome(
    value: &str,
) -> CodecResult<events::ManualResolutionOutcome> {
    match value {
        "confirm_remediated" => Ok(events::ManualResolutionOutcome::ConfirmRemediated),
        "fail_without_acdc_claim" => Ok(events::ManualResolutionOutcome::FailWithoutAcdcClaim),
        other => Err(CodecError::Identity(format!(
            "unknown manual resolution outcome {other}"
        ))),
    }
}

/// Parses a manual resolution note from canonical JSON.
pub fn parse_manual_resolution_note(
    json: &serde_json::Value,
) -> CodecResult<events::ManualResolutionNote> {
    Ok(events::ManualResolutionNote::new(required_str(
        json, "text",
    )?)?)
}

/// Parses structured error info from canonical JSON.
pub fn parse_error_info(json: &serde_json::Value) -> CodecResult<events::MfmErrorInfo> {
    let public_details = optional_obj(json, "public_details")?
        .map(|details| {
            Ok::<events::RedactedJson, CodecError>(events::RedactedJson::new(parse_identity(
                required_str(details, "content_digest")?,
            )?))
        })
        .transpose()?;
    let diagnostic_ref = optional_obj(json, "diagnostic_ref")?
        .map(parse_event_artifact)
        .transpose()?;
    let mut error = events::MfmErrorInfo::new(
        events::ErrorCode::new(required_str(json, "code")?)?,
        parse_error_category(required_str(json, "category")?)?,
        required_bool(json, "retryable")?,
        required_str(json, "safe_message")?,
    )?;
    if let Some(public_details) = public_details {
        error = error.with_public_details(public_details)?;
    }
    if let Some(diagnostic_ref) = diagnostic_ref {
        error = error.with_diagnostic_ref(diagnostic_ref)?;
    }
    Ok(error)
}

/// Parses a run completion outcome from canonical JSON.
pub fn parse_run_completion_outcome(
    json: &serde_json::Value,
) -> CodecResult<events::RunCompletionOutcome> {
    match required_str(json, "kind")? {
        "completed" => {
            let evidence = required_obj(json, "public_output")?;
            Ok(events::RunCompletionOutcome::Completed(Box::new(
                events::PublicOutputCompletionEvidence {
                    public_output_schema_id: parse_identity(required_str(
                        evidence,
                        "public_output_schema_id",
                    )?)?,
                    public_output_event_id: parse_identity(required_str(
                        evidence,
                        "public_output_event_id",
                    )?)?,
                },
            )))
        }
        "compensated" => Ok(events::RunCompletionOutcome::Compensated),
        "manually_resolved" => Ok(events::RunCompletionOutcome::ManuallyResolved),
        "failed_without_acdc_claim" => Ok(events::RunCompletionOutcome::FailedWithoutAcdcClaim),
        other => Err(CodecError::Identity(format!(
            "unknown run completion outcome {other}"
        ))),
    }
}

fn parse_fact_descriptor_ref(json: &serde_json::Value) -> Result<spec::FactDescriptorRef> {
    Ok(spec::FactDescriptorRef {
        descriptor_hash: parse_identity(required_str(json, "descriptor_hash")?)?,
    })
}

fn parse_retention_ref(json: &serde_json::Value) -> Result<events::RetentionRef> {
    Ok(events::RetentionRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: decode_artifact_role_tag(required_str(json, "role")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
    })
}

fn decode_artifact_role_tag(value: &str) -> CodecResult<ArtifactRole> {
    ArtifactRole::parse(value)
        .ok_or_else(|| CodecError::Identity(format!("unknown artifact role {value}")))
}

fn parse_capability_role(value: &str) -> Result<CapabilityRole> {
    match value {
        "read_external" => Ok(CapabilityRole::ReadExternal),
        "managed_platform_write" => Ok(CapabilityRole::ManagedPlatformWrite),
        "support" => Ok(CapabilityRole::Support),
        "external_mutation_authority" => Ok(CapabilityRole::ExternalMutationAuthority),
        other => Err(StoreError::Identity(format!(
            "unknown capability role {other}"
        ))),
    }
}

/// Parses an error category tag.
pub fn parse_error_category(value: &str) -> CodecResult<events::ErrorCategory> {
    match value {
        "planning" => Ok(events::ErrorCategory::Planning),
        "validation" => Ok(events::ErrorCategory::Validation),
        "capability" => Ok(events::ErrorCategory::Capability),
        "side_effect" => Ok(events::ErrorCategory::SideEffect),
        "runtime" => Ok(events::ErrorCategory::Runtime),
        "storage" => Ok(events::ErrorCategory::Storage),
        "cancelled" => Ok(events::ErrorCategory::Cancelled),
        other => Err(CodecError::Identity(format!(
            "unknown error category {other}"
        ))),
    }
}

/// Parses a side-effect failure phase tag.
pub fn parse_failure_phase(value: &str) -> CodecResult<side_effect::FailurePhase> {
    match value {
        "before_invocation_started" => Ok(side_effect::FailurePhase::BeforeInvocationStarted),
        "after_not_submitted_proven" => Ok(side_effect::FailurePhase::AfterNotSubmittedProven),
        other => Err(CodecError::Identity(format!(
            "unknown failure phase {other}"
        ))),
    }
}

fn parse_retention_reason(value: &str) -> Result<events::RetentionReason> {
    match value {
        "run_admitted" => Ok(events::RetentionReason::RunAdmitted),
        "runtime_evidence" => Ok(events::RetentionReason::RuntimeEvidence),
        "public_output" => Ok(events::RetentionReason::PublicOutput),
        "manifest_projection" => Ok(events::RetentionReason::ManifestProjection),
        other => Err(StoreError::Identity(format!(
            "unknown retention reason {other}"
        ))),
    }
}

fn parse_resource_lane_release_authority(
    value: &str,
) -> Result<events::ResourceLaneReleaseAuthority> {
    events::ResourceLaneReleaseAuthority::parse(value).map_err(|error| {
        StoreError::Identity(format!("invalid resource lane release authority: {error}"))
    })
}

pub(super) fn parse_vec<T>(
    json: &serde_json::Value,
    field: &'static str,
    parser: impl Fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    required_array(json, field)?.iter().map(parser).collect()
}

fn required_array<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> Result<&'a [serde_json::Value]> {
    json.get(field)
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| StoreError::Event(format!("missing array field {field}")))
}

fn preconditions_json(preconditions: &CommitPreconditions) -> serde_json::Value {
    let mut absent = preconditions
        .required_absent_logical_keys
        .iter()
        .map(|key| key.as_str())
        .collect::<Vec<_>>();
    absent.sort_unstable();
    let mut present = preconditions
        .required_present_logical_keys
        .iter()
        .map(|key| key.as_str())
        .collect::<Vec<_>>();
    present.sort_unstable();
    let mut cells = preconditions
        .required_cell_states
        .iter()
        .map(|precondition| {
            serde_json::json!({
                "cell_id": precondition.cell_id.as_str(),
                "required": required_cell_state_str(precondition.required),
            })
        })
        .collect::<Vec<_>>();
    cells.sort_by_key(serde_json::Value::to_string);
    let mut side_effects = preconditions
        .required_side_effect_states
        .iter()
        .map(|precondition| {
            serde_json::json!({
                "pair_id": precondition.pair_id.as_str(),
                "required": required_side_effect_state_str(precondition.required),
            })
        })
        .collect::<Vec<_>>();
    side_effects.sort_by_key(serde_json::Value::to_string);
    serde_json::json!({
        "required_absent_logical_keys": absent,
        "required_cell_states": cells,
        "required_present_logical_keys": present,
        "required_public_output_absent": preconditions.required_public_output_absent,
        "required_run_state": required_run_state_str(preconditions.required_run_state),
        "required_side_effect_states": side_effects,
        "certified_run_authority": preconditions.certified_run_authority.as_ref().map(certified_run_authority_json),
    })
}

fn certified_run_authority_json(token: &CertifiedRunStoreAuthority) -> serde_json::Value {
    serde_json::json!({
        "run_id": token.run_id().as_str(),
        "saga_policy_digest": token.saga_policy_digest().as_str(),
        "spec_hash": token.spec_hash().as_str(),
    })
}

pub(super) fn store_artifact_json(evidence: &ArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "artifact_role": evidence.artifact_role.as_str(),
        "byte_len": evidence.byte_len,
        "digest": evidence.digest.as_str(),
        "media_type": evidence.media_type.as_str(),
        "producer_node_id": evidence.producer_node_id.as_ref().map(NodeId::as_str),
        "producer_seed_id": evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        "schema_id": evidence.schema_id.as_ref().map(SchemaId::as_str),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

/// Encodes an artifact evidence reference as canonical JSON.
pub fn event_artifact_json(evidence: &events::ArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "byte_len": evidence.byte_len,
        "content_digest": evidence.content_digest.as_str(),
        "evidence_hash": evidence.evidence_hash.as_str(),
        "media_type": evidence.media_type.as_str(),
        "role": evidence.role.as_str(),
        "schema_id": evidence.schema_id.as_str(),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

fn fact_claim_json(claim: &mfm_facts::FactClaim) -> serde_json::Value {
    serde_json::json!({
        "visibility": fact_visibility_json(claim.visibility()),
        "fact_kind": claim.fact_kind().as_str(),
        "fact_descriptor_hash": claim.fact_descriptor_hash().as_str(),
        "subject": fact_subject_evidence_json(claim.subject()),
        "observed_at": claim.observed_at(),
        "request": claim.request().map(fact_request_evidence_json),
        "response": fact_response_evidence_json(claim.response()),
        "producer": fact_producer_provenance_json(claim.producer()),
    })
}

fn fact_visibility_json(visibility: &mfm_facts::FactVisibility) -> serde_json::Value {
    match visibility {
        mfm_facts::FactVisibility::RunPrivate => serde_json::json!({
            "kind": "run_private",
        }),
        mfm_facts::FactVisibility::Indexed { audience, scope } => serde_json::json!({
            "kind": "indexed",
            "audience": audience.as_str(),
            "scope": scope.as_str(),
        }),
    }
}

fn fact_subject_evidence_json(evidence: &mfm_facts::FactSubjectEvidence) -> serde_json::Value {
    serde_json::json!({
        "fact_subject_namespace_hash": evidence.fact_subject_namespace_hash().as_str(),
        "subject_material": evidence.subject_material().as_str(),
        "subject_material_hash": evidence.subject_material_hash().as_str(),
        "fact_key": evidence.fact_key().as_str(),
    })
}

fn fact_request_evidence_json(evidence: &mfm_facts::FactRequestEvidence) -> serde_json::Value {
    serde_json::json!({
        "request_schema_id": evidence.request_schema_id().as_str(),
        "request_hash": evidence.request_hash().as_str(),
    })
}

fn fact_response_evidence_json(evidence: &mfm_facts::FactResponseEvidence) -> serde_json::Value {
    serde_json::json!({
        "response_schema_id": evidence.response_schema_id().as_str(),
        "response_hash": evidence.response_hash().as_str(),
        "artifact_id": evidence.artifact_id().as_str(),
        "artifact_evidence_hash": evidence.artifact_evidence_hash().as_str(),
    })
}

fn fact_producer_provenance_json(
    provenance: &mfm_facts::FactProducerProvenance,
) -> serde_json::Value {
    serde_json::json!({
        "capability_kind": provenance.capability_kind().as_str(),
        "capability_version": provenance.capability_version().as_str(),
        "adapter_kind": provenance.adapter_kind().as_str(),
        "adapter_version": provenance.adapter_version().as_str(),
    })
}

fn run_artifact_json(evidence: &events::RunArtifactEvidenceRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "byte_len": evidence.byte_len,
        "content_digest": evidence.content_digest.as_str(),
        "evidence_hash": evidence.evidence_hash.as_str(),
        "media_type": evidence.media_type.as_str(),
        "role": evidence.role.as_str(),
        "schema_id": evidence.schema_id.as_ref().map(SchemaId::as_str),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

fn seed_cell_ref_json(seed: &events::SeedCellRef) -> serde_json::Value {
    serde_json::json!({
        "cell_id": seed.cell_id.as_str(),
        "digest": seed.digest.as_str(),
        "schema_id": seed.schema_id.as_str(),
        "scope_id": seed.scope_id.as_str(),
        "seed_artifact": event_artifact_json(&seed.seed_artifact),
        "seed_id": seed.seed_id.as_str(),
        "semantic_type_id": seed.semantic_type_id.as_str(),
    })
}

fn entry_point_launch_evidence_json(
    evidence: &events::EntryPointLaunchEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "entry_point_registry_digest": evidence.entry_point_registry_digest.as_str(),
        "resolved_op_id": evidence.resolved_op_id.as_str(),
    })
}

fn run_identity_material_json(material: &events::RunIdentityMaterialV1) -> serde_json::Value {
    serde_json::json!({
        "certified_spec_hash": material.certified_spec_hash.as_str(),
        "invocation_key_digest": material.invocation_key_digest.as_str(),
        "store_scope_id": material.store_scope_id.as_str(),
    })
}

fn executable_identity_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "cargo_package_digest": identity.cargo_package_digest.as_str(),
        "factory_id": identity.factory_id.as_str(),
        "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(|value| value.as_str()),
        "nix_output_hash": identity.nix_output_hash.as_ref().map(|value| value.as_str()),
    })
}

fn descriptor_identity_json(identity: &DescriptorIdentity) -> serde_json::Value {
    match identity {
        DescriptorIdentity::State(identity) => serde_json::json!({
            "capabilities": capability_set_json(&identity.capabilities),
            "config_schema_id": identity.config_schema_id.as_str(),
            "context": state_context_descriptor_json(&identity.context),
            "descriptor_family": "state",
            "descriptor_id": identity.descriptor_id.as_str(),
            "effect_class": identity.effect_class.as_str(),
            "effect_kind": identity.effect_kind.as_str(),
            "effect_name": identity.effect_name.as_str(),
            "effect_version": identity.effect_version.as_str(),
            "emitted_fact_descriptors": fact_descriptor_refs_json(&identity.emitted_fact_descriptors),
            "input_context": state_input_context_contract_json(&identity.input_context),
            "input_schema_id": identity.input_schema_id.as_str(),
            "name": identity.name.as_str(),
            "output_context": state_output_context_contract_json(&identity.output_context),
            "output_schema_id": identity.output_schema_id.as_str(),
            "output_semantic_type_id": identity.output_semantic_type_id.as_str(),
            "runner": identity.runner.as_str(),
            "side_effect_contract_digest": identity.side_effect_contract_digest.as_ref().map(ContentDigest::as_str),
            "state_kind": identity.state_kind.as_str(),
            "state_version": identity.state_version.as_str(),
        }),
        DescriptorIdentity::Operation(identity) => serde_json::json!({
            "config_schema_id": identity.config_schema_id.as_str(),
            "descriptor_family": "operation",
            "descriptor_id": identity.descriptor_id.as_str(),
            "expansion_abi": identity.expansion_abi.as_str(),
            "input_schema_id": identity.input_schema_id.as_str(),
            "name": identity.name.as_str(),
            "operation_kind": identity.operation_kind.as_str(),
            "operation_version": identity.operation_version.as_str(),
            "output_schema_id": identity.output_schema_id.as_str(),
        }),
        DescriptorIdentity::Renderer(identity) => renderer_descriptor_json(identity),
    }
}

fn state_context_descriptor_json(context: &spec::StateContextDescriptorSpec) -> serde_json::Value {
    match context {
        spec::StateContextDescriptorSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateContextDescriptorSpec::Required(requirement) => {
            let spec::StateContextDescriptorRequirementSpec {
                context_descriptor_id,
                schema_id,
                semantic_type_id,
                canonicalizer_identity,
            } = requirement.as_ref();
            serde_json::json!({
                "canonicalizer_identity": canonicalizer_identity.as_str(),
                "context_descriptor_id": context_descriptor_id.as_str(),
                "kind": "required",
                "schema_id": schema_id.as_str(),
                "semantic_type_id": semantic_type_id.as_str(),
            })
        }
    }
}

fn context_producer_json(producer: &spec::ContextProducerSpec) -> serde_json::Value {
    serde_json::json!({
        "producer_descriptor_ids": producer
            .producer_descriptor_ids
            .iter()
            .map(DescriptorId::as_str)
            .collect::<Vec<_>>(),
        "seed_producers_allowed": producer.seed_producers_allowed,
    })
}

fn state_input_context_contract_json(
    contract: &spec::StateInputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateInputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateInputContextContractSpec::Required {
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "kind": "required",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn state_output_context_contract_json(
    contract: &spec::StateOutputContextContractSpec,
) -> serde_json::Value {
    match contract {
        spec::StateOutputContextContractSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::StateOutputContextContractSpec::Produces {
            resource_kind,
            stage,
        } => serde_json::json!({
            "kind": "produces",
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn cell_context_json(context: &spec::CellContextSpec) -> serde_json::Value {
    match context {
        spec::CellContextSpec::NoContext => serde_json::json!({
            "kind": "no_context",
        }),
        spec::CellContextSpec::Bound {
            context_ref,
            resource_kind,
            stage,
            producer,
        } => serde_json::json!({
            "context_ref": context_ref.as_str(),
            "kind": "bound",
            "producer": context_producer_json(producer),
            "resource_kind": resource_kind.as_str(),
            "stage": stage.as_str(),
        }),
    }
}

fn fact_descriptor_refs_json(refs: &[spec::FactDescriptorRef]) -> Vec<serde_json::Value> {
    refs.iter()
        .map(|reference| {
            serde_json::json!({
                "descriptor_hash": reference.descriptor_hash.as_str(),
            })
        })
        .collect()
}

fn renderer_descriptor_json(
    identity: &mfm_spec::v1::RendererDescriptorIdentity,
) -> serde_json::Value {
    serde_json::json!({
        "canonicalizer_identity": identity.canonicalizer_identity.as_str(),
        "descriptor_family": "renderer",
        "descriptor_id": identity.descriptor_id.as_str(),
        "public_schema_id": identity.public_schema_id.as_str(),
        "renderer_kind": identity.renderer_kind.as_str(),
        "renderer_version": identity.renderer_version.as_str(),
    })
}

fn capability_set_json(
    descriptor: &mfm_capabilities::CapabilitySetDescriptor,
) -> serde_json::Value {
    serde_json::json!({
        "capabilities": descriptor.capabilities.iter().map(|capability| {
            serde_json::json!({
                "kind": capability.kind.as_str(),
                "name": capability.name.as_str(),
                "role": capability.role.as_str(),
                "version": capability.version.as_str(),
            })
        }).collect::<Vec<_>>(),
    })
}

fn named_cell_ref_json(cell: &events::NamedTypedCellRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": cell.artifact_id.as_str(),
        "cell_id": cell.cell_id.as_str(),
        "content_digest": cell.content_digest.as_str(),
        "evidence_hash": cell.evidence_hash.as_str(),
        "producer": cell_producer_json(&cell.producer),
        "public_field_path": cell.public_field_path.as_str(),
        "schema_id": cell.schema_id.as_str(),
        "scope_id": cell.scope_id.as_str(),
        "semantic_type_id": cell.semantic_type_id.as_str(),
        "value_lineage": value_lineage_json(&cell.value_lineage),
    })
}

fn cell_producer_json(producer: &CellProducer) -> serde_json::Value {
    match producer {
        CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
        CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
    }
}

fn value_lineage_json(lineage: &ValueLineageRef) -> serde_json::Value {
    serde_json::json!({
        "lineage_digest": lineage.lineage_digest.as_str(),
    })
}

/// Encodes a cell skip reason as canonical JSON.
pub fn skip_reason_json(reason: &events::SkipReason) -> serde_json::Value {
    serde_json::json!({
        "code": reason.code.as_str(),
        "safe_message": reason.safe_message.as_str(),
    })
}

/// Encodes a side-effect ledger purpose as canonical JSON.
pub fn side_effect_ledger_purpose_json(
    purpose: &events::SideEffectLedgerPurpose,
) -> serde_json::Value {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => serde_json::json!({
            "kind": "forward",
        }),
        events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
            serde_json::json!({
                "forward_pair_id": forward_pair_id.as_str(),
                "kind": "remediation",
            })
        }
    }
}

/// Encodes exclusive resource-key evidence as canonical JSON.
pub fn resource_key_evidence_json(evidence: &events::ResourceKeyEvidence) -> serde_json::Value {
    serde_json::json!({
        "key": evidence.key.as_str(),
        "key_schema_id": evidence.key_schema_id.as_str(),
        "namespace": evidence.namespace.as_str(),
    })
}

/// Encodes touched-set resource evidence as canonical JSON.
pub fn resource_touched_set_evidence_json(
    evidence: &events::ResourceTouchedSetEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "evidence_artifact_evidence_hash": evidence.evidence_artifact_evidence_hash.as_str(),
        "evidence_artifact_id": evidence.evidence_artifact_id.as_str(),
        "evidence_hash": evidence.evidence_hash.as_str(),
        "evidence_schema_id": evidence.evidence_schema_id.as_str(),
        "namespace": evidence.namespace.as_str(),
    })
}

/// Returns the canonical tag for a manual resolution outcome.
pub fn manual_resolution_outcome_str(outcome: events::ManualResolutionOutcome) -> &'static str {
    outcome.as_str()
}

/// Returns the canonical tag for a run-completion outcome.
pub fn run_completion_outcome_str(outcome: &events::RunCompletionOutcome) -> &'static str {
    outcome.kind()
}

/// Returns the public terminal-claim tag represented by a run-completion outcome.
pub fn run_completion_claim_str(outcome: &events::RunCompletionOutcome) -> &'static str {
    match outcome {
        events::RunCompletionOutcome::Completed(_) => "public_output",
        events::RunCompletionOutcome::Compensated => "compensation",
        events::RunCompletionOutcome::ManuallyResolved => "manual_resolution",
        events::RunCompletionOutcome::FailedWithoutAcdcClaim => "no_acdc_claim",
    }
}

/// Encodes a manual resolution note as canonical JSON.
pub fn manual_resolution_note_json(note: &events::ManualResolutionNote) -> serde_json::Value {
    serde_json::json!({
        "text": note.as_str(),
    })
}

/// Encodes structured error info as canonical JSON.
pub fn error_info_json(error: &events::MfmErrorInfo) -> serde_json::Value {
    serde_json::json!({
        "category": error_category_str(error.category),
        "code": error.code.as_str(),
        "diagnostic_ref": error.diagnostic_ref.as_ref().map(event_artifact_json),
        "public_details": error.public_details.as_ref().map(|details| {
            serde_json::json!({
                "content_digest": details.content_digest.as_str(),
            })
        }),
        "retryable": error.retryable,
        "safe_message": error.safe_message.as_str(),
    })
}

/// Encodes a run completion outcome as canonical JSON.
pub fn run_completion_outcome_json(outcome: &events::RunCompletionOutcome) -> serde_json::Value {
    match outcome {
        events::RunCompletionOutcome::Completed(evidence) => serde_json::json!({
            "kind": outcome.kind(),
            "public_output": {
                "public_output_event_id": evidence.public_output_event_id.as_str(),
                "public_output_schema_id": evidence.public_output_schema_id.as_str(),
            },
        }),
        events::RunCompletionOutcome::Compensated => serde_json::json!({
            "kind": outcome.kind(),
        }),
        events::RunCompletionOutcome::ManuallyResolved => serde_json::json!({
            "kind": outcome.kind(),
        }),
        events::RunCompletionOutcome::FailedWithoutAcdcClaim => serde_json::json!({
            "kind": outcome.kind(),
        }),
    }
}
