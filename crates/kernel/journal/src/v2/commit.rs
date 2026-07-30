use mfm_canonical::CanonicalValue;
use mfm_ids::{
    AppendRequestId, GenesisDigest, JournalCandidateDigest, JournalCommitDigest, JournalRecordHash,
    NodeId, RecordId, RunId, SchemaId, StoreEpoch, StoreScopeId,
};

use super::codec::{
    append_request_id_field, array_field, bool_field, cv_array, cv_decimal, cv_string,
    define_schema_value, domain_digest, node_id_field, object, required_field, run_id_field,
    schema_id_field, store_epoch_field, store_scope_id_field, string_field, tagged_object,
    u32_field, unsigned_field,
};
use super::{
    ArtifactAdmissionIntent, AuthorizationRef, ExternalAccessAuthorized, ExternalAccessObserved,
    ObjectPathBinding, Result, RunAdmitted, RunClosed, RunJournalRecord, StateTransitionCommitted,
    TenantFactCoordinate, TransitionSlot,
};

define_schema_value! {
    /// Exact per-run journal head.
    pub struct JournalHead => "mfm.journal-head.v1";
    /// Exact predecessor, either genesis or an assigned journal head.
    pub struct JournalPredecessor => "mfm.journal-predecessor.v1";
    /// Closed logical key attached to one record candidate.
    pub struct RecordLogicalKey => "mfm.record-logical-key.v1";
    /// Unassigned candidate record envelope.
    pub struct CandidateRecordEnvelope => "mfm.candidate-record-envelope.v2";
    /// Complete unassigned commit candidate preimage.
    pub struct CommitCandidatePreimage => "mfm.commit-candidate-preimage.v2";
    /// Assigned record semantic-hash preimage.
    pub struct RecordHashPreimage => "mfm.record-hash-preimage.v2";
    /// Assigned record-identity preimage.
    pub struct RecordIdPreimage => "mfm.record-id-preimage.v1";
    /// Assigned commit semantic-digest preimage.
    pub struct CommitDigestPreimage => "mfm.commit-digest-preimage.v1";
    /// Complete assigned commit envelope.
    pub struct CommitEnvelope => "mfm.commit-envelope.v1";
    /// Closed structurally legal record batch.
    pub struct LegalCommitBatch => "mfm.legal-commit-batch.v2";
    /// Per-run genesis semantic-digest preimage.
    pub struct GenesisPreimage => "mfm.genesis-preimage.v1";
}

/// One exact frozen journal batch purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BatchPurpose {
    /// Sole run admission.
    RunAdmission,
    /// Successful or failed pure settlement.
    PureSettlement,
    /// Successful or failed read settlement.
    ReadSettlement,
    /// Dependency-caused skip.
    DependencySkip,
    /// External access authorization.
    ExternalAccessAuthorization,
    /// External access observation.
    ExternalAccessObservation,
    /// Durable effect request.
    EffectRequest,
    /// Durable effect settlement.
    EffectSettlement,
}

impl BatchPurpose {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunAdmission => "run_admission",
            Self::PureSettlement => "pure_settlement",
            Self::ReadSettlement => "read_settlement",
            Self::DependencySkip => "dependency_skip",
            Self::ExternalAccessAuthorization => "external_access_authorization",
            Self::ExternalAccessObservation => "external_access_observation",
            Self::EffectRequest => "effect_request",
            Self::EffectSettlement => "effect_settlement",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "run_admission" => Ok(Self::RunAdmission),
            "pure_settlement" => Ok(Self::PureSettlement),
            "read_settlement" => Ok(Self::ReadSettlement),
            "dependency_skip" => Ok(Self::DependencySkip),
            "external_access_authorization" => Ok(Self::ExternalAccessAuthorization),
            "external_access_observation" => Ok(Self::ExternalAccessObservation),
            "effect_request" => Ok(Self::EffectRequest),
            "effect_settlement" => Ok(Self::EffectSettlement),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of an exact journal head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalHeadFields {
    /// Per-run commit sequence.
    pub run_sequence: u64,
    /// Semantic digest of the complete containing commit.
    pub commit_digest: JournalCommitDigest,
}

impl JournalHead {
    /// Constructs and validates an exact per-run journal head.
    pub fn new(run_sequence: u64, commit_digest: &JournalCommitDigest) -> Result<Self> {
        Self::from_canonical_value(object([
            ("run_sequence", cv_decimal(run_sequence)?),
            ("commit_digest", cv_string(commit_digest.as_str())),
        ])?)
    }

    /// Projects the exact journal-head fields.
    pub fn fields(&self) -> Result<JournalHeadFields> {
        Ok(JournalHeadFields {
            run_sequence: unsigned_field(self, "run_sequence")?,
            commit_digest: JournalCommitDigest::parse(string_field(self, "commit_digest")?)?,
        })
    }
}

/// Closed predecessor fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalPredecessorFields {
    /// Genesis predecessor used only by the sole admission.
    Genesis {
        /// Store deployment identity.
        store_scope_id: StoreScopeId,
        /// Store epoch.
        store_epoch: StoreEpoch,
        /// Deterministically derived run identity.
        run_id: RunId,
        /// Frozen genesis digest.
        genesis_digest: GenesisDigest,
    },
    /// Exact previous per-run journal head.
    JournalHead(JournalHeadFields),
}

impl JournalPredecessor {
    /// Constructs the genesis predecessor.
    pub fn genesis(
        store_scope_id: &StoreScopeId,
        store_epoch: StoreEpoch,
        run_id: &RunId,
        genesis_digest: &GenesisDigest,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "genesis",
            [
                ("store_scope_id", cv_string(store_scope_id.as_str())),
                ("store_epoch", cv_decimal(store_epoch.get())?),
                ("run_id", cv_string(run_id.as_str())),
                ("genesis_digest", cv_string(genesis_digest.as_str())),
            ],
        )?)
    }

    /// Constructs an assigned-head predecessor.
    pub fn journal_head(head: &JournalHead) -> Result<Self> {
        let fields = head.fields()?;
        Self::from_canonical_value(tagged_object(
            "journal_head",
            [
                ("run_sequence", cv_decimal(fields.run_sequence)?),
                ("commit_digest", cv_string(fields.commit_digest.as_str())),
            ],
        )?)
    }

    /// Projects the closed predecessor variant.
    pub fn fields(&self) -> Result<JournalPredecessorFields> {
        match super::codec::tag(self)?.as_str() {
            "genesis" => Ok(JournalPredecessorFields::Genesis {
                store_scope_id: store_scope_id_field(self, "store_scope_id")?,
                store_epoch: store_epoch_field(self, "store_epoch")?,
                run_id: run_id_field(self, "run_id")?,
                genesis_digest: GenesisDigest::parse(string_field(self, "genesis_digest")?)?,
            }),
            "journal_head" => Ok(JournalPredecessorFields::JournalHead(JournalHeadFields {
                run_sequence: unsigned_field(self, "run_sequence")?,
                commit_digest: JournalCommitDigest::parse(string_field(self, "commit_digest")?)?,
            })),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Closed logical-key fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordLogicalKeyFields {
    /// Sole admission key.
    RunAdmission {
        /// Admitted run.
        run_id: RunId,
    },
    /// One node transition slot.
    Transition {
        /// Admitted run.
        run_id: RunId,
        /// Certified node occurrence.
        node_id: NodeId,
        /// Request or settlement slot.
        slot: TransitionSlot,
    },
    /// Authorization append identity.
    Authorization {
        /// Stable append request identifier.
        append_request_id: AppendRequestId,
    },
    /// One observation for one authorization.
    Observation {
        /// Exact authorization record.
        authorization_ref: AuthorizationRef,
    },
    /// Sole run closure.
    Closure {
        /// Closed run.
        run_id: RunId,
    },
}

impl RecordLogicalKey {
    /// Constructs the sole admission logical key.
    pub fn run_admission(run_id: &RunId) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "run_admission",
            [("run_id", cv_string(run_id.as_str()))],
        )?)
    }

    /// Constructs a transition logical key.
    pub fn transition(run_id: &RunId, node_id: &NodeId, slot: TransitionSlot) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "transition",
            [
                ("run_id", cv_string(run_id.as_str())),
                ("node_id", cv_string(node_id.as_str())),
                ("slot", cv_string(slot.as_str())),
            ],
        )?)
    }

    /// Constructs an authorization logical key.
    pub fn authorization(append_request_id: &AppendRequestId) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "authorization",
            [("append_request_id", cv_string(append_request_id.as_str()))],
        )?)
    }

    /// Constructs an observation logical key.
    pub fn observation(authorization_ref: &AuthorizationRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "observation",
            [("authorization_ref", authorization_ref.canonical_value()?)],
        )?)
    }

    /// Constructs the sole closure logical key.
    pub fn closure(run_id: &RunId) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "closure",
            [("run_id", cv_string(run_id.as_str()))],
        )?)
    }

    /// Projects the closed logical-key variant.
    pub fn fields(&self) -> Result<RecordLogicalKeyFields> {
        match super::codec::tag(self)?.as_str() {
            "run_admission" => Ok(RecordLogicalKeyFields::RunAdmission {
                run_id: run_id_field(self, "run_id")?,
            }),
            "transition" => Ok(RecordLogicalKeyFields::Transition {
                run_id: run_id_field(self, "run_id")?,
                node_id: node_id_field(self, "node_id")?,
                slot: TransitionSlot::parse(&string_field(self, "slot")?)?,
            }),
            "authorization" => Ok(RecordLogicalKeyFields::Authorization {
                append_request_id: append_request_id_field(self, "append_request_id")?,
            }),
            "observation" => Ok(RecordLogicalKeyFields::Observation {
                authorization_ref: AuthorizationRef::from_canonical_value(required_field(
                    self,
                    "authorization_ref",
                )?)?,
            }),
            "closure" => Ok(RecordLogicalKeyFields::Closure {
                run_id: run_id_field(self, "run_id")?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of an unassigned candidate record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRecordEnvelopeFields {
    /// Candidate ordinal.
    pub ordinal: u32,
    /// Exact selected record schema.
    pub schema_id: SchemaId,
    /// Closed logical key.
    pub logical_key: RecordLogicalKey,
    /// Exact five-record payload.
    pub payload: RunJournalRecord,
    /// Whether the transition settlement emits facts.
    pub emits_facts: bool,
}

impl CandidateRecordEnvelope {
    /// Constructs and validates an unassigned candidate record.
    pub fn new(
        ordinal: u32,
        schema_id: &SchemaId,
        logical_key: &RecordLogicalKey,
        payload: &RunJournalRecord,
        emits_facts: bool,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("ordinal", CanonicalValue::Unsigned(u64::from(ordinal))),
            ("schema_id", cv_string(schema_id.as_str())),
            ("logical_key", logical_key.canonical_value()?),
            ("payload", payload.canonical_value()?),
            ("emits_facts", CanonicalValue::Bool(emits_facts)),
        ])?)
    }

    /// Projects every candidate record field.
    pub fn fields(&self) -> Result<CandidateRecordEnvelopeFields> {
        Ok(CandidateRecordEnvelopeFields {
            ordinal: u32_field(self, "ordinal")?,
            schema_id: schema_id_field(self, "schema_id")?,
            logical_key: RecordLogicalKey::from_canonical_value(required_field(
                self,
                "logical_key",
            )?)?,
            payload: RunJournalRecord::from_canonical_value(required_field(self, "payload")?)?,
            emits_facts: bool_field(self, "emits_facts")?,
        })
    }
}

/// Typed fields of an unassigned commit candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitCandidatePreimageFields {
    /// Exact expected predecessor.
    pub expected_predecessor: JournalPredecessor,
    /// Closed batch purpose.
    pub batch_purpose: BatchPurpose,
    /// One or two ordered records.
    pub ordered_candidate_records: Vec<CandidateRecordEnvelope>,
    /// Complete canonically ordered object paths.
    pub ordered_object_bindings: Vec<ObjectPathBinding>,
    /// Complete canonically ordered artifact intents.
    pub artifact_admission_intents: Vec<ArtifactAdmissionIntent>,
}

impl CommitCandidatePreimage {
    /// Constructs and validates a complete unassigned commit candidate.
    pub fn new(
        expected_predecessor: &JournalPredecessor,
        batch_purpose: BatchPurpose,
        ordered_candidate_records: &[CandidateRecordEnvelope],
        ordered_object_bindings: &[ObjectPathBinding],
        artifact_admission_intents: &[ArtifactAdmissionIntent],
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "expected_predecessor",
                expected_predecessor.canonical_value()?,
            ),
            (
                "batch_purpose",
                CanonicalValue::String(batch_purpose.as_str().to_owned()),
            ),
            (
                "ordered_candidate_records",
                cv_array(
                    ordered_candidate_records
                        .iter()
                        .map(|value| value.canonical_value()),
                )?,
            ),
            (
                "ordered_object_bindings",
                cv_array(
                    ordered_object_bindings
                        .iter()
                        .map(|value| value.canonical_value()),
                )?,
            ),
            (
                "artifact_admission_intents",
                cv_array(
                    artifact_admission_intents
                        .iter()
                        .map(|value| value.canonical_value()),
                )?,
            ),
        ])?)
    }

    /// Projects every unassigned commit-candidate field.
    pub fn fields(&self) -> Result<CommitCandidatePreimageFields> {
        Ok(CommitCandidatePreimageFields {
            expected_predecessor: JournalPredecessor::from_canonical_value(required_field(
                self,
                "expected_predecessor",
            )?)?,
            batch_purpose: BatchPurpose::parse(&string_field(self, "batch_purpose")?)?,
            ordered_candidate_records: array_field(self, "ordered_candidate_records")?
                .into_iter()
                .map(CandidateRecordEnvelope::from_canonical_value)
                .collect::<Result<_>>()?,
            ordered_object_bindings: array_field(self, "ordered_object_bindings")?
                .into_iter()
                .map(ObjectPathBinding::from_canonical_value)
                .collect::<Result<_>>()?,
            artifact_admission_intents: array_field(self, "artifact_admission_intents")?
                .into_iter()
                .map(ArtifactAdmissionIntent::from_canonical_value)
                .collect::<Result<_>>()?,
        })
    }

    /// Derives the frozen journal-candidate semantic digest.
    pub fn candidate_digest(&self) -> Result<JournalCandidateDigest> {
        domain_digest("mfm.journal-candidate.v2", self)
            .map(JournalCandidateDigest::from_semantic_digest)
    }
}

impl RecordHashPreimage {
    /// Constructs the exact record-hash preimage.
    pub fn new(
        ordinal: u32,
        schema_id: &SchemaId,
        logical_key: &RecordLogicalKey,
        payload: &RunJournalRecord,
        emits_facts: bool,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("ordinal", CanonicalValue::Unsigned(u64::from(ordinal))),
            ("schema_id", cv_string(schema_id.as_str())),
            ("logical_key", logical_key.canonical_value()?),
            ("payload", payload.canonical_value()?),
            ("emits_facts", CanonicalValue::Bool(emits_facts)),
        ])?)
    }

    /// Constructs the record-hash preimage from an unassigned candidate.
    pub fn from_candidate(candidate: &CandidateRecordEnvelope) -> Result<Self> {
        let fields = candidate.fields()?;
        Self::new(
            fields.ordinal,
            &fields.schema_id,
            &fields.logical_key,
            &fields.payload,
            fields.emits_facts,
        )
    }

    /// Derives the frozen journal-record semantic hash.
    pub fn record_hash(&self) -> Result<JournalRecordHash> {
        domain_digest("mfm.journal-record.v2", self).map(JournalRecordHash::from_semantic_digest)
    }
}

impl RecordIdPreimage {
    /// Constructs and validates an assigned record-identity preimage.
    pub fn new(
        store_scope_id: &StoreScopeId,
        run_id: &RunId,
        assigned_run_sequence: u64,
        ordinal: u32,
        record_hash: &JournalRecordHash,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("store_scope_id", cv_string(store_scope_id.as_str())),
            ("run_id", cv_string(run_id.as_str())),
            ("assigned_run_sequence", cv_decimal(assigned_run_sequence)?),
            ("ordinal", CanonicalValue::Unsigned(u64::from(ordinal))),
            ("record_hash", cv_string(record_hash.as_str())),
        ])?)
    }

    /// Derives the frozen immutable journal record identity.
    pub fn record_id(&self) -> Result<RecordId> {
        domain_digest("mfm.journal-record-id.v1", self).map(RecordId::from_semantic_digest)
    }
}

/// Typed fields shared by a commit digest preimage and assigned envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitCoreFields {
    /// Store deployment identity.
    pub store_scope_id: StoreScopeId,
    /// Admitted run.
    pub run_id: RunId,
    /// Assigned per-run sequence.
    pub run_sequence: u64,
    /// Exact predecessor.
    pub predecessor: JournalPredecessor,
    /// Stable append request identifier.
    pub append_request_id: AppendRequestId,
    /// Unassigned candidate digest.
    pub candidate_digest: JournalCandidateDigest,
    /// Store-assigned optional tenant coordinate.
    pub tenant_fact_coordinate: TenantFactCoordinate,
    /// Ordered exact record hashes.
    pub ordered_record_hashes: Vec<JournalRecordHash>,
    /// Complete canonically ordered object paths.
    pub ordered_object_bindings: Vec<ObjectPathBinding>,
    /// Complete canonically ordered artifact intents.
    pub artifact_admission_intents: Vec<ArtifactAdmissionIntent>,
}

#[allow(clippy::too_many_arguments)]
fn commit_core_value(
    store_scope_id: &StoreScopeId,
    run_id: &RunId,
    run_sequence: u64,
    predecessor: &JournalPredecessor,
    append_request_id: &AppendRequestId,
    candidate_digest: &JournalCandidateDigest,
    tenant_fact_coordinate: &TenantFactCoordinate,
    ordered_record_hashes: &[JournalRecordHash],
    ordered_object_bindings: &[ObjectPathBinding],
    artifact_admission_intents: &[ArtifactAdmissionIntent],
) -> Result<Vec<(String, CanonicalValue)>> {
    Ok(vec![
        (
            "store_scope_id".to_owned(),
            cv_string(store_scope_id.as_str()),
        ),
        ("run_id".to_owned(), cv_string(run_id.as_str())),
        ("run_sequence".to_owned(), cv_decimal(run_sequence)?),
        ("predecessor".to_owned(), predecessor.canonical_value()?),
        (
            "append_request_id".to_owned(),
            cv_string(append_request_id.as_str()),
        ),
        (
            "candidate_digest".to_owned(),
            cv_string(candidate_digest.as_str()),
        ),
        (
            "tenant_fact_coordinate".to_owned(),
            tenant_fact_coordinate.canonical_value()?,
        ),
        (
            "ordered_record_hashes".to_owned(),
            CanonicalValue::Array(
                ordered_record_hashes
                    .iter()
                    .map(|digest| cv_string(digest.as_str()))
                    .collect(),
            ),
        ),
        (
            "ordered_object_bindings".to_owned(),
            cv_array(
                ordered_object_bindings
                    .iter()
                    .map(|value| value.canonical_value()),
            )?,
        ),
        (
            "artifact_admission_intents".to_owned(),
            cv_array(
                artifact_admission_intents
                    .iter()
                    .map(|value| value.canonical_value()),
            )?,
        ),
    ])
}

fn commit_core_fields(value: &impl super::PersistedJournalValue) -> Result<CommitCoreFields> {
    Ok(CommitCoreFields {
        store_scope_id: store_scope_id_field(value, "store_scope_id")?,
        run_id: run_id_field(value, "run_id")?,
        run_sequence: unsigned_field(value, "run_sequence")?,
        predecessor: JournalPredecessor::from_canonical_value(required_field(
            value,
            "predecessor",
        )?)?,
        append_request_id: append_request_id_field(value, "append_request_id")?,
        candidate_digest: JournalCandidateDigest::parse(string_field(value, "candidate_digest")?)?,
        tenant_fact_coordinate: TenantFactCoordinate::from_canonical_value(required_field(
            value,
            "tenant_fact_coordinate",
        )?)?,
        ordered_record_hashes: array_field(value, "ordered_record_hashes")?
            .into_iter()
            .map(|value| match value {
                CanonicalValue::String(value) => {
                    JournalRecordHash::parse(value).map_err(Into::into)
                }
                _ => Err(super::JournalError::Projection),
            })
            .collect::<Result<_>>()?,
        ordered_object_bindings: array_field(value, "ordered_object_bindings")?
            .into_iter()
            .map(ObjectPathBinding::from_canonical_value)
            .collect::<Result<_>>()?,
        artifact_admission_intents: array_field(value, "artifact_admission_intents")?
            .into_iter()
            .map(ArtifactAdmissionIntent::from_canonical_value)
            .collect::<Result<_>>()?,
    })
}

impl CommitDigestPreimage {
    /// Constructs and validates the exact assigned commit-digest preimage.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope_id: &StoreScopeId,
        run_id: &RunId,
        run_sequence: u64,
        predecessor: &JournalPredecessor,
        append_request_id: &AppendRequestId,
        candidate_digest: &JournalCandidateDigest,
        tenant_fact_coordinate: &TenantFactCoordinate,
        ordered_record_hashes: &[JournalRecordHash],
        ordered_object_bindings: &[ObjectPathBinding],
        artifact_admission_intents: &[ArtifactAdmissionIntent],
    ) -> Result<Self> {
        Self::from_canonical_value(object(commit_core_value(
            store_scope_id,
            run_id,
            run_sequence,
            predecessor,
            append_request_id,
            candidate_digest,
            tenant_fact_coordinate,
            ordered_record_hashes,
            ordered_object_bindings,
            artifact_admission_intents,
        )?)?)
    }

    /// Projects every commit-digest preimage field.
    pub fn fields(&self) -> Result<CommitCoreFields> {
        commit_core_fields(self)
    }

    /// Derives the frozen journal-commit semantic digest.
    pub fn commit_digest(&self) -> Result<JournalCommitDigest> {
        domain_digest("mfm.journal-commit.v1", self).map(JournalCommitDigest::from_semantic_digest)
    }
}

/// Typed fields of a complete assigned commit envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitEnvelopeFields {
    /// Hash-participating assigned commit fields.
    pub core: CommitCoreFields,
    /// Frozen commit digest.
    pub commit_digest: JournalCommitDigest,
    /// Operational commit timestamp excluded from semantic hashes.
    pub committed_at: u64,
}

impl CommitEnvelope {
    /// Constructs and validates a complete assigned commit envelope.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope_id: &StoreScopeId,
        run_id: &RunId,
        run_sequence: u64,
        predecessor: &JournalPredecessor,
        append_request_id: &AppendRequestId,
        candidate_digest: &JournalCandidateDigest,
        commit_digest: &JournalCommitDigest,
        tenant_fact_coordinate: &TenantFactCoordinate,
        ordered_record_hashes: &[JournalRecordHash],
        ordered_object_bindings: &[ObjectPathBinding],
        artifact_admission_intents: &[ArtifactAdmissionIntent],
        committed_at: u64,
    ) -> Result<Self> {
        let mut fields = commit_core_value(
            store_scope_id,
            run_id,
            run_sequence,
            predecessor,
            append_request_id,
            candidate_digest,
            tenant_fact_coordinate,
            ordered_record_hashes,
            ordered_object_bindings,
            artifact_admission_intents,
        )?;
        fields.push((
            "version".to_owned(),
            CanonicalValue::String("mfm.commit-envelope.v1".to_owned()),
        ));
        fields.push((
            "commit_digest".to_owned(),
            cv_string(commit_digest.as_str()),
        ));
        fields.push(("committed_at".to_owned(), cv_decimal(committed_at)?));
        Self::from_canonical_value(object(fields)?)
    }

    /// Projects every assigned commit-envelope field.
    pub fn fields(&self) -> Result<CommitEnvelopeFields> {
        Ok(CommitEnvelopeFields {
            core: commit_core_fields(self)?,
            commit_digest: JournalCommitDigest::parse(string_field(self, "commit_digest")?)?,
            committed_at: unsigned_field(self, "committed_at")?,
        })
    }

    /// Projects this envelope's exact assigned head.
    pub fn journal_head(&self) -> Result<JournalHead> {
        let fields = self.fields()?;
        JournalHead::new(fields.core.run_sequence, &fields.commit_digest)
    }
}

/// Closed legal commit batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegalCommitBatchFields {
    /// Sole admission record.
    RunAdmission(RunAdmitted),
    /// One transition and its optional inseparable closure.
    Transition {
        /// Complete transition.
        transition: StateTransitionCommitted,
        /// Closure present only with the terminal transition.
        closure: Option<RunClosed>,
    },
    /// Sole authorization record.
    Authorization(ExternalAccessAuthorized),
    /// Sole observation record.
    Observation(ExternalAccessObserved),
}

impl LegalCommitBatch {
    /// Constructs the admission batch.
    pub fn run_admission(admission: &RunAdmitted) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "run_admission",
            [("admission", admission.canonical_value()?)],
        )?)
    }

    /// Constructs a transition batch with its optional closure.
    pub fn transition(
        transition: &StateTransitionCommitted,
        closure: Option<&RunClosed>,
    ) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "transition",
            [
                ("transition", transition.canonical_value()?),
                (
                    "closure",
                    closure
                        .map(RunClosed::canonical_value)
                        .transpose()?
                        .unwrap_or(CanonicalValue::Null),
                ),
            ],
        )?)
    }

    /// Constructs the authorization batch.
    pub fn authorization(authorization: &ExternalAccessAuthorized) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "authorization",
            [("authorization", authorization.canonical_value()?)],
        )?)
    }

    /// Constructs the observation batch.
    pub fn observation(observation: &ExternalAccessObserved) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "observation",
            [("observation", observation.canonical_value()?)],
        )?)
    }

    /// Projects the closed legal batch.
    pub fn fields(&self) -> Result<LegalCommitBatchFields> {
        match super::codec::tag(self)?.as_str() {
            "run_admission" => Ok(LegalCommitBatchFields::RunAdmission(
                RunAdmitted::from_canonical_value(required_field(self, "admission")?)?,
            )),
            "transition" => Ok(LegalCommitBatchFields::Transition {
                transition: StateTransitionCommitted::from_canonical_value(required_field(
                    self,
                    "transition",
                )?)?,
                closure: super::codec::nullable_field(self, "closure")?
                    .map(RunClosed::from_canonical_value)
                    .transpose()?,
            }),
            "authorization" => Ok(LegalCommitBatchFields::Authorization(
                ExternalAccessAuthorized::from_canonical_value(required_field(
                    self,
                    "authorization",
                )?)?,
            )),
            "observation" => Ok(LegalCommitBatchFields::Observation(
                ExternalAccessObserved::from_canonical_value(required_field(self, "observation")?)?,
            )),
            _ => Err(super::JournalError::Projection),
        }
    }
}

impl GenesisPreimage {
    /// Constructs and validates the exact per-run genesis preimage.
    pub fn new(
        store_scope_id: &StoreScopeId,
        store_epoch: StoreEpoch,
        run_id: &RunId,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("store_scope_id", cv_string(store_scope_id.as_str())),
            ("store_epoch", cv_decimal(store_epoch.get())?),
            ("run_id", cv_string(run_id.as_str())),
        ])?)
    }

    /// Derives the frozen per-run genesis digest.
    pub fn genesis_digest(&self) -> Result<GenesisDigest> {
        domain_digest("mfm.genesis.v1", self).map(GenesisDigest::from_semantic_digest)
    }
}
