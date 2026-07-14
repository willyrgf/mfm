use super::*;

pub(super) async fn read_commit_by_key(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &str,
) -> Result<Option<CommitAuthorityRow>> {
    let row = sqlx::query(
        "SELECT commit_id, run_id, seq, commit_key, commit_purpose, \
         prepared_commit_plan_fingerprint, commit_batch_hash, store_commit_order, event_count \
         FROM commits WHERE run_id = $1 AND commit_key = $2",
    )
    .bind(run_id.as_str())
    .bind(commit_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to query commit authority", error))?;
    row.map(commit_authority_row_from_row).transpose()
}

pub(super) async fn read_commit_by_seq(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    seq: StreamSeq,
) -> Result<CommitAuthorityRow> {
    let row = sqlx::query(
        "SELECT commit_id, run_id, seq, commit_key, commit_purpose, \
         prepared_commit_plan_fingerprint, commit_batch_hash, store_commit_order, event_count \
         FROM commits WHERE run_id = $1 AND seq = $2",
    )
    .bind(run_id.as_str())
    .bind(u64_to_i64(seq.as_u64(), "commits.seq")?)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load commit authority", error))?;
    commit_authority_row_from_row(row)
}

pub(super) async fn load_committed_batch_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    seq: StreamSeq,
) -> Result<CommittedBatch> {
    let commit = read_commit_by_seq(tx, run_id, seq).await?;
    let events = load_run_commit_events_tx(tx, run_id, seq).await?;
    if events.len() != commit.event_count {
        return Err(PostgresStoreError::Corruption(
            "commit event count does not match run_events rows".to_owned(),
        ));
    }
    validate_final_commit_authority_tx(tx, &commit, &events).await?;
    CommittedBatch::from_persisted_events(
        run_id.clone(),
        CommitKey::new(commit.commit_key)?,
        commit.prepared_commit_plan_fingerprint,
        seq,
        commit.store_commit_order,
        events,
    )
    .map_err(PostgresStoreError::from)
}

pub(super) async fn load_commit_authority_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<Vec<CommitAuthorityRow>> {
    let rows = sqlx::query(
        "SELECT commit_id, run_id, seq, commit_key, commit_purpose, \
         prepared_commit_plan_fingerprint, commit_batch_hash, store_commit_order, event_count \
         FROM commits WHERE run_id = $1 ORDER BY seq ASC",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load commit authority rows", error))?;
    rows.into_iter()
        .map(commit_authority_row_from_row)
        .collect()
}

pub(super) async fn validate_persisted_run_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<()> {
    let commits = load_commit_authority_rows_tx(tx, run_id).await?;
    for commit in &commits {
        let events = load_run_commit_events_tx(tx, &commit.run_id, commit.seq).await?;
        if events.len() != commit.event_count {
            return Err(PostgresStoreError::Corruption(
                "commit event count does not match run_events rows".to_owned(),
            ));
        }
        validate_final_commit_authority_tx(tx, commit, &events).await?;
    }
    Ok(())
}

pub(super) async fn validate_final_commit_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit: &CommitAuthorityRow,
    events: &[KernelEventEnvelope],
) -> Result<()> {
    let bindings = load_commit_artifact_bindings_tx(tx, commit).await?;
    let expected = final_commit_authority_from_parts(
        &commit.run_id,
        commit.seq,
        &CommitKey::new(commit.commit_key.clone())?,
        &commit.commit_id,
        events,
        &bindings.required,
        &bindings.admitted,
    )?;
    if expected.commit_batch_hash != commit.commit_batch_hash {
        return Err(PostgresStoreError::Corruption(
            "commit batch authority does not match persisted event and artifact bindings"
                .to_owned(),
        ));
    }
    Ok(())
}

pub(super) struct CommitArtifactBindings {
    pub(super) required: Vec<ArtifactEvidenceRef>,
    pub(super) admitted: Vec<ArtifactEvidenceRef>,
}

pub(super) async fn load_commit_artifact_bindings_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit: &CommitAuthorityRow,
) -> Result<CommitArtifactBindings> {
    let rows = sqlx::query(
        "SELECT cae.commit_key, cae.commit_id, cae.binding_kind, a.artifact_id, a.evidence_hash, \
          a.digest, a.byte_len, a.media_type, a.schema_id, a.semantic_type_id, \
          a.producer_node_id, a.producer_seed_id, a.artifact_role \
         FROM commit_artifact_evidence cae \
         INNER JOIN artifact_admissions a \
           ON a.artifact_id = cae.artifact_id AND a.evidence_hash = cae.evidence_hash \
         WHERE cae.run_id = $1 AND cae.seq = $2 \
         ORDER BY cae.binding_kind, cae.artifact_id, cae.evidence_hash",
    )
    .bind(commit.run_id.as_str())
    .bind(u64_to_i64(
        commit.seq.as_u64(),
        "commit_artifact_evidence.seq",
    )?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load commit artifact bindings", error))?;
    let mut bindings = CommitArtifactBindings {
        required: Vec::new(),
        admitted: Vec::new(),
    };
    for row in rows {
        let commit_key: String = row
            .try_get("commit_key")
            .map_err(|error| database_error("failed to decode artifact binding key", error))?;
        let commit_id: String = row
            .try_get("commit_id")
            .map_err(|error| database_error("failed to decode artifact binding commit", error))?;
        if commit_key != commit.commit_key || commit_id != commit.commit_id {
            return Err(PostgresStoreError::Corruption(
                "commit artifact binding does not match commit authority".to_owned(),
            ));
        }
        let artifact_id: String = row
            .try_get("artifact_id")
            .map_err(|error| database_error("failed to decode artifact binding row", error))?;
        let artifact_id = parse_identity::<ArtifactId>(&artifact_id)?;
        let evidence_hash: String = row
            .try_get("evidence_hash")
            .map_err(|error| database_error("failed to decode artifact binding row", error))?;
        let evidence_hash = parse_identity::<ContentDigest>(&evidence_hash)?;
        let evidence = ArtifactEvidenceParts {
            artifact_id: artifact_id.clone(),
            digest: row
                .try_get("digest")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
            byte_len: row
                .try_get("byte_len")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
            media_type: row
                .try_get("media_type")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
            schema_id: row
                .try_get("schema_id")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
            semantic_type_id: row
                .try_get("semantic_type_id")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
            producer_node_id: row
                .try_get("producer_node_id")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
            producer_seed_id: row
                .try_get("producer_seed_id")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
            artifact_role: row
                .try_get("artifact_role")
                .map_err(|error| database_error("failed to decode artifact binding row", error))?,
        }
        .into_evidence_ref()?;
        if evidence.evidence_hash()? != evidence_hash {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id,
                field: "evidence_hash",
            }
            .into());
        }
        let binding_kind: String = row
            .try_get("binding_kind")
            .map_err(|error| database_error("failed to decode artifact binding kind", error))?;
        match binding_kind.as_str() {
            "required" => bindings.required.push(evidence),
            "admitted" => bindings.admitted.push(evidence),
            _ => {
                return Err(PostgresStoreError::Corruption(
                    "unknown commit artifact binding kind".to_owned(),
                ))
            }
        }
    }
    Ok(bindings)
}

pub(super) async fn load_run_commit_events_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    seq: StreamSeq,
) -> Result<Vec<KernelEventEnvelope>> {
    let rows = sqlx::query(
        "SELECT e.run_id, e.seq, e.ordinal, c.store_commit_order, e.event_id, e.event_schema_id, \
         e.spec_hash, e.commit_key, e.logical_key, e.payload_hash, e.payload_canonical_json \
         FROM run_events e JOIN commits c ON c.run_id = e.run_id AND c.seq = e.seq \
         WHERE e.run_id = $1 AND e.seq = $2 ORDER BY e.ordinal ASC",
    )
    .bind(run_id.as_str())
    .bind(u64_to_i64(seq.as_u64(), "run_events.seq")?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load committed run events", error))?;
    rows.into_iter().map(event_envelope_from_row).collect()
}

pub(super) struct CommitAuthorityRow {
    pub(super) commit_id: String,
    pub(super) run_id: RunId,
    pub(super) commit_key: String,
    pub(super) prepared_commit_plan_fingerprint: CommitFingerprint,
    pub(super) commit_batch_hash: String,
    pub(super) seq: StreamSeq,
    pub(super) store_commit_order: StoreCommitOrder,
    pub(super) event_count: usize,
}

pub(super) fn commit_authority_row_from_row(row: PgRow) -> Result<CommitAuthorityRow> {
    let commit_id: String = row
        .try_get("commit_id")
        .map_err(|error| database_error("failed to decode commit id", error))?;
    let run_id = parse_identity::<RunId>(
        &row.try_get::<String, _>("run_id")
            .map_err(|error| database_error("failed to decode commit run id", error))?,
    )?;
    let commit_key = row
        .try_get::<String, _>("commit_key")
        .map_err(|error| database_error("failed to decode commit key", error))?;
    let commit_purpose: String = row
        .try_get("commit_purpose")
        .map_err(|error| database_error("failed to decode commit purpose", error))?;
    let prepared_commit_plan_fingerprint =
        CommitFingerprint::from_digest(parse_identity::<ContentDigest>(
            &row.try_get::<String, _>("prepared_commit_plan_fingerprint")
                .map_err(|error| database_error("failed to decode commit fingerprint", error))?,
        )?);
    let commit_batch_hash: String = row
        .try_get("commit_batch_hash")
        .map_err(|error| database_error("failed to decode commit batch hash", error))?;
    let seq: i64 = row
        .try_get("seq")
        .map_err(|error| database_error("failed to decode commit seq", error))?;
    let store_commit_order = StoreCommitOrder::new(i64_to_positive_u64(
        row.try_get("store_commit_order")
            .map_err(|error| database_error("failed to decode store commit order", error))?,
        "commits.store_commit_order",
    )?);
    let event_count: i32 = row
        .try_get("event_count")
        .map_err(|error| database_error("failed to decode commit event count", error))?;
    let seq = StreamSeq::new(i64_to_positive_u64(seq, "commits.seq")?)?;
    let commit_key_identity = CommitKey::new(commit_key.clone())?;
    let expected_commit_id = mfm_store::v1::backend::derive_commit_id(
        &run_id,
        seq,
        &commit_key_identity,
        &commit_purpose,
        &prepared_commit_plan_fingerprint,
    )?;
    if commit_id != expected_commit_id {
        return Err(PostgresStoreError::Corruption(
            "commit id does not match persisted fingerprint".to_owned(),
        ));
    }
    Ok(CommitAuthorityRow {
        commit_id,
        run_id,
        commit_key,
        prepared_commit_plan_fingerprint,
        commit_batch_hash,
        seq,
        store_commit_order,
        event_count: usize::try_from(event_count).map_err(|_| {
            PostgresStoreError::Corruption("commits.event_count was negative".to_owned())
        })?,
    })
}

pub(super) async fn next_store_commit_order_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<StoreCommitOrder> {
    let row =
        sqlx::query("SELECT current_order FROM store_commit_order WHERE singleton FOR UPDATE")
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| database_error("failed to lock store commit order", error))?;
    Ok(StoreCommitOrder::new(i64_to_positive_u64(
        row.try_get::<i64, _>("current_order")
            .map_err(|error| database_error("failed to decode store commit order", error))?
            .checked_add(1)
            .ok_or(PostgresStoreError::Corruption(
                "store commit order overflow".to_owned(),
            ))?,
        "next store commit order",
    )?))
}

pub(super) async fn advance_store_commit_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    store_commit_order: StoreCommitOrder,
) -> Result<()> {
    let updated = sqlx::query(
        "UPDATE store_commit_order SET current_order = $1 \
         WHERE singleton AND current_order = $2",
    )
    .bind(u64_to_i64(
        store_commit_order.as_u64(),
        "store_commit_order.current_order",
    )?)
    .bind(u64_to_i64(
        store_commit_order.as_u64() - 1,
        "previous store commit order",
    )?)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to advance store commit order", error))?;
    if updated.rows_affected() != 1 {
        return Err(PostgresStoreError::Corruption(
            "store commit order advanced from an unexpected value".to_owned(),
        ));
    }
    Ok(())
}
