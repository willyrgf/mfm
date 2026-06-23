use super::*;

pub(super) async fn lock_resource_lanes_for_request_tx(
    tx: &mut Transaction<'_, Postgres>,
    request: &mfm_store::v1::CommitRequest,
) -> Result<BTreeSet<Vec<u8>>> {
    let mut lane_ids = BTreeSet::<Vec<u8>>::new();
    for payload in request.payloads() {
        match payload {
            events::KernelEventPayload::ResourceLaneClaimIntent(intent) => {
                lane_ids.insert(resource_lane_id(&intent.resource_key)?.to_vec());
            }
            events::KernelEventPayload::ResourceLaneReleaseIntent(intent) => {
                lane_ids.insert(load_claim_lane_id_tx(tx, &intent.claim_id).await?);
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

pub(super) fn resource_key_canonical_json(
    evidence: &events::ResourceKeyEvidence,
) -> Result<Vec<u8>> {
    Ok(canonical_json(serde_json::json!({
        "key": evidence.key.as_str(),
    }))?
    .to_vec())
}

pub(super) fn resource_lane_id(evidence: &events::ResourceKeyEvidence) -> Result<[u8; 32]> {
    let key_canonical_json = resource_key_canonical_json(evidence)?;
    let canonical = canonical_json(serde_json::json!({
        "key_canonical_json": String::from_utf8(key_canonical_json).map_err(|error| {
            PostgresStoreError::Corruption(format!("resource key canonical JSON was not UTF-8: {error}"))
        })?,
        "key_schema_id": evidence.key_schema_id.as_str(),
        "mode": "exclusive",
        "namespace": evidence.namespace.as_str(),
    }))?;
    let mut input = Vec::with_capacity(RESOURCE_LANE_ID_DOMAIN.len() + canonical.as_bytes().len());
    input.extend_from_slice(RESOURCE_LANE_ID_DOMAIN);
    input.extend_from_slice(canonical.as_bytes());
    let digest = sha256_digest_bytes(&input);
    let mut lane_id = [0_u8; 32];
    lane_id[0] = 1;
    lane_id[1..].copy_from_slice(&digest.as_bytes()[..31]);
    Ok(lane_id)
}

pub(super) struct ResourceLaneClaimAdmission {
    pub(super) lane_id: Vec<u8>,
    pub(super) lane_key: ResourceLaneKey,
    pub(super) holder: mfm_store::v1::SideEffectLedgerRef,
    pub(super) run_id: RunId,
    pub(super) node_id: NodeId,
    pub(super) attempt_id: mfm_ids::AttemptId,
    pub(super) ledger_key: events::SideEffectLedgerKey,
    pub(super) invocation_epoch: u32,
    pub(super) claim_fingerprint: String,
}

pub(super) struct ResourceLaneWaiterRow {
    pub(super) claim_fingerprint: String,
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
    let lane_id = resource_lane_id(&intent.resource_key)?.to_vec();
    let lane_key = ResourceLaneKey::from_evidence(&intent.resource_key);
    let holder = mfm_store::v1::SideEffectLedgerRef::new(
        request.run_id().clone(),
        intent.ledger_key.clone(),
    );
    let claim_fingerprint = resource_lane_claim_fingerprint(request.run_id(), &lane_id, intent)?;
    Ok(Some(ResourceLaneClaimAdmission {
        lane_id,
        lane_key,
        holder,
        run_id: request.run_id().clone(),
        node_id: intent.node_id.clone(),
        attempt_id: intent.attempt_id.clone(),
        ledger_key: intent.ledger_key.clone(),
        invocation_epoch: intent.invocation_epoch,
        claim_fingerprint,
    }))
}

pub(super) fn resource_lane_claim_fingerprint(
    run_id: &RunId,
    lane_id: &[u8],
    intent: &events::ResourceLaneClaimIntent,
) -> Result<String> {
    let canonical = canonical_json(serde_json::json!({
        "attempt_id": intent.attempt_id.as_str(),
        "domain": String::from_utf8_lossy(RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN),
        "invocation_epoch": intent.invocation_epoch,
        "lane_id": bytes_hex(lane_id),
        "ledger_key": intent.ledger_key.as_str(),
        "node_id": intent.node_id.as_str(),
        "requirement_digest": intent.requirement_digest.as_str(),
        "resolved_by_capability_impl": intent.resolved_by_capability_impl.as_str(),
        "run_id": run_id.as_str(),
    }))?;
    let mut input = Vec::with_capacity(
        RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN.len() + canonical.as_bytes().len(),
    );
    input.extend_from_slice(RESOURCE_LANE_WAITER_FINGERPRINT_DOMAIN);
    input.extend_from_slice(canonical.as_bytes());
    Ok(bytes_hex(sha256_digest_bytes(&input).as_bytes()))
}

pub(super) fn resource_lane_waiter_id(claim_fingerprint: &str) -> String {
    let mut input =
        Vec::with_capacity(RESOURCE_LANE_WAITER_ID_DOMAIN.len() + claim_fingerprint.len());
    input.extend_from_slice(RESOURCE_LANE_WAITER_ID_DOMAIN);
    input.extend_from_slice(claim_fingerprint.as_bytes());
    format!(
        "resource_lane_waiter:{}",
        bytes_hex(sha256_digest_bytes(&input).as_bytes())
    )
}

pub(super) async fn resource_lane_fifo_pre_gate_tx(
    tx: &mut Transaction<'_, Postgres>,
    projections: &ProjectionSnapshot,
    admission: &ResourceLaneClaimAdmission,
) -> Result<Option<mfm_store::v1::ResourceLaneClaimBlock>> {
    if let Some(active) = projections.resource_lane(&admission.lane_key) {
        if active.holder != admission.holder {
            let waiter = enqueue_or_refresh_waiter_tx(tx, admission).await?;
            return Ok(Some(mfm_store::v1::ResourceLaneClaimBlock {
                lane_key: admission.lane_key.clone(),
                holder: Some(active.holder.clone()),
                waiter: Some(waiter),
            }));
        }
    }

    if let Some(head) = oldest_live_waiter_tx(tx, &admission.lane_id).await? {
        if head.claim_fingerprint != admission.claim_fingerprint {
            let waiter = enqueue_or_refresh_waiter_tx(tx, admission).await?;
            return Ok(Some(mfm_store::v1::ResourceLaneClaimBlock {
                lane_key: admission.lane_key.clone(),
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
            claim_fingerprint: row.try_get("claim_fingerprint").map_err(|error| {
                database_error("failed to decode resource lane waiter fingerprint", error)
            })?,
        })
    })
    .transpose()
}

pub(super) async fn enqueue_or_refresh_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    admission: &ResourceLaneClaimAdmission,
) -> Result<ResourceLaneWaiterBlock> {
    if let Some(waiter) = refresh_waiting_waiter_tx(tx, admission).await? {
        return Ok(waiter);
    }
    let lane_ticket = allocate_waiter_ticket_tx(tx, &admission.lane_id).await?;
    let waiter_id = resource_lane_waiter_id(&admission.claim_fingerprint);
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
    .bind(&waiter_id)
    .bind(&admission.lane_id)
    .bind(u64_to_i64(lane_ticket, "resource_lane_waiters.lane_ticket")?)
    .bind(admission.run_id.as_str())
    .bind(admission.node_id.as_str())
    .bind(admission.attempt_id.as_str())
    .bind(admission.ledger_key.as_str())
    .bind(i32::try_from(admission.invocation_epoch).map_err(|_| {
        PostgresStoreError::Corruption("resource lane waiter invocation epoch overflow".to_owned())
    })?)
    .bind(&admission.claim_fingerprint)
    .bind(RESOURCE_LANE_WAITER_LEASE_SECS)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to enqueue resource lane waiter", error))?;
    waiter_block_from_row(row)
}

pub(super) async fn refresh_waiting_waiter_tx(
    tx: &mut Transaction<'_, Postgres>,
    admission: &ResourceLaneClaimAdmission,
) -> Result<Option<ResourceLaneWaiterBlock>> {
    let row = sqlx::query(
        "UPDATE resource_lane_waiters \
         SET updated_at = statement_timestamp(), \
             lease_expires_at = statement_timestamp() + make_interval(secs => $3) \
         WHERE lane_id = $1 AND claim_fingerprint = $2 AND status = 'waiting' \
         RETURNING waiter_id, lane_ticket, (EXTRACT(EPOCH FROM lease_expires_at) * 1000)::BIGINT AS lease_expires_at_unix_ms",
    )
    .bind(&admission.lane_id)
    .bind(&admission.claim_fingerprint)
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

pub(super) fn waiter_block_from_row(row: PgRow) -> Result<ResourceLaneWaiterBlock> {
    Ok(ResourceLaneWaiterBlock {
        waiter_id: row
            .try_get("waiter_id")
            .map_err(|error| database_error("failed to decode resource lane waiter id", error))?,
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
    .bind(&admission.claim_fingerprint)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to mark resource lane waiter claimed", error))?;
    Ok(())
}

pub(super) async fn load_claim_lane_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    claim_id: &events::ResourceLaneClaimId,
) -> Result<Vec<u8>> {
    let row = sqlx::query("SELECT lane_id FROM resource_lane_claim_events WHERE claim_id = $1")
        .bind(claim_id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load resource lane claim", error))?;
    let Some(row) = row else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane_claim:{}", claim_id),
            message: "resource lane release references unknown claim".to_owned(),
        }
        .into());
    };
    row.try_get("lane_id")
        .map_err(|error| database_error("failed to decode resource lane id", error))
}

pub(super) async fn insert_resource_lane_authority_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit_id: &str,
    events: &[KernelEventEnvelope],
) -> Result<()> {
    for event in events {
        match event.payload() {
            events::KernelEventPayload::ResourceLaneClaimed(payload) => {
                insert_resource_lane_claim_tx(tx, commit_id, event, payload).await?;
            }
            events::KernelEventPayload::ResourceLaneReleased(payload) => {
                insert_resource_lane_release_tx(tx, commit_id, event, payload).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) async fn insert_resource_lane_claim_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit_id: &str,
    event: &KernelEventEnvelope,
    payload: &events::ResourceLaneClaimed,
) -> Result<()> {
    let lane_id = resource_lane_id(&payload.resource_key)?;
    let key_canonical_json = resource_key_canonical_json(&payload.resource_key)?;
    sqlx::query(
        "INSERT INTO resource_lane_claim_events \
         (claim_id, lane_id, run_id, node_id, attempt_id, ledger_key, ledger_purpose, \
          invocation_epoch, namespace, key_schema_id, key_canonical_json, key_value, mode, \
          requirement_digest, resolved_by_capability_impl, fencing_token, commit_id, source_seq, \
          source_ordinal, source_event_id, source_event_type, source_event_payload_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'exclusive',$13,$14,$15,$16,$17,$18,$19,'ResourceLaneClaimed',$20)",
    )
    .bind(payload.claim_id.as_str())
    .bind(lane_id.as_slice())
    .bind(event.run_id().as_str())
    .bind(payload.node_id.as_str())
    .bind(payload.attempt_id.as_str())
    .bind(payload.ledger_key.as_str())
    .bind(side_effect_ledger_purpose_json(&payload.ledger_purpose))
    .bind(i32::try_from(payload.invocation_epoch).map_err(|_| {
        PostgresStoreError::Corruption("resource lane invocation epoch overflow".to_owned())
    })?)
    .bind(payload.resource_key.namespace.as_str())
    .bind(payload.resource_key.key_schema_id.as_str())
    .bind(key_canonical_json)
    .bind(payload.resource_key.key.as_str())
    .bind(payload.requirement_digest.as_str())
    .bind(payload.resolved_by_capability_impl.as_str())
    .bind(u64_to_i64(payload.claim_fencing_token, "resource_lane_claim_events.fencing_token")?)
    .bind(commit_id)
    .bind(u64_to_i64(event.seq().as_u64(), "resource_lane_claim_events.source_seq")?)
    .bind(i32::try_from(event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption("resource lane source ordinal overflow".to_owned())
    })?)
    .bind(event.event_id().as_str())
    .bind(event.payload_hash().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert resource lane claim", error))?;
    insert_resource_lane_transition_tx(
        tx,
        ResourceLaneTransitionInsert {
            lane_id: &lane_id,
            lane_transition_seq: payload.lane_transition_seq,
            transition_kind: "claim",
            claim_id: &payload.claim_id,
            release_id: None,
            claim_fencing_token: payload.claim_fencing_token,
            event,
            commit_id,
        },
    )
    .await
}

pub(super) async fn insert_resource_lane_release_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit_id: &str,
    event: &KernelEventEnvelope,
    payload: &events::ResourceLaneReleased,
) -> Result<()> {
    let claim = load_resource_lane_claim_row_tx(tx, &payload.claim_id).await?;
    if claim.run_id != event.run_id().as_str()
        || claim.claim_fencing_token != payload.claim_fencing_token
    {
        return Err(PostgresStoreError::Corruption(
            "resource lane release claim binding mismatch".to_owned(),
        ));
    }
    sqlx::query(
        "INSERT INTO resource_lane_release_events \
         (release_id, claim_id, lane_id, run_id, node_id, attempt_id, ledger_key, ledger_purpose, \
          invocation_epoch, claim_fencing_token, release_reason, commit_id, source_seq, \
          source_ordinal, source_event_id, source_event_type, source_event_payload_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'ResourceLaneReleased',$16)",
    )
    .bind(payload.release_id.as_str())
    .bind(payload.claim_id.as_str())
    .bind(claim.lane_id.as_slice())
    .bind(event.run_id().as_str())
    .bind(payload.node_id.as_str())
    .bind(payload.attempt_id.as_str())
    .bind(payload.ledger_key.as_str())
    .bind(side_effect_ledger_purpose_json(&payload.ledger_purpose))
    .bind(i32::try_from(payload.invocation_epoch).map_err(|_| {
        PostgresStoreError::Corruption("resource lane invocation epoch overflow".to_owned())
    })?)
    .bind(u64_to_i64(
        payload.claim_fencing_token,
        "resource_lane_release_events.claim_fencing_token",
    )?)
    .bind(payload.release_reason.as_str())
    .bind(commit_id)
    .bind(u64_to_i64(
        event.seq().as_u64(),
        "resource_lane_release_events.source_seq",
    )?)
    .bind(i32::try_from(event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption("resource lane source ordinal overflow".to_owned())
    })?)
    .bind(event.event_id().as_str())
    .bind(event.payload_hash().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert resource lane release", error))?;
    insert_resource_lane_transition_tx(
        tx,
        ResourceLaneTransitionInsert {
            lane_id: &claim.lane_id,
            lane_transition_seq: payload.lane_transition_seq,
            transition_kind: "release",
            claim_id: &payload.claim_id,
            release_id: Some(&payload.release_id),
            claim_fencing_token: payload.claim_fencing_token,
            event,
            commit_id,
        },
    )
    .await
}

pub(super) struct ResourceLaneTransitionInsert<'a> {
    pub(super) lane_id: &'a [u8],
    pub(super) lane_transition_seq: u64,
    pub(super) transition_kind: &'a str,
    pub(super) claim_id: &'a events::ResourceLaneClaimId,
    pub(super) release_id: Option<&'a events::ResourceLaneReleaseId>,
    pub(super) claim_fencing_token: u64,
    pub(super) event: &'a KernelEventEnvelope,
    pub(super) commit_id: &'a str,
}

pub(super) async fn insert_resource_lane_transition_tx(
    tx: &mut Transaction<'_, Postgres>,
    transition: ResourceLaneTransitionInsert<'_>,
) -> Result<()> {
    let previous_transition_hash =
        previous_resource_lane_transition_hash_tx(tx, transition.lane_id).await?;
    let transition_hash =
        resource_lane_transition_hash(&transition, previous_transition_hash.as_deref())?;
    sqlx::query(
        "INSERT INTO resource_lane_transitions \
         (lane_id, lane_transition_seq, transition_kind, claim_id, release_id, claim_fencing_token, \
          previous_transition_hash, transition_hash, run_id, commit_id, source_seq, source_ordinal, \
         source_event_id, source_event_type, source_event_payload_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
    )
    .bind(transition.lane_id)
    .bind(u64_to_i64(
        transition.lane_transition_seq,
        "resource_lane_transitions.lane_transition_seq",
    )?)
    .bind(transition.transition_kind)
    .bind(transition.claim_id.as_str())
    .bind(
        transition
            .release_id
            .map(events::ResourceLaneReleaseId::as_str),
    )
    .bind(u64_to_i64(
        transition.claim_fencing_token,
        "resource_lane_transitions.claim_fencing_token",
    )?)
    .bind(previous_transition_hash)
    .bind(transition_hash)
    .bind(transition.event.run_id().as_str())
    .bind(transition.commit_id)
    .bind(u64_to_i64(
        transition.event.seq().as_u64(),
        "resource_lane_transitions.source_seq",
    )?)
    .bind(i32::try_from(transition.event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption("resource lane source ordinal overflow".to_owned())
    })?)
    .bind(transition.event.event_id().as_str())
    .bind(match transition.event.payload() {
        events::KernelEventPayload::ResourceLaneClaimed(_) => "ResourceLaneClaimed",
        events::KernelEventPayload::ResourceLaneReleased(_) => "ResourceLaneReleased",
        _ => {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition source event is not a lane event".to_owned(),
            ))
        }
    })
    .bind(transition.event.payload_hash().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert resource lane transition", error))?;
    Ok(())
}

pub(super) async fn previous_resource_lane_transition_hash_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane_id: &[u8],
) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT transition_hash FROM resource_lane_transitions \
         WHERE lane_id = $1 ORDER BY lane_transition_seq DESC LIMIT 1",
    )
    .bind(lane_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load previous resource lane transition", error))
}

pub(super) struct ResourceLaneTransitionAuthorityRow {
    pub(super) lane_id: Vec<u8>,
    pub(super) lane_transition_seq: u64,
    pub(super) transition_kind: String,
    pub(super) claim_id: String,
    pub(super) release_id: Option<String>,
    pub(super) claim_fencing_token: u64,
    pub(super) previous_transition_hash: Option<String>,
    pub(super) transition_hash: String,
    pub(super) source_seq: u64,
    pub(super) source_ordinal: u32,
    pub(super) source_event_id: String,
    pub(super) source_event_type: String,
    pub(super) source_event_payload_hash: String,
}

pub(super) async fn validate_resource_lane_transition_hash_chains_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT lane_id, lane_transition_seq, transition_kind, claim_id, release_id, \
         claim_fencing_token, previous_transition_hash, transition_hash, source_seq, \
         source_ordinal, source_event_id, source_event_type, source_event_payload_hash \
         FROM resource_lane_transitions ORDER BY lane_id ASC, lane_transition_seq ASC",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load resource lane transition rows", error))?;
    let mut current_lane: Option<Vec<u8>> = None;
    let mut expected_seq = 1_u64;
    let mut expected_previous_hash: Option<String> = None;
    for row in rows {
        let row = resource_lane_transition_authority_row(row)?;
        if current_lane.as_ref() != Some(&row.lane_id) {
            current_lane = Some(row.lane_id.clone());
            expected_seq = 1;
            expected_previous_hash = None;
        }
        if row.lane_transition_seq != expected_seq {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition sequence gap".to_owned(),
            ));
        }
        if row.previous_transition_hash != expected_previous_hash {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition previous hash mismatch".to_owned(),
            ));
        }
        if !resource_lane_transition_kind_matches_source(&row) {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition kind/source mismatch".to_owned(),
            ));
        }
        let expected_hash = resource_lane_transition_hash_from_row(&row)?;
        if row.transition_hash != expected_hash {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition hash mismatch".to_owned(),
            ));
        }
        expected_previous_hash = Some(row.transition_hash);
        expected_seq = expected_seq.checked_add(1).ok_or_else(|| {
            PostgresStoreError::Corruption("resource lane transition sequence overflow".to_owned())
        })?;
    }
    Ok(())
}

pub(super) fn resource_lane_transition_authority_row(
    row: PgRow,
) -> Result<ResourceLaneTransitionAuthorityRow> {
    Ok(ResourceLaneTransitionAuthorityRow {
        lane_id: row
            .try_get("lane_id")
            .map_err(|error| database_error("failed to decode transition lane id", error))?,
        lane_transition_seq: i64_to_positive_u64(
            row.try_get("lane_transition_seq")
                .map_err(|error| database_error("failed to decode transition sequence", error))?,
            "resource_lane_transitions.lane_transition_seq",
        )?,
        transition_kind: row
            .try_get("transition_kind")
            .map_err(|error| database_error("failed to decode transition kind", error))?,
        claim_id: row
            .try_get("claim_id")
            .map_err(|error| database_error("failed to decode transition claim id", error))?,
        release_id: row
            .try_get("release_id")
            .map_err(|error| database_error("failed to decode transition release id", error))?,
        claim_fencing_token: i64_to_positive_u64(
            row.try_get("claim_fencing_token").map_err(|error| {
                database_error("failed to decode transition fencing token", error)
            })?,
            "resource_lane_transitions.claim_fencing_token",
        )?,
        previous_transition_hash: row
            .try_get("previous_transition_hash")
            .map_err(|error| database_error("failed to decode transition previous hash", error))?,
        transition_hash: row
            .try_get("transition_hash")
            .map_err(|error| database_error("failed to decode transition hash", error))?,
        source_seq: i64_to_positive_u64(
            row.try_get("source_seq")
                .map_err(|error| database_error("failed to decode transition source seq", error))?,
            "resource_lane_transitions.source_seq",
        )?,
        source_ordinal: i32_to_u32(
            row.try_get("source_ordinal").map_err(|error| {
                database_error("failed to decode transition source ordinal", error)
            })?,
            "resource_lane_transitions.source_ordinal",
        )?,
        source_event_id: row
            .try_get("source_event_id")
            .map_err(|error| database_error("failed to decode transition event id", error))?,
        source_event_type: row
            .try_get("source_event_type")
            .map_err(|error| database_error("failed to decode transition event type", error))?,
        source_event_payload_hash: row
            .try_get("source_event_payload_hash")
            .map_err(|error| database_error("failed to decode transition payload hash", error))?,
    })
}

pub(super) fn resource_lane_transition_kind_matches_source(
    row: &ResourceLaneTransitionAuthorityRow,
) -> bool {
    matches!(
        (
            row.transition_kind.as_str(),
            row.release_id.is_some(),
            row.source_event_type.as_str()
        ),
        ("claim", false, "ResourceLaneClaimed") | ("release", true, "ResourceLaneReleased")
    )
}

pub(super) fn resource_lane_transition_hash_from_row(
    row: &ResourceLaneTransitionAuthorityRow,
) -> Result<String> {
    let digest = canonical_json(serde_json::json!({
        "claim_fencing_token": row.claim_fencing_token,
        "claim_id": row.claim_id.as_str(),
        "lane_id": bytes_hex(&row.lane_id),
        "lane_transition_seq": row.lane_transition_seq,
        "previous_transition_hash": row.previous_transition_hash.as_deref(),
        "release_id": row.release_id.as_deref(),
        "source_event_id": row.source_event_id.as_str(),
        "source_event_payload_hash": row.source_event_payload_hash.as_str(),
        "source_ordinal": row.source_ordinal,
        "source_seq": row.source_seq,
        "transition_kind": row.transition_kind.as_str(),
    }))?
    .content_digest();
    Ok(digest.as_str().to_owned())
}

pub(super) fn resource_lane_transition_hash(
    transition: &ResourceLaneTransitionInsert<'_>,
    previous_transition_hash: Option<&str>,
) -> Result<String> {
    let digest = canonical_json(serde_json::json!({
        "claim_fencing_token": transition.claim_fencing_token,
        "claim_id": transition.claim_id.as_str(),
        "lane_id": bytes_hex(transition.lane_id),
        "lane_transition_seq": transition.lane_transition_seq,
        "previous_transition_hash": previous_transition_hash,
        "release_id": transition.release_id.map(events::ResourceLaneReleaseId::as_str),
        "source_event_id": transition.event.event_id().as_str(),
        "source_event_payload_hash": transition.event.payload_hash().as_str(),
        "source_ordinal": transition.event.ordinal().as_u32(),
        "source_seq": transition.event.seq().as_u64(),
        "transition_kind": transition.transition_kind,
    }))?
    .content_digest();
    Ok(digest.as_str().to_owned())
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

pub(super) struct ResourceLaneClaimAuthorityRow {
    pub(super) lane_id: Vec<u8>,
    pub(super) run_id: String,
    pub(super) claim_fencing_token: u64,
}

pub(super) async fn load_resource_lane_claim_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    claim_id: &events::ResourceLaneClaimId,
) -> Result<ResourceLaneClaimAuthorityRow> {
    let row = sqlx::query(
        "SELECT lane_id, run_id, fencing_token \
         FROM resource_lane_claim_events WHERE claim_id = $1",
    )
    .bind(claim_id.as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load resource lane claim", error))?;
    let fencing_token: i64 = row
        .try_get("fencing_token")
        .map_err(|error| database_error("failed to decode resource lane claim token", error))?;
    Ok(ResourceLaneClaimAuthorityRow {
        lane_id: row
            .try_get("lane_id")
            .map_err(|error| database_error("failed to decode resource lane id", error))?,
        run_id: row.try_get("run_id").map_err(|error| {
            database_error("failed to decode resource lane claim run id", error)
        })?,
        claim_fencing_token: i64_to_positive_u64(
            fencing_token,
            "resource_lane_claim_events.fencing_token",
        )?,
    })
}
