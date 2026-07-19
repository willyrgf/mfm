use super::*;

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

#[path = "schema_types.rs"]
mod schema_types;
use self::schema_types::schema_field;

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

pub(super) const RUN_ADMITTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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
        EventFieldDescriptor::repeated(
            "capability_implementations",
            "CapabilityImplementationIdentity",
        ),
        EventFieldDescriptor::required("admitted_binding_digest", "ContentDigest"),
        EventFieldDescriptor::required("canonicalizer_identity", "CanonicalizerIdentity"),
        EventFieldDescriptor::repeated("seed_cells", "SeedCellRef"),
    ],
};

pub(super) const STATE_ATTEMPT_STARTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const FACT_RECORDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const ARTIFACT_REFERENCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const CELL_PRODUCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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
        EventFieldDescriptor::required("evidence_hash", "ContentDigest"),
        EventFieldDescriptor::optional("producer_state_kind", "StateKind"),
        EventFieldDescriptor::optional("producer_state_version", "StateVersion"),
    ],
};

pub(super) const CELL_SKIPPED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const SIDE_EFFECT_INTENT_PERSISTED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("intent_artifact_evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("idempotency_input_schema_id", "SchemaId"),
            EventFieldDescriptor::required("idempotency_input_hash", "ContentDigest"),
            EventFieldDescriptor::required("idempotency_key", "IdempotencyKeyRef"),
            EventFieldDescriptor::required("capability_kind", "CapabilityKind"),
            EventFieldDescriptor::required("capability_version", "CapabilityVersion"),
            EventFieldDescriptor::required("adapter_kind", "AdapterKind"),
            EventFieldDescriptor::required("adapter_version", "AdapterVersion"),
        ],
    };

pub(super) const SIDE_EFFECT_CLAIMED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const SIDE_EFFECT_CLAIM_TAKEN_OVER_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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

pub(super) const RESOURCE_LANE_CLAIMED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const RESOURCE_LANE_CLAIM_INTENT_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const SIDE_EFFECT_INVOCATION_PREPARED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("prepared_schema_id", "SchemaId"),
            EventFieldDescriptor::required("prepared_artifact_id", "ArtifactId"),
            EventFieldDescriptor::required("prepared_hash", "ContentDigest"),
            EventFieldDescriptor::required("prepared_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_INVOCATION_STARTED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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

pub(super) const SIDE_EFFECT_NOT_SUBMITTED_PROVEN_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("proof_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_SUBMISSION_OBSERVED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("submission_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_SUBMISSION_UNKNOWN_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("evidence_artifact_evidence_hash", "ContentDigest"),
        ],
    };

pub(super) const SIDE_EFFECT_RECEIPT_OBSERVED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("receipt_artifact_evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
            EventFieldDescriptor::optional("resource_touched_set", "ResourceTouchedSetEvidence",),
        ],
    };

pub(super) const SIDE_EFFECT_CONFIRMATION_OBSERVED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("confirmation_artifact_evidence_hash", "ContentDigest"),
            EventFieldDescriptor::required("replay_verifier_id", "ReplayVerifierId"),
            EventFieldDescriptor::optional("resource_touched_set", "ResourceTouchedSetEvidence",),
        ],
    };

pub(super) const SIDE_EFFECT_AMBIGUOUS_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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
        EventFieldDescriptor::required("evidence_artifact_evidence_hash", "ContentDigest"),
    ],
};

pub(super) const SIDE_EFFECT_FAILED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const RESOURCE_LANE_RELEASED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const RESOURCE_LANE_RELEASE_INTENT_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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

pub(super) const PUBLIC_OUTPUT_PRODUCED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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
        EventFieldDescriptor::optional("rendered_artifact_evidence_hash", "ContentDigest"),
        EventFieldDescriptor::required("renderer_descriptor_id", "DescriptorId"),
    ],
};

pub(super) const PUBLIC_OUTPUT_RENDER_FAILED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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

pub(super) const STATE_ATTEMPT_COMPLETED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const STATE_ATTEMPT_INTERRUPTED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.state_attempt_interrupted",
    rust_type_path: "mfm_events::v1::StateAttemptInterrupted",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("node_id", "NodeId"),
        EventFieldDescriptor::required("attempt_id", "AttemptId"),
    ],
};

pub(super) const STATE_ATTEMPT_FAILED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const MANUAL_RESOLUTION_RECORDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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
        EventFieldDescriptor::required("evidence_artifact_evidence_hash", "ContentDigest"),
        EventFieldDescriptor::required("authorization_schema_id", "SchemaId"),
        EventFieldDescriptor::required("authorization_hash", "ContentDigest"),
        EventFieldDescriptor::required("authorization_artifact_id", "ArtifactId"),
        EventFieldDescriptor::required("authorization_artifact_evidence_hash", "ContentDigest",),
        EventFieldDescriptor::optional("note", "ManualResolutionNote"),
    ],
};

pub(super) const RUN_COMPLETED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
    schema_name: "mfm.events.v1.run_completed",
    rust_type_path: "mfm_events::v1::RunCompleted",
    schema_version: EVENT_SCHEMA_VERSION,
    fields: fields![
        EventFieldDescriptor::required("run_id", "RunId"),
        EventFieldDescriptor::required("spec_hash", "SpecHash"),
        EventFieldDescriptor::required("outcome", "RunCompletionOutcome"),
    ],
};

pub(super) const RETENTION_REFS_APPENDED_SCHEMA: EventSchemaDescriptor = EventSchemaDescriptor {
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

pub(super) const RETENTION_MANIFEST_PROJECTED_SCHEMA: EventSchemaDescriptor =
    EventSchemaDescriptor {
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
            EventFieldDescriptor::required("manifest_artifact_evidence_hash", "ContentDigest"),
        ],
    };
