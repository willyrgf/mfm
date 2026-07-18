use super::*;

#[path = "side_effect_projection.rs"]
mod side_effect_projection;
pub use self::side_effect_projection::{
    SideEffectLedgerPhase, SideEffectLedgerState, SideEffectProjection, SideEffectSubmissionState,
};

#[path = "side_effect_ledger.rs"]
mod side_effect_ledger;
pub(super) use self::side_effect_ledger::OwnedSideEffectLedgerState;

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
    artifact_id: &ArtifactId,
    content_digest: &ContentDigest,
    evidence_hash: &ContentDigest,
    schema_id: &SchemaId,
) -> SideEffectArtifactProjection {
    SideEffectArtifactProjection {
        artifact_id: artifact_id.clone(),
        content_digest: content_digest.clone(),
        evidence_hash: evidence_hash.clone(),
        schema_id: Some(schema_id.clone()),
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
