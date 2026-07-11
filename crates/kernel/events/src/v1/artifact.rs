use super::*;

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
    /// Exact retained-artifact evidence identity.
    pub evidence_hash: ContentDigest,
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
    /// Exact retained-artifact evidence identity.
    pub evidence_hash: ContentDigest,
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
    /// Retained request/response evidence for an external capability read.
    ExternalReadEvidence,
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
    /// Attempt-produced external capability read evidence artifact.
    AttemptExternalReadEvidence,
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
            Self::AttemptExternalReadEvidence => "attempt_external_read_evidence",
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
        Self::ExternalReadEvidence,
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
            Self::ExternalReadEvidence => ArtifactRoleContract {
                role: self,
                tag: "external_read_evidence",
                schema: ArtifactSchemaPolicy::ExactEvidenceSchema,
                semantic: ArtifactSemanticPolicy::Absent,
                producer: ArtifactProducerScope::NodeRequired,
                staging: ArtifactStagingClass::AttemptExternalReadEvidence,
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
    /// Expected exact retained-artifact evidence identity.
    pub evidence_hash: ContentDigest,
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

pub(super) fn event_artifact_requirements(
    payload: &KernelEventPayload,
) -> Vec<EventArtifactRequirement> {
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
                evidence_hash: response.artifact_evidence_hash().clone(),
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
                evidence_hash: payload.evidence_hash.clone(),
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
                    evidence_hash: cell.evidence_hash.clone(),
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
            if let (Some(artifact_id), Some(evidence_hash)) = (
                &payload.rendered_artifact_id,
                &payload.rendered_artifact_evidence_hash,
            ) {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::PublicOutputRendered,
                    artifact_id: artifact_id.clone(),
                    evidence_hash: evidence_hash.clone(),
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
                evidence_hash: payload.evidence_artifact_evidence_hash.clone(),
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
                evidence_hash: payload.authorization_artifact_evidence_hash.clone(),
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
                evidence_hash: payload.intent_artifact_evidence_hash.clone(),
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
            if let (Some(artifact_id), Some(hash), Some(evidence_hash)) = (
                &payload.prepared_artifact_id,
                &payload.prepared_hash,
                &payload.prepared_artifact_evidence_hash,
            ) {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::PreparedInvocation,
                    artifact_id: artifact_id.clone(),
                    evidence_hash: evidence_hash.clone(),
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
                evidence_hash: payload.proof_artifact_evidence_hash.clone(),
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
                evidence_hash: payload.submission_artifact_evidence_hash.clone(),
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
                evidence_hash: payload.evidence_artifact_evidence_hash.clone(),
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
                evidence_hash: payload.receipt_artifact_evidence_hash.clone(),
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
                evidence_hash: payload.confirmation_artifact_evidence_hash.clone(),
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
                evidence_hash: payload.evidence_artifact_evidence_hash.clone(),
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
                    evidence_hash: retention_ref.evidence_hash.clone(),
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
                evidence_hash: payload.manifest_artifact_evidence_hash.clone(),
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
        evidence_hash: evidence.evidence_hash.clone(),
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
        evidence_hash: evidence.evidence_hash.clone(),
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
        evidence_hash: evidence.evidence_artifact_evidence_hash.clone(),
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
    /// Exact retained-artifact evidence identity for the cell value artifact.
    pub evidence_hash: ContentDigest,
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
    /// Exact canonical artifact evidence identity retained.
    pub evidence_hash: ContentDigest,
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
