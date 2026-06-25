use super::*;

#[derive(Clone)]
pub(super) struct StoreMetadata {
    pub(super) store_epoch: String,
    pub(super) trust_scope_id: TrustScopeId,
}

#[derive(Debug, Clone)]
pub(super) struct CursorPosition {
    pub(super) append_xid: String,
    pub(super) commit_sort_key: Vec<u8>,
    pub(super) kind: CursorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorKind {
    Frontier,
    Row,
}

impl CursorKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Frontier => "frontier",
            Self::Row => "row",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "frontier" => Ok(Self::Frontier),
            "row" => Ok(Self::Row),
            _ => Err(StoreError::InvalidCursor {
                message: "unknown cursor kind".to_owned(),
            }
            .into()),
        }
    }
}

pub(super) struct ObservationRow {
    pub(super) observation: RunObservation,
    pub(super) append_xid: String,
    pub(super) commit_sort_key: Vec<u8>,
}

struct ObservationCandidate {
    run_id: RunId,
    head_seq: StreamSeq,
    commit_id: String,
    append_xid: String,
    commit_sort_key: Vec<u8>,
    started_at: String,
    updated_at: String,
}

pub(super) async fn read_run_observations_from_pool(
    pool: &PgPool,
    query: RunObservationQuery,
) -> Result<RunObservationPage> {
    if query.limit == 0 || query.limit > MAX_OBSERVATION_LIMIT {
        return Err(StoreError::LimitOutOfRange {
            limit: query.limit,
            max: MAX_OBSERVATION_LIMIT,
        }
        .into());
    }
    let metadata = load_store_metadata(pool).await?;
    let cursor = match query.cursor.as_deref() {
        Some(cursor) => Some(decode_observation_cursor(pool, cursor, &metadata).await?),
        None => None,
    };
    let mut listener = if cursor.is_some() && query.wait_ms > 0 {
        Some(observation_change_listener(pool).await?)
    } else {
        None
    };
    let mut page =
        read_run_observation_page_once(pool, &metadata, cursor.as_ref(), query.limit).await?;
    if page.runs.is_empty() && cursor.is_some() && query.wait_ms > 0 {
        let listener = listener.as_mut().ok_or_else(|| {
            PostgresStoreError::Corruption(
                "observation watch listener was not initialized".to_owned(),
            )
        })?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(query.wait_ms);
        while page.runs.is_empty() {
            let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now())
            else {
                break;
            };
            let wait = remaining.min(Duration::from_millis(OBSERVATION_WAIT_POLL_INTERVAL_MS));
            let notified = wait_for_observation_change(listener, wait).await?;
            page = read_run_observation_page_once(pool, &metadata, cursor.as_ref(), query.limit)
                .await?;
            if !notified && wait == remaining {
                break;
            }
        }
    }
    Ok(page)
}

pub(super) async fn observation_change_listener(pool: &PgPool) -> Result<PgListener> {
    let listener_pool = PgPoolOptions::new()
        .max_connections(1)
        .max_lifetime(None)
        .idle_timeout(None)
        .connect_with((*pool.connect_options()).clone())
        .await
        .map_err(|error| database_error("failed to open observation listener pool", error))?;
    let mut listener = PgListener::connect_with(&listener_pool)
        .await
        .map_err(|error| database_error("failed to open observation listener", error))?;
    listener
        .listen(OBSERVATION_NOTIFY_CHANNEL)
        .await
        .map_err(|error| database_error("failed to listen for observation changes", error))?;
    Ok(listener)
}

pub(super) async fn wait_for_observation_change(
    listener: &mut PgListener,
    wait: Duration,
) -> Result<bool> {
    match tokio::time::timeout(wait, listener.recv()).await {
        Ok(Ok(_notification)) => Ok(true),
        Ok(Err(error)) => Err(database_error(
            "failed to wait for observation change notification",
            error,
        )),
        Err(_elapsed) => Ok(false),
    }
}

pub(super) async fn read_run_observation_page_once(
    pool: &PgPool,
    metadata: &StoreMetadata,
    cursor: Option<&CursorPosition>,
    limit: u32,
) -> Result<RunObservationPage> {
    let frontier_xid = sealed_frontier_xid(pool).await?;
    let rows = match cursor {
        Some(cursor) => {
            read_observation_watch_rows(pool, metadata, cursor, &frontier_xid, limit).await?
        }
        None => read_observation_list_rows(pool, metadata, &frontier_xid, limit).await?,
    };
    let next_position = if cursor.is_some() && rows.len() == limit as usize {
        let last = rows
            .last()
            .ok_or_else(|| StoreError::ObservationUnavailable {
                message: "observation page limit was reached without a last row".to_owned(),
            })?;
        CursorPosition {
            append_xid: last.append_xid.clone(),
            commit_sort_key: last.commit_sort_key.clone(),
            kind: CursorKind::Row,
        }
    } else {
        CursorPosition {
            append_xid: frontier_xid,
            commit_sort_key: frontier_sort_key(),
            kind: CursorKind::Frontier,
        }
    };
    Ok(RunObservationPage {
        next_cursor: encode_observation_cursor(pool, metadata, &next_position).await?,
        runs: rows.into_iter().map(|row| row.observation).collect(),
    })
}

pub(super) async fn load_store_metadata(pool: &PgPool) -> Result<StoreMetadata> {
    let row = sqlx::query("SELECT store_epoch, trust_scope_id FROM store_metadata WHERE singleton")
        .fetch_one(pool)
        .await
        .map_err(|error| database_error("failed to load store metadata", error))?;
    Ok(StoreMetadata {
        store_epoch: row
            .try_get("store_epoch")
            .map_err(|error| database_error("failed to decode store epoch", error))?,
        trust_scope_id: TrustScopeId::new(
            row.try_get::<String, _>("trust_scope_id")
                .map_err(|error| database_error("failed to decode trust scope id", error))?,
        )?,
    })
}

pub(super) async fn load_trust_scope_id_client(pool: &PgPool) -> Result<TrustScopeId> {
    load_store_metadata(pool)
        .await
        .map(|metadata| metadata.trust_scope_id)
}

pub(super) async fn sealed_frontier_xid(pool: &PgPool) -> Result<String> {
    let row = sqlx::query("SELECT pg_snapshot_xmin(pg_current_snapshot())::text AS frontier_xid")
        .fetch_one(pool)
        .await
        .map_err(|error| database_error("failed to read observation frontier", error))?;
    row.try_get("frontier_xid")
        .map_err(|error| database_error("failed to decode observation frontier", error))
}

pub(super) async fn notify_observation_change_tx(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(OBSERVATION_NOTIFY_CHANNEL)
        .bind(OBSERVATION_NOTIFY_PAYLOAD)
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to notify observation change", error))?;
    Ok(())
}

pub(super) async fn read_observation_list_rows(
    pool: &PgPool,
    metadata: &StoreMetadata,
    frontier_xid: &str,
    limit: u32,
) -> Result<Vec<ObservationRow>> {
    let rows = sqlx::query(
        "WITH bounded AS ( \
           SELECT DISTINCT ON (c.run_id) \
             c.run_id, c.seq AS head_seq, \
             to_char((SELECT MIN(started.committed_at) FROM commits started \
               WHERE started.run_id = c.run_id) AT TIME ZONE 'UTC', \
               'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
             to_char(c.committed_at AT TIME ZONE 'UTC', \
               'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
             c.commit_id, c.append_xid::text AS append_xid, c.commit_sort_key \
           FROM commits c \
           WHERE c.append_xid < $1::xid8 \
           ORDER BY c.run_id, c.seq DESC \
         ) \
         SELECT * FROM bounded ORDER BY updated_at DESC, run_id ASC LIMIT $2",
    )
    .bind(frontier_xid)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("failed to read run observation list", error))?;
    materialize_observation_rows(pool, metadata, rows).await
}

pub(super) async fn read_observation_watch_rows(
    pool: &PgPool,
    metadata: &StoreMetadata,
    cursor: &CursorPosition,
    frontier_xid: &str,
    limit: u32,
) -> Result<Vec<ObservationRow>> {
    let rows = sqlx::query(
        "SELECT c.run_id, c.seq AS head_seq, \
          to_char((SELECT MIN(started.committed_at) FROM commits started \
            WHERE started.run_id = c.run_id) AT TIME ZONE 'UTC', \
            'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
          to_char(c.committed_at AT TIME ZONE 'UTC', \
            'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
          c.commit_id, c.append_xid::text AS append_xid, c.commit_sort_key \
         FROM commits c \
         WHERE c.append_xid < $1::xid8 \
           AND ( \
             ($4 AND c.append_xid >= $2::xid8) \
             OR \
             (NOT $4 AND (c.append_xid, c.commit_sort_key) > ($2::xid8, $3::bytea)) \
           ) \
         ORDER BY c.append_xid ASC, c.commit_sort_key ASC \
         LIMIT $5",
    )
    .bind(frontier_xid)
    .bind(&cursor.append_xid)
    .bind(cursor.commit_sort_key.as_slice())
    .bind(cursor.kind == CursorKind::Frontier)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("failed to read run observation changes", error))?;
    materialize_observation_rows(pool, metadata, rows).await
}

async fn materialize_observation_rows(
    pool: &PgPool,
    metadata: &StoreMetadata,
    rows: Vec<PgRow>,
) -> Result<Vec<ObservationRow>> {
    let mut observations = Vec::with_capacity(rows.len());
    for row in rows {
        observations
            .push(materialize_observation_row(pool, metadata, observation_candidate(row)?).await?);
    }
    Ok(observations)
}

fn observation_candidate(row: PgRow) -> Result<ObservationCandidate> {
    let run_id: String = row
        .try_get("run_id")
        .map_err(|error| database_error("failed to decode observation run id", error))?;
    let head_seq: i64 = row
        .try_get("head_seq")
        .map_err(|error| database_error("failed to decode observation head seq", error))?;
    let commit_id: String = row
        .try_get("commit_id")
        .map_err(|error| database_error("failed to decode observation commit id", error))?;
    let append_xid: String = row
        .try_get("append_xid")
        .map_err(|error| database_error("failed to decode observation append xid", error))?;
    let commit_sort_key = row
        .try_get("commit_sort_key")
        .map_err(|error| database_error("failed to decode observation commit sort key", error))?;
    let started_at = row
        .try_get("started_at")
        .map_err(|error| database_error("failed to decode observation started_at", error))?;
    let updated_at = row
        .try_get("updated_at")
        .map_err(|error| database_error("failed to decode observation updated_at", error))?;
    Ok(ObservationCandidate {
        run_id: parse_identity(&run_id)?,
        head_seq: StreamSeq::new(i64_to_positive_u64(head_seq, "commits.seq")?)?,
        commit_id,
        append_xid,
        commit_sort_key,
        started_at,
        updated_at,
    })
}

async fn materialize_observation_row(
    pool: &PgPool,
    metadata: &StoreMetadata,
    candidate: ObservationCandidate,
) -> Result<ObservationRow> {
    let events = load_run_prefix(pool, &candidate.run_id, candidate.head_seq).await?;
    let projections = ProjectionSnapshot::rebuild_from_run_stream(&events)?;
    let observed_status = observed_status_from_run_state(projections.run_state(&candidate.run_id))?;
    let completed_at =
        (observed_status == ObservedRunStatus::Completed).then_some(candidate.updated_at.clone());
    Ok(ObservationRow {
        observation: RunObservation {
            run_id: candidate.run_id,
            head_seq: candidate.head_seq,
            observed_status,
            started_at: candidate.started_at,
            updated_at: candidate.updated_at,
            completed_at,
            change_id: Some(observation_change_id(metadata, &candidate.commit_id)?),
        },
        append_xid: candidate.append_xid,
        commit_sort_key: candidate.commit_sort_key,
    })
}

async fn load_run_prefix(
    pool: &PgPool,
    run_id: &RunId,
    head_seq: StreamSeq,
) -> Result<Vec<KernelEventEnvelope>> {
    let rows = sqlx::query(
        "SELECT run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
         logical_key, payload_hash, payload_canonical_json \
         FROM run_events WHERE run_id = $1 AND seq <= $2 ORDER BY seq ASC, ordinal ASC",
    )
    .bind(run_id.as_str())
    .bind(u64_to_i64(head_seq.as_u64(), "commits.seq")?)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("failed to load run observation prefix", error))?;
    let events = rows
        .into_iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    if events.last().map(KernelEventEnvelope::seq) != Some(head_seq) {
        return Err(PostgresStoreError::Corruption(
            "observation commit has no matching event prefix".to_owned(),
        ));
    }
    Ok(events)
}

fn observed_status_from_run_state(run_state: RunState) -> Result<ObservedRunStatus> {
    match run_state {
        RunState::Started => Ok(ObservedRunStatus::Started),
        RunState::Completed => Ok(ObservedRunStatus::Completed),
        RunState::Absent => Err(StoreError::ObservationUnavailable {
            message: "committed run observation had absent run state".to_owned(),
        }
        .into()),
    }
}

pub(super) async fn encode_observation_cursor(
    pool: &PgPool,
    metadata: &StoreMetadata,
    position: &CursorPosition,
) -> Result<String> {
    let token = random_observation_cursor_token(pool).await?;
    let token_hash = observation_cursor_token_hash(&token);
    sqlx::query(
        "INSERT INTO run_observation_cursors \
          (token_hash, cursor_version, store_epoch, cursor_kind, append_xid, \
           commit_sort_key) \
         VALUES ($1, $2, $3, $4, $5::xid8, $6) \
         ON CONFLICT (token_hash) DO NOTHING",
    )
    .bind(&token_hash)
    .bind(CURSOR_VERSION)
    .bind(&metadata.store_epoch)
    .bind(position.kind.as_str())
    .bind(&position.append_xid)
    .bind(position.commit_sort_key.as_slice())
    .execute(pool)
    .await
    .map_err(|error| database_error("failed to persist observation cursor token", error))?;
    Ok(token)
}

pub(super) async fn decode_observation_cursor(
    pool: &PgPool,
    cursor: &str,
    metadata: &StoreMetadata,
) -> Result<CursorPosition> {
    let token = bytes_from_hex(cursor)?;
    if token.len() != 32 {
        return Err(StoreError::InvalidCursor {
            message: "malformed cursor".to_owned(),
        }
        .into());
    }
    let token_hash = observation_cursor_token_hash(cursor);
    let Some(row) = sqlx::query(
        "SELECT cursor_version, store_epoch, cursor_kind, \
          append_xid::text AS append_xid, commit_sort_key \
         FROM run_observation_cursors WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("failed to load observation cursor token", error))?
    else {
        return Err(StoreError::InvalidCursor {
            message: "unknown cursor".to_owned(),
        }
        .into());
    };
    let store_epoch: String = row
        .try_get("store_epoch")
        .map_err(|error| database_error("failed to decode observation cursor epoch", error))?;
    if store_epoch != metadata.store_epoch {
        return Err(StoreError::CursorExpired.into());
    }
    let cursor_version: String = row
        .try_get("cursor_version")
        .map_err(|error| database_error("failed to decode observation cursor version", error))?;
    if cursor_version != CURSOR_VERSION {
        return Err(StoreError::InvalidCursor {
            message: "stale cursor format".to_owned(),
        }
        .into());
    }
    let cursor_kind: String = row
        .try_get("cursor_kind")
        .map_err(|error| database_error("failed to decode observation cursor kind", error))?;
    let kind = CursorKind::parse(&cursor_kind)?;
    Ok(CursorPosition {
        append_xid: row
            .try_get("append_xid")
            .map_err(|error| database_error("failed to decode observation cursor xid", error))?,
        commit_sort_key: row.try_get("commit_sort_key").map_err(|error| {
            database_error("failed to decode observation cursor sort key", error)
        })?,
        kind,
    })
}

async fn random_observation_cursor_token(pool: &PgPool) -> Result<String> {
    sqlx::query_scalar("SELECT encode(public.gen_random_bytes(32), 'hex')")
        .fetch_one(pool)
        .await
        .map_err(|error| database_error("failed to issue observation cursor token", error))
}

pub(super) fn observation_change_id(metadata: &StoreMetadata, commit_id: &str) -> Result<String> {
    let canonical = canonical_json(serde_json::json!({
        "change_id_version": "mfm.run_observation.change_id.v1",
        "commit_id": commit_id,
        "store_epoch": metadata.store_epoch.as_str(),
    }))?;
    Ok(bytes_hex(
        sha256_digest_bytes(canonical.as_bytes()).as_bytes(),
    ))
}

pub(super) fn observation_cursor_token_hash(token: &str) -> String {
    bytes_hex(sha256_digest_bytes(token.as_bytes()).as_bytes())
}

pub(super) fn frontier_sort_key() -> Vec<u8> {
    vec![0; 32]
}
