use super::event_codec_decode::payload_from_json_value;
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
    Ok(CommitFingerprint::from_digest(canonical.content_digest()))
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
            "prepared_schema_id": payload.prepared_schema_id.as_str(),
            "prepared_artifact_evidence_hash": payload.prepared_artifact_evidence_hash.as_str(),
            "prepared_artifact_id": payload.prepared_artifact_id.as_str(),
            "prepared_hash": payload.prepared_hash.as_str(),
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
        "configured_targets": evidence
            .configured_targets
            .iter()
            .map(|source| {
                serde_json::json!({
                    "digest": source.digest.as_str(),
                    "target": source.target.as_str(),
                    "schema_id": source.schema_id.as_str(),
                })
            })
            .collect::<Vec<_>>(),
        "entry_point_id": evidence.entry_point_id.as_str(),
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
            "effect_contract_digest": identity.effect_contract_digest.as_ref().map(ContentDigest::as_str),
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
