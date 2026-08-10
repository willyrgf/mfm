//! Deterministic artifact, record-assignment, and projection compiler.

use std::collections::BTreeSet;

use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, RequestDigest, SchemaId, StableId,
};
use mfm_journal::structured::{
    derive_candidate_digest, derive_commit_digest, derive_record_hash, AssignedRecord,
    CommitCandidate, CommitDigestPreimage, CommittedBatch, HistoryObject, JournalHead,
    LexicalValueRef, RecordHashPreimage, RecordRef, RunRecord, TenantFactCoordinate,
    TenantFactFrontier, TypedValueRef,
};
use mfm_program_derive::PersistedSchema;
use mfm_spec::CanonicalJsonValue;
use mfm_values::{CanonicalJsonPersistedSchema, PersistedObjectPayload};
use serde::{Deserialize, Serialize};

use super::backend::{StructuredStoreIdentity, TenantFactPublication};
use super::qualification::{
    invalid, QualifiedRunContext, RecordedAssertions, StructuredStoreError,
};
use super::reducer::{PendingSemanticStep, RecordIntent, TenantFactRequirement};
use super::validated_append::{RunCurrentProjection, RunProjectionPlan, TenantFactProjectionPlan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ArtifactIntent {
    TypedValue {
        schema_id: SchemaId,
        value: CanonicalJsonValue,
    },
    StateOutcome(StateOutcomeArtifact),
    OperationOutcome(OperationOutcomeArtifact),
    FactClaim(FactClaimArtifact),
    QualifiedObject(ContentRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[mfm(schema = "mfm.structured-state-outcome", version = "1")]
pub(super) enum StateOutcomeArtifact {
    #[serde(rename = "Success")]
    Success(LexicalValueRef),
    #[serde(rename = "Failure")]
    Failure(LexicalValueRef),
}

impl PersistedObjectPayload for StateOutcomeArtifact {
    fn object_type() -> mfm_values::Result<StableId> {
        StableId::new("structured.state_outcome")
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[mfm(schema = "mfm.structured-operation-outcome", version = "1")]
pub(super) enum OperationOutcomeArtifact {
    #[serde(rename = "Success")]
    Success(LexicalValueRef),
    #[serde(rename = "Failure")]
    Failure(LexicalValueRef),
}

impl PersistedObjectPayload for OperationOutcomeArtifact {
    fn object_type() -> mfm_values::Result<StableId> {
        StableId::new("structured.operation_outcome")
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.structured-fact-claim", version = "1")]
pub(super) struct FactClaimArtifact {
    pub(super) descriptor_ref: ContentRef,
    pub(super) subject: TypedValueRef,
    pub(super) response: TypedValueRef,
}

impl PersistedObjectPayload for FactClaimArtifact {
    fn object_type() -> mfm_values::Result<StableId> {
        StableId::new("structured.fact_claim")
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

impl ArtifactIntent {
    pub(super) fn content_ref(&self) -> Result<ContentRef, StructuredStoreError> {
        match self {
            Self::TypedValue { schema_id, value } => {
                let bytes = value.canonical_json().map_err(|_| invalid())?;
                ContentRef::new(
                    schema_id.clone(),
                    ContentDigest::from_digest(
                        DigestAlgorithm::Sha256V1,
                        mfm_canonical::sha256_digest_bytes(bytes.as_bytes()),
                    ),
                )
                .map_err(|_| invalid())
            }
            Self::StateOutcome(value) => value.content_ref().map_err(|_| invalid()),
            Self::OperationOutcome(value) => value.content_ref().map_err(|_| invalid()),
            Self::FactClaim(value) => value.content_ref().map_err(|_| invalid()),
            Self::QualifiedObject(reference) => Ok(reference.clone()),
        }
    }

    fn materialize(
        &self,
        context: &QualifiedRunContext,
    ) -> Result<HistoryObject, StructuredStoreError> {
        match self {
            Self::TypedValue { value, .. } => Ok(HistoryObject {
                object_type: StableId::new(mfm_journal::structured::TYPED_VALUE_OBJECT_TYPE)
                    .map_err(|_| invalid())?,
                content_ref: self.content_ref()?,
                canonical_json: value
                    .canonical_json()
                    .map_err(|_| invalid())?
                    .as_str()
                    .to_owned(),
            }),
            Self::StateOutcome(value) => {
                HistoryObject::from_persisted(value).map_err(|_| invalid())
            }
            Self::OperationOutcome(value) => {
                HistoryObject::from_persisted(value).map_err(|_| invalid())
            }
            Self::FactClaim(value) => HistoryObject::from_persisted(value).map_err(|_| invalid()),
            Self::QualifiedObject(reference) => {
                context.object(reference).cloned().ok_or_else(invalid)
            }
        }
    }
}

pub(super) fn request_digest(
    value: &CanonicalJsonValue,
) -> Result<RequestDigest, StructuredStoreError> {
    let bytes = value.canonical_json().map_err(|_| invalid())?;
    Ok(RequestDigest::from_digest(
        mfm_canonical::sha256_digest_bytes(bytes.as_bytes()),
    ))
}

fn materialize_artifacts(
    intents: &[ArtifactIntent],
    context: &QualifiedRunContext,
    retained: &BTreeSet<ContentRef>,
) -> Result<Vec<HistoryObject>, StructuredStoreError> {
    let mut objects = intents
        .iter()
        .map(|intent| intent.materialize(context))
        .collect::<Result<Vec<_>, _>>()?;
    objects.sort_by(|left, right| left.content_ref.cmp(&right.content_ref));
    let mut canonical: Vec<HistoryObject> = Vec::with_capacity(objects.len());
    for object in objects {
        match canonical.last() {
            Some(existing) if existing == &object => {}
            Some(existing) if existing.content_ref == object.content_ref => {
                return Err(invalid());
            }
            _ if retained.contains(&object.content_ref) => {}
            _ => canonical.push(object),
        }
    }
    Ok(canonical)
}

fn record(intent: &RecordIntent) -> RunRecord {
    match intent {
        RecordIntent::Admission(value) => RunRecord::RunAdmitted(value.clone()),
        RecordIntent::Transition(value) => RunRecord::StateTransitionCommitted(value.clone()),
        RecordIntent::Authorization(value) => RunRecord::ExternalAccessAuthorized(value.clone()),
        RecordIntent::Observation(value) => RunRecord::ExternalAccessObserved(value.clone()),
        RecordIntent::Closure(value) => RunRecord::RunClosed(value.clone()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FactScanPermitSpec {
    pub(super) authorization_ref: RecordRef,
    pub(super) request_ref: TypedValueRef,
}

pub(super) struct CompiledPreview {
    committed: CommittedBatch,
    run_projection: RunProjectionPlan,
    tenant_fact_plan: TenantFactProjectionPlan,
    fact_read_capability_spec: Option<FactScanPermitSpec>,
}

impl CompiledPreview {
    pub(super) fn committed(&self) -> &CommittedBatch {
        &self.committed
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_preview(
    identity: &StructuredStoreIdentity,
    context: &QualifiedRunContext,
    previous_projection: Option<RunCurrentProjection>,
    tenant_frontier: Option<TenantFactFrontier>,
    retained_objects: &BTreeSet<ContentRef>,
    append_request_id: &AppendRequestId,
    pending: &PendingSemanticStep,
) -> Result<CompiledPreview, StructuredStoreError> {
    let objects = materialize_artifacts(&pending.artifacts, context, retained_objects)?;
    let coordinate = match (&pending.tenant_fact_requirement, tenant_frontier.as_ref()) {
        (TenantFactRequirement::None, None) => TenantFactCoordinate::None,
        (TenantFactRequirement::Barrier, Some(frontier)) => {
            TenantFactCoordinate::FactSelectionBarrier {
                frontier: frontier.clone(),
            }
        }
        (TenantFactRequirement::Publish, Some(frontier)) => TenantFactCoordinate::FactPublication {
            frontier: frontier.next_publication().map_err(|_| invalid())?,
        },
        _ => return Err(invalid()),
    };
    if coordinate.frontier().is_some_and(|frontier| {
        frontier.store_scope_id != identity.store_scope_id
            || frontier.store_epoch != identity.store_epoch
            || frontier.tenant_scope_id != context.admission.tenant_scope_id
    }) {
        return Err(invalid());
    }
    let candidate = CommitCandidate {
        run_id: context.admission.run_id.clone(),
        expected_head: previous_projection
            .as_ref()
            .map(|projection| projection.journal_head.clone()),
        append_request_id: append_request_id.clone(),
        tenant_fact_coordinate: coordinate,
        records: pending.records.iter().map(record).collect(),
        objects,
    };
    let committed = assign(identity, candidate)?;
    let successor = RunCurrentProjection {
        run_id: context.admission.run_id.clone(),
        tenant_scope_id: context.admission.tenant_scope_id.clone(),
        journal_head: committed.head.clone(),
        has_effect_entry_attention: pending.has_effect_entry_attention(),
    };
    let run_projection = RunProjectionPlan::new(previous_projection, successor);
    let tenant_fact_plan = match &committed.tenant_fact_coordinate {
        TenantFactCoordinate::None => TenantFactProjectionPlan::None,
        TenantFactCoordinate::FactSelectionBarrier { frontier } => {
            TenantFactProjectionPlan::Barrier {
                expected_frontier: frontier.clone(),
            }
        }
        TenantFactCoordinate::FactPublication { frontier } => {
            let expected_predecessor = tenant_frontier.ok_or_else(invalid)?;
            TenantFactProjectionPlan::Publish {
                expected_predecessor,
                publication: TenantFactPublication {
                    frontier: frontier.clone(),
                    producer_head: committed.head.clone(),
                    transition_ref: committed
                        .records
                        .first()
                        .ok_or_else(invalid)?
                        .record_ref
                        .clone(),
                },
            }
        }
    };
    let fact_read_capability_spec = if matches!(
        committed.tenant_fact_coordinate,
        TenantFactCoordinate::FactSelectionBarrier { .. }
    ) {
        let assigned = committed.records.first().ok_or_else(invalid)?;
        let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
            return Err(invalid());
        };
        Some(FactScanPermitSpec {
            authorization_ref: assigned.record_ref.clone(),
            request_ref: authorization.request.clone(),
        })
    } else {
        None
    };
    Ok(CompiledPreview {
        committed,
        run_projection,
        tenant_fact_plan,
        fact_read_capability_spec,
    })
}

pub(super) fn assign(
    identity: &StructuredStoreIdentity,
    candidate: CommitCandidate,
) -> Result<CommittedBatch, StructuredStoreError> {
    let sequence = candidate.expected_head.as_ref().map_or(Ok(1), |head| {
        head.run_sequence.checked_add(1).ok_or_else(invalid)
    })?;
    let candidate_digest = derive_candidate_digest(&candidate).map_err(|_| invalid())?;
    let records = candidate
        .records
        .iter()
        .enumerate()
        .map(|(ordinal, record)| {
            let ordinal = u32::try_from(ordinal).map_err(|_| invalid())?;
            Ok(AssignedRecord {
                record_ref: RecordRef::new(
                    candidate.run_id.clone(),
                    sequence,
                    ordinal,
                    derive_record_hash(&RecordHashPreimage {
                        run_id: &candidate.run_id,
                        run_sequence: sequence,
                        ordinal,
                        record,
                    })
                    .map_err(|_| invalid())?,
                ),
                record: record.clone(),
            })
        })
        .collect::<Result<Vec<_>, StructuredStoreError>>()?;
    let commit_digest = derive_commit_digest(&CommitDigestPreimage {
        store_scope_id: &identity.store_scope_id,
        store_epoch: identity.store_epoch,
        predecessor: &candidate.expected_head,
        append_request_id: &candidate.append_request_id,
        tenant_fact_coordinate: &candidate.tenant_fact_coordinate,
        candidate_digest: &candidate_digest,
        record_refs: records.iter().map(|record| &record.record_ref).collect(),
        object_refs: candidate
            .objects
            .iter()
            .map(|object| &object.content_ref)
            .collect(),
    })
    .map_err(|_| invalid())?;
    Ok(CommittedBatch {
        store_scope_id: identity.store_scope_id.clone(),
        store_epoch: identity.store_epoch,
        predecessor: candidate.expected_head,
        append_request_id: candidate.append_request_id,
        tenant_fact_coordinate: candidate.tenant_fact_coordinate,
        candidate_digest,
        records,
        objects: candidate.objects,
        head: JournalHead {
            run_sequence: sequence,
            commit_digest,
        },
    })
}

pub(super) struct ComparedReduction {
    pub(super) pending: PendingSemanticStep,
    pub(super) committed: CommittedBatch,
    pub(super) run_projection: RunProjectionPlan,
    pub(super) tenant_fact_plan: TenantFactProjectionPlan,
    pub(super) fact_read_capability_spec: Option<FactScanPermitSpec>,
}

impl ComparedReduction {
    pub(super) fn compare(
        intent: Option<PendingSemanticStep>,
        recorded: PendingSemanticStep,
        assertions: &RecordedAssertions,
        qualified: &CommittedBatch,
        preview: CompiledPreview,
    ) -> Result<Self, StructuredStoreError> {
        if intent
            .as_ref()
            .is_some_and(|intent| !intent.semantic_eq(&recorded))
            || preview.committed != *qualified
            || assertions.records != qualified.records
            || assertions.head != qualified.head
            || assertions.candidate_digest != qualified.candidate_digest
            || assertions.object_refs
                != qualified
                    .objects
                    .iter()
                    .map(|object| object.content_ref.clone())
                    .collect::<Vec<_>>()
        {
            return Err(invalid());
        }
        Ok(Self {
            pending: recorded,
            committed: preview.committed,
            run_projection: preview.run_projection,
            tenant_fact_plan: preview.tenant_fact_plan,
            fact_read_capability_spec: preview.fact_read_capability_spec,
        })
    }
}
