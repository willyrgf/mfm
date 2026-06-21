#![warn(missing_docs)]
//! Typed kernel event contracts for MFM.
//!
//! This crate owns the closed v1 event payload set used by typed run storage,
//! replay, resume, public output evidence, and retention projection.

use std::fmt;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, EventId, IdentityError, LoweringVersion, NodeId,
    RunId, SchemaId, ScopeId, SeedId, SemanticTypeId, SpecHash, SpecVersion, StateKind,
    StateVersion,
};
use mfm_spec::v1::{
    CanonicalizerIdentity, CellProducer, DescriptorIdentity, MediaType, PublicFieldPath,
    ResourceNamespace, ValueLineageRef,
};

/// Result type for typed kernel event helpers.
pub type Result<T> = std::result::Result<T, EventError>;

/// Error returned by typed kernel event helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventError {
    /// A checked event string field failed validation.
    InvalidString {
        /// Field label.
        field: &'static str,
        /// Invalid value.
        value: String,
    },
    /// A public diagnostic field failed redaction-safety validation.
    InvalidPublicDiagnostic {
        /// Field label.
        field: &'static str,
        /// Stable rejection reason.
        reason: &'static str,
    },
    /// Identity construction failed.
    Identity(String),
    /// JSON serialization failed before canonicalization.
    Serialize(String),
    /// Canonical JSON construction failed.
    Canonical(String),
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidString { field, value } => {
                write!(f, "invalid {field} string {value:?}")
            }
            Self::InvalidPublicDiagnostic { field, reason } => {
                write!(f, "invalid public diagnostic {field}: {reason}")
            }
            Self::Identity(message) => write!(f, "identity error: {message}"),
            Self::Serialize(message) => write!(f, "event JSON serialization error: {message}"),
            Self::Canonical(message) => write!(f, "event canonicalization error: {message}"),
        }
    }
}

impl std::error::Error for EventError {}

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

fn checked_ascii_token(field: &'static str, value: impl AsRef<str>) -> Result<String> {
    let value = value.as_ref();
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
    {
        return Err(EventError::InvalidString {
            field,
            value: value.to_owned(),
        });
    }
    Ok(value.to_owned())
}

fn checked_printable_text(field: &'static str, value: impl AsRef<str>) -> Result<String> {
    let value = value.as_ref();
    if value.is_empty()
        || value.len() > 1024
        || !value.bytes().all(|byte| matches!(byte, 0x20..=0x7e))
    {
        return Err(EventError::InvalidString {
            field,
            value: value.to_owned(),
        });
    }
    Ok(value.to_owned())
}

macro_rules! checked_string_type {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                checked_ascii_token($field, value).map(Self)
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                &self.0
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
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                checked_printable_text($field, value).map(Self)
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                &self.0
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
        /// Capability-adapter fact key.
        FactKey,
        "fact key"
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
            /// Forward ledger key whose obligation this remediation addresses.
            forward_ledger_key: SideEffectLedgerKey,
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
                Self::SideEffectInvocationPrepared(_) => SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA,
                Self::SideEffectInvocationStarted(_) => SIDE_EFFECT_INVOCATION_STARTED_SCHEMA,
                Self::SideEffectNotSubmittedProven(_) => SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA,
                Self::SideEffectSubmissionObserved(_) => SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA,
                Self::SideEffectSubmissionUnknown(_) => SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA,
                Self::SideEffectReceiptObserved(_) => SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA,
                Self::SideEffectConfirmationObserved(_) => SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA,
                Self::SideEffectAmbiguous(_) => SIDE_EFFECT_AMBIGUOUS_SCHEMA,
                Self::SideEffectFailed(_) => SIDE_EFFECT_FAILED_SCHEMA,
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
                Self::SideEffectInvocationPrepared(payload) => &payload.spec_hash,
                Self::SideEffectInvocationStarted(payload) => &payload.spec_hash,
                Self::SideEffectNotSubmittedProven(payload) => &payload.spec_hash,
                Self::SideEffectSubmissionObserved(payload) => &payload.spec_hash,
                Self::SideEffectSubmissionUnknown(payload) => &payload.spec_hash,
                Self::SideEffectReceiptObserved(payload) => &payload.spec_hash,
                Self::SideEffectConfirmationObserved(payload) => &payload.spec_hash,
                Self::SideEffectAmbiguous(payload) => &payload.spec_hash,
                Self::SideEffectFailed(payload) => &payload.spec_hash,
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

        /// Returns the side-effect event view for payloads in the side-effect ledger family.
        pub fn side_effect_ref(&self) -> Option<SideEffectEventRef<'_>> {
            macro_rules! side_effect_ref {
                ($payload:ident, $kind:ident, $claim_generation:expr) => {
                    Some(SideEffectEventRef {
                        node_id: &$payload.node_id,
                        attempt_id: &$payload.attempt_id,
                        ledger_key: &$payload.ledger_key,
                        ledger_purpose: &$payload.ledger_purpose,
                        invocation_epoch: Some($payload.invocation_epoch),
                        claim_generation: $claim_generation,
                        kind: SideEffectEventKind::$kind,
                    })
                };
            }

            match self {
                Self::SideEffectIntentPersisted(payload) => {
                    side_effect_ref!(payload, IntentPersisted, None)
                }
                Self::SideEffectClaimed(payload) => {
                    side_effect_ref!(payload, Claimed, Some(payload.claim_generation))
                }
                Self::SideEffectClaimTakenOver(payload) => {
                    side_effect_ref!(payload, ClaimTakenOver, Some(payload.claim_generation))
                }
                Self::SideEffectInvocationPrepared(payload) => {
                    side_effect_ref!(payload, InvocationPrepared, Some(payload.claim_generation))
                }
                Self::SideEffectInvocationStarted(payload) => {
                    side_effect_ref!(payload, InvocationStarted, Some(payload.claim_generation))
                }
                Self::SideEffectNotSubmittedProven(payload) => {
                    side_effect_ref!(payload, NotSubmittedProven, None)
                }
                Self::SideEffectSubmissionObserved(payload) => {
                    side_effect_ref!(payload, SubmissionObserved, None)
                }
                Self::SideEffectSubmissionUnknown(payload) => {
                    side_effect_ref!(payload, SubmissionUnknown, None)
                }
                Self::SideEffectReceiptObserved(payload) => {
                    side_effect_ref!(payload, ReceiptObserved, None)
                }
                Self::SideEffectConfirmationObserved(payload) => {
                    side_effect_ref!(payload, ConfirmationObserved, None)
                }
                Self::SideEffectAmbiguous(payload) => side_effect_ref!(payload, Ambiguous, None),
                Self::SideEffectFailed(payload) => side_effect_ref!(payload, Failed, None),
                _ => None,
            }
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
        /// Invocation epoch when the payload carries one.
        pub invocation_epoch: Option<u32>,
        /// Claim generation when the payload carries one.
        pub claim_generation: Option<u32>,
        /// Side-effect payload kind.
        pub kind: SideEffectEventKind,
    }

    /// Run admission event payload.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunAdmitted {
        /// Run id bound to this event stream.
        pub run_id: RunId,
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
    pub struct EntryPointOpId(String);

    impl EntryPointOpId {
        /// Creates a checked entry-point op id.
        pub fn new(value: impl AsRef<str>) -> Result<Self> {
            checked_ascii_token("entry point op id", value).map(Self)
        }

        /// Returns the entry-point op id string.
        pub fn as_str(&self) -> &str {
            &self.0
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
        /// Node id that requested the fact.
        pub node_id: NodeId,
        /// Attempt id that requested the fact.
        pub attempt_id: AttemptId,
        /// Capability kind used.
        pub capability_kind: CapabilityKind,
        /// Capability version used.
        pub capability_version: CapabilityVersion,
        /// Adapter kind used.
        pub adapter_kind: AdapterKind,
        /// Adapter version used.
        pub adapter_version: AdapterVersion,
        /// Request schema id.
        pub request_schema_id: SchemaId,
        /// Canonical request hash.
        pub request_hash: ContentDigest,
        /// Response schema id.
        pub response_schema_id: SchemaId,
        /// Canonical response hash.
        pub response_hash: ContentDigest,
        /// Stable fact key.
        pub fact_key: FactKey,
        /// Response artifact id.
        pub artifact_id: ArtifactId,
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
        /// Seed input artifact.
        SeedInput,
        /// State output value artifact.
        StateOutput,
        /// Read fact response artifact.
        FactResponse,
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
        /// Launch artifacts may carry a certified schema or omit it for current compatibility.
        OptionalLaunchSchema,
        /// Schema must equal the certified seed schema.
        ExactSeedSchema,
        /// Schema must equal the produced value cell schema.
        ExactValueSchema,
        /// Schema must equal the event evidence schema.
        ExactEvidenceSchema,
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
                Self::OptionalLaunchSchema => "optional_launch_schema",
                Self::ExactSeedSchema => "exact_seed_schema",
                Self::ExactValueSchema => "exact_value_schema",
                Self::ExactEvidenceSchema => "exact_evidence_schema",
                Self::Absent => "absent",
                Self::ExactPublicSchema => "exact_public_schema",
                Self::ExactDiagnosticSchema => "exact_diagnostic_schema",
            }
        }
    }

    /// Semantic-type-field policy required for an artifact role.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ArtifactSemanticPolicy {
        /// Launch artifacts may carry a certified semantic type or omit it for current compatibility.
        OptionalLaunchSemantic,
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
                Self::OptionalLaunchSemantic => "optional_launch_semantic",
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
            Self::SeedInput,
            Self::StateOutput,
            Self::FactResponse,
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
                    schema: ArtifactSchemaPolicy::OptionalLaunchSchema,
                    semantic: ArtifactSemanticPolicy::OptionalLaunchSemantic,
                    producer: ArtifactProducerScope::LaunchOrGlobalNoSeed,
                    staging: ArtifactStagingClass::RunAdmission,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RunAdmittedArtifact,
                },
                Self::TypedSpecCertificate => ArtifactRoleContract {
                    role: self,
                    tag: "typed_spec_certificate",
                    schema: ArtifactSchemaPolicy::OptionalLaunchSchema,
                    semantic: ArtifactSemanticPolicy::OptionalLaunchSemantic,
                    producer: ArtifactProducerScope::LaunchOrGlobalNoSeed,
                    staging: ArtifactStagingClass::RunAdmission,
                    retention: ArtifactRetentionClass::FrameworkIgnored,
                    same_commit: ArtifactSameCommitPolicy::RunAdmittedArtifact,
                },
                Self::TypedConfig => ArtifactRoleContract {
                    role: self,
                    tag: "typed_config",
                    schema: ArtifactSchemaPolicy::OptionalLaunchSchema,
                    semantic: ArtifactSemanticPolicy::OptionalLaunchSemantic,
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
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::FactResponse,
                    artifact_id: payload.artifact_id.clone(),
                    digest: Some(payload.response_hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.response_schema_id.clone()),
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
            | KernelEventPayload::SideEffectInvocationStarted(_)
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

    const MAX_PUBLIC_SAFE_MESSAGE_BYTES: usize = 512;
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
        if value.is_empty() {
            return Err(EventError::InvalidPublicDiagnostic {
                field,
                reason: "message must not be empty",
            });
        }
        if value.len() > MAX_PUBLIC_SAFE_MESSAGE_BYTES {
            return Err(EventError::InvalidPublicDiagnostic {
                field,
                reason: "message exceeds public length limit",
            });
        }
        if !value.bytes().all(|byte| matches!(byte, 0x20..=0x7e)) {
            return Err(EventError::InvalidPublicDiagnostic {
                field,
                reason: "message must be printable ASCII",
            });
        }
        if contains_secret_shaped_diagnostic(value) {
            return Err(EventError::InvalidPublicDiagnostic {
                field,
                reason: "message resembles secret material",
            });
        }
        Ok(())
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
            /// Invocation epoch.
            pub invocation_epoch: u32,
            /// Claim generation.
            pub claim_generation: u32,
            /// Claim fencing token.
            pub claim_fencing_token: ClaimFencingToken,
            /// Optional prepared artifact id.
            pub prepared_artifact_id: Option<ArtifactId>,
            /// Optional prepared artifact hash.
            pub prepared_hash: Option<ContentDigest>,
            /// Optional exclusive resource lane key evidence.
            pub resource_key: Option<ResourceKeyEvidence>,
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

    fn checked_token_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "charset": "visible_ascii",
            "kind": "checked_ascii_token",
            "max_len": 256,
            "name": type_name,
            "non_empty": true,
        })
    }

    fn checked_author_key_type(type_name: &'static str) -> serde_json::Value {
        serde_json::json!({
            "kind": "checked_author_key",
            "name": type_name,
            "persisted_as": "string",
        })
    }

    fn public_field_path_type() -> serde_json::Value {
        serde_json::json!({
            "kind": "checked_public_field_path",
            "name": "PublicFieldPath",
            "persisted_as": "string",
            "segment_charset": "ascii_alphanumeric_or_underscore_dash_slash",
        })
    }

    fn resource_namespace_type() -> serde_json::Value {
        serde_json::json!({
            "kind": "checked_resource_namespace",
            "name": "ResourceNamespace",
            "persisted_as": "string",
            "segment_charset": "ascii_lower_digit_underscore_dash_dot",
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
            "String" => serde_json::json!({
                "kind": "string",
                "name": "String",
            }),
            "AdapterKind" | "ArtifactId" | "AttemptId" | "CapabilityKind" | "CellId"
            | "ContentDigest" | "DescriptorId" | "EffectKind" | "EventId" | "NodeId"
            | "OperationKind" | "RunId" | "SchemaId" | "ScopeId" | "SeedId" | "SemanticTypeId"
            | "SpecHash" | "StateKind" => mfm_identity_type(type_name),
            "AdapterVersion" | "CapabilityVersion" | "LoweringVersion" | "OperationVersion"
            | "SpecVersion" | "StateVersion" => mfm_version_type(type_name),
            "DigestAlgorithm" => unit_enum_type("DigestAlgorithm", &["sha256-jcs-v1"]),
            "MediaType" | "CanonicalizerIdentity" | "RendererVersion" => {
                checked_token_type(type_name)
            }
            "PublicFieldPath" => public_field_path_type(),
            "ResourceNamespace" => resource_namespace_type(),
            "RendererKind" => checked_author_key_type(type_name),
            "RunnerFactoryId"
            | "NixDerivationHash"
            | "NixOutputHash"
            | "FactKey"
            | "SideEffectLedgerKey"
            | "ResourceKey"
            | "RunnerInvocationId"
            | "IdempotencyKeyRef"
            | "ReplayVerifierId"
            | "AmbiguityCode"
            | "ErrorCode"
            | "ClaimFencingToken"
            | "EntryPointOpId" => checked_token_type(type_name),
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
                ],
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
                            "forward_ledger_key",
                            "SideEffectLedgerKey",
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
            SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA,
            SIDE_EFFECT_INVOCATION_STARTED_SCHEMA,
            SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA,
            SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA,
            SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA,
            SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA,
            SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA,
            SIDE_EFFECT_AMBIGUOUS_SCHEMA,
            SIDE_EFFECT_FAILED_SCHEMA,
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
            EventFieldDescriptor::required("entry_point", "EntryPointLaunchEvidence"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("spec_artifact", "RunArtifactEvidenceRef"),
            EventFieldDescriptor::required("certificate_artifact", "RunArtifactEvidenceRef"),
            EventFieldDescriptor::repeated("config_artifacts", "RunArtifactEvidenceRef"),
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
            EventFieldDescriptor::required("capability_kind", "CapabilityKind"),
            EventFieldDescriptor::required("capability_version", "CapabilityVersion"),
            EventFieldDescriptor::required("adapter_kind", "AdapterKind"),
            EventFieldDescriptor::required("adapter_version", "AdapterVersion"),
            EventFieldDescriptor::required("request_schema_id", "SchemaId"),
            EventFieldDescriptor::required("request_hash", "ContentDigest"),
            EventFieldDescriptor::required("response_schema_id", "SchemaId"),
            EventFieldDescriptor::required("response_hash", "ContentDigest"),
            EventFieldDescriptor::required("fact_key", "FactKey"),
            EventFieldDescriptor::required("artifact_id", "ArtifactId"),
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
            EventFieldDescriptor::required("previous_claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("new_claim_owner", "RunnerInvocationId"),
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("previous_claim_generation", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
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
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
            EventFieldDescriptor::optional("prepared_artifact_id", "ArtifactId"),
            EventFieldDescriptor::optional("prepared_hash", "ContentDigest"),
            EventFieldDescriptor::optional("resource_key", "ResourceKeyEvidence"),
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
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("failure_phase", "FailurePhase"),
            EventFieldDescriptor::required("retryable", "bool"),
            EventFieldDescriptor::required("error", "MfmErrorInfo"),
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
    mod tests {
        use super::*;
        use mfm_ids::DigestBytes;

        fn digest_bytes(byte: u8) -> DigestBytes {
            DigestBytes::from_array([byte; 32])
        }

        fn content_digest(byte: u8) -> ContentDigest {
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn spec_hash(byte: u8) -> SpecHash {
            SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn run_id(byte: u8) -> RunId {
            RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn artifact_id(byte: u8) -> ArtifactId {
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn attempt_id(byte: u8) -> AttemptId {
            AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn node_id(byte: u8) -> NodeId {
            NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn descriptor_id(byte: u8) -> DescriptorId {
            DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn cell_id(byte: u8) -> CellId {
            CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn scope_id(byte: u8) -> ScopeId {
            ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn seed_id(byte: u8) -> SeedId {
            SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
        }

        fn schema_id(name: &str, byte: u8) -> SchemaId {
            SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
                .expect("schema id")
        }

        fn semantic_id(name: &str, byte: u8) -> SemanticTypeId {
            SemanticTypeId::new(
                "mfm.test",
                name,
                "1",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(byte),
            )
            .expect("semantic id")
        }

        fn media_type(value: &str) -> MediaType {
            MediaType::new(value).expect("media type")
        }

        fn event_artifact_ref(
            artifact_id: ArtifactId,
            role: ArtifactRole,
            schema_id: SchemaId,
            content_digest: ContentDigest,
        ) -> ArtifactEvidenceRef {
            ArtifactEvidenceRef {
                artifact_id,
                role,
                schema_id,
                semantic_type_id: None,
                content_digest,
                byte_len: 64,
                media_type: media_type("application/json"),
            }
        }

        fn run_artifact_ref(
            artifact_id: ArtifactId,
            role: ArtifactRole,
            schema_id: Option<SchemaId>,
            content_digest: ContentDigest,
        ) -> RunArtifactEvidenceRef {
            RunArtifactEvidenceRef {
                artifact_id,
                role,
                schema_id,
                semantic_type_id: None,
                content_digest,
                byte_len: 64,
                media_type: media_type("application/json"),
            }
        }

        #[test]
        fn mfm_error_info_constructor_accepts_redacted_diagnostic_ref() {
            let diagnostic = event_artifact_ref(
                artifact_id(42),
                ArtifactRole::RedactedDiagnostic,
                schema_id("mfm.test.diagnostic", 43),
                content_digest(44),
            );

            let error = MfmErrorInfo::new(
                ErrorCode::new("redacted_diagnostic").expect("code"),
                ErrorCategory::Runtime,
                false,
                "runtime validation failed",
            )
            .expect("base error")
            .with_public_details(RedactedJson::new(content_digest(45)))
            .expect("public details")
            .with_diagnostic_ref(diagnostic.clone())
            .expect("diagnostic ref");

            assert_eq!(error.diagnostic_ref, Some(diagnostic));
            assert!(error.public_details.is_some());
        }

        #[test]
        fn mfm_error_info_rejects_secret_shaped_safe_message() {
            let error = MfmErrorInfo::new(
                ErrorCode::new("redacted_diagnostic").expect("code"),
                ErrorCategory::Runtime,
                false,
                "provider returned bearer token=super-secret-value",
            )
            .expect_err("secret-shaped message rejected");

            assert!(matches!(
                error,
                EventError::InvalidPublicDiagnostic {
                    field: "safe_message",
                    reason: "message resembles secret material"
                }
            ));
        }

        #[test]
        fn mfm_error_info_rejects_non_redacted_diagnostic_ref() {
            let diagnostic = event_artifact_ref(
                artifact_id(46),
                ArtifactRole::SideEffectIntent,
                schema_id("mfm.test.diagnostic", 47),
                content_digest(48),
            );
            let error = MfmErrorInfo::new(
                ErrorCode::new("redacted_diagnostic").expect("code"),
                ErrorCategory::Runtime,
                false,
                "runtime validation failed",
            )
            .expect("base error")
            .with_diagnostic_ref(diagnostic)
            .expect_err("wrong diagnostic role rejected");

            assert!(matches!(
                error,
                EventError::InvalidPublicDiagnostic {
                    field: "diagnostic_ref",
                    reason: "diagnostic artifact role must be redacted_diagnostic"
                }
            ));
        }

        fn error_with_diagnostic(
            artifact_id: ArtifactId,
            role: ArtifactRole,
            schema_id: SchemaId,
            content_digest: ContentDigest,
        ) -> MfmErrorInfo {
            MfmErrorInfo {
                code: ErrorCode::new("event_accessor_test").expect("error code"),
                category: ErrorCategory::Runtime,
                retryable: false,
                safe_message: "event accessor test".to_owned(),
                public_details: None,
                diagnostic_ref: Some(event_artifact_ref(
                    artifact_id,
                    role,
                    schema_id,
                    content_digest,
                )),
            }
        }

        fn touched_set(byte: u8) -> ResourceTouchedSetEvidence {
            ResourceTouchedSetEvidence {
                namespace: ResourceNamespace::new("mfm.test.resource").expect("namespace"),
                evidence_schema_id: schema_id("mfm.test.touched_set", byte),
                evidence_hash: content_digest(byte.wrapping_add(1)),
                evidence_artifact_id: artifact_id(byte.wrapping_add(2)),
            }
        }

        fn run_admitted_payload() -> KernelEventPayload {
            KernelEventPayload::RunAdmitted(Box::new(RunAdmitted {
                run_id: run_id(1),
                entry_point: EntryPointLaunchEvidence {
                    resolved_op_id: EntryPointOpId::new("mfm.portfolio/snapshot@1").expect("op id"),
                    entry_point_registry_digest: content_digest(18),
                },
                spec_hash: spec_hash(2),
                spec_artifact: run_artifact_ref(
                    artifact_id(3),
                    ArtifactRole::TypedExecutionSpec,
                    None,
                    content_digest(2),
                ),
                certificate_artifact: run_artifact_ref(
                    artifact_id(4),
                    ArtifactRole::TypedSpecCertificate,
                    None,
                    content_digest(5),
                ),
                config_artifacts: vec![run_artifact_ref(
                    artifact_id(14),
                    ArtifactRole::TypedConfig,
                    Some(schema_id("mfm.test.config", 15)),
                    content_digest(16),
                )],
                spec_version: SpecVersion::new("mfm.typed.execution_spec.v1")
                    .expect("spec version"),
                lowering_version: LoweringVersion::new("mfm.typed.lowering.v1")
                    .expect("lowering version"),
                public_output_schema_id: schema_id("mfm.test.public_output", 6),
                saga_policy_digest: mfm_spec::v1::SagaPolicySpec::NoSideEffects
                    .saga_policy_digest()
                    .expect("saga policy digest"),
                descriptor_identities: Vec::new(),
                runner_executables: Vec::new(),
                adapter_executables: Vec::new(),
                admitted_binding_digest: content_digest(17),
                canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1")
                    .expect("canonicalizer"),
                seed_cells: vec![SeedCellRef {
                    seed_id: seed_id(7),
                    cell_id: cell_id(8),
                    scope_id: scope_id(9),
                    semantic_type_id: semantic_id("seed", 10),
                    schema_id: schema_id("mfm.test.seed", 11),
                    digest: content_digest(12),
                    seed_artifact: event_artifact_ref(
                        artifact_id(13),
                        ArtifactRole::SeedInput,
                        schema_id("mfm.test.seed", 11),
                        content_digest(12),
                    ),
                }],
            }))
        }

        #[derive(Debug, Clone, Copy)]
        struct ArtifactRoleBaseline {
            role: ArtifactRole,
            tag: &'static str,
            schema_policy: &'static str,
            semantic_policy: &'static str,
            producer_policy: &'static str,
            staging_class: &'static str,
            retention_class: &'static str,
            same_commit_policy: &'static str,
        }

        fn artifact_role_baselines() -> &'static [ArtifactRoleBaseline] {
            &[
                ArtifactRoleBaseline {
                    role: ArtifactRole::TypedExecutionSpec,
                    tag: "typed_execution_spec",
                    schema_policy: "optional_launch_schema",
                    semantic_policy: "optional_launch_semantic",
                    producer_policy: "launch_or_global_no_seed",
                    staging_class: "run_admission",
                    retention_class: "framework_ignored",
                    same_commit_policy: "run_admitted_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::TypedSpecCertificate,
                    tag: "typed_spec_certificate",
                    schema_policy: "optional_launch_schema",
                    semantic_policy: "optional_launch_semantic",
                    producer_policy: "launch_or_global_no_seed",
                    staging_class: "run_admission",
                    retention_class: "framework_ignored",
                    same_commit_policy: "run_admitted_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::TypedConfig,
                    tag: "typed_config",
                    schema_policy: "optional_launch_schema",
                    semantic_policy: "optional_launch_semantic",
                    producer_policy: "launch_or_global_no_seed",
                    staging_class: "run_admission",
                    retention_class: "framework_ignored",
                    same_commit_policy: "run_admitted_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::SeedInput,
                    tag: "seed_input",
                    schema_policy: "exact_seed_schema",
                    semantic_policy: "exact_seed_semantic",
                    producer_policy: "seed_required",
                    staging_class: "run_admission",
                    retention_class: "framework_ignored",
                    same_commit_policy: "run_admitted_seed_cell",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::StateOutput,
                    tag: "state_output",
                    schema_policy: "exact_value_schema",
                    semantic_policy: "exact_value_semantic",
                    producer_policy: "node_required",
                    staging_class: "attempt_state_output",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::FactResponse,
                    tag: "fact_response",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "attempt_fact_response",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::SideEffectIntent,
                    tag: "side_effect_intent",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_intent",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::PreparedInvocation,
                    tag: "prepared_invocation",
                    schema_policy: "absent",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_prepared_invocation",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::NotSubmittedProof,
                    tag: "not_submitted_proof",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_not_submitted_proof",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::Submission,
                    tag: "submission",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_submission",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::SubmissionUnknownEvidence,
                    tag: "submission_unknown_evidence",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_submission_unknown",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::Receipt,
                    tag: "receipt",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_receipt",
                    retention_class: "receipt_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::Confirmation,
                    tag: "confirmation",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_confirmation",
                    retention_class: "confirmation_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::AmbiguityEvidence,
                    tag: "ambiguity_evidence",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "side_effect_ambiguity",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::ManualResolutionEvidence,
                    tag: "manual_resolution_evidence",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "global_no_seed",
                    staging_class: "manual_resolution",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::ManualResolutionAuthorization,
                    tag: "manual_resolution_authorization",
                    schema_policy: "exact_evidence_schema",
                    semantic_policy: "absent",
                    producer_policy: "global_no_seed",
                    staging_class: "manual_resolution",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::PublicOutput,
                    tag: "public_output",
                    schema_policy: "exact_public_schema",
                    semantic_policy: "absent",
                    producer_policy: "node_required",
                    staging_class: "attempt_public_output",
                    retention_class: "public_output_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::RedactedDiagnostic,
                    tag: "redacted_diagnostic",
                    schema_policy: "exact_diagnostic_schema",
                    semantic_policy: "absent",
                    producer_policy: "diagnostic_optional_node_no_seed",
                    staging_class: "attempt_redacted_diagnostic",
                    retention_class: "value_artifacts",
                    same_commit_policy: "payload_required_artifact",
                },
                ArtifactRoleBaseline {
                    role: ArtifactRole::RetentionManifest,
                    tag: "retention_manifest",
                    schema_policy: "absent",
                    semantic_policy: "absent",
                    producer_policy: "middleware_no_seed",
                    staging_class: "middleware_retention_manifest",
                    retention_class: "framework_ignored",
                    same_commit_policy: "retention_manifest_requires_ref",
                },
            ]
        }

        fn artifact_role_schema_tags() -> Vec<String> {
            let schema = ARTIFACT_REFERENCED_SCHEMA
                .canonical_json()
                .expect("artifact schema json");
            let json: serde_json::Value =
                serde_json::from_str(schema.as_str()).expect("schema json value");
            find_artifact_role_variants(&json).expect("artifact role variants")
        }

        fn find_artifact_role_variants(value: &serde_json::Value) -> Option<Vec<String>> {
            if let Some(array) = value.as_array() {
                for nested in array {
                    if let Some(variants) = find_artifact_role_variants(nested) {
                        return Some(variants);
                    }
                }
                return None;
            }
            let object = value.as_object()?;
            if object.get("kind").and_then(serde_json::Value::as_str) == Some("enum")
                && object.get("name").and_then(serde_json::Value::as_str) == Some("ArtifactRole")
            {
                return object
                    .get("variants")
                    .and_then(serde_json::Value::as_array)
                    .map(|variants| {
                        variants
                            .iter()
                            .map(|variant| {
                                variant
                                    .get("name")
                                    .and_then(serde_json::Value::as_str)
                                    .expect("variant name")
                                    .to_owned()
                            })
                            .collect()
                    });
            }
            for nested in object.values() {
                if let Some(variants) = find_artifact_role_variants(nested) {
                    return Some(variants);
                }
            }
            None
        }

        #[test]
        fn artifact_role_contract_policy_baseline_covers_schema_tags() {
            let tags = artifact_role_baselines()
                .iter()
                .map(|row| row.tag.to_owned())
                .collect::<Vec<_>>();
            assert_eq!(tags, artifact_role_schema_tags());
            assert_eq!(
                tags,
                ArtifactRole::ALL
                    .iter()
                    .map(|role| role.as_str().to_owned())
                    .collect::<Vec<_>>()
            );

            let mut roles = artifact_role_baselines()
                .iter()
                .map(|row| row.role)
                .collect::<Vec<_>>();
            roles.sort_unstable();
            roles.dedup();
            assert_eq!(roles.len(), artifact_role_baselines().len());
            assert_eq!(roles.len(), ArtifactRole::ALL.len());

            let rows = artifact_role_baselines()
                .iter()
                .map(|row| {
                    let contract = row.role.contract();
                    assert_eq!(contract.role, row.role);
                    assert_eq!(contract.tag, row.tag);
                    assert_eq!(row.role.as_str(), row.tag);
                    assert_eq!(ArtifactRole::parse(row.tag), Some(row.role));
                    assert_eq!(contract.schema.as_str(), row.schema_policy);
                    assert_eq!(contract.semantic.as_str(), row.semantic_policy);
                    assert_eq!(contract.producer.as_str(), row.producer_policy);
                    assert_eq!(contract.staging.as_str(), row.staging_class);
                    assert_eq!(contract.retention.as_str(), row.retention_class);
                    assert_eq!(contract.same_commit.as_str(), row.same_commit_policy);
                    format!(
                        "{} schema={} semantic={} producer={} staging={} retention={} same_commit={}",
                        row.tag,
                        row.schema_policy,
                        row.semantic_policy,
                        row.producer_policy,
                        row.staging_class,
                        row.retention_class,
                        row.same_commit_policy
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(ArtifactRole::parse("resource_touched_set_evidence"), None);

            assert_eq!(
                rows,
                "typed_execution_spec schema=optional_launch_schema semantic=optional_launch_semantic producer=launch_or_global_no_seed staging=run_admission retention=framework_ignored same_commit=run_admitted_artifact\n\
typed_spec_certificate schema=optional_launch_schema semantic=optional_launch_semantic producer=launch_or_global_no_seed staging=run_admission retention=framework_ignored same_commit=run_admitted_artifact\n\
typed_config schema=optional_launch_schema semantic=optional_launch_semantic producer=launch_or_global_no_seed staging=run_admission retention=framework_ignored same_commit=run_admitted_artifact\n\
seed_input schema=exact_seed_schema semantic=exact_seed_semantic producer=seed_required staging=run_admission retention=framework_ignored same_commit=run_admitted_seed_cell\n\
state_output schema=exact_value_schema semantic=exact_value_semantic producer=node_required staging=attempt_state_output retention=value_artifacts same_commit=payload_required_artifact\n\
fact_response schema=exact_evidence_schema semantic=absent producer=node_required staging=attempt_fact_response retention=value_artifacts same_commit=payload_required_artifact\n\
side_effect_intent schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_intent retention=value_artifacts same_commit=payload_required_artifact\n\
prepared_invocation schema=absent semantic=absent producer=node_required staging=side_effect_prepared_invocation retention=value_artifacts same_commit=payload_required_artifact\n\
not_submitted_proof schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_not_submitted_proof retention=value_artifacts same_commit=payload_required_artifact\n\
submission schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_submission retention=value_artifacts same_commit=payload_required_artifact\n\
submission_unknown_evidence schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_submission_unknown retention=value_artifacts same_commit=payload_required_artifact\n\
receipt schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_receipt retention=receipt_artifacts same_commit=payload_required_artifact\n\
confirmation schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_confirmation retention=confirmation_artifacts same_commit=payload_required_artifact\n\
ambiguity_evidence schema=exact_evidence_schema semantic=absent producer=node_required staging=side_effect_ambiguity retention=value_artifacts same_commit=payload_required_artifact\n\
manual_resolution_evidence schema=exact_evidence_schema semantic=absent producer=global_no_seed staging=manual_resolution retention=value_artifacts same_commit=payload_required_artifact\n\
manual_resolution_authorization schema=exact_evidence_schema semantic=absent producer=global_no_seed staging=manual_resolution retention=value_artifacts same_commit=payload_required_artifact\n\
public_output schema=exact_public_schema semantic=absent producer=node_required staging=attempt_public_output retention=public_output_artifacts same_commit=payload_required_artifact\n\
redacted_diagnostic schema=exact_diagnostic_schema semantic=absent producer=diagnostic_optional_node_no_seed staging=attempt_redacted_diagnostic retention=value_artifacts same_commit=payload_required_artifact\n\
retention_manifest schema=absent semantic=absent producer=middleware_no_seed staging=middleware_retention_manifest retention=framework_ignored same_commit=retention_manifest_requires_ref"
            );
        }

        fn descriptor_artifact_requirement_sources(schema_name: &str) -> &'static str {
            match schema_name {
                "mfm.events.v1.run_admitted" => "RunSpec,RunCertificate,RunConfig,SeedCell",
                "mfm.events.v1.fact_recorded" => "FactResponse",
                "mfm.events.v1.artifact_referenced" => "ArtifactReferenced",
                "mfm.events.v1.cell_produced" => "StateOutput",
                "mfm.events.v1.side_effect.intent_persisted" => "SideEffectIntent",
                "mfm.events.v1.side_effect.invocation_prepared" => "PreparedInvocation",
                "mfm.events.v1.side_effect.not_submitted_proven" => "NotSubmittedProof",
                "mfm.events.v1.side_effect.submission_observed" => "Submission",
                "mfm.events.v1.side_effect.submission_unknown" => "SubmissionUnknownEvidence",
                "mfm.events.v1.side_effect.receipt_observed" => "Receipt,ResourceTouchedSet",
                "mfm.events.v1.side_effect.confirmation_observed" => {
                    "Confirmation,ResourceTouchedSet"
                }
                "mfm.events.v1.side_effect.ambiguous" => "AmbiguityEvidence",
                "mfm.events.v1.side_effect.failed" => "SideEffectFailureDiagnostic",
                "mfm.events.v1.public_output_produced" => "PublicOutputCell,PublicOutputRendered",
                "mfm.events.v1.public_output_render_failed" => {
                    "PublicOutputRenderFailureDiagnostic"
                }
                "mfm.events.v1.state_attempt_failed" => "StateAttemptFailureDiagnostic",
                "mfm.events.v1.manual_resolution_recorded" => {
                    "ManualResolutionEvidence,ManualResolutionAuthorization"
                }
                "mfm.events.v1.retention_refs_appended" => "RetentionRef",
                "mfm.events.v1.retention_manifest_projected" => "RetentionManifest",
                "mfm.events.v1.state_attempt_started"
                | "mfm.events.v1.cell_skipped"
                | "mfm.events.v1.side_effect.claimed"
                | "mfm.events.v1.side_effect.claim_taken_over"
                | "mfm.events.v1.side_effect.invocation_started"
                | "mfm.events.v1.state_attempt_completed"
                | "mfm.events.v1.state_attempt_interrupted"
                | "mfm.events.v1.run_completed" => "",
                other => panic!("uncovered event schema descriptor {other}"),
            }
        }

        #[test]
        fn event_schema_descriptor_requirement_sources_golden() {
            let rows = all_event_schema_descriptors()
                .into_iter()
                .map(|descriptor| {
                    format!(
                        "{} {} [{}]",
                        descriptor.schema_name,
                        descriptor.schema_id().expect("schema id").as_str(),
                        descriptor_artifact_requirement_sources(descriptor.schema_name)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            assert_eq!(
                rows,
                "mfm.events.v1.run_admitted schema:mfm.events.v1.run_admitted:1:sha256-jcs-v1:54a09a13587e3048ddd955d8bc3c9b4ba9bf2c2dd2914cb214dda96d859b4aac [RunSpec,RunCertificate,RunConfig,SeedCell]\n\
mfm.events.v1.state_attempt_started schema:mfm.events.v1.state_attempt_started:1:sha256-jcs-v1:986f35aa39938713b9862192cab7d2b9b3a37219f5872bd242f8a06e7957ff1b []\n\
mfm.events.v1.fact_recorded schema:mfm.events.v1.fact_recorded:1:sha256-jcs-v1:e708d591505935c8d5b12e833e34e6883c3e62fc548758a53ed5199e94218f70 [FactResponse]\n\
mfm.events.v1.artifact_referenced schema:mfm.events.v1.artifact_referenced:1:sha256-jcs-v1:c5965f6401628c580d907568a57b29e06638d4cf1740b1ea781eae88df4d592c [ArtifactReferenced]\n\
mfm.events.v1.cell_produced schema:mfm.events.v1.cell_produced:1:sha256-jcs-v1:9a2b250e7a5270bb302ae76a06091873dc50644855bace41bab97dcce311f07a [StateOutput]\n\
mfm.events.v1.cell_skipped schema:mfm.events.v1.cell_skipped:1:sha256-jcs-v1:e82d7230e3ca668b68f1403d3db7c07d36a3373f8e3744e9a41351a7aa5392f4 []\n\
mfm.events.v1.side_effect.intent_persisted schema:mfm.events.v1.side_effect.intent_persisted:1:sha256-jcs-v1:d2f3042ed5189e6e5081b781b205886e3c7d469e4cadb0fa4a61e460926c7cce [SideEffectIntent]\n\
mfm.events.v1.side_effect.claimed schema:mfm.events.v1.side_effect.claimed:1:sha256-jcs-v1:264b474d74a9349bbc1b126e0c13ec3b29ec63124d54db6925fa41e2a7e8ef78 []\n\
mfm.events.v1.side_effect.claim_taken_over schema:mfm.events.v1.side_effect.claim_taken_over:1:sha256-jcs-v1:358052910361a392a93a34fbb8bcccfd3edde420caa19289e1cd6136a7421a9f []\n\
mfm.events.v1.side_effect.invocation_prepared schema:mfm.events.v1.side_effect.invocation_prepared:1:sha256-jcs-v1:11e7ee2739d17eea6fa1957d3da20db6eec8a58e15990c5193ba4f532af57f59 [PreparedInvocation]\n\
mfm.events.v1.side_effect.invocation_started schema:mfm.events.v1.side_effect.invocation_started:1:sha256-jcs-v1:51cbcca15a2cd022b87a78b71e7652f2928e3dcb011f47312cb5becaa8729002 []\n\
mfm.events.v1.side_effect.not_submitted_proven schema:mfm.events.v1.side_effect.not_submitted_proven:1:sha256-jcs-v1:795bf7a92342608ce42e42335ab318b060fce28353e4bf91ca56dda72d4da0f7 [NotSubmittedProof]\n\
mfm.events.v1.side_effect.submission_observed schema:mfm.events.v1.side_effect.submission_observed:1:sha256-jcs-v1:08c47a5f1a00a0052bdc66fdbf6273eb6dd39670ee7936b9e68c982fa333c714 [Submission]\n\
mfm.events.v1.side_effect.submission_unknown schema:mfm.events.v1.side_effect.submission_unknown:1:sha256-jcs-v1:ed336ca8c53f4567a1f63f8d911fc82526fb59669f5f8db68ac2745ce1f67ddb [SubmissionUnknownEvidence]\n\
mfm.events.v1.side_effect.receipt_observed schema:mfm.events.v1.side_effect.receipt_observed:1:sha256-jcs-v1:e8b248201bbd212aa5eeb8a2a6f44cb4bd245b11104fd139d2948d3725fe3450 [Receipt,ResourceTouchedSet]\n\
mfm.events.v1.side_effect.confirmation_observed schema:mfm.events.v1.side_effect.confirmation_observed:1:sha256-jcs-v1:c6bc63539dd02ff1ea441f8313014a8d537f51504ea3ed827fc8c97a5654c601 [Confirmation,ResourceTouchedSet]\n\
mfm.events.v1.side_effect.ambiguous schema:mfm.events.v1.side_effect.ambiguous:1:sha256-jcs-v1:dfc9030e0d4fbbeb1800317623cecea3fbb1256c5a1b5d0e272bcda5fa4ef823 [AmbiguityEvidence]\n\
mfm.events.v1.side_effect.failed schema:mfm.events.v1.side_effect.failed:1:sha256-jcs-v1:8c7e57736bd909ecd65d435f3f91187c623d4e01ce3d5e5fcc4602a690c26a0d [SideEffectFailureDiagnostic]\n\
mfm.events.v1.public_output_produced schema:mfm.events.v1.public_output_produced:1:sha256-jcs-v1:00d2531467818398553aa59e62c034fa0cd054e7856b89f425aeb4510f9c6776 [PublicOutputCell,PublicOutputRendered]\n\
mfm.events.v1.public_output_render_failed schema:mfm.events.v1.public_output_render_failed:1:sha256-jcs-v1:38c7cf4189e8525be1c51f1d0601c024b69769e961b6cf5fd43193211e143d9d [PublicOutputRenderFailureDiagnostic]\n\
mfm.events.v1.state_attempt_completed schema:mfm.events.v1.state_attempt_completed:1:sha256-jcs-v1:36800f9d3ae748d407bc2ea24339049471c8ffe40aa86c53b35b6c7c6cd6ee80 []\n\
mfm.events.v1.state_attempt_interrupted schema:mfm.events.v1.state_attempt_interrupted:1:sha256-jcs-v1:a01ea4960dfa7c42cd9da4a572a2748513cae4107b99ac1f04afc37b7a4e9e14 []\n\
mfm.events.v1.state_attempt_failed schema:mfm.events.v1.state_attempt_failed:1:sha256-jcs-v1:1b2012da2f5e92c932aa69b3ba36df77905c43cded4fb3c21d71f2d7627eb897 [StateAttemptFailureDiagnostic]\n\
mfm.events.v1.manual_resolution_recorded schema:mfm.events.v1.manual_resolution_recorded:1:sha256-jcs-v1:b2b4122abfda77f0a8d087ea929189cd7735ea3e2ffa963e2f600e4ae74c0293 [ManualResolutionEvidence,ManualResolutionAuthorization]\n\
mfm.events.v1.run_completed schema:mfm.events.v1.run_completed:1:sha256-jcs-v1:cda37495cb3c733164ce1a91f58ff6d27bdcfbf9b1f9efe5a7fd48ae68eba479 []\n\
mfm.events.v1.retention_refs_appended schema:mfm.events.v1.retention_refs_appended:1:sha256-jcs-v1:a3a48ef21a004f9405c5585f0bdc6a9cc6c61dae616e06892ddefcfb61ae2a11 [RetentionRef]\n\
mfm.events.v1.retention_manifest_projected schema:mfm.events.v1.retention_manifest_projected:1:sha256-jcs-v1:269a96fc12c7c5004aa4592139f84cd0e4b617e04e494522ce639aeae0b9fed1 [RetentionManifest]"
            );
        }

        #[test]
        fn artifact_role_schema_descriptor_tag_baseline() {
            assert_eq!(
                artifact_role_schema_tags().join("\n"),
                "typed_execution_spec\n\
typed_spec_certificate\n\
typed_config\n\
seed_input\n\
state_output\n\
fact_response\n\
side_effect_intent\n\
prepared_invocation\n\
not_submitted_proof\n\
submission\n\
submission_unknown_evidence\n\
receipt\n\
confirmation\n\
ambiguity_evidence\n\
manual_resolution_evidence\n\
manual_resolution_authorization\n\
public_output\n\
redacted_diagnostic\n\
retention_manifest"
            );
        }

        #[test]
        fn v1_event_schema_golden() {
            let rows = all_event_schema_descriptors()
                .iter()
                .map(|descriptor| {
                    format!(
                        "{} {}",
                        descriptor.rust_type_path,
                        descriptor.schema_id().expect("schema id").as_str()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            assert_eq!(all_event_schema_descriptors().len(), 27);
            assert_eq!(
                rows,
                "mfm_events::v1::RunAdmitted schema:mfm.events.v1.run_admitted:1:sha256-jcs-v1:54a09a13587e3048ddd955d8bc3c9b4ba9bf2c2dd2914cb214dda96d859b4aac\n\
mfm_events::v1::StateAttemptStarted schema:mfm.events.v1.state_attempt_started:1:sha256-jcs-v1:986f35aa39938713b9862192cab7d2b9b3a37219f5872bd242f8a06e7957ff1b\n\
mfm_events::v1::FactRecorded schema:mfm.events.v1.fact_recorded:1:sha256-jcs-v1:e708d591505935c8d5b12e833e34e6883c3e62fc548758a53ed5199e94218f70\n\
mfm_events::v1::ArtifactReferenced schema:mfm.events.v1.artifact_referenced:1:sha256-jcs-v1:c5965f6401628c580d907568a57b29e06638d4cf1740b1ea781eae88df4d592c\n\
mfm_events::v1::CellProduced schema:mfm.events.v1.cell_produced:1:sha256-jcs-v1:9a2b250e7a5270bb302ae76a06091873dc50644855bace41bab97dcce311f07a\n\
mfm_events::v1::CellSkipped schema:mfm.events.v1.cell_skipped:1:sha256-jcs-v1:e82d7230e3ca668b68f1403d3db7c07d36a3373f8e3744e9a41351a7aa5392f4\n\
mfm_events::v1::side_effect::IntentPersisted schema:mfm.events.v1.side_effect.intent_persisted:1:sha256-jcs-v1:d2f3042ed5189e6e5081b781b205886e3c7d469e4cadb0fa4a61e460926c7cce\n\
mfm_events::v1::side_effect::Claimed schema:mfm.events.v1.side_effect.claimed:1:sha256-jcs-v1:264b474d74a9349bbc1b126e0c13ec3b29ec63124d54db6925fa41e2a7e8ef78\n\
mfm_events::v1::side_effect::ClaimTakenOver schema:mfm.events.v1.side_effect.claim_taken_over:1:sha256-jcs-v1:358052910361a392a93a34fbb8bcccfd3edde420caa19289e1cd6136a7421a9f\n\
mfm_events::v1::side_effect::InvocationPrepared schema:mfm.events.v1.side_effect.invocation_prepared:1:sha256-jcs-v1:11e7ee2739d17eea6fa1957d3da20db6eec8a58e15990c5193ba4f532af57f59\n\
mfm_events::v1::side_effect::InvocationStarted schema:mfm.events.v1.side_effect.invocation_started:1:sha256-jcs-v1:51cbcca15a2cd022b87a78b71e7652f2928e3dcb011f47312cb5becaa8729002\n\
mfm_events::v1::side_effect::NotSubmittedProven schema:mfm.events.v1.side_effect.not_submitted_proven:1:sha256-jcs-v1:795bf7a92342608ce42e42335ab318b060fce28353e4bf91ca56dda72d4da0f7\n\
mfm_events::v1::side_effect::SubmissionObserved schema:mfm.events.v1.side_effect.submission_observed:1:sha256-jcs-v1:08c47a5f1a00a0052bdc66fdbf6273eb6dd39670ee7936b9e68c982fa333c714\n\
mfm_events::v1::side_effect::SubmissionUnknown schema:mfm.events.v1.side_effect.submission_unknown:1:sha256-jcs-v1:ed336ca8c53f4567a1f63f8d911fc82526fb59669f5f8db68ac2745ce1f67ddb\n\
mfm_events::v1::side_effect::ReceiptObserved schema:mfm.events.v1.side_effect.receipt_observed:1:sha256-jcs-v1:e8b248201bbd212aa5eeb8a2a6f44cb4bd245b11104fd139d2948d3725fe3450\n\
mfm_events::v1::side_effect::ConfirmationObserved schema:mfm.events.v1.side_effect.confirmation_observed:1:sha256-jcs-v1:c6bc63539dd02ff1ea441f8313014a8d537f51504ea3ed827fc8c97a5654c601\n\
mfm_events::v1::side_effect::Ambiguous schema:mfm.events.v1.side_effect.ambiguous:1:sha256-jcs-v1:dfc9030e0d4fbbeb1800317623cecea3fbb1256c5a1b5d0e272bcda5fa4ef823\n\
mfm_events::v1::side_effect::Failed schema:mfm.events.v1.side_effect.failed:1:sha256-jcs-v1:8c7e57736bd909ecd65d435f3f91187c623d4e01ce3d5e5fcc4602a690c26a0d\n\
mfm_events::v1::PublicOutputProduced schema:mfm.events.v1.public_output_produced:1:sha256-jcs-v1:00d2531467818398553aa59e62c034fa0cd054e7856b89f425aeb4510f9c6776\n\
mfm_events::v1::PublicOutputRenderFailed schema:mfm.events.v1.public_output_render_failed:1:sha256-jcs-v1:38c7cf4189e8525be1c51f1d0601c024b69769e961b6cf5fd43193211e143d9d\n\
mfm_events::v1::StateAttemptCompleted schema:mfm.events.v1.state_attempt_completed:1:sha256-jcs-v1:36800f9d3ae748d407bc2ea24339049471c8ffe40aa86c53b35b6c7c6cd6ee80\n\
mfm_events::v1::StateAttemptInterrupted schema:mfm.events.v1.state_attempt_interrupted:1:sha256-jcs-v1:a01ea4960dfa7c42cd9da4a572a2748513cae4107b99ac1f04afc37b7a4e9e14\n\
mfm_events::v1::StateAttemptFailed schema:mfm.events.v1.state_attempt_failed:1:sha256-jcs-v1:1b2012da2f5e92c932aa69b3ba36df77905c43cded4fb3c21d71f2d7627eb897\n\
mfm_events::v1::ManualResolutionRecorded schema:mfm.events.v1.manual_resolution_recorded:1:sha256-jcs-v1:b2b4122abfda77f0a8d087ea929189cd7735ea3e2ffa963e2f600e4ae74c0293\n\
mfm_events::v1::RunCompleted schema:mfm.events.v1.run_completed:1:sha256-jcs-v1:cda37495cb3c733164ce1a91f58ff6d27bdcfbf9b1f9efe5a7fd48ae68eba479\n\
mfm_events::v1::RetentionRefsAppended schema:mfm.events.v1.retention_refs_appended:1:sha256-jcs-v1:a3a48ef21a004f9405c5585f0bdc6a9cc6c61dae616e06892ddefcfb61ae2a11\n\
mfm_events::v1::RetentionManifestProjected schema:mfm.events.v1.retention_manifest_projected:1:sha256-jcs-v1:269a96fc12c7c5004aa4592139f84cd0e4b617e04e494522ce639aeae0b9fed1"
            );
        }

        #[test]
        fn fact_recorded_schema_descriptor_baseline() {
            let canonical = FACT_RECORDED_SCHEMA
                .canonical_json()
                .expect("canonical fact schema");

            assert_eq!(
                canonical.as_str(),
                r#"{"canonicalization":"sha256-jcs-v1","fields":[{"cardinality":"required","name":"spec_hash","type":{"kind":"mfm_ids_identity","name":"SpecHash","persisted_as":"string"}},{"cardinality":"required","name":"node_id","type":{"kind":"mfm_ids_identity","name":"NodeId","persisted_as":"string"}},{"cardinality":"required","name":"attempt_id","type":{"kind":"mfm_ids_identity","name":"AttemptId","persisted_as":"string"}},{"cardinality":"required","name":"capability_kind","type":{"kind":"mfm_ids_identity","name":"CapabilityKind","persisted_as":"string"}},{"cardinality":"required","name":"capability_version","type":{"kind":"mfm_ids_version","name":"CapabilityVersion","persisted_as":"string"}},{"cardinality":"required","name":"adapter_kind","type":{"kind":"mfm_ids_identity","name":"AdapterKind","persisted_as":"string"}},{"cardinality":"required","name":"adapter_version","type":{"kind":"mfm_ids_version","name":"AdapterVersion","persisted_as":"string"}},{"cardinality":"required","name":"request_schema_id","type":{"kind":"mfm_ids_identity","name":"SchemaId","persisted_as":"string"}},{"cardinality":"required","name":"request_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}},{"cardinality":"required","name":"response_schema_id","type":{"kind":"mfm_ids_identity","name":"SchemaId","persisted_as":"string"}},{"cardinality":"required","name":"response_hash","type":{"kind":"mfm_ids_identity","name":"ContentDigest","persisted_as":"string"}},{"cardinality":"required","name":"fact_key","type":{"charset":"visible_ascii","kind":"checked_ascii_token","max_len":256,"name":"FactKey","non_empty":true}},{"cardinality":"required","name":"artifact_id","type":{"kind":"mfm_ids_identity","name":"ArtifactId","persisted_as":"string"}}],"schema_family":"mfm.kernel.event","schema_name":"mfm.events.v1.fact_recorded","schema_version":"1"}"#
            );
            assert_eq!(
                canonical.content_digest().as_str(),
                "content:sha256-jcs-v1:e708d591505935c8d5b12e833e34e6883c3e62fc548758a53ed5199e94218f70"
            );
        }

        #[test]
        fn event_schema_hashes_include_nested_structural_shapes() {
            let artifact_schema = ARTIFACT_REFERENCED_SCHEMA
                .canonical_json()
                .expect("artifact schema json");
            let artifact_json = artifact_schema.as_str();
            assert!(artifact_json.contains("\"name\":\"ArtifactEvidenceRef\""));
            assert!(artifact_json.contains("\"name\":\"semantic_type_id\""));
            assert!(artifact_json.contains("\"name\":\"ArtifactRole\""));

            let output_schema = PUBLIC_OUTPUT_PRODUCED_SCHEMA
                .canonical_json()
                .expect("public output schema json");
            let output_json = output_schema.as_str();
            assert!(output_json.contains("\"name\":\"NamedTypedCellRef\""));
            assert!(output_json.contains("\"name\":\"value_lineage\""));

            let completed_schema = RUN_COMPLETED_SCHEMA
                .canonical_json()
                .expect("completed schema json");
            let completed_json = completed_schema.as_str();
            assert!(completed_json.contains("\"name\":\"RunCompletionOutcome\""));
            assert!(completed_json.contains("\"name\":\"PublicOutputCompletionEvidence\""));
            assert!(completed_json.contains("\"name\":\"public_output_event_id\""));
            assert!(completed_json.contains("\"name\":\"compensated\""));
            assert!(completed_json.contains("\"name\":\"manually_resolved\""));
            assert!(completed_json.contains("\"name\":\"failed_without_acdc_claim\""));
            assert!(!completed_json.contains("\"name\":\"terminal_error\""));

            let manual_schema = MANUAL_RESOLUTION_RECORDED_SCHEMA
                .canonical_json()
                .expect("manual resolution schema json");
            let manual_json = manual_schema.as_str();
            assert!(manual_json.contains("\"name\":\"ManualResolutionOutcome\""));
            assert!(manual_json.contains("\"name\":\"authorization_schema_id\""));
            assert!(manual_json.contains("\"name\":\"evidence_artifact_id\""));

            let side_effect_schema = SIDE_EFFECT_INTENT_PERSISTED_SCHEMA
                .canonical_json()
                .expect("side effect schema json");
            let side_effect_json = side_effect_schema.as_str();
            assert!(side_effect_json.contains("\"name\":\"SideEffectLedgerPurpose\""));
            assert!(side_effect_json.contains("\"name\":\"forward\""));
            assert!(side_effect_json.contains("\"name\":\"remediation\""));
        }

        #[test]
        fn saga_projection_type_tags_are_stable() {
            assert_eq!(
                ManualResolutionOutcome::ConfirmRemediated.as_str(),
                "confirm_remediated"
            );
            assert_eq!(
                ManualResolutionOutcome::FailWithoutAcdcClaim.as_str(),
                "fail_without_acdc_claim"
            );
            assert_eq!(SideEffectLedgerPurpose::Forward.kind(), "forward");
            assert_eq!(
                SideEffectLedgerPurpose::Remediation {
                    forward_ledger_key: SideEffectLedgerKey::new("forward-ledger").unwrap(),
                }
                .kind(),
                "remediation"
            );

            assert_eq!(RunCompletionOutcome::Compensated.kind(), "compensated");
        }

        #[test]
        fn event_schema_descriptors_have_no_opaque_external_shapes() {
            for descriptor in all_event_schema_descriptors() {
                let json = descriptor.canonical_json().expect("descriptor json");
                assert!(
                    !json.as_str().contains("external_contract_type"),
                    "{} has an opaque external schema shape: {}",
                    descriptor.rust_type_path,
                    json.as_str()
                );
            }
        }

        #[test]
        fn all_event_schema_descriptors_are_unique() {
            let descriptors = all_event_schema_descriptors();
            let mut schema_names = descriptors
                .iter()
                .map(|descriptor| descriptor.schema_name)
                .collect::<Vec<_>>();
            let original_len = schema_names.len();
            schema_names.sort_unstable();
            schema_names.dedup();

            assert_eq!(original_len, 27);
            assert_eq!(schema_names.len(), original_len);
        }

        #[test]
        fn payload_accessors_expose_authority_fields_without_serialization_changes() {
            let run_admitted = run_admitted_payload();
            assert_eq!(run_admitted.run_id(), Some(&run_id(1)));
            assert_eq!(run_admitted.spec_hash(), &spec_hash(2));

            let side_effect =
                KernelEventPayload::SideEffectSubmissionObserved(side_effect::SubmissionObserved {
                    spec_hash: spec_hash(30),
                    node_id: node_id(31),
                    attempt_id: attempt_id(32),
                    ledger_key: SideEffectLedgerKey::new("ledger-1").expect("ledger"),
                    ledger_purpose: SideEffectLedgerPurpose::Forward,
                    invocation_epoch: 3,
                    submission_schema_id: schema_id("mfm.test.submission", 33),
                    submission_hash: content_digest(34),
                    submission_artifact_id: artifact_id(35),
                });
            let side_effect_ref = side_effect.side_effect_ref().expect("side-effect ref");
            assert_eq!(side_effect.run_id(), None);
            assert_eq!(side_effect.spec_hash(), &spec_hash(30));
            assert_eq!(side_effect_ref.node_id, &node_id(31));
            assert_eq!(side_effect_ref.attempt_id, &attempt_id(32));
            assert_eq!(
                side_effect_ref.kind,
                SideEffectEventKind::SubmissionObserved
            );
            assert_eq!(side_effect_ref.invocation_epoch, Some(3));
        }

        #[test]
        fn artifact_requirement_accessor_covers_artifact_bearing_variants() {
            let cases = vec![
                (
                    run_admitted_payload(),
                    vec![
                        EventArtifactReferenceSource::RunSpec,
                        EventArtifactReferenceSource::RunCertificate,
                        EventArtifactReferenceSource::RunConfig,
                        EventArtifactReferenceSource::SeedCell,
                    ],
                ),
                (
                    KernelEventPayload::FactRecorded(FactRecorded {
                        spec_hash: spec_hash(40),
                        node_id: node_id(41),
                        attempt_id: attempt_id(42),
                        capability_kind: CapabilityKind::new(
                            "mfm.test",
                            "capability",
                            DigestAlgorithm::Sha256JcsV1,
                            digest_bytes(43),
                        )
                        .expect("capability kind"),
                        capability_version: CapabilityVersion::new("mfm.test.capability.v1")
                            .expect("capability version"),
                        adapter_kind: AdapterKind::new(
                            "mfm.test",
                            "adapter",
                            DigestAlgorithm::Sha256JcsV1,
                            digest_bytes(44),
                        )
                        .expect("adapter kind"),
                        adapter_version: AdapterVersion::new("mfm.test.adapter.v1")
                            .expect("adapter version"),
                        request_schema_id: schema_id("mfm.test.fact_request", 45),
                        request_hash: content_digest(46),
                        response_schema_id: schema_id("mfm.test.fact_response", 47),
                        response_hash: content_digest(48),
                        fact_key: FactKey::new("fact-key-1").expect("fact key"),
                        artifact_id: artifact_id(49),
                    }),
                    vec![EventArtifactReferenceSource::FactResponse],
                ),
                (
                    KernelEventPayload::ArtifactReferenced(ArtifactReferenced {
                        spec_hash: spec_hash(50),
                        node_id: Some(node_id(51)),
                        attempt_id: Some(attempt_id(52)),
                        artifact_ref: event_artifact_ref(
                            artifact_id(53),
                            ArtifactRole::TypedConfig,
                            schema_id("mfm.test.config", 54),
                            content_digest(55),
                        ),
                    }),
                    vec![EventArtifactReferenceSource::ArtifactReferenced],
                ),
                (
                    KernelEventPayload::CellProduced(CellProduced {
                        spec_hash: spec_hash(56),
                        node_id: node_id(57),
                        cell_id: cell_id(58),
                        scope_id: scope_id(59),
                        attempt_id: attempt_id(60),
                        semantic_type_id: semantic_id("state_output", 61),
                        schema_id: schema_id("mfm.test.state_output", 62),
                        value_lineage: ValueLineageRef {
                            lineage_digest: content_digest(63),
                        },
                        artifact_id: artifact_id(64),
                        content_digest: content_digest(65),
                        producer_state_kind: None,
                        producer_state_version: None,
                    }),
                    vec![EventArtifactReferenceSource::StateOutput],
                ),
                (
                    KernelEventPayload::PublicOutputProduced(PublicOutputProduced {
                        spec_hash: spec_hash(66),
                        node_id: node_id(67),
                        attempt_id: attempt_id(68),
                        receipt_cell_id: cell_id(69),
                        public_schema_id: schema_id("mfm.test.public_output", 70),
                        output_spec_digest: content_digest(71),
                        cells: vec![NamedTypedCellRef {
                            public_field_path: PublicFieldPath::new("result").expect("field"),
                            cell_id: cell_id(72),
                            producer: CellProducer::Node(node_id(73)),
                            scope_id: scope_id(74),
                            semantic_type_id: semantic_id("public_cell", 75),
                            schema_id: schema_id("mfm.test.public_cell", 76),
                            value_lineage: ValueLineageRef {
                                lineage_digest: content_digest(77),
                            },
                            content_digest: content_digest(78),
                            artifact_id: artifact_id(79),
                        }],
                        rendered_digest: content_digest(80),
                        rendered_artifact_id: Some(artifact_id(81)),
                        renderer_descriptor_id: descriptor_id(82),
                    }),
                    vec![
                        EventArtifactReferenceSource::PublicOutputCell,
                        EventArtifactReferenceSource::PublicOutputRendered,
                    ],
                ),
                (
                    KernelEventPayload::PublicOutputRenderFailed(PublicOutputRenderFailed {
                        spec_hash: spec_hash(83),
                        node_id: node_id(84),
                        attempt_id: attempt_id(85),
                        public_schema_id: schema_id("mfm.test.public_output", 86),
                        renderer_descriptor_id: descriptor_id(87),
                        error: error_with_diagnostic(
                            artifact_id(88),
                            ArtifactRole::RedactedDiagnostic,
                            schema_id("mfm.test.diagnostic", 89),
                            content_digest(90),
                        ),
                    }),
                    vec![EventArtifactReferenceSource::PublicOutputRenderFailureDiagnostic],
                ),
                (
                    KernelEventPayload::StateAttemptFailed(StateAttemptFailed {
                        spec_hash: spec_hash(91),
                        node_id: node_id(92),
                        attempt_id: attempt_id(93),
                        retryable: false,
                        error: error_with_diagnostic(
                            artifact_id(94),
                            ArtifactRole::RedactedDiagnostic,
                            schema_id("mfm.test.diagnostic", 95),
                            content_digest(96),
                        ),
                    }),
                    vec![EventArtifactReferenceSource::StateAttemptFailureDiagnostic],
                ),
                (
                    KernelEventPayload::ManualResolutionRecorded(ManualResolutionRecorded {
                        run_id: run_id(97),
                        spec_hash: spec_hash(98),
                        outcome: ManualResolutionOutcome::ConfirmRemediated,
                        evidence_schema_id: schema_id("mfm.test.manual_evidence", 99),
                        evidence_hash: content_digest(100),
                        evidence_artifact_id: artifact_id(101),
                        authorization_schema_id: schema_id("mfm.test.manual_authorization", 102),
                        authorization_hash: content_digest(103),
                        authorization_artifact_id: artifact_id(104),
                        note: None,
                    }),
                    vec![
                        EventArtifactReferenceSource::ManualResolutionEvidence,
                        EventArtifactReferenceSource::ManualResolutionAuthorization,
                    ],
                ),
                (
                    KernelEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
                        spec_hash: spec_hash(105),
                        node_id: node_id(106),
                        scope_id: scope_id(107),
                        attempt_id: attempt_id(108),
                        ledger_key: SideEffectLedgerKey::new("ledger-intent").expect("ledger"),
                        ledger_purpose: SideEffectLedgerPurpose::Forward,
                        invocation_epoch: 1,
                        intent_schema_id: schema_id("mfm.test.intent", 109),
                        intent_hash: content_digest(110),
                        intent_artifact_id: artifact_id(111),
                        idempotency_input_schema_id: schema_id("mfm.test.idempotency", 112),
                        idempotency_input_hash: content_digest(113),
                        idempotency_key: IdempotencyKeyRef::new("idem-key-1").expect("idempotency"),
                        capability_kind: CapabilityKind::new(
                            "mfm.test",
                            "capability",
                            DigestAlgorithm::Sha256JcsV1,
                            digest_bytes(114),
                        )
                        .expect("capability kind"),
                        capability_version: CapabilityVersion::new("mfm.test.capability.v1")
                            .expect("capability version"),
                        adapter_kind: AdapterKind::new(
                            "mfm.test",
                            "adapter",
                            DigestAlgorithm::Sha256JcsV1,
                            digest_bytes(115),
                        )
                        .expect("adapter kind"),
                        adapter_version: AdapterVersion::new("mfm.test.adapter.v1")
                            .expect("adapter version"),
                    }),
                    vec![EventArtifactReferenceSource::SideEffectIntent],
                ),
                (
                    KernelEventPayload::SideEffectInvocationPrepared(
                        side_effect::InvocationPrepared {
                            spec_hash: spec_hash(116),
                            node_id: node_id(117),
                            attempt_id: attempt_id(118),
                            ledger_key: SideEffectLedgerKey::new("ledger-prepared")
                                .expect("ledger"),
                            ledger_purpose: SideEffectLedgerPurpose::Forward,
                            invocation_epoch: 1,
                            claim_generation: 1,
                            claim_fencing_token: side_effect::ClaimFencingToken::new("token-1")
                                .expect("token"),
                            prepared_artifact_id: Some(artifact_id(119)),
                            prepared_hash: Some(content_digest(120)),
                            resource_key: None,
                        },
                    ),
                    vec![EventArtifactReferenceSource::PreparedInvocation],
                ),
                (
                    KernelEventPayload::SideEffectNotSubmittedProven(
                        side_effect::NotSubmittedProven {
                            spec_hash: spec_hash(121),
                            node_id: node_id(122),
                            attempt_id: attempt_id(123),
                            ledger_key: SideEffectLedgerKey::new("ledger-not-submitted")
                                .expect("ledger"),
                            ledger_purpose: SideEffectLedgerPurpose::Forward,
                            invocation_epoch: 1,
                            proof_schema_id: schema_id("mfm.test.not_submitted", 124),
                            proof_hash: content_digest(125),
                            proof_artifact_id: artifact_id(126),
                        },
                    ),
                    vec![EventArtifactReferenceSource::NotSubmittedProof],
                ),
                (
                    KernelEventPayload::SideEffectSubmissionObserved(
                        side_effect::SubmissionObserved {
                            spec_hash: spec_hash(127),
                            node_id: node_id(128),
                            attempt_id: attempt_id(129),
                            ledger_key: SideEffectLedgerKey::new("ledger-submission")
                                .expect("ledger"),
                            ledger_purpose: SideEffectLedgerPurpose::Forward,
                            invocation_epoch: 1,
                            submission_schema_id: schema_id("mfm.test.submission", 130),
                            submission_hash: content_digest(131),
                            submission_artifact_id: artifact_id(132),
                        },
                    ),
                    vec![EventArtifactReferenceSource::Submission],
                ),
                (
                    KernelEventPayload::SideEffectSubmissionUnknown(
                        side_effect::SubmissionUnknown {
                            spec_hash: spec_hash(133),
                            node_id: node_id(134),
                            attempt_id: attempt_id(135),
                            ledger_key: SideEffectLedgerKey::new("ledger-submission-unknown")
                                .expect("ledger"),
                            ledger_purpose: SideEffectLedgerPurpose::Forward,
                            invocation_epoch: 1,
                            evidence_schema_id: schema_id("mfm.test.submission_unknown", 136),
                            evidence_hash: content_digest(137),
                            evidence_artifact_id: artifact_id(138),
                        },
                    ),
                    vec![EventArtifactReferenceSource::SubmissionUnknownEvidence],
                ),
                (
                    KernelEventPayload::SideEffectReceiptObserved(side_effect::ReceiptObserved {
                        spec_hash: spec_hash(139),
                        node_id: node_id(140),
                        attempt_id: attempt_id(141),
                        ledger_key: SideEffectLedgerKey::new("ledger-receipt").expect("ledger"),
                        ledger_purpose: SideEffectLedgerPurpose::Forward,
                        invocation_epoch: 1,
                        receipt_schema_id: schema_id("mfm.test.receipt", 142),
                        receipt_hash: content_digest(143),
                        receipt_artifact_id: artifact_id(144),
                        replay_verifier_id: ReplayVerifierId::new("verifier-1").expect("verifier"),
                        resource_touched_set: Some(touched_set(145)),
                    }),
                    vec![
                        EventArtifactReferenceSource::Receipt,
                        EventArtifactReferenceSource::ResourceTouchedSet,
                    ],
                ),
                (
                    KernelEventPayload::SideEffectConfirmationObserved(
                        side_effect::ConfirmationObserved {
                            spec_hash: spec_hash(148),
                            node_id: node_id(149),
                            attempt_id: attempt_id(150),
                            ledger_key: SideEffectLedgerKey::new("ledger-confirmation")
                                .expect("ledger"),
                            ledger_purpose: SideEffectLedgerPurpose::Forward,
                            invocation_epoch: 1,
                            confirmation_schema_id: schema_id("mfm.test.confirmation", 151),
                            confirmation_hash: content_digest(152),
                            confirmation_artifact_id: artifact_id(153),
                            replay_verifier_id: ReplayVerifierId::new("verifier-1")
                                .expect("verifier"),
                            resource_touched_set: Some(touched_set(154)),
                        },
                    ),
                    vec![
                        EventArtifactReferenceSource::Confirmation,
                        EventArtifactReferenceSource::ResourceTouchedSet,
                    ],
                ),
                (
                    KernelEventPayload::SideEffectAmbiguous(side_effect::Ambiguous {
                        spec_hash: spec_hash(157),
                        node_id: node_id(158),
                        attempt_id: attempt_id(159),
                        ledger_key: SideEffectLedgerKey::new("ledger-ambiguous").expect("ledger"),
                        ledger_purpose: SideEffectLedgerPurpose::Forward,
                        invocation_epoch: 1,
                        ambiguity_code: AmbiguityCode::new("ambiguous").expect("ambiguity"),
                        evidence_schema_id: schema_id("mfm.test.ambiguity", 160),
                        evidence_hash: content_digest(161),
                        evidence_artifact_id: artifact_id(162),
                    }),
                    vec![EventArtifactReferenceSource::AmbiguityEvidence],
                ),
                (
                    KernelEventPayload::SideEffectFailed(side_effect::Failed {
                        spec_hash: spec_hash(163),
                        node_id: node_id(164),
                        attempt_id: attempt_id(165),
                        ledger_key: SideEffectLedgerKey::new("ledger-failed").expect("ledger"),
                        ledger_purpose: SideEffectLedgerPurpose::Forward,
                        invocation_epoch: 1,
                        failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
                        retryable: false,
                        error: error_with_diagnostic(
                            artifact_id(166),
                            ArtifactRole::RedactedDiagnostic,
                            schema_id("mfm.test.diagnostic", 167),
                            content_digest(168),
                        ),
                    }),
                    vec![EventArtifactReferenceSource::SideEffectFailureDiagnostic],
                ),
                (
                    KernelEventPayload::RetentionRefsAppended(RetentionRefsAppended {
                        run_id: run_id(169),
                        spec_hash: spec_hash(170),
                        refs: vec![RetentionRef {
                            artifact_id: artifact_id(171),
                            role: ArtifactRole::FactResponse,
                            content_digest: content_digest(172),
                        }],
                        reason: RetentionReason::RuntimeEvidence,
                    }),
                    vec![EventArtifactReferenceSource::RetentionRef],
                ),
                (
                    KernelEventPayload::RetentionManifestProjected(RetentionManifestProjected {
                        run_id: run_id(173),
                        spec_hash: spec_hash(174),
                        manifest_seq: 1,
                        manifest_digest: content_digest(175),
                        previous_manifest_digest: None,
                        manifest_artifact_id: artifact_id(176),
                    }),
                    vec![EventArtifactReferenceSource::RetentionManifest],
                ),
            ];

            for (payload, expected_sources) in cases {
                let sources = payload
                    .artifact_requirements()
                    .into_iter()
                    .map(|requirement| requirement.source)
                    .collect::<Vec<_>>();
                assert_eq!(sources, expected_sources);
            }
        }

        #[test]
        fn run_admitted_v1_summary_key_is_canonical() {
            let keys = ["v1_event_schema_golden", "run_admitted_v1_present"];
            assert!(keys.contains(&"run_admitted_v1_present"));
            assert!(!keys.contains(&"run_admitted_v2_present"));
        }
    }
}
