use super::*;

pub(super) fn note_saga_engagement(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    engagement: SagaEngagementProjection,
) {
    projections
        .saga_engagements
        .entry(run_id.clone())
        .or_insert(engagement);
}

pub(super) fn require_forward_fence_open(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_id: &SideEffectPairId,
    pair_role: events::SideEffectPairRole,
) -> Result<()> {
    if matches!(purpose, events::SideEffectLedgerPurpose::Forward)
        && projections.saga_engagement(run_id).is_some()
    {
        if pair_role == events::SideEffectPairRole::Verify
            && projections
                .side_effect_for_pair(run_id, pair_id)
                .is_some_and(|projection| {
                    projection.pair_id == *pair_id
                        && projection.ledger_key == *ledger_key
                        && matches!(
                            projection.ledger_purpose,
                            events::SideEffectLedgerPurpose::Forward
                        )
                })
        {
            return Ok(());
        }
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{ledger_key}"),
            message: "forward side-effect boundary event rejected after saga engagement".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn require_forward_terminal_closure_admissible(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_id: &SideEffectPairId,
) -> Result<()> {
    if matches!(purpose, events::SideEffectLedgerPurpose::Forward)
        && projections.saga_engagement(run_id).is_some()
    {
        if projections
            .side_effect_for_pair(run_id, pair_id)
            .is_some_and(|projection| {
                projection.pair_id == *pair_id
                    && projection.ledger_key == *ledger_key
                    && matches!(
                        projection.ledger_purpose,
                        events::SideEffectLedgerPurpose::Forward
                    )
            })
        {
            return Ok(());
        }
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{ledger_key}"),
            message: "forward side-effect terminal event rejected after saga engagement".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn require_remediation_intent_admissible(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    remediation_ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    terminal_policies: &SideEffectTerminalPolicies,
) -> Result<()> {
    let events::SideEffectLedgerPurpose::Remediation { forward_pair_id } = purpose else {
        return Ok(());
    };
    if projections.saga_engagement(run_id).is_none() {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger requires prior saga engagement".to_owned(),
        });
    }
    let Some(forward) = projections.side_effect_for_pair(run_id, forward_pair_id) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger references missing forward pair".to_owned(),
        });
    };
    if !matches!(
        forward.ledger_purpose,
        events::SideEffectLedgerPurpose::Forward
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger references a non-forward ledger".to_owned(),
        });
    }
    if !terminal_policies
        .require(forward_pair_id)?
        .is_terminal_phase(&forward.phase)
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger requires terminal forward ledger".to_owned(),
        });
    }
    if projections.side_effects.values().any(|projection| {
        projection.run_id == *run_id
            && matches!(
                &projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Remediation {
                    forward_pair_id: linked,
                    ..
                } if *linked == *forward_pair_id
            )
    }) {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{remediation_ledger_key}"),
            message: "remediation ledger already exists for forward pair".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn require_side_effect_pair_role(
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_role: events::SideEffectPairRole,
    expected_role: events::SideEffectPairRole,
) -> Result<()> {
    match purpose {
        events::SideEffectLedgerPurpose::Forward
        | events::SideEffectLedgerPurpose::Remediation { .. } => {
            if pair_role != expected_role {
                return Err(side_effect_projection_error(
                    ledger_key,
                    "side-effect pair role does not match event phase",
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn require_side_effect_pair_consistent(
    projection: &SideEffectProjection,
    ledger_key: &events::SideEffectLedgerKey,
    pair_id: &SideEffectPairId,
) -> Result<()> {
    if projection.pair_id == *pair_id {
        Ok(())
    } else {
        Err(side_effect_projection_error(
            ledger_key,
            "side-effect pair id changed",
        ))
    }
}

pub(super) fn side_effect_projection_error(
    ledger_key: &events::SideEffectLedgerKey,
    message: impl Into<String>,
) -> StoreError {
    StoreError::ProjectionConflict {
        key: format!("sidefx:{ledger_key}"),
        message: message.into(),
    }
}

pub(super) fn require_side_effect_phase<'a>(
    projections: &'a ProjectionSnapshot,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
    pair_id: &SideEffectPairId,
    expected: &'static str,
    predicate: impl FnOnce(&SideEffectLedgerState<'_>) -> bool,
) -> Result<&'a SideEffectProjection> {
    let Some(projection) = projections.side_effect_for_pair(run_id, pair_id) else {
        return Err(side_effect_projection_error(
            ledger_key,
            format!("missing side-effect projection; expected {expected}"),
        ));
    };
    if projection.ledger_key != *ledger_key {
        return Err(side_effect_projection_error(
            ledger_key,
            "side-effect ledger key changed",
        ));
    }
    let state = projection.ledger_state()?;
    if predicate(&state) {
        Ok(projection)
    } else {
        Err(side_effect_projection_error(
            ledger_key,
            format!("illegal side-effect transition; expected {expected}"),
        ))
    }
}

pub(super) fn require_side_effect_purpose(
    projection: &SideEffectProjection,
    ledger_key: &events::SideEffectLedgerKey,
    expected: &events::SideEffectLedgerPurpose,
) -> Result<()> {
    if projection.ledger_purpose == *expected {
        Ok(())
    } else {
        Err(side_effect_projection_error(
            ledger_key,
            "side-effect ledger purpose changed",
        ))
    }
}

pub(super) fn require_active_attempt_for_side_effect(
    projections: &ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    ledger_key: &events::SideEffectLedgerKey,
) -> Result<()> {
    match projections.attempt(node_id, attempt_id) {
        Some(AttemptProjection {
            status: AttemptStatus::Started { .. },
            ..
        }) => Ok(()),
        Some(_) => Err(side_effect_projection_error(
            ledger_key,
            "side-effect event requires an active started attempt",
        )),
        None => Err(side_effect_projection_error(
            ledger_key,
            "side-effect event requires a started attempt",
        )),
    }
}

pub(super) fn prepared_invocation_projection(
    artifact_id: &Option<ArtifactId>,
    content_digest: &Option<ContentDigest>,
    evidence_hash: &Option<ContentDigest>,
    ledger_key: &events::SideEffectLedgerKey,
) -> Result<Option<SideEffectArtifactProjection>> {
    match (artifact_id, content_digest, evidence_hash) {
        (Some(artifact_id), Some(content_digest), Some(evidence_hash)) => {
            Ok(Some(SideEffectArtifactProjection {
                artifact_id: artifact_id.clone(),
                content_digest: content_digest.clone(),
                evidence_hash: evidence_hash.clone(),
                schema_id: None,
            }))
        }
        (None, None, None) => Ok(None),
        _ => Err(side_effect_projection_error(
            ledger_key,
            "prepared invocation artifact id, content hash, and evidence hash must be recorded together",
        )),
    }
}

fn phase_matches_expected(phase: &SideEffectLedgerPhase<'_>, expected: &'static str) -> bool {
    match expected {
        "started" => matches!(phase, SideEffectLedgerPhase::Started { .. }),
        "submission_recovery" => matches!(
            phase,
            SideEffectLedgerPhase::Started { .. }
                | SideEffectLedgerPhase::SubmissionKnown {
                    status: SideEffectSubmissionState::Unknown,
                    ..
                }
        ),
        "submission_observed" => {
            matches!(
                phase,
                SideEffectLedgerPhase::SubmissionKnown {
                    status: SideEffectSubmissionState::Observed { .. },
                    ..
                }
            )
        }
        "receipt" => matches!(phase, SideEffectLedgerPhase::ReceiptObserved { .. }),
        "not_submitted" => matches!(
            phase,
            SideEffectLedgerPhase::SubmissionKnown {
                status: SideEffectSubmissionState::NotSubmitted,
                ..
            }
        ),
        "ambiguity_source" => matches!(
            phase,
            SideEffectLedgerPhase::Started { .. }
                | SideEffectLedgerPhase::SubmissionKnown { .. }
                | SideEffectLedgerPhase::ReceiptObserved { .. }
        ),
        _ => false,
    }
}

pub(super) struct EpochOnlyTransition<'a> {
    pub(super) run_id: &'a RunId,
    pub(super) ledger_key: &'a events::SideEffectLedgerKey,
    pub(super) ledger_purpose: &'a events::SideEffectLedgerPurpose,
    pub(super) pair_id: &'a SideEffectPairId,
    pub(super) pair_role: events::SideEffectPairRole,
    pub(super) expected_pair_role: PairRoleRequirement,
    pub(super) required_previous: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PairRoleRequirement {
    Exact(events::SideEffectPairRole),
    SubmitOrVerify,
}

pub(super) fn transition_side_effect_epoch_only(
    projections: &mut ProjectionSnapshot,
    transition: EpochOnlyTransition<'_>,
    transition_state: impl FnOnce(OwnedSideEffectLedgerState) -> Result<OwnedSideEffectLedgerState>,
) -> Result<()> {
    require_side_effect_pair_role_requirement(
        transition.ledger_key,
        transition.ledger_purpose,
        transition.pair_role,
        transition.expected_pair_role,
    )?;
    require_forward_fence_open(
        projections,
        transition.run_id,
        transition.ledger_key,
        transition.ledger_purpose,
        transition.pair_id,
        transition.pair_role,
    )?;
    let previous = require_side_effect_phase(
        projections,
        transition.run_id,
        transition.ledger_key,
        transition.pair_id,
        transition.required_previous,
        |state| phase_matches_expected(&state.phase(), transition.required_previous),
    )?;
    require_side_effect_purpose(previous, transition.ledger_key, transition.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, transition.ledger_key, transition.pair_id)?;
    let projection = transition_state(OwnedSideEffectLedgerState::from_projection(
        previous.clone(),
    )?)?
    .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new((*transition.run_id).clone(), transition.pair_id.clone()),
        projection,
    );
    Ok(())
}

fn require_side_effect_pair_role_requirement(
    ledger_key: &events::SideEffectLedgerKey,
    purpose: &events::SideEffectLedgerPurpose,
    pair_role: events::SideEffectPairRole,
    expected_role: PairRoleRequirement,
) -> Result<()> {
    match expected_role {
        PairRoleRequirement::Exact(expected_role) => {
            require_side_effect_pair_role(ledger_key, purpose, pair_role, expected_role)
        }
        PairRoleRequirement::SubmitOrVerify => match purpose {
            events::SideEffectLedgerPurpose::Forward
            | events::SideEffectLedgerPurpose::Remediation { .. } => {
                if matches!(
                    pair_role,
                    events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
                ) {
                    Ok(())
                } else {
                    Err(side_effect_projection_error(
                        ledger_key,
                        "side-effect pair role does not match event phase",
                    ))
                }
            }
        },
    }
}

pub(super) fn transition_side_effect_failure(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    payload: &side_effect::Failed,
    event_id: EventId,
) -> Result<()> {
    require_side_effect_pair_role_requirement(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        PairRoleRequirement::SubmitOrVerify,
    )?;
    require_forward_terminal_closure_admissible(
        projections,
        run_id,
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
    )?;
    let Some(previous) = projections.side_effect_for_pair(run_id, &payload.pair_id) else {
        return Err(side_effect_projection_error(
            &payload.ledger_key,
            "missing side-effect projection",
        ));
    };
    if previous.ledger_key != payload.ledger_key {
        return Err(side_effect_projection_error(
            &payload.ledger_key,
            "side-effect ledger key changed",
        ));
    }
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .fail(event_id, payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(run_id.clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}
/// Side-effect projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectProjection {
    /// Run id that owns this ledger.
    pub run_id: RunId,
    /// Ledger key.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Ledger purpose.
    pub ledger_purpose: events::SideEffectLedgerPurpose,
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Last event id that updated this projection.
    pub event_id: EventId,
    /// Intent evidence that opened this ledger key.
    pub intent: SideEffectIntentProjection,
    /// Prepared invocation artifact evidence, when one has been recorded.
    pub prepared_invocation: Option<SideEffectArtifactProjection>,
    /// Exclusive resource key evidence recorded by `ResourceLaneClaimed`.
    pub resource_key: Option<events::ResourceKeyEvidence>,
    /// Submission artifact evidence, when one has been recorded.
    pub submission: Option<SideEffectArtifactProjection>,
    /// Receipt artifact evidence, when one has been recorded.
    pub receipt: Option<SideEffectArtifactProjection>,
    /// Confirmation artifact evidence, when one has been recorded.
    pub confirmation: Option<SideEffectArtifactProjection>,
    /// Exact touched-set evidence recorded on receipt or confirmation.
    pub resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
    /// Active claim, when one exists.
    pub claim: Option<SideEffectClaimProjection>,
    /// Current projected phase.
    pub phase: SideEffectPhase,
}

impl SideEffectProjection {
    /// Returns a read-only semantic view over this projected ledger state.
    pub fn ledger_state(&self) -> Result<SideEffectLedgerState<'_>> {
        SideEffectLedgerState::new(self)
    }
}

/// Validated read-only side-effect ledger state.
///
/// This wraps the persisted projection DTO and exposes only phase-appropriate evidence for the
/// current semantic state.
#[derive(Debug, Clone, Copy)]
pub struct SideEffectLedgerState<'a> {
    projection: &'a SideEffectProjection,
    phase: SideEffectLedgerPhase<'a>,
}

impl<'a> SideEffectLedgerState<'a> {
    fn new(projection: &'a SideEffectProjection) -> Result<Self> {
        let phase = match &projection.phase {
            SideEffectPhase::IntentPersisted { invocation_epoch } => {
                require_intent_epoch(projection, *invocation_epoch)?;
                SideEffectLedgerPhase::IntentPersisted {
                    invocation_epoch: *invocation_epoch,
                }
            }
            SideEffectPhase::Claimed {
                claim_owner,
                invocation_epoch,
                claim_generation,
                claim_fencing_token,
            } => {
                let claim = require_projected_claim(
                    projection,
                    *invocation_epoch,
                    Some(*claim_generation),
                    Some(claim_owner),
                    Some(claim_fencing_token),
                )?;
                SideEffectLedgerPhase::Claimed { claim }
            }
            SideEffectPhase::InvocationPrepared {
                invocation_epoch,
                claim_generation,
                claim_fencing_token,
            } => {
                let claim = require_projected_claim(
                    projection,
                    *invocation_epoch,
                    Some(*claim_generation),
                    None,
                    Some(claim_fencing_token),
                )?;
                SideEffectLedgerPhase::Prepared {
                    claim,
                    prepared_invocation: projection.prepared_invocation.as_ref(),
                    resource_key: projection.resource_key.as_ref(),
                }
            }
            SideEffectPhase::InvocationStarted {
                claim_owner,
                invocation_epoch,
                claim_generation,
                claim_fencing_token,
            } => {
                let claim = require_projected_claim(
                    projection,
                    *invocation_epoch,
                    Some(*claim_generation),
                    Some(claim_owner),
                    Some(claim_fencing_token),
                )?;
                SideEffectLedgerPhase::Started {
                    claim,
                    prepared_invocation: projection.prepared_invocation.as_ref(),
                    resource_key: projection.resource_key.as_ref(),
                }
            }
            SideEffectPhase::SubmissionObserved { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                let submission = require_side_effect_artifact(projection, "submission")?;
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::Observed { submission },
                }
            }
            SideEffectPhase::NotSubmittedProven { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::NotSubmitted,
                }
            }
            SideEffectPhase::SubmissionUnknown { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::Unknown,
                }
            }
            SideEffectPhase::ReceiptObserved { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                let submission = require_side_effect_artifact(projection, "submission")?;
                let receipt = projection.receipt.as_ref().ok_or_else(|| {
                    side_effect_projection_conflict(projection, "receipt evidence is missing")
                })?;
                SideEffectLedgerPhase::ReceiptObserved {
                    claim,
                    submission,
                    receipt,
                    resource_touched_set: projection.resource_touched_set.as_ref(),
                }
            }
            SideEffectPhase::ConfirmationObserved { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                let submission = require_side_effect_artifact(projection, "submission")?;
                let receipt = projection.receipt.as_ref().ok_or_else(|| {
                    side_effect_projection_conflict(projection, "receipt evidence is missing")
                })?;
                let confirmation = projection.confirmation.as_ref().ok_or_else(|| {
                    side_effect_projection_conflict(projection, "confirmation evidence is missing")
                })?;
                SideEffectLedgerPhase::Confirmed {
                    claim,
                    submission,
                    receipt,
                    confirmation,
                    resource_touched_set: projection.resource_touched_set.as_ref(),
                }
            }
            SideEffectPhase::Ambiguous { invocation_epoch } => {
                let claim =
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                SideEffectLedgerPhase::Ambiguous {
                    claim,
                    invocation_epoch: *invocation_epoch,
                    submission: projection.submission.as_ref(),
                    receipt: projection.receipt.as_ref(),
                }
            }
            SideEffectPhase::Failed {
                invocation_epoch,
                failure_phase,
            } => {
                if matches!(
                    failure_phase,
                    side_effect::FailurePhase::AfterNotSubmittedProven
                ) || projection.claim.is_some()
                {
                    require_projected_claim(projection, *invocation_epoch, None, None, None)?;
                } else {
                    require_intent_epoch(projection, *invocation_epoch)?;
                }
                SideEffectLedgerPhase::Failed {
                    invocation_epoch: *invocation_epoch,
                    failure_phase: *failure_phase,
                    claim: projection.claim.as_ref(),
                }
            }
        };
        Ok(Self { projection, phase })
    }

    /// Returns the backing persisted projection DTO.
    pub fn projection(&self) -> &'a SideEffectProjection {
        self.projection
    }

    /// Returns the validated semantic phase.
    pub fn phase(&self) -> SideEffectLedgerPhase<'a> {
        self.phase
    }

    /// Returns the ledger purpose.
    pub fn ledger_purpose(&self) -> &'a events::SideEffectLedgerPurpose {
        &self.projection.ledger_purpose
    }

    /// Returns true when this is a forward ledger that can still complete after saga engagement.
    pub fn is_forward_completion_candidate(&self) -> bool {
        matches!(
            self.ledger_purpose(),
            events::SideEffectLedgerPurpose::Forward
        ) && matches!(
            self.phase,
            SideEffectLedgerPhase::IntentPersisted { .. }
                | SideEffectLedgerPhase::Claimed { .. }
                | SideEffectLedgerPhase::Prepared { .. }
                | SideEffectLedgerPhase::Started { .. }
                | SideEffectLedgerPhase::SubmissionKnown { .. }
                | SideEffectLedgerPhase::ReceiptObserved { .. }
                | SideEffectLedgerPhase::Confirmed { .. }
        )
    }

    /// Returns true when this phase is ambiguous.
    pub fn is_ambiguous(&self) -> bool {
        matches!(self.phase, SideEffectLedgerPhase::Ambiguous { .. })
    }

    /// Returns true when this phase is failed.
    pub fn is_failed(&self) -> bool {
        matches!(self.phase, SideEffectLedgerPhase::Failed { .. })
    }

    /// Returns true when this phase has observed confirmation.
    pub fn is_confirmed(&self) -> bool {
        matches!(self.phase, SideEffectLedgerPhase::Confirmed { .. })
    }
}

/// Validated side-effect ledger phase.
#[derive(Debug, Clone, Copy)]
pub enum SideEffectLedgerPhase<'a> {
    /// Intent has been persisted and no active claim exists.
    IntentPersisted {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// A claim is active.
    Claimed {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
    },
    /// Invocation preparation was recorded.
    Prepared {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Prepared invocation artifact evidence, when the ledger carries one.
        prepared_invocation: Option<&'a SideEffectArtifactProjection>,
        /// Exclusive resource key evidence, when the ledger carries one.
        resource_key: Option<&'a events::ResourceKeyEvidence>,
    },
    /// Invocation start was recorded.
    Started {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Prepared invocation artifact evidence, when the ledger carries one.
        prepared_invocation: Option<&'a SideEffectArtifactProjection>,
        /// Exclusive resource key evidence, when the ledger carries one.
        resource_key: Option<&'a events::ResourceKeyEvidence>,
    },
    /// Submission result is known for the current epoch.
    SubmissionKnown {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Submission result status.
        status: SideEffectSubmissionState<'a>,
    },
    /// Receipt was observed after a submission artifact.
    ReceiptObserved {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Submission artifact evidence.
        submission: &'a SideEffectArtifactProjection,
        /// Receipt artifact evidence.
        receipt: &'a SideEffectArtifactProjection,
        /// Exact touched-set evidence, when present.
        resource_touched_set: Option<&'a events::ResourceTouchedSetEvidence>,
    },
    /// Confirmation was observed after receipt.
    Confirmed {
        /// Active claim.
        claim: &'a SideEffectClaimProjection,
        /// Submission artifact evidence.
        submission: &'a SideEffectArtifactProjection,
        /// Receipt artifact evidence.
        receipt: &'a SideEffectArtifactProjection,
        /// Confirmation artifact evidence.
        confirmation: &'a SideEffectArtifactProjection,
        /// Exact touched-set evidence, when present.
        resource_touched_set: Option<&'a events::ResourceTouchedSetEvidence>,
    },
    /// Terminal ambiguity was recorded.
    Ambiguous {
        /// Claim retained from the ambiguity source state.
        claim: &'a SideEffectClaimProjection,
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Submission artifact evidence retained from the source state, when any.
        submission: Option<&'a SideEffectArtifactProjection>,
        /// Receipt artifact evidence retained from the source state, when any.
        receipt: Option<&'a SideEffectArtifactProjection>,
    },
    /// Terminal failure was recorded.
    Failed {
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Failure phase.
        failure_phase: side_effect::FailurePhase,
        /// Claim retained from the source state, when one existed.
        claim: Option<&'a SideEffectClaimProjection>,
    },
}

impl SideEffectLedgerPhase<'_> {
    /// Returns the invocation epoch for this phase.
    pub fn invocation_epoch(&self) -> u32 {
        match self {
            Self::IntentPersisted { invocation_epoch }
            | Self::Failed {
                invocation_epoch, ..
            } => *invocation_epoch,
            Self::Claimed { claim }
            | Self::Prepared { claim, .. }
            | Self::Started { claim, .. }
            | Self::SubmissionKnown { claim, .. }
            | Self::ReceiptObserved { claim, .. }
            | Self::Confirmed { claim, .. }
            | Self::Ambiguous { claim, .. } => claim.invocation_epoch,
        }
    }
}

/// Validated submission-result state for a side-effect ledger epoch.
#[derive(Debug, Clone, Copy)]
pub enum SideEffectSubmissionState<'a> {
    /// Submission artifact was observed.
    Observed {
        /// Submission artifact evidence.
        submission: &'a SideEffectArtifactProjection,
    },
    /// A proof established that submission did not happen.
    NotSubmitted,
    /// Submission outcome remains unknown.
    Unknown,
}

fn require_intent_epoch(projection: &SideEffectProjection, invocation_epoch: u32) -> Result<()> {
    if projection.intent.invocation_epoch == invocation_epoch {
        Ok(())
    } else {
        Err(side_effect_projection_conflict(
            projection,
            "phase invocation epoch does not match intent",
        ))
    }
}

fn require_projected_claim<'a>(
    projection: &'a SideEffectProjection,
    invocation_epoch: u32,
    claim_generation: Option<u32>,
    claim_owner: Option<&events::RunnerInvocationId>,
    claim_fencing_token: Option<&side_effect::ClaimFencingToken>,
) -> Result<&'a SideEffectClaimProjection> {
    let claim = projection.claim.as_ref().ok_or_else(|| {
        side_effect_projection_conflict(projection, "phase requires active claim")
    })?;
    if claim.node_id != projection.intent.node_id
        || claim.attempt_id != projection.intent.attempt_id
        || claim.invocation_epoch != invocation_epoch
        || claim_generation.is_some_and(|expected| claim.claim_generation != expected)
        || claim_owner.is_some_and(|expected| &claim.claim_owner != expected)
        || claim_fencing_token.is_some_and(|expected| &claim.claim_fencing_token != expected)
    {
        return Err(side_effect_projection_conflict(
            projection,
            "active claim does not match projected phase",
        ));
    }
    Ok(claim)
}

fn require_side_effect_artifact<'a>(
    projection: &'a SideEffectProjection,
    label: &'static str,
) -> Result<&'a SideEffectArtifactProjection> {
    match label {
        "submission" => projection.submission.as_ref(),
        _ => None,
    }
    .ok_or_else(|| {
        side_effect_projection_conflict(projection, format!("{label} evidence is missing"))
    })
}

fn side_effect_projection_conflict(
    projection: &SideEffectProjection,
    message: impl Into<String>,
) -> StoreError {
    StoreError::ProjectionConflict {
        key: format!("sidefx_pair:{}", projection.pair_id),
        message: message.into(),
    }
}

#[derive(Debug, Clone)]
pub(super) struct OwnedSideEffectLedgerState {
    core: SideEffectLedgerCore,
    retained: SideEffectLedgerRetained,
    phase: OwnedSideEffectLedgerPhase,
}

struct SubmissionResultContext<'a> {
    event_id: EventId,
    ledger_key: &'a events::SideEffectLedgerKey,
    ledger_purpose: &'a events::SideEffectLedgerPurpose,
    pair_role: events::SideEffectPairRole,
    node_id: &'a NodeId,
    attempt_id: &'a AttemptId,
    invocation_epoch: u32,
}

#[derive(Debug, Clone)]
struct SideEffectLedgerCore {
    run_id: RunId,
    ledger_key: events::SideEffectLedgerKey,
    ledger_purpose: events::SideEffectLedgerPurpose,
    pair_id: SideEffectPairId,
    event_id: EventId,
    intent: SideEffectIntentProjection,
}

#[derive(Debug, Clone, Default)]
struct SideEffectLedgerRetained {
    prepared_invocation: Option<SideEffectArtifactProjection>,
    resource_key: Option<events::ResourceKeyEvidence>,
    submission: Option<SideEffectArtifactProjection>,
    receipt: Option<SideEffectArtifactProjection>,
    confirmation: Option<SideEffectArtifactProjection>,
    resource_touched_set: Option<events::ResourceTouchedSetEvidence>,
}

#[derive(Debug, Clone)]
enum OwnedSideEffectLedgerPhase {
    IntentPersisted {
        invocation_epoch: u32,
    },
    Claimed {
        claim: SideEffectClaimProjection,
    },
    Prepared {
        claim: SideEffectClaimProjection,
    },
    Started {
        claim: SideEffectClaimProjection,
    },
    SubmissionObserved {
        claim: SideEffectClaimProjection,
    },
    NotSubmitted {
        claim: SideEffectClaimProjection,
    },
    SubmissionUnknown {
        claim: SideEffectClaimProjection,
    },
    ReceiptObserved {
        claim: SideEffectClaimProjection,
    },
    Confirmed {
        claim: SideEffectClaimProjection,
    },
    Ambiguous {
        claim: SideEffectClaimProjection,
        invocation_epoch: u32,
    },
    Failed {
        claim: Option<SideEffectClaimProjection>,
        invocation_epoch: u32,
        failure_phase: side_effect::FailurePhase,
    },
}

impl OwnedSideEffectLedgerState {
    pub(super) fn intent_persisted(
        run_id: RunId,
        ledger_key: events::SideEffectLedgerKey,
        ledger_purpose: events::SideEffectLedgerPurpose,
        pair_id: SideEffectPairId,
        event_id: EventId,
        intent: SideEffectIntentProjection,
        invocation_epoch: u32,
    ) -> Self {
        Self {
            core: SideEffectLedgerCore {
                run_id,
                ledger_key,
                ledger_purpose,
                pair_id,
                event_id,
                intent,
            },
            retained: SideEffectLedgerRetained::default(),
            phase: OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch },
        }
    }

    pub(super) fn from_projection(projection: SideEffectProjection) -> Result<Self> {
        projection.ledger_state()?;
        let core = SideEffectLedgerCore {
            run_id: projection.run_id,
            ledger_key: projection.ledger_key,
            ledger_purpose: projection.ledger_purpose,
            pair_id: projection.pair_id,
            event_id: projection.event_id,
            intent: projection.intent,
        };
        let retained = SideEffectLedgerRetained {
            prepared_invocation: projection.prepared_invocation,
            resource_key: projection.resource_key,
            submission: projection.submission,
            receipt: projection.receipt,
            confirmation: projection.confirmation,
            resource_touched_set: projection.resource_touched_set,
        };
        let phase = match projection.phase {
            SideEffectPhase::IntentPersisted { invocation_epoch } => {
                OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch }
            }
            SideEffectPhase::Claimed { .. } => OwnedSideEffectLedgerPhase::Claimed {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::InvocationPrepared { .. } => OwnedSideEffectLedgerPhase::Prepared {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::InvocationStarted { .. } => OwnedSideEffectLedgerPhase::Started {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::SubmissionObserved { .. } => {
                OwnedSideEffectLedgerPhase::SubmissionObserved {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::NotSubmittedProven { .. } => {
                OwnedSideEffectLedgerPhase::NotSubmitted {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::SubmissionUnknown { .. } => {
                OwnedSideEffectLedgerPhase::SubmissionUnknown {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::ReceiptObserved { .. } => {
                OwnedSideEffectLedgerPhase::ReceiptObserved {
                    claim: required_owned_claim(projection.claim)?,
                }
            }
            SideEffectPhase::ConfirmationObserved { .. } => OwnedSideEffectLedgerPhase::Confirmed {
                claim: required_owned_claim(projection.claim)?,
            },
            SideEffectPhase::Ambiguous { invocation_epoch } => {
                OwnedSideEffectLedgerPhase::Ambiguous {
                    claim: required_owned_claim(projection.claim)?,
                    invocation_epoch,
                }
            }
            SideEffectPhase::Failed {
                invocation_epoch,
                failure_phase,
            } => OwnedSideEffectLedgerPhase::Failed {
                claim: projection.claim,
                invocation_epoch,
                failure_phase,
            },
        };
        Ok(Self {
            core,
            retained,
            phase,
        })
    }

    pub(super) fn into_projection(self) -> SideEffectProjection {
        let (claim, phase) = self.phase.into_projection_parts();
        SideEffectProjection {
            run_id: self.core.run_id,
            ledger_key: self.core.ledger_key,
            ledger_purpose: self.core.ledger_purpose,
            pair_id: self.core.pair_id,
            event_id: self.core.event_id,
            intent: self.core.intent,
            prepared_invocation: self.retained.prepared_invocation,
            resource_key: self.retained.resource_key,
            submission: self.retained.submission,
            receipt: self.retained.receipt,
            confirmation: self.retained.confirmation,
            resource_touched_set: self.retained.resource_touched_set,
            claim,
            phase,
        }
    }

    pub(super) fn claim(
        mut self,
        event_id: EventId,
        payload: &side_effect::Claimed,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let next_claim = match &self.phase {
            OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch } => {
                if payload.invocation_epoch != *invocation_epoch {
                    return Err(self.error("initial claim invocation epoch does not match intent"));
                }
                self.require_intent_context(
                    &payload.node_id,
                    &payload.attempt_id,
                    payload.invocation_epoch,
                )?;
                side_effect_claim_projection(
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.claim_owner,
                    payload.invocation_epoch,
                    payload.claim_generation,
                    &payload.claim_fencing_token,
                )
            }
            OwnedSideEffectLedgerPhase::NotSubmitted { claim } => {
                self.require_intent_attempt_context(&payload.node_id, &payload.attempt_id)?;
                if payload.claim_generation <= claim.claim_generation {
                    return Err(self.error("retry claim generation must increase"));
                }
                if payload.claim_fencing_token == claim.claim_fencing_token {
                    return Err(self.error("retry claim fencing token must change"));
                }
                let next_epoch = claim
                    .invocation_epoch
                    .checked_add(1)
                    .ok_or_else(|| self.error("invocation epoch overflow"))?;
                if payload.invocation_epoch != next_epoch {
                    return Err(self.error("retry claim must advance to the next invocation epoch"));
                }
                side_effect_claim_projection(
                    &payload.node_id,
                    &payload.attempt_id,
                    &payload.claim_owner,
                    payload.invocation_epoch,
                    payload.claim_generation,
                    &payload.claim_fencing_token,
                )
            }
            _ => return Err(self.error("claim requires intent or not-submitted phase")),
        };
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Claimed { claim: next_claim };
        Ok(self)
    }

    pub(super) fn take_over(
        mut self,
        event_id: EventId,
        payload: &side_effect::ClaimTakenOver,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_claimed_or_prepared_claim()?;
        if claim.node_id != payload.node_id
            || claim.attempt_id != payload.attempt_id
            || claim.claim_owner != payload.previous_claim_owner
            || claim.claim_generation != payload.previous_claim_generation
            || claim.invocation_epoch != payload.invocation_epoch
        {
            return Err(self.error("takeover claim context does not match active claim"));
        }
        if payload.claim_generation <= payload.previous_claim_generation {
            return Err(self.error("takeover claim generation must increase"));
        }
        if payload.claim_fencing_token == claim.claim_fencing_token {
            return Err(self.error("takeover claim fencing token must change"));
        }
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Claimed {
            claim: side_effect_claim_projection(
                &payload.node_id,
                &payload.attempt_id,
                &payload.new_claim_owner,
                payload.invocation_epoch,
                payload.claim_generation,
                &payload.claim_fencing_token,
            ),
        };
        Ok(self)
    }

    pub(super) fn prepare(
        mut self,
        event_id: EventId,
        payload: &side_effect::InvocationPrepared,
        prepared_invocation: Option<SideEffectArtifactProjection>,
        resource_key: Option<events::ResourceKeyEvidence>,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("claim")?.clone();
        require_claim_context_for_payload(
            &self.core.ledger_key,
            &claim,
            &payload.node_id,
            &payload.attempt_id,
            payload.invocation_epoch,
            payload.claim_generation,
            &payload.claim_fencing_token,
            None,
        )?;
        self.core.event_id = event_id;
        self.retained.prepared_invocation =
            prepared_invocation.or(self.retained.prepared_invocation);
        self.retained.resource_key = resource_key;
        self.phase = OwnedSideEffectLedgerPhase::Prepared { claim };
        Ok(self)
    }

    pub(super) fn start(
        mut self,
        event_id: EventId,
        payload: &side_effect::InvocationStarted,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("prepared")?.clone();
        require_claim_context_for_payload(
            &self.core.ledger_key,
            &claim,
            &payload.node_id,
            &payload.attempt_id,
            payload.invocation_epoch,
            payload.claim_generation,
            &payload.claim_fencing_token,
            Some(&payload.claim_owner),
        )?;
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Started { claim };
        Ok(self)
    }

    pub(super) fn mark_not_submitted(
        self,
        event_id: EventId,
        payload: &side_effect::NotSubmittedProven,
    ) -> Result<Self> {
        self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::NotSubmitted {
                claim: claim.clone(),
            },
        )
    }

    pub(super) fn record_submission(
        mut self,
        event_id: EventId,
        payload: &side_effect::SubmissionObserved,
    ) -> Result<Self> {
        let next = self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::SubmissionObserved {
                claim: claim.clone(),
            },
        )?;
        self = next;
        self.retained.submission = Some(SideEffectArtifactProjection {
            artifact_id: payload.submission_artifact_id.clone(),
            content_digest: payload.submission_hash.clone(),
            evidence_hash: payload.submission_artifact_evidence_hash.clone(),
            schema_id: Some(payload.submission_schema_id.clone()),
        });
        Ok(self)
    }

    pub(super) fn mark_submission_unknown(
        self,
        event_id: EventId,
        payload: &side_effect::SubmissionUnknown,
    ) -> Result<Self> {
        self.submission_result(
            SubmissionResultContext {
                event_id,
                ledger_key: &payload.ledger_key,
                ledger_purpose: &payload.ledger_purpose,
                pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
            |claim| OwnedSideEffectLedgerPhase::SubmissionUnknown {
                claim: claim.clone(),
            },
        )
    }

    pub(super) fn record_receipt(
        mut self,
        event_id: EventId,
        payload: &side_effect::ReceiptObserved,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_observed_submission_claim()?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.retained.receipt = Some(SideEffectArtifactProjection {
            artifact_id: payload.receipt_artifact_id.clone(),
            content_digest: payload.receipt_hash.clone(),
            evidence_hash: payload.receipt_artifact_evidence_hash.clone(),
            schema_id: Some(payload.receipt_schema_id.clone()),
        });
        if let Some(touched_set) = payload.resource_touched_set.clone() {
            self.retained.resource_touched_set = Some(touched_set);
        }
        self.phase = OwnedSideEffectLedgerPhase::ReceiptObserved { claim };
        Ok(self)
    }

    pub(super) fn confirm(
        mut self,
        event_id: EventId,
        payload: &side_effect::ConfirmationObserved,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_phase_claim("receipt")?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.retained.confirmation = Some(SideEffectArtifactProjection {
            artifact_id: payload.confirmation_artifact_id.clone(),
            content_digest: payload.confirmation_hash.clone(),
            evidence_hash: payload.confirmation_artifact_evidence_hash.clone(),
            schema_id: Some(payload.confirmation_schema_id.clone()),
        });
        if let Some(touched_set) = payload.resource_touched_set.clone() {
            self.retained.resource_touched_set = Some(touched_set);
        }
        self.phase = OwnedSideEffectLedgerPhase::Confirmed { claim };
        Ok(self)
    }

    pub(super) fn mark_ambiguous(
        mut self,
        event_id: EventId,
        payload: &side_effect::Ambiguous,
    ) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = self.require_ambiguity_source_claim()?.clone();
        require_claim_or_verify_context_for_observation(
            &self.core.ledger_key,
            &claim,
            ObservationClaimContext {
                ledger_pair_id: &self.core.pair_id,
                payload_pair_id: &payload.pair_id,
                payload_pair_role: payload.pair_role,
                node_id: &payload.node_id,
                attempt_id: &payload.attempt_id,
                invocation_epoch: payload.invocation_epoch,
            },
        )?;
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Ambiguous {
            claim,
            invocation_epoch: payload.invocation_epoch,
        };
        Ok(self)
    }

    fn fail(mut self, event_id: EventId, payload: &side_effect::Failed) -> Result<Self> {
        self.require_purpose(&payload.ledger_key, &payload.ledger_purpose)?;
        let claim = match payload.failure_phase {
            side_effect::FailurePhase::BeforeInvocationStarted => match &self.phase {
                OwnedSideEffectLedgerPhase::IntentPersisted { invocation_epoch } => {
                    if payload.invocation_epoch != *invocation_epoch {
                        return Err(self.error("failure invocation epoch does not match intent"));
                    }
                    self.require_intent_context(
                        &payload.node_id,
                        &payload.attempt_id,
                        payload.invocation_epoch,
                    )?;
                    None
                }
                OwnedSideEffectLedgerPhase::Claimed { claim }
                | OwnedSideEffectLedgerPhase::Prepared { claim } => {
                    require_claim_or_verify_context_for_observation(
                        &self.core.ledger_key,
                        claim,
                        ObservationClaimContext {
                            ledger_pair_id: &self.core.pair_id,
                            payload_pair_id: &payload.pair_id,
                            payload_pair_role: payload.pair_role,
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                        },
                    )?;
                    Some(claim.clone())
                }
                _ => {
                    return Err(self
                        .error("before-start failure requires intent, claim, or prepared phase"));
                }
            },
            side_effect::FailurePhase::AfterNotSubmittedProven => match &self.phase {
                OwnedSideEffectLedgerPhase::NotSubmitted { claim } => {
                    require_claim_or_verify_context_for_observation(
                        &self.core.ledger_key,
                        claim,
                        ObservationClaimContext {
                            ledger_pair_id: &self.core.pair_id,
                            payload_pair_id: &payload.pair_id,
                            payload_pair_role: payload.pair_role,
                            node_id: &payload.node_id,
                            attempt_id: &payload.attempt_id,
                            invocation_epoch: payload.invocation_epoch,
                        },
                    )?;
                    Some(claim.clone())
                }
                _ => {
                    return Err(
                        self.error("after-not-submitted failure requires not-submitted phase")
                    );
                }
            },
        };
        self.core.event_id = event_id;
        self.phase = OwnedSideEffectLedgerPhase::Failed {
            claim,
            invocation_epoch: payload.invocation_epoch,
            failure_phase: payload.failure_phase,
        };
        Ok(self)
    }

    fn submission_result(
        mut self,
        context: SubmissionResultContext<'_>,
        next_phase: impl FnOnce(&SideEffectClaimProjection) -> OwnedSideEffectLedgerPhase,
    ) -> Result<Self> {
        self.require_purpose(context.ledger_key, context.ledger_purpose)?;
        let claim = self
            .require_submission_recovery_claim_for_role(context.pair_role)?
            .clone();
        match context.pair_role {
            events::SideEffectPairRole::Submit => {
                require_claim_identity(
                    &self.core.ledger_key,
                    &claim,
                    context.node_id,
                    context.attempt_id,
                    context.invocation_epoch,
                )?;
            }
            events::SideEffectPairRole::Verify => {
                if claim.invocation_epoch != context.invocation_epoch {
                    return Err(
                        self.error("verify recovery invocation epoch does not match active claim")
                    );
                }
            }
        }
        self.core.event_id = context.event_id;
        self.phase = next_phase(&claim);
        Ok(self)
    }

    fn require_purpose(
        &self,
        ledger_key: &events::SideEffectLedgerKey,
        purpose: &events::SideEffectLedgerPurpose,
    ) -> Result<()> {
        if self.core.ledger_key != *ledger_key {
            return Err(self.error("side-effect ledger key changed"));
        }
        if self.core.ledger_purpose != *purpose {
            return Err(self.error("side-effect ledger purpose changed"));
        }
        Ok(())
    }

    fn require_intent_context(
        &self,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        invocation_epoch: u32,
    ) -> Result<()> {
        self.require_intent_attempt_context(node_id, attempt_id)?;
        if self.core.intent.invocation_epoch != invocation_epoch {
            return Err(self.error("invocation epoch does not match intent projection"));
        }
        Ok(())
    }

    fn require_intent_attempt_context(
        &self,
        node_id: &NodeId,
        attempt_id: &AttemptId,
    ) -> Result<()> {
        if self.core.intent.node_id != *node_id {
            return Err(self.error("node id does not match intent projection"));
        }
        if self.core.intent.attempt_id != *attempt_id {
            return Err(self.error("attempt id does not match intent projection"));
        }
        Ok(())
    }

    fn require_phase_claim(&self, expected: &'static str) -> Result<&SideEffectClaimProjection> {
        match (&self.phase, expected) {
            (OwnedSideEffectLedgerPhase::Claimed { claim }, "claim")
            | (OwnedSideEffectLedgerPhase::Prepared { claim }, "prepared")
            | (OwnedSideEffectLedgerPhase::ReceiptObserved { claim }, "receipt") => Ok(claim),
            _ => Err(self.error(format!(
                "illegal side-effect transition; expected {expected}"
            ))),
        }
    }

    fn require_claimed_or_prepared_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Claimed { claim }
            | OwnedSideEffectLedgerPhase::Prepared { claim } => Ok(claim),
            _ => Err(self.error("takeover requires claim or prepared phase")),
        }
    }

    fn require_submission_recovery_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Started { claim }
            | OwnedSideEffectLedgerPhase::SubmissionUnknown { claim } => Ok(claim),
            _ => Err(self.error("submission recovery requires started or unknown phase")),
        }
    }

    fn require_submission_recovery_claim_for_role(
        &self,
        pair_role: events::SideEffectPairRole,
    ) -> Result<&SideEffectClaimProjection> {
        match pair_role {
            events::SideEffectPairRole::Submit => self.require_submission_recovery_claim(),
            events::SideEffectPairRole::Verify => match &self.phase {
                OwnedSideEffectLedgerPhase::SubmissionUnknown { claim } => Ok(claim),
                _ => Err(self.error("verify submission recovery requires unknown phase")),
            },
        }
    }

    fn require_observed_submission_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::SubmissionObserved { claim } => {
                if self.retained.submission.is_none() {
                    return Err(self.error("submission evidence is missing"));
                }
                Ok(claim)
            }
            _ => Err(self.error("receipt requires submission-observed phase")),
        }
    }

    fn require_ambiguity_source_claim(&self) -> Result<&SideEffectClaimProjection> {
        match &self.phase {
            OwnedSideEffectLedgerPhase::Started { claim }
            | OwnedSideEffectLedgerPhase::SubmissionUnknown { claim }
            | OwnedSideEffectLedgerPhase::SubmissionObserved { claim }
            | OwnedSideEffectLedgerPhase::ReceiptObserved { claim } => Ok(claim),
            _ => {
                Err(self.error("ambiguity requires started, unknown, submitted, or receipt phase"))
            }
        }
    }

    fn error(&self, message: impl Into<String>) -> StoreError {
        StoreError::ProjectionConflict {
            key: format!("sidefx_pair:{}", self.core.pair_id),
            message: message.into(),
        }
    }
}

impl OwnedSideEffectLedgerPhase {
    fn into_projection_parts(self) -> (Option<SideEffectClaimProjection>, SideEffectPhase) {
        match self {
            Self::IntentPersisted { invocation_epoch } => {
                (None, SideEffectPhase::IntentPersisted { invocation_epoch })
            }
            Self::Claimed { claim } => (
                Some(claim.clone()),
                SideEffectPhase::Claimed {
                    claim_owner: claim.claim_owner,
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::Prepared { claim } => (
                Some(claim.clone()),
                SideEffectPhase::InvocationPrepared {
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::Started { claim } => (
                Some(claim.clone()),
                SideEffectPhase::InvocationStarted {
                    claim_owner: claim.claim_owner,
                    invocation_epoch: claim.invocation_epoch,
                    claim_generation: claim.claim_generation,
                    claim_fencing_token: claim.claim_fencing_token,
                },
            ),
            Self::SubmissionObserved { claim } => (
                Some(claim.clone()),
                SideEffectPhase::SubmissionObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::NotSubmitted { claim } => (
                Some(claim.clone()),
                SideEffectPhase::NotSubmittedProven {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::SubmissionUnknown { claim } => (
                Some(claim.clone()),
                SideEffectPhase::SubmissionUnknown {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::ReceiptObserved { claim } => (
                Some(claim.clone()),
                SideEffectPhase::ReceiptObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::Confirmed { claim } => (
                Some(claim.clone()),
                SideEffectPhase::ConfirmationObserved {
                    invocation_epoch: claim.invocation_epoch,
                },
            ),
            Self::Ambiguous {
                claim,
                invocation_epoch,
            } => (Some(claim), SideEffectPhase::Ambiguous { invocation_epoch }),
            Self::Failed {
                claim,
                invocation_epoch,
                failure_phase,
            } => (
                claim,
                SideEffectPhase::Failed {
                    invocation_epoch,
                    failure_phase,
                },
            ),
        }
    }
}

fn required_owned_claim(
    claim: Option<SideEffectClaimProjection>,
) -> Result<SideEffectClaimProjection> {
    claim.ok_or_else(|| StoreError::ProjectionConflict {
        key: "sidefx:owned".to_owned(),
        message: "phase requires active claim".to_owned(),
    })
}

fn side_effect_claim_projection(
    node_id: &NodeId,
    attempt_id: &AttemptId,
    claim_owner: &events::RunnerInvocationId,
    invocation_epoch: u32,
    claim_generation: u32,
    claim_fencing_token: &side_effect::ClaimFencingToken,
) -> SideEffectClaimProjection {
    SideEffectClaimProjection {
        node_id: node_id.clone(),
        attempt_id: attempt_id.clone(),
        claim_owner: claim_owner.clone(),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: claim_fencing_token.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn require_claim_context_for_payload(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
    claim_generation: u32,
    claim_fencing_token: &side_effect::ClaimFencingToken,
    claim_owner: Option<&events::RunnerInvocationId>,
) -> Result<()> {
    require_claim_identity(ledger_key, claim, node_id, attempt_id, invocation_epoch)?;
    if claim.claim_generation != claim_generation {
        return Err(side_effect_key_conflict(
            ledger_key,
            "claim generation does not match active claim",
        ));
    }
    if claim.claim_fencing_token != *claim_fencing_token {
        return Err(side_effect_key_conflict(
            ledger_key,
            "claim fencing token does not match active claim",
        ));
    }
    if claim_owner.is_some_and(|owner| &claim.claim_owner != owner) {
        return Err(side_effect_key_conflict(
            ledger_key,
            "claim owner does not match active claim",
        ));
    }
    Ok(())
}

struct ObservationClaimContext<'a> {
    ledger_pair_id: &'a SideEffectPairId,
    payload_pair_id: &'a SideEffectPairId,
    payload_pair_role: events::SideEffectPairRole,
    node_id: &'a NodeId,
    attempt_id: &'a AttemptId,
    invocation_epoch: u32,
}

fn require_claim_or_verify_context_for_observation(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    context: ObservationClaimContext<'_>,
) -> Result<()> {
    if context.payload_pair_role == events::SideEffectPairRole::Verify
        && context.ledger_pair_id == context.payload_pair_id
    {
        if claim.invocation_epoch != context.invocation_epoch {
            return Err(side_effect_key_conflict(
                ledger_key,
                "invocation epoch does not match active claim",
            ));
        }
        return Ok(());
    }
    require_claim_context_for_payload(
        ledger_key,
        claim,
        context.node_id,
        context.attempt_id,
        context.invocation_epoch,
        claim.claim_generation,
        &claim.claim_fencing_token,
        Some(&claim.claim_owner),
    )
}

fn require_claim_identity(
    ledger_key: &events::SideEffectLedgerKey,
    claim: &SideEffectClaimProjection,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) -> Result<()> {
    if claim.node_id != *node_id {
        return Err(side_effect_key_conflict(
            ledger_key,
            "node id does not match active claim",
        ));
    }
    if claim.attempt_id != *attempt_id {
        return Err(side_effect_key_conflict(
            ledger_key,
            "attempt id does not match active claim",
        ));
    }
    if claim.invocation_epoch != invocation_epoch {
        return Err(side_effect_key_conflict(
            ledger_key,
            "invocation epoch does not match active claim",
        ));
    }
    Ok(())
}

fn side_effect_key_conflict(
    ledger_key: &events::SideEffectLedgerKey,
    message: impl Into<String>,
) -> StoreError {
    StoreError::ProjectionConflict {
        key: format!("sidefx:{ledger_key}"),
        message: message.into(),
    }
}

/// Durable identity for one certified side-effect pair within a run.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SideEffectPairLedgerRef {
    /// Run id that owns the pair.
    pub run_id: RunId,
    /// Certified side-effect pair id.
    pub pair_id: SideEffectPairId,
}

impl SideEffectPairLedgerRef {
    /// Creates a side-effect pair ledger reference.
    pub fn new(run_id: RunId, pair_id: SideEffectPairId) -> Self {
        Self { run_id, pair_id }
    }
}

/// Side-effect artifact evidence retained by the projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectArtifactProjection {
    /// Artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical content digest.
    pub content_digest: ContentDigest,
    /// Exact retained-artifact evidence identity.
    pub evidence_hash: ContentDigest,
    /// Schema id, when the artifact is a typed value.
    pub schema_id: Option<SchemaId>,
}

/// Side-effect intent evidence projected from the authoritative run stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectIntentProjection {
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Scope id.
    pub scope_id: ScopeId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Intent schema id.
    pub intent_schema_id: SchemaId,
    /// Intent hash.
    pub intent_hash: ContentDigest,
    /// Intent artifact id.
    pub intent_artifact_id: ArtifactId,
    /// Idempotency input schema id.
    pub idempotency_input_schema_id: SchemaId,
    /// Idempotency input hash.
    pub idempotency_input_hash: ContentDigest,
    /// Idempotency key.
    pub idempotency_key: events::IdempotencyKeyRef,
    /// Capability kind.
    pub capability_kind: CapabilityKind,
    /// Capability version.
    pub capability_version: CapabilityVersion,
    /// Adapter kind.
    pub adapter_kind: AdapterKind,
    /// Adapter version.
    pub adapter_version: AdapterVersion,
}

/// Active side-effect claim evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectClaimProjection {
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Claim owner.
    pub claim_owner: events::RunnerInvocationId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: side_effect::ClaimFencingToken,
}

/// Side-effect projected phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideEffectPhase {
    /// Intent persisted.
    IntentPersisted {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Claim acquired.
    Claimed {
        /// Claim owner.
        claim_owner: events::RunnerInvocationId,
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Claim generation.
        claim_generation: u32,
        /// Fencing token.
        claim_fencing_token: side_effect::ClaimFencingToken,
    },
    /// Invocation prepared.
    InvocationPrepared {
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Claim generation.
        claim_generation: u32,
        /// Fencing token.
        claim_fencing_token: side_effect::ClaimFencingToken,
    },
    /// Invocation started.
    InvocationStarted {
        /// Claim owner.
        claim_owner: events::RunnerInvocationId,
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Claim generation.
        claim_generation: u32,
        /// Fencing token.
        claim_fencing_token: side_effect::ClaimFencingToken,
    },
    /// Submission was observed.
    SubmissionObserved {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Not-submitted proof was persisted.
    NotSubmittedProven {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Submission status is unknown.
    SubmissionUnknown {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Receipt was observed.
    ReceiptObserved {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Confirmation was observed.
    ConfirmationObserved {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Side effect is ambiguous.
    Ambiguous {
        /// Invocation epoch.
        invocation_epoch: u32,
    },
    /// Side effect failed.
    Failed {
        /// Invocation epoch.
        invocation_epoch: u32,
        /// Failure phase.
        failure_phase: side_effect::FailurePhase,
    },
}

impl SideEffectPhase {
    /// Returns the canonical snake-case tag for this side-effect phase.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::IntentPersisted { .. } => "intent_persisted",
            Self::Claimed { .. } => "claimed",
            Self::InvocationPrepared { .. } => "invocation_prepared",
            Self::InvocationStarted { .. } => "invocation_started",
            Self::SubmissionObserved { .. } => "submission_observed",
            Self::NotSubmittedProven { .. } => "not_submitted_proven",
            Self::SubmissionUnknown { .. } => "submission_unknown",
            Self::ReceiptObserved { .. } => "receipt_observed",
            Self::ConfirmationObserved { .. } => "confirmation_observed",
            Self::Ambiguous { .. } => "ambiguous",
            Self::Failed { .. } => "failed",
        }
    }
}
