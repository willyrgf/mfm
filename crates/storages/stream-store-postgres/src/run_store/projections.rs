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
        if !is_unique_logical_key(&logical_key) {
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

pub(super) async fn load_stream_authoritative_projection_snapshot_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let stream = load_run_stream_tx(tx, run_id).await?;
    let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
    let resource_lanes = load_active_resource_lanes_tx(tx).await?;
    projection_snapshot_with_resource_lanes(&snapshot, resource_lanes)
}

pub(super) async fn load_active_resource_lanes_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<BTreeMap<ResourceLaneKey, ResourceLaneProjection>> {
    let rows = sqlx::query(
        "SELECT c.lane_id, c.run_id, c.node_id, c.attempt_id, c.ledger_key, c.ledger_purpose, \
         c.invocation_epoch, c.namespace, c.key_schema_id, c.key_value, c.claim_id, \
         c.fencing_token, c.source_event_id, t.lane_transition_seq \
         FROM resource_lane_claim_events c \
         INNER JOIN resource_lane_transitions t \
           ON t.claim_id = c.claim_id AND t.transition_kind = 'claim' \
         WHERE NOT EXISTS (\
           SELECT 1 FROM resource_lane_release_events r WHERE r.claim_id = c.claim_id\
         ) \
         ORDER BY c.lane_id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load active resource lanes", error))?;
    let mut resource_lanes = BTreeMap::new();
    for row in rows {
        let evidence = resource_key_evidence_from_lane_row(&row)?;
        verify_lane_id_row(&row, &evidence)?;
        let lane_key = ResourceLaneKey::from_evidence(&evidence);
        let projection = ResourceLaneProjection {
            event_id: parse_identity(&row.try_get::<String, _>("source_event_id").map_err(
                |error| database_error("failed to decode resource lane event id", error),
            )?)?,
            holder: mfm_store::v1::SideEffectLedgerRef::new(
                parse_identity(&row.try_get::<String, _>("run_id").map_err(|error| {
                    database_error("failed to decode resource lane run id", error)
                })?)?,
                events::SideEffectLedgerKey::new(row.try_get::<String, _>("ledger_key").map_err(
                    |error| database_error("failed to decode resource lane ledger key", error),
                )?)?,
            ),
            ledger_purpose: parse_side_effect_ledger_purpose(
                &row.try_get::<Value, _>("ledger_purpose").map_err(|error| {
                    database_error("failed to decode resource lane purpose", error)
                })?,
            )?,
            node_id: parse_identity(&row.try_get::<String, _>("node_id").map_err(|error| {
                database_error("failed to decode resource lane node id", error)
            })?)?,
            attempt_id: parse_identity(&row.try_get::<String, _>("attempt_id").map_err(
                |error| database_error("failed to decode resource lane attempt id", error),
            )?)?,
            invocation_epoch: i32_to_u32(
                row.try_get("invocation_epoch").map_err(|error| {
                    database_error("failed to decode resource lane epoch", error)
                })?,
                "resource_lane_claim_events.invocation_epoch",
            )?,
            claim_id: events::ResourceLaneClaimId::new(
                row.try_get::<String, _>("claim_id").map_err(|error| {
                    database_error("failed to decode resource lane claim id", error)
                })?,
            )?,
            claim_fencing_token: i64_to_positive_u64(
                row.try_get("fencing_token").map_err(|error| {
                    database_error("failed to decode resource lane fencing token", error)
                })?,
                "resource_lane_claim_events.fencing_token",
            )?,
            lane_transition_seq: i64_to_positive_u64(
                row.try_get("lane_transition_seq").map_err(|error| {
                    database_error("failed to decode resource lane transition seq", error)
                })?,
                "resource_lane_transitions.lane_transition_seq",
            )?,
        };
        if resource_lanes.insert(lane_key, projection).is_some() {
            return Err(PostgresStoreError::Corruption(
                "resource lane tables contain duplicate active resource lane".to_owned(),
            ));
        }
    }
    Ok(resource_lanes)
}

pub(super) async fn load_resource_lane_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<ResourceLaneAuthoritySet> {
    let rows = sqlx::query(
        "SELECT c.lane_id, c.namespace, c.key_schema_id, c.key_value, \
         MAX(t.lane_transition_seq) AS last_transition_seq, \
         MAX(t.claim_fencing_token) AS last_claim_fencing_token \
         FROM resource_lane_transitions t \
         INNER JOIN resource_lane_claim_events c ON c.claim_id = t.claim_id \
         GROUP BY c.lane_id, c.namespace, c.key_schema_id, c.key_value",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load resource lane authority", error))?;
    let mut authority = ResourceLaneAuthoritySet::new();
    for row in rows {
        let evidence = resource_key_evidence_from_lane_row(&row)?;
        verify_lane_id_row(&row, &evidence)?;
        authority.insert(
            ResourceLaneKey::from_evidence(&evidence),
            mfm_store::v1::ResourceLaneAuthority {
                last_transition_seq: i64_to_positive_u64(
                    row.try_get("last_transition_seq").map_err(|error| {
                        database_error("failed to decode resource lane transition seq", error)
                    })?,
                    "resource_lane_transitions.lane_transition_seq",
                )?,
                last_claim_fencing_token: i64_to_positive_u64(
                    row.try_get("last_claim_fencing_token").map_err(|error| {
                        database_error("failed to decode resource lane fencing token", error)
                    })?,
                    "resource_lane_transitions.claim_fencing_token",
                )?,
            },
        );
    }
    Ok(authority)
}

pub(super) fn resource_key_evidence_from_lane_row(
    row: &PgRow,
) -> Result<events::ResourceKeyEvidence> {
    Ok(events::ResourceKeyEvidence {
        namespace: ResourceNamespace::new(
            row.try_get::<String, _>("namespace").map_err(|error| {
                database_error("failed to decode resource lane namespace", error)
            })?,
        )
        .map_err(|error| PostgresStoreError::Store(StoreError::Identity(error.to_string())))?,
        key_schema_id: parse_identity(&row.try_get::<String, _>("key_schema_id").map_err(
            |error| database_error("failed to decode resource lane key schema", error),
        )?)?,
        key: events::ResourceKey::new(
            row.try_get::<String, _>("key_value").map_err(|error| {
                database_error("failed to decode resource lane key value", error)
            })?,
        )?,
    })
}

pub(super) fn verify_lane_id_row(
    row: &PgRow,
    evidence: &events::ResourceKeyEvidence,
) -> Result<()> {
    let stored: Vec<u8> = row
        .try_get("lane_id")
        .map_err(|error| database_error("failed to decode resource lane id", error))?;
    let derived = resource_lane_id(evidence)?;
    if stored != derived {
        return Err(PostgresStoreError::Corruption(
            "resource lane id does not match lane descriptor".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn projection_snapshot_with_resource_lanes(
    snapshot: &ProjectionSnapshot,
    resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
) -> Result<ProjectionSnapshot> {
    Ok(ProjectionSnapshot::from_parts(ProjectionSnapshotParts {
        run_states: snapshot
            .run_states()
            .map(|(run_id, state)| (run_id.clone(), *state))
            .collect(),
        saga_policy_digests: snapshot
            .saga_policy_digests()
            .map(|(run_id, digest)| (run_id.clone(), digest.clone()))
            .collect(),
        run_completions: snapshot
            .run_completions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        saga_engagements: snapshot
            .saga_engagements()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        manual_resolutions: snapshot
            .manual_resolutions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        attempts: snapshot
            .attempts()
            .map(|(key, projection)| (key.clone(), projection.clone()))
            .collect(),
        cells: snapshot
            .cells()
            .map(|(cell_id, projection)| (cell_id.clone(), projection.clone()))
            .collect(),
        facts: snapshot
            .facts()
            .map(|(key, projection)| (key.clone(), projection.clone()))
            .collect(),
        side_effects: snapshot
            .side_effects()
            .map(|(ledger_ref, projection)| (ledger_ref.clone(), projection.clone()))
            .collect(),
        resource_lanes,
        public_outputs: snapshot
            .public_outputs()
            .map(|(schema_id, projection)| (schema_id.clone(), projection.clone()))
            .collect(),
        retentions: snapshot
            .retentions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
    })?)
}
