use super::*;

/// Run state used by typed commit preconditions and projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunState {
    /// No start event has been committed.
    Absent,
    /// Run has started and is not terminal.
    Started,
    /// Run has reached a terminal outcome.
    Completed,
}

/// Stream-derived semantic run mode for saga-aware status and terminal resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunMode {
    /// Forward graph execution is still the active frontier.
    Forward,
    /// Remediation work is the active frontier.
    Remediating,
    /// The run requires typed operator evidence before terminal resolution.
    ManualBlocked,
    /// Successful forward public output completed the run.
    Completed,
    /// The run completed after closing compensating obligations.
    Compensated,
    /// The run completed after accepted manual remediation evidence.
    ManuallyResolved,
    /// The run completed without making a compensation or AC/DC-equivalence claim.
    FailedWithoutAcdcClaim,
}

impl RunMode {
    /// Returns the canonical snake-case tag for this run mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Remediating => "remediating",
            Self::ManualBlocked => "manual_blocked",
            Self::Completed => "completed",
            Self::Compensated => "compensated",
            Self::ManuallyResolved => "manually_resolved",
            Self::FailedWithoutAcdcClaim => "failed_without_acdc_claim",
        }
    }

    /// Returns the saga terminal outcome represented by this run mode, when terminal.
    pub fn saga_terminal_outcome(self) -> Option<events::RunCompletionOutcome> {
        match self {
            Self::Compensated => Some(events::RunCompletionOutcome::Compensated),
            Self::ManuallyResolved => Some(events::RunCompletionOutcome::ManuallyResolved),
            Self::FailedWithoutAcdcClaim => {
                Some(events::RunCompletionOutcome::FailedWithoutAcdcClaim)
            }
            Self::Forward | Self::Remediating | Self::ManualBlocked | Self::Completed => None,
        }
    }

    /// Returns the canonical run mode represented by a committed run-completion outcome.
    pub fn from_completion_outcome(outcome: &events::RunCompletionOutcome) -> Self {
        match outcome {
            events::RunCompletionOutcome::Completed(_) => Self::Completed,
            events::RunCompletionOutcome::Compensated => Self::Compensated,
            events::RunCompletionOutcome::ManuallyResolved => Self::ManuallyResolved,
            events::RunCompletionOutcome::FailedWithoutAcdcClaim => Self::FailedWithoutAcdcClaim,
        }
    }
}

/// Reason the derived saga mode is manually blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManualBlockReason {
    /// Certified run policy requires operator resolution after engagement.
    PolicyManualResolution,
    /// A forward ledger is ambiguous at quiescence.
    ForwardAmbiguous,
    /// A remediation ledger failed non-retryably.
    RemediationFailed,
    /// A remediation ledger is ambiguous.
    RemediationAmbiguous,
}

/// First stream event that engaged saga handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaEngagementProjection {
    /// Store-owned event id that first engaged saga handling.
    pub event_id: EventId,
    /// Reason saga handling engaged.
    pub reason: SagaEngagementReason,
}

/// Stream-derived reason for saga engagement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SagaEngagementReason {
    /// A non-retryable attempt or side-effect failure was recorded.
    NonRetryableFailure {
        /// Node id that failed.
        node_id: NodeId,
        /// Attempt id that failed.
        attempt_id: AttemptId,
    },
    /// A forward side-effect pair became ambiguous.
    ForwardAmbiguous {
        /// Forward pair id.
        pair_id: SideEffectPairId,
    },
}

/// Run-scoped manual resolution projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionProjection {
    /// Store-owned event id that recorded manual resolution.
    pub event_id: EventId,
    /// Operator-selected outcome.
    pub outcome: events::ManualResolutionOutcome,
    /// Evidence schema id.
    pub evidence_schema_id: SchemaId,
    /// Evidence hash.
    pub evidence_hash: ContentDigest,
    /// Evidence artifact id.
    pub evidence_artifact_id: ArtifactId,
    /// Authorization proof schema id.
    pub authorization_schema_id: SchemaId,
    /// Authorization proof hash.
    pub authorization_hash: ContentDigest,
    /// Authorization proof artifact id.
    pub authorization_artifact_id: ArtifactId,
    /// Optional redaction-safe operator note.
    pub note: Option<events::ManualResolutionNote>,
}

/// Run completion projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCompletionProjection {
    /// Store-owned event id that recorded completion.
    pub event_id: EventId,
    /// Recorded run completion outcome.
    pub outcome: events::RunCompletionOutcome,
}

/// Forward ledger classification at the current stream prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForwardLedgerClassification {
    /// The ledger is not owed, remediated, or unresolvable for compensation decisions yet.
    Pending,
    /// The ledger owes no compensation.
    NothingOwed,
    /// The ledger is confirmed and must be remediated under compensating policy.
    Owed,
    /// The ledger is ambiguous and cannot be platform-compensated.
    Unresolvable,
}

/// Remediation state linked to one forward ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemediationLedgerProjection {
    /// Remediation ledger key.
    pub ledger_key: events::SideEffectLedgerKey,
    /// Remediation side-effect pair id.
    pub pair_id: SideEffectPairId,
    /// Current remediation side-effect phase.
    pub phase: SideEffectPhase,
    /// Whether remediation confirmation closed the obligation.
    pub closed: bool,
    /// Whether remediation reached an unresolved condition.
    pub unresolved: Option<ManualBlockReason>,
}

/// Derived obligation state for one forward ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaObligationProjection {
    /// Forward ledger key.
    pub forward_ledger_key: events::SideEffectLedgerKey,
    /// Forward side-effect pair id.
    pub forward_pair_id: SideEffectPairId,
    /// Current forward side-effect phase.
    pub forward_phase: SideEffectPhase,
    /// Forward ledger classification at this stream prefix.
    pub classification: ForwardLedgerClassification,
    /// Linked remediation ledger state, when one exists.
    pub remediation: Option<RemediationLedgerProjection>,
}

/// Saga projection derived from certified policy plus the store stream projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SagaProjection {
    /// Run id this projection describes.
    pub run_id: RunId,
    /// Derived semantic run mode.
    pub run_mode: RunMode,
    /// First engaging event, if any.
    pub engagement: Option<SagaEngagementProjection>,
    /// Whether every past-boundary forward ledger is quiescent.
    pub forward_quiescent: bool,
    /// Derived manual block reason, when manually blocked.
    pub manual_block_reason: Option<ManualBlockReason>,
    /// Derived forward-pair obligation state.
    pub obligations: BTreeMap<SideEffectPairId, SagaObligationProjection>,
    /// Manual resolution evidence recorded for the run, if any.
    pub manual_resolution: Option<ManualResolutionProjection>,
    /// Run completion recorded for the run, if any.
    pub run_completion: Option<RunCompletionProjection>,
}

/// Certified terminal evidence policy for a side-effect pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SideEffectTerminalPolicy {
    /// Receipt evidence is the terminal side-effect proof.
    Receipt,
    /// Confirmation evidence is the terminal side-effect proof.
    Confirmation,
}

impl SideEffectTerminalPolicy {
    /// Returns the terminal policy implied by a certified side-effect verification spec.
    pub const fn from_verification(verification: &SideEffectVerificationSpec) -> Self {
        match verification {
            SideEffectVerificationSpec::Receipt => Self::Receipt,
            SideEffectVerificationSpec::Finalized { .. } => Self::Confirmation,
        }
    }

    /// Returns whether the projected phase satisfies this terminal policy.
    pub const fn is_terminal_phase(self, phase: &SideEffectPhase) -> bool {
        match self {
            Self::Receipt => matches!(
                phase,
                SideEffectPhase::ReceiptObserved { .. }
                    | SideEffectPhase::ConfirmationObserved { .. }
            ),
            Self::Confirmation => matches!(phase, SideEffectPhase::ConfirmationObserved { .. }),
        }
    }
}

/// Certified terminal policies for side-effect pairs in a typed spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideEffectTerminalPolicies {
    by_pair: BTreeMap<SideEffectPairId, SideEffectTerminalPolicy>,
}

impl SideEffectTerminalPolicies {
    /// Builds terminal policies for every side-effect node in a certified typed spec.
    pub fn from_spec(spec: &TypedExecutionSpec) -> Result<Self> {
        let mut by_pair = BTreeMap::new();
        for node in spec.nodes.iter().chain(spec.remediations.values()) {
            let Some(contract) = node.side_effect.as_ref() else {
                continue;
            };
            let pair_id = spec::side_effect_pair_id(&node.node_id, &node.output_cell, contract)
                .map_err(|error| StoreError::Identity(error.to_string()))?;
            by_pair.insert(
                pair_id,
                SideEffectTerminalPolicy::from_verification(&contract.verification),
            );
        }
        Ok(Self { by_pair })
    }

    /// Builds terminal policies from explicit pair entries.
    pub fn new(by_pair: BTreeMap<SideEffectPairId, SideEffectTerminalPolicy>) -> Self {
        Self { by_pair }
    }

    /// Returns the policy for a side-effect pair.
    pub fn get(&self, pair_id: &SideEffectPairId) -> Option<SideEffectTerminalPolicy> {
        self.by_pair.get(pair_id).copied()
    }

    /// Returns the policy for a side-effect pair or a typed projection error.
    pub fn require(&self, pair_id: &SideEffectPairId) -> Result<SideEffectTerminalPolicy> {
        self.get(pair_id)
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("sidefx:{pair_id}:terminal_policy"),
                message: "missing certified side-effect terminal policy".to_owned(),
            })
    }
}

fn require_closed_obligations_non_empty(
    policy: &SagaPolicySpec,
    saga: &SagaProjection,
) -> Result<()> {
    if !matches!(policy, SagaPolicySpec::CompensateCompleted { .. }) {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "compensated terminal requires compensating saga policy".to_owned(),
        });
    }
    if saga.run_mode != RunMode::Compensated {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: format!(
                "compensated terminal requires compensated saga mode, found {}",
                saga.run_mode.as_str()
            ),
        });
    }
    let mut closed_count = 0;
    for obligation in saga.obligations.values() {
        if obligation.classification != ForwardLedgerClassification::Owed {
            continue;
        }
        match obligation.remediation.as_ref() {
            Some(remediation) if remediation.closed && remediation.unresolved.is_none() => {
                closed_count += 1;
            }
            _ => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "run:{}:obligation:{}",
                        saga.run_id, obligation.forward_ledger_key
                    ),
                    message: "compensated terminal requires every owed obligation to be closed"
                        .to_owned(),
                });
            }
        }
    }
    if closed_count == 0 {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "compensated terminal requires a non-empty owed obligation set".to_owned(),
        });
    }
    Ok(())
}

fn require_failed_without_acdc_claim(
    policy: &SagaPolicySpec,
    saga: &SagaProjection,
    certified_spec_hash: &SpecHash,
    manual: Option<&crate::v1::journal::history_validation::VerifiedHistoricalManualResolution>,
) -> Result<Option<SpecHash>> {
    if saga.run_mode != RunMode::FailedWithoutAcdcClaim {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: format!(
                "failed-without-ACDC terminal requires failed_without_acdc_claim saga mode, found {}",
                saga.run_mode.as_str()
            ),
        });
    }
    if saga.manual_resolution.is_some() {
        let verified = manual.ok_or_else(|| StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "manual failed terminal requires verified manual resolution history"
                .to_owned(),
        })?;
        require_verified_manual_resolution_matches(
            saga,
            verified,
            events::ManualResolutionOutcome::FailWithoutAcdcClaim,
        )?;
        return Ok(Some(certified_spec_hash.clone()));
    }
    if !policy_allows_failed_without_acdc_claim(policy) {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "certified saga policy does not permit failed_without_acdc_claim terminal"
                .to_owned(),
        });
    }
    Ok(None)
}

/// Opaque proof for one terminal saga outcome.
#[derive(Debug, Clone)]
pub struct SagaTerminalProof {
    run_id: RunId,
    prefix_next_seq: StreamSeq,
    saga_policy_digest: ContentDigest,
    manual_spec_hash: Option<SpecHash>,
    kind: SagaTerminalProofKind,
}

#[derive(Debug, Clone)]
enum SagaTerminalProofKind {
    Completed(Box<events::PublicOutputCompletionEvidence>),
    Compensated,
    ManuallyResolved,
    FailedWithoutAcdcClaim,
}

impl SagaTerminalProof {
    /// Mints terminal proof from certified saga policy plus current saga projection.
    pub(in crate::v1) fn new(
        policy: &SagaPolicySpec,
        saga: &SagaProjection,
        prefix_next_seq: StreamSeq,
        certified_spec_hash: &SpecHash,
        manual: Option<&crate::v1::journal::history_validation::VerifiedHistoricalManualResolution>,
    ) -> Result<Self> {
        let saga_policy_digest = policy
            .saga_policy_digest()
            .map_err(|error| StoreError::Canonical(error.to_string()))?;
        let (kind, manual_spec_hash) = match saga.run_mode {
            RunMode::Compensated => {
                require_closed_obligations_non_empty(policy, saga)?;
                (SagaTerminalProofKind::Compensated, None)
            }
            RunMode::ManuallyResolved => {
                let verified = manual.ok_or_else(|| StoreError::ProjectionConflict {
                    key: format!("run:{}:saga_terminal", saga.run_id),
                    message: "manual terminal requires verified manual resolution history"
                        .to_owned(),
                })?;
                require_verified_manual_resolution_matches(
                    saga,
                    verified,
                    events::ManualResolutionOutcome::ConfirmRemediated,
                )?;
                (
                    SagaTerminalProofKind::ManuallyResolved,
                    Some(certified_spec_hash.clone()),
                )
            }
            RunMode::FailedWithoutAcdcClaim => {
                let manual_spec_hash =
                    require_failed_without_acdc_claim(policy, saga, certified_spec_hash, manual)?;
                (
                    SagaTerminalProofKind::FailedWithoutAcdcClaim,
                    manual_spec_hash,
                )
            }
            RunMode::Completed => {
                let Some(RunCompletionProjection {
                    outcome: events::RunCompletionOutcome::Completed(evidence),
                    ..
                }) = saga.run_completion.as_ref()
                else {
                    return Err(StoreError::ProjectionConflict {
                        key: format!("run:{}:saga_terminal", saga.run_id),
                        message: "completed proof requires completed run projection".to_owned(),
                    });
                };
                (SagaTerminalProofKind::Completed(evidence.clone()), None)
            }
            RunMode::Forward | RunMode::Remediating | RunMode::ManualBlocked => {
                return Err(StoreError::ProjectionConflict {
                    key: format!("run:{}:saga_terminal", saga.run_id),
                    message: format!(
                        "saga terminal proof requires terminal saga mode, found {}",
                        saga.run_mode.as_str()
                    ),
                });
            }
        };
        Ok(Self {
            run_id: saga.run_id.clone(),
            prefix_next_seq,
            saga_policy_digest,
            manual_spec_hash,
            kind,
        })
    }

    /// Returns the run id this proof was minted for.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the stream sequence immediately after the prefix this proof was minted from.
    pub const fn prefix_next_seq(&self) -> StreamSeq {
        self.prefix_next_seq
    }

    /// Returns the saga policy digest this proof was minted under.
    pub fn saga_policy_digest(&self) -> &ContentDigest {
        &self.saga_policy_digest
    }

    /// Returns the run completion outcome proven by this terminal proof.
    pub fn outcome(&self) -> events::RunCompletionOutcome {
        match &self.kind {
            SagaTerminalProofKind::Completed(evidence) => {
                events::RunCompletionOutcome::Completed(evidence.clone())
            }
            SagaTerminalProofKind::Compensated => events::RunCompletionOutcome::Compensated,
            SagaTerminalProofKind::ManuallyResolved => {
                events::RunCompletionOutcome::ManuallyResolved
            }
            SagaTerminalProofKind::FailedWithoutAcdcClaim => {
                events::RunCompletionOutcome::FailedWithoutAcdcClaim
            }
        }
    }

    pub(super) fn manual_spec_hash(&self) -> Option<&SpecHash> {
        self.manual_spec_hash.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn forged_failed_without_acdc_claim_for_test(
        run_id: RunId,
        prefix_next_seq: StreamSeq,
        saga_policy_digest: ContentDigest,
    ) -> Self {
        Self {
            run_id,
            prefix_next_seq,
            saga_policy_digest,
            manual_spec_hash: None,
            kind: SagaTerminalProofKind::FailedWithoutAcdcClaim,
        }
    }
}

pub(in crate::v1) fn manual_policy_for_block_reason(
    policy: &SagaPolicySpec,
    reason: ManualBlockReason,
) -> Option<&ManualResolutionEvidenceSpec> {
    match (policy, reason) {
        (
            SagaPolicySpec::ManualResolution { manual },
            ManualBlockReason::PolicyManualResolution,
        ) => Some(manual),
        (
            SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved: RemediationUnresolvedSpec::ManualResolution { manual },
            },
            ManualBlockReason::ForwardAmbiguous
            | ManualBlockReason::RemediationFailed
            | ManualBlockReason::RemediationAmbiguous,
        ) => Some(manual.as_ref()),
        _ => None,
    }
}

fn policy_allows_failed_without_acdc_claim(policy: &SagaPolicySpec) -> bool {
    matches!(
        policy,
        SagaPolicySpec::NoSideEffects
            | SagaPolicySpec::FailWithoutAcdcClaim
            | SagaPolicySpec::CompensateCompleted {
                on_remediation_unresolved: RemediationUnresolvedSpec::FailWithoutAcdcClaim
            }
    )
}

fn require_verified_manual_resolution_matches(
    saga: &SagaProjection,
    verified: &crate::v1::journal::history_validation::VerifiedHistoricalManualResolution,
    expected_outcome: events::ManualResolutionOutcome,
) -> Result<()> {
    if verified.outcome() != expected_outcome {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "verified manual history outcome does not match terminal outcome".to_owned(),
        });
    }
    let Some(manual) = saga.manual_resolution.as_ref() else {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "manual terminal requires recorded manual resolution".to_owned(),
        });
    };
    if manual.outcome != expected_outcome {
        return Err(StoreError::ProjectionConflict {
            key: format!("run:{}:saga_terminal", saga.run_id),
            message: "recorded manual resolution outcome does not match terminal outcome"
                .to_owned(),
        });
    }
    Ok(())
}

pub(super) const fn manual_block_reason_from_auth(
    reason: ManualResolutionBlockReason,
) -> ManualBlockReason {
    match reason {
        ManualResolutionBlockReason::PolicyManualResolution => {
            ManualBlockReason::PolicyManualResolution
        }
        ManualResolutionBlockReason::ForwardAmbiguous => ManualBlockReason::ForwardAmbiguous,
        ManualResolutionBlockReason::RemediationFailed => ManualBlockReason::RemediationFailed,
        ManualResolutionBlockReason::RemediationAmbiguous => {
            ManualBlockReason::RemediationAmbiguous
        }
    }
}
