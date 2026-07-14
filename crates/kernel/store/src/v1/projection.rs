use super::*;

#[path = "projection_logic.rs"]
mod projection_logic;
pub(super) use self::projection_logic::{
    apply_projection, apply_projection_for_external_fact_indexes, fact_claim_projection_key,
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

/// Store-owned projection for every recorded fact claim, indexed or private.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactRecordProjection {
    /// Store-derived claim id from run-stream coordinates.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Store-owned event id that recorded the fact.
    pub source_event_id: EventId,
    /// Producing run id.
    pub source_run_id: RunId,
    /// Producing stream sequence.
    pub source_seq: u64,
    /// Producing event ordinal.
    pub source_ordinal: u32,
    /// Producing node id.
    pub node_id: NodeId,
    /// Producing attempt id.
    pub attempt_id: AttemptId,
    /// Exact response artifact evidence when this projection was hydrated with artifact authority.
    pub response_artifact_evidence: Option<ArtifactEvidenceRef>,
    /// Normalized claim payload.
    pub claim: mfm_facts::FactClaim,
}

impl FactRecordProjection {
    /// Builds the store-owned fact record projection for a `FactRecorded` event.
    ///
    /// The fact claim id is derived from the event envelope's run-stream coordinates. Attempt
    /// state validation, duplicate response-artifact checks, and indexed visibility checks remain
    /// the caller's responsibility because they depend on the surrounding projection state.
    pub fn from_recorded_event(
        envelope: &KernelEventEnvelope,
        payload: &mfm_events::v1::FactRecorded,
        response_artifact_evidence: Option<ArtifactEvidenceRef>,
    ) -> Result<Self> {
        let fact_claim_id = mfm_facts::derive_fact_claim_id(
            envelope.run_id().clone(),
            envelope.seq().as_u64(),
            envelope.ordinal().as_u32(),
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?;
        Ok(Self {
            fact_claim_id,
            source_event_id: envelope.event_id().clone(),
            source_run_id: envelope.run_id().clone(),
            source_seq: envelope.seq().as_u64(),
            source_ordinal: envelope.ordinal().as_u32(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            response_artifact_evidence,
            claim: payload.claim.clone(),
        })
    }

    /// Returns true when this stream-derived record projection agrees with an indexed row.
    pub fn matches_index_projection(&self, index: &FactIndexProjection) -> bool {
        let record_ref =
            match internal_fact_ref_from_record_projection(self, index.recorded_at.clone()) {
                Ok(Some(fact_ref)) => fact_ref,
                Ok(None) | Err(_) => return false,
            };
        let index_ref = match index.internal_ref() {
            Ok(fact_ref) => fact_ref,
            Err(_) => return false,
        };
        self.fact_claim_id == index.fact_claim_id
            && self.source_run_id == index.source_run_id
            && self.source_seq == index.source_seq
            && self.source_ordinal == index.source_ordinal
            && self.source_event_id == index.source_event_id
            && self.node_id == index.producer_node_id
            && record_ref == index_ref
    }
}

fn internal_fact_ref_from_record_projection(
    record: &FactRecordProjection,
    recorded_at: String,
) -> Result<Option<mfm_facts::InternalFactRef>> {
    mfm_facts::InternalFactRef::from_claim(
        record.fact_claim_id.clone(),
        record.source_event_id.clone(),
        recorded_at,
        record.node_id.clone(),
        &record.claim,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))
}

/// Queryable indexed fact projection for one recorded claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexProjection {
    /// Store-derived claim id from run-stream coordinates.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Producing run id.
    pub source_run_id: RunId,
    /// Producing stream sequence.
    pub source_seq: u64,
    /// Producing event ordinal.
    pub source_ordinal: u32,
    /// Store-owned event id that recorded the fact.
    pub source_event_id: EventId,
    /// Producing node id.
    pub producer_node_id: NodeId,
    /// Commit idempotency key for the append.
    pub commit_id: CommitKey,
    /// Deterministic store commit ordering coordinate.
    pub store_commit_order: u64,
    /// Store-observed record timestamp.
    pub recorded_at: String,
    /// Optional source observation timestamp.
    pub observed_at: Option<String>,
    /// Indexed audience.
    pub audience: mfm_facts::FactAudience,
    /// Indexed visibility scope.
    pub visibility_scope: mfm_facts::FactVisibilityScope,
    /// Fact kind.
    pub fact_kind: mfm_facts::FactKind,
    /// Fact descriptor hash.
    pub fact_descriptor_hash: ContentDigest,
    /// Subject namespace hash.
    pub fact_subject_namespace_hash: ContentDigest,
    /// Descriptor-derived fact key.
    pub fact_key: mfm_facts::FactKey,
    /// Canonical subject material hash.
    pub subject_material_hash: ContentDigest,
    /// Request schema id, when request evidence is present.
    pub request_schema_id: Option<SchemaId>,
    /// Canonical request hash, when request evidence is present.
    pub request_hash: Option<ContentDigest>,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Canonical response hash.
    pub response_hash: ContentDigest,
    /// Response artifact id.
    pub artifact_id: ArtifactId,
    /// Canonical response artifact evidence hash.
    pub artifact_evidence_hash: ContentDigest,
    /// Adapter capability kind.
    pub capability_kind: CapabilityKind,
    /// Adapter capability version.
    pub capability_version: CapabilityVersion,
    /// Adapter kind.
    pub adapter_kind: AdapterKind,
    /// Adapter version.
    pub adapter_version: AdapterVersion,
}

impl FactIndexProjection {
    /// Builds an indexed fact projection from a recorded fact projection.
    ///
    /// Run-private records return `Ok(None)`. Indexed records are validated through the same
    /// internal reference shape used by fact query and replay surfaces.
    pub fn from_record_projection(
        record: &FactRecordProjection,
        commit_id: CommitKey,
        store_commit_order: u64,
        recorded_at: impl Into<String>,
    ) -> Result<Option<Self>> {
        let recorded_at = recorded_at.into();
        let Some(fact_ref) = internal_fact_ref_from_record_projection(record, recorded_at.clone())?
        else {
            return Ok(None);
        };
        let _metadata = mfm_facts::FactExtractionMetadata::new(
            recorded_at.clone(),
            fact_ref.observed_at().map(str::to_owned),
            store_commit_order,
        )
        .map_err(|error| StoreError::Identity(error.to_string()))?;
        let projection = Self::from_internal_ref(&fact_ref, commit_id, store_commit_order)?;
        projection.internal_ref()?;
        if !record.matches_index_projection(&projection) {
            return Err(StoreError::ProjectionConflict {
                key: format!("fact-index:{:?}", projection.fact_claim_id),
                message: "fact index projection does not match recorded fact".to_owned(),
            });
        }
        Ok(Some(projection))
    }

    fn from_internal_ref(
        fact_ref: &mfm_facts::InternalFactRef,
        commit_id: CommitKey,
        store_commit_order: u64,
    ) -> Result<Self> {
        let (audience, visibility_scope) = match fact_ref.visibility() {
            mfm_facts::FactVisibility::Indexed { audience, scope } => (*audience, *scope),
            mfm_facts::FactVisibility::RunPrivate => {
                return Err(StoreError::ProjectionConflict {
                    key: format!("fact-index:{:?}", fact_ref.fact_claim_id()),
                    message: "internal fact ref was not indexed".to_owned(),
                });
            }
        };
        Ok(Self {
            fact_claim_id: fact_ref.fact_claim_id().clone(),
            source_run_id: fact_ref.fact_claim_id().source_run_id().clone(),
            source_seq: fact_ref.fact_claim_id().source_seq(),
            source_ordinal: fact_ref.fact_claim_id().source_ordinal(),
            source_event_id: fact_ref.source_event_id().clone(),
            producer_node_id: fact_ref.producer_node_id().clone(),
            commit_id,
            store_commit_order,
            recorded_at: fact_ref.recorded_at().to_owned(),
            observed_at: fact_ref.observed_at().map(str::to_owned),
            audience,
            visibility_scope,
            fact_kind: fact_ref.fact_kind().clone(),
            fact_descriptor_hash: fact_ref.fact_descriptor_hash().clone(),
            fact_subject_namespace_hash: fact_ref.fact_subject_namespace_hash().clone(),
            fact_key: fact_ref.fact_key().clone(),
            subject_material_hash: fact_ref.subject_material_hash().clone(),
            request_schema_id: fact_ref.request_schema_id().cloned(),
            request_hash: fact_ref.request_hash().cloned(),
            response_schema_id: fact_ref.response_schema_id().clone(),
            response_hash: fact_ref.response_hash().clone(),
            artifact_id: fact_ref.artifact_id().clone(),
            artifact_evidence_hash: fact_ref.artifact_evidence_hash().clone(),
            capability_kind: fact_ref.capability_kind().clone(),
            capability_version: fact_ref.capability_version().clone(),
            adapter_kind: fact_ref.adapter_kind().clone(),
            adapter_version: fact_ref.adapter_version().clone(),
        })
    }

    /// Builds the durable internal fact reference represented by this index row.
    pub fn internal_ref(&self) -> Result<mfm_facts::InternalFactRef> {
        let request = match (&self.request_schema_id, &self.request_hash) {
            (Some(schema_id), Some(hash)) => Some(mfm_facts::FactRequestEvidence::new(
                schema_id.clone(),
                hash.clone(),
            )),
            (None, None) => None,
            _ => {
                return Err(StoreError::Identity(
                    "internal fact index row has partial request evidence".to_owned(),
                ));
            }
        };
        let parts = mfm_facts::InternalFactRefParts {
            fact_claim_id: self.fact_claim_id.clone(),
            source_event_id: self.source_event_id.clone(),
            recorded_at: self.recorded_at.clone(),
            producer_node_id: self.producer_node_id.clone(),
            observed_at: self.observed_at.clone(),
            visibility: mfm_facts::FactVisibility::Indexed {
                audience: self.audience,
                scope: self.visibility_scope,
            },
            fact_kind: self.fact_kind.clone(),
            fact_descriptor_hash: self.fact_descriptor_hash.clone(),
            subject: mfm_facts::FactSubjectRef::new(
                self.fact_subject_namespace_hash.clone(),
                self.fact_key.clone(),
                self.subject_material_hash.clone(),
            ),
            request,
            response: mfm_facts::FactResponseEvidence::new(
                self.response_schema_id.clone(),
                self.response_hash.clone(),
                self.artifact_id.clone(),
                self.artifact_evidence_hash.clone(),
            ),
            producer: mfm_facts::FactProducerProvenance::new(
                self.capability_kind.clone(),
                self.capability_version.clone(),
                self.adapter_kind.clone(),
                self.adapter_version.clone(),
            ),
        };
        mfm_facts::InternalFactRef::new(parts)
            .map_err(|error| StoreError::Identity(error.to_string()))
    }
}

/// Extracted index term projection for one indexed fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexTermProjection {
    /// Store-derived claim id from run-stream coordinates.
    pub fact_claim_id: mfm_facts::FactClaimId,
    /// Fact descriptor hash.
    pub fact_descriptor_hash: ContentDigest,
    /// Descriptor-owned field id.
    pub field_id: mfm_facts::FactFieldId,
    /// Descriptor field source category.
    pub source: mfm_facts::FactFieldSource,
    /// Descriptor value type.
    pub value_type: mfm_facts::FactFieldValueType,
    /// Extracted canonical scalar.
    pub value: mfm_facts::FactCanonicalScalar,
    /// Optional descriptor unit.
    pub unit: Option<mfm_facts::FactUnit>,
    /// Optional descriptor scale.
    pub scale: Option<mfm_facts::FactScale>,
}

impl FactIndexTermProjection {
    /// Builds an index term projection from descriptor-extracted fact term material.
    pub fn from_extracted_term(
        fact_claim_id: &mfm_facts::FactClaimId,
        fact_descriptor_hash: &ContentDigest,
        term: &mfm_facts::FactIndexTerm,
    ) -> Self {
        Self {
            fact_claim_id: fact_claim_id.clone(),
            fact_descriptor_hash: fact_descriptor_hash.clone(),
            field_id: term.field_id().clone(),
            source: term.source(),
            value_type: term.value_type(),
            value: term.value().clone(),
            unit: term.unit().cloned(),
            scale: term.scale(),
        }
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
    pub(super) fact_records: BTreeMap<mfm_facts::FactClaimId, FactRecordProjection>,
    pub(super) fact_index_entries: BTreeMap<mfm_facts::FactClaimId, FactIndexProjection>,
    pub(super) fact_term_entries:
        BTreeMap<(mfm_facts::FactClaimId, mfm_facts::FactFieldId), FactIndexTermProjection>,
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
    /// Recorded fact projections.
    pub fact_records: BTreeMap<mfm_facts::FactClaimId, FactRecordProjection>,
    /// Indexed fact projections.
    pub fact_index_entries: BTreeMap<mfm_facts::FactClaimId, FactIndexProjection>,
    /// Extracted fact term projections.
    pub fact_term_entries:
        BTreeMap<(mfm_facts::FactClaimId, mfm_facts::FactFieldId), FactIndexTermProjection>,
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
            fact_records: snapshot.fact_records.clone(),
            fact_index_entries: snapshot.fact_index_entries.clone(),
            fact_term_entries: snapshot.fact_term_entries.clone(),
            side_effects: snapshot.side_effects.clone(),
            resource_lanes: snapshot.resource_lanes.clone(),
            public_outputs: snapshot.public_outputs.clone(),
            retentions: snapshot.retentions.clone(),
        }
    }

    fn replace_fact_authority_from(&mut self, authority: &ProjectionSnapshot) {
        self.fact_descriptors = authority.fact_descriptors.clone();
        self.fact_records = authority.fact_records.clone();
        self.fact_index_entries = authority.fact_index_entries.clone();
        self.fact_term_entries = authority.fact_term_entries.clone();
    }

    fn replace_resource_lanes_from(&mut self, authority: &ProjectionSnapshot) {
        self.resource_lanes = authority.resource_lanes.clone();
    }
}
