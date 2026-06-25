use super::*;

pub(super) async fn lock_resource_lanes_for_request_tx(
    tx: &mut Transaction<'_, Postgres>,
    request: &mfm_store::v1::CommitRequest,
    projections: &ProjectionSnapshot,
) -> Result<BTreeMap<mfm_store::v1::AdmissionLaneKey, mfm_store::v1::ResourceAdmissionLane>> {
    let mut lanes =
        BTreeMap::<mfm_store::v1::AdmissionLaneKey, mfm_store::v1::ResourceAdmissionLane>::new();
    for payload in request.payloads() {
        match payload {
            events::KernelEventPayload::ResourceLaneClaimIntent(intent) => {
                let lane = mfm_store::v1::ResourceAdmissionLane::from_resource_key_evidence(
                    &intent.resource_key,
                )?;
                lanes.insert(lane.erased_key(), lane);
            }
            events::KernelEventPayload::ResourceLaneReleaseIntent(intent) => {
                let lane = resource_lane_for_release(request.run_id(), projections, intent)?;
                lanes.insert(lane.erased_key(), lane);
            }
            _ => {}
        }
    }
    for lane in lanes.values() {
        lock_admission_lane_tx(tx, &lane.erased_key()).await?;
    }
    Ok(lanes)
}

pub(super) fn resource_lane_for_release(
    run_id: &RunId,
    projections: &ProjectionSnapshot,
    intent: &events::ResourceLaneReleaseIntent,
) -> Result<mfm_store::v1::ResourceAdmissionLane> {
    let holder =
        mfm_store::v1::SideEffectPairLedgerRef::new(run_id.clone(), intent.pair_id.clone());
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
    let Some((pair_lane_key, _)) = projections.resource_lanes().find(|(_, projection)| {
        projection.holder.run_id == *run_id && projection.pair_id == intent.pair_id
    }) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", intent.ledger_key),
            message: "resource lane release pair references unknown active claim".to_owned(),
        }
        .into());
    };
    if pair_lane_key != lane_key {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", intent.ledger_key),
            message: "resource lane release pair does not match active holder".to_owned(),
        }
        .into());
    }
    if active.claim_id != intent.claim_id
        || active.node_id != intent.node_id
        || active.attempt_id != intent.attempt_id
        || active.ledger_purpose != intent.ledger_purpose
        || active.pair_id != intent.pair_id
        || active.invocation_epoch != intent.invocation_epoch
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
            message: "resource lane release intent does not match active claim".to_owned(),
        }
        .into());
    }
    Ok(mfm_store::v1::ResourceAdmissionLane::from_resource_lane_key(lane_key)?)
}

pub(super) struct ResourceLaneClaimAdmission {
    pub(super) lane: mfm_store::v1::ResourceAdmissionLane,
    pub(super) lane_key: ResourceLaneKey,
    pub(super) holder: mfm_store::v1::SideEffectPairLedgerRef,
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
    let lane_key = ResourceLaneKey::from_evidence(&intent.resource_key);
    let holder = mfm_store::v1::SideEffectPairLedgerRef::new(
        request.run_id().clone(),
        intent.pair_id.clone(),
    );
    let admission_token =
        mfm_store::v1::resource_wait_fifo_admission_token(request.run_id(), &lane, intent)?;
    Ok(Some(ResourceLaneClaimAdmission {
        lane,
        lane_key,
        holder,
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
            let waiter = enqueue_or_refresh_wait_fifo_waiter_tx(
                tx,
                &admission.lane,
                &admission.admission_token,
            )
            .await?;
            return Ok(Some(mfm_store::v1::WaitFifoAdmissionBlock {
                lane: admission.lane.clone(),
                resource_lane_key: admission.lane_key.clone(),
                holder: Some(active.holder.clone()),
                waiter: Some(waiter),
            }));
        }
    }

    if let Some(head) = head_wait_fifo_waiter_tx(tx, &admission.lane).await? {
        if head.admission_token != admission.admission_token {
            let waiter = enqueue_or_refresh_wait_fifo_waiter_tx(
                tx,
                &admission.lane,
                &admission.admission_token,
            )
            .await?;
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
