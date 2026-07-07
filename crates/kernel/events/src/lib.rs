#![warn(missing_docs)]
//! Typed kernel event contracts for MFM.
//!
//! This crate owns the closed v1 event payload set used by typed run storage,
//! replay, resume, public output evidence, and retention projection.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts as facts;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, EventId, IdentityError, LoweringVersion, NodeId,
    PrintableAscii1024 as CheckedPrintableAscii1024, PrintableAscii512 as CheckedPrintableAscii512,
    RunId, SchemaId, ScopeId, SeedId, SemanticTypeId, SideEffectPairId, SpecHash, SpecVersion,
    StateKind, StateVersion, TrustScopeId, VisibleAscii256 as CheckedVisibleAscii256,
};
use mfm_spec::v1::{
    CanonicalizerIdentity, CellContextSpec, CellProducer, DescriptorIdentity, MediaType,
    PublicFieldPath, ResourceNamespace, ValueLineageRef,
};

/// Result type for typed kernel event helpers.
pub type Result<T> = std::result::Result<T, EventError>;

/// Error returned by typed kernel event helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    /// A checked event string field failed validation.
    #[error("invalid {field} string {value:?}")]
    InvalidString {
        /// Field label.
        field: &'static str,
        /// Invalid value.
        value: String,
    },
    /// A public diagnostic field failed redaction-safety validation.
    #[error("invalid public diagnostic {field}: {reason}")]
    InvalidPublicDiagnostic {
        /// Field label.
        field: &'static str,
        /// Stable rejection reason.
        reason: &'static str,
    },
    /// An invocation key failed validation.
    #[error("invalid invocation key: {reason}")]
    InvalidInvocationKey {
        /// Stable rejection reason.
        reason: &'static str,
    },
    /// Identity construction failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// JSON serialization failed before canonicalization.
    #[error("event JSON serialization error: {0}")]
    Serialize(String),
    /// Canonical JSON construction failed.
    #[error("event canonicalization error: {0}")]
    Canonical(String),
}

impl From<IdentityError> for EventError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(&value).map_err(|error| EventError::Serialize(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| EventError::Canonical(error.to_string()))
}

macro_rules! checked_string_type {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(CheckedVisibleAscii256);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                CheckedVisibleAscii256::new(value)
                    .map(Self)
                    .map_err(|_| EventError::InvalidString {
                        field: $field,
                        value: value.to_owned(),
                    })
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

macro_rules! checked_text_type {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(CheckedPrintableAscii1024);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                CheckedPrintableAscii1024::new(value)
                    .map(Self)
                    .map_err(|_| EventError::InvalidString {
                        field: $field,
                        value: value.to_owned(),
                    })
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// Versioned v1 typed kernel event contracts.
pub mod v1 {
    use super::*;

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
        /// Canonical touched-set evidence hash.
        pub evidence_hash: ContentDigest,
        /// Touched-set evidence artifact id.
        pub evidence_artifact_id: ArtifactId,
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
                    side_effect_ledger_ref!(
                        payload,
                        InvocationPrepared,
                        Some(payload.claim_generation)
                    )
                }
                Self::SideEffectInvocationStarted(payload) => {
                    side_effect_ledger_ref!(
                        payload,
                        InvocationStarted,
                        Some(payload.claim_generation)
                    )
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
        /// Store-owned deployment trust scope.
        pub trust_scope_id: TrustScopeId,
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
                "trust_scope_id": self.trust_scope_id.as_str(),
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

    /// Artifact evidence carried directly by run admission.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunArtifactEvidenceRef {
        /// Artifact id.
        pub artifact_id: ArtifactId,
        /// Artifact role.
        pub role: ArtifactRole,
        /// Artifact schema id, when schema-bearing.
        pub schema_id: Option<SchemaId>,
        /// Artifact semantic type id, when value-bearing.
        pub semantic_type_id: Option<SemanticTypeId>,
        /// Artifact content digest.
        pub content_digest: ContentDigest,
        /// Artifact byte length.
        pub byte_len: u64,
        /// Artifact media type.
        pub media_type: MediaType,
    }

    /// Artifact evidence reference persisted in events.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ArtifactEvidenceRef {
        /// Artifact id.
        pub artifact_id: ArtifactId,
        /// Artifact role.
        pub role: ArtifactRole,
        /// Artifact schema id.
        pub schema_id: SchemaId,
        /// Artifact semantic type id, when value-bearing.
        pub semantic_type_id: Option<SemanticTypeId>,
        /// Artifact content digest.
        pub content_digest: ContentDigest,
        /// Artifact byte length.
        pub byte_len: u64,
        /// Artifact media type.
        pub media_type: MediaType,
    }

    /// Artifact role in typed event evidence.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactRole {
        /// Certified typed execution spec artifact.
        TypedExecutionSpec,
        /// Certified typed spec certificate artifact.
        TypedSpecCertificate,
        /// Certified typed config artifact.
        TypedConfig,
        /// Certified fact descriptor authority artifact.
        FactDescriptor,
        /// Seed input artifact.
        SeedInput,
        /// State output value artifact.
        StateOutput,
        /// Read fact response artifact.
        FactResponse,
        /// Private fact query replay evidence artifact.
        FactQueryEvidence,
        /// Side-effect intent artifact.
        SideEffectIntent,
        /// Prepared invocation artifact.
        PreparedInvocation,
        /// Not-submitted proof artifact.
        NotSubmittedProof,
        /// Submission artifact.
        Submission,
        /// Submission-unknown evidence artifact.
        SubmissionUnknownEvidence,
        /// Receipt artifact.
        Receipt,
        /// Confirmation artifact.
        Confirmation,
        /// Ambiguity evidence artifact.
        AmbiguityEvidence,
        /// Manual-resolution evidence artifact.
        ManualResolutionEvidence,
        /// Manual-resolution authorization proof artifact.
        ManualResolutionAuthorization,
        /// Rendered public output artifact.
        PublicOutput,
        /// Redacted diagnostic artifact.
        RedactedDiagnostic,
        /// Retention manifest artifact.
        RetentionManifest,
    }

    /// Schema-field policy required for an artifact role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactSchemaPolicy {
        /// Schema must equal the certified seed schema.
        ExactSeedSchema,
        /// Schema must equal the produced value cell schema.
        ExactValueSchema,
        /// Schema must equal the event evidence schema.
        ExactEvidenceSchema,
        /// Schema must equal the canonical fact descriptor schema.
        ExactFactDescriptorSchema,
        /// Schema must equal the canonical fact query evidence schema.
        ExactFactQueryEvidenceSchema,
        /// Schema must be absent.
        Absent,
        /// Schema must equal the public-output schema.
        ExactPublicSchema,
        /// Schema must equal the redacted diagnostic schema.
        ExactDiagnosticSchema,
    }

    impl ArtifactSchemaPolicy {
        /// Returns the stable policy label used by contract goldens.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::ExactSeedSchema => "exact_seed_schema",
                Self::ExactValueSchema => "exact_value_schema",
                Self::ExactEvidenceSchema => "exact_evidence_schema",
                Self::ExactFactDescriptorSchema => "exact_fact_descriptor_schema",
                Self::ExactFactQueryEvidenceSchema => "exact_fact_query_evidence_schema",
                Self::Absent => "absent",
                Self::ExactPublicSchema => "exact_public_schema",
                Self::ExactDiagnosticSchema => "exact_diagnostic_schema",
            }
        }
    }

    /// Semantic-type-field policy required for an artifact role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactSemanticPolicy {
        /// Semantic type must equal the certified seed semantic type.
        ExactSeedSemantic,
        /// Semantic type must equal the produced value cell semantic type.
        ExactValueSemantic,
        /// Semantic type must be absent.
        Absent,
    }

    impl ArtifactSemanticPolicy {
        /// Returns the stable policy label used by contract goldens.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::ExactSeedSemantic => "exact_seed_semantic",
                Self::ExactValueSemantic => "exact_value_semantic",
                Self::Absent => "absent",
            }
        }
    }

    /// Producer-scope policy required for an artifact role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactProducerScope {
        /// Launch artifact producer may be omitted or non-seed.
        LaunchOrGlobalNoSeed,
        /// Producer seed id is required and node producer must be absent.
        SeedRequired,
        /// Producer node id is required and seed producer must be absent.
        NodeRequired,
        /// Producer node and seed ids must both be absent.
        GlobalNoSeed,
        /// Diagnostic artifacts may carry a node producer, but never a seed producer.
        DiagnosticOptionalNodeNoSeed,
        /// Middleware-produced artifacts must not carry node or seed producers.
        MiddlewareNoSeed,
    }

    impl ArtifactProducerScope {
        /// Returns the stable policy label used by contract goldens.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::LaunchOrGlobalNoSeed => "launch_or_global_no_seed",
                Self::SeedRequired => "seed_required",
                Self::NodeRequired => "node_required",
                Self::GlobalNoSeed => "global_no_seed",
                Self::DiagnosticOptionalNodeNoSeed => "diagnostic_optional_node_no_seed",
                Self::MiddlewareNoSeed => "middleware_no_seed",
            }
        }
    }

    /// Runtime staging class for artifacts with a given role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactStagingClass {
        /// Artifact is admitted as run-start authority.
        RunAdmission,
        /// Attempt-produced state output artifact.
        AttemptStateOutput,
        /// Attempt-produced fact response artifact.
        AttemptFactResponse,
        /// Attempt-produced private fact query replay evidence artifact.
        AttemptFactQueryEvidence,
        /// Side-effect intent artifact.
        SideEffectIntent,
        /// Side-effect prepared-invocation artifact.
        SideEffectPreparedInvocation,
        /// Side-effect not-submitted proof artifact.
        SideEffectNotSubmittedProof,
        /// Side-effect submission artifact.
        SideEffectSubmission,
        /// Side-effect submission-unknown evidence artifact.
        SideEffectSubmissionUnknown,
        /// Side-effect receipt artifact.
        SideEffectReceipt,
        /// Side-effect confirmation artifact.
        SideEffectConfirmation,
        /// Side-effect ambiguity evidence artifact.
        SideEffectAmbiguity,
        /// Manual-resolution artifact.
        ManualResolution,
        /// Attempt-produced public output cache artifact.
        AttemptPublicOutput,
        /// Attempt-produced redacted diagnostic artifact.
        AttemptRedactedDiagnostic,
        /// Middleware-produced retention manifest artifact.
        MiddlewareRetentionManifest,
    }

    impl ArtifactStagingClass {
        /// Returns the stable staging label used by contract goldens.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::RunAdmission => "run_admission",
                Self::AttemptStateOutput => "attempt_state_output",
                Self::AttemptFactResponse => "attempt_fact_response",
                Self::AttemptFactQueryEvidence => "attempt_fact_query_evidence",
                Self::SideEffectIntent => "side_effect_intent",
                Self::SideEffectPreparedInvocation => "side_effect_prepared_invocation",
                Self::SideEffectNotSubmittedProof => "side_effect_not_submitted_proof",
                Self::SideEffectSubmission => "side_effect_submission",
                Self::SideEffectSubmissionUnknown => "side_effect_submission_unknown",
                Self::SideEffectReceipt => "side_effect_receipt",
                Self::SideEffectConfirmation => "side_effect_confirmation",
                Self::SideEffectAmbiguity => "side_effect_ambiguity",
                Self::ManualResolution => "manual_resolution",
                Self::AttemptPublicOutput => "attempt_public_output",
                Self::AttemptRedactedDiagnostic => "attempt_redacted_diagnostic",
                Self::MiddlewareRetentionManifest => "middleware_retention_manifest",
            }
        }
    }

    /// Retention class for artifacts with a given role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactRetentionClass {
        /// Runtime framework does not retain this role directly.
        FrameworkIgnored,
        /// Ordinary value/evidence artifact.
        ValueArtifacts,
        /// Side-effect receipt artifact.
        ReceiptArtifacts,
        /// Side-effect confirmation artifact.
        ConfirmationArtifacts,
        /// Public-output rendered artifact.
        PublicOutputArtifacts,
    }

    impl ArtifactRetentionClass {
        /// Returns the stable retention label used by contract goldens.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::FrameworkIgnored => "framework_ignored",
                Self::ValueArtifacts => "value_artifacts",
                Self::ReceiptArtifacts => "receipt_artifacts",
                Self::ConfirmationArtifacts => "confirmation_artifacts",
                Self::PublicOutputArtifacts => "public_output_artifacts",
            }
        }
    }

    /// Same-commit policy for artifacts with a given role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactSameCommitPolicy {
        /// Artifact evidence is embedded directly in `RunAdmitted`.
        RunAdmittedArtifact,
        /// Seed-cell artifact evidence is embedded in `RunAdmitted`.
        RunAdmittedSeedCell,
        /// Artifact must be admitted with the payload that references it.
        PayloadRequiredArtifact,
        /// Retention manifests must commit with their retention reference.
        RetentionManifestRequiresRef,
    }

    impl ArtifactSameCommitPolicy {
        /// Returns the stable same-commit label used by contract goldens.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::RunAdmittedArtifact => "run_admitted_artifact",
                Self::RunAdmittedSeedCell => "run_admitted_seed_cell",
                Self::PayloadRequiredArtifact => "payload_required_artifact",
                Self::RetentionManifestRequiresRef => "retention_manifest_requires_ref",
            }
        }
    }

    /// Closed contract row for one artifact role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ArtifactRoleContract {
        /// Artifact role.
        pub role: ArtifactRole,
        /// Persisted role tag.
        pub tag: &'static str,
        /// Schema-field policy.
        pub schema: ArtifactSchemaPolicy,
        /// Semantic-type-field policy.
        pub semantic: ArtifactSemanticPolicy,
        /// Producer-scope policy.
        pub producer: ArtifactProducerScope,
        /// Runtime staging class.
        pub staging: ArtifactStagingClass,
        /// Retention class.
        pub retention: ArtifactRetentionClass,
        /// Same-commit admission policy.
        pub same_commit: ArtifactSameCommitPolicy,
    }

    impl ArtifactRole {
        /// All v1 artifact roles in persisted schema/tag order.
        pub const ALL: &'static [Self] = &[
            Self::TypedExecutionSpec,
            Self::TypedSpecCertificate,
            Self::TypedConfig,
            Self::FactDescriptor,
            Self::SeedInput,
            Self::StateOutput,
            Self::FactResponse,
            Self::FactQueryEvidence,
            Self::SideEffectIntent,
            Self::PreparedInvocation,
            Self::NotSubmittedProof,
            Self::Submission,
            Self::SubmissionUnknownEvidence,
            Self::Receipt,
            Self::Confirmation,
            Self::AmbiguityEvidence,
            Self::ManualResolutionEvidence,
            Self::ManualResolutionAuthorization,
            Self::PublicOutput,
            Self::RedactedDiagnostic,
            Self::RetentionManifest,
        ];

        /// Returns this role's closed contract row.
        pub const fn contract(self) -> ArtifactRoleContract {
            match self {
                Self::TypedExecutionSpec => ArtifactRoleContract {
                    role: self,
                    tag: "typed_execution_spec",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::LaunchOrGlobalNoSeed,
                    staging: ArtifactStagingClass::RunAdmission,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RunAdmittedArtifact,
                },
                Self::TypedSpecCertificate => ArtifactRoleContract {
                    role: self,
                    tag: "typed_spec_certificate",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::LaunchOrGlobalNoSeed,
                    staging: ArtifactStagingClass::RunAdmission,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RunAdmittedArtifact,
                },
                Self::TypedConfig => ArtifactRoleContract {
                    role: self,
                    tag: "typed_config",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::LaunchOrGlobalNoSeed,
                    staging: ArtifactStagingClass::RunAdmission,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RunAdmittedArtifact,
                },
                Self::FactDescriptor => ArtifactRoleContract {
                    role: self,
                    tag: "fact_descriptor",
                    schema: ArtifactSchemaPolicy::ExactFactDescriptorSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::LaunchOrGlobalNoSeed,
                    staging: ArtifactStagingClass::RunAdmission,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RunAdmittedArtifact,
                },
                Self::SeedInput => ArtifactRoleContract {
                    role: self,
                    tag: "seed_input",
                    schema: ArtifactSchemaPolicy::ExactSeedSchema,
                    semantic: ArtifactSemanticPolicy::ExactSeedSemantic,
                    producer: ArtifactProducerScope::SeedRequired,
                    staging: ArtifactStagingClass::RunAdmission,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RunAdmittedSeedCell,
                },
                Self::StateOutput => ArtifactRoleContract {
                    role: self,
                    tag: "state_output",
                    schema: ArtifactSchemaPolicy::ExactValueSchema,
                    semantic: ArtifactSemanticPolicy::ExactValueSemantic,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::AttemptStateOutput,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::FactResponse => ArtifactRoleContract {
                    role: self,
                    tag: "fact_response",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::AttemptFactResponse,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::FactQueryEvidence => ArtifactRoleContract {
                    role: self,
                    tag: "fact_query_evidence",
                    schema: ArtifactSchemaPolicy::ExactFactQueryEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::AttemptFactQueryEvidence,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::SideEffectIntent => ArtifactRoleContract {
                    role: self,
                    tag: "side_effect_intent",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectIntent,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::PreparedInvocation => ArtifactRoleContract {
                    role: self,
                    tag: "prepared_invocation",
                    schema: ArtifactSchemaPolicy::Absent,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectPreparedInvocation,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::NotSubmittedProof => ArtifactRoleContract {
                    role: self,
                    tag: "not_submitted_proof",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectNotSubmittedProof,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::Submission => ArtifactRoleContract {
                    role: self,
                    tag: "submission",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectSubmission,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::SubmissionUnknownEvidence => ArtifactRoleContract {
                    role: self,
                    tag: "submission_unknown_evidence",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectSubmissionUnknown,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::Receipt => ArtifactRoleContract {
                    role: self,
                    tag: "receipt",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectReceipt,
                    retention: ArtifactRetentionClass::ReceiptArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::Confirmation => ArtifactRoleContract {
                    role: self,
                    tag: "confirmation",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectConfirmation,
                    retention: ArtifactRetentionClass::ConfirmationArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::AmbiguityEvidence => ArtifactRoleContract {
                    role: self,
                    tag: "ambiguity_evidence",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::SideEffectAmbiguity,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::ManualResolutionEvidence => ArtifactRoleContract {
                    role: self,
                    tag: "manual_resolution_evidence",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::GlobalNoSeed,
                    staging: ArtifactStagingClass::ManualResolution,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::ManualResolutionAuthorization => ArtifactRoleContract {
                    role: self,
                    tag: "manual_resolution_authorization",
                    schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::GlobalNoSeed,
                    staging: ArtifactStagingClass::ManualResolution,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::PublicOutput => ArtifactRoleContract {
                    role: self,
                    tag: "public_output",
                    schema: ArtifactSchemaPolicy::ExactPublicSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::NodeRequired,
                    staging: ArtifactStagingClass::AttemptPublicOutput,
                    retention: ArtifactRetentionClass::PublicOutputArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::RedactedDiagnostic => ArtifactRoleContract {
                    role: self,
                    tag: "redacted_diagnostic",
                    schema: ArtifactSchemaPolicy::ExactDiagnosticSchema,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::DiagnosticOptionalNodeNoSeed,
                    staging: ArtifactStagingClass::AttemptRedactedDiagnostic,
                    retention: ArtifactRetentionClass::ValueArtifacts,
                    same_commit: ArtifactSameCommitPolicy::PayloadRequiredArtifact,
                },
                Self::RetentionManifest => ArtifactRoleContract {
                    role: self,
                    tag: "retention_manifest",
                    schema: ArtifactSchemaPolicy::Absent,
                    semantic: ArtifactSemanticPolicy::Absent,
                    producer: ArtifactProducerScope::MiddlewareNoSeed,
                    staging: ArtifactStagingClass::MiddlewareRetentionManifest,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RetentionManifestRequiresRef,
                },
            }
        }

        /// Returns the stable persisted tag for this role.
        pub const fn as_str(self) -> &'static str {
            self.contract().tag
        }

        /// Parses a stable persisted role tag.
        pub fn parse(value: &str) -> Option<Self> {
            Self::ALL
                .iter()
                .copied()
                .find(|role| role.as_str() == value)
        }
    }

    /// Source of an artifact reference carried by a kernel event.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum EventArtifactReferenceSource {
        /// Typed execution spec artifact from `RunAdmitted`.
        RunSpec,
        /// Typed spec certificate artifact from `RunAdmitted`.
        RunCertificate,
        /// Certified config artifact from `RunAdmitted`.
        RunConfig,
        /// Certified fact descriptor artifact from `RunAdmitted`.
        FactDescriptor,
        /// Seed-cell artifact reference from `RunAdmitted`.
        SeedCell,
        /// Read-fact response artifact.
        FactResponse,
        /// Explicit event artifact reference.
        ArtifactReferenced,
        /// State-output cell artifact.
        StateOutput,
        /// Public-output source cell artifact.
        PublicOutputCell,
        /// Rendered public-output artifact.
        PublicOutputRendered,
        /// Public-output render-failure diagnostic artifact reference.
        PublicOutputRenderFailureDiagnostic,
        /// State-attempt failure diagnostic artifact reference.
        StateAttemptFailureDiagnostic,
        /// Side-effect failure diagnostic artifact reference.
        SideEffectFailureDiagnostic,
        /// Manual-resolution evidence artifact.
        ManualResolutionEvidence,
        /// Manual-resolution authorization proof artifact.
        ManualResolutionAuthorization,
        /// Side-effect intent artifact.
        SideEffectIntent,
        /// Prepared side-effect invocation artifact.
        PreparedInvocation,
        /// Not-submitted proof artifact.
        NotSubmittedProof,
        /// Side-effect submission artifact.
        Submission,
        /// Side-effect submission-unknown evidence artifact.
        SubmissionUnknownEvidence,
        /// Side-effect receipt artifact.
        Receipt,
        /// Side-effect confirmation artifact.
        Confirmation,
        /// Side-effect resource touched-set evidence artifact.
        ResourceTouchedSet,
        /// Side-effect ambiguity evidence artifact.
        AmbiguityEvidence,
        /// Retention reference artifact.
        RetentionRef,
        /// Retention manifest artifact.
        RetentionManifest,
    }

    impl EventArtifactReferenceSource {
        /// Returns true when this source may be the framework terminal-lifecycle receipt.
        pub fn is_terminal_lifecycle_receipt_candidate(self) -> bool {
            matches!(self, Self::ArtifactReferenced | Self::StateOutput)
        }

        /// Returns true when this source belongs to retention-only metadata events.
        pub fn is_retention(self) -> bool {
            matches!(self, Self::RetentionRef | Self::RetentionManifest)
        }

        /// Returns true when this source is payload evidence that may authorize same-commit
        /// runtime retention refs.
        pub fn is_same_commit_payload_evidence(self) -> bool {
            matches!(
                self,
                Self::FactResponse
                    | Self::StateOutput
                    | Self::PublicOutputRendered
                    | Self::PublicOutputRenderFailureDiagnostic
                    | Self::StateAttemptFailureDiagnostic
                    | Self::SideEffectFailureDiagnostic
                    | Self::ManualResolutionEvidence
                    | Self::ManualResolutionAuthorization
                    | Self::SideEffectIntent
                    | Self::PreparedInvocation
                    | Self::NotSubmittedProof
                    | Self::Submission
                    | Self::SubmissionUnknownEvidence
                    | Self::Receipt
                    | Self::Confirmation
                    | Self::AmbiguityEvidence
            )
        }
    }

    /// Artifact evidence requirement derived from a single kernel event payload.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct EventArtifactRequirement {
        /// Event field that referenced the artifact.
        pub source: EventArtifactReferenceSource,
        /// Referenced artifact id.
        pub artifact_id: ArtifactId,
        /// Expected content digest, when the event carries one.
        pub digest: Option<ContentDigest>,
        /// Expected byte length, when the event carries one.
        pub byte_len: Option<u64>,
        /// Expected media type, when the event carries one.
        pub media_type: Option<MediaType>,
        /// Expected schema id, when the event carries one.
        pub schema_id: Option<SchemaId>,
        /// Expected semantic type id, when the event carries one.
        pub semantic_type_id: Option<SemanticTypeId>,
        /// Expected producer node id, when applicable.
        pub producer_node_id: Option<NodeId>,
        /// Expected producer seed id, when applicable.
        pub producer_seed_id: Option<SeedId>,
        /// Expected artifact role, when the event carries or implies one.
        pub artifact_role: Option<ArtifactRole>,
    }

    fn event_artifact_requirements(payload: &KernelEventPayload) -> Vec<EventArtifactRequirement> {
        let mut requirements = Vec::new();
        match payload {
            KernelEventPayload::RunAdmitted(payload) => {
                push_run_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::RunSpec,
                    &payload.spec_artifact,
                );
                push_run_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::RunCertificate,
                    &payload.certificate_artifact,
                );
                for artifact in &payload.config_artifacts {
                    push_run_artifact(
                        &mut requirements,
                        EventArtifactReferenceSource::RunConfig,
                        artifact,
                    );
                }
                for artifact in &payload.fact_descriptor_artifacts {
                    push_run_artifact(
                        &mut requirements,
                        EventArtifactReferenceSource::FactDescriptor,
                        artifact,
                    );
                }
                for seed in &payload.seed_cells {
                    push_event_artifact(
                        &mut requirements,
                        EventArtifactReferenceSource::SeedCell,
                        &seed.seed_artifact,
                        None,
                        Some(seed.seed_id.clone()),
                    );
                }
            }
            KernelEventPayload::FactRecorded(payload) => {
                let response = payload.claim.response();
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::FactResponse,
                    artifact_id: response.artifact_id().clone(),
                    digest: Some(response.response_hash().clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(response.response_schema_id().clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::FactResponse),
                });
            }
            KernelEventPayload::ArtifactReferenced(payload) => {
                push_event_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::ArtifactReferenced,
                    &payload.artifact_ref,
                    payload.node_id.clone(),
                    None,
                );
            }
            KernelEventPayload::CellProduced(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::StateOutput,
                    artifact_id: payload.artifact_id.clone(),
                    digest: Some(payload.content_digest.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.schema_id.clone()),
                    semantic_type_id: Some(payload.semantic_type_id.clone()),
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::StateOutput),
                });
            }
            KernelEventPayload::PublicOutputProduced(payload) => {
                for cell in &payload.cells {
                    let (producer_node_id, producer_seed_id) =
                        cell_producer_artifact_owner(&cell.producer);
                    requirements.push(EventArtifactRequirement {
                        source: EventArtifactReferenceSource::PublicOutputCell,
                        artifact_id: cell.artifact_id.clone(),
                        digest: Some(cell.content_digest.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: Some(cell.schema_id.clone()),
                        semantic_type_id: Some(cell.semantic_type_id.clone()),
                        producer_node_id,
                        producer_seed_id,
                        artifact_role: None,
                    });
                }
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    requirements.push(EventArtifactRequirement {
                        source: EventArtifactReferenceSource::PublicOutputRendered,
                        artifact_id: artifact_id.clone(),
                        digest: Some(payload.rendered_digest.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: Some(payload.public_schema_id.clone()),
                        semantic_type_id: None,
                        producer_node_id: Some(payload.node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: Some(ArtifactRole::PublicOutput),
                    });
                }
            }
            KernelEventPayload::PublicOutputRenderFailed(payload) => {
                if let Some(ref evidence) = payload.error.diagnostic_ref {
                    push_event_artifact(
                        &mut requirements,
                        EventArtifactReferenceSource::PublicOutputRenderFailureDiagnostic,
                        evidence,
                        Some(payload.node_id.clone()),
                        None,
                    );
                }
            }
            KernelEventPayload::StateAttemptFailed(payload) => {
                if let Some(ref evidence) = payload.error.diagnostic_ref {
                    push_event_artifact(
                        &mut requirements,
                        EventArtifactReferenceSource::StateAttemptFailureDiagnostic,
                        evidence,
                        Some(payload.node_id.clone()),
                        None,
                    );
                }
            }
            KernelEventPayload::ManualResolutionRecorded(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::ManualResolutionEvidence,
                    artifact_id: payload.evidence_artifact_id.clone(),
                    digest: Some(payload.evidence_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.evidence_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::ManualResolutionEvidence),
                });
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::ManualResolutionAuthorization,
                    artifact_id: payload.authorization_artifact_id.clone(),
                    digest: Some(payload.authorization_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.authorization_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::ManualResolutionAuthorization),
                });
            }
            KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
                RunCompletionOutcome::Completed(_) => {}
                RunCompletionOutcome::Compensated
                | RunCompletionOutcome::ManuallyResolved
                | RunCompletionOutcome::FailedWithoutAcdcClaim => {}
            },
            KernelEventPayload::SideEffectIntentPersisted(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::SideEffectIntent,
                    artifact_id: payload.intent_artifact_id.clone(),
                    digest: Some(payload.intent_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.intent_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::SideEffectIntent),
                });
            }
            KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                if let (Some(artifact_id), Some(hash)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    requirements.push(EventArtifactRequirement {
                        source: EventArtifactReferenceSource::PreparedInvocation,
                        artifact_id: artifact_id.clone(),
                        digest: Some(hash.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: None,
                        semantic_type_id: None,
                        producer_node_id: Some(payload.node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: Some(ArtifactRole::PreparedInvocation),
                    });
                }
            }
            KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::NotSubmittedProof,
                    artifact_id: payload.proof_artifact_id.clone(),
                    digest: Some(payload.proof_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.proof_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::NotSubmittedProof),
                });
            }
            KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::Submission,
                    artifact_id: payload.submission_artifact_id.clone(),
                    digest: Some(payload.submission_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.submission_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::Submission),
                });
            }
            KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::SubmissionUnknownEvidence,
                    artifact_id: payload.evidence_artifact_id.clone(),
                    digest: Some(payload.evidence_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.evidence_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::SubmissionUnknownEvidence),
                });
            }
            KernelEventPayload::SideEffectReceiptObserved(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::Receipt,
                    artifact_id: payload.receipt_artifact_id.clone(),
                    digest: Some(payload.receipt_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.receipt_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::Receipt),
                });
                if let Some(touched_set) = &payload.resource_touched_set {
                    push_resource_touched_set_requirement(&mut requirements, touched_set);
                }
            }
            KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::Confirmation,
                    artifact_id: payload.confirmation_artifact_id.clone(),
                    digest: Some(payload.confirmation_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.confirmation_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::Confirmation),
                });
                if let Some(touched_set) = &payload.resource_touched_set {
                    push_resource_touched_set_requirement(&mut requirements, touched_set);
                }
            }
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::AmbiguityEvidence,
                    artifact_id: payload.evidence_artifact_id.clone(),
                    digest: Some(payload.evidence_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.evidence_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::AmbiguityEvidence),
                });
            }
            KernelEventPayload::SideEffectFailed(payload) => {
                if let Some(ref evidence) = payload.error.diagnostic_ref {
                    push_event_artifact(
                        &mut requirements,
                        EventArtifactReferenceSource::SideEffectFailureDiagnostic,
                        evidence,
                        Some(payload.node_id.clone()),
                        None,
                    );
                }
            }
            KernelEventPayload::RetentionRefsAppended(payload) => {
                for retention_ref in &payload.refs {
                    requirements.push(EventArtifactRequirement {
                        source: EventArtifactReferenceSource::RetentionRef,
                        artifact_id: retention_ref.artifact_id.clone(),
                        digest: Some(retention_ref.content_digest.clone()),
                        byte_len: None,
                        media_type: None,
                        schema_id: None,
                        semantic_type_id: None,
                        producer_node_id: None,
                        producer_seed_id: None,
                        artifact_role: Some(retention_ref.role),
                    });
                }
            }
            KernelEventPayload::RetentionManifestProjected(payload) => {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::RetentionManifest,
                    artifact_id: payload.manifest_artifact_id.clone(),
                    digest: Some(payload.manifest_digest.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::RetentionManifest),
                });
            }
            KernelEventPayload::StateAttemptStarted(_)
            | KernelEventPayload::CellSkipped(_)
            | KernelEventPayload::SideEffectClaimed(_)
            | KernelEventPayload::SideEffectClaimTakenOver(_)
            | KernelEventPayload::ResourceLaneClaimed(_)
            | KernelEventPayload::ResourceLaneClaimIntent(_)
            | KernelEventPayload::SideEffectInvocationStarted(_)
            | KernelEventPayload::ResourceLaneReleased(_)
            | KernelEventPayload::ResourceLaneReleaseIntent(_)
            | KernelEventPayload::StateAttemptCompleted(_)
            | KernelEventPayload::StateAttemptInterrupted(_) => {}
        }
        requirements
    }

    fn push_run_artifact(
        requirements: &mut Vec<EventArtifactRequirement>,
        source: EventArtifactReferenceSource,
        evidence: &RunArtifactEvidenceRef,
    ) {
        requirements.push(EventArtifactRequirement {
            source,
            artifact_id: evidence.artifact_id.clone(),
            digest: Some(evidence.content_digest.clone()),
            byte_len: Some(evidence.byte_len),
            media_type: Some(evidence.media_type.clone()),
            schema_id: evidence.schema_id.clone(),
            semantic_type_id: evidence.semantic_type_id.clone(),
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: Some(evidence.role),
        });
    }

    fn push_event_artifact(
        requirements: &mut Vec<EventArtifactRequirement>,
        source: EventArtifactReferenceSource,
        evidence: &ArtifactEvidenceRef,
        producer_node_id: Option<NodeId>,
        producer_seed_id: Option<SeedId>,
    ) {
        requirements.push(EventArtifactRequirement {
            source,
            artifact_id: evidence.artifact_id.clone(),
            digest: Some(evidence.content_digest.clone()),
            byte_len: Some(evidence.byte_len),
            media_type: Some(evidence.media_type.clone()),
            schema_id: Some(evidence.schema_id.clone()),
            semantic_type_id: evidence.semantic_type_id.clone(),
            producer_node_id,
            producer_seed_id,
            artifact_role: Some(evidence.role),
        });
    }

    fn push_resource_touched_set_requirement(
        requirements: &mut Vec<EventArtifactRequirement>,
        evidence: &ResourceTouchedSetEvidence,
    ) {
        requirements.push(EventArtifactRequirement {
            source: EventArtifactReferenceSource::ResourceTouchedSet,
            artifact_id: evidence.evidence_artifact_id.clone(),
            digest: Some(evidence.evidence_hash.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(evidence.evidence_schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: None,
        });
    }

    fn cell_producer_artifact_owner(producer: &CellProducer) -> (Option<NodeId>, Option<SeedId>) {
        match producer {
            CellProducer::Node(node_id) => (Some(node_id.clone()), None),
            CellProducer::Seed(seed_id) => (None, Some(seed_id.clone())),
        }
    }

    /// Named typed cell reference used by public output events.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct NamedTypedCellRef {
        /// Public output field path.
        pub public_field_path: PublicFieldPath,
        /// Cell id.
        pub cell_id: CellId,
        /// Cell producer.
        pub producer: CellProducer,
        /// Cell scope id.
        pub scope_id: ScopeId,
        /// Cell semantic type id.
        pub semantic_type_id: SemanticTypeId,
        /// Cell schema id.
        pub schema_id: SchemaId,
        /// Value lineage reference.
        pub value_lineage: ValueLineageRef,
        /// Cell value content digest.
        pub content_digest: ContentDigest,
        /// Cell value artifact id.
        pub artifact_id: ArtifactId,
    }

    /// Redaction-safe error information.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MfmErrorInfo {
        /// Stable error code.
        pub code: ErrorCode,
        /// Error category.
        pub category: ErrorCategory,
        /// Whether retry is allowed.
        pub retryable: bool,
        /// Public safe message.
        pub safe_message: String,
        /// Optional redacted public details digest.
        pub public_details: Option<RedactedJson>,
        /// Optional redacted diagnostic artifact reference.
        pub diagnostic_ref: Option<ArtifactEvidenceRef>,
    }

    impl MfmErrorInfo {
        /// Creates redaction-safe error information without optional details or diagnostics.
        pub fn new(
            code: ErrorCode,
            category: ErrorCategory,
            retryable: bool,
            safe_message: impl Into<String>,
        ) -> Result<Self> {
            let error = Self {
                code,
                category,
                retryable,
                safe_message: safe_message.into(),
                public_details: None,
                diagnostic_ref: None,
            };
            error.validate()?;
            Ok(error)
        }

        /// Adds a redacted public details digest after validating the full public diagnostic.
        pub fn with_public_details(mut self, public_details: RedactedJson) -> Result<Self> {
            self.public_details = Some(public_details);
            self.validate()?;
            Ok(self)
        }

        /// Adds a redacted diagnostic artifact reference after validating the full diagnostic.
        pub fn with_diagnostic_ref(mut self, diagnostic_ref: ArtifactEvidenceRef) -> Result<Self> {
            self.diagnostic_ref = Some(diagnostic_ref);
            self.validate()?;
            Ok(self)
        }

        /// Validates that persisted public diagnostics are redaction-safe.
        pub fn validate(&self) -> Result<()> {
            validate_public_error_text("safe_message", &self.safe_message)?;
            if let Some(details) = &self.public_details {
                details.validate()?;
            }
            if let Some(diagnostic_ref) = &self.diagnostic_ref {
                validate_public_diagnostic_ref(diagnostic_ref)?;
            }
            Ok(())
        }
    }

    /// Error category.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ErrorCategory {
        /// Planning or certification error.
        Planning,
        /// Input, config, or value validation error.
        Validation,
        /// Capability or adapter error.
        Capability,
        /// External side-effect error.
        SideEffect,
        /// Runtime infrastructure error.
        Runtime,
        /// Storage or replay error.
        Storage,
        /// Cancellation.
        Cancelled,
    }

    /// Redacted JSON public details reference.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RedactedJson {
        /// Canonical digest of the redacted public details.
        pub content_digest: ContentDigest,
    }

    impl RedactedJson {
        /// Creates a redacted public details digest reference.
        pub fn new(content_digest: ContentDigest) -> Self {
            Self { content_digest }
        }

        /// Validates the public-details reference.
        pub fn validate(&self) -> Result<()> {
            let _ = &self.content_digest;
            Ok(())
        }
    }

    const SECRET_SHAPED_DIAGNOSTIC_MARKERS: &[&str] = &[
        "private key",
        "private_key",
        "mnemonic",
        "password",
        "passphrase",
        "seed phrase",
        "seed_phrase",
        "api key",
        "api_key",
        "authorization:",
        "bearer ",
        "raw transaction",
        "raw_transaction",
        "raw_tx",
        "signed payload",
        "signed_payload",
        "keystore path",
        "keystore_path",
        "rpc url",
        "rpc_url",
        "secret=",
        "token=",
        "-----begin",
    ];

    fn validate_public_error_text(field: &'static str, value: &str) -> Result<()> {
        CheckedPrintableAscii512::new(value).map_err(|error| {
            EventError::InvalidPublicDiagnostic {
                field,
                reason: public_error_text_reason(error.reason()),
            }
        })?;
        if contains_secret_shaped_diagnostic(value) {
            return Err(EventError::InvalidPublicDiagnostic {
                field,
                reason: "message resembles secret material",
            });
        }
        Ok(())
    }

    fn public_error_text_reason(reason: &mfm_ids::CheckedStringErrorReason) -> &'static str {
        match reason {
            mfm_ids::CheckedStringErrorReason::Empty => "message must not be empty",
            mfm_ids::CheckedStringErrorReason::TooLong { .. } => {
                "message exceeds public length limit"
            }
            mfm_ids::CheckedStringErrorReason::SegmentTooLong { .. }
            | mfm_ids::CheckedStringErrorReason::InvalidCharacter { .. }
            | mfm_ids::CheckedStringErrorReason::InvalidStart
            | mfm_ids::CheckedStringErrorReason::InvalidEnd
            | mfm_ids::CheckedStringErrorReason::MissingSeparator { .. }
            | mfm_ids::CheckedStringErrorReason::EmptySegment
            | mfm_ids::CheckedStringErrorReason::ReservedPrefix { .. } => {
                "message must be printable ASCII"
            }
        }
    }

    fn contains_secret_shaped_diagnostic(value: &str) -> bool {
        let lower = value.to_ascii_lowercase();
        SECRET_SHAPED_DIAGNOSTIC_MARKERS
            .iter()
            .any(|marker| lower.contains(marker))
            || lower
                .split(is_public_diagnostic_token_boundary)
                .any(|token| is_key_shaped_token(token) || is_hex_secret_shaped_token(token))
    }

    fn is_public_diagnostic_token_boundary(ch: char) -> bool {
        !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    }

    fn is_key_shaped_token(token: &str) -> bool {
        token.starts_with("sk_")
            || token.starts_with("akia")
            || token.starts_with("ghp_")
            || token.starts_with("github_pat_")
            || token.starts_with("xoxb-")
    }

    fn is_hex_secret_shaped_token(token: &str) -> bool {
        let token = token.strip_prefix("0x").unwrap_or(token);
        token.len() >= 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    fn validate_public_diagnostic_ref(reference: &ArtifactEvidenceRef) -> Result<()> {
        if reference.role != ArtifactRole::RedactedDiagnostic {
            return Err(EventError::InvalidPublicDiagnostic {
                field: "diagnostic_ref",
                reason: "diagnostic artifact role must be redacted_diagnostic",
            });
        }
        if reference.semantic_type_id.is_some() {
            return Err(EventError::InvalidPublicDiagnostic {
                field: "diagnostic_ref",
                reason: "diagnostic artifact semantic type must be absent",
            });
        }
        Ok(())
    }

    /// Typed skip reason.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SkipReason {
        /// Stable skip code.
        pub code: ErrorCode,
        /// Public safe message.
        pub safe_message: String,
    }

    /// Retained artifact reference.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RetentionRef {
        /// Artifact id retained.
        pub artifact_id: ArtifactId,
        /// Artifact role retained.
        pub role: ArtifactRole,
        /// Artifact content digest.
        pub content_digest: ContentDigest,
    }

    /// Retention reason.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum RetentionReason {
        /// Initial certified run retention.
        RunAdmitted,
        /// Runtime artifact retention.
        RuntimeEvidence,
        /// Public output retention.
        PublicOutput,
        /// Manifest compaction/projection retention.
        ManifestProjection,
    }

    /// Side-effect event payloads.
    pub mod side_effect {
        use super::*;

        checked_string_type!(
            /// Store-owned opaque claim fencing token.
            ClaimFencingToken,
            "claim fencing token"
        );

        /// Side-effect intent persisted event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct IntentPersisted {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Scope id.
            pub scope_id: ScopeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
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
            pub idempotency_key: IdempotencyKeyRef,
            /// Capability kind.
            pub capability_kind: CapabilityKind,
            /// Capability version.
            pub capability_version: CapabilityVersion,
            /// Adapter kind.
            pub adapter_kind: AdapterKind,
            /// Adapter version.
            pub adapter_version: AdapterVersion,
        }

        /// Side-effect claim acquired event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct Claimed {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Claim owner.
            pub claim_owner: RunnerInvocationId,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Claim generation.
            pub claim_generation: u32,
            /// Claim fencing token.
            pub claim_fencing_token: ClaimFencingToken,
        }

        /// Side-effect claim takeover event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct ClaimTakenOver {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Previous claim owner.
            pub previous_claim_owner: RunnerInvocationId,
            /// New claim owner.
            pub new_claim_owner: RunnerInvocationId,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Previous claim generation.
            pub previous_claim_generation: u32,
            /// New claim generation.
            pub claim_generation: u32,
            /// Claim fencing token.
            pub claim_fencing_token: ClaimFencingToken,
        }

        /// Side-effect invocation prepared event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct InvocationPrepared {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Claim generation.
            pub claim_generation: u32,
            /// Claim fencing token.
            pub claim_fencing_token: ClaimFencingToken,
            /// Optional exclusive resource lane key evidence echoed from the held lane.
            pub resource_key: Option<ResourceKeyEvidence>,
            /// Optional prepared artifact id.
            pub prepared_artifact_id: Option<ArtifactId>,
            /// Optional prepared artifact hash.
            pub prepared_hash: Option<ContentDigest>,
        }

        /// Side-effect invocation started event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct InvocationStarted {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Claim owner.
            pub claim_owner: RunnerInvocationId,
            /// Claim generation.
            pub claim_generation: u32,
            /// Claim fencing token.
            pub claim_fencing_token: ClaimFencingToken,
        }

        /// Side-effect not-submitted proof event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct NotSubmittedProven {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Proof schema id.
            pub proof_schema_id: SchemaId,
            /// Proof hash.
            pub proof_hash: ContentDigest,
            /// Proof artifact id.
            pub proof_artifact_id: ArtifactId,
        }

        /// Side-effect submission observed event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct SubmissionObserved {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Submission schema id.
            pub submission_schema_id: SchemaId,
            /// Submission hash.
            pub submission_hash: ContentDigest,
            /// Submission artifact id.
            pub submission_artifact_id: ArtifactId,
        }

        /// Side-effect submission unknown event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct SubmissionUnknown {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Evidence schema id.
            pub evidence_schema_id: SchemaId,
            /// Evidence hash.
            pub evidence_hash: ContentDigest,
            /// Evidence artifact id.
            pub evidence_artifact_id: ArtifactId,
        }

        /// Side-effect receipt observed event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct ReceiptObserved {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Receipt schema id.
            pub receipt_schema_id: SchemaId,
            /// Receipt hash.
            pub receipt_hash: ContentDigest,
            /// Receipt artifact id.
            pub receipt_artifact_id: ArtifactId,
            /// Replay verifier id.
            pub replay_verifier_id: ReplayVerifierId,
            /// Optional exact touched-set evidence.
            pub resource_touched_set: Option<ResourceTouchedSetEvidence>,
        }

        /// Side-effect confirmation observed event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct ConfirmationObserved {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Confirmation schema id.
            pub confirmation_schema_id: SchemaId,
            /// Confirmation hash.
            pub confirmation_hash: ContentDigest,
            /// Confirmation artifact id.
            pub confirmation_artifact_id: ArtifactId,
            /// Replay verifier id.
            pub replay_verifier_id: ReplayVerifierId,
            /// Optional exact touched-set evidence.
            pub resource_touched_set: Option<ResourceTouchedSetEvidence>,
        }

        /// Side-effect ambiguous event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct Ambiguous {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Ambiguity code.
            pub ambiguity_code: AmbiguityCode,
            /// Evidence schema id.
            pub evidence_schema_id: SchemaId,
            /// Evidence hash.
            pub evidence_hash: ContentDigest,
            /// Evidence artifact id.
            pub evidence_artifact_id: ArtifactId,
        }

        /// Side-effect failed event payload.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct Failed {
            /// Certified typed spec hash.
            pub spec_hash: SpecHash,
            /// Node id.
            pub node_id: NodeId,
            /// Attempt id.
            pub attempt_id: AttemptId,
            /// Side-effect ledger key.
            pub ledger_key: SideEffectLedgerKey,
            /// Side-effect ledger purpose.
            pub ledger_purpose: SideEffectLedgerPurpose,
            /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
            pub pair_id: SideEffectPairId,
            /// Pair phase authority, when this ledger is bound to a paired forward node.
            pub pair_role: SideEffectPairRole,
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Failure phase.
            pub failure_phase: FailurePhase,
            /// Whether retry is allowed.
            pub retryable: bool,
            /// Redaction-safe error information.
            pub error: MfmErrorInfo,
        }

        /// Legal side-effect failure phase.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum FailurePhase {
            /// Failure before invocation started.
            BeforeInvocationStarted,
            /// Failure after not-submitted was proven.
            AfterNotSubmittedProven,
        }
    }

    /// Field cardinality in an event schema descriptor.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum EventFieldCardinality {
        /// Required scalar field.
        Required,
        /// Optional field.
        Optional,
        /// Repeated field.
        Repeated,
    }

    impl EventFieldCardinality {
        const fn as_str(self) -> &'static str {
            match self {
                Self::Required => "required",
                Self::Optional => "optional",
                Self::Repeated => "repeated",
            }
        }
    }

    fn schema_field(
        name: &'static str,
        type_name: &'static str,
        cardinality: EventFieldCardinality,
    ) -> serde_json::Value {
        serde_json::json!({
            "cardinality": cardinality.as_str(),
            "name": name,
            "type": event_type_schema(type_name),
        })
    }

    fn scalar_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "kind": "scalar",
            "name": type_name,
        })
    }

    fn visible_ascii_256_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "kind": "mfm_ids_visible_ascii",
            "max_len": 256,
            "name": type_name,
            "non_empty": true,
        })
    }

    fn stable_author_key_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "kind": "mfm_ids_stable_author_key",
            "name": type_name,
            "persisted_as": "string",
        })
    }

    fn public_field_path_type() -> serde_json::Value {
        serde_json::json!({
            "kind": "mfm_ids_field_path",
            "name": "PublicFieldPath",
            "persisted_as": "string",
        })
    }

    fn resource_namespace_type() -> serde_json::Value {
        serde_json::json!({
            "kind": "mfm_ids_resource_namespace",
            "name": "ResourceNamespace",
            "persisted_as": "string",
        })
    }

    fn mfm_identity_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "kind": "mfm_ids_identity",
            "name": type_name,
            "persisted_as": "string",
        })
    }

    fn mfm_version_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "kind": "mfm_ids_version",
            "name": type_name,
            "persisted_as": "string",
        })
    }

    fn external_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "kind": "external_contract_type",
            "name": type_name,
        })
    }

    fn struct_type(name: &'static str, fields: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({
            "fields": fields,
            "kind": "struct",
            "name": name,
        })
    }

    fn unit_enum_type(name: &'static str, variants: &[&'static str]) -> serde_json::Value {
        serde_json::json!({
            "kind": "enum",
            "name": name,
            "variants": variants
                .iter()
                .map(|variant| serde_json::json!({
                    "fields": [],
                    "name": variant,
                }))
                .collect::<Vec<_>>(),
        })
    }

    fn artifact_role_schema_variants() -> Vec<&'static str> {
        ArtifactRole::ALL.iter().map(|role| role.as_str()).collect()
    }

    fn enum_type(name: &'static str, variants: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({
            "kind": "enum",
            "name": name,
            "variants": variants,
        })
    }

    fn enum_variant(name: &'static str, fields: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({
            "fields": fields,
            "name": name,
        })
    }

    fn event_type_schema(type_name: &'static str) -> serde_json::Value {
        match type_name {
            "bool" | "u32" | "u64" => scalar_type(type_name),
            "String" | "PlainCanonicalJsonBytes" => serde_json::json!({
                "kind": "string",
                "name": type_name,
            }),
            "AdapterKind" | "ArtifactId" | "AttemptId" | "CapabilityKind" | "CellId"
            | "ContentDigest" | "ContextRef" | "DescriptorId" | "EffectKind" | "EventId"
            | "NodeId" | "OperationKind" | "RunId" | "SchemaId" | "ScopeId" | "SeedId"
            | "SemanticTypeId" | "SideEffectPairId" | "SpecHash" | "StateKind" | "TrustScopeId" => {
                mfm_identity_type(type_name)
            }
            "AdapterVersion" | "CapabilityVersion" | "LoweringVersion" | "OperationVersion"
            | "SpecVersion" | "StateVersion" => mfm_version_type(type_name),
            "DigestAlgorithm" => unit_enum_type("DigestAlgorithm", &["sha256-jcs-v1"]),
            "SideEffectPairRole" => unit_enum_type("SideEffectPairRole", &["submit", "verify"]),
            "ResourceLaneReleaseAuthority" => unit_enum_type(
                "ResourceLaneReleaseAuthority",
                &["verify_terminal", "manual_resolution"],
            ),
            "MediaType"
            | "CanonicalizerIdentity"
            | "ContextResourceKind"
            | "ContextStage"
            | "RendererVersion" => visible_ascii_256_type(type_name),
            "PublicFieldPath" => public_field_path_type(),
            "ResourceNamespace" => resource_namespace_type(),
            "RendererKind" => stable_author_key_type(type_name),
            "RunnerFactoryId"
            | "NixDerivationHash"
            | "NixOutputHash"
            | "SideEffectLedgerKey"
            | "ResourceKey"
            | "ResourceLaneClaimId"
            | "ResourceLaneReleaseId"
            | "ResourceLaneReleaseReason"
            | "RunnerInvocationId"
            | "IdempotencyKeyRef"
            | "ReplayVerifierId"
            | "AmbiguityCode"
            | "ErrorCode"
            | "ClaimFencingToken"
            | "EntryPointOpId" => visible_ascii_256_type(type_name),
            "EntryPointLaunchEvidence" => struct_type(
                "EntryPointLaunchEvidence",
                vec![
                    schema_field(
                        "resolved_op_id",
                        "EntryPointOpId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "entry_point_registry_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "FactKind" | "FactKey" => visible_ascii_256_type(type_name),
            "FactAudience" => unit_enum_type("FactAudience", &["control", "platform"]),
            "FactVisibilityScope" => unit_enum_type("FactVisibilityScope", &["default"]),
            "FactVisibility" => enum_type(
                "FactVisibility",
                vec![
                    enum_variant("run_private", Vec::new()),
                    enum_variant(
                        "indexed",
                        vec![
                            schema_field(
                                "audience",
                                "FactAudience",
                                EventFieldCardinality::Required,
                            ),
                            schema_field(
                                "scope",
                                "FactVisibilityScope",
                                EventFieldCardinality::Required,
                            ),
                        ],
                    ),
                ],
            ),
            "FactSubjectEvidence" => struct_type(
                "FactSubjectEvidence",
                vec![
                    schema_field(
                        "fact_subject_namespace_hash",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "subject_material",
                        "PlainCanonicalJsonBytes",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "subject_material_hash",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("fact_key", "FactKey", EventFieldCardinality::Required),
                ],
            ),
            "FactRequestEvidence" => struct_type(
                "FactRequestEvidence",
                vec![
                    schema_field(
                        "request_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "request_hash",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "FactResponseEvidence" => struct_type(
                "FactResponseEvidence",
                vec![
                    schema_field(
                        "response_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "response_hash",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                    schema_field(
                        "artifact_evidence_hash",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "FactProducerProvenance" => struct_type(
                "FactProducerProvenance",
                vec![
                    schema_field(
                        "capability_kind",
                        "CapabilityKind",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "capability_version",
                        "CapabilityVersion",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "adapter_kind",
                        "AdapterKind",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "adapter_version",
                        "AdapterVersion",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "FactClaim" => struct_type(
                "FactClaim",
                vec![
                    schema_field(
                        "visibility",
                        "FactVisibility",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("fact_kind", "FactKind", EventFieldCardinality::Required),
                    schema_field(
                        "fact_descriptor_hash",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "subject",
                        "FactSubjectEvidence",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("observed_at", "String", EventFieldCardinality::Optional),
                    schema_field(
                        "request",
                        "FactRequestEvidence",
                        EventFieldCardinality::Optional,
                    ),
                    schema_field(
                        "response",
                        "FactResponseEvidence",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "producer",
                        "FactProducerProvenance",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "RunIdentityMaterialV1" => struct_type(
                "RunIdentityMaterialV1",
                vec![
                    schema_field(
                        "certified_spec_hash",
                        "SpecHash",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "trust_scope_id",
                        "TrustScopeId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "invocation_key_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "DescriptorIdentity" => enum_type(
                "DescriptorIdentity",
                vec![
                    enum_variant(
                        "state",
                        vec![schema_field(
                            "identity",
                            "StateDescriptorIdentity",
                            EventFieldCardinality::Required,
                        )],
                    ),
                    enum_variant(
                        "operation",
                        vec![schema_field(
                            "identity",
                            "OperationDescriptorIdentity",
                            EventFieldCardinality::Required,
                        )],
                    ),
                    enum_variant(
                        "renderer",
                        vec![schema_field(
                            "identity",
                            "RendererDescriptorIdentity",
                            EventFieldCardinality::Required,
                        )],
                    ),
                ],
            ),
            "StateDescriptorIdentity" => struct_type(
                "StateDescriptorIdentity",
                vec![
                    schema_field(
                        "descriptor_id",
                        "DescriptorId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("state_kind", "StateKind", EventFieldCardinality::Required),
                    schema_field(
                        "state_version",
                        "StateVersion",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "config_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "input_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "output_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "output_semantic_type_id",
                        "SemanticTypeId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("effect_kind", "EffectKind", EventFieldCardinality::Required),
                    schema_field(
                        "capabilities",
                        "CapabilitySetDescriptor",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "emitted_fact_descriptors",
                        "FactDescriptorRef",
                        EventFieldCardinality::Repeated,
                    ),
                ],
            ),
            "FactDescriptorRef" => struct_type(
                "FactDescriptorRef",
                vec![schema_field(
                    "descriptor_hash",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                )],
            ),
            "OperationDescriptorIdentity" => struct_type(
                "OperationDescriptorIdentity",
                vec![
                    schema_field(
                        "descriptor_id",
                        "DescriptorId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "operation_kind",
                        "OperationKind",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "operation_version",
                        "OperationVersion",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "config_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "input_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "output_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("expansion_abi", "String", EventFieldCardinality::Required),
                ],
            ),
            "RendererDescriptorIdentity" => struct_type(
                "RendererDescriptorIdentity",
                vec![
                    schema_field(
                        "descriptor_id",
                        "DescriptorId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "renderer_kind",
                        "RendererKind",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "renderer_version",
                        "RendererVersion",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "public_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "canonicalizer_identity",
                        "CanonicalizerIdentity",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "CapabilitySetDescriptor" => struct_type(
                "CapabilitySetDescriptor",
                vec![schema_field(
                    "capabilities",
                    "CapabilityDescriptor",
                    EventFieldCardinality::Repeated,
                )],
            ),
            "CapabilityDescriptor" => struct_type(
                "CapabilityDescriptor",
                vec![
                    schema_field("kind", "CapabilityKind", EventFieldCardinality::Required),
                    schema_field(
                        "version",
                        "CapabilityVersion",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("role", "CapabilityRole", EventFieldCardinality::Required),
                    schema_field("name", "String", EventFieldCardinality::Required),
                ],
            ),
            "CapabilityRole" => unit_enum_type(
                "CapabilityRole",
                &[
                    "read_external",
                    "managed_platform_write",
                    "support",
                    "external_mutation_authority",
                ],
            ),
            "SeedCellRef" => struct_type(
                "SeedCellRef",
                vec![
                    schema_field("seed_id", "SeedId", EventFieldCardinality::Required),
                    schema_field("cell_id", "CellId", EventFieldCardinality::Required),
                    schema_field("scope_id", "ScopeId", EventFieldCardinality::Required),
                    schema_field(
                        "semantic_type_id",
                        "SemanticTypeId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("schema_id", "SchemaId", EventFieldCardinality::Required),
                    schema_field("digest", "ContentDigest", EventFieldCardinality::Required),
                    schema_field(
                        "seed_artifact",
                        "ArtifactEvidenceRef",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "RunArtifactEvidenceRef" => struct_type(
                "RunArtifactEvidenceRef",
                vec![
                    schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                    schema_field("role", "ArtifactRole", EventFieldCardinality::Required),
                    schema_field("schema_id", "SchemaId", EventFieldCardinality::Optional),
                    schema_field(
                        "semantic_type_id",
                        "SemanticTypeId",
                        EventFieldCardinality::Optional,
                    ),
                    schema_field(
                        "content_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("byte_len", "u64", EventFieldCardinality::Required),
                    schema_field("media_type", "MediaType", EventFieldCardinality::Required),
                ],
            ),
            "ExecutableIdentity" => struct_type(
                "ExecutableIdentity",
                vec![
                    schema_field(
                        "factory_id",
                        "RunnerFactoryId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "cargo_package_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "binary_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "nix_derivation_hash",
                        "NixDerivationHash",
                        EventFieldCardinality::Optional,
                    ),
                    schema_field(
                        "nix_output_hash",
                        "NixOutputHash",
                        EventFieldCardinality::Optional,
                    ),
                ],
            ),
            "ArtifactEvidenceRef" => struct_type(
                "ArtifactEvidenceRef",
                vec![
                    schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                    schema_field("role", "ArtifactRole", EventFieldCardinality::Required),
                    schema_field("schema_id", "SchemaId", EventFieldCardinality::Required),
                    schema_field(
                        "semantic_type_id",
                        "SemanticTypeId",
                        EventFieldCardinality::Optional,
                    ),
                    schema_field(
                        "content_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("byte_len", "u64", EventFieldCardinality::Required),
                    schema_field("media_type", "MediaType", EventFieldCardinality::Required),
                ],
            ),
            "ArtifactRole" => unit_enum_type("ArtifactRole", &artifact_role_schema_variants()),
            "ResourceKeyEvidence" => struct_type(
                "ResourceKeyEvidence",
                vec![
                    schema_field(
                        "namespace",
                        "ResourceNamespace",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("key_schema_id", "SchemaId", EventFieldCardinality::Required),
                    schema_field("key", "ResourceKey", EventFieldCardinality::Required),
                ],
            ),
            "ResourceTouchedSetEvidence" => struct_type(
                "ResourceTouchedSetEvidence",
                vec![
                    schema_field(
                        "namespace",
                        "ResourceNamespace",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "evidence_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "evidence_hash",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "evidence_artifact_id",
                        "ArtifactId",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "NamedTypedCellRef" => struct_type(
                "NamedTypedCellRef",
                vec![
                    schema_field(
                        "public_field_path",
                        "PublicFieldPath",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("cell_id", "CellId", EventFieldCardinality::Required),
                    schema_field("producer", "CellProducer", EventFieldCardinality::Required),
                    schema_field("scope_id", "ScopeId", EventFieldCardinality::Required),
                    schema_field(
                        "semantic_type_id",
                        "SemanticTypeId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("schema_id", "SchemaId", EventFieldCardinality::Required),
                    schema_field(
                        "value_lineage",
                        "ValueLineageRef",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "content_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                    schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                ],
            ),
            "CellProducer" => enum_type(
                "CellProducer",
                vec![
                    enum_variant(
                        "node",
                        vec![schema_field(
                            "node_id",
                            "NodeId",
                            EventFieldCardinality::Required,
                        )],
                    ),
                    enum_variant(
                        "seed",
                        vec![schema_field(
                            "seed_id",
                            "SeedId",
                            EventFieldCardinality::Required,
                        )],
                    ),
                ],
            ),
            "ValueLineageRef" => struct_type(
                "ValueLineageRef",
                vec![schema_field(
                    "lineage_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                )],
            ),
            "ContextProducerSpec" => struct_type(
                "ContextProducerSpec",
                vec![
                    schema_field(
                        "producer_descriptor_ids",
                        "DescriptorId",
                        EventFieldCardinality::Repeated,
                    ),
                    schema_field(
                        "seed_producers_allowed",
                        "bool",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "CellContextSpec" => enum_type(
                "CellContextSpec",
                vec![
                    enum_variant("no_context", Vec::new()),
                    enum_variant(
                        "bound",
                        vec![
                            schema_field(
                                "context_ref",
                                "ContextRef",
                                EventFieldCardinality::Required,
                            ),
                            schema_field(
                                "resource_kind",
                                "ContextResourceKind",
                                EventFieldCardinality::Required,
                            ),
                            schema_field("stage", "ContextStage", EventFieldCardinality::Required),
                            schema_field(
                                "producer",
                                "ContextProducerSpec",
                                EventFieldCardinality::Required,
                            ),
                        ],
                    ),
                ],
            ),
            "MfmErrorInfo" => struct_type(
                "MfmErrorInfo",
                vec![
                    schema_field("code", "ErrorCode", EventFieldCardinality::Required),
                    schema_field("category", "ErrorCategory", EventFieldCardinality::Required),
                    schema_field("retryable", "bool", EventFieldCardinality::Required),
                    schema_field("safe_message", "String", EventFieldCardinality::Required),
                    schema_field(
                        "public_details",
                        "RedactedJson",
                        EventFieldCardinality::Optional,
                    ),
                    schema_field(
                        "diagnostic_ref",
                        "ArtifactEvidenceRef",
                        EventFieldCardinality::Optional,
                    ),
                ],
            ),
            "ErrorCategory" => unit_enum_type(
                "ErrorCategory",
                &[
                    "planning",
                    "validation",
                    "capability",
                    "side_effect",
                    "runtime",
                    "storage",
                    "cancelled",
                ],
            ),
            "RedactedJson" => struct_type(
                "RedactedJson",
                vec![schema_field(
                    "content_digest",
                    "ContentDigest",
                    EventFieldCardinality::Required,
                )],
            ),
            "SkipReason" => struct_type(
                "SkipReason",
                vec![
                    schema_field("code", "ErrorCode", EventFieldCardinality::Required),
                    schema_field("safe_message", "String", EventFieldCardinality::Required),
                ],
            ),
            "RetentionRef" => struct_type(
                "RetentionRef",
                vec![
                    schema_field("artifact_id", "ArtifactId", EventFieldCardinality::Required),
                    schema_field("role", "ArtifactRole", EventFieldCardinality::Required),
                    schema_field(
                        "content_digest",
                        "ContentDigest",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "RetentionReason" => unit_enum_type(
                "RetentionReason",
                &[
                    "run_admitted",
                    "runtime_evidence",
                    "public_output",
                    "manifest_projection",
                ],
            ),
            "PublicOutputCompletionEvidence" => struct_type(
                "PublicOutputCompletionEvidence",
                vec![
                    schema_field(
                        "public_output_schema_id",
                        "SchemaId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "public_output_event_id",
                        "EventId",
                        EventFieldCardinality::Required,
                    ),
                ],
            ),
            "SideEffectLedgerPurpose" => enum_type(
                "SideEffectLedgerPurpose",
                vec![
                    enum_variant("forward", Vec::new()),
                    enum_variant(
                        "remediation",
                        vec![schema_field(
                            "forward_pair_id",
                            "SideEffectPairId",
                            EventFieldCardinality::Required,
                        )],
                    ),
                ],
            ),
            "ManualResolutionOutcome" => unit_enum_type(
                "ManualResolutionOutcome",
                &["confirm_remediated", "fail_without_acdc_claim"],
            ),
            "ManualResolutionNote" => struct_type(
                "ManualResolutionNote",
                vec![schema_field(
                    "text",
                    "String",
                    EventFieldCardinality::Required,
                )],
            ),
            "RunCompletionOutcome" => enum_type(
                "RunCompletionOutcome",
                vec![
                    enum_variant(
                        "completed",
                        vec![schema_field(
                            "public_output",
                            "PublicOutputCompletionEvidence",
                            EventFieldCardinality::Required,
                        )],
                    ),
                    enum_variant("compensated", Vec::new()),
                    enum_variant("manually_resolved", Vec::new()),
                    enum_variant("failed_without_acdc_claim", Vec::new()),
                ],
            ),
            "FailurePhase" => unit_enum_type(
                "FailurePhase",
                &["before_invocation_started", "after_not_submitted_proven"],
            ),
            name => external_type(name),
        }
    }

    /// Field descriptor for a typed event payload schema.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EventFieldDescriptor {
        /// Persisted field name.
        pub name: &'static str,
        /// Field type name expanded into the hash-defining structural schema.
        pub type_name: &'static str,
        /// Field cardinality.
        pub cardinality: EventFieldCardinality,
    }

    impl EventFieldDescriptor {
        const fn required(name: &'static str, type_name: &'static str) -> Self {
            Self {
                name,
                type_name,
                cardinality: EventFieldCardinality::Required,
            }
        }

        const fn optional(name: &'static str, type_name: &'static str) -> Self {
            Self {
                name,
                type_name,
                cardinality: EventFieldCardinality::Optional,
            }
        }

        const fn repeated(name: &'static str, type_name: &'static str) -> Self {
            Self {
                name,
                type_name,
                cardinality: EventFieldCardinality::Repeated,
            }
        }

        fn json(self) -> serde_json::Value {
            schema_field(self.name, self.type_name, self.cardinality)
        }
    }

    /// Hash-defining event schema descriptor.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EventSchemaDescriptor {
        /// Stable schema name.
        pub schema_name: &'static str,
        /// Stable Rust payload path.
        pub rust_type_path: &'static str,
        /// Schema version.
        pub schema_version: &'static str,
        /// Event fields.
        pub fields: &'static [EventFieldDescriptor],
    }

    impl EventSchemaDescriptor {
        /// Returns canonical JSON bytes for the hash-defining event schema descriptor.
        pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
            canonical_json(serde_json::json!({
                "canonicalization": DigestAlgorithm::Sha256JcsV1.as_str(),
                "fields": self.fields.iter().copied().map(EventFieldDescriptor::json).collect::<Vec<_>>(),
                "schema_family": "mfm.kernel.event",
                "schema_name": self.schema_name,
                "schema_version": self.schema_version,
            }))
        }

        /// Returns the schema id derived from this event schema descriptor.
        pub fn schema_id(&self) -> Result<SchemaId> {
            let digest = self.canonical_json()?.digest_bytes();
            Ok(SchemaId::new(
                self.schema_name,
                self.schema_version,
                DigestAlgorithm::Sha256JcsV1,
                digest,
            )?)
        }
    }

    /// Returns every closed v1 event schema descriptor.
    pub fn all_event_schema_descriptors() -> Vec<EventSchemaDescriptor> {
        vec![
            RUN_ADMITTED_SCHEMA,
            STATE_ATTEMPT_STARTED_SCHEMA,
            FACT_RECORDED_SCHEMA,
            ARTIFACT_REFERENCED_SCHEMA,
            CELL_PRODUCED_SCHEMA,
            CELL_SKIPPED_SCHEMA,
            SIDE_EFFECT_INTENT_PERSISTED_SCHEMA,
            SIDE_EFFECT_CLAIMED_SCHEMA,
            SIDE_EFFECT_CLAIM_TAKEN_OVER_SCHEMA,
            RESOURCE_LANE_CLAIMED_SCHEMA,
            RESOURCE_LANE_CLAIM_INTENT_SCHEMA,
            SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA,
            SIDE_EFFECT_INVOCATION_STARTED_SCHEMA,
            SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA,
            SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA,
            SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA,
            SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA,
            SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA,
            SIDE_EFFECT_AMBIGUOUS_SCHEMA,
            SIDE_EFFECT_FAILED_SCHEMA,
            RESOURCE_LANE_RELEASED_SCHEMA,
            RESOURCE_LANE_RELEASE_INTENT_SCHEMA,
            PUBLIC_OUTPUT_PRODUCED_SCHEMA,
            PUBLIC_OUTPUT_RENDER_FAILED_SCHEMA,
            STATE_ATTEMPT_COMPLETED_SCHEMA,
            STATE_ATTEMPT_INTERRUPTED_SCHEMA,
            STATE_ATTEMPT_FAILED_SCHEMA,
            MANUAL_RESOLUTION_RECORDED_SCHEMA,
            RUN_COMPLETED_SCHEMA,
            RETENTION_REFS_APPENDED_SCHEMA,
            RETENTION_MANIFEST_PROJECTED_SCHEMA,
        ]
    }

    macro_rules! fields {
        ($($field:expr),+ $(,)?) => {
            &[$($field),+]
        };
    }

    const RUN_ADMITTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.run_admitted",
        rust_type_path: "mfm_events::v1::RunAdmitted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("run_id", "RunId"),
            EventFieldDescriptor::required("identity_material", "RunIdentityMaterialV1"),
            EventFieldDescriptor::required("entry_point", "EntryPointLaunchEvidence"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("spec_artifact", "RunArtifactEvidenceRef"),
            EventFieldDescriptor::required("certificate_artifact", "RunArtifactEvidenceRef"),
            EventFieldDescriptor::repeated("config_artifacts", "RunArtifactEvidenceRef"),
            EventFieldDescriptor::repeated("fact_descriptor_artifacts", "RunArtifactEvidenceRef"),
            EventFieldDescriptor::required("spec_version", "SpecVersion"),
            EventFieldDescriptor::required("lowering_version", "LoweringVersion"),
            EventFieldDescriptor::required("public_output_schema_id", "SchemaId"),
            EventFieldDescriptor::repeated("descriptor_identities", "DescriptorIdentity"),
            EventFieldDescriptor::repeated("runner_executables", "ExecutableIdentity"),
            EventFieldDescriptor::repeated("adapter_executables", "ExecutableIdentity"),
            EventFieldDescriptor::required("admitted_binding_digest", "ContentDigest"),
            EventFieldDescriptor::required("canonicalizer_identity", "CanonicalizerIdentity"),
            EventFieldDescriptor::repeated("seed_cells", "SeedCellRef"),
        ],
    };

    const STATE_ATTEMPT_STARTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.state_attempt_started",
        rust_type_path: "mfm_events::v1::StateAttemptStarted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("attempt_no", "u32"),
            EventFieldDescriptor::required("state_kind", "StateKind"),
            EventFieldDescriptor::required("state_version", "StateVersion"),
        ],
    };

    const FACT_RECORDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.fact_recorded",
        rust_type_path: "mfm_events::v1::FactRecorded",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("claim", "FactClaim"),
        ],
    };

    const ARTIFACT_REFERENCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.artifact_referenced",
        rust_type_path: "mfm_events::v1::ArtifactReferenced",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::optional("node_id", "NodeId"),
            EventFieldDescriptor::optional("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("artifact_ref", "ArtifactEvidenceRef"),
        ],
    };

    const CELL_PRODUCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.cell_produced",
        rust_type_path: "mfm_events::v1::CellProduced",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("cell_id", "CellId"),
            EventFieldDescriptor::required("scope_id", "ScopeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("semantic_type_id", "SemanticTypeId"),
            EventFieldDescriptor::required("schema_id", "SchemaId"),
            EventFieldDescriptor::required("value_lineage", "ValueLineageRef"),
            EventFieldDescriptor::required("context", "CellContextSpec"),
            EventFieldDescriptor::required("artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("content_digest", "ContentDigest"),
            EventFieldDescriptor::optional("producer_state_kind", "StateKind"),
            EventFieldDescriptor::optional("producer_state_version", "StateVersion"),
        ],
    };

    const CELL_SKIPPED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.cell_skipped",
        rust_type_path: "mfm_events::v1::CellSkipped",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("cell_id", "CellId"),
            EventFieldDescriptor::required("scope_id", "ScopeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("semantic_type_id", "SemanticTypeId"),
            EventFieldDescriptor::required("schema_id", "SchemaId"),
            EventFieldDescriptor::required("value_lineage", "ValueLineageRef"),
            EventFieldDescriptor::required("context", "CellContextSpec"),
            EventFieldDescriptor::required("skip_reason", "SkipReason"),
        ],
    };

    const SIDE_EFFECT_INTENT_PERSISTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.intent_persisted",
        rust_type_path: "mfm_events::v1::side_effect::IntentPersisted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("scope_id", "ScopeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("intent_schema_id", "SchemaId"),
            EventFieldDescriptor::required("intent_hash", "ContentDigest"),
            EventFieldDescriptor::required("intent_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("idempotency_input_schema_id", "SchemaId"),
            EventFieldDescriptor::required("idempotency_input_hash", "ContentDigest"),
            EventFieldDescriptor::required("idempotency_key", "IdempotencyKeyRef"),
            EventFieldDescriptor::required("capability_kind", "CapabilityKind"),
            EventFieldDescriptor::required("capability_version", "CapabilityVersion"),
            EventFieldDescriptor::required("adapter_kind", "AdapterKind"),
            EventFieldDescriptor::required("adapter_version", "AdapterVersion"),
        ],
    };

    const SIDE_EFFECT_CLAIMED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.claimed",
        rust_type_path: "mfm_events::v1::side_effect::Claimed",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
        ],
    };

    const SIDE_EFFECT_CLAIM_TAKEN_OVER_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.claim_taken_over",
        rust_type_path: "mfm_events::v1::side_effect::ClaimTakenOver",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("previous_claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("new_claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("previous_claim_generation", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
        ],
    };

    const RESOURCE_LANE_CLAIMED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.resource_lane.claimed",
        rust_type_path: "mfm_events::v1::ResourceLaneClaimed",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("resource_key", "ResourceKeyEvidence"),
            EventFieldDescriptor::required("requirement_digest", "ContentDigest"),
            EventFieldDescriptor::required("resolved_by_capability_impl", "RunnerFactoryId"),
            EventFieldDescriptor::required("claim_id", "ResourceLaneClaimId"),
            EventFieldDescriptor::required("claim_fencing_token", "u64"),
            EventFieldDescriptor::required("lane_transition_seq", "u64"),
        ],
    };

    const RESOURCE_LANE_CLAIM_INTENT_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.resource_lane.claim_intent",
        rust_type_path: "mfm_events::v1::ResourceLaneClaimIntent",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("resource_key", "ResourceKeyEvidence"),
            EventFieldDescriptor::required("requirement_digest", "ContentDigest"),
            EventFieldDescriptor::required("resolved_by_capability_impl", "RunnerFactoryId"),
        ],
    };

    const SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.invocation_prepared",
        rust_type_path: "mfm_events::v1::side_effect::InvocationPrepared",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
            EventFieldDescriptor::optional("resource_key", "ResourceKeyEvidence"),
            EventFieldDescriptor::optional("prepared_artifact_id", "ArtifactId"),
            EventFieldDescriptor::optional("prepared_hash", "ContentDigest"),
        ],
    };

    const SIDE_EFFECT_INVOCATION_STARTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.invocation_started",
        rust_type_path: "mfm_events::v1::side_effect::InvocationStarted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
        ],
    };

    const SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.not_submitted_proven",
        rust_type_path: "mfm_events::v1::side_effect::NotSubmittedProven",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("proof_schema_id", "SchemaId"),
            EventFieldDescriptor::required("proof_hash", "ContentDigest"),
            EventFieldDescriptor::required("proof_artifact_id", "ArtifactId"),
        ],
    };

    const SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.submission_observed",
        rust_type_path: "mfm_events::v1::side_effect::SubmissionObserved",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("submission_schema_id", "SchemaId"),
            EventFieldDescriptor::required("submission_hash", "ContentDigest"),
            EventFieldDescriptor::required("submission_artifact_id", "ArtifactId"),
        ],
    };

    const SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.submission_unknown",
        rust_type_path: "mfm_events::v1::side_effect::SubmissionUnknown",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("evidence_schema_id", "SchemaId"),
            EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("evidence_artifact_id", "ArtifactId"),
        ],
    };

    const SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.receipt_observed",
        rust_type_path: "mfm_events::v1::side_effect::ReceiptObserved",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("receipt_schema_id", "SchemaId"),
            EventFieldDescriptor::required("receipt_hash", "ContentDigest"),
            EventFieldDescriptor::required("receipt_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
            EventFieldDescriptor::optional("resource_touched_set", "ResourceTouchedSetEvidence",),
        ],
    };

    const SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.confirmation_observed",
        rust_type_path: "mfm_events::v1::side_effect::ConfirmationObserved",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("confirmation_schema_id", "SchemaId"),
            EventFieldDescriptor::required("confirmation_hash", "ContentDigest"),
            EventFieldDescriptor::required("confirmation_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
            EventFieldDescriptor::optional("resource_touched_set", "ResourceTouchedSetEvidence",),
        ],
    };

    const SIDE_EFFECT_AMBIGUOUS_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.ambiguous",
        rust_type_path: "mfm_events::v1::side_effect::Ambiguous",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("ambiguity_code", "AmbiguityCode"),
            EventFieldDescriptor::required("evidence_schema_id", "SchemaId"),
            EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("evidence_artifact_id", "ArtifactId"),
        ],
    };

    const SIDE_EFFECT_FAILED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.side_effect.failed",
        rust_type_path: "mfm_events::v1::side_effect::Failed",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("failure_phase", "FailurePhase"),
            EventFieldDescriptor::required("retryable", "bool"),
            EventFieldDescriptor::required("error", "MfmErrorInfo"),
        ],
    };

    const RESOURCE_LANE_RELEASED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.resource_lane.released",
        rust_type_path: "mfm_events::v1::ResourceLaneReleased",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_id", "ResourceLaneClaimId"),
            EventFieldDescriptor::required("release_id", "ResourceLaneReleaseId"),
            EventFieldDescriptor::required("claim_fencing_token", "u64"),
            EventFieldDescriptor::required("release_authority", "ResourceLaneReleaseAuthority",),
            EventFieldDescriptor::required("release_reason", "ResourceLaneReleaseReason"),
            EventFieldDescriptor::required("lane_transition_seq", "u64"),
        ],
    };

    const RESOURCE_LANE_RELEASE_INTENT_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.resource_lane.release_intent",
        rust_type_path: "mfm_events::v1::ResourceLaneReleaseIntent",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("ledger_key", "SideEffectLedgerKey"),
            EventFieldDescriptor::required("ledger_purpose", "SideEffectLedgerPurpose"),
            EventFieldDescriptor::required("pair_id", "SideEffectPairId"),
            EventFieldDescriptor::required("pair_role", "SideEffectPairRole"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_id", "ResourceLaneClaimId"),
            EventFieldDescriptor::required("release_authority", "ResourceLaneReleaseAuthority",),
            EventFieldDescriptor::required("release_reason", "ResourceLaneReleaseReason"),
        ],
    };

    const PUBLIC_OUTPUT_PRODUCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.public_output_produced",
        rust_type_path: "mfm_events::v1::PublicOutputProduced",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("receipt_cell_id", "CellId"),
            EventFieldDescriptor::required("public_schema_id", "SchemaId"),
            EventFieldDescriptor::required("output_spec_digest", "ContentDigest"),
            EventFieldDescriptor::repeated("cells", "NamedTypedCellRef"),
            EventFieldDescriptor::required("rendered_digest", "ContentDigest"),
            EventFieldDescriptor::optional("rendered_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("renderer_descriptor_id", "DescriptorId"),
        ],
    };

    const PUBLIC_OUTPUT_RENDER_FAILED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.public_output_render_failed",
        rust_type_path: "mfm_events::v1::PublicOutputRenderFailed",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("public_schema_id", "SchemaId"),
            EventFieldDescriptor::required("renderer_descriptor_id", "DescriptorId"),
            EventFieldDescriptor::required("error", "MfmErrorInfo"),
        ],
    };

    const STATE_ATTEMPT_COMPLETED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.state_attempt_completed",
        rust_type_path: "mfm_events::v1::StateAttemptCompleted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("output_cell_id", "CellId"),
        ],
    };

    const STATE_ATTEMPT_INTERRUPTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.state_attempt_interrupted",
        rust_type_path: "mfm_events::v1::StateAttemptInterrupted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
        ],
    };

    const STATE_ATTEMPT_FAILED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.state_attempt_failed",
        rust_type_path: "mfm_events::v1::StateAttemptFailed",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("node_id", "NodeId"),
            EventFieldDescriptor::required("attempt_id", "AttemptId"),
            EventFieldDescriptor::required("retryable", "bool"),
            EventFieldDescriptor::required("error", "MfmErrorInfo"),
        ],
    };

    const MANUAL_RESOLUTION_RECORDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.manual_resolution_recorded",
        rust_type_path: "mfm_events::v1::ManualResolutionRecorded",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("run_id", "RunId"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("outcome", "ManualResolutionOutcome"),
            EventFieldDescriptor::required("evidence_schema_id", "SchemaId"),
            EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("evidence_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("authorization_schema_id", "SchemaId"),
            EventFieldDescriptor::required("authorization_hash", "ContentDigest"),
            EventFieldDescriptor::required("authorization_artifact_id", "ArtifactId"),
            EventFieldDescriptor::optional("note", "ManualResolutionNote"),
        ],
    };

    const RUN_COMPLETED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.run_completed",
        rust_type_path: "mfm_events::v1::RunCompleted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("run_id", "RunId"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("outcome", "RunCompletionOutcome"),
        ],
    };

    const RETENTION_REFS_APPENDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.retention_refs_appended",
        rust_type_path: "mfm_events::v1::RetentionRefsAppended",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("run_id", "RunId"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::repeated("refs", "RetentionRef"),
            EventFieldDescriptor::required("reason", "RetentionReason"),
        ],
    };

    const RETENTION_MANIFEST_PROJECTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.retention_manifest_projected",
        rust_type_path: "mfm_events::v1::RetentionManifestProjected",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("run_id", "RunId"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("manifest_seq", "u64"),
            EventFieldDescriptor::required("manifest_digest", "ContentDigest"),
            EventFieldDescriptor::optional("previous_manifest_digest", "ContentDigest"),
            EventFieldDescriptor::required("manifest_artifact_id", "ArtifactId"),
        ],
    };

    #[cfg(test)]
    mod tests;
}
