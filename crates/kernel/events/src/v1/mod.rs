use super::*;

#[path = "artifact.rs"]
mod artifact;
#[path = "diagnostics.rs"]
mod diagnostics;
#[path = "schema.rs"]
mod schema;
use self::artifact::event_artifact_requirements;
pub use self::artifact::*;
pub use self::diagnostics::{ErrorCategory, MfmErrorInfo, RedactedJson};
pub use self::schema::{
    all_event_schema_descriptors, EventFieldCardinality, EventFieldDescriptor,
    EventSchemaDescriptor,
};
use self::schema::{
    ARTIFACT_REFERENCED_SCHEMA, CELL_PRODUCED_SCHEMA, CELL_SKIPPED_SCHEMA, FACT_RECORDED_SCHEMA,
    MANUAL_RESOLUTION_RECORDED_SCHEMA, PUBLIC_OUTPUT_PRODUCED_SCHEMA,
    PUBLIC_OUTPUT_RENDER_FAILED_SCHEMA, RESOURCE_LANE_CLAIMED_SCHEMA,
    RESOURCE_LANE_CLAIM_INTENT_SCHEMA, RESOURCE_LANE_RELEASED_SCHEMA,
    RESOURCE_LANE_RELEASE_INTENT_SCHEMA, RETENTION_MANIFEST_PROJECTED_SCHEMA,
    RETENTION_REFS_APPENDED_SCHEMA, RUN_ADMITTED_SCHEMA, RUN_COMPLETED_SCHEMA,
    SIDE_EFFECT_AMBIGUOUS_SCHEMA, SIDE_EFFECT_CLAIMED_SCHEMA, SIDE_EFFECT_CLAIM_TAKEN_OVER_SCHEMA,
    SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA, SIDE_EFFECT_FAILED_SCHEMA,
    SIDE_EFFECT_INTENT_PERSISTED_SCHEMA, SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA,
    SIDE_EFFECT_INVOCATION_STARTED_SCHEMA, SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA,
    SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA, SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA,
    SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA, STATE_ATTEMPT_COMPLETED_SCHEMA,
    STATE_ATTEMPT_FAILED_SCHEMA, STATE_ATTEMPT_INTERRUPTED_SCHEMA, STATE_ATTEMPT_STARTED_SCHEMA,
};

/// v1 event schema version string.
pub const EVENT_SCHEMA_VERSION: &str = "1";

checked_string_type!(
    /// Runner or adapter factory id.
    RunnerFactoryId,
    "runner factory id"
);
checked_string_type!(
    /// Nix derivation hash or equivalent build input hash.
    NixDerivationHash,
    "nix derivation hash"
);
checked_string_type!(
    /// Nix output hash or equivalent build output hash.
    NixOutputHash,
    "nix output hash"
);
checked_string_type!(
    /// Side-effect ledger key.
    SideEffectLedgerKey,
    "side effect ledger key"
);
checked_string_type!(
    /// Resource key recorded for an exclusive cross-run lane.
    ResourceKey,
    "resource key"
);
checked_string_type!(
    /// Store-assigned resource-lane claim id.
    ResourceLaneClaimId,
    "resource lane claim id"
);
checked_string_type!(
    /// Store-assigned resource-lane release id.
    ResourceLaneReleaseId,
    "resource lane release id"
);
checked_string_type!(
    /// Resource-lane release reason.
    ResourceLaneReleaseReason,
    "resource lane release reason"
);

/// Authority path that released an exclusive resource lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceLaneReleaseAuthority {
    /// Certified verify node reached side-effect terminal evidence.
    VerifyTerminal,
    /// Operator-signed manual resolution released an ambiguous lane.
    ManualResolution,
}

impl ResourceLaneReleaseAuthority {
    /// Returns the stable persisted string for this authority path.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VerifyTerminal => "verify_terminal",
            Self::ManualResolution => "manual_resolution",
        }
    }

    /// Parses a stable persisted authority string.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "verify_terminal" => Ok(Self::VerifyTerminal),
            "manual_resolution" => Ok(Self::ManualResolution),
            _ => Err(EventError::InvalidString {
                field: "resource lane release authority",
                value: value.to_owned(),
            }),
        }
    }
}
checked_string_type!(
    /// Runner invocation identity.
    RunnerInvocationId,
    "runner invocation id"
);
checked_string_type!(
    /// Idempotency key reference.
    IdempotencyKeyRef,
    "idempotency key"
);
checked_string_type!(
    /// Replay verifier identity.
    ReplayVerifierId,
    "replay verifier id"
);
checked_string_type!(
    /// Ambiguity classifier code.
    AmbiguityCode,
    "ambiguity code"
);
checked_string_type!(
    /// Stable typed error code.
    ErrorCode,
    "error code"
);
checked_text_type!(
    /// Optional redaction-safe operator note for manual resolution.
    ManualResolutionNote,
    "manual resolution note"
);

/// Typed evidence for an exclusive resource lane key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceKeyEvidence {
    /// Resource namespace.
    pub namespace: ResourceNamespace,
    /// Schema id for the typed key evidence.
    pub key_schema_id: SchemaId,
    /// Store-comparable resource key.
    pub key: ResourceKey,
}

/// Store-materialized exclusive resource-lane claim event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLaneClaimed {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id that owns the side-effect attempt.
    pub node_id: NodeId,
    /// Attempt id holding the lane.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this lane is bound to a paired forward ledger.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this lane is bound to a paired forward ledger.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch protected by the lane.
    pub invocation_epoch: u32,
    /// Exact resource key evidence resolved before invocation.
    pub resource_key: ResourceKeyEvidence,
    /// Digest of the certified resource-lane requirement.
    pub requirement_digest: ContentDigest,
    /// Capability implementation that resolved the lane.
    pub resolved_by_capability_impl: RunnerFactoryId,
    /// Store-assigned claim id.
    pub claim_id: ResourceLaneClaimId,
    /// Store-assigned lane-local fencing token.
    pub claim_fencing_token: u64,
    /// Store-assigned lane-local transition sequence.
    pub lane_transition_seq: u64,
}

/// Prepared exclusive resource-lane claim authority before store fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLaneClaimIntent {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id that owns the side-effect attempt.
    pub node_id: NodeId,
    /// Attempt id requesting the lane.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this lane is bound to a paired forward ledger.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this lane is bound to a paired forward ledger.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch protected by the lane.
    pub invocation_epoch: u32,
    /// Exact resource key evidence resolved before invocation.
    pub resource_key: ResourceKeyEvidence,
    /// Digest of the certified resource-lane requirement.
    pub requirement_digest: ContentDigest,
    /// Capability implementation that resolved the lane.
    pub resolved_by_capability_impl: RunnerFactoryId,
}

/// Store-materialized exclusive resource-lane release event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLaneReleased {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this lane is bound to a paired forward ledger.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this lane is bound to a paired forward ledger.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch protected by the lane.
    pub invocation_epoch: u32,
    /// Claim being released.
    pub claim_id: ResourceLaneClaimId,
    /// Store-assigned release id.
    pub release_id: ResourceLaneReleaseId,
    /// Fencing token from the active claim.
    pub claim_fencing_token: u64,
    /// Certified release authority path.
    pub release_authority: ResourceLaneReleaseAuthority,
    /// Reason the lane is being released.
    pub release_reason: ResourceLaneReleaseReason,
    /// Store-assigned lane-local transition sequence.
    pub lane_transition_seq: u64,
}

/// Prepared exclusive resource-lane release authority before store fill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLaneReleaseIntent {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this lane is bound to a paired forward ledger.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this lane is bound to a paired forward ledger.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch protected by the lane.
    pub invocation_epoch: u32,
    /// Claim being released.
    pub claim_id: ResourceLaneClaimId,
    /// Certified release authority path.
    pub release_authority: ResourceLaneReleaseAuthority,
    /// Reason the lane is being released.
    pub release_reason: ResourceLaneReleaseReason,
}

/// Typed evidence for an exact touched resource set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTouchedSetEvidence {
    /// Resource namespace.
    pub namespace: ResourceNamespace,
    /// Schema id for the typed touched-set evidence.
    pub evidence_schema_id: SchemaId,
    /// Canonical touched-set content hash.
    pub evidence_hash: ContentDigest,
    /// Touched-set evidence artifact id.
    pub evidence_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the touched-set artifact.
    pub evidence_artifact_evidence_hash: ContentDigest,
}

/// Persisted purpose for a side-effect ledger.
///
/// Purpose is part of certified saga semantics: forward ledgers are eligible for
/// classification, while remediation ledgers link to the confirmed forward ledger they
/// compensate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SideEffectLedgerPurpose {
    /// Ordinary forward side-effect ledger.
    Forward,
    /// Remediation ledger for the linked forward side-effect ledger.
    Remediation {
        /// Certified forward side-effect pair id whose obligation this remediation addresses.
        forward_pair_id: SideEffectPairId,
    },
}

impl SideEffectLedgerPurpose {
    /// Returns the persisted lowercase purpose tag.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Remediation { .. } => "remediation",
        }
    }
}

/// Pair-phase authority attached to side-effect events for a certified submit/verify pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SideEffectPairRole {
    /// Submit phase authority.
    Submit,
    /// Verify phase authority.
    Verify,
}

impl SideEffectPairRole {
    /// Returns the persisted lowercase role tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Submit => "submit",
            Self::Verify => "verify",
        }
    }

    /// Parses a persisted lowercase role tag.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "submit" => Some(Self::Submit),
            "verify" => Some(Self::Verify),
            _ => None,
        }
    }
}

impl std::str::FromStr for SideEffectPairRole {
    type Err = EventError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(value).ok_or_else(|| EventError::InvalidString {
            field: "side_effect_pair_role",
            value: value.to_owned(),
        })
    }
}

/// Run-scoped manual resolution outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManualResolutionOutcome {
    /// Operator certifies unresolved obligations have been remediated.
    ConfirmRemediated,
    /// Operator closes the run without a compensation or AC/DC-equivalence claim.
    FailWithoutAcdcClaim,
}

impl ManualResolutionOutcome {
    /// Returns the persisted lowercase outcome tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmRemediated => "confirm_remediated",
            Self::FailWithoutAcdcClaim => "fail_without_acdc_claim",
        }
    }
}

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

/// Transient hash-defining material for a caller-requested invocation.
///
/// The raw key is never persisted. Only the digest returned by [`Self::digest`] may enter
/// [`RunIdentityMaterialV1`].
pub struct InvocationKeyMaterialV1<'a> {
    raw_key: &'a str,
}

impl<'a> InvocationKeyMaterialV1<'a> {
    /// Stable canonical domain.
    pub const DOMAIN: &'static str = "mfm.invocation_key.v1";
    /// Maximum accepted raw key size in bytes.
    pub const MAX_RAW_KEY_BYTES: usize = 1024;

    /// Creates transient invocation material from the exact caller-supplied UTF-8 key.
    pub fn new(raw_key: &'a str) -> Result<Self> {
        if raw_key.is_empty() {
            return Err(EventError::InvalidInvocationKey { reason: "empty" });
        }
        if raw_key.len() > Self::MAX_RAW_KEY_BYTES {
            return Err(EventError::InvalidInvocationKey {
                reason: "too_large",
            });
        }
        Ok(Self { raw_key })
    }

    /// Returns canonical JSON bytes for the invocation key digest material.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(serde_json::json!({
            "domain": Self::DOMAIN,
            "raw_key": self.raw_key,
        }))
    }

    /// Returns the digest that may be recorded in run identity material.
    pub fn digest(&self) -> Result<ContentDigest> {
        Ok(ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            self.canonical_json()?.digest_bytes(),
        ))
    }
}

impl std::fmt::Debug for InvocationKeyMaterialV1<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvocationKeyMaterialV1")
            .field("raw_key", &"<redacted>")
            .finish()
    }
}

/// Hash-defining material for content-addressed run identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunIdentityMaterialV1 {
    /// Certified typed spec hash.
    pub certified_spec_hash: SpecHash,
    /// Store-owned deployment scope.
    pub store_scope_id: StoreScopeId,
    /// Digest of caller-supplied or app-minted invocation material.
    pub invocation_key_digest: ContentDigest,
}

impl RunIdentityMaterialV1 {
    /// Stable canonical domain.
    pub const DOMAIN: &'static str = "mfm.run_identity.v1";

    /// Returns canonical JSON bytes for the hash-defining run identity material.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(serde_json::json!({
            "certified_spec_hash": self.certified_spec_hash.as_str(),
            "invocation_key_digest": self.invocation_key_digest.as_str(),
            "domain": Self::DOMAIN,
            "store_scope_id": self.store_scope_id.as_str(),
        }))
    }

    /// Derives the typed run id from this identity material.
    pub fn derive_run_id(&self) -> Result<RunId> {
        Ok(RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            self.canonical_json()?.digest_bytes(),
        ))
    }
}

/// Run admission event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunAdmitted {
    /// Run id bound to this event stream.
    pub run_id: RunId,
    /// Hash-defining identity material used to derive `run_id`.
    pub identity_material: RunIdentityMaterialV1,
    /// Public entry-point operation evidence selected by app assembly.
    pub entry_point: EntryPointLaunchEvidence,
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Certified spec artifact evidence.
    pub spec_artifact: RunArtifactEvidenceRef,
    /// Certified typed spec certificate artifact evidence.
    pub certificate_artifact: RunArtifactEvidenceRef,
    /// Certified config artifact evidence.
    pub config_artifacts: Vec<RunArtifactEvidenceRef>,
    /// Certified fact descriptor artifact evidence.
    pub fact_descriptor_artifacts: Vec<RunArtifactEvidenceRef>,
    /// Certified spec version.
    pub spec_version: SpecVersion,
    /// Certified lowering version.
    pub lowering_version: LoweringVersion,
    /// Public output schema id.
    pub public_output_schema_id: SchemaId,
    /// Canonical digest of the certified saga policy.
    pub saga_policy_digest: ContentDigest,
    /// Descriptor identities bound to the run.
    pub descriptor_identities: Vec<DescriptorIdentity>,
    /// Runner executable identities.
    pub runner_executables: Vec<ExecutableIdentity>,
    /// Adapter executable identities.
    pub adapter_executables: Vec<ExecutableIdentity>,
    /// Digest binding admitted runtime context and adapter executable identities.
    pub admitted_binding_digest: ContentDigest,
    /// Canonicalizer identity used for the certified spec.
    pub canonicalizer_identity: CanonicalizerIdentity,
    /// Seed cells materialized at run start.
    pub seed_cells: Vec<SeedCellRef>,
}

/// Public entry-point operation evidence bound into `RunAdmitted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPointLaunchEvidence {
    /// Fully resolved app entry-point op id.
    pub resolved_op_id: EntryPointOpId,
    /// Digest of the registered entry-point operation set.
    pub entry_point_registry_digest: ContentDigest,
}

/// Fully qualified app entry-point operation id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryPointOpId(CheckedVisibleAscii256);

impl EntryPointOpId {
    /// Creates a checked entry-point op id.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = value.as_ref();
        CheckedVisibleAscii256::new(value)
            .map(Self)
            .map_err(|_| EventError::InvalidString {
                field: "entry point op id",
                value: value.to_owned(),
            })
    }

    /// Returns the entry-point op id string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Seed cell evidence bound by `RunAdmitted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedCellRef {
    /// Seed id from the certified spec.
    pub seed_id: SeedId,
    /// Cell id from the certified spec.
    pub cell_id: CellId,
    /// Scope id owning the seed cell.
    pub scope_id: ScopeId,
    /// Seed semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Seed schema id.
    pub schema_id: SchemaId,
    /// Canonical seed value digest.
    pub digest: ContentDigest,
    /// Mandatory seed artifact evidence.
    pub seed_artifact: ArtifactEvidenceRef,
}

/// State attempt start event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateAttemptStarted {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id attempted.
    pub node_id: NodeId,
    /// Store-owned attempt id.
    pub attempt_id: AttemptId,
    /// Attempt number for this node.
    pub attempt_no: u32,
    /// State kind being attempted.
    pub state_kind: StateKind,
    /// State version being attempted.
    pub state_version: StateVersion,
}

/// Recorded read fact event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRecorded {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id that produced the fact.
    pub node_id: NodeId,
    /// Attempt id that produced the fact.
    pub attempt_id: AttemptId,
    /// Normalized typed fact claim.
    pub claim: facts::FactClaim,
}

/// Artifact reference event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReferenced {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Optional node id associated with the artifact.
    pub node_id: Option<NodeId>,
    /// Optional attempt id associated with the artifact.
    pub attempt_id: Option<AttemptId>,
    /// Artifact evidence.
    pub artifact_ref: ArtifactEvidenceRef,
}

/// Cell produced terminal event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellProduced {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Producer node id.
    pub node_id: NodeId,
    /// Produced cell id.
    pub cell_id: CellId,
    /// Cell scope id.
    pub scope_id: ScopeId,
    /// Attempt id that produced the cell.
    pub attempt_id: AttemptId,
    /// Cell semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Cell schema id.
    pub schema_id: SchemaId,
    /// Value lineage reference.
    pub value_lineage: ValueLineageRef,
    /// Certified context constraint for this terminal cell.
    pub context: CellContextSpec,
    /// Value artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical value content digest.
    pub content_digest: ContentDigest,
    /// Exact retained-artifact evidence identity for the value artifact.
    pub evidence_hash: ContentDigest,
    /// Producer state kind.
    pub producer_state_kind: Option<StateKind>,
    /// Producer state version.
    pub producer_state_version: Option<StateVersion>,
}

/// Cell skipped terminal event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSkipped {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Producer node id.
    pub node_id: NodeId,
    /// Skipped cell id.
    pub cell_id: CellId,
    /// Cell scope id.
    pub scope_id: ScopeId,
    /// Attempt id that skipped the cell.
    pub attempt_id: AttemptId,
    /// Cell semantic type id.
    pub semantic_type_id: SemanticTypeId,
    /// Cell schema id.
    pub schema_id: SchemaId,
    /// Value lineage reference.
    pub value_lineage: ValueLineageRef,
    /// Certified context constraint for this terminal cell.
    pub context: CellContextSpec,
    /// Typed skip reason.
    pub skip_reason: SkipReason,
}

/// Public output produced event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputProduced {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Public-output render node id.
    pub node_id: NodeId,
    /// Render attempt id.
    pub attempt_id: AttemptId,
    /// Receipt cell id produced by the render node.
    pub receipt_cell_id: CellId,
    /// Public output schema id.
    pub public_schema_id: SchemaId,
    /// Canonical public output spec digest.
    pub output_spec_digest: ContentDigest,
    /// Public output source cells.
    pub cells: Vec<NamedTypedCellRef>,
    /// Canonical rendered output digest.
    pub rendered_digest: ContentDigest,
    /// Optional rendered output artifact id.
    pub rendered_artifact_id: Option<ArtifactId>,
    /// Exact retained-artifact evidence identity for the rendered output artifact, when any.
    pub rendered_artifact_evidence_hash: Option<ContentDigest>,
    /// Renderer descriptor id.
    pub renderer_descriptor_id: DescriptorId,
}

/// Public output render failure audit payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputRenderFailed {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Public-output render node id.
    pub node_id: NodeId,
    /// Render attempt id.
    pub attempt_id: AttemptId,
    /// Public output schema id.
    pub public_schema_id: SchemaId,
    /// Renderer descriptor id.
    pub renderer_descriptor_id: DescriptorId,
    /// Redaction-safe error information.
    pub error: MfmErrorInfo,
}

/// State attempt completed event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateAttemptCompleted {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Completed node id.
    pub node_id: NodeId,
    /// Completed attempt id.
    pub attempt_id: AttemptId,
    /// Output cell id produced by the attempt.
    pub output_cell_id: CellId,
}

/// State attempt interrupted event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateAttemptInterrupted {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Interrupted node id.
    pub node_id: NodeId,
    /// Interrupted attempt id.
    pub attempt_id: AttemptId,
}

/// State attempt failed event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateAttemptFailed {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Failed node id.
    pub node_id: NodeId,
    /// Failed attempt id.
    pub attempt_id: AttemptId,
    /// Whether retry is allowed.
    pub retryable: bool,
    /// Redaction-safe error information.
    pub error: MfmErrorInfo,
}

/// Run completed event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCompleted {
    /// Run id.
    pub run_id: RunId,
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Structurally typed completion outcome.
    pub outcome: RunCompletionOutcome,
}

/// Manual resolution recorded event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualResolutionRecorded {
    /// Run id.
    pub run_id: RunId,
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Operator-selected manual outcome.
    pub outcome: ManualResolutionOutcome,
    /// Evidence schema id.
    pub evidence_schema_id: SchemaId,
    /// Evidence content hash.
    pub evidence_hash: ContentDigest,
    /// Evidence artifact id.
    pub evidence_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the evidence artifact.
    pub evidence_artifact_evidence_hash: ContentDigest,
    /// Authorization proof schema id.
    pub authorization_schema_id: SchemaId,
    /// Authorization proof content hash.
    pub authorization_hash: ContentDigest,
    /// Authorization proof artifact id.
    pub authorization_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the authorization artifact.
    pub authorization_artifact_evidence_hash: ContentDigest,
    /// Optional redaction-safe operator note.
    pub note: Option<ManualResolutionNote>,
}

/// Run completion outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunCompletionOutcome {
    /// Run completed successfully with public-output evidence.
    Completed(Box<PublicOutputCompletionEvidence>),
    /// Confirmed forward side effects were remediated.
    Compensated,
    /// Operator evidence manually resolved the run.
    ManuallyResolved,
    /// Run ended without a compensation or AC/DC-equivalence claim.
    FailedWithoutAcdcClaim,
}

impl RunCompletionOutcome {
    /// Returns the canonical lowercase outcome tag.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Completed(_) => "completed",
            Self::Compensated => "compensated",
            Self::ManuallyResolved => "manually_resolved",
            Self::FailedWithoutAcdcClaim => "failed_without_acdc_claim",
        }
    }
}

/// Public-output evidence required for a completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOutputCompletionEvidence {
    /// Public output schema id.
    pub public_output_schema_id: SchemaId,
    /// Public output event id.
    pub public_output_event_id: EventId,
}

/// Retention references appended event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionRefsAppended {
    /// Run id.
    pub run_id: RunId,
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Retention references appended.
    pub refs: Vec<RetentionRef>,
    /// Retention reason.
    pub reason: RetentionReason,
}

/// Retention manifest projected event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionManifestProjected {
    /// Run id.
    pub run_id: RunId,
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Monotonic manifest sequence.
    pub manifest_seq: u64,
    /// Manifest content digest.
    pub manifest_digest: ContentDigest,
    /// Previous manifest digest, when any.
    pub previous_manifest_digest: Option<ContentDigest>,
    /// Manifest artifact id.
    pub manifest_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the manifest artifact.
    pub manifest_artifact_evidence_hash: ContentDigest,
}

/// Executable identity for state runners, adapters, and framework executables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableIdentity {
    /// Logical runner or adapter factory id.
    pub factory_id: RunnerFactoryId,
    /// Cargo package digest.
    pub cargo_package_digest: ContentDigest,
    /// Binary digest.
    pub binary_digest: ContentDigest,
    /// Optional Nix derivation hash.
    pub nix_derivation_hash: Option<NixDerivationHash>,
    /// Optional Nix output hash.
    pub nix_output_hash: Option<NixOutputHash>,
}

/// Side-effect event payloads.
#[path = "side_effect.rs"]
pub mod side_effect;

#[cfg(test)]
mod tests;
