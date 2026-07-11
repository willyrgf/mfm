use super::*;

/// Cross-run resource lane key derived from resource key evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceLaneKey {
    /// Resource namespace.
    pub namespace: ResourceNamespace,
    /// Schema id for the typed resource-key evidence.
    pub key_schema_id: SchemaId,
    /// Store-comparable resource key.
    pub key: events::ResourceKey,
}

impl ResourceLaneKey {
    /// Creates a lane key from typed resource key evidence.
    pub fn from_evidence(evidence: &events::ResourceKeyEvidence) -> Self {
        Self {
            namespace: evidence.namespace.clone(),
            key_schema_id: evidence.key_schema_id.clone(),
            key: evidence.key.clone(),
        }
    }
}

/// Active holder for an exclusive resource lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLaneProjection {
    /// Store event id that acquired or refreshed the lane.
    pub event_id: EventId,
    /// Run-scoped side-effect pair holding the lane.
    pub holder: SideEffectPairLedgerRef,
    /// Diagnostic side-effect ledger key that claimed the lane.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Ledger purpose.
    pub ledger_purpose: events::SideEffectLedgerPurpose,
    /// Node id that prepared the invocation.
    pub node_id: NodeId,
    /// Attempt id that prepared the invocation.
    pub attempt_id: AttemptId,
    /// Invocation epoch that prepared the invocation.
    pub invocation_epoch: u32,
    /// Store-visible claim id for the active lane claim.
    pub claim_id: events::ResourceLaneClaimId,
    /// Lane-local fencing token for the active claim.
    pub claim_fencing_token: u64,
    /// Lane-local transition sequence that acquired the active claim.
    pub lane_transition_seq: u64,
}

/// Durable lane-local authority folded from committed resource-lane transitions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceLaneAuthority {
    /// Highest committed lane-local transition sequence for this lane.
    pub last_transition_seq: u64,
    /// Highest committed lane-local fencing token assigned to a claim for this lane.
    pub last_claim_fencing_token: u64,
}

/// Resource-lane authority rows keyed by lane identity.
pub type ResourceLaneAuthoritySet = BTreeMap<ResourceLaneKey, ResourceLaneAuthority>;

pub(super) fn acquire_resource_lane(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::ResourceLaneClaimed,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
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
        }
    }

    projections.resource_lanes.insert(
        lane_key,
        ResourceLaneProjection {
            event_id: event_id.clone(),
            holder,
            ledger_key: payload.ledger_key.clone(),
            ledger_purpose: payload.ledger_purpose.clone(),
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
            (projection.holder.run_id == *run_id && projection.holder.pair_id == *pair_id)
                .then(|| key.clone())
        })
}

pub(super) trait ActiveResourceLaneReleaseView {
    fn holder(&self) -> &SideEffectPairLedgerRef;
    fn ledger_purpose(&self) -> &events::SideEffectLedgerPurpose;
    fn invocation_epoch(&self) -> u32;
    fn claim_id(&self) -> &events::ResourceLaneClaimId;
}

impl ActiveResourceLaneReleaseView for ResourceLaneProjection {
    fn holder(&self) -> &SideEffectPairLedgerRef {
        &self.holder
    }

    fn ledger_purpose(&self) -> &events::SideEffectLedgerPurpose {
        &self.ledger_purpose
    }

    fn invocation_epoch(&self) -> u32 {
        self.invocation_epoch
    }

    fn claim_id(&self) -> &events::ResourceLaneClaimId {
        &self.claim_id
    }
}

impl ActiveResourceLaneReleaseView for MaterializedActiveLane {
    fn holder(&self) -> &SideEffectPairLedgerRef {
        &self.holder
    }

    fn ledger_purpose(&self) -> &events::SideEffectLedgerPurpose {
        &self.ledger_purpose
    }

    fn invocation_epoch(&self) -> u32 {
        self.invocation_epoch
    }

    fn claim_id(&self) -> &events::ResourceLaneClaimId {
        &self.claim_id
    }
}

#[derive(Clone, Copy)]
pub(super) struct ResourceLaneReleaseMatch<'a> {
    pub(super) run_id: &'a RunId,
    pub(super) ledger_key: &'a events::SideEffectLedgerKey,
    pub(super) ledger_purpose: &'a events::SideEffectLedgerPurpose,
    pub(super) pair_id: &'a SideEffectPairId,
    pub(super) invocation_epoch: u32,
    pub(super) claim_id: &'a events::ResourceLaneClaimId,
    pub(super) mismatch_message: &'static str,
}

pub(super) fn resolve_active_resource_lane_release<'a, L>(
    active_lanes: &'a BTreeMap<ResourceLaneKey, L>,
    release: ResourceLaneReleaseMatch<'_>,
) -> Result<(&'a ResourceLaneKey, &'a L)>
where
    L: ActiveResourceLaneReleaseView,
{
    let ResourceLaneReleaseMatch {
        run_id,
        ledger_key,
        ledger_purpose,
        pair_id,
        invocation_epoch,
        claim_id,
        mismatch_message,
    } = release;
    let holder = SideEffectPairLedgerRef::new(run_id.clone(), pair_id.clone());
    let Some((key, active)) = active_lanes
        .iter()
        .find(|(_, active)| active.holder() == &holder)
    else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", ledger_key),
            message: "resource lane release requires an active claim".to_owned(),
        });
    };
    let Some(pair_key) = active_lanes.iter().find_map(|(key, active)| {
        (active.holder().run_id == *run_id && active.holder().pair_id == *pair_id).then_some(key)
    }) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", ledger_key),
            message: "resource lane release pair references no active claim".to_owned(),
        });
    };
    if pair_key != key {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}", ledger_key),
            message: "resource lane release pair does not match active holder".to_owned(),
        });
    }
    if active.holder().pair_id != *pair_id
        || active.ledger_purpose() != ledger_purpose
        || active.invocation_epoch() != invocation_epoch
        || active.claim_id() != claim_id
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}:{}", key.namespace, key.key),
            message: mismatch_message.to_owned(),
        });
    }
    Ok((key, active))
}

/// Resolves the active resource lane authorized by a release intent.
///
/// The returned lane is pair-bound: the active holder, side-effect pair, ledger purpose,
/// invocation epoch, and claim id must all match the release intent.
pub fn resource_lane_release_intent_resolution<'a>(
    run_id: &RunId,
    projections: &'a ProjectionSnapshot,
    intent: &events::ResourceLaneReleaseIntent,
) -> Result<(&'a ResourceLaneKey, &'a ResourceLaneProjection)> {
    require_side_effect_pair_role(
        &intent.ledger_key,
        &intent.ledger_purpose,
        intent.pair_role,
        events::SideEffectPairRole::Verify,
    )?;
    resolve_resource_lane_release(
        projections,
        ResourceLaneReleaseMatch {
            run_id,
            ledger_key: &intent.ledger_key,
            ledger_purpose: &intent.ledger_purpose,
            pair_id: &intent.pair_id,
            invocation_epoch: intent.invocation_epoch,
            claim_id: &intent.claim_id,
            mismatch_message: "resource lane release intent does not match active claim",
        },
    )
}

fn resolve_resource_lane_release<'a>(
    projections: &'a ProjectionSnapshot,
    release: ResourceLaneReleaseMatch<'_>,
) -> Result<(&'a ResourceLaneKey, &'a ResourceLaneProjection)> {
    resolve_active_resource_lane_release(&projections.resource_lanes, release)
}

pub(super) fn release_resource_lane(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    payload: &events::ResourceLaneReleased,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Verify,
    )?;
    let (key, active) = resolve_resource_lane_release(
        projections,
        ResourceLaneReleaseMatch {
            run_id,
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            invocation_epoch: payload.invocation_epoch,
            claim_id: &payload.claim_id,
            mismatch_message: "resource lane release does not match active claim",
        },
    )?;
    if active.claim_fencing_token != payload.claim_fencing_token {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane:{}:{}", key.namespace, key.key),
            message: "resource lane release does not match active claim".to_owned(),
        });
    }
    let key = key.clone();
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
