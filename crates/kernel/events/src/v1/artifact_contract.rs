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
