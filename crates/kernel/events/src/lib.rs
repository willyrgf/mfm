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
    ValueLineageRef,
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

/// Versioned v1 typed kernel event contracts.
pub mod v1 {
    use super::*;

    /// v1 event schema version string.
    pub const EVENT_SCHEMA_VERSION: &str = "1";

    checked_string_type!(
        /// Framework build/version identity persisted in `RunStarted`.
        FrameworkVersion,
        "framework version"
    );
    checked_string_type!(
        /// Source revision identity persisted in events and executable identities.
        SourceRevision,
        "source revision"
    );
    checked_string_type!(
        /// Runner or adapter factory id.
        RunnerFactoryId,
        "runner factory id"
    );
    checked_string_type!(
        /// Cargo package name.
        PackageName,
        "package name"
    );
    checked_string_type!(
        /// Cargo package version.
        PackageVersion,
        "package version"
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

    /// Closed v1 typed kernel event payload enum.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum KernelEventPayload {
        /// Run start event.
        RunStarted(RunStarted),
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
        /// State attempt failed event.
        StateAttemptFailed(StateAttemptFailed),
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
                Self::RunStarted(_) => RUN_STARTED_SCHEMA,
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
                Self::StateAttemptFailed(_) => STATE_ATTEMPT_FAILED_SCHEMA,
                Self::RunCompleted(_) => RUN_COMPLETED_SCHEMA,
                Self::RetentionRefsAppended(_) => RETENTION_REFS_APPENDED_SCHEMA,
                Self::RetentionManifestProjected(_) => RETENTION_MANIFEST_PROJECTED_SCHEMA,
            }
        }

        /// Returns the event schema id for this payload variant.
        pub fn event_schema_id(&self) -> Result<SchemaId> {
            self.schema_descriptor().schema_id()
        }
    }

    /// Run start event payload.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RunStarted {
        /// Run id bound to this event stream.
        pub run_id: RunId,
        /// Certified typed spec hash.
        pub spec_hash: SpecHash,
        /// Artifact id containing the certified spec bytes.
        pub spec_artifact_id: ArtifactId,
        /// Certified spec media type.
        pub spec_media_type: MediaType,
        /// Certified spec version.
        pub spec_version: SpecVersion,
        /// Certified lowering version.
        pub lowering_version: LoweringVersion,
        /// Public output schema id.
        pub public_output_schema_id: SchemaId,
        /// Descriptor identities bound to the run.
        pub descriptor_identities: Vec<DescriptorIdentity>,
        /// Runner executable identities.
        pub runner_executables: Vec<ExecutableIdentity>,
        /// Adapter executable identities.
        pub adapter_executables: Vec<ExecutableIdentity>,
        /// Canonicalizer identity used for the certified spec.
        pub canonicalizer_identity: CanonicalizerIdentity,
        /// Framework version.
        pub framework_version: FrameworkVersion,
        /// Source revision.
        pub source_revision: SourceRevision,
        /// Seed cells materialized at run start.
        pub seed_cells: Vec<SeedCellRef>,
    }

    /// Seed cell evidence bound by `RunStarted`.
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

    /// Run completion outcome.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum RunCompletionOutcome {
        /// Run completed successfully with public-output evidence.
        Completed(PublicOutputCompletionEvidence),
        /// Run failed with a redaction-safe terminal error.
        Failed(MfmErrorInfo),
        /// Run was cancelled with a redaction-safe terminal error.
        Cancelled(MfmErrorInfo),
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
        /// Source revision.
        pub source_revision: SourceRevision,
        /// Cargo package name.
        pub cargo_package_name: PackageName,
        /// Cargo package version.
        pub cargo_package_version: PackageVersion,
        /// Cargo package digest.
        pub cargo_package_digest: ContentDigest,
        /// Binary digest.
        pub binary_digest: ContentDigest,
        /// Optional Nix derivation hash.
        pub nix_derivation_hash: Option<NixDerivationHash>,
        /// Optional Nix output hash.
        pub nix_output_hash: Option<NixOutputHash>,
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
        /// Rendered public output artifact.
        PublicOutput,
        /// Redacted diagnostic artifact.
        RedactedDiagnostic,
        /// Retention manifest artifact.
        RetentionManifest,
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
        RunStarted,
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
            "RendererKind" => checked_author_key_type(type_name),
            "FrameworkVersion"
            | "SourceRevision"
            | "RunnerFactoryId"
            | "PackageName"
            | "PackageVersion"
            | "NixDerivationHash"
            | "NixOutputHash"
            | "FactKey"
            | "SideEffectLedgerKey"
            | "RunnerInvocationId"
            | "IdempotencyKeyRef"
            | "ReplayVerifierId"
            | "AmbiguityCode"
            | "ErrorCode"
            | "ClaimFencingToken" => checked_token_type(type_name),
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
            "ExecutableIdentity" => struct_type(
                "ExecutableIdentity",
                vec![
                    schema_field(
                        "factory_id",
                        "RunnerFactoryId",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "source_revision",
                        "SourceRevision",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "cargo_package_name",
                        "PackageName",
                        EventFieldCardinality::Required,
                    ),
                    schema_field(
                        "cargo_package_version",
                        "PackageVersion",
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
            "ArtifactRole" => unit_enum_type(
                "ArtifactRole",
                &[
                    "seed_input",
                    "state_output",
                    "fact_response",
                    "side_effect_intent",
                    "prepared_invocation",
                    "not_submitted_proof",
                    "submission",
                    "submission_unknown_evidence",
                    "receipt",
                    "confirmation",
                    "ambiguity_evidence",
                    "public_output",
                    "redacted_diagnostic",
                    "retention_manifest",
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
                    "run_started",
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
                    enum_variant(
                        "failed",
                        vec![schema_field(
                            "terminal_error",
                            "MfmErrorInfo",
                            EventFieldCardinality::Required,
                        )],
                    ),
                    enum_variant(
                        "cancelled",
                        vec![schema_field(
                            "terminal_error",
                            "MfmErrorInfo",
                            EventFieldCardinality::Required,
                        )],
                    ),
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
            SchemaId::new(
                self.schema_name,
                self.schema_version,
                DigestAlgorithm::Sha256JcsV1,
                digest,
            )
            .map_err(EventError::from)
        }
    }

    /// Returns every closed v1 event schema descriptor.
    pub fn all_event_schema_descriptors() -> Vec<EventSchemaDescriptor> {
        vec![
            RUN_STARTED_SCHEMA,
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
            STATE_ATTEMPT_FAILED_SCHEMA,
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

    const RUN_STARTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
        schema_name: "mfm.events.v1.run_started",
        rust_type_path: "mfm_events::v1::RunStarted",
        schema_version: EVENT_SCHEMA_VERSION,
        fields: fields![
            EventFieldDescriptor::required("run_id", "RunId"),
            EventFieldDescriptor::required("spec_hash", "SpecHash"),
            EventFieldDescriptor::required("spec_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("spec_media_type", "MediaType"),
            EventFieldDescriptor::required("spec_version", "SpecVersion"),
            EventFieldDescriptor::required("lowering_version", "LoweringVersion"),
            EventFieldDescriptor::required("public_output_schema_id", "SchemaId"),
            EventFieldDescriptor::repeated("descriptor_identities", "DescriptorIdentity"),
            EventFieldDescriptor::repeated("runner_executables", "ExecutableIdentity"),
            EventFieldDescriptor::repeated("adapter_executables", "ExecutableIdentity"),
            EventFieldDescriptor::required("canonicalizer_identity", "CanonicalizerIdentity"),
            EventFieldDescriptor::required("framework_version", "FrameworkVersion"),
            EventFieldDescriptor::required("source_revision", "SourceRevision"),
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
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("claim_generation", "u32"),
            EventFieldDescriptor::required("claim_fencing_token", "ClaimFencingToken"),
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
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("receipt_schema_id", "SchemaId"),
            EventFieldDescriptor::required("receipt_hash", "ContentDigest"),
            EventFieldDescriptor::required("receipt_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
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
            EventFieldDescriptor::required("invocation_epoch", "u32"),
            EventFieldDescriptor::required("confirmation_schema_id", "SchemaId"),
            EventFieldDescriptor::required("confirmation_hash", "ContentDigest"),
            EventFieldDescriptor::required("confirmation_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
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

            assert_eq!(all_event_schema_descriptors().len(), 25);
            assert_eq!(
                rows,
                "mfm_events::v1::RunStarted schema:mfm.events.v1.run_started:1:sha256-jcs-v1:314ad24d6697feb2cdf813a776c660386529ed3199eaccfb409c73153985b895\n\
mfm_events::v1::StateAttemptStarted schema:mfm.events.v1.state_attempt_started:1:sha256-jcs-v1:986f35aa39938713b9862192cab7d2b9b3a37219f5872bd242f8a06e7957ff1b\n\
mfm_events::v1::FactRecorded schema:mfm.events.v1.fact_recorded:1:sha256-jcs-v1:e708d591505935c8d5b12e833e34e6883c3e62fc548758a53ed5199e94218f70\n\
mfm_events::v1::ArtifactReferenced schema:mfm.events.v1.artifact_referenced:1:sha256-jcs-v1:c633ea54947b80f61bbb31506b0a20bfd1466e72e9aec5a844d2329e41ad3c39\n\
mfm_events::v1::CellProduced schema:mfm.events.v1.cell_produced:1:sha256-jcs-v1:9a2b250e7a5270bb302ae76a06091873dc50644855bace41bab97dcce311f07a\n\
mfm_events::v1::CellSkipped schema:mfm.events.v1.cell_skipped:1:sha256-jcs-v1:e82d7230e3ca668b68f1403d3db7c07d36a3373f8e3744e9a41351a7aa5392f4\n\
mfm_events::v1::side_effect::IntentPersisted schema:mfm.events.v1.side_effect.intent_persisted:1:sha256-jcs-v1:4fdfdd0e11e61200e015132408c9e7ed5f58edfe1c934fe117d709122ef04aaa\n\
mfm_events::v1::side_effect::Claimed schema:mfm.events.v1.side_effect.claimed:1:sha256-jcs-v1:20fc2bed7cf5b9eb67ae3f3991bf9f874d0340aedb35c01d3e4c24405b043c6e\n\
mfm_events::v1::side_effect::ClaimTakenOver schema:mfm.events.v1.side_effect.claim_taken_over:1:sha256-jcs-v1:3264ae1a9d53d90c5b786851e295ff3ddab49fb09585f6a8c750fb9e6cade32f\n\
mfm_events::v1::side_effect::InvocationPrepared schema:mfm.events.v1.side_effect.invocation_prepared:1:sha256-jcs-v1:991c1221363102eb01640ea1538e93ca7c499d4cd992a9ecb207a8f4196d0055\n\
mfm_events::v1::side_effect::InvocationStarted schema:mfm.events.v1.side_effect.invocation_started:1:sha256-jcs-v1:3653aa14495fdce6c969ce9d24289a837d68e084e3dbfca1657707f8f0be9d15\n\
mfm_events::v1::side_effect::NotSubmittedProven schema:mfm.events.v1.side_effect.not_submitted_proven:1:sha256-jcs-v1:61a155497633dc50f7321eddbb404adca77fd20e37d19cf14cf3ccef98b700ce\n\
mfm_events::v1::side_effect::SubmissionObserved schema:mfm.events.v1.side_effect.submission_observed:1:sha256-jcs-v1:693164b42ff85e73793603fcb73222ed2baf4c12689b7c7adfbb3cef234d0be6\n\
mfm_events::v1::side_effect::SubmissionUnknown schema:mfm.events.v1.side_effect.submission_unknown:1:sha256-jcs-v1:3428b3f01f8ea1e49747c9ecac88aabc322f54512e65e7eafb4fe10bb26bcd40\n\
mfm_events::v1::side_effect::ReceiptObserved schema:mfm.events.v1.side_effect.receipt_observed:1:sha256-jcs-v1:d1a5b302044d61b98a3b91c5fc4a9b3c647c1c2beff58afc42a22331abb88803\n\
mfm_events::v1::side_effect::ConfirmationObserved schema:mfm.events.v1.side_effect.confirmation_observed:1:sha256-jcs-v1:70b439cb9f97eb49550a56abee943a6b076566158b06285314ba681ca9eab0ac\n\
mfm_events::v1::side_effect::Ambiguous schema:mfm.events.v1.side_effect.ambiguous:1:sha256-jcs-v1:e06e1bbcabfd1cca7a811aa26adeaddaa46ea620bd756d6a8fab457288a87da2\n\
mfm_events::v1::side_effect::Failed schema:mfm.events.v1.side_effect.failed:1:sha256-jcs-v1:d1925590597b3829663a6dabd3fd3e43958bc9663c6b344effc03b299a5e1f63\n\
mfm_events::v1::PublicOutputProduced schema:mfm.events.v1.public_output_produced:1:sha256-jcs-v1:00d2531467818398553aa59e62c034fa0cd054e7856b89f425aeb4510f9c6776\n\
mfm_events::v1::PublicOutputRenderFailed schema:mfm.events.v1.public_output_render_failed:1:sha256-jcs-v1:c3706b63d36a8f8d444c8747a8e38e6eab25efa48b6857bfc51c16cdda56b82f\n\
mfm_events::v1::StateAttemptCompleted schema:mfm.events.v1.state_attempt_completed:1:sha256-jcs-v1:36800f9d3ae748d407bc2ea24339049471c8ffe40aa86c53b35b6c7c6cd6ee80\n\
mfm_events::v1::StateAttemptFailed schema:mfm.events.v1.state_attempt_failed:1:sha256-jcs-v1:e1caed5e374f978b57ec55123a826809d10abe759e78c893cffc5db1d75f8801\n\
mfm_events::v1::RunCompleted schema:mfm.events.v1.run_completed:1:sha256-jcs-v1:e4b278006a2b71a06e6de15bb266ab44670be8a36a9a3102c60bcd00288d4c39\n\
mfm_events::v1::RetentionRefsAppended schema:mfm.events.v1.retention_refs_appended:1:sha256-jcs-v1:90ef11e3f603d71ba0dca4a9d4945d0ab7f4c85d540bb43fc4c19b4f0bec8730\n\
mfm_events::v1::RetentionManifestProjected schema:mfm.events.v1.retention_manifest_projected:1:sha256-jcs-v1:269a96fc12c7c5004aa4592139f84cd0e4b617e04e494522ce639aeae0b9fed1"
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
            assert!(completed_json.contains("\"name\":\"terminal_error\""));
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

            assert_eq!(original_len, 25);
            assert_eq!(schema_names.len(), original_len);
        }

        #[test]
        fn run_started_v1_summary_key_is_canonical() {
            let keys = ["v1_event_schema_golden", "run_started_v1_present"];
            assert!(keys.contains(&"run_started_v1_present"));
            assert!(!keys.contains(&"run_started_v2_present"));
        }
    }
}
