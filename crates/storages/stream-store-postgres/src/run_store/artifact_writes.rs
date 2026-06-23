use super::*;

pub(super) async fn insert_prepared_artifact_bytes_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact: &PreparedArtifactBytes,
) -> Result<()> {
    verify_prepared_artifact_bytes(artifact)?;
    let evidence = artifact.evidence();
    let evidence_hash = artifact.evidence_hash();
    let evidence_canonical_json = artifact_evidence_canonical_json(evidence)?;
    let byte_len = u64_to_i64(evidence.byte_len, "artifact_blobs.byte_len")?;
    insert_artifact_blob_tx(tx, artifact).await?;
    sqlx::query(
        "INSERT INTO artifact_admissions \
         (evidence_hash, artifact_id, digest, byte_len, media_type, \
          schema_id, semantic_type_id, producer_node_id, producer_seed_id, artifact_role, \
          evidence_canonical_json) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
         ON CONFLICT (evidence_hash) DO NOTHING",
    )
    .bind(evidence_hash.as_str())
    .bind(evidence.artifact_id.as_str())
    .bind(evidence.digest.as_str())
    .bind(byte_len)
    .bind(evidence.media_type.as_str())
    .bind(evidence.schema_id.as_ref().map(SchemaId::as_str))
    .bind(
        evidence
            .semantic_type_id
            .as_ref()
            .map(SemanticTypeId::as_str),
    )
    .bind(evidence.producer_node_id.as_ref().map(NodeId::as_str))
    .bind(evidence.producer_seed_id.as_ref().map(SeedId::as_str))
    .bind(evidence.artifact_role.as_str())
    .bind(evidence_canonical_json.as_slice())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert artifact admission", error))?
    .rows_affected();
    let record = load_artifact_record_tx(tx, &evidence.artifact_id, evidence_hash).await?;
    verify_artifact_record(&record, evidence, Some(artifact.bytes()))?;
    Ok(())
}

pub(super) async fn insert_artifact_blob_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact: &PreparedArtifactBytes,
) -> Result<()> {
    verify_prepared_artifact_bytes(artifact)?;
    let evidence = artifact.evidence();
    let byte_len = u64_to_i64(evidence.byte_len, "artifact_blobs.byte_len")?;
    sqlx::query(
        "INSERT INTO artifact_blobs (artifact_id, digest, byte_len, bytes) \
         VALUES ($1,$2,$3,$4) \
         ON CONFLICT (artifact_id) DO NOTHING",
    )
    .bind(evidence.artifact_id.as_str())
    .bind(evidence.digest.as_str())
    .bind(byte_len)
    .bind(artifact.bytes())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert artifact blob", error))?;
    verify_artifact_blob_tx(tx, artifact).await
}

pub(super) async fn verify_artifact_blob_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact: &PreparedArtifactBytes,
) -> Result<()> {
    let evidence = artifact.evidence();
    let row =
        sqlx::query("SELECT digest, byte_len, bytes FROM artifact_blobs WHERE artifact_id = $1")
            .bind(evidence.artifact_id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| database_error("failed to load artifact blob", error))?;
    let Some(row) = row else {
        return Err(StoreError::MissingArtifact {
            artifact_id: evidence.artifact_id.clone(),
        }
        .into());
    };
    let digest: String = row
        .try_get("digest")
        .map_err(|error| database_error("failed to decode artifact blob", error))?;
    let byte_len: i64 = row
        .try_get("byte_len")
        .map_err(|error| database_error("failed to decode artifact blob", error))?;
    let bytes: Vec<u8> = row
        .try_get("bytes")
        .map_err(|error| database_error("failed to decode artifact blob", error))?;
    if digest != evidence.digest.as_str()
        || i64_to_nonnegative_u64(byte_len, "artifact_blobs.byte_len")? != evidence.byte_len
        || bytes.as_slice() != artifact.bytes()
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "artifact_blob",
        }
        .into());
    }
    Ok(())
}

pub(super) async fn link_run_artifact_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &CommitKey,
    seq: StreamSeq,
    commit_id: &str,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    insert_commit_artifact_evidence_tx(
        tx, run_id, commit_key, seq, commit_id, evidence, "admitted",
    )
    .await?;
    let evidence_hash = evidence.evidence_hash()?;
    sqlx::query(
        "INSERT INTO run_artifact_admissions \
         (run_id, artifact_id, evidence_hash, first_commit_id, first_commit_key, first_seq, \
          first_binding_kind) \
         VALUES ($1,$2,$3,$4,$5,$6,'admitted') \
         ON CONFLICT (run_id, artifact_id, evidence_hash) DO NOTHING",
    )
    .bind(run_id.as_str())
    .bind(evidence.artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .bind(commit_id)
    .bind(commit_key.as_str())
    .bind(u64_to_i64(
        seq.as_u64(),
        "run_artifact_admissions.first_seq",
    )?)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert run artifact evidence", error))?;
    Ok(())
}

pub(super) async fn insert_commit_artifact_evidence_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &CommitKey,
    seq: StreamSeq,
    commit_id: &str,
    evidence: &ArtifactEvidenceRef,
    binding_kind: &str,
) -> Result<()> {
    let evidence_hash = evidence.evidence_hash()?;
    sqlx::query(
        "INSERT INTO commit_artifact_evidence \
         (run_id, seq, commit_key, commit_id, artifact_id, evidence_hash, binding_kind) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) \
         ON CONFLICT (run_id, seq, binding_kind, artifact_id, evidence_hash) DO NOTHING",
    )
    .bind(run_id.as_str())
    .bind(u64_to_i64(seq.as_u64(), "commit_artifact_evidence.seq")?)
    .bind(commit_key.as_str())
    .bind(commit_id)
    .bind(evidence.artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .bind(binding_kind)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert commit artifact evidence", error))?;
    Ok(())
}

pub(super) async fn insert_run_commit_log_tx(
    tx: &mut Transaction<'_, Postgres>,
    request: &mfm_store::v1::CommitRequest,
    commit_id: &str,
    commit_batch_hash: &str,
    seq: StreamSeq,
    event_count: i32,
) -> Result<()> {
    let last_ordinal = event_count.checked_sub(1).ok_or_else(|| {
        PostgresStoreError::Corruption("run commit log event count underflow".to_owned())
    })?;
    let commit_sort_key = run_commit_sort_key(
        request.run_id(),
        seq,
        request.commit_key(),
        commit_id,
        commit_batch_hash,
    )?;
    sqlx::query(
        "INSERT INTO run_commit_log \
         (commit_id, run_id, seq, commit_key, first_ordinal, last_ordinal, event_count, \
          commit_sort_key) \
         VALUES ($1,$2,$3,$4,0,$5,$6,$7)",
    )
    .bind(commit_id)
    .bind(request.run_id().as_str())
    .bind(u64_to_i64(seq.as_u64(), "run_commit_log.seq")?)
    .bind(request.commit_key().as_str())
    .bind(last_ordinal)
    .bind(event_count)
    .bind(commit_sort_key)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert run commit log", error))?;
    Ok(())
}
