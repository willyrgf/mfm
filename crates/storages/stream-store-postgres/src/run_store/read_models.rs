use super::*;

pub(super) async fn notify_observation_change_tx(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(OBSERVATION_NOTIFY_CHANNEL)
        .bind(OBSERVATION_NOTIFY_PAYLOAD)
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to notify observation change", error))?;
    Ok(())
}

pub(super) async fn insert_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    commit_id: &str,
    head_seq: StreamSeq,
    source_event_count: i64,
) -> Result<()> {
    let observed_status = observed_status_from_run_state(projections.run_state(run_id))?;
    let summary = materialize_run_observation_summary_tx(
        tx,
        PROJECTION_VERSION,
        run_id,
        commit_id,
        head_seq,
        observed_status,
        source_event_count,
    )
    .await?;
    if !insert_materialized_run_observation_summary_tx(tx, &summary).await? {
        return Err(PostgresStoreError::Corruption(
            "append observation summary already existed".to_owned(),
        ));
    }
    Ok(())
}

pub(super) struct RunObservationSummaryMaterialization {
    pub(super) projection_version: String,
    pub(super) commit_id: String,
    pub(super) summary_kind: &'static str,
    pub(super) run_id: RunId,
    pub(super) head_seq: StreamSeq,
    pub(super) observed_status: ObservedRunStatus,
    #[cfg(all(test, feature = "parity-tests"))]
    pub(super) started_at: String,
    #[cfg(all(test, feature = "parity-tests"))]
    pub(super) updated_at: String,
    #[cfg(all(test, feature = "parity-tests"))]
    pub(super) completed_at: Option<String>,
    pub(super) source_authority_hash: String,
    pub(super) source_event_count: i64,
    pub(super) summary_row_hash: String,
    pub(super) summary_row_canonical_json: PlainCanonicalJsonBytes,
    pub(super) derivation: ObservationDerivationMaterialization,
}

pub(super) struct ObservationDerivationMaterialization {
    pub(super) projection_version: String,
    pub(super) model_name: &'static str,
    pub(super) derived_key: String,
    pub(super) source_kind: &'static str,
    pub(super) source_run_id: RunId,
    pub(super) source_from_seq: StreamSeq,
    pub(super) source_to_seq: StreamSeq,
    pub(super) source_last_ordinal: i32,
    pub(super) source_high_append_xid: String,
    pub(super) source_high_commit_id: String,
    pub(super) source_high_commit_sort_key: Vec<u8>,
    pub(super) source_event_count: i64,
    pub(super) source_input_hash: String,
    pub(super) derived_row_hash: String,
    pub(super) derived_row_canonical_json: PlainCanonicalJsonBytes,
}

pub(super) async fn materialize_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection_version: &str,
    run_id: &RunId,
    commit_id: &str,
    head_seq: StreamSeq,
    observed_status: ObservedRunStatus,
    source_event_count: i64,
) -> Result<RunObservationSummaryMaterialization> {
    if source_event_count < 1 {
        return Err(PostgresStoreError::Corruption(
            "observation source event count must be positive".to_owned(),
        ));
    }
    let row = sqlx::query(
        "SELECT \
           to_char((SELECT MIN(committed_at) FROM commits WHERE run_id = $1) AT TIME ZONE 'UTC', \
             'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
           to_char(c.committed_at AT TIME ZONE 'UTC', \
             'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
           CASE WHEN $2 = 'completed' THEN \
             to_char(c.committed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
           ELSE NULL END AS completed_at, \
           l.append_xid::text AS source_high_append_xid, \
           l.last_ordinal AS source_last_ordinal, \
           l.commit_sort_key AS source_high_commit_sort_key \
         FROM commits c \
         INNER JOIN run_commit_log l ON l.commit_id = c.commit_id \
         WHERE c.commit_id = $3",
    )
    .bind(run_id.as_str())
    .bind(observed_status.as_str())
    .bind(commit_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to materialize observation timestamps", error))?;
    let started_at: String = row
        .try_get("started_at")
        .map_err(|error| database_error("failed to decode observation started_at", error))?;
    let updated_at: String = row
        .try_get("updated_at")
        .map_err(|error| database_error("failed to decode observation updated_at", error))?;
    let completed_at: Option<String> = row
        .try_get("completed_at")
        .map_err(|error| database_error("failed to decode observation completed_at", error))?;
    let source_high_append_xid: String = row
        .try_get("source_high_append_xid")
        .map_err(|error| database_error("failed to decode observation source append xid", error))?;
    let source_last_ordinal: i32 = row.try_get("source_last_ordinal").map_err(|error| {
        database_error("failed to decode observation source last ordinal", error)
    })?;
    let source_high_commit_sort_key: Vec<u8> = row
        .try_get("source_high_commit_sort_key")
        .map_err(|error| database_error("failed to decode observation source sort key", error))?;
    let source_authority_hash = observation_source_authority_hash(
        run_id,
        commit_id,
        head_seq,
        observed_status,
        source_event_count,
    )?;
    let summary_row_canonical_json =
        run_observation_summary_canonical_json(&RunObservationSummaryCanonicalFields {
            projection_version,
            run_id,
            commit_id,
            head_seq,
            observed_status,
            started_at: &started_at,
            updated_at: &updated_at,
            completed_at: completed_at.as_deref(),
            source_authority_hash: &source_authority_hash,
            source_event_count,
        })?;
    let summary_row_hash = summary_row_canonical_json
        .content_digest()
        .as_str()
        .to_owned();
    let derived_key = observation_derivation_key(run_id, commit_id);
    let source_input_hash = observation_source_input_hash(
        run_id,
        head_seq,
        source_last_ordinal,
        commit_id,
        &source_high_append_xid,
        &source_high_commit_sort_key,
        source_event_count,
    )?;
    let derivation = ObservationDerivationMaterialization {
        projection_version: projection_version.to_owned(),
        model_name: RUN_OBSERVATION_DERIVATION_MODEL,
        derived_key,
        source_kind: RUN_PREFIX_SOURCE_KIND,
        source_run_id: run_id.clone(),
        source_from_seq: StreamSeq::FIRST,
        source_to_seq: head_seq,
        source_last_ordinal,
        source_high_append_xid,
        source_high_commit_id: commit_id.to_owned(),
        source_high_commit_sort_key,
        source_event_count,
        source_input_hash,
        derived_row_hash: summary_row_hash.clone(),
        derived_row_canonical_json: summary_row_canonical_json.clone(),
    };
    Ok(RunObservationSummaryMaterialization {
        projection_version: projection_version.to_owned(),
        commit_id: commit_id.to_owned(),
        summary_kind: RUN_OBSERVATION_SUMMARY_KIND,
        run_id: run_id.clone(),
        head_seq,
        observed_status,
        #[cfg(all(test, feature = "parity-tests"))]
        started_at,
        #[cfg(all(test, feature = "parity-tests"))]
        updated_at,
        #[cfg(all(test, feature = "parity-tests"))]
        completed_at,
        source_authority_hash,
        source_event_count,
        summary_row_hash,
        summary_row_canonical_json,
        derivation,
    })
}

pub(super) fn observed_status_from_run_state(run_state: RunState) -> Result<ObservedRunStatus> {
    match run_state {
        RunState::Started => Ok(ObservedRunStatus::Started),
        RunState::Completed => Ok(ObservedRunStatus::Completed),
        RunState::Absent => Err(StoreError::ObservationUnavailable {
            message: "committed run observation had absent run state".to_owned(),
        }
        .into()),
    }
}

pub(super) fn observation_source_authority_hash(
    run_id: &RunId,
    commit_id: &str,
    head_seq: StreamSeq,
    observed_status: ObservedRunStatus,
    source_event_count: i64,
) -> Result<String> {
    Ok(canonical_json(serde_json::json!({
        "commit_id": commit_id,
        "domain": "mfm.run_observation.summary.source.v1",
        "head_seq": head_seq.as_u64(),
        "observed_status": observed_status.as_str(),
        "run_id": run_id.as_str(),
        "source_event_count": source_event_count,
    }))?
    .content_digest()
    .as_str()
    .to_owned())
}

pub(super) fn observation_derivation_key(run_id: &RunId, commit_id: &str) -> String {
    format!(
        "run:{}:commit:{}:summary:{}",
        run_id.as_str(),
        commit_id,
        RUN_OBSERVATION_SUMMARY_KIND
    )
}

pub(super) fn observation_source_input_hash(
    run_id: &RunId,
    head_seq: StreamSeq,
    source_last_ordinal: i32,
    commit_id: &str,
    source_high_append_xid: &str,
    source_high_commit_sort_key: &[u8],
    source_event_count: i64,
) -> Result<String> {
    Ok(canonical_json(serde_json::json!({
        "commit_id": commit_id,
        "domain": "mfm.run_observation.source_input.v1",
        "source_event_count": source_event_count,
        "source_high_append_xid": source_high_append_xid,
        "source_high_commit_sort_key": bytes_hex(source_high_commit_sort_key),
        "source_kind": RUN_PREFIX_SOURCE_KIND,
        "source_last_ordinal": source_last_ordinal,
        "source_run_id": run_id.as_str(),
        "source_to_seq": head_seq.as_u64(),
    }))?
    .content_digest()
    .as_str()
    .to_owned())
}

pub(super) struct RunObservationSummaryCanonicalFields<'a> {
    pub(super) projection_version: &'a str,
    pub(super) run_id: &'a RunId,
    pub(super) commit_id: &'a str,
    pub(super) head_seq: StreamSeq,
    pub(super) observed_status: ObservedRunStatus,
    pub(super) started_at: &'a str,
    pub(super) updated_at: &'a str,
    pub(super) completed_at: Option<&'a str>,
    pub(super) source_authority_hash: &'a str,
    pub(super) source_event_count: i64,
}

pub(super) fn run_observation_summary_canonical_json(
    fields: &RunObservationSummaryCanonicalFields<'_>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "commit_id": fields.commit_id,
        "completed_at": fields.completed_at,
        "domain": "mfm.run_observation.summary.row.v1",
        "head_seq": fields.head_seq.as_u64(),
        "observed_status": fields.observed_status.as_str(),
        "projection_version": fields.projection_version,
        "run_id": fields.run_id.as_str(),
        "source_authority_hash": fields.source_authority_hash,
        "source_event_count": fields.source_event_count,
        "started_at": fields.started_at,
        "summary_kind": RUN_OBSERVATION_SUMMARY_KIND,
        "updated_at": fields.updated_at,
    }))
}

#[cfg(all(test, feature = "parity-tests"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadModelValidationReport {
    pub(super) checked_commits: u64,
    pub(super) missing_rows: u64,
    pub(super) drift_rows: u64,
}

#[cfg(all(test, feature = "parity-tests"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadModelDrift {
    pub(super) kind: ReadModelDriftKind,
}

#[cfg(all(test, feature = "parity-tests"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadModelDriftKind {
    Missing,
    Mismatched,
    Orphaned,
}

#[cfg(all(test, feature = "parity-tests"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadModelDriftReport {
    pub(super) findings: Vec<ReadModelDrift>,
}

#[cfg(all(test, feature = "parity-tests"))]
pub(super) async fn validate_read_models_from_pool(
    pool: &PgPool,
    projection_version: &str,
) -> Result<ReadModelValidationReport> {
    validate_projection_version(projection_version)?;
    let mut tx = pool.begin().await.map_err(|error| {
        database_error("failed to start read-model validation transaction", error)
    })?;
    let summaries =
        materialize_all_run_observation_summaries_tx(&mut tx, projection_version).await?;
    let mut report = ReadModelValidationReport {
        checked_commits: 0,
        missing_rows: 0,
        drift_rows: 0,
    };
    for summary in summaries {
        report.checked_commits = report.checked_commits.checked_add(1).ok_or_else(|| {
            PostgresStoreError::Corruption("read-model checked row count overflow".to_owned())
        })?;
        match compare_existing_run_observation_summary_tx(&mut tx, &summary).await? {
            SummaryComparison::Match => {}
            SummaryComparison::Missing => {
                report.missing_rows = report.missing_rows.checked_add(1).ok_or_else(|| {
                    PostgresStoreError::Corruption(
                        "read-model missing row count overflow".to_owned(),
                    )
                })?;
            }
            SummaryComparison::Mismatch => {
                report.drift_rows = report.drift_rows.checked_add(1).ok_or_else(|| {
                    PostgresStoreError::Corruption("read-model drift row count overflow".to_owned())
                })?;
            }
        }
    }
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read-model validation", error))?;
    Ok(report)
}

#[cfg(all(test, feature = "parity-tests"))]
pub(super) async fn read_model_drift_report_from_pool(
    pool: &PgPool,
) -> Result<ReadModelDriftReport> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start read-model drift transaction", error))?;
    let version_rows =
        sqlx::query("SELECT DISTINCT projection_version FROM run_observation_change_summaries")
            .fetch_all(&mut *tx)
            .await
            .map_err(|error| database_error("failed to load read-model versions", error))?;
    let mut versions = BTreeSet::from([PROJECTION_VERSION.to_owned()]);
    for row in version_rows {
        versions.insert(row.try_get("projection_version").map_err(|error| {
            database_error("failed to decode read-model projection version", error)
        })?);
    }

    let mut findings = Vec::new();
    for projection_version in versions {
        let summaries =
            materialize_all_run_observation_summaries_tx(&mut tx, &projection_version).await?;
        for summary in summaries {
            match compare_existing_run_observation_summary_tx(&mut tx, &summary).await? {
                SummaryComparison::Match => {}
                SummaryComparison::Missing => findings.push(ReadModelDrift {
                    kind: ReadModelDriftKind::Missing,
                }),
                SummaryComparison::Mismatch => findings.push(ReadModelDrift {
                    kind: ReadModelDriftKind::Mismatched,
                }),
            }
        }
    }
    let orphan_rows = sqlx::query(
        "SELECT s.projection_version, s.commit_id, s.summary_kind \
         FROM run_observation_change_summaries s \
         LEFT JOIN run_commit_log l ON l.commit_id = s.commit_id \
         WHERE l.commit_id IS NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| database_error("failed to load orphaned read-model rows", error))?;
    for row in orphan_rows {
        let _: String = row
            .try_get("projection_version")
            .map_err(|error| database_error("failed to decode orphan projection version", error))?;
        let _: String = row
            .try_get("commit_id")
            .map_err(|error| database_error("failed to decode orphan commit id", error))?;
        let _: String = row
            .try_get("summary_kind")
            .map_err(|error| database_error("failed to decode orphan summary kind", error))?;
        findings.push(ReadModelDrift {
            kind: ReadModelDriftKind::Orphaned,
        });
    }
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read-model drift transaction", error))?;
    Ok(ReadModelDriftReport { findings })
}

#[cfg(all(test, feature = "parity-tests"))]
pub(super) fn validate_projection_version(projection_version: &str) -> Result<()> {
    if projection_version.trim().is_empty() {
        return Err(StoreError::ObservationUnavailable {
            message: "projection version must not be empty".to_owned(),
        }
        .into());
    }
    Ok(())
}

pub(super) enum SummaryComparison {
    Missing,
    Match,
    Mismatch,
}

#[cfg(all(test, feature = "parity-tests"))]
pub(super) async fn materialize_all_run_observation_summaries_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection_version: &str,
) -> Result<Vec<RunObservationSummaryMaterialization>> {
    let rows = sqlx::query("SELECT DISTINCT run_id FROM commits ORDER BY run_id ASC")
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load read-model run ids", error))?;
    let mut summaries = Vec::new();
    for row in rows {
        let run_id = parse_identity::<RunId>(
            &row.try_get::<String, _>("run_id")
                .map_err(|error| database_error("failed to decode read-model run id", error))?,
        )?;
        let commits = load_commit_authority_rows_tx(tx, &run_id).await?;
        let stream = load_run_stream_tx(tx, &run_id).await?;
        let mut prefix = Vec::new();
        let mut cursor = 0_usize;
        for commit in commits {
            while cursor < stream.len() && stream[cursor].seq().as_u64() <= commit.seq.as_u64() {
                prefix.push(stream[cursor].clone());
                cursor += 1;
            }
            if prefix.last().map(KernelEventEnvelope::seq) != Some(commit.seq) {
                return Err(PostgresStoreError::Corruption(
                    "commit has no authority event in rebuilt observation prefix".to_owned(),
                ));
            }
            let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&prefix)?;
            let observed_status = observed_status_from_run_state(snapshot.run_state(&run_id))?;
            let source_event_count = i64::try_from(prefix.len()).map_err(|_| {
                PostgresStoreError::Corruption(
                    "read-model source event count exceeded PostgreSQL bigint range".to_owned(),
                )
            })?;
            summaries.push(
                materialize_run_observation_summary_tx(
                    tx,
                    projection_version,
                    &run_id,
                    &commit.commit_id,
                    commit.seq,
                    observed_status,
                    source_event_count,
                )
                .await?,
            );
        }
    }
    Ok(summaries)
}

#[cfg(all(test, feature = "parity-tests"))]
pub(super) async fn compare_existing_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    summary: &RunObservationSummaryMaterialization,
) -> Result<SummaryComparison> {
    let row = sqlx::query(
        "SELECT run_id, head_seq, observed_status, \
          to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
          to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
          CASE WHEN completed_at IS NULL THEN NULL \
            ELSE to_char(completed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
          END AS completed_at, \
          source_authority_hash, source_event_count, summary_row_hash, summary_row_canonical_json \
         FROM run_observation_change_summaries \
         WHERE projection_version = $1 AND commit_id = $2 AND summary_kind = $3",
    )
    .bind(summary.projection_version.as_str())
    .bind(summary.commit_id.as_str())
    .bind(summary.summary_kind)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load existing observation summary", error))?;
    let Some(row) = row else {
        return Ok(SummaryComparison::Missing);
    };
    let run_id_text: String = row
        .try_get("run_id")
        .map_err(|error| database_error("failed to decode summary run id", error))?;
    let run_id = parse_identity::<RunId>(&run_id_text)?;
    let head_seq = StreamSeq::new(i64_to_positive_u64(
        row.try_get("head_seq")
            .map_err(|error| database_error("failed to decode summary head seq", error))?,
        "run_observation_change_summaries.head_seq",
    )?)?;
    let observed_status_text: String = row
        .try_get("observed_status")
        .map_err(|error| database_error("failed to decode summary observed status", error))?;
    let source_event_count: i64 = row
        .try_get("source_event_count")
        .map_err(|error| database_error("failed to decode summary source count", error))?;
    let summary_row_hash: String = row
        .try_get("summary_row_hash")
        .map_err(|error| database_error("failed to decode summary row hash", error))?;
    let summary_row_canonical_json: Vec<u8> = row
        .try_get("summary_row_canonical_json")
        .map_err(|error| database_error("failed to decode summary row canonical bytes", error))?;
    let stored_canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(
        &summary_row_canonical_json,
    )
    .map_err(|error| {
        PostgresStoreError::Corruption(format!(
            "read-model summary canonical bytes are invalid: {error}"
        ))
    })?;
    if stored_canonical.content_digest().as_str() != summary_row_hash {
        return Ok(SummaryComparison::Mismatch);
    }
    let matches = run_id == summary.run_id
        && head_seq == summary.head_seq
        && observed_status_text == summary.observed_status.as_str()
        && row
            .try_get::<String, _>("started_at")
            .map_err(|error| database_error("failed to decode summary started_at", error))?
            == summary.started_at
        && row
            .try_get::<String, _>("updated_at")
            .map_err(|error| database_error("failed to decode summary updated_at", error))?
            == summary.updated_at
        && row
            .try_get::<Option<String>, _>("completed_at")
            .map_err(|error| database_error("failed to decode summary completed_at", error))?
            == summary.completed_at
        && row
            .try_get::<String, _>("source_authority_hash")
            .map_err(|error| database_error("failed to decode summary source hash", error))?
            == summary.source_authority_hash
        && source_event_count == summary.source_event_count
        && summary_row_hash == summary.summary_row_hash
        && summary_row_canonical_json == summary.summary_row_canonical_json.as_bytes();
    if !matches {
        return Ok(SummaryComparison::Mismatch);
    }
    compare_existing_observation_derivation_tx(tx, &summary.derivation).await
}

pub(super) async fn compare_existing_observation_derivation_tx(
    tx: &mut Transaction<'_, Postgres>,
    derivation: &ObservationDerivationMaterialization,
) -> Result<SummaryComparison> {
    let row = sqlx::query(
        "SELECT source_kind, source_run_id, source_from_seq, source_to_seq, source_last_ordinal, \
          source_high_append_xid::text AS source_high_append_xid, source_high_commit_id, \
          source_high_commit_sort_key, source_event_count, source_input_hash, \
          derived_row_canonical_json, derived_row_hash \
         FROM observation_derivations \
         WHERE projection_version = $1 AND model_name = $2 AND derived_key = $3",
    )
    .bind(derivation.projection_version.as_str())
    .bind(derivation.model_name)
    .bind(derivation.derived_key.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load observation derivation", error))?;
    let Some(row) = row else {
        return Ok(SummaryComparison::Missing);
    };
    let derived_row_hash: String = row
        .try_get("derived_row_hash")
        .map_err(|error| database_error("failed to decode derivation row hash", error))?;
    let derived_row_canonical_json: Vec<u8> =
        row.try_get("derived_row_canonical_json").map_err(|error| {
            database_error("failed to decode derivation row canonical bytes", error)
        })?;
    let stored_canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(
        &derived_row_canonical_json,
    )
    .map_err(|error| {
        PostgresStoreError::Corruption(format!(
            "read-model derivation canonical bytes are invalid: {error}"
        ))
    })?;
    if stored_canonical.content_digest().as_str() != derived_row_hash {
        return Ok(SummaryComparison::Mismatch);
    }
    let source_run_id_text: String = row
        .try_get("source_run_id")
        .map_err(|error| database_error("failed to decode derivation source run id", error))?;
    let source_run_id = parse_identity::<RunId>(&source_run_id_text)?;
    let source_from_seq = StreamSeq::new(i64_to_positive_u64(
        row.try_get("source_from_seq").map_err(|error| {
            database_error("failed to decode derivation source from seq", error)
        })?,
        "observation_derivations.source_from_seq",
    )?)?;
    let source_to_seq = StreamSeq::new(i64_to_positive_u64(
        row.try_get("source_to_seq")
            .map_err(|error| database_error("failed to decode derivation source to seq", error))?,
        "observation_derivations.source_to_seq",
    )?)?;
    let matches = row
        .try_get::<String, _>("source_kind")
        .map_err(|error| database_error("failed to decode derivation source kind", error))?
        == derivation.source_kind
        && source_run_id == derivation.source_run_id
        && source_from_seq == derivation.source_from_seq
        && source_to_seq == derivation.source_to_seq
        && row
            .try_get::<i32, _>("source_last_ordinal")
            .map_err(|error| {
                database_error("failed to decode derivation source last ordinal", error)
            })?
            == derivation.source_last_ordinal
        && row
            .try_get::<String, _>("source_high_append_xid")
            .map_err(|error| database_error("failed to decode derivation source xid", error))?
            == derivation.source_high_append_xid
        && row
            .try_get::<String, _>("source_high_commit_id")
            .map_err(|error| {
                database_error("failed to decode derivation source commit id", error)
            })?
            == derivation.source_high_commit_id
        && row
            .try_get::<Vec<u8>, _>("source_high_commit_sort_key")
            .map_err(|error| {
                database_error("failed to decode derivation source sort key", error)
            })?
            == derivation.source_high_commit_sort_key
        && row
            .try_get::<i64, _>("source_event_count")
            .map_err(|error| {
                database_error("failed to decode derivation source event count", error)
            })?
            == derivation.source_event_count
        && row
            .try_get::<String, _>("source_input_hash")
            .map_err(|error| {
                database_error("failed to decode derivation source input hash", error)
            })?
            == derivation.source_input_hash
        && derived_row_hash == derivation.derived_row_hash
        && derived_row_canonical_json == derivation.derived_row_canonical_json.as_bytes();
    Ok(if matches {
        SummaryComparison::Match
    } else {
        SummaryComparison::Mismatch
    })
}

pub(super) async fn insert_materialized_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    summary: &RunObservationSummaryMaterialization,
) -> Result<bool> {
    let rows = sqlx::query(
        "INSERT INTO run_observation_change_summaries \
         (projection_version, commit_id, summary_kind, run_id, head_seq, observed_status, \
          started_at, updated_at, completed_at, source_authority_hash, source_event_count, \
          summary_row_hash, summary_row_canonical_json) \
         SELECT $1, $2, $3, $4, $5, $6, \
           (SELECT MIN(committed_at) FROM commits WHERE run_id = $4), \
           c.committed_at, \
           CASE WHEN $6 = 'completed' THEN c.committed_at ELSE NULL END, \
           $7, $8, $9, $10 \
         FROM commits c WHERE c.commit_id = $2 \
         ON CONFLICT (projection_version, commit_id, summary_kind) DO NOTHING",
    )
    .bind(summary.projection_version.as_str())
    .bind(summary.commit_id.as_str())
    .bind(summary.summary_kind)
    .bind(summary.run_id.as_str())
    .bind(u64_to_i64(
        summary.head_seq.as_u64(),
        "run_observation_change_summaries.head_seq",
    )?)
    .bind(summary.observed_status.as_str())
    .bind(summary.source_authority_hash.as_str())
    .bind(summary.source_event_count)
    .bind(summary.summary_row_hash.as_str())
    .bind(summary.summary_row_canonical_json.as_bytes())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert rebuilt observation summary", error))?
    .rows_affected();
    let derivation_rows = insert_materialized_observation_derivation_tx(tx, &summary.derivation)
        .await?
        .rows_affected();
    if !matches!(
        compare_existing_observation_derivation_tx(tx, &summary.derivation).await?,
        SummaryComparison::Match
    ) {
        return Err(PostgresStoreError::Corruption(
            "observation derivation drift after rebuild".to_owned(),
        ));
    }
    Ok(rows == 1 || derivation_rows == 1)
}

pub(super) async fn insert_materialized_observation_derivation_tx(
    tx: &mut Transaction<'_, Postgres>,
    derivation: &ObservationDerivationMaterialization,
) -> Result<sqlx::postgres::PgQueryResult> {
    sqlx::query(
        "INSERT INTO observation_derivations \
         (projection_version, model_name, derived_key, source_kind, source_run_id, \
          source_from_seq, source_to_seq, source_last_ordinal, source_high_append_xid, \
          source_high_commit_id, source_high_commit_sort_key, source_event_count, \
          source_input_hash, derived_row_canonical_json, derived_row_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9::xid8,$10,$11,$12,$13,$14,$15) \
         ON CONFLICT (projection_version, model_name, derived_key) DO NOTHING",
    )
    .bind(derivation.projection_version.as_str())
    .bind(derivation.model_name)
    .bind(derivation.derived_key.as_str())
    .bind(derivation.source_kind)
    .bind(derivation.source_run_id.as_str())
    .bind(u64_to_i64(
        derivation.source_from_seq.as_u64(),
        "observation_derivations.source_from_seq",
    )?)
    .bind(u64_to_i64(
        derivation.source_to_seq.as_u64(),
        "observation_derivations.source_to_seq",
    )?)
    .bind(derivation.source_last_ordinal)
    .bind(derivation.source_high_append_xid.as_str())
    .bind(derivation.source_high_commit_id.as_str())
    .bind(derivation.source_high_commit_sort_key.as_slice())
    .bind(derivation.source_event_count)
    .bind(derivation.source_input_hash.as_str())
    .bind(derivation.derived_row_canonical_json.as_bytes())
    .bind(derivation.derived_row_hash.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert observation derivation", error))
}
