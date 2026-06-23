use super::*;

pub(super) struct FinalCommitAuthority {
    pub(super) commit_batch_hash: String,
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
