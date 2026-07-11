use super::*;

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

impl ProjectionSnapshot {
    /// Returns a snapshot with store-owned fact authority and resource lanes copied together.
    pub fn with_store_authority_from(&self, authority: &ProjectionSnapshot) -> Result<Self> {
        let mut parts = ProjectionSnapshotParts::from_snapshot(self);
        parts.replace_fact_authority_from(authority);
        parts.replace_resource_lanes_from(authority);
        Self::from_parts(parts)
    }

    /// Creates a projection snapshot from storage-owned projection maps.
    ///
    /// Callers populate only the projection families they hydrate and leave the rest empty via
    /// [`ProjectionSnapshotParts`]'s [`Default`].
    pub fn from_parts(parts: ProjectionSnapshotParts) -> Result<Self> {
        let ProjectionSnapshotParts {
            run_states,
            run_spec_hashes,
            saga_policy_digests,
            run_completions,
            saga_engagements,
            manual_resolutions,
            attempts,
            cells,
            fact_descriptors,
            fact_records,
            fact_index_entries,
            fact_term_entries,
            side_effects,
            resource_lanes,
            public_outputs,
            retentions,
        } = parts;
        for ((node_id, attempt_id), projection) in &attempts {
            if node_id != &projection.node_id || attempt_id != &projection.attempt_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("attempt:{}:{}", projection.node_id, projection.attempt_id),
                    message: "attempt projection key does not match projection identity".to_owned(),
                });
            }
        }
        for ((run_id, cell_id), projection) in &cells {
            let (projection_node_id, projection_attempt_id) = projection.attempt_key();
            let Some(attempt) =
                attempts.get(&(projection_node_id.clone(), projection_attempt_id.clone()))
            else {
                return Err(StoreError::ProjectionConflict {
                    key: format!("cell:{run_id}:{cell_id}:terminal"),
                    message: "cell projection references missing attempt projection".to_owned(),
                });
            };
            if &attempt.run_id != run_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("cell:{run_id}:{cell_id}:terminal"),
                    message: "cell projection key does not match attempt run id".to_owned(),
                });
            }
        }
        for (descriptor_hash, projection) in &fact_descriptors {
            if descriptor_hash != &projection.descriptor_hash {
                return Err(StoreError::ProjectionConflict {
                    key: format!("fact_descriptor:{}", projection.descriptor_hash),
                    message: "fact descriptor projection key does not match descriptor hash"
                        .to_owned(),
                });
            }
        }
        for (claim_id, projection) in &fact_records {
            if claim_id != &projection.fact_claim_id {
                return Err(StoreError::ProjectionConflict {
                    key: fact_claim_projection_key("fact_record", claim_id),
                    message: "fact record projection key does not match claim id".to_owned(),
                });
            }
        }
        for (claim_id, projection) in &fact_index_entries {
            if claim_id != &projection.fact_claim_id {
                return Err(StoreError::ProjectionConflict {
                    key: fact_claim_projection_key("fact_index", claim_id),
                    message: "fact index projection key does not match claim id".to_owned(),
                });
            }
        }
        for ((claim_id, field_id), projection) in &fact_term_entries {
            if claim_id != &projection.fact_claim_id || field_id != &projection.field_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "{}:{}",
                        fact_claim_projection_key("fact_term", claim_id),
                        field_id
                    ),
                    message: "fact term projection key does not match projection identity"
                        .to_owned(),
                });
            }
        }
        for (ledger_ref, projection) in &side_effects {
            if ledger_ref.run_id != projection.run_id || ledger_ref.pair_id != projection.pair_id {
                return Err(StoreError::ProjectionConflict {
                    key: format!("sidefx_pair:{}", ledger_ref.pair_id),
                    message: "side-effect projection key does not match pair reference".to_owned(),
                });
            }
            projection.ledger_state()?;
        }
        Ok(Self {
            run_states,
            run_spec_hashes,
            saga_policy_digests,
            run_completions,
            saga_engagements,
            manual_resolutions,
            attempts,
            cells,
            fact_descriptors,
            fact_records,
            fact_index_entries,
            fact_term_entries,
            side_effects,
            resource_lanes,
            public_outputs,
            retentions,
        })
    }

    /// Validates that a loaded run stream is ordered and contiguous.
    pub fn validate_run_stream(events: &[KernelEventEnvelope]) -> Result<()> {
        validate_run_stream_order(events)?;
        validate_supported_stream_model(events)
    }

    /// Rebuilds projections from store-owned event envelopes.
    pub fn rebuild_from_run_stream(events: &[KernelEventEnvelope]) -> Result<Self> {
        Self::validate_run_stream(events)?;
        for event in events {
            match event.payload() {
                KernelEventPayload::RunAdmitted(payload)
                    if !payload.fact_descriptor_artifacts.is_empty() =>
                {
                    return Err(StoreError::ProjectionConflict {
                        key: "fact_descriptor:artifact_bytes".to_owned(),
                        message:
                            "fact descriptor projection requires retained descriptor artifact bytes"
                                .to_owned(),
                    });
                }
                KernelEventPayload::FactRecorded(_) => {
                    return Err(StoreError::ProjectionConflict {
                        key: "fact:artifact_bytes".to_owned(),
                        message: "fact projection requires retained artifact bytes".to_owned(),
                    });
                }
                _ => {}
            }
        }
        let mut snapshot = Self::default();
        let artifact_bytes = ArtifactByteAuthorityMap::new();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection(&mut snapshot, &event, &artifact_bytes)?;
            }
        }
        Ok(snapshot)
    }

    /// Rebuilds projections from a run stream using exact retained artifact bytes.
    ///
    /// Durable stores use this for validation and physical projection rebuilds when fact descriptor
    /// and response artifacts are already loaded from authoritative storage. The supplied artifact
    /// byte authority must be keyed by exact `(artifact_id, evidence_hash)`.
    pub fn rebuild_from_run_stream_with_artifact_bytes(
        events: &[KernelEventEnvelope],
        artifact_bytes: &ArtifactByteAuthorityMap,
    ) -> Result<Self> {
        Self::validate_run_stream(events)?;
        let mut snapshot = Self::default();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection(&mut snapshot, &event, artifact_bytes)?;
            }
        }
        Ok(snapshot)
    }

    /// Rebuilds non-fact projections plus fact record identities for stores with physical fact indexes.
    ///
    /// This helper is for stores that maintain descriptor, index, and term projections in separate
    /// validated tables. It does not rebuild queryable fact indexes from the stream, and it is not a
    /// replay validation substitute for retained descriptor and response artifact authority.
    pub fn rebuild_for_external_fact_indexes(events: &[KernelEventEnvelope]) -> Result<Self> {
        Self::validate_run_stream(events)?;
        let mut snapshot = Self::default();
        for commit in committed_run_stream_commits(events) {
            let payloads = commit
                .events
                .iter()
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            validate_terminal_attempt_cell_pairs(&payloads)?;
            validate_terminal_side_effect_evidence_pairs(&payloads)?;
            validate_side_effect_attempt_failures_have_terminal_evidence(&snapshot, &payloads)?;
            validate_retention_manifest_pairs(&payloads)?;
            for event in commit.events {
                projection::apply_projection_for_external_fact_indexes(&mut snapshot, &event)?;
            }
        }
        Ok(snapshot)
    }

    /// Returns the run state for a run id.
    pub fn run_state(&self, run_id: &RunId) -> RunState {
        self.run_states
            .get(run_id)
            .copied()
            .unwrap_or(RunState::Absent)
    }

    /// Returns the certified spec hash recorded at run start.
    pub fn run_spec_hash(&self, run_id: &RunId) -> Option<&SpecHash> {
        self.run_spec_hashes.get(run_id)
    }

    /// Returns the saga policy digest recorded at run start.
    pub fn saga_policy_digest(&self, run_id: &RunId) -> Option<&ContentDigest> {
        self.saga_policy_digests.get(run_id)
    }

    /// Returns the run completion projection for a run id.
    pub fn run_completion(&self, run_id: &RunId) -> Option<&RunCompletionProjection> {
        self.run_completions.get(run_id)
    }

    /// Returns the first saga engagement projection for a run id.
    pub fn saga_engagement(&self, run_id: &RunId) -> Option<&SagaEngagementProjection> {
        self.saga_engagements.get(run_id)
    }

    /// Returns the manual resolution projection for a run id.
    pub fn manual_resolution(&self, run_id: &RunId) -> Option<&ManualResolutionProjection> {
        self.manual_resolutions.get(run_id)
    }

    /// Returns a cell terminal projection.
    pub fn cell_terminal(&self, cell_id: &CellId) -> Option<&CellTerminalProjection> {
        self.cells
            .iter()
            .find_map(|((_run_id, key_cell_id), projection)| {
                (key_cell_id == cell_id).then_some(projection)
            })
    }

    /// Returns a cell terminal projection for a specific run.
    pub fn cell_terminal_for_run(
        &self,
        run_id: &RunId,
        cell_id: &CellId,
    ) -> Option<&CellTerminalProjection> {
        self.cells.get(&(run_id.clone(), cell_id.clone()))
    }

    /// Returns a descriptor catalog projection.
    pub fn fact_descriptor(
        &self,
        descriptor_hash: &ContentDigest,
    ) -> Option<&FactDescriptorProjection> {
        self.fact_descriptors.get(descriptor_hash)
    }

    /// Returns a recorded fact projection.
    pub fn fact_record(&self, claim_id: &mfm_facts::FactClaimId) -> Option<&FactRecordProjection> {
        self.fact_records.get(claim_id)
    }

    /// Returns an indexed fact projection.
    pub fn fact_index_entry(
        &self,
        claim_id: &mfm_facts::FactClaimId,
    ) -> Option<&FactIndexProjection> {
        self.fact_index_entries.get(claim_id)
    }

    /// Returns an extracted fact term for a claim and field id.
    pub fn fact_term(
        &self,
        claim_id: &mfm_facts::FactClaimId,
        field_id: &mfm_facts::FactFieldId,
    ) -> Option<&FactIndexTermProjection> {
        self.fact_term_entries
            .get(&(claim_id.clone(), field_id.clone()))
    }

    /// Iterates extracted fact terms for one claim id.
    pub fn fact_terms_for_claim<'a>(
        &'a self,
        claim_id: &'a mfm_facts::FactClaimId,
    ) -> impl Iterator<Item = &'a FactIndexTermProjection> + 'a {
        self.fact_term_entries
            .iter()
            .filter(move |((term_claim_id, _field_id), _term)| term_claim_id == claim_id)
            .map(|(_key, term)| term)
    }

    /// Returns an attempt lifecycle projection.
    pub fn attempt(&self, node_id: &NodeId, attempt_id: &AttemptId) -> Option<&AttemptProjection> {
        self.attempts.get(&(node_id.clone(), attempt_id.clone()))
    }

    /// Returns the first open semantic attempt projected for a run.
    pub fn open_attempt_for_run(&self, run_id: &RunId) -> Option<&AttemptProjection> {
        self.attempts.values().find(|attempt| {
            &attempt.run_id == run_id && matches!(attempt.status, AttemptStatus::Started { .. })
        })
    }

    /// Requires that a run prefix has no open semantic attempt.
    pub fn require_no_open_semantic_attempts_for_run(&self, run_id: &RunId) -> Result<()> {
        if let Some(attempt) = self.open_attempt_for_run(run_id) {
            return Err(StoreError::ProjectionConflict {
                key: format!("run:{run_id}:attempts"),
                message: format!(
                    "manual resolution requires no open semantic attempts; attempt {}:{} is still started",
                    attempt.node_id, attempt.attempt_id
                ),
            });
        }
        Ok(())
    }

    /// Returns a side-effect projection for a run-scoped certified pair id.
    pub fn side_effect_for_pair(
        &self,
        run_id: &RunId,
        pair_id: &SideEffectPairId,
    ) -> Option<&SideEffectProjection> {
        self.side_effects.get(&SideEffectPairLedgerRef::new(
            run_id.clone(),
            pair_id.clone(),
        ))
    }

    /// Returns a validated side-effect ledger state for a run-scoped certified pair id.
    pub fn side_effect_state_for_pair(
        &self,
        run_id: &RunId,
        pair_id: &SideEffectPairId,
    ) -> Result<Option<SideEffectLedgerState<'_>>> {
        self.side_effect_for_pair(run_id, pair_id)
            .map(SideEffectProjection::ledger_state)
            .transpose()
    }

    /// Returns an active resource lane holder.
    pub fn resource_lane(&self, key: &ResourceLaneKey) -> Option<&ResourceLaneProjection> {
        self.resource_lanes.get(key)
    }

    /// Returns a public-output projection.
    pub fn public_output(
        &self,
        run_id: &RunId,
        schema_id: &SchemaId,
    ) -> Option<&PublicOutputProjection> {
        self.public_outputs
            .get(&(run_id.clone(), schema_id.clone()))
    }

    /// Returns a retention projection.
    pub fn retention(&self, run_id: &RunId) -> Option<&RetentionProjection> {
        self.retentions.get(run_id)
    }

    /// Returns whether terminal public-output authority is projected.
    pub fn has_public_output(&self, run_id: &RunId) -> bool {
        self.public_outputs
            .iter()
            .any(|((projection_run_id, _), projection)| {
                projection_run_id == run_id
                    && matches!(projection, PublicOutputProjection::Produced { .. })
            })
    }

    /// Returns whether all past-boundary forward ledgers for the current projection are quiescent.
    pub fn forward_ledgers_quiescent(
        &self,
        run_id: &RunId,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<bool> {
        forward_ledgers_quiescent(self, run_id, terminal_policies)
    }

    /// Derives saga status from certified saga policy plus the current stream projection.
    pub fn derive_saga_projection(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<SagaProjection> {
        derive_saga_projection(self, run_id, policy, terminal_policies)
    }

    /// Requires that the current prefix derives a manual-blocked saga mode.
    pub fn require_manual_resolution_admissible(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<()> {
        self.require_no_open_semantic_attempts_for_run(run_id)?;
        let saga = self.derive_saga_projection(run_id, policy, terminal_policies)?;
        if saga.run_mode == RunMode::ManualBlocked {
            Ok(())
        } else {
            Err(StoreError::ProjectionConflict {
                key: format!("run:{run_id}:manual_resolution"),
                message: format!(
                    "manual resolution requires prefix-derived manual_blocked saga mode, found {}",
                    saga.run_mode.as_str()
                ),
            })
        }
    }

    /// Returns the saga terminal outcome supported by the current prefix.
    pub fn saga_terminal_completion_outcome(
        &self,
        run_id: &RunId,
        policy: &SagaPolicySpec,
        terminal_policies: &SideEffectTerminalPolicies,
    ) -> Result<events::RunCompletionOutcome> {
        let saga = self.derive_saga_projection(run_id, policy, terminal_policies)?;
        saga.run_mode
            .saga_terminal_outcome()
            .ok_or_else(|| StoreError::ProjectionConflict {
                key: format!("run:{run_id}:saga_terminal"),
                message: format!(
                    "saga terminal resolution requires terminal saga mode, found {}",
                    saga.run_mode.as_str()
                ),
            })
    }

    /// Iterates projected run states.
    pub fn run_states(&self) -> impl Iterator<Item = (&RunId, &RunState)> {
        self.run_states.iter()
    }

    /// Iterates run-start certified spec hashes.
    pub fn run_spec_hashes(&self) -> impl Iterator<Item = (&RunId, &SpecHash)> {
        self.run_spec_hashes.iter()
    }

    /// Iterates run-start saga policy digests.
    pub fn saga_policy_digests(&self) -> impl Iterator<Item = (&RunId, &ContentDigest)> {
        self.saga_policy_digests.iter()
    }

    /// Iterates run completion projections.
    pub fn run_completions(&self) -> impl Iterator<Item = (&RunId, &RunCompletionProjection)> {
        self.run_completions.iter()
    }

    /// Iterates saga engagement projections.
    pub fn saga_engagements(&self) -> impl Iterator<Item = (&RunId, &SagaEngagementProjection)> {
        self.saga_engagements.iter()
    }

    /// Iterates manual resolution projections.
    pub fn manual_resolutions(
        &self,
    ) -> impl Iterator<Item = (&RunId, &ManualResolutionProjection)> {
        self.manual_resolutions.iter()
    }

    /// Iterates attempt lifecycle projections.
    pub fn attempts(&self) -> impl Iterator<Item = (&(NodeId, AttemptId), &AttemptProjection)> {
        self.attempts.iter()
    }

    /// Iterates cell terminal projections.
    pub fn cells(&self) -> impl Iterator<Item = (&RunId, &CellId, &CellTerminalProjection)> {
        self.cells
            .iter()
            .map(|((run_id, cell_id), projection)| (run_id, cell_id, projection))
    }

    /// Iterates descriptor catalog projections.
    pub fn fact_descriptors(
        &self,
    ) -> impl Iterator<Item = (&ContentDigest, &FactDescriptorProjection)> {
        self.fact_descriptors.iter()
    }

    /// Iterates recorded fact projections.
    pub fn fact_records(
        &self,
    ) -> impl Iterator<Item = (&mfm_facts::FactClaimId, &FactRecordProjection)> {
        self.fact_records.iter()
    }

    /// Iterates indexed fact projections.
    pub fn fact_index_entries(
        &self,
    ) -> impl Iterator<Item = (&mfm_facts::FactClaimId, &FactIndexProjection)> {
        self.fact_index_entries.iter()
    }

    /// Iterates extracted fact term projections.
    pub fn fact_term_entries(
        &self,
    ) -> impl Iterator<
        Item = (
            &(mfm_facts::FactClaimId, mfm_facts::FactFieldId),
            &FactIndexTermProjection,
        ),
    > {
        self.fact_term_entries.iter()
    }

    /// Iterates side-effect projections.
    pub fn side_effects(
        &self,
    ) -> impl Iterator<Item = (&SideEffectPairLedgerRef, &SideEffectProjection)> {
        self.side_effects.iter()
    }

    /// Iterates active resource lane projections.
    pub fn resource_lanes(
        &self,
    ) -> impl Iterator<Item = (&ResourceLaneKey, &ResourceLaneProjection)> {
        self.resource_lanes.iter()
    }

    /// Iterates public-output projections.
    pub fn public_outputs(
        &self,
    ) -> impl Iterator<Item = (&RunId, &SchemaId, &PublicOutputProjection)> {
        self.public_outputs
            .iter()
            .map(|((run_id, schema_id), projection)| (run_id, schema_id, projection))
    }

    /// Iterates retention projections.
    pub fn retentions(&self) -> impl Iterator<Item = (&RunId, &RetentionProjection)> {
        self.retentions.iter()
    }
}

pub(super) fn fact_claim_projection_key(prefix: &str, claim_id: &mfm_facts::FactClaimId) -> String {
    format!(
        "{}:{}:{}:{}",
        prefix,
        claim_id.source_run_id(),
        claim_id.source_seq(),
        claim_id.source_ordinal()
    )
}

pub(super) fn apply_projection(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    match envelope.payload() {
        KernelEventPayload::RunAdmitted(payload) => apply_run_admitted(
            projections,
            envelope.run_id(),
            &envelope.event_id,
            payload,
            artifact_bytes,
        )?,
        KernelEventPayload::RunCompleted(payload) => {
            apply_run_completed(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::StateAttemptStarted(payload) => {
            apply_attempt_started(projections, envelope.run_id(), &envelope.event_id, payload)?
        }
        KernelEventPayload::StateAttemptCompleted(payload) => {
            apply_attempt_completed(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            apply_attempt_interrupted(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            apply_attempt_failed(projections, envelope, payload)?;
        }
        KernelEventPayload::CellProduced(payload) => {
            apply_cell_produced(projections, envelope.run_id(), &envelope.event_id, payload)?;
        }
        KernelEventPayload::CellSkipped(payload) => {
            apply_cell_skipped(projections, envelope.run_id(), &envelope.event_id, payload)?;
        }
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            apply_side_effect_intent_persisted(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectClaimed(payload) => {
            apply_side_effect_claimed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            apply_side_effect_claim_taken_over(projections, envelope, payload)?;
        }
        KernelEventPayload::ResourceLaneClaimed(payload) => {
            apply_resource_lane_claimed(projections, envelope, payload)?;
        }
        KernelEventPayload::ResourceLaneClaimIntent(_)
        | KernelEventPayload::ResourceLaneReleaseIntent(_) => {
            return Err(StoreError::ProjectionConflict {
                key: "resource_lane:intent".to_owned(),
                message: "resource-lane intents must be store-filled before projection".to_owned(),
            });
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            apply_side_effect_invocation_prepared(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectInvocationStarted(payload) => {
            apply_side_effect_invocation_started(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            apply_side_effect_not_submitted_proven(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            apply_side_effect_submission_observed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            apply_side_effect_submission_unknown(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectReceiptObserved(payload) => {
            apply_side_effect_receipt_observed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            apply_side_effect_confirmation_observed(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectAmbiguous(payload) => {
            apply_side_effect_ambiguous(projections, envelope, payload)?;
        }
        KernelEventPayload::SideEffectFailed(payload) => {
            apply_side_effect_failed(projections, envelope, payload)?;
        }
        KernelEventPayload::ResourceLaneReleased(payload) => {
            apply_resource_lane_released(projections, envelope, payload)?;
        }
        KernelEventPayload::PublicOutputProduced(payload) => {
            apply_public_output_produced(
                projections,
                envelope.run_id(),
                &envelope.event_id,
                payload,
            )?;
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            apply_public_output_render_failed(
                projections,
                envelope.run_id(),
                &envelope.event_id,
                payload,
            )?;
        }
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            apply_manual_resolution_recorded(projections, &envelope.event_id, payload)?;
        }
        KernelEventPayload::RetentionRefsAppended(payload) => {
            apply_retention_refs_appended(projections, payload)?;
        }
        KernelEventPayload::RetentionManifestProjected(payload) => {
            apply_retention_manifest_projected(projections, payload)?;
        }
        KernelEventPayload::FactRecorded(payload) => {
            apply_fact_recorded(projections, envelope, payload, artifact_bytes)?;
        }
        KernelEventPayload::ArtifactReferenced(_) => {}
    }
    Ok(())
}

pub(super) fn apply_projection_for_external_fact_indexes(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
) -> Result<()> {
    match envelope.payload() {
        KernelEventPayload::RunAdmitted(payload) => {
            apply_run_admitted_base(projections, envelope.run_id(), &envelope.event_id, payload)?;
        }
        KernelEventPayload::FactRecorded(payload) => {
            apply_fact_recorded_record_only(projections, envelope, payload)?;
        }
        _ => apply_projection(projections, envelope, &ArtifactByteAuthorityMap::new())?,
    }
    Ok(())
}

fn apply_run_admitted(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::RunAdmitted,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    apply_run_admitted_base(projections, run_id, event_id, payload)?;
    for artifact in &payload.fact_descriptor_artifacts {
        apply_fact_descriptor_artifact(projections, artifact, artifact_bytes)?;
    }
    Ok(())
}

fn apply_run_admitted_base(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    _event_id: &EventId,
    payload: &events::RunAdmitted,
) -> Result<()> {
    validate_run_admitted_identity(run_id, payload)?;
    let state = projections.run_state(&payload.run_id);
    if state != RunState::Absent {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "run already admitted".to_owned(),
        });
    }
    projections
        .run_states
        .insert(payload.run_id.clone(), RunState::Started);
    projections
        .run_spec_hashes
        .insert(payload.run_id.clone(), payload.spec_hash.clone());
    projections
        .saga_policy_digests
        .insert(payload.run_id.clone(), payload.saga_policy_digest.clone());
    let retention = projections
        .retentions
        .entry(payload.run_id.clone())
        .or_default();
    insert_run_admission_retention(retention, &payload.spec_artifact)?;
    insert_run_admission_retention(retention, &payload.certificate_artifact)?;
    for artifact in &payload.config_artifacts {
        insert_run_admission_retention(retention, artifact)?;
    }
    for seed in &payload.seed_cells {
        let evidence = ArtifactEvidenceRef {
            artifact_id: seed.seed_artifact.artifact_id.clone(),
            digest: seed.seed_artifact.content_digest.clone(),
            byte_len: seed.seed_artifact.byte_len,
            media_type: seed.seed_artifact.media_type.clone(),
            schema_id: Some(seed.seed_artifact.schema_id.clone()),
            semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
            producer_node_id: None,
            producer_seed_id: Some(seed.seed_id.clone()),
            artifact_role: seed.seed_artifact.role,
        };
        insert_retention_evidence(retention, evidence)?;
    }
    Ok(())
}

fn validate_run_admitted_identity(run_id: &RunId, payload: &events::RunAdmitted) -> Result<()> {
    if payload.run_id != *run_id {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "RunAdmitted run id does not match stream run id".to_owned(),
        });
    }
    if payload.identity_material.certified_spec_hash != payload.spec_hash {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "RunAdmitted identity material spec hash does not match event spec hash"
                .to_owned(),
        });
    }
    let derived =
        payload
            .identity_material
            .derive_run_id()
            .map_err(|_| StoreError::ProjectionConflict {
                key: "run:admission".to_owned(),
                message: "RunAdmitted identity material is invalid".to_owned(),
            })?;
    if derived != payload.run_id {
        return Err(StoreError::ProjectionConflict {
            key: "run:admission".to_owned(),
            message: "RunAdmitted run id does not match identity material".to_owned(),
        });
    }
    Ok(())
}

fn insert_run_admission_retention(
    retention: &mut RetentionProjection,
    artifact: &events::RunArtifactEvidenceRef,
) -> Result<()> {
    insert_retention_evidence(retention, ArtifactEvidenceRef::from_run_artifact(artifact))
}

fn insert_retention_evidence(
    retention: &mut RetentionProjection,
    evidence: ArtifactEvidenceRef,
) -> Result<()> {
    let retention_ref = evidence.retention_ref()?;
    let key = (
        retention_ref.artifact_id.clone(),
        retention_ref.evidence_hash.clone(),
    );
    retention.refs.insert(key, retention_ref);
    Ok(())
}

fn apply_run_completed(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::RunCompleted,
) -> Result<()> {
    let state = projections.run_state(&payload.run_id);
    if state != RunState::Started {
        return Err(StoreError::ProjectionConflict {
            key: "run:complete".to_owned(),
            message: "run must be started and not completed".to_owned(),
        });
    }
    require_no_resource_lanes_for_run(projections, &payload.run_id, "run completion")?;
    projections
        .run_states
        .insert(payload.run_id.clone(), RunState::Completed);
    projections.run_completions.insert(
        payload.run_id.clone(),
        RunCompletionProjection {
            event_id: event_id.clone(),
            outcome: payload.outcome.clone(),
        },
    );
    Ok(())
}

fn apply_attempt_started(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::StateAttemptStarted,
) -> Result<()> {
    let key = (payload.node_id.clone(), payload.attempt_id.clone());
    if projections.attempts.contains_key(&key) {
        return Err(StoreError::ProjectionConflict {
            key: format!("attempt:{}:{}", payload.node_id, payload.attempt_id),
            message: "attempt already started".to_owned(),
        });
    }
    projections.attempts.insert(
        key,
        AttemptProjection {
            run_id: run_id.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            event_id: event_id.clone(),
            status: AttemptStatus::Started {
                attempt_no: payload.attempt_no,
                state_kind: payload.state_kind.clone(),
                state_version: payload.state_version.clone(),
            },
        },
    );
    Ok(())
}

fn apply_attempt_completed(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::StateAttemptCompleted,
) -> Result<()> {
    update_attempt_terminal_projection(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        event_id.clone(),
        AttemptStatus::Completed {
            output_cell_id: payload.output_cell_id.clone(),
        },
    )
}

fn apply_attempt_interrupted(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::StateAttemptInterrupted,
) -> Result<()> {
    require_no_prepared_side_effect_authority_for_interruption(projections, payload)?;
    update_attempt_terminal_projection(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        event_id.clone(),
        AttemptStatus::Interrupted,
    )
}

fn apply_attempt_failed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::StateAttemptFailed,
) -> Result<()> {
    update_attempt_terminal_projection(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        envelope.event_id.clone(),
        AttemptStatus::Failed {
            retryable: payload.retryable,
            error: Box::new(payload.error.clone()),
        },
    )?;
    if !payload.retryable {
        note_saga_engagement(
            projections,
            envelope.run_id(),
            SagaEngagementProjection {
                event_id: envelope.event_id.clone(),
                reason: SagaEngagementReason::NonRetryableFailure {
                    node_id: payload.node_id.clone(),
                    attempt_id: payload.attempt_id.clone(),
                },
            },
        );
    }
    Ok(())
}

fn require_no_prepared_side_effect_authority_for_interruption(
    projections: &ProjectionSnapshot,
    payload: &events::StateAttemptInterrupted,
) -> Result<()> {
    for side_effect in projections.side_effects.values() {
        if side_effect.intent.node_id != payload.node_id
            || side_effect.intent.attempt_id != payload.attempt_id
        {
            continue;
        }
        let ledger_state = side_effect.ledger_state()?;
        if side_effect_phase_blocks_standalone_interruption(ledger_state.phase()) {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{}:{}", payload.node_id, payload.attempt_id),
                message: "interruption is not legal after side-effect invocation was prepared"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

fn side_effect_phase_blocks_standalone_interruption(phase: SideEffectLedgerPhase<'_>) -> bool {
    !matches!(
        phase,
        SideEffectLedgerPhase::IntentPersisted { .. } | SideEffectLedgerPhase::Claimed { .. }
    )
}

fn apply_side_effect_intent_persisted(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::IntentPersisted,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let ledger_ref =
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone());
    if projections.side_effects.contains_key(&ledger_ref) {
        return Err(side_effect_projection_error(
            &payload.ledger_key,
            "intent already persisted",
        ));
    }
    let intent = SideEffectIntentProjection {
        node_id: payload.node_id.clone(),
        attempt_id: payload.attempt_id.clone(),
        scope_id: payload.scope_id.clone(),
        invocation_epoch: payload.invocation_epoch,
        intent_schema_id: payload.intent_schema_id.clone(),
        intent_hash: payload.intent_hash.clone(),
        intent_artifact_id: payload.intent_artifact_id.clone(),
        idempotency_input_schema_id: payload.idempotency_input_schema_id.clone(),
        idempotency_input_hash: payload.idempotency_input_hash.clone(),
        idempotency_key: payload.idempotency_key.clone(),
        capability_kind: payload.capability_kind.clone(),
        capability_version: payload.capability_version.clone(),
        adapter_kind: payload.adapter_kind.clone(),
        adapter_version: payload.adapter_version.clone(),
    };
    projections.side_effects.insert(
        ledger_ref,
        OwnedSideEffectLedgerState::intent_persisted(
            envelope.run_id().clone(),
            payload.ledger_key.clone(),
            payload.ledger_purpose.clone(),
            payload.pair_id.clone(),
            envelope.event_id.clone(),
            intent,
            payload.invocation_epoch,
        )
        .into_projection(),
    );
    Ok(())
}

fn apply_side_effect_claimed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::Claimed,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "intent or not-submitted",
        |state| {
            matches!(
                state.phase(),
                SideEffectLedgerPhase::IntentPersisted { .. }
                    | SideEffectLedgerPhase::SubmissionKnown {
                        status: SideEffectSubmissionState::NotSubmitted,
                        ..
                    }
            )
        },
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .claim(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

fn apply_side_effect_claim_taken_over(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::ClaimTakenOver,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "claim or prepared",
        |state| {
            matches!(
                state.phase(),
                SideEffectLedgerPhase::Claimed { .. } | SideEffectLedgerPhase::Prepared { .. }
            )
        },
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .take_over(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

fn apply_side_effect_invocation_prepared(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::InvocationPrepared,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "claim",
        |state| matches!(state.phase(), SideEffectLedgerPhase::Claimed { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let prepared_invocation = prepared_invocation_projection(
        &payload.prepared_artifact_id,
        &payload.prepared_hash,
        &payload.prepared_artifact_evidence_hash,
        &payload.ledger_key,
    )?
    .or_else(|| previous.prepared_invocation.clone());
    let resource_key = match (&previous.resource_key, &payload.resource_key) {
        (Some(held), Some(prepared)) if held == prepared => Some(held.clone()),
        (Some(_), Some(_)) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "prepared invocation resource key must match the held resource lane"
                    .to_owned(),
            });
        }
        (Some(_), None) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "prepared invocation must echo the held resource lane key".to_owned(),
            });
        }
        (None, Some(_)) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "resource lane must be claimed before invocation prepare".to_owned(),
            });
        }
        (None, None) => None,
    };
    if let Some(resource_key) = &resource_key {
        let lane_key = ResourceLaneKey::from_evidence(resource_key);
        let holder =
            SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone());
        if projections
            .resource_lane(&lane_key)
            .is_none_or(|projection| projection.holder != holder)
        {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "resource lane must be claimed before invocation prepare".to_owned(),
            });
        }
    }
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .prepare(
            envelope.event_id.clone(),
            payload,
            prepared_invocation,
            resource_key,
        )?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

fn apply_resource_lane_claimed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::ResourceLaneClaimed,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "claim",
        |state| matches!(state.phase(), SideEffectLedgerPhase::Claimed { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let mut projection = previous.clone();
    if let Some(existing) = &projection.resource_key {
        if existing != &payload.resource_key {
            return Err(StoreError::ProjectionConflict {
                key: format!("sidefx:{}", payload.ledger_key),
                message: "resource lane claim changed held resource key".to_owned(),
            });
        }
    }
    acquire_resource_lane(projections, envelope.run_id(), &envelope.event_id, payload)?;
    projection.event_id = envelope.event_id.clone();
    projection.resource_key = Some(payload.resource_key.clone());
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

fn apply_resource_lane_released(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::ResourceLaneReleased,
) -> Result<()> {
    release_resource_lane(projections, envelope.run_id(), payload)
}

fn apply_side_effect_invocation_started(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::InvocationStarted,
) -> Result<()> {
    require_side_effect_pair_role(
        &payload.ledger_key,
        &payload.ledger_purpose,
        payload.pair_role,
        events::SideEffectPairRole::Submit,
    )?;
    require_forward_fence_open(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.ledger_purpose,
        &payload.pair_id,
        payload.pair_role,
    )?;
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    let previous = require_side_effect_phase(
        projections,
        envelope.run_id(),
        &payload.ledger_key,
        &payload.pair_id,
        "prepared",
        |state| matches!(state.phase(), SideEffectLedgerPhase::Prepared { .. }),
    )?;
    require_side_effect_purpose(previous, &payload.ledger_key, &payload.ledger_purpose)?;
    require_side_effect_pair_consistent(previous, &payload.ledger_key, &payload.pair_id)?;
    let projection = OwnedSideEffectLedgerState::from_projection(previous.clone())?
        .start(envelope.event_id.clone(), payload)?
        .into_projection();
    projections.side_effects.insert(
        SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        projection,
    );
    Ok(())
}

fn apply_side_effect_not_submitted_proven(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::NotSubmittedProven,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::SubmitOrVerify,
            required_previous: "submission_recovery",
        },
        |state| state.mark_not_submitted(envelope.event_id.clone(), payload),
    )?;
    require_no_resource_lane_for_holder(
        projections,
        &SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        "not-submitted proof",
    )?;
    Ok(())
}

fn apply_side_effect_submission_observed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::SubmissionObserved,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::SubmitOrVerify,
            required_previous: "submission_recovery",
        },
        |state| state.record_submission(envelope.event_id.clone(), payload),
    )?;
    Ok(())
}

fn apply_side_effect_submission_unknown(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::SubmissionUnknown,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Submit),
            required_previous: "submission_recovery",
        },
        |state| state.mark_submission_unknown(envelope.event_id.clone(), payload),
    )?;
    Ok(())
}

fn apply_side_effect_receipt_observed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::ReceiptObserved,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Verify),
            required_previous: "submission_observed",
        },
        |state| state.record_receipt(envelope.event_id.clone(), payload),
    )?;
    Ok(())
}

fn apply_side_effect_confirmation_observed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::ConfirmationObserved,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::Exact(events::SideEffectPairRole::Verify),
            required_previous: "receipt",
        },
        |state| state.confirm(envelope.event_id.clone(), payload),
    )?;
    require_no_resource_lane_for_holder(
        projections,
        &SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        "confirmation",
    )?;
    Ok(())
}

fn apply_side_effect_ambiguous(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::Ambiguous,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_epoch_only(
        projections,
        EpochOnlyTransition {
            run_id: envelope.run_id(),
            ledger_key: &payload.ledger_key,
            ledger_purpose: &payload.ledger_purpose,
            pair_id: &payload.pair_id,
            pair_role: payload.pair_role,
            expected_pair_role: PairRoleRequirement::SubmitOrVerify,
            required_previous: "ambiguity_source",
        },
        |state| state.mark_ambiguous(envelope.event_id.clone(), payload),
    )?;
    if matches!(
        payload.ledger_purpose,
        events::SideEffectLedgerPurpose::Forward
    ) {
        note_saga_engagement(
            projections,
            envelope.run_id(),
            SagaEngagementProjection {
                event_id: envelope.event_id.clone(),
                reason: SagaEngagementReason::ForwardAmbiguous {
                    pair_id: payload.pair_id.clone(),
                },
            },
        );
    }
    Ok(())
}

fn apply_side_effect_failed(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &side_effect::Failed,
) -> Result<()> {
    require_active_attempt_for_side_effect(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        &payload.ledger_key,
    )?;
    transition_side_effect_failure(
        projections,
        envelope.run_id(),
        payload,
        envelope.event_id.clone(),
    )?;
    require_no_resource_lane_for_holder(
        projections,
        &SideEffectPairLedgerRef::new(envelope.run_id().clone(), payload.pair_id.clone()),
        "side-effect failure",
    )?;
    if !payload.retryable {
        note_saga_engagement(
            projections,
            envelope.run_id(),
            SagaEngagementProjection {
                event_id: envelope.event_id.clone(),
                reason: SagaEngagementReason::NonRetryableFailure {
                    node_id: payload.node_id.clone(),
                    attempt_id: payload.attempt_id.clone(),
                },
            },
        );
    }
    Ok(())
}

fn apply_cell_produced(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::CellProduced,
) -> Result<()> {
    let key = (run_id.clone(), payload.cell_id.clone());
    if projections.cells.contains_key(&key) {
        return Err(StoreError::ProjectionConflict {
            key: format!("cell:{run_id}:{}:terminal", payload.cell_id),
            message: "cell already terminal".to_owned(),
        });
    }
    projections.cells.insert(
        key,
        CellTerminalProjection::Produced {
            event_id: event_id.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            schema_id: payload.schema_id.clone(),
            semantic_type_id: payload.semantic_type_id.clone(),
            artifact_id: payload.artifact_id.clone(),
            content_digest: payload.content_digest.clone(),
            evidence_hash: payload.evidence_hash.clone(),
        },
    );
    Ok(())
}

fn apply_cell_skipped(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::CellSkipped,
) -> Result<()> {
    let key = (run_id.clone(), payload.cell_id.clone());
    if projections.cells.contains_key(&key) {
        return Err(StoreError::ProjectionConflict {
            key: format!("cell:{run_id}:{}:terminal", payload.cell_id),
            message: "cell already terminal".to_owned(),
        });
    }
    projections.cells.insert(
        key,
        CellTerminalProjection::Skipped {
            event_id: event_id.clone(),
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            schema_id: payload.schema_id.clone(),
            semantic_type_id: payload.semantic_type_id.clone(),
            skip_reason: payload.skip_reason.clone(),
        },
    );
    Ok(())
}

fn apply_public_output_produced(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    let key = (run_id.clone(), payload.public_schema_id.clone());
    if matches!(
        projections.public_outputs.get(&key),
        Some(PublicOutputProjection::Produced { .. })
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("public_output:{run_id}:{}", payload.public_schema_id),
            message: "public output already projected".to_owned(),
        });
    }
    projections.public_outputs.insert(
        key,
        PublicOutputProjection::Produced {
            event_id: event_id.clone(),
            rendered_digest: payload.rendered_digest.clone(),
            rendered_artifact_id: payload.rendered_artifact_id.clone(),
        },
    );
    Ok(())
}

fn apply_public_output_render_failed(
    projections: &mut ProjectionSnapshot,
    run_id: &RunId,
    event_id: &EventId,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    let key = (run_id.clone(), payload.public_schema_id.clone());
    if matches!(
        projections.public_outputs.get(&key),
        Some(PublicOutputProjection::Produced { .. })
    ) {
        return Err(StoreError::ProjectionConflict {
            key: format!("public_output:{run_id}:{}", payload.public_schema_id),
            message: "public output already produced".to_owned(),
        });
    }
    projections.public_outputs.insert(
        key,
        PublicOutputProjection::RenderFailed {
            event_id: event_id.clone(),
            error: Box::new(payload.error.clone()),
        },
    );
    Ok(())
}

fn apply_manual_resolution_recorded(
    projections: &mut ProjectionSnapshot,
    event_id: &EventId,
    payload: &events::ManualResolutionRecorded,
) -> Result<()> {
    if projections.manual_resolutions.contains_key(&payload.run_id) {
        return Err(StoreError::ProjectionConflict {
            key: "run:manual_resolution".to_owned(),
            message: "manual resolution already recorded".to_owned(),
        });
    }
    projections.require_no_open_semantic_attempts_for_run(&payload.run_id)?;
    require_no_resource_lanes_for_run(projections, &payload.run_id, "manual resolution")?;
    projections.manual_resolutions.insert(
        payload.run_id.clone(),
        ManualResolutionProjection {
            event_id: event_id.clone(),
            outcome: payload.outcome,
            evidence_schema_id: payload.evidence_schema_id.clone(),
            evidence_hash: payload.evidence_hash.clone(),
            evidence_artifact_id: payload.evidence_artifact_id.clone(),
            authorization_schema_id: payload.authorization_schema_id.clone(),
            authorization_hash: payload.authorization_hash.clone(),
            authorization_artifact_id: payload.authorization_artifact_id.clone(),
            note: payload.note.clone(),
        },
    );
    Ok(())
}

fn apply_retention_refs_appended(
    projections: &mut ProjectionSnapshot,
    payload: &events::RetentionRefsAppended,
) -> Result<()> {
    if projections.run_state(&payload.run_id) == RunState::Absent {
        return Err(StoreError::ProjectionConflict {
            key: format!("retention:{}:refs", payload.run_id),
            message: "retention refs require a started run".to_owned(),
        });
    }
    let retention = projections
        .retentions
        .entry(payload.run_id.clone())
        .or_default();
    for retention_ref in &payload.refs {
        retention.refs.insert(
            (
                retention_ref.artifact_id.clone(),
                retention_ref.evidence_hash.clone(),
            ),
            retention_ref.clone(),
        );
    }
    Ok(())
}

fn apply_retention_manifest_projected(
    projections: &mut ProjectionSnapshot,
    payload: &events::RetentionManifestProjected,
) -> Result<()> {
    if projections.run_state(&payload.run_id) == RunState::Absent {
        return Err(StoreError::ProjectionConflict {
            key: format!("retention:{}:manifest", payload.run_id),
            message: "retention manifest requires a started run".to_owned(),
        });
    }
    let retention = projections
        .retentions
        .entry(payload.run_id.clone())
        .or_default();
    if retention.manifests.contains_key(&payload.manifest_seq) {
        return Err(StoreError::ProjectionConflict {
            key: format!(
                "retention:{}:manifest:{}",
                payload.run_id, payload.manifest_seq
            ),
            message: "manifest sequence already projected".to_owned(),
        });
    }
    match &retention.manifest {
        Some(previous) => {
            let expected_seq = previous.manifest_seq.checked_add(1).ok_or_else(|| {
                StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "manifest sequence overflow".to_owned(),
                }
            })?;
            if payload.manifest_seq != expected_seq {
                return Err(StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "manifest sequence must advance by one".to_owned(),
                });
            }
            if payload.previous_manifest_digest.as_ref() != Some(&previous.manifest_digest) {
                return Err(StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "manifest previous digest does not match latest projection".to_owned(),
                });
            }
        }
        None => {
            if payload.manifest_seq != 1 || payload.previous_manifest_digest.is_some() {
                return Err(StoreError::ProjectionConflict {
                    key: format!("retention:{}:manifest", payload.run_id),
                    message: "first manifest must use sequence 1 and no previous digest".to_owned(),
                });
            }
        }
    }
    let projection = RetentionManifestProjection {
        manifest_seq: payload.manifest_seq,
        manifest_digest: payload.manifest_digest.clone(),
        manifest_artifact_id: payload.manifest_artifact_id.clone(),
        previous_manifest_digest: payload.previous_manifest_digest.clone(),
    };
    retention
        .manifests
        .insert(payload.manifest_seq, projection.clone());
    retention.manifest = Some(projection);
    Ok(())
}

fn apply_fact_recorded(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::FactRecorded,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    let claim = &payload.claim;
    let mut record = FactRecordProjection::from_recorded_event(envelope, payload, None)?;
    let claim_id = record.fact_claim_id.clone();
    require_started_fact_attempt(projections, payload, &claim_id)?;

    let descriptor_projection = projections
        .fact_descriptor(claim.fact_descriptor_hash())
        .cloned()
        .ok_or_else(|| StoreError::ProjectionConflict {
            key: format!("fact_descriptor:{}", claim.fact_descriptor_hash()),
            message: "fact descriptor must be admitted before recording a fact".to_owned(),
        })?;
    let descriptor = load_projected_fact_descriptor(&descriptor_projection, artifact_bytes)?;
    validate_fact_claim_against_descriptor(claim, &descriptor_projection)?;
    let subject_material = mfm_facts::parse_canonical_fact_subject_material_bytes(
        claim.subject().subject_material().as_bytes(),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;

    let response = claim.response();
    let (response_bytes, response_evidence) = require_artifact_bytes_by_key(
        artifact_bytes,
        response.artifact_id(),
        response.artifact_evidence_hash(),
    )?;
    validate_fact_response_evidence(response, response_evidence)?;
    record.response_artifact_evidence = Some(response_evidence.clone());

    insert_fact_record_projection(projections, record.clone())?;
    if !matches!(
        claim.visibility(),
        mfm_facts::FactVisibility::Indexed { .. }
    ) {
        return Ok(());
    }

    let recorded_at = fact_recorded_at(envelope);
    let store_commit_order = envelope.store_commit_order().as_u64();
    let index = FactIndexProjection::from_record_projection(
        &record,
        envelope.commit_key().clone(),
        store_commit_order,
        recorded_at.clone(),
    )?
    .ok_or_else(|| StoreError::ProjectionConflict {
        key: fact_claim_projection_key("fact_index", &claim_id),
        message: "indexed fact record did not produce an index projection".to_owned(),
    })?;
    let metadata = mfm_facts::FactExtractionMetadata::new(
        recorded_at.clone(),
        claim.observed_at().map(str::to_owned),
        store_commit_order,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;
    let response_value =
        mfm_facts::parse_canonical_fact_response_bytes(&descriptor, response_bytes)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
    let terms = mfm_facts::extract_terms_from_material(
        &descriptor,
        &subject_material,
        &response_value,
        &metadata,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;

    projections
        .fact_index_entries
        .insert(claim_id.clone(), index);
    for term in terms {
        let key = (claim_id.clone(), term.field_id().clone());
        if projections
            .fact_term_entries
            .insert(
                key.clone(),
                FactIndexTermProjection::from_extracted_term(
                    &claim_id,
                    claim.fact_descriptor_hash(),
                    &term,
                ),
            )
            .is_some()
        {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "{}:{}",
                    fact_claim_projection_key("fact_term", &claim_id),
                    key.1
                ),
                message: "duplicate fact term projection".to_owned(),
            });
        }
    }
    Ok(())
}

fn apply_fact_recorded_record_only(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::FactRecorded,
) -> Result<()> {
    let record = FactRecordProjection::from_recorded_event(envelope, payload, None)?;
    require_started_fact_attempt(projections, payload, &record.fact_claim_id)?;
    insert_fact_record_projection(projections, record)
}

fn require_started_fact_attempt(
    projections: &ProjectionSnapshot,
    payload: &events::FactRecorded,
    claim_id: &mfm_facts::FactClaimId,
) -> Result<()> {
    match projections.attempt(&payload.node_id, &payload.attempt_id) {
        Some(AttemptProjection {
            status: AttemptStatus::Started { .. },
            ..
        }) => Ok(()),
        Some(_) => Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact", claim_id),
            message: "fact requires an active started attempt".to_owned(),
        }),
        None => Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact", claim_id),
            message: "fact requires a started attempt".to_owned(),
        }),
    }
}

fn insert_fact_record_projection(
    projections: &mut ProjectionSnapshot,
    record: FactRecordProjection,
) -> Result<()> {
    let claim = &record.claim;
    let claim_id = &record.fact_claim_id;
    if projections.fact_records.values().any(|record| {
        record.claim.response().artifact_id() == claim.response().artifact_id()
            && record.claim.response().artifact_evidence_hash()
                == claim.response().artifact_evidence_hash()
    }) {
        return Err(StoreError::ProjectionConflict {
            key: format!(
                "fact_response:{}:{}",
                claim.response().artifact_id(),
                claim.response().artifact_evidence_hash()
            ),
            message: "response artifact is already bound to a fact claim".to_owned(),
        });
    }
    if projections.fact_records.contains_key(claim_id) {
        return Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact_record", claim_id),
            message: "fact claim id is already projected".to_owned(),
        });
    }
    projections
        .fact_records
        .insert(record.fact_claim_id.clone(), record);
    Ok(())
}

fn apply_fact_descriptor_artifact(
    projections: &mut ProjectionSnapshot,
    artifact: &events::RunArtifactEvidenceRef,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let evidence = ArtifactEvidenceRef::from_run_artifact(artifact);
    if evidence.artifact_role != ArtifactRole::FactDescriptor
        || evidence.schema_id.as_ref() != Some(&expected_schema)
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "fact_descriptor",
        });
    }
    let bytes = require_artifact_bytes_exact(artifact_bytes, &evidence)?;
    let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    if descriptor_hash != evidence.digest {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "digest",
        });
    }
    let namespace_hash = mfm_facts::fact_subject_namespace_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let projection = FactDescriptorProjection {
        descriptor_hash: descriptor_hash.clone(),
        descriptor_artifact_id: artifact.artifact_id.clone(),
        descriptor_artifact_evidence: evidence.clone(),
        fact_kind: descriptor.fact_kind().clone(),
        descriptor_schema_id: descriptor.descriptor_schema_id().clone(),
        subject_schema_id: descriptor.subject_schema_id().clone(),
        response_schema_id: descriptor.response_schema_id().clone(),
        fact_subject_namespace_hash: namespace_hash,
    };
    match projections
        .fact_descriptors
        .insert(descriptor_hash.clone(), projection.clone())
    {
        Some(existing) if equivalent_fact_descriptor_projection(&existing, &projection) => {
            projections
                .fact_descriptors
                .insert(descriptor_hash, existing);
        }
        Some(_) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("fact_descriptor:{descriptor_hash}"),
                message: "conflicting fact descriptor projection".to_owned(),
            });
        }
        None => {}
    }
    Ok(())
}

fn equivalent_fact_descriptor_projection(
    left: &FactDescriptorProjection,
    right: &FactDescriptorProjection,
) -> bool {
    left.descriptor_hash == right.descriptor_hash
        && left.descriptor_artifact_id == right.descriptor_artifact_id
        && left.descriptor_artifact_evidence == right.descriptor_artifact_evidence
        && left.fact_kind == right.fact_kind
        && left.descriptor_schema_id == right.descriptor_schema_id
        && left.subject_schema_id == right.subject_schema_id
        && left.response_schema_id == right.response_schema_id
        && left.fact_subject_namespace_hash == right.fact_subject_namespace_hash
}

fn load_projected_fact_descriptor(
    projection: &FactDescriptorProjection,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<mfm_facts::FactDescriptor> {
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    for ((artifact_id, _), (bytes, evidence)) in artifact_bytes {
        if artifact_id != &projection.descriptor_artifact_id {
            continue;
        }
        if evidence.digest != projection.descriptor_hash {
            continue;
        }
        if evidence.artifact_role != ArtifactRole::FactDescriptor
            || evidence.schema_id.as_ref() != Some(&expected_schema)
        {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "fact_descriptor",
            });
        }
        let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        if descriptor_hash != projection.descriptor_hash {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "digest",
            });
        }
        return Ok(descriptor);
    }
    Err(StoreError::MissingArtifact {
        artifact_id: projection.descriptor_artifact_id.clone(),
    })
}

fn validate_fact_claim_against_descriptor(
    claim: &mfm_facts::FactClaim,
    descriptor: &FactDescriptorProjection,
) -> Result<()> {
    if claim.fact_descriptor_hash() != &descriptor.descriptor_hash
        || claim.fact_kind() != &descriptor.fact_kind
        || claim.subject().fact_subject_namespace_hash() != &descriptor.fact_subject_namespace_hash
        || claim.response().response_schema_id() != &descriptor.response_schema_id
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("fact_descriptor:{}", claim.fact_descriptor_hash()),
            message: "fact claim does not match admitted descriptor".to_owned(),
        });
    }
    Ok(())
}

fn validate_fact_response_evidence(
    response: &mfm_facts::FactResponseEvidence,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if evidence.artifact_role != ArtifactRole::FactResponse
        || evidence.schema_id.as_ref() != Some(response.response_schema_id())
        || &evidence.digest != response.response_hash()
        || &evidence.artifact_id != response.artifact_id()
        || evidence.evidence_hash()? != *response.artifact_evidence_hash()
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: response.artifact_id().clone(),
            field: "fact_response",
        });
    }
    Ok(())
}

fn require_artifact_bytes_exact<'a>(
    artifact_bytes: &'a ArtifactByteAuthorityMap,
    evidence: &ArtifactEvidenceRef,
) -> Result<&'a [u8]> {
    let evidence_hash = evidence.evidence_hash()?;
    let Some((bytes, stored_evidence)) =
        artifact_bytes.get(&(evidence.artifact_id.clone(), evidence_hash))
    else {
        return Err(StoreError::MissingArtifact {
            artifact_id: evidence.artifact_id.clone(),
        });
    };
    if stored_evidence != evidence {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "artifact",
        });
    }
    super::verify_retained_artifact_bytes(bytes, evidence)?;
    Ok(bytes.as_slice())
}

fn require_artifact_bytes_by_key<'a>(
    artifact_bytes: &'a ArtifactByteAuthorityMap,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<(&'a [u8], &'a ArtifactEvidenceRef)> {
    let Some((bytes, evidence)) = artifact_bytes.get(&(artifact_id.clone(), evidence_hash.clone()))
    else {
        return Err(StoreError::MissingArtifact {
            artifact_id: artifact_id.clone(),
        });
    };
    super::verify_retained_artifact_bytes(bytes, evidence)?;
    Ok((bytes.as_slice(), evidence))
}

fn fact_recorded_at(_envelope: &KernelEventEnvelope) -> String {
    "1970-01-01T00:00:00Z".to_owned()
}

fn update_attempt_terminal_projection(
    projections: &mut ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    event_id: EventId,
    status: AttemptStatus,
) -> Result<()> {
    let key = (node_id.clone(), attempt_id.clone());
    let Some(projection) = projections.attempts.get_mut(&key) else {
        return Err(StoreError::ProjectionConflict {
            key: format!("attempt:{node_id}:{attempt_id}"),
            message: "attempt terminal event requires a started attempt".to_owned(),
        });
    };
    if !matches!(projection.status, AttemptStatus::Started { .. }) {
        return Err(StoreError::ProjectionConflict {
            key: format!("attempt:{node_id}:{attempt_id}"),
            message: "attempt already terminal".to_owned(),
        });
    }
    projection.event_id = event_id;
    projection.status = status;
    Ok(())
}
