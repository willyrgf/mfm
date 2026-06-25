use super::*;

pub(super) fn acquire_resource_lane(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::ResourceLaneClaimed,
) -> Result<()> {
    require_side_effect_pair_event(
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    let holder = SideEffectPairLedgerRef::new(run_id.clone(), payload.pair_id.clone());
    let lane_key = ResourceLaneKey::from_evidence(&payload.resource_key);
    if let Some(existing_key) = resource_lane_key_for_pair(projections, run_id, &payload.pair_id) {
        let existing = projections
            .resource_lanes
            .get(&existing_key)
            .expect("resource lane key was found from pair projection");
        if existing.holder != holder {
            return Err(StoreError::ProjectionConflict {
                key: format!("resource_lane:{}", payload.ledger_key),
                message: "side-effect pair already has active resource lane".to_owned(),
            });
        }
    }
    if let Some(existing_key) = resource_lane_key_for_holder(projections, &holder) {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", payload.ledger_key),
            message: format!(
                "side-effect holder already has active resource lane {}:{}",
                existing_key.namespace, existing_key.key
            ),
        });
    }
    if let Some(existing) = projections.resource_lanes.get(&lane_key) {
        if existing.holder != holder {
            return Err(StoreError::ProjectionConflict {
                key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
                message: format!(
                    "resource lane already held by run {} pair {}",
                    existing.holder.run_id, existing.holder.pair_id
                ),
            });
        } else if existing.pair_id != payload.pair_id {
            return Err(StoreError::ProjectionConflict {
                key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
                message: "resource lane claim pair authority does not match active lane".to_owned(),
            });
        }
    }

    projections.resource_lanes.insert(
        lane_key,
        ResourceLaneProjection {
            event_id: event_id.clone(),
            holder,
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose: payload.ledger_purpose.clone(),
            pair_id: payload.pair_id.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            claim_id: payload.claim_id.clone(),
            claim_fencing_token: payload.claim_fencing_token,
            lane_transition_seq: payload.lane_transition_seq,
        },
    );
    Ok(())
}

fn resource_lane_key_for_holder(
    projections: &ProjectionSnapshot,
    holder: &SideEffectPairLedgerRef,
) -> Option<ResourceLaneKey> {
    projections
        .resource_lanes
        .iter()
        .find_map(|(key, projection)| (projection.holder == *holder).then(|| key.clone()))
}

fn resource_lane_key_for_pair(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    pair_id: &SideEffectPairId,
) -> Option<ResourceLaneKey> {
    projections
        .resource_lanes
        .iter()
        .find_map(|(key, projection)| {
            (projection.holder.run_id == *run_id && projection.pair_id == *pair_id)
                .then(|| key.clone())
        })
}

pub(super) fn release_resource_lane(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    payload: &events::ResourceLaneReleased,
) -> Result<()> {
    require_side_effect_pair_event(
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
        events::SideEffectPairRole::Verify,
    )?;
    let holder = SideEffectPairLedgerRef::new(run_id.clone(), payload.pair_id.clone());
    let Some(key) = resource_lane_key_for_holder(projections, &holder) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", payload.ledger_key),
            message: "resource lane release requires an active claim".to_owned(),
        });
    };
    let Some(pair_key) = resource_lane_key_for_pair(projections, run_id, &payload.pair_id) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", payload.ledger_key),
            message: "resource lane release pair references no active claim".to_owned(),
        });
    };
    if pair_key != key {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", payload.ledger_key),
            message: "resource lane release pair does not match active holder".to_owned(),
        });
    }
    let active = projections
        .resource_lanes
        .get(&key)
        .expect("resource lane key was found from projection");
    if active.pair_id != payload.pair_id
        || active.ledger_purpose != payload.ledger_purpose
        || active.invocation_epoch != payload.invocation_epoch
        || active.claim_id != payload.claim_id
        || active.claim_fencing_token != payload.claim_fencing_token
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}:{}", key.namespace, key.key),
            message: "resource lane release does not match active claim".to_owned(),
        });
    }
    projections.resource_lanes.remove(&key);
    Ok(())
}

pub(super) fn require_no_resource_lane_for_holder(
    projections: &ProjectionSnapshot,
    holder: &SideEffectPairLedgerRef,
    context: &str,
) -> Result<()> {
    if let Some(key) = resource_lane_key_for_holder(projections, holder) {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}:{}", key.namespace, key.key),
            message: format!("{context} requires a prior ResourceLaneReleased event"),
        });
    }
    Ok(())
}

pub(super) fn require_no_resource_lanes_for_run(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    context: &str,
) -> Result<()> {
    if let Some((key, _)) = projections
        .resource_lanes
        .iter()
        .find(|(_, projection)| projection.holder.run_id == *run_id)
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}:{}", key.namespace, key.key),
            message: format!("{context} requires prior ResourceLaneReleased events"),
        });
    }
    Ok(())
}
