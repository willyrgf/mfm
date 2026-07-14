use super::*;

/// Closed v1 typed kernel event payload enum.
///
/// Payloads stay unboxed here to preserve the stable event API and keep schema-bearing
/// authority payloads directly inspectable at commit/replay boundaries.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelEventPayload {
    /// Run start event.
    RunAdmitted(Box<RunAdmitted>),
    /// State attempt start event.
    StateAttemptStarted(StateAttemptStarted),
    /// Recorded read fact event.
    FactRecorded(FactRecorded),
    /// Artifact reference event.
    ArtifactReferenced(ArtifactReferenced),
    /// Cell produced terminal event.
    CellProduced(CellProduced),
    /// Cell skipped terminal event.
    CellSkipped(CellSkipped),
    /// Side-effect intent persisted event.
    SideEffectIntentPersisted(side_effect::IntentPersisted),
    /// Side-effect claim acquired event.
    SideEffectClaimed(side_effect::Claimed),
    /// Side-effect claim takeover event.
    SideEffectClaimTakenOver(side_effect::ClaimTakenOver),
    /// Resource lane claimed before side-effect invocation.
    ResourceLaneClaimed(ResourceLaneClaimed),
    /// Prepared resource-lane claim before store fill.
    ResourceLaneClaimIntent(ResourceLaneClaimIntent),
    /// Side-effect invocation prepared event.
    SideEffectInvocationPrepared(side_effect::InvocationPrepared),
    /// Side-effect invocation started event.
    SideEffectInvocationStarted(side_effect::InvocationStarted),
    /// Side-effect not-submitted proof event.
    SideEffectNotSubmittedProven(side_effect::NotSubmittedProven),
    /// Side-effect submission observed event.
    SideEffectSubmissionObserved(side_effect::SubmissionObserved),
    /// Side-effect submission unknown event.
    SideEffectSubmissionUnknown(side_effect::SubmissionUnknown),
    /// Side-effect receipt observed event.
    SideEffectReceiptObserved(side_effect::ReceiptObserved),
    /// Side-effect confirmation observed event.
    SideEffectConfirmationObserved(side_effect::ConfirmationObserved),
    /// Side-effect ambiguity event.
    SideEffectAmbiguous(side_effect::Ambiguous),
    /// Side-effect failure event.
    SideEffectFailed(side_effect::Failed),
    /// Resource lane released after side-effect terminal evidence or cleanup.
    ResourceLaneReleased(ResourceLaneReleased),
    /// Prepared resource-lane release before store fill.
    ResourceLaneReleaseIntent(ResourceLaneReleaseIntent),
    /// Public output produced event.
    PublicOutputProduced(PublicOutputProduced),
    /// Public output render failure audit event.
    PublicOutputRenderFailed(PublicOutputRenderFailed),
    /// State attempt completed event.
    StateAttemptCompleted(StateAttemptCompleted),
    /// State attempt interrupted event.
    StateAttemptInterrupted(StateAttemptInterrupted),
    /// State attempt failed event.
    StateAttemptFailed(StateAttemptFailed),
    /// Manual saga resolution recorded event.
    ManualResolutionRecorded(ManualResolutionRecorded),
    /// Run completed event.
    RunCompleted(RunCompleted),
    /// Retention references appended event.
    RetentionRefsAppended(RetentionRefsAppended),
    /// Retention manifest projected event.
    RetentionManifestProjected(RetentionManifestProjected),
}

impl KernelEventPayload {
    /// Returns the schema descriptor for this payload variant.
    pub const fn schema_descriptor(&self) -> EventSchemaDescriptor {
        match self {
            Self::RunAdmitted(_) => RUN_ADMITTED_SCHEMA,
            Self::StateAttemptStarted(_) => STATE_ATTEMPT_STARTED_SCHEMA,
            Self::FactRecorded(_) => FACT_RECORDED_SCHEMA,
            Self::ArtifactReferenced(_) => ARTIFACT_REFERENCED_SCHEMA,
            Self::CellProduced(_) => CELL_PRODUCED_SCHEMA,
            Self::CellSkipped(_) => CELL_SKIPPED_SCHEMA,
            Self::SideEffectIntentPersisted(_) => SIDE_EFFECT_INTENT_PERSISTED_SCHEMA,
            Self::SideEffectClaimed(_) => SIDE_EFFECT_CLAIMED_SCHEMA,
            Self::SideEffectClaimTakenOver(_) => SIDE_EFFECT_CLAIM_TAKEN_OVER_SCHEMA,
            Self::ResourceLaneClaimed(_) => RESOURCE_LANE_CLAIMED_SCHEMA,
            Self::ResourceLaneClaimIntent(_) => RESOURCE_LANE_CLAIM_INTENT_SCHEMA,
            Self::SideEffectInvocationPrepared(_) => SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA,
            Self::SideEffectInvocationStarted(_) => SIDE_EFFECT_INVOCATION_STARTED_SCHEMA,
            Self::SideEffectNotSubmittedProven(_) => SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA,
            Self::SideEffectSubmissionObserved(_) => SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA,
            Self::SideEffectSubmissionUnknown(_) => SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA,
            Self::SideEffectReceiptObserved(_) => SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA,
            Self::SideEffectConfirmationObserved(_) => SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA,
            Self::SideEffectAmbiguous(_) => SIDE_EFFECT_AMBIGUOUS_SCHEMA,
            Self::SideEffectFailed(_) => SIDE_EFFECT_FAILED_SCHEMA,
            Self::ResourceLaneReleased(_) => RESOURCE_LANE_RELEASED_SCHEMA,
            Self::ResourceLaneReleaseIntent(_) => RESOURCE_LANE_RELEASE_INTENT_SCHEMA,
            Self::PublicOutputProduced(_) => PUBLIC_OUTPUT_PRODUCED_SCHEMA,
            Self::PublicOutputRenderFailed(_) => PUBLIC_OUTPUT_RENDER_FAILED_SCHEMA,
            Self::StateAttemptCompleted(_) => STATE_ATTEMPT_COMPLETED_SCHEMA,
            Self::StateAttemptInterrupted(_) => STATE_ATTEMPT_INTERRUPTED_SCHEMA,
            Self::StateAttemptFailed(_) => STATE_ATTEMPT_FAILED_SCHEMA,
            Self::ManualResolutionRecorded(_) => MANUAL_RESOLUTION_RECORDED_SCHEMA,
            Self::RunCompleted(_) => RUN_COMPLETED_SCHEMA,
            Self::RetentionRefsAppended(_) => RETENTION_REFS_APPENDED_SCHEMA,
            Self::RetentionManifestProjected(_) => RETENTION_MANIFEST_PROJECTED_SCHEMA,
        }
    }

    /// Returns the event schema id for this payload variant.
    pub fn event_schema_id(&self) -> Result<SchemaId> {
        self.schema_descriptor().schema_id()
    }

    /// Returns the run id carried directly by run-scoped payloads.
    ///
    /// Most node-scoped events rely on the store-owned envelope for run identity, so they
    /// return `None` here.
    pub fn run_id(&self) -> Option<&RunId> {
        match self {
            Self::RunAdmitted(payload) => Some(&payload.run_id),
            Self::ManualResolutionRecorded(payload) => Some(&payload.run_id),
            Self::RunCompleted(payload) => Some(&payload.run_id),
            Self::RetentionRefsAppended(payload) => Some(&payload.run_id),
            Self::RetentionManifestProjected(payload) => Some(&payload.run_id),
            _ => None,
        }
    }

    /// Returns the certified spec hash carried by this payload.
    pub fn spec_hash(&self) -> &SpecHash {
        match self {
            Self::RunAdmitted(payload) => &payload.spec_hash,
            Self::StateAttemptStarted(payload) => &payload.spec_hash,
            Self::FactRecorded(payload) => &payload.spec_hash,
            Self::ArtifactReferenced(payload) => &payload.spec_hash,
            Self::CellProduced(payload) => &payload.spec_hash,
            Self::CellSkipped(payload) => &payload.spec_hash,
            Self::SideEffectIntentPersisted(payload) => &payload.spec_hash,
            Self::SideEffectClaimed(payload) => &payload.spec_hash,
            Self::SideEffectClaimTakenOver(payload) => &payload.spec_hash,
            Self::ResourceLaneClaimed(payload) => &payload.spec_hash,
            Self::ResourceLaneClaimIntent(payload) => &payload.spec_hash,
            Self::SideEffectInvocationPrepared(payload) => &payload.spec_hash,
            Self::SideEffectInvocationStarted(payload) => &payload.spec_hash,
            Self::SideEffectNotSubmittedProven(payload) => &payload.spec_hash,
            Self::SideEffectSubmissionObserved(payload) => &payload.spec_hash,
            Self::SideEffectSubmissionUnknown(payload) => &payload.spec_hash,
            Self::SideEffectReceiptObserved(payload) => &payload.spec_hash,
            Self::SideEffectConfirmationObserved(payload) => &payload.spec_hash,
            Self::SideEffectAmbiguous(payload) => &payload.spec_hash,
            Self::SideEffectFailed(payload) => &payload.spec_hash,
            Self::ResourceLaneReleased(payload) => &payload.spec_hash,
            Self::ResourceLaneReleaseIntent(payload) => &payload.spec_hash,
            Self::PublicOutputProduced(payload) => &payload.spec_hash,
            Self::PublicOutputRenderFailed(payload) => &payload.spec_hash,
            Self::StateAttemptCompleted(payload) => &payload.spec_hash,
            Self::StateAttemptInterrupted(payload) => &payload.spec_hash,
            Self::StateAttemptFailed(payload) => &payload.spec_hash,
            Self::ManualResolutionRecorded(payload) => &payload.spec_hash,
            Self::RunCompleted(payload) => &payload.spec_hash,
            Self::RetentionRefsAppended(payload) => &payload.spec_hash,
            Self::RetentionManifestProjected(payload) => &payload.spec_hash,
        }
    }

    /// Returns whether this payload is terminal evidence for a state attempt.
    pub fn is_attempt_terminal(&self) -> bool {
        matches!(
            self,
            Self::StateAttemptCompleted(_)
                | Self::StateAttemptInterrupted(_)
                | Self::StateAttemptFailed(_)
                | Self::CellProduced(_)
                | Self::CellSkipped(_)
                | Self::FactRecorded(_)
                | Self::ArtifactReferenced(_)
                | Self::PublicOutputProduced(_)
                | Self::PublicOutputRenderFailed(_)
                | Self::RunCompleted(_)
        )
    }

    /// Returns whether this payload is terminal evidence for a side-effect attempt.
    pub fn is_side_effect_terminal(&self) -> bool {
        matches!(
            self,
            Self::SideEffectNotSubmittedProven(_)
                | Self::SideEffectSubmissionObserved(_)
                | Self::SideEffectSubmissionUnknown(_)
                | Self::SideEffectReceiptObserved(_)
                | Self::SideEffectConfirmationObserved(_)
                | Self::SideEffectAmbiguous(_)
                | Self::SideEffectFailed(_)
                | Self::ResourceLaneReleased(_)
                | Self::ResourceLaneReleaseIntent(_)
        )
    }

    /// Returns whether this payload is side-effect terminal evidence excluding lane releases.
    pub fn is_side_effect_terminal_disposition(&self) -> bool {
        self.is_side_effect_terminal()
            && !matches!(
                self,
                Self::ResourceLaneReleased(_) | Self::ResourceLaneReleaseIntent(_)
            )
    }

    /// Returns pair-ledger authority fields for payloads in the side-effect ledger family.
    pub fn side_effect_ledger_ref(&self) -> Option<SideEffectPairLedgerEventRef<'_>> {
        macro_rules! side_effect_ledger_ref {
            ($payload:ident, $kind:ident, $claim_generation:expr) => {
                Some(SideEffectPairLedgerEventRef {
                    ledger_key: &$payload.ledger_key,
                    ledger_purpose: &$payload.ledger_purpose,
                    pair_id: &$payload.pair_id,
                    pair_role: $payload.pair_role,
                    invocation_epoch: Some($payload.invocation_epoch),
                    claim_generation: $claim_generation,
                    kind: SideEffectEventKind::$kind,
                })
            };
        }

        match self {
            Self::SideEffectIntentPersisted(payload) => {
                side_effect_ledger_ref!(payload, IntentPersisted, None)
            }
            Self::SideEffectClaimed(payload) => {
                side_effect_ledger_ref!(payload, Claimed, Some(payload.claim_generation))
            }
            Self::SideEffectClaimTakenOver(payload) => {
                side_effect_ledger_ref!(payload, ClaimTakenOver, Some(payload.claim_generation))
            }
            Self::ResourceLaneClaimed(payload) => {
                side_effect_ledger_ref!(payload, ResourceLaneClaimed, None)
            }
            Self::ResourceLaneClaimIntent(payload) => {
                side_effect_ledger_ref!(payload, ResourceLaneClaimed, None)
            }
            Self::SideEffectInvocationPrepared(payload) => {
                side_effect_ledger_ref!(payload, InvocationPrepared, Some(payload.claim_generation))
            }
            Self::SideEffectInvocationStarted(payload) => {
                side_effect_ledger_ref!(payload, InvocationStarted, Some(payload.claim_generation))
            }
            Self::SideEffectNotSubmittedProven(payload) => {
                side_effect_ledger_ref!(payload, NotSubmittedProven, None)
            }
            Self::SideEffectSubmissionObserved(payload) => {
                side_effect_ledger_ref!(payload, SubmissionObserved, None)
            }
            Self::SideEffectSubmissionUnknown(payload) => {
                side_effect_ledger_ref!(payload, SubmissionUnknown, None)
            }
            Self::SideEffectReceiptObserved(payload) => {
                side_effect_ledger_ref!(payload, ReceiptObserved, None)
            }
            Self::SideEffectConfirmationObserved(payload) => {
                side_effect_ledger_ref!(payload, ConfirmationObserved, None)
            }
            Self::SideEffectAmbiguous(payload) => {
                side_effect_ledger_ref!(payload, Ambiguous, None)
            }
            Self::SideEffectFailed(payload) => side_effect_ledger_ref!(payload, Failed, None),
            Self::ResourceLaneReleased(payload) => {
                side_effect_ledger_ref!(payload, ResourceLaneReleased, None)
            }
            Self::ResourceLaneReleaseIntent(payload) => {
                side_effect_ledger_ref!(payload, ResourceLaneReleased, None)
            }
            _ => None,
        }
    }

    /// Returns emitter and attempt attribution for payloads in the side-effect ledger family.
    pub fn side_effect_emitter_ref(&self) -> Option<SideEffectEmitterAttemptRef<'_>> {
        macro_rules! side_effect_emitter_ref {
            ($payload:ident, $kind:ident) => {
                Some(SideEffectEmitterAttemptRef {
                    node_id: &$payload.node_id,
                    attempt_id: &$payload.attempt_id,
                    pair_role: $payload.pair_role,
                    kind: SideEffectEventKind::$kind,
                })
            };
        }

        match self {
            Self::SideEffectIntentPersisted(payload) => {
                side_effect_emitter_ref!(payload, IntentPersisted)
            }
            Self::SideEffectClaimed(payload) => side_effect_emitter_ref!(payload, Claimed),
            Self::SideEffectClaimTakenOver(payload) => {
                side_effect_emitter_ref!(payload, ClaimTakenOver)
            }
            Self::ResourceLaneClaimed(payload) => {
                side_effect_emitter_ref!(payload, ResourceLaneClaimed)
            }
            Self::ResourceLaneClaimIntent(payload) => {
                side_effect_emitter_ref!(payload, ResourceLaneClaimed)
            }
            Self::SideEffectInvocationPrepared(payload) => {
                side_effect_emitter_ref!(payload, InvocationPrepared)
            }
            Self::SideEffectInvocationStarted(payload) => {
                side_effect_emitter_ref!(payload, InvocationStarted)
            }
            Self::SideEffectNotSubmittedProven(payload) => {
                side_effect_emitter_ref!(payload, NotSubmittedProven)
            }
            Self::SideEffectSubmissionObserved(payload) => {
                side_effect_emitter_ref!(payload, SubmissionObserved)
            }
            Self::SideEffectSubmissionUnknown(payload) => {
                side_effect_emitter_ref!(payload, SubmissionUnknown)
            }
            Self::SideEffectReceiptObserved(payload) => {
                side_effect_emitter_ref!(payload, ReceiptObserved)
            }
            Self::SideEffectConfirmationObserved(payload) => {
                side_effect_emitter_ref!(payload, ConfirmationObserved)
            }
            Self::SideEffectAmbiguous(payload) => side_effect_emitter_ref!(payload, Ambiguous),
            Self::SideEffectFailed(payload) => side_effect_emitter_ref!(payload, Failed),
            _ => None,
        }
    }

    /// Returns resource-lane holder/release authority fields for resource-lane payloads.
    pub fn resource_lane_authority_ref(&self) -> Option<ResourceLaneAuthorityRef<'_>> {
        match self {
            Self::ResourceLaneClaimed(payload) => Some(ResourceLaneAuthorityRef {
                ledger: self.side_effect_ledger_ref()?,
                emitter: Some(self.side_effect_emitter_ref()?),
                claim_id: Some(&payload.claim_id),
                claim_fencing_token: Some(payload.claim_fencing_token),
                lane_transition_seq: Some(payload.lane_transition_seq),
                release_authority: None,
                release_reason: None,
            }),
            Self::ResourceLaneClaimIntent(_) => Some(ResourceLaneAuthorityRef {
                ledger: self.side_effect_ledger_ref()?,
                emitter: Some(self.side_effect_emitter_ref()?),
                claim_id: None,
                claim_fencing_token: None,
                lane_transition_seq: None,
                release_authority: None,
                release_reason: None,
            }),
            Self::ResourceLaneReleased(payload) => Some(ResourceLaneAuthorityRef {
                ledger: self.side_effect_ledger_ref()?,
                emitter: None,
                claim_id: Some(&payload.claim_id),
                claim_fencing_token: Some(payload.claim_fencing_token),
                lane_transition_seq: Some(payload.lane_transition_seq),
                release_authority: Some(payload.release_authority),
                release_reason: Some(&payload.release_reason),
            }),
            Self::ResourceLaneReleaseIntent(payload) => Some(ResourceLaneAuthorityRef {
                ledger: self.side_effect_ledger_ref()?,
                emitter: None,
                claim_id: Some(&payload.claim_id),
                claim_fencing_token: None,
                lane_transition_seq: None,
                release_authority: Some(payload.release_authority),
                release_reason: Some(&payload.release_reason),
            }),
            _ => None,
        }
    }

    /// Returns the full side-effect event view for payloads in the side-effect ledger family.
    pub fn side_effect_ref(&self) -> Option<SideEffectEventRef<'_>> {
        let ledger = self.side_effect_ledger_ref()?;
        let emitter = self.side_effect_emitter_ref()?;
        Some(SideEffectEventRef {
            node_id: emitter.node_id,
            attempt_id: emitter.attempt_id,
            ledger_key: ledger.ledger_key,
            ledger_purpose: ledger.ledger_purpose,
            pair_id: ledger.pair_id,
            pair_role: ledger.pair_role,
            invocation_epoch: ledger.invocation_epoch,
            claim_generation: ledger.claim_generation,
            kind: ledger.kind,
        })
    }

    /// Returns artifact evidence requirements referenced by this payload.
    pub fn artifact_requirements(&self) -> Vec<EventArtifactRequirement> {
        event_artifact_requirements(self)
    }
}

/// Side-effect payload kind used by read-only event views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SideEffectEventKind {
    /// Intent persisted.
    IntentPersisted,
    /// Claim acquired.
    Claimed,
    /// Claim taken over.
    ClaimTakenOver,
    /// Resource lane claimed.
    ResourceLaneClaimed,
    /// Invocation prepared.
    InvocationPrepared,
    /// Invocation started.
    InvocationStarted,
    /// Not-submitted proof recorded.
    NotSubmittedProven,
    /// Submission observed.
    SubmissionObserved,
    /// Submission status unknown.
    SubmissionUnknown,
    /// Receipt observed.
    ReceiptObserved,
    /// Confirmation observed.
    ConfirmationObserved,
    /// Side effect became ambiguous.
    Ambiguous,
    /// Side effect failed.
    Failed,
    /// Resource lane released.
    ResourceLaneReleased,
}

/// Pair-ledger authority fields common to side-effect event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectPairLedgerEventRef<'a> {
    /// Side-effect ledger key.
    pub ledger_key: &'a SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: &'a SideEffectLedgerPurpose,
    /// Certified side-effect pair id.
    pub pair_id: &'a SideEffectPairId,
    /// Pair phase authority.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch when the payload carries one.
    pub invocation_epoch: Option<u32>,
    /// Claim generation when the payload carries one.
    pub claim_generation: Option<u32>,
    /// Side-effect payload kind.
    pub kind: SideEffectEventKind,
}

/// Emitter and active-attempt attribution for side-effect event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectEmitterAttemptRef<'a> {
    /// Node id associated with the side-effect event.
    pub node_id: &'a NodeId,
    /// Attempt id associated with the side-effect event.
    pub attempt_id: &'a AttemptId,
    /// Pair phase emitted by the node.
    pub pair_role: SideEffectPairRole,
    /// Side-effect payload kind.
    pub kind: SideEffectEventKind,
}

/// Resource-lane holder or release authority borrowed from a lane payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLaneAuthorityRef<'a> {
    /// Pair-ledger authority bound to the lane payload.
    pub ledger: SideEffectPairLedgerEventRef<'a>,
    /// Emitter and attempt attribution for lane claim payloads.
    pub emitter: Option<SideEffectEmitterAttemptRef<'a>>,
    /// Claim id when the lane payload has one.
    pub claim_id: Option<&'a ResourceLaneClaimId>,
    /// Store-assigned fencing token when materialized.
    pub claim_fencing_token: Option<u64>,
    /// Store-assigned lane-local transition sequence when materialized.
    pub lane_transition_seq: Option<u64>,
    /// Release authority when this is a release payload.
    pub release_authority: Option<ResourceLaneReleaseAuthority>,
    /// Release reason when this is a release payload.
    pub release_reason: Option<&'a ResourceLaneReleaseReason>,
}

/// Borrowed side-effect fields common to side-effect event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SideEffectEventRef<'a> {
    /// Node id associated with the side-effect event.
    pub node_id: &'a NodeId,
    /// Attempt id associated with the side-effect event.
    pub attempt_id: &'a AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: &'a SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: &'a SideEffectLedgerPurpose,
    /// Certified side-effect pair id.
    pub pair_id: &'a SideEffectPairId,
    /// Pair phase authority.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch when the payload carries one.
    pub invocation_epoch: Option<u32>,
    /// Claim generation when the payload carries one.
    pub claim_generation: Option<u32>,
    /// Side-effect payload kind.
    pub kind: SideEffectEventKind,
}
