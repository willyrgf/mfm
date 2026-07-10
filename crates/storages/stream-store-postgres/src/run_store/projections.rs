use super::*;

pub(super) async fn load_logical_keys(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeSet<(RunId, LogicalEventKey)>> {
    let rows = sqlx::query("SELECT logical_key FROM run_events WHERE run_id = $1")
        .bind(run_id.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load logical keys", error))?;
    let mut keys = BTreeSet::new();
    for row in rows {
        let logical_key: String = row
            .try_get("logical_key")
            .map_err(|error| database_error("failed to decode logical key", error))?;
        keys.insert((run_id.clone(), LogicalEventKey::new(logical_key)?));
    }
    Ok(keys)
}

pub(super) async fn load_unique_logical_payloads(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeMap<(RunId, LogicalEventKey), ContentDigest>> {
    let rows = sqlx::query("SELECT logical_key, payload_hash FROM run_events WHERE run_id = $1")
        .bind(run_id.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load unique logical payloads", error))?;
    let mut payloads = BTreeMap::new();
    for row in rows {
        let logical_key = LogicalEventKey::new(
            row.try_get::<String, _>("logical_key")
                .map_err(|error| database_error("failed to decode logical key", error))?,
        )?;
        if !mfm_store::v1::backend::is_unique_logical_key(&logical_key) {
            continue;
        }
        let payload_hash = parse_identity::<ContentDigest>(
            &row.try_get::<String, _>("payload_hash")
                .map_err(|error| database_error("failed to decode payload hash", error))?,
        )?;
        let key = (run_id.clone(), logical_key);
        if let Some(existing) = payloads.insert(key.clone(), payload_hash.clone()) {
            if existing != payload_hash {
                return Err(PostgresStoreError::Corruption(format!(
                    "run_events contain conflicting payload hashes for logical key {}",
                    key.1
                )));
            }
        }
    }
    Ok(payloads)
}

pub(super) async fn load_projection_snapshot_client(
    pool: &PgPool,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start read transaction", error))?;
    let snapshot = load_stream_authoritative_projection_snapshot_tx(&mut tx, run_id).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read transaction", error))?;
    Ok(snapshot)
}

pub(super) async fn load_fact_projection_snapshot_client(
    pool: &PgPool,
) -> Result<ProjectionSnapshot> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start fact projection read", error))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to set fact projection read mode", error))?;
    let authority_events = super::fact_queries::load_fact_authority_events_tx(&mut tx).await?;
    let snapshot = super::fact_queries::load_authoritative_fact_projection_snapshot_tx(
        &mut tx,
        &authority_events,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit fact projection read", error))?;
    Ok(snapshot)
}

pub(super) async fn load_stream_authoritative_projection_snapshot_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let stream = load_run_stream_tx(tx, run_id).await?;
    let snapshot = projection_snapshot_from_physical_fact_tables_tx(tx, run_id, &stream).await?;
    let resource_lane_state = load_resource_lane_state_tx(tx).await?;
    let authority_events = super::fact_queries::load_fact_authority_events_tx(tx).await?;
    let store_fact_projection =
        super::fact_queries::load_authoritative_fact_projection_snapshot_tx(tx, &authority_events)
            .await?;
    projection_snapshot_with_store_authority(
        &snapshot,
        &store_fact_projection,
        resource_lane_state.active,
    )
}

pub(super) async fn projection_snapshot_from_physical_fact_tables_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    stream: &[KernelEventEnvelope],
) -> Result<ProjectionSnapshot> {
    let artifact_bytes = load_fact_rebuild_artifact_bytes_tx(tx, stream).await?;
    let snapshot =
        ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(stream, &artifact_bytes)?;
    let fact_projections = load_fact_projection_tables_tx(tx, run_id).await?;
    let expected = PhysicalFactProjections::from_snapshot_and_stream(&snapshot, stream)?;
    if fact_projections != expected {
        return Err(PostgresStoreError::Corruption(
            "physical fact projections did not match authoritative run stream".to_owned(),
        ));
    }
    Ok(snapshot)
}

#[cfg(all(test, feature = "parity-tests"))]
pub(super) async fn rebuild_fact_projection_tables_client(
    pool: &PgPool,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start fact projection rebuild", error))?;
    lock_run_tx(&mut tx, run_id).await?;
    let stream = load_run_stream_tx(&mut tx, run_id).await?;
    let snapshot = rebuild_fact_projection_tables_tx(&mut tx, run_id, &stream).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit fact projection rebuild", error))?;
    Ok(snapshot)
}

pub(super) struct ResourceLaneState {
    pub(super) active: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
    pub(super) authority: ResourceLaneAuthoritySet,
}

pub(super) async fn load_resource_lane_state_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<ResourceLaneState> {
    let rows = sqlx::query(
        "SELECT e.run_id, e.seq, e.ordinal, c.store_commit_order, e.event_id, e.event_schema_id, e.spec_hash, \
         e.commit_key, e.logical_key, e.payload_hash, e.payload_canonical_json \
         FROM run_events e \
         JOIN commits c ON c.run_id = e.run_id AND c.seq = e.seq \
         WHERE e.logical_key LIKE 'resource_lane:%' \
         ORDER BY c.store_commit_order ASC, e.ordinal ASC",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load resource lane events", error))?;
    let events = rows
        .into_iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    fold_resource_lane_state(&events)
}

pub(super) fn fold_resource_lane_state(
    events: &[KernelEventEnvelope],
) -> Result<ResourceLaneState> {
    let mut active: BTreeMap<ResourceLaneKey, ResourceLaneProjection> = BTreeMap::new();
    let mut authority = ResourceLaneAuthoritySet::new();
    let mut claim_lanes = BTreeMap::new();

    for event in events {
        if let events::KernelEventPayload::ResourceLaneClaimed(payload) = event.payload() {
            claim_lanes.insert(
                payload.claim_id.clone(),
                ResourceLaneKey::from_evidence(&payload.resource_key),
            );
        }
    }

    for event in events {
        match event.payload() {
            events::KernelEventPayload::ResourceLaneClaimed(payload) => {
                let lane_key = ResourceLaneKey::from_evidence(&payload.resource_key);
                let holder = mfm_store::v1::SideEffectPairLedgerRef::new(
                    event.run_id().clone(),
                    payload.pair_id.clone(),
                );
                if let Some((existing_key, _)) = active
                    .iter()
                    .find(|(key, projection)| **key != lane_key && projection.holder == holder)
                {
                    return Err(PostgresStoreError::Corruption(format!(
                        "resource lane holder already holds {}:{}",
                        existing_key.namespace, existing_key.key
                    )));
                }
                if let Some(existing) = active.get(&lane_key) {
                    if existing.holder != holder {
                        return Err(PostgresStoreError::Corruption(
                            "run_events contain conflicting active resource lane claims".to_owned(),
                        ));
                    }
                }
                let authority = authority.entry(lane_key.clone()).or_default();
                authority.last_transition_seq = authority
                    .last_transition_seq
                    .max(payload.lane_transition_seq);
                authority.last_claim_fencing_token = authority
                    .last_claim_fencing_token
                    .max(payload.claim_fencing_token);
                let projection = ResourceLaneProjection {
                    event_id: event.event_id().clone(),
                    holder,
                    ledger_key: payload.ledger_key.clone(),
                    ledger_purpose: payload.ledger_purpose.clone(),
                    node_id: payload.node_id.clone(),
                    attempt_id: payload.attempt_id.clone(),
                    invocation_epoch: payload.invocation_epoch,
                    claim_id: payload.claim_id.clone(),
                    claim_fencing_token: payload.claim_fencing_token,
                    lane_transition_seq: payload.lane_transition_seq,
                };
                if active.insert(lane_key, projection).is_some() {
                    return Err(PostgresStoreError::Corruption(
                        "run_events contain duplicate active resource lane claims".to_owned(),
                    ));
                }
            }
            events::KernelEventPayload::ResourceLaneReleased(payload) => {
                let lane_key = claim_lanes.get(&payload.claim_id).ok_or_else(|| {
                    PostgresStoreError::Corruption(
                        "resource lane release references unknown claim".to_owned(),
                    )
                })?;
                let authority = authority.entry(lane_key.clone()).or_default();
                authority.last_transition_seq = authority
                    .last_transition_seq
                    .max(payload.lane_transition_seq);
                authority.last_claim_fencing_token = authority
                    .last_claim_fencing_token
                    .max(payload.claim_fencing_token);
                let active_projection = active.get(lane_key).ok_or_else(|| {
                    PostgresStoreError::Corruption(
                        "resource lane release requires active claim".to_owned(),
                    )
                })?;
                if active_projection.claim_id != payload.claim_id
                    || active_projection.claim_fencing_token != payload.claim_fencing_token
                    || active_projection.holder.pair_id != payload.pair_id
                    || active_projection.ledger_purpose != payload.ledger_purpose
                    || active_projection.invocation_epoch != payload.invocation_epoch
                {
                    return Err(PostgresStoreError::Corruption(
                        "resource lane release does not match active claim".to_owned(),
                    ));
                }
                active.remove(lane_key);
            }
            _ => {}
        }
    }

    Ok(ResourceLaneState { active, authority })
}

pub(super) fn projection_snapshot_with_resource_lanes(
    snapshot: &ProjectionSnapshot,
    resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
) -> Result<ProjectionSnapshot> {
    let mut parts = ProjectionSnapshotParts::from_snapshot(snapshot);
    parts.resource_lanes = resource_lanes;
    Ok(ProjectionSnapshot::from_parts(parts)?)
}

fn projection_snapshot_with_store_authority(
    snapshot: &ProjectionSnapshot,
    fact_projection: &ProjectionSnapshot,
    resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
) -> Result<ProjectionSnapshot> {
    let snapshot = snapshot.with_store_authority_from(fact_projection)?;
    projection_snapshot_with_resource_lanes(&snapshot, resource_lanes)
}
