use super::*;

pub(super) struct PreparedCommitAuthority {
    pub(super) commit_idempotency_hash: String,
    pub(super) idempotency_canonical_json: Vec<u8>,
    pub(super) prepared_authority_hash: String,
    pub(super) prepared_authority_canonical_json: Vec<u8>,
}

pub(super) struct FinalCommitAuthority {
    pub(super) commit_batch_hash: String,
    pub(super) commit_batch_canonical_json: Vec<u8>,
}

pub(super) fn prepared_commit_authority(
    plan: &mfm_store::v1::PreparedCommitPlan,
    fingerprint: &CommitFingerprint,
) -> Result<PreparedCommitAuthority> {
    let request = plan.request();
    let payloads = request
        .payloads()
        .iter()
        .map(payload_json_value)
        .collect::<Result<Vec<_>>>()?;
    let required_artifacts = request
        .required_artifacts()
        .iter()
        .map(artifact_evidence_authority_json)
        .collect::<Result<Vec<_>>>()?;
    let admitted_artifacts = plan
        .admitted_artifacts()
        .iter()
        .map(artifact_evidence_authority_json)
        .collect::<Result<Vec<_>>>()?;
    let stable_request = serde_json::json!({
        "admitted_artifacts": admitted_artifacts,
        "commit_key": request.commit_key().as_str(),
        "commit_purpose": plan.purpose_name(),
        "hash_domain_version": HASH_DOMAIN_VERSION,
        "payloads": payloads,
        "preconditions": preconditions_authority_json(request.preconditions())?,
        "prepared_plan_fingerprint": fingerprint.as_digest().as_str(),
        "required_artifacts": required_artifacts,
        "run_id": request.run_id().as_str(),
    });
    let idempotency = canonical_json(serde_json::json!({
        "domain": "mfm.commit.idempotency.v1",
        "request": stable_request,
    }))?;
    let prepared = canonical_json(serde_json::json!({
        "domain": "mfm.commit.prepared_authority.v1",
        "expected_next_seq": request.expected_next_seq().as_u64(),
        "request": stable_request,
        "store_fill_policy": "mfm.store_fill.resource_lanes.v1",
    }))?;
    Ok(PreparedCommitAuthority {
        commit_idempotency_hash: idempotency.content_digest().as_str().to_owned(),
        idempotency_canonical_json: idempotency.as_bytes().to_vec(),
        prepared_authority_hash: prepared.content_digest().as_str().to_owned(),
        prepared_authority_canonical_json: prepared.as_bytes().to_vec(),
    })
}

pub(super) fn final_commit_authority(
    plan: &mfm_store::v1::PreparedCommitPlan,
    bundle: &PreparedCommitBundle,
    batch: &CommittedBatch,
    commit_id: &str,
) -> Result<FinalCommitAuthority> {
    final_commit_authority_from_parts(
        batch.run_id(),
        batch.seq(),
        batch.commit_key(),
        commit_id,
        batch.events(),
        plan.request().required_artifacts(),
        bundle.admitted_artifacts(),
    )
}

pub(super) fn final_commit_authority_from_parts(
    run_id: &RunId,
    seq: StreamSeq,
    commit_key: &CommitKey,
    commit_id: &str,
    events: &[KernelEventEnvelope],
    required_artifacts: &[ArtifactEvidenceRef],
    admitted_artifacts: &[ArtifactEvidenceRef],
) -> Result<FinalCommitAuthority> {
    let events = events
        .iter()
        .map(event_authority_json)
        .collect::<Result<Vec<_>>>()?;
    let required = sorted_artifact_binding_authority_json("required", required_artifacts)?;
    let admitted = sorted_artifact_binding_authority_json("admitted", admitted_artifacts)?;
    let canonical = canonical_json(serde_json::json!({
        "admitted_artifacts": admitted,
        "commit_id": commit_id,
        "commit_key": commit_key.as_str(),
        "domain": "mfm.commit.batch.v1",
        "events": events,
        "hash_domain_version": HASH_DOMAIN_VERSION,
        "required_artifacts": required,
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?;
    Ok(FinalCommitAuthority {
        commit_batch_hash: canonical.content_digest().as_str().to_owned(),
        commit_batch_canonical_json: canonical.as_bytes().to_vec(),
    })
}

pub(super) fn event_authority_json(event: &KernelEventEnvelope) -> Result<Value> {
    Ok(serde_json::json!({
        "commit_key": event.commit_key().as_str(),
        "event_id": event.event_id().as_str(),
        "event_schema_id": event.event_schema_id().as_str(),
        "logical_key": event.logical_key().as_str(),
        "ordinal": event.ordinal().as_u32(),
        "payload": payload_json_value(event.payload())?,
        "payload_hash": event.payload_hash().as_str(),
        "run_id": event.run_id().as_str(),
        "seq": event.seq().as_u64(),
        "spec_hash": event.spec_hash().as_str(),
    }))
}

pub(super) fn artifact_binding_authority_json(
    binding_kind: &str,
    evidence: &ArtifactEvidenceRef,
) -> Result<Value> {
    let mut value = artifact_evidence_authority_json(evidence)?;
    if let Value::Object(ref mut object) = value {
        object.insert(
            "binding_kind".to_owned(),
            Value::String(binding_kind.to_owned()),
        );
    }
    Ok(value)
}

pub(super) fn sorted_artifact_binding_authority_json(
    binding_kind: &str,
    evidence: &[ArtifactEvidenceRef],
) -> Result<Vec<Value>> {
    let mut bindings = evidence
        .iter()
        .map(|evidence| {
            Ok((
                evidence.artifact_id.as_str().to_owned(),
                evidence.evidence_hash()?.as_str().to_owned(),
                artifact_binding_authority_json(binding_kind, evidence)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    bindings.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(bindings
        .into_iter()
        .map(|(_artifact_id, _evidence_hash, value)| value)
        .collect())
}

pub(super) fn artifact_evidence_authority_json(evidence: &ArtifactEvidenceRef) -> Result<Value> {
    Ok(serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "artifact_role": evidence.artifact_role.as_str(),
        "byte_len": evidence.byte_len,
        "digest": evidence.digest.as_str(),
        "evidence_hash": evidence.evidence_hash()?.as_str(),
        "media_type": evidence.media_type.as_str(),
        "producer_node_id": evidence.producer_node_id.as_ref().map(NodeId::as_str),
        "producer_seed_id": evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        "schema_id": evidence.schema_id.as_ref().map(SchemaId::as_str),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    }))
}

pub(super) fn artifact_evidence_canonical_json(evidence: &ArtifactEvidenceRef) -> Result<Vec<u8>> {
    Ok(canonical_json(serde_json::json!({
        "domain": "mfm.artifact.evidence.v1",
        "evidence": artifact_evidence_authority_json(evidence)?,
    }))?
    .as_bytes()
    .to_vec())
}

pub(super) fn preconditions_authority_json(
    preconditions: &mfm_store::v1::CommitPreconditions,
) -> Result<Value> {
    Ok(serde_json::json!({
        "required_absent_logical_keys": preconditions.required_absent_logical_keys.iter().map(LogicalEventKey::as_str).collect::<Vec<_>>(),
        "required_cell_states": preconditions.required_cell_states.iter().map(cell_precondition_json).collect::<Vec<_>>(),
        "required_present_logical_keys": preconditions.required_present_logical_keys.iter().map(LogicalEventKey::as_str).collect::<Vec<_>>(),
        "required_public_output_absent": preconditions.required_public_output_absent,
        "required_run_state": required_run_state_tag(preconditions.required_run_state),
        "required_side_effect_states": preconditions.required_side_effect_states.iter().map(side_effect_precondition_json).collect::<Vec<_>>(),
        "saga_admit_token": preconditions.saga_admit_token.as_ref().map(saga_admit_token_json),
    }))
}

pub(super) fn cell_precondition_json(precondition: &mfm_store::v1::CellStatePrecondition) -> Value {
    serde_json::json!({
        "cell_id": precondition.cell_id.as_str(),
        "required": required_cell_state_tag(precondition.required),
    })
}

pub(super) fn side_effect_precondition_json(
    precondition: &mfm_store::v1::SideEffectStatePrecondition,
) -> Value {
    serde_json::json!({
        "ledger_key": precondition.ledger_key.as_str(),
        "required": required_side_effect_state_tag(precondition.required),
    })
}

pub(super) fn saga_admit_token_json(token: &mfm_store::v1::SagaAdmitToken) -> Value {
    serde_json::json!({
        "run_id": token.run_id().as_str(),
        "saga_policy_digest": token.saga_policy_digest().as_str(),
        "spec_hash": token.spec_hash().as_str(),
    })
}

pub(super) fn required_run_state_tag(value: mfm_store::v1::RequiredRunState) -> &'static str {
    match value {
        mfm_store::v1::RequiredRunState::Any => "any",
        mfm_store::v1::RequiredRunState::Absent => "absent",
        mfm_store::v1::RequiredRunState::Started => "started",
        mfm_store::v1::RequiredRunState::NotCompleted => "not_completed",
        mfm_store::v1::RequiredRunState::Completed => "completed",
    }
}

pub(super) fn required_cell_state_tag(value: mfm_store::v1::RequiredCellState) -> &'static str {
    match value {
        mfm_store::v1::RequiredCellState::Absent => "absent",
        mfm_store::v1::RequiredCellState::Produced => "produced",
        mfm_store::v1::RequiredCellState::Skipped => "skipped",
        mfm_store::v1::RequiredCellState::Terminal => "terminal",
    }
}

pub(super) fn required_side_effect_state_tag(
    value: mfm_store::v1::RequiredSideEffectState,
) -> &'static str {
    match value {
        mfm_store::v1::RequiredSideEffectState::Absent => "absent",
        mfm_store::v1::RequiredSideEffectState::IntentPersisted => "intent_persisted",
        mfm_store::v1::RequiredSideEffectState::Claimed => "claimed",
        mfm_store::v1::RequiredSideEffectState::InvocationPrepared => "invocation_prepared",
        mfm_store::v1::RequiredSideEffectState::InvocationStarted => "invocation_started",
        mfm_store::v1::RequiredSideEffectState::SubmissionResult => "submission_result",
        mfm_store::v1::RequiredSideEffectState::ReceiptObserved => "receipt_observed",
        mfm_store::v1::RequiredSideEffectState::ConfirmationObserved => "confirmation_observed",
        mfm_store::v1::RequiredSideEffectState::Ambiguous => "ambiguous",
        mfm_store::v1::RequiredSideEffectState::Failed => "failed",
    }
}
