use super::*;

/// Domain parse/encode functions for kernel events, projections, and saga types.
///
/// Defined at the `v1` root (where the surrounding store internals live) and re-exported
/// here so the Postgres adapter consumes one implementation through this module.
pub use super::{
    error_info_json, event_artifact_json, manual_resolution_note_json,
    manual_resolution_outcome_str, parse_error_category, parse_error_info, parse_event_artifact,
    parse_failure_phase, parse_manual_resolution_note, parse_manual_resolution_outcome,
    parse_resource_key_evidence, parse_resource_touched_set_evidence, parse_run_completion_outcome,
    parse_side_effect_ledger_purpose, parse_skip_reason, resource_key_evidence_json,
    resource_touched_set_evidence_json, run_completion_claim_str, run_completion_outcome_json,
    run_completion_outcome_str, side_effect_ledger_purpose_json, skip_reason_json,
};

/// Codec result over the backend-neutral [`CodecError`].
pub type CodecResult<T> = std::result::Result<T, CodecError>;

/// Parses a typed identity from its canonical string form.
pub fn parse_identity<T>(value: &str) -> CodecResult<T>
where
    T: std::str::FromStr<Err = IdentityError>,
{
    Ok(value.parse()?)
}

/// Returns a required string field.
pub fn required_str<'a>(json: &'a serde_json::Value, field: &'static str) -> CodecResult<&'a str> {
    json.get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CodecError::Field(format!("missing string field {field}")))
}

/// Returns an optional string field, treating null or absent as `None`.
pub fn optional_str<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> CodecResult<Option<&'a str>> {
    match json.get(field) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| CodecError::Field(format!("field {field} was not a string"))),
    }
}

/// Returns a required object field.
pub fn required_obj<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> CodecResult<&'a serde_json::Value> {
    let value = json
        .get(field)
        .ok_or_else(|| CodecError::Field(format!("missing object field {field}")))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(CodecError::Field(format!(
            "field {field} was not an object"
        )))
    }
}

/// Returns an optional object field, treating null or absent as `None`.
pub fn optional_obj<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> CodecResult<Option<&'a serde_json::Value>> {
    match json.get(field) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value)),
        Some(_) => Err(CodecError::Field(format!(
            "field {field} was not an object"
        ))),
    }
}

/// Returns a required unsigned 64-bit field.
pub fn required_u64(json: &serde_json::Value, field: &'static str) -> CodecResult<u64> {
    json.get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CodecError::Field(format!("missing u64 field {field}")))
}

/// Returns a required unsigned 32-bit field.
pub fn required_u32(json: &serde_json::Value, field: &'static str) -> CodecResult<u32> {
    required_u64(json, field)?
        .try_into()
        .map_err(|_| CodecError::Field(format!("{field} overflowed u32")))
}

/// Returns a required boolean field.
pub fn required_bool(json: &serde_json::Value, field: &'static str) -> CodecResult<bool> {
    json.get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CodecError::Field(format!("missing bool field {field}")))
}
/// Encodes a run-completion projection as JSON.
pub fn run_completion_projection_json(
    run_id: &RunId,
    projection: &RunCompletionProjection,
) -> serde_json::Value {
    serde_json::json!({
        "event_id": projection.event_id.as_str(),
        "outcome": run_completion_outcome_json(&projection.outcome),
        "run_id": run_id.as_str(),
    })
}

/// Parses a run-completion projection from JSON.
pub fn parse_run_completion_projection(
    json: &serde_json::Value,
) -> CodecResult<(RunId, RunCompletionProjection)> {
    Ok((
        parse_identity(required_str(json, "run_id")?)?,
        RunCompletionProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            outcome: parse_run_completion_outcome(required_obj(json, "outcome")?)?,
        },
    ))
}

/// Encodes a saga-engagement projection as JSON.
pub fn saga_engagement_projection_json(
    run_id: &RunId,
    projection: &SagaEngagementProjection,
) -> serde_json::Value {
    serde_json::json!({
        "event_id": projection.event_id.as_str(),
        "reason": saga_engagement_reason_json(&projection.reason),
        "run_id": run_id.as_str(),
    })
}

/// Parses a saga-engagement projection from JSON.
pub fn parse_saga_engagement_projection(
    json: &serde_json::Value,
) -> CodecResult<(RunId, SagaEngagementProjection)> {
    Ok((
        parse_identity(required_str(json, "run_id")?)?,
        SagaEngagementProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            reason: parse_saga_engagement_reason(required_obj(json, "reason")?)?,
        },
    ))
}

fn saga_engagement_reason_json(reason: &SagaEngagementReason) -> serde_json::Value {
    match reason {
        SagaEngagementReason::NonRetryableFailure {
            node_id,
            attempt_id,
        } => serde_json::json!({
            "attempt_id": attempt_id.as_str(),
            "kind": "non_retryable_failure",
            "node_id": node_id.as_str(),
        }),
        SagaEngagementReason::ForwardAmbiguous { pair_id } => serde_json::json!({
            "kind": "forward_ambiguous",
            "pair_id": pair_id.as_str(),
        }),
    }
}

fn parse_saga_engagement_reason(json: &serde_json::Value) -> CodecResult<SagaEngagementReason> {
    match required_str(json, "kind")? {
        "non_retryable_failure" => Ok(SagaEngagementReason::NonRetryableFailure {
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        }),
        "forward_ambiguous" => Ok(SagaEngagementReason::ForwardAmbiguous {
            pair_id: parse_identity(required_str(json, "pair_id")?)?,
        }),
        other => Err(CodecError::Identity(format!(
            "unknown saga engagement reason {other}"
        ))),
    }
}

/// Encodes a manual-resolution projection as JSON.
pub fn manual_resolution_projection_json(
    run_id: &RunId,
    projection: &ManualResolutionProjection,
) -> serde_json::Value {
    serde_json::json!({
        "authorization_artifact_id": projection.authorization_artifact_id.as_str(),
        "authorization_hash": projection.authorization_hash.as_str(),
        "authorization_schema_id": projection.authorization_schema_id.as_str(),
        "event_id": projection.event_id.as_str(),
        "evidence_artifact_id": projection.evidence_artifact_id.as_str(),
        "evidence_hash": projection.evidence_hash.as_str(),
        "evidence_schema_id": projection.evidence_schema_id.as_str(),
        "note": projection.note.as_ref().map(manual_resolution_note_json),
        "outcome": manual_resolution_outcome_str(projection.outcome),
        "run_id": run_id.as_str(),
    })
}

/// Parses a manual-resolution projection from JSON.
pub fn parse_manual_resolution_projection(
    json: &serde_json::Value,
) -> CodecResult<(RunId, ManualResolutionProjection)> {
    Ok((
        parse_identity(required_str(json, "run_id")?)?,
        ManualResolutionProjection {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            outcome: parse_manual_resolution_outcome(required_str(json, "outcome")?)?,
            evidence_schema_id: parse_identity(required_str(json, "evidence_schema_id")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
            evidence_artifact_id: parse_identity(required_str(json, "evidence_artifact_id")?)?,
            authorization_schema_id: parse_identity(required_str(
                json,
                "authorization_schema_id",
            )?)?,
            authorization_hash: parse_identity(required_str(json, "authorization_hash")?)?,
            authorization_artifact_id: parse_identity(required_str(
                json,
                "authorization_artifact_id",
            )?)?,
            note: optional_obj(json, "note")?
                .map(parse_manual_resolution_note)
                .transpose()?,
        },
    ))
}

/// Encodes an attempt projection as JSON.
pub fn attempt_projection_json(projection: &AttemptProjection) -> serde_json::Value {
    let status = match &projection.status {
        AttemptStatus::Started {
            attempt_no,
            state_kind,
            state_version,
        } => serde_json::json!({
            "variant": "started",
            "attempt_no": attempt_no,
            "state_kind": state_kind.as_str(),
            "state_version": state_version.as_str(),
        }),
        AttemptStatus::Completed { output_cell_id } => serde_json::json!({
            "variant": "completed",
            "output_cell_id": output_cell_id.as_str(),
        }),
        AttemptStatus::Failed { retryable, error } => serde_json::json!({
            "variant": "failed",
            "retryable": retryable,
            "error": error_info_json(error),
        }),
        AttemptStatus::Interrupted => serde_json::json!({
            "variant": "interrupted",
        }),
    };
    serde_json::json!({
        "attempt_id": projection.attempt_id.as_str(),
        "event_id": projection.event_id.as_str(),
        "node_id": projection.node_id.as_str(),
        "run_id": projection.run_id.as_str(),
        "status": status,
    })
}

/// Parses an attempt projection from JSON.
pub fn parse_attempt_projection(json: &serde_json::Value) -> CodecResult<AttemptProjection> {
    let status_json = required_obj(json, "status")?;
    let status = match required_str(status_json, "variant")? {
        "started" => AttemptStatus::Started {
            attempt_no: required_u64(status_json, "attempt_no")?
                .try_into()
                .map_err(|_| CodecError::Field("attempt_no overflow".into()))?,
            state_kind: parse_identity(required_str(status_json, "state_kind")?)?,
            state_version: required_str(status_json, "state_version")?.parse()?,
        },
        "completed" => AttemptStatus::Completed {
            output_cell_id: parse_identity(required_str(status_json, "output_cell_id")?)?,
        },
        "failed" => AttemptStatus::Failed {
            retryable: required_bool(status_json, "retryable")?,
            error: Box::new(parse_error_info(required_obj(status_json, "error")?)?),
        },
        "interrupted" => AttemptStatus::Interrupted,
        other => {
            return Err(CodecError::Identity(format!(
                "unknown attempt status {other}"
            )));
        }
    };
    Ok(AttemptProjection {
        run_id: parse_identity(required_str(json, "run_id")?)?,
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        event_id: parse_identity(required_str(json, "event_id")?)?,
        status,
    })
}

/// Encodes a cell terminal projection as JSON.
pub fn cell_projection_json(
    run_id: &RunId,
    cell_id: &CellId,
    projection: &CellTerminalProjection,
) -> serde_json::Value {
    match projection {
        CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            evidence_hash,
        } => serde_json::json!({
            "variant": "produced",
            "run_id": run_id.as_str(),
            "cell_id": cell_id.as_str(),
            "event_id": event_id.as_str(),
            "node_id": node_id.as_str(),
            "attempt_id": attempt_id.as_str(),
            "schema_id": schema_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
            "artifact_id": artifact_id.as_str(),
            "content_digest": content_digest.as_str(),
            "evidence_hash": evidence_hash.as_str(),
        }),
        CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        } => serde_json::json!({
            "variant": "skipped",
            "run_id": run_id.as_str(),
            "cell_id": cell_id.as_str(),
            "event_id": event_id.as_str(),
            "node_id": node_id.as_str(),
            "attempt_id": attempt_id.as_str(),
            "schema_id": schema_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
            "skip_reason": skip_reason_json(skip_reason),
        }),
    }
}

/// Parses a cell terminal projection from JSON.
pub fn parse_cell_projection(
    json: &serde_json::Value,
) -> CodecResult<((RunId, CellId), CellTerminalProjection)> {
    let run_id = parse_identity(required_str(json, "run_id")?)?;
    let cell_id = parse_identity(required_str(json, "cell_id")?)?;
    let projection = match required_str(json, "variant")? {
        "produced" => CellTerminalProjection::Produced {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
            evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        },
        "skipped" => CellTerminalProjection::Skipped {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            skip_reason: parse_skip_reason(required_obj(json, "skip_reason")?)?,
        },
        other => {
            return Err(CodecError::Identity(format!(
                "unknown cell projection {other}"
            )));
        }
    };
    Ok(((run_id, cell_id), projection))
}

/// Encodes a side-effect projection as JSON.
pub fn side_effect_projection_json(projection: &SideEffectProjection) -> serde_json::Value {
    serde_json::json!({
        "claim": projection.claim.as_ref().map(side_effect_claim_json),
        "confirmation": projection.confirmation.as_ref().map(side_effect_artifact_json),
        "event_id": projection.event_id.as_str(),
        "intent": side_effect_intent_json(&projection.intent),
        "ledger_key": projection.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&projection.ledger_purpose),
        "pair_id": projection.pair_id.as_str(),
        "phase": side_effect_phase_json(&projection.phase),
        "prepared_invocation": projection.prepared_invocation.as_ref().map(side_effect_artifact_json),
        "receipt": projection.receipt.as_ref().map(side_effect_artifact_json),
        "resource_key": projection.resource_key.as_ref().map(resource_key_evidence_json),
        "resource_touched_set": projection.resource_touched_set.as_ref().map(resource_touched_set_evidence_json),
        "run_id": projection.run_id.as_str(),
        "submission": projection.submission.as_ref().map(side_effect_artifact_json),
    })
}

/// Parses a side-effect projection from JSON.
pub fn parse_side_effect_projection(json: &serde_json::Value) -> CodecResult<SideEffectProjection> {
    Ok(SideEffectProjection {
        run_id: parse_identity(required_str(json, "run_id")?)?,
        ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
        ledger_purpose: parse_side_effect_ledger_purpose(required_obj(json, "ledger_purpose")?)?,
        pair_id: parse_identity(required_str(json, "pair_id")?)?,
        event_id: parse_identity(required_str(json, "event_id")?)?,
        intent: parse_side_effect_intent(required_obj(json, "intent")?)?,
        prepared_invocation: optional_obj(json, "prepared_invocation")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        resource_key: optional_obj(json, "resource_key")?
            .map(parse_resource_key_evidence)
            .transpose()?,
        submission: optional_obj(json, "submission")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        receipt: optional_obj(json, "receipt")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        confirmation: optional_obj(json, "confirmation")?
            .map(parse_side_effect_artifact)
            .transpose()?,
        resource_touched_set: optional_obj(json, "resource_touched_set")?
            .map(parse_resource_touched_set_evidence)
            .transpose()?,
        claim: optional_obj(json, "claim")?
            .map(parse_side_effect_claim)
            .transpose()?,
        phase: parse_side_effect_phase(required_obj(json, "phase")?)?,
    })
}

fn side_effect_artifact_json(artifact: &SideEffectArtifactProjection) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": artifact.artifact_id.as_str(),
        "content_digest": artifact.content_digest.as_str(),
        "evidence_hash": artifact.evidence_hash.as_str(),
        "schema_id": artifact.schema_id.as_ref().map(SchemaId::as_str),
    })
}

fn parse_side_effect_artifact(
    json: &serde_json::Value,
) -> CodecResult<SideEffectArtifactProjection> {
    Ok(SideEffectArtifactProjection {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        evidence_hash: parse_identity(required_str(json, "evidence_hash")?)?,
        schema_id: optional_str(json, "schema_id")?
            .map(parse_identity)
            .transpose()?,
    })
}

/// Encodes a resource-lane projection as JSON.
pub fn resource_lane_projection_json(
    lane_key: &ResourceLaneKey,
    projection: &ResourceLaneProjection,
) -> serde_json::Value {
    serde_json::json!({
        "attempt_id": projection.attempt_id.as_str(),
        "claim_fencing_token": projection.claim_fencing_token,
        "claim_id": projection.claim_id.as_str(),
        "event_id": projection.event_id.as_str(),
        "invocation_epoch": projection.invocation_epoch,
        "key": lane_key.key.as_str(),
        "key_schema_id": lane_key.key_schema_id.as_str(),
        "lane_transition_seq": projection.lane_transition_seq,
        "ledger_key": projection.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&projection.ledger_purpose),
        "namespace": lane_key.namespace.as_str(),
        "node_id": projection.node_id.as_str(),
        "pair_id": projection.holder.pair_id.as_str(),
        "run_id": projection.holder.run_id.as_str(),
    })
}

/// Parses a resource-lane projection from JSON.
pub fn parse_resource_lane_projection(
    json: &serde_json::Value,
) -> CodecResult<(ResourceLaneKey, ResourceLaneProjection)> {
    let lane_key = ResourceLaneKey {
        namespace: ResourceNamespace::new(required_str(json, "namespace")?)
            .map_err(|error| CodecError::Identity(error.to_string()))?,
        key_schema_id: parse_identity(required_str(json, "key_schema_id")?)?,
        key: events::ResourceKey::new(required_str(json, "key")?)?,
    };
    let projection = ResourceLaneProjection {
        event_id: parse_identity(required_str(json, "event_id")?)?,
        holder: SideEffectPairLedgerRef::new(
            parse_identity(required_str(json, "run_id")?)?,
            parse_identity(required_str(json, "pair_id")?)?,
        ),
        ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
        ledger_purpose: parse_side_effect_ledger_purpose(required_obj(json, "ledger_purpose")?)?,
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        claim_id: events::ResourceLaneClaimId::new(required_str(json, "claim_id")?)?,
        claim_fencing_token: required_u64(json, "claim_fencing_token")?,
        lane_transition_seq: required_u64(json, "lane_transition_seq")?,
    };
    Ok((lane_key, projection))
}

fn side_effect_intent_json(intent: &SideEffectIntentProjection) -> serde_json::Value {
    serde_json::json!({
        "adapter_kind": intent.adapter_kind.as_str(),
        "adapter_version": intent.adapter_version.as_str(),
        "attempt_id": intent.attempt_id.as_str(),
        "capability_kind": intent.capability_kind.as_str(),
        "capability_version": intent.capability_version.as_str(),
        "idempotency_input_hash": intent.idempotency_input_hash.as_str(),
        "idempotency_input_schema_id": intent.idempotency_input_schema_id.as_str(),
        "idempotency_key": intent.idempotency_key.as_str(),
        "intent_artifact_id": intent.intent_artifact_id.as_str(),
        "intent_hash": intent.intent_hash.as_str(),
        "intent_schema_id": intent.intent_schema_id.as_str(),
        "invocation_epoch": intent.invocation_epoch,
        "node_id": intent.node_id.as_str(),
        "scope_id": intent.scope_id.as_str(),
    })
}

fn parse_side_effect_intent(json: &serde_json::Value) -> CodecResult<SideEffectIntentProjection> {
    Ok(SideEffectIntentProjection {
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        intent_schema_id: parse_identity(required_str(json, "intent_schema_id")?)?,
        intent_hash: parse_identity(required_str(json, "intent_hash")?)?,
        intent_artifact_id: parse_identity(required_str(json, "intent_artifact_id")?)?,
        idempotency_input_schema_id: parse_identity(required_str(
            json,
            "idempotency_input_schema_id",
        )?)?,
        idempotency_input_hash: parse_identity(required_str(json, "idempotency_input_hash")?)?,
        idempotency_key: events::IdempotencyKeyRef::new(required_str(json, "idempotency_key")?)?,
        capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
        capability_version: required_str(json, "capability_version")?.parse()?,
        adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
        adapter_version: required_str(json, "adapter_version")?.parse()?,
    })
}

fn side_effect_claim_json(claim: &SideEffectClaimProjection) -> serde_json::Value {
    serde_json::json!({
        "attempt_id": claim.attempt_id.as_str(),
        "claim_fencing_token": claim.claim_fencing_token.as_str(),
        "claim_generation": claim.claim_generation,
        "claim_owner": claim.claim_owner.as_str(),
        "invocation_epoch": claim.invocation_epoch,
        "node_id": claim.node_id.as_str(),
    })
}

fn parse_side_effect_claim(json: &serde_json::Value) -> CodecResult<SideEffectClaimProjection> {
    Ok(SideEffectClaimProjection {
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        claim_generation: required_u32(json, "claim_generation")?,
        claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
            json,
            "claim_fencing_token",
        )?)?,
    })
}

fn side_effect_phase_json(phase: &SideEffectPhase) -> serde_json::Value {
    match phase {
        SideEffectPhase::IntentPersisted { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Claimed {
            claim_owner,
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "claim_owner": claim_owner.as_str(),
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::InvocationPrepared {
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::InvocationStarted {
            claim_owner,
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "claim_owner": claim_owner.as_str(),
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::SubmissionObserved { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::NotSubmittedProven { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::SubmissionUnknown { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::ReceiptObserved { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::ConfirmationObserved { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Ambiguous { invocation_epoch } => {
            phase_json(phase.as_str(), *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Failed {
            invocation_epoch,
            failure_phase,
        } => phase_json(
            phase.as_str(),
            *invocation_epoch,
            serde_json::json!({
                "failure_phase": failure_phase_str(*failure_phase),
            }),
        ),
    }
}

fn phase_json(
    variant: &'static str,
    invocation_epoch: u32,
    mut extra: serde_json::Value,
) -> serde_json::Value {
    extra["variant"] = serde_json::json!(variant);
    extra["invocation_epoch"] = serde_json::json!(invocation_epoch);
    extra
}

fn parse_side_effect_phase(json: &serde_json::Value) -> CodecResult<SideEffectPhase> {
    let invocation_epoch = required_u32(json, "invocation_epoch")?;
    match required_str(json, "variant")? {
        "intent_persisted" => Ok(SideEffectPhase::IntentPersisted { invocation_epoch }),
        "claimed" => Ok(SideEffectPhase::Claimed {
            claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "invocation_prepared" => Ok(SideEffectPhase::InvocationPrepared {
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "invocation_started" => Ok(SideEffectPhase::InvocationStarted {
            claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "submission_observed" => Ok(SideEffectPhase::SubmissionObserved { invocation_epoch }),
        "not_submitted_proven" => Ok(SideEffectPhase::NotSubmittedProven { invocation_epoch }),
        "submission_unknown" => Ok(SideEffectPhase::SubmissionUnknown { invocation_epoch }),
        "receipt_observed" => Ok(SideEffectPhase::ReceiptObserved { invocation_epoch }),
        "confirmation_observed" => Ok(SideEffectPhase::ConfirmationObserved { invocation_epoch }),
        "ambiguous" => Ok(SideEffectPhase::Ambiguous { invocation_epoch }),
        "failed" => Ok(SideEffectPhase::Failed {
            invocation_epoch,
            failure_phase: parse_failure_phase(required_str(json, "failure_phase")?)?,
        }),
        other => Err(CodecError::Identity(format!(
            "unknown side-effect phase {other}"
        ))),
    }
}

/// Encodes a public-output projection as JSON.
pub fn public_output_projection_json(
    run_id: &RunId,
    schema_id: &SchemaId,
    projection: &PublicOutputProjection,
) -> serde_json::Value {
    match projection {
        PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        } => serde_json::json!({
            "variant": "produced",
            "run_id": run_id.as_str(),
            "public_schema_id": schema_id.as_str(),
            "event_id": event_id.as_str(),
            "rendered_digest": rendered_digest.as_str(),
            "rendered_artifact_id": rendered_artifact_id.as_ref().map(ArtifactId::as_str),
        }),
        PublicOutputProjection::RenderFailed { event_id, error } => serde_json::json!({
            "variant": "render_failed",
            "run_id": run_id.as_str(),
            "public_schema_id": schema_id.as_str(),
            "event_id": event_id.as_str(),
            "error": error_info_json(error),
        }),
    }
}

/// Parses a public-output projection from JSON.
pub fn parse_public_output_projection(
    json: &serde_json::Value,
) -> CodecResult<((RunId, SchemaId), PublicOutputProjection)> {
    let run_id = parse_identity(required_str(json, "run_id")?)?;
    let schema_id = parse_identity(required_str(json, "public_schema_id")?)?;
    let projection = match required_str(json, "variant")? {
        "produced" => PublicOutputProjection::Produced {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            rendered_digest: parse_identity(required_str(json, "rendered_digest")?)?,
            rendered_artifact_id: optional_str(json, "rendered_artifact_id")?
                .map(parse_identity)
                .transpose()?,
        },
        "render_failed" => PublicOutputProjection::RenderFailed {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            error: Box::new(parse_error_info(required_obj(json, "error")?)?),
        },
        other => {
            return Err(CodecError::Identity(format!(
                "unknown public output projection {other}"
            )));
        }
    };
    Ok(((run_id, schema_id), projection))
}

/// Encodes a retention manifest projection as JSON.
pub fn retention_manifest_projection_json(
    manifest: &RetentionManifestProjection,
) -> serde_json::Value {
    serde_json::json!({
        "manifest_artifact_id": manifest.manifest_artifact_id.as_str(),
        "manifest_digest": manifest.manifest_digest.as_str(),
        "previous_manifest_digest": manifest.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
        "manifest_seq": manifest.manifest_seq,
    })
}

/// Parses a retention manifest projection from JSON.
pub fn parse_retention_manifest_projection(
    json: &serde_json::Value,
) -> CodecResult<RetentionManifestProjection> {
    Ok(RetentionManifestProjection {
        manifest_seq: required_u64(json, "manifest_seq")?,
        manifest_digest: parse_identity(required_str(json, "manifest_digest")?)?,
        previous_manifest_digest: optional_str(json, "previous_manifest_digest")?
            .map(parse_identity)
            .transpose()?,
        manifest_artifact_id: parse_identity(required_str(json, "manifest_artifact_id")?)?,
    })
}

/// Returns the canonical run-state projection tag.
pub fn run_state_str(state: RunState) -> &'static str {
    match state {
        RunState::Absent => "absent",
        RunState::Started => "started",
        RunState::Completed => "completed",
    }
}

/// Parses a run-state projection tag.
pub fn parse_run_state(value: &str) -> CodecResult<RunState> {
    match value {
        "absent" => Ok(RunState::Absent),
        "started" => Ok(RunState::Started),
        "completed" => Ok(RunState::Completed),
        other => Err(CodecError::Identity(format!("unknown run state {other}"))),
    }
}

/// Encodes a retention reference as canonical JSON.
pub fn retention_ref_json(retention_ref: &events::RetentionRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": retention_ref.artifact_id.as_str(),
        "content_digest": retention_ref.content_digest.as_str(),
        "evidence_hash": retention_ref.evidence_hash.as_str(),
        "role": retention_ref.role.as_str(),
    })
}

pub(super) fn required_run_state_str(state: RequiredRunState) -> &'static str {
    match state {
        RequiredRunState::Any => "any",
        RequiredRunState::Absent => "absent",
        RequiredRunState::Started => "started",
        RequiredRunState::NotCompleted => "not_completed",
        RequiredRunState::Completed => "completed",
    }
}

pub(super) fn required_cell_state_str(state: RequiredCellState) -> &'static str {
    match state {
        RequiredCellState::Absent => "absent",
        RequiredCellState::Produced => "produced",
        RequiredCellState::Skipped => "skipped",
        RequiredCellState::Terminal => "terminal",
    }
}

pub(super) fn required_side_effect_state_str(state: RequiredSideEffectState) -> &'static str {
    match state {
        RequiredSideEffectState::Absent => "absent",
        RequiredSideEffectState::IntentPersisted => "intent_persisted",
        RequiredSideEffectState::Claimed => "claimed",
        RequiredSideEffectState::InvocationPrepared => "invocation_prepared",
        RequiredSideEffectState::InvocationStarted => "invocation_started",
        RequiredSideEffectState::SubmissionResult => "submission_result",
        RequiredSideEffectState::ReceiptObserved => "receipt_observed",
        RequiredSideEffectState::ConfirmationObserved => "confirmation_observed",
        RequiredSideEffectState::Ambiguous => "ambiguous",
        RequiredSideEffectState::Failed => "failed",
    }
}

/// Returns the canonical tag for an error category.
pub fn error_category_str(category: events::ErrorCategory) -> &'static str {
    match category {
        events::ErrorCategory::Planning => "planning",
        events::ErrorCategory::Validation => "validation",
        events::ErrorCategory::Capability => "capability",
        events::ErrorCategory::SideEffect => "side_effect",
        events::ErrorCategory::Runtime => "runtime",
        events::ErrorCategory::Storage => "storage",
        events::ErrorCategory::Cancelled => "cancelled",
    }
}

/// Returns the canonical tag for a side-effect failure phase.
pub fn failure_phase_str(phase: side_effect::FailurePhase) -> &'static str {
    match phase {
        side_effect::FailurePhase::BeforeInvocationStarted => "before_invocation_started",
        side_effect::FailurePhase::AfterNotSubmittedProven => "after_not_submitted_proven",
    }
}

pub(super) fn retention_reason_str(reason: events::RetentionReason) -> &'static str {
    match reason {
        events::RetentionReason::RunAdmitted => "run_admitted",
        events::RetentionReason::RuntimeEvidence => "runtime_evidence",
        events::RetentionReason::PublicOutput => "public_output",
        events::RetentionReason::ManifestProjection => "manifest_projection",
    }
}
