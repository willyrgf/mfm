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
                let (lane_key, _) = mfm_store::v1::resource_lane_release_intent_resolution(
                    request.run_id(),
                    projections,
                    intent,
                )?;
                let lane = mfm_store::v1::ResourceAdmissionLane::from_resource_lane_key(lane_key)?;
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

pub(super) struct ResourceLaneClaimAdmission {
    pub(super) lane: mfm_store::v1::ResourceAdmissionLane,
    pub(super) lane_key: ResourceLaneKey,
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
    let admission_token =
        mfm_store::v1::resource_wait_fifo_admission_token(request.run_id(), &lane, intent)?;
    Ok(Some(ResourceLaneClaimAdmission {
        lane,
        lane_key,
        admission_token,
    }))
}

pub(super) async fn resource_lane_fifo_pre_gate_tx(
    tx: &mut Transaction<'_, Postgres>,
    admission: &ResourceLaneClaimAdmission,
) -> Result<Option<mfm_store::v1::WaitFifoAdmissionBlock>> {
    if let Some(head) = head_wait_fifo_waiter_tx(tx, &admission.lane).await? {
        if head.admission_token != admission.admission_token {
            let waiter = enqueue_or_refresh_wait_fifo_waiter_tx(
                tx,
                &admission.lane,
                &admission.admission_token,
            )
            .await?;
            return Ok(Some(mfm_store::v1::WaitFifoAdmissionBlock {
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
