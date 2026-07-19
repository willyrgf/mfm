use super::*;

pub(super) async fn verify_prepared_artifact_bundle_tx(
    tx: &mut Transaction<'_, Postgres>,
    bundle: &PreparedCommitBundle,
) -> Result<()> {
    for artifact in bundle.artifact_bytes() {
        verify_prepared_artifact_bytes(artifact)?;
    }
    for existing in bundle.existing_artifacts() {
        let evidence = mfm_store::v1::backend::admitted_artifact_evidence(
            bundle,
            existing.artifact_id(),
            existing.evidence_hash(),
        )?;
        let record =
            load_artifact_record_tx(tx, existing.artifact_id(), existing.evidence_hash()).await?;
        verify_artifact_record(&record, evidence, None)?;
    }
    Ok(())
}

pub(super) async fn admit_artifact_bundle_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &CommitKey,
    seq: StreamSeq,
    commit_id: &str,
    bundle: &PreparedCommitBundle,
) -> Result<()> {
    for artifact in bundle.artifact_bytes() {
        insert_prepared_artifact_bytes_tx(tx, artifact).await?;
    }
    for existing in bundle.existing_artifacts() {
        let evidence = mfm_store::v1::backend::admitted_artifact_evidence(
            bundle,
            existing.artifact_id(),
            existing.evidence_hash(),
        )?;
        let record =
            load_artifact_record_tx(tx, existing.artifact_id(), existing.evidence_hash()).await?;
        verify_artifact_record(&record, evidence, None)?;
    }
    for evidence in bundle.request().required_artifacts() {
        insert_commit_artifact_evidence_tx(
            tx, run_id, commit_key, seq, commit_id, evidence, "required",
        )
        .await?;
    }
    for evidence in bundle.admitted_artifacts() {
        link_run_artifact_tx(tx, run_id, commit_key, seq, commit_id, evidence).await?;
    }
    Ok(())
}

pub(super) fn verify_prepared_artifact_bytes(artifact: &PreparedArtifactBytes) -> Result<()> {
    let verified =
        PreparedArtifactBytes::new(artifact.bytes().to_vec(), artifact.evidence().clone())?;
    if verified.evidence_hash() != artifact.evidence_hash() {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact.evidence().artifact_id.clone(),
            field: "evidence_hash",
        }
        .into());
    }
    validate_artifact_blob_size(artifact.evidence())?;
    Ok(())
}

pub(super) fn validate_artifact_blob_size(evidence: &ArtifactEvidenceRef) -> Result<()> {
    if evidence.byte_len <= MAX_ARTIFACT_BLOB_BYTES {
        Ok(())
    } else {
        Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "byte_len",
        }
        .into())
    }
}
