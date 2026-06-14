use super::*;

pub(super) fn acquire_resource_lane(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &side_effect::InvocationPrepared,
) -> Result<Option<events::ResourceKeyEvidence>> {
    let holder = SideEffectLedgerRef::new(run_id.clone(), payload.ledger_key.clone());
    let resource_key = match (
        projections
            .side_effects
            .get(&holder)
            .and_then(|projection| projection.resource_key.as_ref()),
        payload.resource_key.as_ref(),
    ) {
        (Some(previous), Some(current)) if previous == current => Some(current.clone()),
        (Some(_), Some(_)) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("resource_lane:{}", payload.ledger_key),
                message: "resource key evidence changed for ledger".to_owned(),
            });
        }
        (Some(_), None) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("resource_lane:{}", payload.ledger_key),
                message: "resource key evidence is required after prior resource key".to_owned(),
            });
        }
        (None, current) => current.cloned(),
    };

    let Some(resource_key) = resource_key.as_ref() else {
        return Ok(None);
    };

    let lane_key = ResourceLaneKey::from_evidence(resource_key);
    if let Some(existing_key) = resource_lane_key_for_holder(projections, &holder) {
        if existing_key != lane_key {
            return Err(StoreError::ProjectionConflict {
                key: format!("resource_lane:{}", payload.ledger_key),
                message: "resource lane key changed for ledger".to_owned(),
            });
        }
    }
    if let Some(existing) = projections.resource_lanes.get(&lane_key) {
        if existing.holder != holder {
            return Err(StoreError::ResourceLaneBlocked {
                lane_key: Box::new(lane_key),
                holder: Box::new(existing.holder.clone()),
            });
        }
    }

    projections.resource_lanes.insert(
        lane_key,
        ResourceLaneProjection {
            event_id: event_id.clone(),
            holder,
            ledger_purpose: payload.ledger_purpose.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            invocation_epoch: payload.invocation_epoch,
        },
    );
    Ok(Some(resource_key.clone()))
}

fn resource_lane_key_for_holder(
    projections: &ProjectionSnapshot,
    holder: &SideEffectLedgerRef,
) -> Option<ResourceLaneKey> {
    projections
        .resource_lanes
        .iter()
        .find_map(|(key, projection)| (projection.holder == *holder).then(|| key.clone()))
}

pub(super) fn release_resource_lane_for_holder(
    projections: &mut ProjectionSnapshot,
    holder: &SideEffectLedgerRef,
) {
    if let Some(key) = resource_lane_key_for_holder(projections, holder) {
        projections.resource_lanes.remove(&key);
    }
}

pub(super) fn release_resource_lanes_for_run(projections: &mut ProjectionSnapshot, run_id: &RunId) {
    projections
        .resource_lanes
        .retain(|_, projection| projection.holder.run_id != *run_id);
}
