use super::*;

pub(super) async fn load_run_stream_client(
    pool: &PgPool,
    run_id: &RunId,
) -> Result<Vec<KernelEventEnvelope>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start read transaction", error))?;
    let events = load_run_stream_tx(&mut tx, run_id).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read transaction", error))?;
    Ok(events)
}

pub(super) async fn load_run_stream_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<Vec<KernelEventEnvelope>> {
    validate_persisted_run_authority_tx(tx, run_id).await?;
    let rows = sqlx::query(
        "SELECT e.run_id, e.seq, e.ordinal, c.store_commit_order, e.event_id, e.event_schema_id, \
         e.spec_hash, e.commit_key, e.logical_key, e.payload_hash, e.payload_canonical_json \
         FROM run_events e JOIN commits c ON c.run_id = e.run_id AND c.seq = e.seq \
         WHERE e.run_id = $1 ORDER BY e.seq ASC, e.ordinal ASC",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load run stream", error))?;
    let events = rows
        .into_iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    ProjectionSnapshot::validate_run_stream(&events)?;
    Ok(events)
}

pub(super) fn event_envelope_from_row(row: PgRow) -> Result<KernelEventEnvelope> {
    let row = RunEventRow {
        run_id: row
            .try_get("run_id")
            .map_err(|error| database_error("failed to decode run event run_id", error))?,
        seq: row
            .try_get("seq")
            .map_err(|error| database_error("failed to decode run event seq", error))?,
        ordinal: row
            .try_get("ordinal")
            .map_err(|error| database_error("failed to decode run event ordinal", error))?,
        store_commit_order: row.try_get("store_commit_order").map_err(|error| {
            database_error("failed to decode run event store commit order", error)
        })?,
        event_id: row
            .try_get("event_id")
            .map_err(|error| database_error("failed to decode run event event_id", error))?,
        event_schema_id: row
            .try_get("event_schema_id")
            .map_err(|error| database_error("failed to decode run event event_schema_id", error))?,
        spec_hash: row
            .try_get("spec_hash")
            .map_err(|error| database_error("failed to decode run event spec_hash", error))?,
        commit_key: row
            .try_get("commit_key")
            .map_err(|error| database_error("failed to decode run event commit_key", error))?,
        logical_key: row
            .try_get("logical_key")
            .map_err(|error| database_error("failed to decode run event logical_key", error))?,
        payload_hash: row
            .try_get("payload_hash")
            .map_err(|error| database_error("failed to decode run event payload_hash", error))?,
        payload_canonical_json: row.try_get("payload_canonical_json").map_err(|error| {
            database_error("failed to decode run event payload_canonical_json", error)
        })?,
    };
    let payload_value: Value =
        serde_json::from_slice(&row.payload_canonical_json).map_err(|error| {
            PostgresStoreError::Corruption(format!(
                "persisted payload canonical bytes were not JSON: {error}"
            ))
        })?;
    let payload = payload_from_json_value(&payload_value)?;
    let canonical = payload_canonical_bytes(&payload)?;
    if canonical.as_bytes() != row.payload_canonical_json.as_slice() {
        return Err(PostgresStoreError::Corruption(
            "run event payload bytes are not canonical for decoded payload".to_owned(),
        ));
    }
    let ordinal = u32::try_from(row.ordinal).map_err(|_| {
        PostgresStoreError::Corruption("run_events.ordinal contained a negative integer".into())
    })?;

    Ok(KernelEventEnvelope::from_persisted_record(
        PersistedKernelEventRecord {
            event_id: parse_identity::<mfm_ids::EventId>(&row.event_id)?,
            event_schema_id: parse_identity::<SchemaId>(&row.event_schema_id)?,
            run_id: parse_identity::<RunId>(&row.run_id)?,
            seq: StreamSeq::new(i64_to_positive_u64(row.seq, "run_events.seq")?)?,
            store_commit_order: StoreCommitOrder::new(i64_to_positive_u64(
                row.store_commit_order,
                "commits.store_commit_order",
            )?),
            ordinal: CommitOrdinal::new(ordinal),
            spec_hash: parse_identity::<mfm_ids::SpecHash>(&row.spec_hash)?,
            commit_key: CommitKey::new(row.commit_key)?,
            logical_key: LogicalEventKey::new(row.logical_key)?,
            payload_hash: parse_identity::<ContentDigest>(&row.payload_hash)?,
            payload,
        },
    )?)
}

pub(super) struct RunEventRow {
    pub(super) run_id: String,
    pub(super) seq: i64,
    pub(super) ordinal: i32,
    pub(super) store_commit_order: i64,
    pub(super) event_id: String,
    pub(super) event_schema_id: String,
    pub(super) spec_hash: String,
    pub(super) commit_key: String,
    pub(super) logical_key: String,
    pub(super) payload_hash: String,
    pub(super) payload_canonical_json: Vec<u8>,
}

pub(super) async fn rebuild_projection_snapshot_with_head(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<(ProjectionSnapshot, u64)> {
    let stream = load_run_stream_tx(tx, run_id).await?;
    let head = stream.last().map(|event| event.seq().as_u64()).unwrap_or(0);
    let snapshot = projection_snapshot_from_physical_fact_tables_tx(tx, run_id, &stream).await?;
    Ok((snapshot, head))
}
