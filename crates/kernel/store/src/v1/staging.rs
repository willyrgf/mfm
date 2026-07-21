use super::*;

/// Authoritative state needed to validate and stage one absent commit-key append.
///
/// Durable stores load this from their run stream, artifact table, logical-key table, and
/// rebuilt projections before calling [`stage_prepared_commit_plan`]. Commit-key lookup remains
/// the storage implementation's responsibility because the RFC requires that lookup to precede
/// stale `expected_next_seq` checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitBase {
    /// Exact artifact evidence recorded before event commit.
    pub artifacts: ArtifactAuthorityMap,
    /// Exact retained or same-commit artifact bytes available for projection validation.
    pub artifact_bytes: ArtifactByteAuthorityMap,
    /// Logical keys already present in the run stream.
    pub logical_keys: LogicalKeySet,
    /// Unique logical keys and their current payload hash.
    ///
    /// Most unique logical keys are immutable after their first write. The side-effect
    /// `submission_result` key is the one recoverable slot: `SubmissionUnknown` may be
    /// refreshed or superseded in projection by later recovery evidence for the same invocation
    /// epoch.
    pub unique_logical_payloads: UniqueLogicalPayloads,
    /// Current projections derived from the authoritative run stream.
    pub projections: ProjectionSnapshot,
    /// Lane-local transition authority folded from committed resource-lane history.
    pub resource_lane_authority: ResourceLaneAuthoritySet,
    /// Store-owned next sequence for the run being committed.
    pub actual_next_seq: StreamSeq,
    /// Store-owned append coordinate assigned to this atomic commit.
    pub store_commit_order: StoreCommitOrder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterializedActiveLane {
    holder: SideEffectPairLedgerRef,
    ledger_purpose: events::SideEffectLedgerPurpose,
    node_id: NodeId,
    attempt_id: AttemptId,
    invocation_epoch: u32,
    claim_id: events::ResourceLaneClaimId,
    claim_fencing_token: u64,
    lane_transition_seq: u64,
}

impl super::resource_lanes::ActiveResourceLaneReleaseView for MaterializedActiveLane {
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

/// Staged result of validating a typed commit against a [`CommitBase`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedCommit {
    batch: CommittedBatch,
    logical_keys: LogicalKeySet,
    unique_logical_payloads: UniqueLogicalPayloads,
    projections: ProjectionSnapshot,
    resource_lane_authority: ResourceLaneAuthoritySet,
}

impl StagedCommit {
    /// Store-owned committed batch.
    pub fn batch(&self) -> &CommittedBatch {
        &self.batch
    }

    /// Staged projection snapshot after this commit.
    pub fn projections(&self) -> &ProjectionSnapshot {
        &self.projections
    }

    /// Consumes this staged commit into owned parts.
    pub fn into_parts(
        self,
    ) -> (
        CommittedBatch,
        LogicalKeySet,
        UniqueLogicalPayloads,
        ProjectionSnapshot,
        ResourceLaneAuthoritySet,
    ) {
        (
            self.batch,
            self.logical_keys,
            self.unique_logical_payloads,
            self.projections,
            self.resource_lane_authority,
        )
    }
}

/// Result of staging an absent commit key before durable insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedCommitOutcome {
    /// The commit staged successfully and is ready for durable insertion.
    Staged(Box<StagedCommit>),
    /// FIFO admission was blocked; no domain authority rows should be persisted.
    AdmissionBlocked(Box<WaitFifoAdmissionBlock>),
}

struct CommitStagingVerifier<'a> {
    artifacts: &'a ArtifactAuthorityMap,
    logical_keys: &'a BTreeSet<(RunId, LogicalEventKey)>,
    projections: &'a ProjectionSnapshot,
}

impl CommitStagingVerifier<'_> {
    fn validate_artifact_evidence(&self, evidence: &ArtifactEvidenceRef) -> Result<()> {
        let key = artifact_authority_key(evidence)?;
        let Some(stored) = self.artifacts.get(&key) else {
            return Err(StoreError::MissingArtifact {
                artifact_id: evidence.artifact_id.clone(),
            });
        };
        compare_artifact_field(
            &evidence.artifact_id,
            "digest",
            stored.digest.as_str(),
            evidence.digest.as_str(),
        )?;
        compare_artifact_field(
            &evidence.artifact_id,
            "byte_len",
            stored.byte_len,
            evidence.byte_len,
        )?;
        compare_artifact_field(
            &evidence.artifact_id,
            "media_type",
            stored.media_type.as_str(),
            evidence.media_type.as_str(),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "schema_id",
            stored.schema_id.as_ref().map(SchemaId::as_str),
            evidence.schema_id.as_ref().map(SchemaId::as_str),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "semantic_type_id",
            stored.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
            evidence
                .semantic_type_id
                .as_ref()
                .map(SemanticTypeId::as_str),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "producer_node_id",
            stored.producer_node_id.as_ref().map(NodeId::as_str),
            evidence.producer_node_id.as_ref().map(NodeId::as_str),
        )?;
        compare_artifact_option(
            &evidence.artifact_id,
            "producer_seed_id",
            stored.producer_seed_id.as_ref().map(SeedId::as_str),
            evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        )?;
        compare_artifact_field(
            &evidence.artifact_id,
            "artifact_role",
            stored.artifact_role.as_str(),
            evidence.artifact_role.as_str(),
        )
    }

    fn validate_artifact_requirement(&self, requirement: &EventArtifactRequirement) -> Result<()> {
        let key = (
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        );
        let Some(evidence) = self.artifacts.get(&key) else {
            return Err(StoreError::MissingArtifact {
                artifact_id: requirement.artifact_id.clone(),
            });
        };
        validate_artifact_requirement_against_evidence(requirement, evidence)
    }

    fn validate_preconditions(&self, request: &CommitRequest) -> Result<()> {
        let actual_run_state = self.projections.run_state(&request.run_id);
        let run_state_ok = match request.preconditions.required_run_state {
            RequiredRunState::Any => true,
            RequiredRunState::Absent => actual_run_state == RunState::Absent,
            RequiredRunState::Started => actual_run_state != RunState::Absent,
            RequiredRunState::NotCompleted => actual_run_state == RunState::Started,
            RequiredRunState::Completed => actual_run_state == RunState::Completed,
        };
        if !run_state_ok {
            return Err(StoreError::RunStatePreconditionFailed {
                required: request.preconditions.required_run_state,
                actual: actual_run_state,
            });
        }

        for key in &request.preconditions.required_absent_logical_keys {
            if self
                .logical_keys
                .contains(&(request.run_id.clone(), key.clone()))
            {
                return Err(StoreError::LogicalKeyPreconditionFailed {
                    logical_key: key.clone(),
                    message: "key must be absent".to_owned(),
                });
            }
        }
        for key in &request.preconditions.required_present_logical_keys {
            if !self
                .logical_keys
                .contains(&(request.run_id.clone(), key.clone()))
            {
                return Err(StoreError::LogicalKeyPreconditionFailed {
                    logical_key: key.clone(),
                    message: "key must be present".to_owned(),
                });
            }
        }
        for precondition in &request.preconditions.required_cell_states {
            let actual = self
                .projections
                .cell_terminal_for_run(&request.run_id, &precondition.cell_id);
            let ok = match precondition.required {
                RequiredCellState::Absent => actual.is_none(),
                RequiredCellState::Produced => {
                    matches!(actual, Some(CellTerminalProjection::Produced { .. }))
                }
                RequiredCellState::Skipped => {
                    matches!(actual, Some(CellTerminalProjection::Skipped { .. }))
                }
                RequiredCellState::Terminal => actual.is_some(),
            };
            if !ok {
                return Err(StoreError::CellStatePreconditionFailed {
                    cell_id: precondition.cell_id.clone(),
                    required: precondition.required,
                });
            }
        }
        for precondition in &request.preconditions.required_side_effect_states {
            let actual = self
                .projections
                .side_effect_for_pair(&request.run_id, &precondition.pair_id);
            if !side_effect_precondition_matches(actual, precondition.required)? {
                return Err(StoreError::SideEffectStatePreconditionFailed {
                    pair_id: precondition.pair_id.clone(),
                    required: precondition.required,
                });
            }
        }
        if request.preconditions.required_public_output_absent
            && self.projections.has_public_output(&request.run_id)
        {
            return Err(StoreError::PublicOutputPreconditionFailed);
        }
        if request
            .payloads
            .iter()
            .any(|payload| matches!(payload, KernelEventPayload::FactRecorded(_)))
            && request.preconditions.certified_run_authority.is_none()
        {
            return Err(StoreError::ProjectionConflict {
                key: "fact:certified_run_authority".to_owned(),
                message: "fact recording requires certified run authority".to_owned(),
            });
        }
        Ok(())
    }
}

/// Validates and stages a purpose-specific prepared commit plan after commit-key idempotency handling.
///
/// The returned batch fingerprint covers the full prepared mutation, including the artifact
/// evidence admitted atomically with the event payloads.
pub fn stage_prepared_commit_plan(
    base: &CommitBase,
    plan: &PreparedCommitPlan,
) -> Result<StagedCommitOutcome> {
    let fingerprint = prepared_commit_plan_fingerprint(plan)?;
    match materialize_resource_lane_intents(base, plan.request(), &fingerprint)? {
        ResourceLaneMaterialization::Materialized {
            request,
            resource_lane_authority,
        } => {
            stage_run_commit_with_fingerprint(base, &request, fingerprint, resource_lane_authority)
                .map(|staged| StagedCommitOutcome::Staged(Box::new(staged)))
        }
        ResourceLaneMaterialization::Blocked(block) => {
            Ok(StagedCommitOutcome::AdmissionBlocked(block))
        }
    }
}

enum ResourceLaneMaterialization {
    Materialized {
        request: Box<CommitRequest>,
        resource_lane_authority: ResourceLaneAuthoritySet,
    },
    Blocked(Box<WaitFifoAdmissionBlock>),
}

fn stage_run_commit_with_fingerprint(
    base: &CommitBase,
    request: &CommitRequest,
    fingerprint: CommitFingerprint,
    staged_resource_lane_authority: ResourceLaneAuthoritySet,
) -> Result<StagedCommit> {
    if request.expected_next_seq != base.actual_next_seq {
        return Err(StoreError::StaleExpectedNextSeq {
            expected: request.expected_next_seq,
            actual: base.actual_next_seq,
        });
    }

    validate_terminal_attempt_cell_pairs(&request.payloads)?;
    validate_terminal_side_effect_evidence_pairs(&request.payloads)?;
    validate_side_effect_attempt_failures_have_terminal_evidence(
        &base.projections,
        &request.payloads,
    )?;
    validate_retention_manifest_pairs(&request.payloads)?;
    validate_payload_public_diagnostics(&request.payloads)?;

    let verifier = CommitStagingVerifier {
        artifacts: &base.artifacts,
        logical_keys: &base.logical_keys,
        projections: &base.projections,
    };
    verifier.validate_preconditions(request)?;

    for evidence in &request.required_artifacts {
        verifier.validate_artifact_evidence(evidence)?;
    }
    for payload in &request.payloads {
        for requirement in event_artifact_requirements(payload) {
            verifier.validate_artifact_requirement(&requirement)?;
        }
    }

    let mut staged_projections = base.projections.clone();
    let mut staged_logical_keys = base.logical_keys.clone();
    let mut staged_unique_payloads = base.unique_logical_payloads.clone();
    let mut commit_unique_keys = BTreeSet::new();
    let mut events = Vec::with_capacity(request.payloads.len());
    for (index, payload) in request.payloads.iter().cloned().enumerate() {
        let canonical_payload = payload_canonical_json(&payload)?;
        let payload_hash = canonical_payload.content_digest();
        let schema_id = payload.event_schema_id()?;
        let ordinal = CommitOrdinal::from_index(index)?;
        let logical_key = derive_logical_key(
            &request.run_id,
            request.expected_next_seq,
            ordinal,
            &payload,
            &payload_hash,
        )?;
        let event_id = derive_event_id(
            &request.run_id,
            request.expected_next_seq,
            ordinal,
            &schema_id,
            &payload_hash,
        )?;
        let spec_hash = payload_spec_hash(&payload);
        let envelope = KernelEventEnvelope {
            event_id,
            event_schema_id: schema_id,
            run_id: request.run_id.clone(),
            seq: request.expected_next_seq,
            store_commit_order: base.store_commit_order,
            ordinal,
            spec_hash,
            commit_key: request.commit_key.clone(),
            logical_key,
            payload_hash,
            payload,
        };

        let key = (request.run_id.clone(), envelope.logical_key.clone());
        if is_unique_logical_key(&envelope.logical_key) {
            if !commit_unique_keys.insert(key.clone()) {
                return Err(StoreError::DuplicateLogicalKey {
                    logical_key: envelope.logical_key.clone(),
                });
            }
            if let Some(existing_hash) = staged_unique_payloads.get(&key) {
                if existing_hash == &envelope.payload_hash {
                    return Err(StoreError::DuplicateLogicalKey {
                        logical_key: envelope.logical_key.clone(),
                    });
                }
                if !unique_logical_key_rewrite_allowed(
                    base,
                    &request.run_id,
                    &envelope.logical_key,
                    &envelope.payload,
                    &staged_projections,
                )? {
                    return Err(StoreError::LogicalKeyConflict {
                        logical_key: envelope.logical_key.clone(),
                    });
                }
            }
            staged_unique_payloads.insert(key.clone(), envelope.payload_hash.clone());
        }
        staged_logical_keys.insert(key);
        require_admission_preconditions(
            &staged_projections,
            &request.run_id,
            &envelope.payload,
            request.preconditions.certified_run_authority.as_ref(),
        )?;
        projection::apply_projection(&mut staged_projections, &envelope, &base.artifact_bytes)?;
        events.push(envelope);
    }

    Ok(StagedCommit {
        batch: CommittedBatch {
            run_id: request.run_id.clone(),
            commit_key: request.commit_key.clone(),
            fingerprint,
            seq: request.expected_next_seq,
            store_commit_order: base.store_commit_order,
            events,
        },
        logical_keys: staged_logical_keys,
        unique_logical_payloads: staged_unique_payloads,
        projections: staged_projections,
        resource_lane_authority: staged_resource_lane_authority,
    })
}

fn materialize_resource_lane_intents(
    base: &CommitBase,
    request: &CommitRequest,
    fingerprint: &CommitFingerprint,
) -> Result<ResourceLaneMaterialization> {
    let mut resource_lane_authority = base.resource_lane_authority.clone();
    let mut active_lanes = materialized_active_resource_lanes(&base.projections);
    let mut materialized_payloads = Vec::with_capacity(request.payloads.len());

    for payload in &request.payloads {
        match payload {
            KernelEventPayload::ResourceLaneClaimIntent(intent) => {
                require_side_effect_pair_role(
                    &intent.ledger_key,
                    &intent.ledger_purpose,
                    intent.pair_role,
                    events::SideEffectPairRole::Submit,
                )?;
                let lane_key = ResourceLaneKey::from_evidence(&intent.resource_key);
                let holder =
                    SideEffectPairLedgerRef::new(request.run_id.clone(), intent.pair_id.clone());
                if let Some(existing_key) =
                    materialized_lane_key_for_pair(&active_lanes, &request.run_id, &intent.pair_id)
                {
                    let existing = active_lanes
                        .get(&existing_key)
                        .expect("resource lane key was found from pair materialization");
                    if existing.holder != holder {
                        return Err(StoreError::ProjectionConflict {
                            key: format!("resource_lane_pair:{}", intent.pair_id),
                            message: "side-effect pair already has active resource lane".to_owned(),
                        });
                    }
                }
                if let Some((existing_key, _)) = active_lanes
                    .iter()
                    .find(|(_, active)| active.holder == holder)
                {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("resource_lane_pair:{}", intent.pair_id),
                        message: format!(
                            "side-effect holder already has active resource lane {}:{}",
                            existing_key.namespace, existing_key.key
                        ),
                    });
                }
                if let Some(existing) = active_lanes.get(&lane_key) {
                    if existing.holder != holder {
                        return Ok(ResourceLaneMaterialization::Blocked(Box::new(
                            WaitFifoAdmissionBlock {
                                resource_lane_key: lane_key,
                                holder: Some(existing.holder.clone()),
                                waiter: None,
                            },
                        )));
                    }
                }

                let authority = resource_lane_authority.entry(lane_key.clone()).or_default();
                let lane_transition_seq = checked_lane_increment(
                    authority.last_transition_seq,
                    "resource lane transition sequence",
                )?;
                let claim_fencing_token = checked_lane_increment(
                    authority.last_claim_fencing_token,
                    "resource lane fencing token",
                )?;
                let claim_id = derive_resource_lane_claim_id(
                    &request.run_id,
                    &lane_key,
                    intent,
                    claim_fencing_token,
                    lane_transition_seq,
                    fingerprint,
                )?;
                let claimed = events::ResourceLaneClaimed {
                    spec_hash: intent.spec_hash.clone(),
                    node_id: intent.node_id.clone(),
                    attempt_id: intent.attempt_id.clone(),
                    ledger_key: intent.ledger_key.clone(),
                    ledger_purpose: intent.ledger_purpose.clone(),
                    pair_id: intent.pair_id.clone(),
                    pair_role: intent.pair_role,
                    invocation_epoch: intent.invocation_epoch,
                    resource_key: intent.resource_key.clone(),
                    requirement_digest: intent.requirement_digest.clone(),
                    resolved_by_capability_impl: intent.resolved_by_capability_impl.clone(),
                    claim_id,
                    claim_fencing_token,
                    lane_transition_seq,
                };
                authority.last_transition_seq = lane_transition_seq;
                authority.last_claim_fencing_token = claim_fencing_token;
                active_lanes.insert(
                    lane_key,
                    MaterializedActiveLane::from_claimed(&request.run_id, &claimed),
                );
                materialized_payloads.push(KernelEventPayload::ResourceLaneClaimed(claimed));
            }
            KernelEventPayload::ResourceLaneReleaseIntent(intent) => {
                require_side_effect_pair_role(
                    &intent.ledger_key,
                    &intent.ledger_purpose,
                    intent.pair_role,
                    events::SideEffectPairRole::Verify,
                )?;
                let (lane_key, active) = resolve_active_resource_lane_release(
                    &active_lanes,
                    ResourceLaneReleaseMatch {
                        run_id: &request.run_id,
                        ledger_key: &intent.ledger_key,
                        ledger_purpose: &intent.ledger_purpose,
                        pair_id: &intent.pair_id,
                        invocation_epoch: intent.invocation_epoch,
                        claim_id: &intent.claim_id,
                        mismatch_message:
                            "resource lane release intent does not match active claim",
                    },
                )?;
                let lane_key = lane_key.clone();
                let active = active.clone();
                let authority = resource_lane_authority.get_mut(&lane_key).ok_or_else(|| {
                    StoreError::ProjectionConflict {
                        key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
                        message: "resource lane authority missing active claim history".to_owned(),
                    }
                })?;
                if authority.last_transition_seq < active.lane_transition_seq
                    || authority.last_claim_fencing_token < active.claim_fencing_token
                {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("resource_lane:{}:{}", lane_key.namespace, lane_key.key),
                        message: "resource lane authority is behind active claim".to_owned(),
                    });
                }
                let lane_transition_seq = checked_lane_increment(
                    authority.last_transition_seq,
                    "resource lane transition sequence",
                )?;
                let release_id = derive_resource_lane_release_id(
                    &request.run_id,
                    &lane_key,
                    intent,
                    active.claim_fencing_token,
                    lane_transition_seq,
                    fingerprint,
                )?;
                let released = events::ResourceLaneReleased {
                    spec_hash: intent.spec_hash.clone(),
                    ledger_key: intent.ledger_key.clone(),
                    ledger_purpose: intent.ledger_purpose.clone(),
                    pair_id: intent.pair_id.clone(),
                    pair_role: intent.pair_role,
                    invocation_epoch: intent.invocation_epoch,
                    claim_id: intent.claim_id.clone(),
                    release_id,
                    claim_fencing_token: active.claim_fencing_token,
                    release_authority: intent.release_authority,
                    release_reason: intent.release_reason.clone(),
                    lane_transition_seq,
                };
                authority.last_transition_seq = lane_transition_seq;
                active_lanes.remove(&lane_key);
                materialized_payloads.push(KernelEventPayload::ResourceLaneReleased(released));
            }
            KernelEventPayload::ResourceLaneClaimed(_)
            | KernelEventPayload::ResourceLaneReleased(_) => {
                return Err(StoreError::Event(
                    "prepared commits must use resource-lane intents, not store-filled lane events"
                        .to_owned(),
                ));
            }
            _ => materialized_payloads.push(payload.clone()),
        }
    }

    let request = CommitRequest::from_payloads(
        request.run_id.clone(),
        request.expected_next_seq,
        request.commit_key.clone(),
        materialized_payloads,
        request.required_artifacts.clone(),
        request.preconditions.clone(),
    )?;
    Ok(ResourceLaneMaterialization::Materialized {
        request: Box::new(request),
        resource_lane_authority,
    })
}

impl MaterializedActiveLane {
    fn from_projection(projection: &ResourceLaneProjection) -> Self {
        Self {
            holder: projection.holder.clone(),
            ledger_purpose: projection.ledger_purpose.clone(),
            node_id: projection.node_id.clone(),
            attempt_id: projection.attempt_id.clone(),
            invocation_epoch: projection.invocation_epoch,
            claim_id: projection.claim_id.clone(),
            claim_fencing_token: projection.claim_fencing_token,
            lane_transition_seq: projection.lane_transition_seq,
        }
    }

    fn from_claimed(run_id: &RunId, payload: &events::ResourceLaneClaimed) -> Self {
        Self {
            holder: SideEffectPairLedgerRef::new(run_id.clone(), payload.pair_id.clone()),
            ledger_purpose: payload.ledger_purpose.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            invocation_epoch: payload.invocation_epoch,
            claim_id: payload.claim_id.clone(),
            claim_fencing_token: payload.claim_fencing_token,
            lane_transition_seq: payload.lane_transition_seq,
        }
    }
}

fn materialized_active_resource_lanes(
    projections: &ProjectionSnapshot,
) -> BTreeMap<ResourceLaneKey, MaterializedActiveLane> {
    projections
        .resource_lanes()
        .map(|(lane_key, projection)| {
            (
                lane_key.clone(),
                MaterializedActiveLane::from_projection(projection),
            )
        })
        .collect()
}

fn materialized_lane_key_for_pair(
    active_lanes: &BTreeMap<ResourceLaneKey, MaterializedActiveLane>,
    run_id: &RunId,
    pair_id: &SideEffectPairId,
) -> Option<ResourceLaneKey> {
    active_lanes.iter().find_map(|(key, active)| {
        (active.holder.run_id == *run_id && active.holder.pair_id == *pair_id).then(|| key.clone())
    })
}

fn checked_lane_increment(value: u64, label: &'static str) -> Result<u64> {
    value
        .checked_add(1)
        .filter(|value| *value > 0)
        .ok_or_else(|| StoreError::Event(format!("{label} overflow")))
}

fn derive_resource_lane_claim_id(
    run_id: &RunId,
    lane_key: &ResourceLaneKey,
    intent: &events::ResourceLaneClaimIntent,
    claim_fencing_token: u64,
    lane_transition_seq: u64,
    fingerprint: &CommitFingerprint,
) -> Result<events::ResourceLaneClaimId> {
    let digest = canonical_json(serde_json::json!({
        "attempt_id": intent.attempt_id.as_str(),
        "claim_fencing_token": claim_fencing_token,
        "commit_fingerprint": fingerprint.as_digest().as_str(),
        "invocation_epoch": intent.invocation_epoch,
        "lane_key": {
            "key": lane_key.key.as_str(),
            "namespace": lane_key.namespace.as_str(),
        },
        "lane_transition_seq": lane_transition_seq,
        "ledger_key": intent.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&intent.ledger_purpose),
        "node_id": intent.node_id.as_str(),
        "pair_id": intent.pair_id.as_str(),
        "pair_role": intent.pair_role.as_str(),
        "run_id": run_id.as_str(),
    }))?
    .content_digest();
    Ok(events::ResourceLaneClaimId::new(format!(
        "mfm.store.lane.claim.{}",
        short_stable_id_fragment(digest.as_str(), 32)
    ))?)
}

fn derive_resource_lane_release_id(
    run_id: &RunId,
    lane_key: &ResourceLaneKey,
    intent: &events::ResourceLaneReleaseIntent,
    claim_fencing_token: u64,
    lane_transition_seq: u64,
    fingerprint: &CommitFingerprint,
) -> Result<events::ResourceLaneReleaseId> {
    let digest = canonical_json(serde_json::json!({
        "claim_fencing_token": claim_fencing_token,
        "claim_id": intent.claim_id.as_str(),
        "commit_fingerprint": fingerprint.as_digest().as_str(),
        "invocation_epoch": intent.invocation_epoch,
        "lane_key": {
            "key": lane_key.key.as_str(),
            "namespace": lane_key.namespace.as_str(),
        },
        "lane_transition_seq": lane_transition_seq,
        "ledger_key": intent.ledger_key.as_str(),
        "ledger_purpose": side_effect_ledger_purpose_json(&intent.ledger_purpose),
        "pair_id": intent.pair_id.as_str(),
        "pair_role": intent.pair_role.as_str(),
        "release_authority": intent.release_authority.as_str(),
        "release_reason": intent.release_reason.as_str(),
        "run_id": run_id.as_str(),
    }))?
    .content_digest();
    Ok(events::ResourceLaneReleaseId::new(format!(
        "mfm.store.lane.release.{}",
        short_stable_id_fragment(digest.as_str(), 32)
    ))?)
}

pub(super) fn admit_artifact_evidence(
    artifacts: &mut ArtifactAuthorityMap,
    admitted_artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    for evidence in admitted_artifacts {
        let key = artifact_authority_key(evidence)?;
        if let Some(existing) = artifacts.get(&key) {
            if existing != evidence {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: evidence.artifact_id.clone(),
                    field: "artifact",
                });
            }
            continue;
        }
        artifacts.insert(key, evidence.clone());
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn verify_existing_artifact_admissions(
    artifacts: &ArtifactAuthorityMap,
    bundle: &PreparedCommitBundle,
) -> Result<()> {
    for existing in bundle.existing_artifacts() {
        let evidence =
            admitted_artifact_evidence(bundle, existing.artifact_id(), existing.evidence_hash())?;
        let key = artifact_authority_key(evidence)?;
        match artifacts.get(&key) {
            Some(stored) if stored == evidence => {}
            Some(_) => {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: existing.artifact_id().clone(),
                    field: "artifact",
                });
            }
            None => {
                return Err(StoreError::MissingArtifact {
                    artifact_id: existing.artifact_id().clone(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn admitted_artifact_evidence<'a>(
    bundle: &'a PreparedCommitBundle,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<&'a ArtifactEvidenceRef> {
    for evidence in bundle.admitted_artifacts() {
        if &evidence.artifact_id == artifact_id && evidence.evidence_hash()? == *evidence_hash {
            return Ok(evidence);
        }
    }
    Err(StoreError::MissingPreparedArtifactBytes {
        artifact_id: artifact_id.clone(),
    })
}

/// Returns artifact-byte projection authority after applying a prepared bundle.
pub fn artifact_byte_authority_for_bundle(
    existing: &ArtifactByteAuthorityMap,
    bundle: &PreparedCommitBundle,
) -> Result<ArtifactByteAuthorityMap> {
    let mut authority = existing.clone();
    for artifact in bundle.artifact_bytes() {
        let verified =
            PreparedArtifactBytes::new(artifact.bytes().to_vec(), artifact.evidence().clone())?;
        let (bytes, evidence, evidence_hash) = verified.into_parts();
        let key = (evidence.artifact_id.clone(), evidence_hash);
        match authority.get(&key) {
            Some((stored_bytes, stored_evidence))
                if stored_bytes == &bytes && stored_evidence == &evidence => {}
            Some((_, stored_evidence)) => {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: stored_evidence.artifact_id.clone(),
                    field: "artifact",
                });
            }
            None => {
                authority.insert(key, (bytes, evidence));
            }
        }
    }
    Ok(authority)
}

pub(super) fn artifact_authority_key(
    evidence: &ArtifactEvidenceRef,
) -> Result<ArtifactAuthorityKey> {
    Ok((evidence.artifact_id.clone(), evidence.evidence_hash()?))
}

pub(super) fn compare_artifact_field<T: PartialEq>(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: T,
    expected: T,
) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact_id.clone(),
            field,
        })
    }
}

pub(super) fn compare_artifact_option(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: Option<&str>,
    expected: Option<&str>,
) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact_id.clone(),
            field,
        })
    }
}

fn side_effect_precondition_matches(
    projection: Option<&SideEffectProjection>,
    required: RequiredSideEffectState,
) -> Result<bool> {
    let state = projection
        .map(SideEffectProjection::ledger_state)
        .transpose()?;
    match required {
        RequiredSideEffectState::Absent => Ok(state.is_none()),
        RequiredSideEffectState::IntentPersisted => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::IntentPersisted { .. })
        )),
        RequiredSideEffectState::Claimed => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Claimed { .. })
        )),
        RequiredSideEffectState::InvocationPrepared => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Prepared { .. })
        )),
        RequiredSideEffectState::InvocationStarted => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Started { .. })
        )),
        RequiredSideEffectState::SubmissionResult => Ok(matches!(
            state.map(|state| state.phase()),
            Some(
                SideEffectLedgerPhase::SubmissionKnown { .. }
                    | SideEffectLedgerPhase::Ambiguous { .. }
            )
        )),
        RequiredSideEffectState::ReceiptObserved => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::ReceiptObserved { .. })
        )),
        RequiredSideEffectState::ConfirmationObserved => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Confirmed { .. })
        )),
        RequiredSideEffectState::Ambiguous => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Ambiguous { .. })
        )),
        RequiredSideEffectState::Failed => Ok(matches!(
            state.map(|state| state.phase()),
            Some(SideEffectLedgerPhase::Failed { .. })
        )),
    }
}
