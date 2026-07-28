use super::*;

pub(super) fn insert_fact_descriptor_candidate<'a>(
    candidates: &mut Vec<(StoredArtifactEvidenceRef, &'a [u8])>,
    evidence: &StoredArtifactEvidenceRef,
    bytes: &'a [u8],
) -> Result<()> {
    let key = replay_artifact_key(evidence)?;
    for (existing, existing_bytes) in candidates.iter() {
        if replay_artifact_key(existing)? != key {
            continue;
        }
        if existing != evidence || *existing_bytes != bytes {
            return Err(ReplayError::new(
                ReplayErrorKind::ArtifactMismatch,
                format!(
                    "conflicting retained fact descriptor evidence for {}",
                    evidence.artifact_id
                ),
            ));
        }
        return Ok(());
    }
    candidates.push((evidence.clone(), bytes));
    Ok(())
}

pub(super) fn verify_artifact_expectation(
    evidence: &StoredArtifactEvidenceRef,
    expected: ArtifactEvidenceExpectation<'_>,
) -> Result<()> {
    if evidence.artifact_id != *expected.artifact_id {
        return Err(ReplayError::new(
            ReplayErrorKind::ArtifactMismatch,
            format!(
                "artifact id mismatch: expected {}, found {}",
                expected.artifact_id, evidence.artifact_id
            ),
        ));
    }
    let actual_evidence_hash = evidence.evidence_hash().map_err(ReplayError::from)?;
    if &actual_evidence_hash != expected.evidence_hash {
        return Err(ReplayError::new(
            ReplayErrorKind::ArtifactMismatch,
            format!(
                "artifact evidence hash mismatch for {}",
                evidence.artifact_id
            ),
        ));
    }
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: evidence.artifact_id.clone(),
        evidence_hash: actual_evidence_hash,
        digest: Some(expected.digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: expected.schema_id.cloned(),
        semantic_type_id: expected.semantic_type_id.cloned(),
        producer_node_id: expected.producer_node_id.cloned(),
        producer_seed_id: expected.producer_seed_id.cloned(),
        artifact_role: Some(expected.role),
    };
    store::validate_artifact_requirement_against_evidence(&requirement, evidence)
        .map_err(|error| artifact_requirement_replay_error(error, "artifact evidence mismatch"))
}

pub(super) fn replay_artifact_key(
    evidence: &StoredArtifactEvidenceRef,
) -> Result<ReplayArtifactKey> {
    Ok((
        evidence.artifact_id.clone(),
        evidence.evidence_hash().map_err(ReplayError::from)?,
    ))
}

pub(super) fn artifact_requirement_replay_error(
    error: store::StoreError,
    context: &'static str,
) -> ReplayError {
    match error {
        store::StoreError::ArtifactEvidenceMismatch { artifact_id, field } => ReplayError::new(
            ReplayErrorKind::ArtifactMismatch,
            format!("{context} for {artifact_id} field {field}"),
        ),
        error => error.into(),
    }
}

pub(super) fn certified_spec_error(error: mfm_spec::SpecError) -> ReplayError {
    ReplayError::new(
        ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

pub(super) fn verify_replay_verifier(
    requested: Option<&events::ReplayVerifierId>,
    recorded: &events::ReplayVerifierId,
) -> Result<()> {
    match requested {
        Some(requested) if requested == recorded => Ok(()),
        _ => Err(ReplayError::new(
            ReplayErrorKind::ReplayVerifierMismatch,
            "side-effect replay verifier id does not match recorded evidence",
        )),
    }
}

pub(super) fn side_effect_mismatch(message: &'static str) -> ReplayError {
    ReplayError::new(ReplayErrorKind::SideEffectMismatch, message)
}
