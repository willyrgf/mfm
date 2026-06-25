use super::*;

pub(super) struct WaitFifoWaiterRow {
    pub(super) admission_token: mfm_store::v1::AdmissionToken,
}

pub(super) async fn lock_admission_lane_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::AdmissionLaneKey,
) -> Result<()> {
    let key = mfm_store::v1::admission_advisory_lock_key(lane)?.as_i64();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to lock admission lane", error))?;
    Ok(())
}

pub(super) async fn expire_wait_fifo_waiters_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ResourceAdmissionLane,
) -> Result<()> {
    let lane_id = lane.id().to_vec();
    sqlx::query(
        "UPDATE admission_waiter \
         SET status = 'expired', updated_at = statement_timestamp() \
         WHERE class = $1 AND lane_id = $2 AND status = 'waiting' \
           AND lease_expires_at <= statement_timestamp()",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to expire admission waiters", error))?;
    Ok(())
}

pub(super) async fn head_wait_fifo_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ResourceAdmissionLane,
) -> Result<Option<WaitFifoWaiterRow>> {
    let lane_id = lane.id().to_vec();
    let row = sqlx::query(
        "SELECT token FROM admission_waiter \
         WHERE class = $1 AND lane_id = $2 AND status = 'waiting' \
         ORDER BY lane_ticket ASC LIMIT 1",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load head admission waiter", error))?;
    row.map(|row| {
        Ok(WaitFifoWaiterRow {
            admission_token: mfm_store::v1::AdmissionToken::new(
                row.try_get::<String, _>("token")
                    .map_err(|error| database_error("failed to decode admission token", error))?,
            )?,
        })
    })
    .transpose()
}

pub(super) async fn enqueue_or_refresh_wait_fifo_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ResourceAdmissionLane,
    token: &mfm_store::v1::AdmissionToken,
) -> Result<AdmissionWaiter> {
    if let Some(waiter) = refresh_wait_fifo_waiter_tx(tx, lane, token).await? {
        return Ok(waiter);
    }
    let lane_ticket = allocate_wait_fifo_ticket_tx(tx, lane).await?;
    let waiter_id = mfm_store::v1::admission_waiter_id(token)?;
    let lane_id = lane.id().to_vec();
    let row = sqlx::query(
        "INSERT INTO admission_waiter \
         (class, lane_id, lane_ticket, waiter_id, token, status, lease_expires_at) \
         VALUES ($1,$2,$3,$4,$5,'waiting', statement_timestamp() + make_interval(secs => $6)) \
         ON CONFLICT (class, lane_id, token) DO UPDATE \
         SET lane_ticket = EXCLUDED.lane_ticket, status = 'waiting', updated_at = statement_timestamp(), \
             lease_expires_at = statement_timestamp() + make_interval(secs => $6) \
         RETURNING waiter_id, lane_ticket, (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(u64_to_i64(lane_ticket, "admission_waiter.lane_ticket")?)
    .bind(waiter_id.as_str())
    .bind(token.as_str())
    .bind(WAIT_FIFO_ADMISSION_WAITER_LEASE_SECS)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to enqueue admission waiter", error))?;
    admission_waiter_from_row(row)
}

pub(super) async fn refresh_wait_fifo_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ResourceAdmissionLane,
    token: &mfm_store::v1::AdmissionToken,
) -> Result<Option<AdmissionWaiter>> {
    let lane_id = lane.id().to_vec();
    let row = sqlx::query(
        "UPDATE admission_waiter \
         SET updated_at = statement_timestamp(), \
             lease_expires_at = statement_timestamp() + make_interval(secs => $4) \
         WHERE class = $1 AND lane_id = $2 AND token = $3 AND status = 'waiting' \
         RETURNING waiter_id, lane_ticket, (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(token.as_str())
    .bind(WAIT_FIFO_ADMISSION_WAITER_LEASE_SECS)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to refresh admission waiter", error))?;
    row.map(admission_waiter_from_row).transpose()
}

pub(super) async fn allocate_wait_fifo_ticket_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ResourceAdmissionLane,
) -> Result<u64> {
    let lane_id = lane.id().to_vec();
    let lane_ticket: i64 = sqlx::query_scalar(
        "INSERT INTO admission_lane (class, lane_id, mode, next_ticket) \
         VALUES ($1, $2, $3, 2) \
         ON CONFLICT (class, lane_id) DO UPDATE \
         SET next_ticket = admission_lane.next_ticket + 1, updated_at = statement_timestamp() \
         RETURNING next_ticket - 1",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(lane.mode().as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to allocate admission waiter ticket", error))?;
    i64_to_positive_u64(lane_ticket, "admission_waiter.lane_ticket")
}

pub(super) fn admission_waiter_from_row(row: PgRow) -> Result<AdmissionWaiter> {
    Ok(AdmissionWaiter {
        waiter_id: mfm_store::v1::AdmissionWaiterId::new(
            row.try_get::<String, _>("waiter_id")
                .map_err(|error| database_error("failed to decode admission waiter id", error))?,
        )?,
        lane_ticket: i64_to_positive_u64(
            row.try_get("lane_ticket").map_err(|error| {
                database_error("failed to decode admission waiter ticket", error)
            })?,
            "admission_waiter.lane_ticket",
        )?,
        lease_expires_at_unix_ms: row
            .try_get("lease_expires_at_unix_ms")
            .map_err(|error| database_error("failed to decode admission waiter lease", error))?,
    })
}

pub(super) async fn mark_wait_fifo_waiter_admitted_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ResourceAdmissionLane,
    token: &mfm_store::v1::AdmissionToken,
) -> Result<()> {
    let lane_id = lane.id().to_vec();
    sqlx::query(
        "UPDATE admission_waiter \
         SET status = 'admitted', updated_at = statement_timestamp() \
         WHERE class = $1 AND lane_id = $2 AND token = $3 AND status = 'waiting'",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(token.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to mark admission waiter admitted", error))?;
    Ok(())
}
