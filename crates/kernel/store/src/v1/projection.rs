use super::*;

#[path = "projection_logic.rs"]
mod projection_logic;
pub(super) use self::projection_logic::{
    apply_projection, apply_projection_for_external_fact_queries, fact_claim_projection_key,
};

/// Cell terminal projection derived from committed run events.
///
/// Both variants already carry many identity/digest fields; boxing a single digest would not
/// meaningfully shrink the type and would complicate exact-evidence authority fields.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellTerminalProjection {
    /// Produced cell projection.
    Produced {
        /// Store-owned event id that produced the terminal projection.
        event_id: EventId,
        /// Producer node id.
        node_id: NodeId,
        /// Attempt id.
        attempt_id: AttemptId,
        /// Schema id.
        schema_id: SchemaId,
        /// Semantic type id.
        semantic_type_id: SemanticTypeId,
        /// Value artifact id.
        artifact_id: ArtifactId,
        /// Canonical content digest.
        content_digest: ContentDigest,
        /// Exact retained-artifact evidence identity.
        evidence_hash: ContentDigest,
    },
    /// Skipped cell projection.
    Skipped {
        /// Store-owned event id that produced the terminal projection.
        event_id: EventId,
        /// Producer node id.
        node_id: NodeId,
        /// Attempt id.
        attempt_id: AttemptId,
        /// Schema id.
        schema_id: SchemaId,
        /// Semantic type id.
        semantic_type_id: SemanticTypeId,
        /// Skip reason.
        skip_reason: events::SkipReason,
    },
}

impl CellTerminalProjection {
    fn attempt_key(&self) -> (&NodeId, &AttemptId) {
        match self {
            Self::Produced {
                node_id,
                attempt_id,
                ..
            }
            | Self::Skipped {
                node_id,
                attempt_id,
                ..
            } => (node_id, attempt_id),
        }
    }
}

/// Attempt lifecycle projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptProjection {
    /// Run id.
    pub run_id: RunId,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Last event id that updated the attempt.
    pub event_id: EventId,
    /// Current attempt status.
    pub status: AttemptStatus,
}

/// Attempt lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptStatus {
    /// Attempt has started.
    Started {
        /// Attempt number.
        attempt_no: u32,
        /// State kind.
        state_kind: StateKind,
        /// State version.
        state_version: StateVersion,
    },
    /// Attempt completed.
    Completed {
        /// Output cell id produced by the attempt.
        output_cell_id: CellId,
    },
    /// Attempt failed.
    Failed {
        /// Whether retry is allowed.
        retryable: bool,
        /// Redaction-safe error information.
        error: Box<events::MfmErrorInfo>,
    },
    /// Attempt was interrupted and may be retried.
    Interrupted,
}

/// Descriptor catalog projection derived from certified descriptor artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactDescriptorProjection {
    /// Canonical descriptor content hash.
    pub descriptor_hash: ContentDigest,
    /// Descriptor artifact id.
    pub descriptor_artifact_id: ArtifactId,
    /// Exact descriptor artifact evidence.
    pub descriptor_artifact_evidence: ArtifactEvidenceRef,
    /// Fact kind declared by the descriptor.
    pub fact_kind: mfm_facts::FactKind,
    /// Descriptor schema id.
    pub descriptor_schema_id: SchemaId,
    /// Subject schema id.
    pub subject_schema_id: SchemaId,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Descriptor-derived subject namespace hash.
    pub fact_subject_namespace_hash: ContentDigest,
}

/// Backend-facing query projection for one recorded platform fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactQueryProjection {
    pub(crate) fact_claim_id: mfm_facts::FactClaimId,
    pub(crate) source_run_id: RunId,
    pub(crate) source_seq: u64,
    pub(crate) source_ordinal: u32,
    pub(crate) source_event_id: EventId,
    pub(crate) producer_node_id: NodeId,
    pub(crate) attempt_id: AttemptId,
    pub(crate) commit_id: CommitKey,
    pub(crate) store_commit_order: u64,
    pub(crate) recorded_at: String,
    pub(crate) fact_kind: mfm_facts::FactKind,
    pub(crate) fact_descriptor_hash: ContentDigest,
    pub(crate) fact_subject_namespace_hash: ContentDigest,
    pub(crate) fact_key: mfm_facts::FactKey,
    pub(crate) subject_material_hash: ContentDigest,
    pub(crate) response_schema_id: SchemaId,
    pub(crate) response_hash: ContentDigest,
    pub(crate) artifact_id: ArtifactId,
    pub(crate) artifact_evidence_hash: ContentDigest,
    pub(crate) response_artifact_evidence: Option<ArtifactEvidenceRef>,
    pub(crate) terms: BTreeMap<mfm_facts::FactFieldId, mfm_facts::FactQueryTerm>,
}

impl FactQueryProjection {
    /// Builds a query projection from a committed `FactRecorded` event and retained response
    /// evidence.
    pub fn from_recorded_event(
        envelope: &KernelEventEnvelope,
        payload: &mfm_events::v1::FactRecorded,
        response_artifact_evidence: Option<ArtifactEvidenceRef>,
        terms: Vec<mfm_facts::FactQueryTerm>,
    ) -> Result<Self> {
        let fact_claim_id = mfm_facts::derive_fact_claim_id(
            envelope.run_id().clone(),
            envelope.seq().as_u64(),
            envelope.ordinal().as_u32(),
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?;
        let recorded_at = "1970-01-01T00:00:00Z".to_owned();
        let fact_ref = mfm_facts::InternalFactRef::from_claim(
            fact_claim_id,
            envelope.event_id().clone(),
            recorded_at.clone(),
            payload.node_id.clone(),
            &payload.claim,
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?;
        let store_commit_order = envelope.store_commit_order().as_u64();
        let _metadata =
            mfm_facts::FactExtractionMetadata::new(recorded_at.clone(), store_commit_order)
                .map_err(|error| StoreError::Identity(error.to_string()))?;
        Self::from_internal_ref(
            &fact_ref,
            payload.attempt_id.clone(),
            envelope.commit_key().clone(),
            store_commit_order,
            response_artifact_evidence,
            terms,
        )
    }

    /// Rehydrates a query projection from its durable fact reference and store coordinates.
    pub fn from_internal_ref(
        fact_ref: &mfm_facts::InternalFactRef,
        attempt_id: AttemptId,
        commit_id: CommitKey,
        store_commit_order: u64,
        response_artifact_evidence: Option<ArtifactEvidenceRef>,
        terms: Vec<mfm_facts::FactQueryTerm>,
    ) -> Result<Self> {
        let mut terms_by_field = BTreeMap::new();
        for term in terms {
            let field_id = term.field_id().clone();
            if terms_by_field.insert(field_id.clone(), term).is_some() {
                return Err(StoreError::ProjectionConflict {
                    key: format!("fact_query_term:{field_id}"),
                    message: "duplicate fact query term field id".to_owned(),
                });
            }
        }
        let projection = Self {
            fact_claim_id: fact_ref.fact_claim_id().clone(),
            source_run_id: fact_ref.fact_claim_id().source_run_id().clone(),
            source_seq: fact_ref.fact_claim_id().source_seq(),
            source_ordinal: fact_ref.fact_claim_id().source_ordinal(),
            source_event_id: fact_ref.source_event_id().clone(),
            producer_node_id: fact_ref.producer_node_id().clone(),
            attempt_id,
            commit_id,
            store_commit_order,
            recorded_at: fact_ref.recorded_at().to_owned(),
            fact_kind: fact_ref.fact_kind().clone(),
            fact_descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
            fact_subject_namespace_hash: fact_ref.fact_subject_namespace_hash().clone(),
            fact_key: fact_ref.fact_key().clone(),
            subject_material_hash: fact_ref.subject_material_hash().clone(),
            response_schema_id: fact_ref.response_schema_id().clone(),
            response_hash: fact_ref.response_hash().clone(),
            artifact_id: fact_ref.artifact_id().clone(),
            artifact_evidence_hash: fact_ref.artifact_evidence_hash().clone(),
            response_artifact_evidence,
            terms: terms_by_field,
        };
        projection.internal_ref()?;
        Ok(projection)
    }

    /// Builds the durable internal fact reference represented by this query projection.
    pub fn internal_ref(&self) -> Result<mfm_facts::InternalFactRef> {
        let parts = mfm_facts::InternalFactRefParts {
            fact_claim_id: self.fact_claim_id.clone(),
            source_event_id: self.source_event_id.clone(),
            recorded_at: self.recorded_at.clone(),
            producer_node_id: self.producer_node_id.clone(),
            fact_kind: self.fact_kind.clone(),
            fact_descriptor_hash: self.fact_descriptor_hash.clone(),
            subject: mfm_facts::FactSubjectRef::new(
                self.fact_subject_namespace_hash.clone(),
                self.fact_key.clone(),
                self.subject_material_hash.clone(),
            ),
            response: mfm_facts::FactResponseEvidence::new(
                self.response_schema_id.clone(),
                self.response_hash.clone(),
                self.artifact_id.clone(),
                self.artifact_evidence_hash.clone(),
            ),
        };
        mfm_facts::InternalFactRef::new(parts)
            .map_err(|error| StoreError::Identity(error.to_string()))
    }

    /// Returns the store-derived claim identity.
    pub const fn fact_claim_id(&self) -> &mfm_facts::FactClaimId {
        &self.fact_claim_id
    }
    /// Returns the producing run identity.
    pub const fn source_run_id(&self) -> &RunId {
        &self.source_run_id
    }
    /// Returns the producing stream sequence.
    pub const fn source_seq(&self) -> u64 {
        self.source_seq
    }
    /// Returns the producing event ordinal.
    pub const fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }
    /// Returns the store-owned source event identity.
    pub const fn source_event_id(&self) -> &EventId {
        &self.source_event_id
    }
    /// Returns the producing node identity.
    pub const fn producer_node_id(&self) -> &NodeId {
        &self.producer_node_id
    }
    /// Returns the producing attempt identity.
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }
    /// Returns the settlement commit key.
    pub const fn commit_id(&self) -> &CommitKey {
        &self.commit_id
    }
    /// Returns the store-wide commit order.
    pub const fn store_commit_order(&self) -> u64 {
        self.store_commit_order
    }
    /// Returns the store-observed recording timestamp.
    pub fn recorded_at(&self) -> &str {
        &self.recorded_at
    }
    /// Returns the fact kind.
    pub const fn fact_kind(&self) -> &mfm_facts::FactKind {
        &self.fact_kind
    }
    /// Returns the fact descriptor hash.
    pub const fn fact_descriptor_hash(&self) -> &ContentDigest {
        &self.fact_descriptor_hash
    }
    /// Returns the fact subject namespace hash.
    pub const fn fact_subject_namespace_hash(&self) -> &ContentDigest {
        &self.fact_subject_namespace_hash
    }
    /// Returns the descriptor-derived fact key.
    pub const fn fact_key(&self) -> &mfm_facts::FactKey {
        &self.fact_key
    }
    /// Returns the canonical subject material hash.
    pub const fn subject_material_hash(&self) -> &ContentDigest {
        &self.subject_material_hash
    }
    /// Returns the response schema identity.
    pub const fn response_schema_id(&self) -> &SchemaId {
        &self.response_schema_id
    }
    /// Returns the canonical response hash.
    pub const fn response_hash(&self) -> &ContentDigest {
        &self.response_hash
    }
    /// Returns the response artifact identity.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }
    /// Returns the response artifact evidence hash.
    pub const fn artifact_evidence_hash(&self) -> &ContentDigest {
        &self.artifact_evidence_hash
    }
    /// Returns exact retained response artifact evidence when hydrated.
    pub const fn response_artifact_evidence(&self) -> Option<&ArtifactEvidenceRef> {
        self.response_artifact_evidence.as_ref()
    }

    /// Returns one descriptor-derived query term by field id.
    pub fn term(&self, field_id: &mfm_facts::FactFieldId) -> Option<&mfm_facts::FactQueryTerm> {
        self.terms.get(field_id)
    }

    /// Iterates descriptor-derived query terms in field-id order.
    pub fn terms(&self) -> impl ExactSizeIterator<Item = &mfm_facts::FactQueryTerm> {
        self.terms.values()
    }
}

/// Public-output projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicOutputProjection {
    /// Public output rendered successfully.
    Produced {
        /// Producing event id.
        event_id: EventId,
        /// Rendered output digest.
        rendered_digest: ContentDigest,
        /// Optional rendered artifact id.
        rendered_artifact_id: Option<ArtifactId>,
    },
    /// Public output render failed.
    RenderFailed {
        /// Failure event id.
        event_id: EventId,
        /// Redaction-safe error information.
        error: Box<events::MfmErrorInfo>,
    },
}

/// Retention projection derived from committed run events.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetentionProjection {
    /// Retained artifact refs by exact `(artifact_id, evidence_hash)` authority.
    pub refs: BTreeMap<ArtifactAuthorityKey, events::RetentionRef>,
    /// Projected retention manifests by manifest sequence.
    pub manifests: BTreeMap<u64, RetentionManifestProjection>,
    /// Last retention manifest projected for this run.
    pub manifest: Option<RetentionManifestProjection>,
}

/// Retention manifest projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionManifestProjection {
    /// Manifest sequence.
    pub manifest_seq: u64,
    /// Manifest digest.
    pub manifest_digest: ContentDigest,
    /// Previous manifest digest, when any.
    pub previous_manifest_digest: Option<ContentDigest>,
    /// Manifest artifact id.
    pub manifest_artifact_id: ArtifactId,
}

/// Store-owned projection snapshot derived from authoritative run streams.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionSnapshot {
    pub(super) run_states: BTreeMap<RunId, RunState>,
    pub(super) run_spec_hashes: BTreeMap<RunId, SpecHash>,
    pub(super) saga_policy_digests: BTreeMap<RunId, ContentDigest>,
    pub(super) run_completions: BTreeMap<RunId, RunCompletionProjection>,
    pub(super) saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
    pub(super) manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
    pub(super) attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
    pub(super) cells: BTreeMap<(RunId, CellId), CellTerminalProjection>,
    pub(super) fact_descriptors: BTreeMap<ContentDigest, FactDescriptorProjection>,
    pub(super) fact_query_entries: BTreeMap<mfm_facts::FactClaimId, FactQueryProjection>,
    pub(super) side_effects: BTreeMap<SideEffectPairLedgerRef, SideEffectProjection>,
    pub(super) resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
    pub(super) public_outputs: BTreeMap<(RunId, SchemaId), PublicOutputProjection>,
    pub(super) retentions: BTreeMap<RunId, RetentionProjection>,
}

/// Storage-owned projection maps used to construct a [`ProjectionSnapshot`].
///
/// Hydrating callers fill the projection families they own and default the rest, replacing the
/// previous telescoping `from_parts*` constructors.
#[derive(Debug, Default)]
pub struct ProjectionSnapshotParts {
    /// Run lifecycle states.
    pub run_states: BTreeMap<RunId, RunState>,
    /// Certified spec hash recorded at run start.
    pub run_spec_hashes: BTreeMap<RunId, SpecHash>,
    /// Saga policy digest recorded at run start.
    pub saga_policy_digests: BTreeMap<RunId, ContentDigest>,
    /// Terminal run completion projections.
    pub run_completions: BTreeMap<RunId, RunCompletionProjection>,
    /// First saga engagement per run.
    pub saga_engagements: BTreeMap<RunId, SagaEngagementProjection>,
    /// Manual resolution evidence per run.
    pub manual_resolutions: BTreeMap<RunId, ManualResolutionProjection>,
    /// Attempt projections.
    pub attempts: BTreeMap<(NodeId, AttemptId), AttemptProjection>,
    /// Terminal cell projections.
    pub cells: BTreeMap<(RunId, CellId), CellTerminalProjection>,
    /// Descriptor catalog projections.
    pub fact_descriptors: BTreeMap<ContentDigest, FactDescriptorProjection>,
    /// Queryable fact projections.
    pub fact_query_entries: BTreeMap<mfm_facts::FactClaimId, FactQueryProjection>,
    /// Side-effect pair projections.
    pub side_effects: BTreeMap<SideEffectPairLedgerRef, SideEffectProjection>,
    /// Cross-run resource lane projections.
    pub resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
    /// Public output projections.
    pub public_outputs: BTreeMap<(RunId, SchemaId), PublicOutputProjection>,
    /// Retention projections.
    pub retentions: BTreeMap<RunId, RetentionProjection>,
}

impl ProjectionSnapshotParts {
    /// Clones every projection family from an existing snapshot.
    pub fn from_snapshot(snapshot: &ProjectionSnapshot) -> Self {
        Self {
            run_states: snapshot.run_states.clone(),
            run_spec_hashes: snapshot.run_spec_hashes.clone(),
            saga_policy_digests: snapshot.saga_policy_digests.clone(),
            run_completions: snapshot.run_completions.clone(),
            saga_engagements: snapshot.saga_engagements.clone(),
            manual_resolutions: snapshot.manual_resolutions.clone(),
            attempts: snapshot.attempts.clone(),
            cells: snapshot.cells.clone(),
            fact_descriptors: snapshot.fact_descriptors.clone(),
            fact_query_entries: snapshot.fact_query_entries.clone(),
            side_effects: snapshot.side_effects.clone(),
            resource_lanes: snapshot.resource_lanes.clone(),
            public_outputs: snapshot.public_outputs.clone(),
            retentions: snapshot.retentions.clone(),
        }
    }

    fn replace_fact_authority_from(&mut self, authority: &ProjectionSnapshot) {
        self.fact_descriptors = authority.fact_descriptors.clone();
        self.fact_query_entries = authority.fact_query_entries.clone();
    }

    fn replace_resource_lanes_from(&mut self, authority: &ProjectionSnapshot) {
        self.resource_lanes = authority.resource_lanes.clone();
    }
}
