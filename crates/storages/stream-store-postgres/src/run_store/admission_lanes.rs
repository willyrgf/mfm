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

pub(super) async fn acquire_execution_claim_client(
    pool: &PgPool,
    run_id: &RunId,
    token: AdmissionToken,
) -> Result<mfm_store::v1::NowaitSkipAdmissionResult> {
    let lane = mfm_store::v1::ExecutionClaimAdmissionLane::from_run_id(run_id)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to begin execution claim transaction", error))?;
    lock_admission_lane_tx(&mut tx, &lane.erased_key()).await?;

    if let Some(lease) = admit_execution_claim_holder_tx(&mut tx, run_id, &lane, &token).await? {
        tx.commit()
            .await
            .map_err(|error| database_error("failed to commit execution claim acquire", error))?;
        return Ok(mfm_store::v1::NowaitSkipAdmissionResult::Admitted(lease));
    }
    let holder = read_execution_claim_holder_tx(&mut tx, &lane).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit execution claim busy read", error))?;
    Ok(mfm_store::v1::NowaitSkipAdmissionResult::Busy(
        mfm_store::v1::NowaitSkipAdmissionBusy { lane, holder },
    ))
}

pub(super) async fn execution_claim_status_client(
    pool: &PgPool,
    run_id: &RunId,
) -> Result<mfm_store::v1::ExecutionClaimStatus> {
    let lane = mfm_store::v1::ExecutionClaimAdmissionLane::from_run_id(run_id)?;
    let lane_id = lane.id().to_vec();
    let row = sqlx::query(
        "SELECT holder_token, \
            (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms, \
            (lease_expires_at <= statement_timestamp()) AS lease_expired \
         FROM admission_lane WHERE class = $1 AND lane_id = $2",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("failed to read execution claim status", error))?;
    row.map(|row| execution_claim_status_from_row(&lane, row))
        .transpose()
        .map(|status| status.unwrap_or(mfm_store::v1::ExecutionClaimStatus::Unclaimed))
}

pub(super) async fn renew_execution_claim_client(
    pool: &PgPool,
    run_id: &RunId,
    token: &AdmissionToken,
) -> Result<Option<AdmissionLease>> {
    let lane = mfm_store::v1::ExecutionClaimAdmissionLane::from_run_id(run_id)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to begin execution claim transaction", error))?;
    lock_admission_lane_tx(&mut tx, &lane.erased_key()).await?;
    let lease = update_execution_claim_lease_tx(&mut tx, &lane, token).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit execution claim renew", error))?;
    Ok(lease)
}

pub(super) async fn release_execution_claim_client(
    pool: &PgPool,
    run_id: &RunId,
    token: &AdmissionToken,
) -> Result<bool> {
    let lane = mfm_store::v1::ExecutionClaimAdmissionLane::from_run_id(run_id)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to begin execution claim transaction", error))?;
    lock_admission_lane_tx(&mut tx, &lane.erased_key()).await?;
    let released = release_execution_claim_holder_tx(&mut tx, &lane, token).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit execution claim release", error))?;
    Ok(released)
}

pub(super) async fn expired_execution_claims_client(
    pool: &PgPool,
) -> Result<Vec<mfm_store::v1::ExpiredExecutionClaim>> {
    let rows = sqlx::query(
        "SELECT execution_run_id, holder_token, \
            (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms \
         FROM admission_lane \
         WHERE class = 'execution_claim' AND holder_token IS NOT NULL \
           AND lease_expires_at <= statement_timestamp() \
         ORDER BY lease_expires_at ASC, execution_run_id ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("failed to read expired execution claims", error))?;
    rows.into_iter()
        .map(expired_execution_claim_from_row)
        .collect()
}

pub(super) async fn reap_expired_execution_claim_client(
    pool: &PgPool,
    run_id: &RunId,
    token: &AdmissionToken,
) -> Result<bool> {
    let lane = mfm_store::v1::ExecutionClaimAdmissionLane::from_run_id(run_id)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to begin execution claim transaction", error))?;
    lock_admission_lane_tx(&mut tx, &lane.erased_key()).await?;
    let reaped = reap_execution_claim_holder_tx(&mut tx, &lane, token).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit execution claim reap", error))?;
    Ok(reaped)
}

async fn admit_execution_claim_holder_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
    token: &AdmissionToken,
) -> Result<Option<AdmissionLease>> {
    let lane_id = lane.id().to_vec();
    let row = sqlx::query(
        "INSERT INTO admission_lane \
         (class, lane_id, mode, holder_token, lease_expires_at, execution_run_id) \
         VALUES ($1,$2,$3,$4, statement_timestamp() + make_interval(secs => $5), $6) \
         ON CONFLICT (class, lane_id) DO UPDATE \
         SET holder_token = EXCLUDED.holder_token, \
             lease_expires_at = EXCLUDED.lease_expires_at, \
             execution_run_id = EXCLUDED.execution_run_id, \
             updated_at = statement_timestamp() \
         WHERE admission_lane.holder_token IS NULL \
         RETURNING holder_token, (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(lane.mode().as_str())
    .bind(token.as_str())
    .bind(EXECUTION_CLAIM_LEASE_SECS)
    .bind(run_id.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to admit execution claim", error))?;
    row.map(|row| execution_claim_lease_from_row(lane, row))
        .transpose()
}

async fn read_execution_claim_holder_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
) -> Result<Option<AdmissionLease>> {
    let lane_id = lane.id().to_vec();
    let row = sqlx::query(
        "SELECT holder_token, \
            (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms \
         FROM admission_lane WHERE class = $1 AND lane_id = $2",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to read execution claim holder", error))?;
    row.map(|row| optional_execution_claim_lease_from_row(lane, row))
        .transpose()
        .map(Option::flatten)
}

async fn update_execution_claim_lease_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
    token: &AdmissionToken,
) -> Result<Option<AdmissionLease>> {
    let lane_id = lane.id().to_vec();
    let row = sqlx::query(
        "UPDATE admission_lane \
         SET lease_expires_at = statement_timestamp() + make_interval(secs => $4), \
             updated_at = statement_timestamp() \
         WHERE class = $1 AND lane_id = $2 AND holder_token = $3 \
         RETURNING holder_token, (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(token.as_str())
    .bind(EXECUTION_CLAIM_LEASE_SECS)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to renew execution claim", error))?;
    row.map(|row| execution_claim_lease_from_row(lane, row))
        .transpose()
}

async fn release_execution_claim_holder_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
    token: &AdmissionToken,
) -> Result<bool> {
    let lane_id = lane.id().to_vec();
    let result = sqlx::query(
        "UPDATE admission_lane \
         SET holder_token = NULL, lease_expires_at = NULL, updated_at = statement_timestamp() \
         WHERE class = $1 AND lane_id = $2 AND holder_token = $3",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(token.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to release execution claim", error))?;
    Ok(result.rows_affected() == 1)
}

async fn reap_execution_claim_holder_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
    token: &AdmissionToken,
) -> Result<bool> {
    let lane_id = lane.id().to_vec();
    let result = sqlx::query(
        "UPDATE admission_lane \
         SET holder_token = NULL, lease_expires_at = NULL, updated_at = statement_timestamp() \
         WHERE class = $1 AND lane_id = $2 AND holder_token = $3 \
           AND lease_expires_at <= statement_timestamp()",
    )
    .bind(lane.class().as_str())
    .bind(&lane_id)
    .bind(token.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to reap execution claim", error))?;
    Ok(result.rows_affected() == 1)
}

fn execution_claim_lease_from_row(
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
    row: PgRow,
) -> Result<AdmissionLease> {
    let token = row
        .try_get::<String, _>("holder_token")
        .map_err(|error| database_error("failed to decode execution claim token", error))?;
    Ok(AdmissionLease {
        lane: lane.erased_key(),
        token: AdmissionToken::new(token)?,
        lease_expires_at_unix_ms: row
            .try_get("lease_expires_at_unix_ms")
            .map_err(|error| database_error("failed to decode execution claim lease", error))?,
    })
}

fn optional_execution_claim_lease_from_row(
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
    row: PgRow,
) -> Result<Option<AdmissionLease>> {
    let Some(token) = row
        .try_get::<Option<String>, _>("holder_token")
        .map_err(|error| database_error("failed to decode execution claim token", error))?
    else {
        return Ok(None);
    };
    Ok(Some(AdmissionLease {
        lane: lane.erased_key(),
        token: AdmissionToken::new(token)?,
        lease_expires_at_unix_ms: row
            .try_get("lease_expires_at_unix_ms")
            .map_err(|error| database_error("failed to decode execution claim lease", error))?,
    }))
}

fn execution_claim_status_from_row(
    lane: &mfm_store::v1::ExecutionClaimAdmissionLane,
    row: PgRow,
) -> Result<mfm_store::v1::ExecutionClaimStatus> {
    let Some(token) = row
        .try_get::<Option<String>, _>("holder_token")
        .map_err(|error| database_error("failed to decode execution claim token", error))?
    else {
        return Ok(mfm_store::v1::ExecutionClaimStatus::Unclaimed);
    };
    let lease = AdmissionLease {
        lane: lane.erased_key(),
        token: AdmissionToken::new(token)?,
        lease_expires_at_unix_ms: row
            .try_get("lease_expires_at_unix_ms")
            .map_err(|error| database_error("failed to decode execution claim lease", error))?,
    };
    let expired = row
        .try_get::<Option<bool>, _>("lease_expired")
        .map_err(|error| database_error("failed to decode execution claim lease status", error))?
        .unwrap_or(false);
    if expired {
        Ok(mfm_store::v1::ExecutionClaimStatus::Expired(lease))
    } else {
        Ok(mfm_store::v1::ExecutionClaimStatus::Live(lease))
    }
}

fn expired_execution_claim_from_row(row: PgRow) -> Result<mfm_store::v1::ExpiredExecutionClaim> {
    let run_id =
        parse_identity::<RunId>(&row.try_get::<String, _>("execution_run_id").map_err(
            |error| database_error("failed to decode expired execution claim run", error),
        )?)?;
    let lane = mfm_store::v1::ExecutionClaimAdmissionLane::from_run_id(&run_id)?;
    let token = row
        .try_get::<String, _>("holder_token")
        .map_err(|error| database_error("failed to decode expired execution claim token", error))?;
    let lease = AdmissionLease {
        lane: lane.erased_key(),
        token: AdmissionToken::new(token)?,
        lease_expires_at_unix_ms: row.try_get("lease_expires_at_unix_ms").map_err(|error| {
            database_error("failed to decode expired execution claim lease", error)
        })?,
    };
    Ok(mfm_store::v1::ExpiredExecutionClaim {
        run_id,
        lane,
        lease,
    })
}
