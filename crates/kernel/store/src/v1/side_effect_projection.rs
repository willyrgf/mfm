use super::*;

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
