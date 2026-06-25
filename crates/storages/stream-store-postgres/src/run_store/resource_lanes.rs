use super::*;

pub(super) async fn lock_resource_lanes_for_request_tx(
    tx: &mut Transaction<'_, Postgres>,
    request: &mfm_store::v1::CommitRequest,
    projections: &ProjectionSnapshot,
) -> Result<BTreeSet<Vec<u8>>> {
    let mut lane_ids = BTreeSet::<Vec<u8>>::new();
    for payload in request.payloads() {
        match payload {
            events::KernelEventPayload::ResourceLaneClaimIntent(intent) => {
                lane_ids.insert(
                    mfm_store::v1::ResourceAdmissionLane::from_resource_key_evidence(
                        &intent.resource_key,
                    )?
                    .id()
                    .to_vec(),
                );
            }
            events::KernelEventPayload::ResourceLaneReleaseIntent(intent) => {
                lane_ids.insert(resource_lane_id_for_release(
                    request.run_id(),
                    projections,
                    intent,
                )?);
            }
            _ => {}
        }
    }
    for lane_id in &lane_ids {
        lock_resource_lane_tx(tx, lane_id).await?;
    }
    Ok(lane_ids)
}

pub(super) async fn lock_resource_lane_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane_id: &[u8],
) -> Result<()> {
    const RESOURCE_LANE_LOCK_CLASS_ID: i32 = 0x4d46_5202;
    let object_id = advisory_object_id(lane_id);
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(RESOURCE_LANE_LOCK_CLASS_ID)
        .bind(object_id)
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to lock resource lane", error))?;
    Ok(())
}

pub(super) fn resource_lane_id_for_release(
    run_id: &RunId,
    projections: &ProjectionSnapshot,
    intent: &events::ResourceLaneReleaseIntent,
) -> Result<Vec<u8>> {
    let holder = mfm_store::v1::SideEffectLedgerRef::new(run_id.clone(), intent.ledger_key.clone());
    let Some((lane_key, active)) = projections
        .resource_lanes()
        .find(|(_, projection)| projection.holder == holder)
    else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", intent.ledger_key),
            message: "resource lane release references unknown active claim".to_owned(),
        }
        .into());
    };
    if active.claim_id != intent.claim_id
        || active.node_id != intent.node_id
        || active.attempt_id != intent.attempt_id
        || active.ledger_purpose != intent.ledger_purpose
        || active.invocation_epoch != intent.invocation_epoch
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
            message: "resource lane release intent does not match active claim".to_owned(),
        }
        .into());
    }
    Ok(
        mfm_store::v1::ResourceAdmissionLane::from_resource_lane_key(lane_key)?
            .id()
            .to_vec(),
    )
}

pub(super) struct ResourceLaneClaimAdmission {
    pub(super) lane: mfm_store::v1::ResourceAdmissionLane,
    pub(super) lane_id: Vec<u8>,
    pub(super) lane_key: ResourceLaneKey,
    pub(super) holder: mfm_store::v1::SideEffectLedgerRef,
    pub(super) run_id: RunId,
    pub(super) node_id: NodeId,
    pub(super) attempt_id: mfm_ids::AttemptId,
    pub(super) ledger_key: events::SideEffectLedgerKey,
    pub(super) invocation_epoch: u32,
    pub(super) admission_token: mfm_store::v1::AdmissionToken,
}

pub(super) struct ResourceLaneWaiterRow {
    pub(super) admission_token: mfm_store::v1::AdmissionToken,
}

pub(super) fn single_lane_claim_admission(
    request: &mfm_store::v1::CommitRequest,
) -> Result<Option<ResourceLaneClaimAdmission>> {
    let mut claims = request.payloads().iter().filter_map(|payload| {
        if let events::KernelEventPayload::ResourceLaneClaimIntent(intent) = payload {
            Some(intent)
        } else {
            None
        }
    });
    let Some(intent) = claims.next() else {
        return Ok(None);
    };
    if claims.next().is_some() {
        return Ok(None);
    }
    let lane =
        mfm_store::v1::ResourceAdmissionLane::from_resource_key_evidence(&intent.resource_key)?;
    let lane_id = lane.id().to_vec();
    let lane_key = ResourceLaneKey::from_evidence(&intent.resource_key);
    let holder = mfm_store::v1::SideEffectLedgerRef::new(
        request.run_id().clone(),
        intent.ledger_key.clone(),
    );
    let admission_token =
        mfm_store::v1::resource_wait_fifo_admission_token(request.run_id(), &lane, intent)?;
    Ok(Some(ResourceLaneClaimAdmission {
        lane,
        lane_id,
        lane_key,
        holder,
        run_id: request.run_id().clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: intent.ledger_key.clone(),
        invocation_epoch: intent.invocation_epoch,
        admission_token,
    }))
}

pub(super) async fn resource_lane_fifo_pre_gate_tx(
    tx: &mut Transaction<'_, Postgres>,
    projections: &ProjectionSnapshot,
    admission: &ResourceLaneClaimAdmission,
) -> Result<Option<mfm_store::v1::WaitFifoAdmissionBlock>> {
    if let Some(active) = projections.resource_lane(&admission.lane_key) {
        if active.holder != admission.holder {
            let waiter = enqueue_or_refresh_waiter_tx(tx, admission).await?;
            return Ok(Some(mfm_store::v1::WaitFifoAdmissionBlock {
                lane: admission.lane.clone(),
                resource_lane_key: admission.lane_key.clone(),
                holder: Some(active.holder.clone()),
                waiter: Some(waiter),
            }));
        }
    }

    if let Some(head) = oldest_live_waiter_tx(tx, &admission.lane_id).await? {
        if head.admission_token != admission.admission_token {
            let waiter = enqueue_or_refresh_waiter_tx(tx, admission).await?;
            return Ok(Some(mfm_store::v1::WaitFifoAdmissionBlock {
                lane: admission.lane.clone(),
                resource_lane_key: admission.lane_key.clone(),
                holder: None,
                waiter: Some(waiter),
            }));
        }
    }
    Ok(None)
}

pub(super) async fn expire_stale_waiters_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane_id: &[u8],
) -> Result<()> {
    sqlx::query(
        "UPDATE resource_lane_waiters \
         SET status = 'expired', updated_at = statement_timestamp() \
         WHERE lane_id = $1 AND status = 'waiting' AND lease_expires_at <= statement_timestamp()",
    )
    .bind(lane_id)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to expire resource lane waiters", error))?;
    Ok(())
}

pub(super) async fn oldest_live_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane_id: &[u8],
) -> Result<Option<ResourceLaneWaiterRow>> {
    let row = sqlx::query(
        "SELECT claim_fingerprint FROM resource_lane_waiters \
         WHERE lane_id = $1 AND status = 'waiting' \
         ORDER BY lane_ticket ASC LIMIT 1",
    )
    .bind(lane_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load oldest resource lane waiter", error))?;
    row.map(|row| {
        Ok(ResourceLaneWaiterRow {
            admission_token: mfm_store::v1::AdmissionToken::new(
                row.try_get::<String, _>("claim_fingerprint")
                    .map_err(|error| {
                        database_error("failed to decode resource lane waiter fingerprint", error)
                    })?,
            )?,
        })
    })
    .transpose()
}

pub(super) async fn enqueue_or_refresh_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    admission: &ResourceLaneClaimAdmission,
) -> Result<AdmissionWaiter> {
    if let Some(waiter) = refresh_waiting_waiter_tx(tx, admission).await? {
        return Ok(waiter);
    }
    let lane_ticket = allocate_waiter_ticket_tx(tx, &admission.lane_id).await?;
    let waiter_id = mfm_store::v1::admission_waiter_id(&admission.admission_token)?;
    let row = sqlx::query(
        "INSERT INTO resource_lane_waiters \
         (waiter_id, lane_id, lane_ticket, run_id, node_id, attempt_id, ledger_key, \
          invocation_epoch, claim_fingerprint, status, lease_expires_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,'waiting', statement_timestamp() + make_interval(secs => $10)) \
         ON CONFLICT (lane_id, claim_fingerprint) DO UPDATE \
         SET lane_ticket = EXCLUDED.lane_ticket, status = 'waiting', updated_at = statement_timestamp(), \
             lease_expires_at = statement_timestamp() + make_interval(secs => $10) \
         RETURNING waiter_id, lane_ticket, (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms",
    )
    .bind(waiter_id.as_str())
    .bind(&admission.lane_id)
    .bind(u64_to_i64(lane_ticket, "resource_lane_waiters.lane_ticket")?)
    .bind(admission.run_id.as_str())
    .bind(admission.node_id.as_str())
    .bind(admission.attempt_id.as_str())
    .bind(admission.ledger_key.as_str())
    .bind(i32::try_from(admission.invocation_epoch).map_err(|_| {
        PostgresStoreError::Corruption("resource lane waiter invocation epoch overflow".to_owned())
    })?)
    .bind(admission.admission_token.as_str())
    .bind(RESOURCE_LANE_WAITER_LEASE_SECS)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to enqueue resource lane waiter", error))?;
    waiter_block_from_row(row)
}

pub(super) async fn refresh_waiting_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    admission: &ResourceLaneClaimAdmission,
) -> Result<Option<AdmissionWaiter>> {
    let row = sqlx::query(
        "UPDATE resource_lane_waiters \
         SET updated_at = statement_timestamp(), \
             lease_expires_at = statement_timestamp() + make_interval(secs => $3) \
         WHERE lane_id = $1 AND claim_fingerprint = $2 AND status = 'waiting' \
         RETURNING waiter_id, lane_ticket, (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms",
    )
    .bind(&admission.lane_id)
    .bind(admission.admission_token.as_str())
    .bind(RESOURCE_LANE_WAITER_LEASE_SECS)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to refresh resource lane waiter", error))?;
    row.map(waiter_block_from_row).transpose()
}

pub(super) async fn allocate_waiter_ticket_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane_id: &[u8],
) -> Result<u64> {
    let lane_ticket: i64 = sqlx::query_scalar(
        "INSERT INTO resource_lane_waiter_counters (lane_id, next_ticket) \
         VALUES ($1, 2) \
         ON CONFLICT (lane_id) DO UPDATE \
         SET next_ticket = resource_lane_waiter_counters.next_ticket + 1, \
             updated_at = statement_timestamp() \
         RETURNING next_ticket - 1",
    )
    .bind(lane_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to allocate resource lane waiter ticket", error))?;
    i64_to_positive_u64(lane_ticket, "resource_lane_waiters.lane_ticket")
}

pub(super) fn waiter_block_from_row(row: PgRow) -> Result<AdmissionWaiter> {
    Ok(AdmissionWaiter {
        waiter_id: mfm_store::v1::AdmissionWaiterId::new(
            row.try_get::<String, _>("waiter_id").map_err(|error| {
                database_error("failed to decode resource lane waiter id", error)
            })?,
        )?,
        lane_ticket: i64_to_positive_u64(
            row.try_get("lane_ticket").map_err(|error| {
                database_error("failed to decode resource lane waiter ticket", error)
            })?,
            "resource_lane_waiters.lane_ticket",
        )?,
        lease_expires_at_unix_ms: row.try_get("lease_expires_at_unix_ms").map_err(|error| {
            database_error("failed to decode resource lane waiter lease", error)
        })?,
    })
}

pub(super) async fn mark_waiter_claimed_tx(
    tx: &mut Transaction<'_, Postgres>,
    admission: &ResourceLaneClaimAdmission,
) -> Result<()> {
    sqlx::query(
        "UPDATE resource_lane_waiters \
         SET status = 'claimed', updated_at = statement_timestamp() \
         WHERE lane_id = $1 AND claim_fingerprint = $2 AND status = 'waiting'",
    )
    .bind(&admission.lane_id)
    .bind(admission.admission_token.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to mark resource lane waiter claimed", error))?;
    Ok(())
}

pub(super) fn bytes_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(super) fn bytes_from_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(StoreError::InvalidCursor {
            message: "hex field has odd length".to_owned(),
        }
        .into());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let high = hex_nibble(chunk[0])?;
        let low = hex_nibble(chunk[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

pub(super) fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(StoreError::InvalidCursor {
            message: "hex field contains a non-hex byte".to_owned(),
        }
        .into()),
    }
}
