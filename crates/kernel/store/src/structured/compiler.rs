//! Deterministic artifact, record-assignment, and projection compiler.

use std::collections::BTreeSet;

use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, RequestDigest, StableId,
};
use mfm_journal::structured::{
    derive_access_attempt_id, derive_candidate_digest, derive_commit_digest, derive_record_hash,
    AccessAttemptIdentityPreimage, AssignedRecord, CommitCandidate, CommitDigestPreimage,
    CommittedBatch, CommittedFactRef, ExternalAccessAuthorized, ExternalAccessObserved,
    HistoryObject, JournalHead, LexicalValueRef, ObservationOutcome, RecordHashPreimage, RecordRef,
    RunAdmitted, RunClosed, RunRecord, StateOutcomeRef, StateTransitionCommitted,
    TenantFactCoordinate, TenantFactFrontier, TypedValueRef,
};
use mfm_program_derive::PersistedSchema;
use mfm_spec::CanonicalJsonValue;
use mfm_values::PersistedObjectPayload;
use serde::{Deserialize, Serialize};

use super::backend::{StructuredStoreIdentity, TenantFactPublication};
use super::qualification::{
    invalid, QualifiedRunContext, RecordedAssertions, StructuredStoreError,
};
use super::reducer::{
    ArtifactIntent, AuthorizationIntent, FactIntent, ObservationIntent, ObservationOutcomeIntent,
    PendingSemanticStep, PrimaryIntent, ReducedRunState, SemanticObligation, StateOutcomeIntent,
    TenantFactRequirement, TransitionIntent,
};
use super::validated_append::{RunCurrentProjection, RunProjectionPlan, TenantFactProjectionPlan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AddressedArtifactIntent {
    reference: ContentRef,
    payload: ArtifactIntent,
}

impl AddressedArtifactIntent {
    pub(super) const fn content_ref(&self) -> &ContentRef {
        &self.reference
    }

    pub(super) fn typed_value(&self) -> Option<&CanonicalJsonValue> {
        match &self.payload {
            ArtifactIntent::TypedValue { value, .. } => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[mfm(schema = "mfm.structured-state-outcome", version = "1")]
enum StateOutcomeArtifact {
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
enum OperationOutcomeArtifact {
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
struct FactClaimArtifact {
    descriptor_ref: ContentRef,
    subject: TypedValueRef,
    response: TypedValueRef,
}

impl PersistedObjectPayload for FactClaimArtifact {
    fn object_type() -> mfm_values::Result<StableId> {
        StableId::new("structured.fact_claim")
            .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

pub(super) fn address_artifact(
    payload: ArtifactIntent,
) -> Result<AddressedArtifactIntent, StructuredStoreError> {
    let reference = materialize_artifact(&payload)?.content_ref;
    Ok(AddressedArtifactIntent { reference, payload })
}

fn materialize_artifact(payload: &ArtifactIntent) -> Result<HistoryObject, StructuredStoreError> {
    match payload {
        ArtifactIntent::TypedValue { schema_id, value } => {
            let canonical = value.canonical_json().map_err(|_| invalid())?;
            let content_ref = ContentRef::new(
                schema_id.clone(),
                ContentDigest::from_digest(
                    DigestAlgorithm::Sha256V1,
                    mfm_canonical::sha256_digest_bytes(canonical.as_bytes()),
                ),
            )
            .map_err(|_| invalid())?;
            Ok(HistoryObject {
                object_type: StableId::new(mfm_journal::structured::TYPED_VALUE_OBJECT_TYPE)
                    .map_err(|_| invalid())?,
                content_ref,
                canonical_json: canonical.as_str().to_owned(),
            })
        }
        ArtifactIntent::StateOutcome(value) => {
            let artifact = match value {
                StateOutcomeIntent::Success(value) => StateOutcomeArtifact::Success(value.clone()),
                StateOutcomeIntent::Failure(value) => StateOutcomeArtifact::Failure(value.clone()),
            };
            HistoryObject::from_persisted(&artifact).map_err(|_| invalid())
        }
        ArtifactIntent::OperationOutcome(value) => {
            let artifact = match value {
                StateOutcomeIntent::Success(value) => {
                    OperationOutcomeArtifact::Success(value.clone())
                }
                StateOutcomeIntent::Failure(value) => {
                    OperationOutcomeArtifact::Failure(value.clone())
                }
            };
            HistoryObject::from_persisted(&artifact).map_err(|_| invalid())
        }
        ArtifactIntent::FactClaim {
            descriptor_ref,
            subject,
            response,
        } => HistoryObject::from_persisted(&FactClaimArtifact {
            descriptor_ref: descriptor_ref.clone(),
            subject: subject.clone(),
            response: response.clone(),
        })
        .map_err(|_| invalid()),
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
    authored: &[AddressedArtifactIntent],
    retained_objects: &[ContentRef],
    context: &QualifiedRunContext,
    retained: &BTreeSet<ContentRef>,
) -> Result<Vec<HistoryObject>, StructuredStoreError> {
    let mut objects = authored
        .iter()
        .map(|intent| {
            let object = materialize_artifact(&intent.payload)?;
            (object.content_ref == intent.reference)
                .then_some(object)
                .ok_or_else(invalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    objects.extend(
        retained_objects
            .iter()
            .map(|reference| context.object(reference).cloned().ok_or_else(invalid))
            .collect::<Result<Vec<_>, _>>()?,
    );
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

fn materialize_fact(intent: &FactIntent) -> CommittedFactRef {
    CommittedFactRef {
        emission_ordinal: intent.emission_ordinal,
        fact_slot_ordinal: intent.fact_slot_ordinal,
        descriptor_ref: intent.descriptor_ref.clone(),
        subject: intent.subject.clone(),
        response: intent.response.clone(),
        claim_ref: intent.claim_ref.clone(),
    }
}

fn materialize_state_outcome(intent: &StateOutcomeIntent) -> StateOutcomeRef {
    match intent {
        StateOutcomeIntent::Success(value) => StateOutcomeRef::Success(value.clone()),
        StateOutcomeIntent::Failure(value) => StateOutcomeRef::Failure(value.clone()),
    }
}

fn materialize_transition(intent: &TransitionIntent) -> StateTransitionCommitted {
    StateTransitionCommitted {
        occurrence_id: intent.occurrence_id.clone(),
        occurrence_path_ref: intent.occurrence_path_ref.clone(),
        semantic_call_id: intent.semantic_call_id.clone(),
        input: intent.input.clone(),
        consumed_observation_ref: intent.consumed_observation_ref.clone(),
        outcome_ref: intent.outcome_ref.clone(),
        outcome: materialize_state_outcome(&intent.outcome),
        facts: intent.facts.iter().map(materialize_fact).collect(),
        before_semantic_state_digest: intent.before_semantic_state_digest.clone(),
        after_semantic_state_digest: intent.after_semantic_state_digest.clone(),
    }
}

fn materialize_authorization(
    run_id: &mfm_ids::RunId,
    intent: &AuthorizationIntent,
) -> Result<ExternalAccessAuthorized, StructuredStoreError> {
    let authorization = ExternalAccessAuthorized {
        access_attempt_id: intent.access_attempt_id.clone(),
        attempt_ordinal: intent.attempt_ordinal,
        occurrence_id: intent.occurrence_id.clone(),
        occurrence_path_ref: intent.occurrence_path_ref.clone(),
        semantic_call_id: intent.semantic_call_id.clone(),
        state_input_ref: intent.state_input_ref.clone(),
        access_kind: intent.access_kind,
        semantic_head: intent.semantic_head.clone(),
        store_scope_id: intent.store_scope_id.clone(),
        store_epoch: intent.store_epoch,
        tenant_scope_id: intent.tenant_scope_id.clone(),
        admitted_routing_policy_ref: intent.admitted_routing_policy_ref.clone(),
        minimum_lineage_head_ref: intent.minimum_lineage_head_ref.clone(),
        capability_contract_ref: intent.capability_contract_ref.clone(),
        capability_implementation_ref: intent.capability_implementation_ref.clone(),
        adapter_contract_ref: intent.adapter_contract_ref.clone(),
        adapter_implementation_ref: intent.adapter_implementation_ref.clone(),
        request: intent.request.clone(),
        request_digest: intent.request_digest.clone(),
        physical_binding_ref: intent.physical_binding_ref.clone(),
        stable_resource_lineage_contract_ref: intent.stable_resource_lineage_contract_ref.clone(),
    };
    let derived = derive_access_attempt_id(&AccessAttemptIdentityPreimage {
        run_id,
        occurrence_id: &authorization.occurrence_id,
        occurrence_path_ref: &authorization.occurrence_path_ref,
        semantic_call_id: &authorization.semantic_call_id,
        state_input_ref: &authorization.state_input_ref,
        attempt_ordinal: authorization.attempt_ordinal,
        access_kind: authorization.access_kind,
        semantic_head: &authorization.semantic_head,
        capability_contract_ref: &authorization.capability_contract_ref,
        capability_implementation_ref: &authorization.capability_implementation_ref,
        adapter_contract_ref: &authorization.adapter_contract_ref,
        adapter_implementation_ref: &authorization.adapter_implementation_ref,
        request: &authorization.request,
        request_digest: &authorization.request_digest,
        physical_binding_ref: &authorization.physical_binding_ref,
        stable_resource_lineage_contract_ref: &authorization.stable_resource_lineage_contract_ref,
    })
    .map_err(|_| invalid())?;
    (derived == authorization.access_attempt_id)
        .then_some(authorization)
        .ok_or_else(invalid)
}

fn materialize_observation(intent: &ObservationIntent) -> ExternalAccessObserved {
    let outcome = match &intent.outcome {
        ObservationOutcomeIntent::Returned(value) => ObservationOutcome::Returned {
            value: value.clone(),
        },
        ObservationOutcomeIntent::SafeFailure(value) => ObservationOutcome::SafeFailure {
            value: value.clone(),
        },
        ObservationOutcomeIntent::SupersededBeforeEntry {
            public_lineage_head_ref,
            evidence_ref,
        } => ObservationOutcome::SupersededBeforeEntry {
            public_lineage_head_ref: public_lineage_head_ref.clone(),
            evidence_ref: evidence_ref.clone(),
        },
        ObservationOutcomeIntent::EntryUnknown { fault_code } => ObservationOutcome::EntryUnknown {
            fault_code: fault_code.clone(),
        },
        ObservationOutcomeIntent::IntegrityFault { fault_code } => {
            ObservationOutcome::IntegrityFault {
                fault_code: fault_code.clone(),
            }
        }
    };
    ExternalAccessObserved {
        authorization_ref: intent.authorization_ref.clone(),
        access_attempt_id: intent.access_attempt_id.clone(),
        outcome,
    }
}

fn materialize_primary(
    context: &QualifiedRunContext,
    intent: &PrimaryIntent,
) -> Result<RunRecord, StructuredStoreError> {
    Ok(match intent {
        PrimaryIntent::Admission(intent) => RunRecord::RunAdmitted(RunAdmitted {
            store_scope_id: context.admission.store_scope_id.clone(),
            store_epoch: context.admission.store_epoch,
            run_id: context.admission.run_id.clone(),
            tenant_scope_id: context.admission.tenant_scope_id.clone(),
            invocation_identity: context.admission.invocation_identity.clone(),
            entry_point_operation_id: context.admission.entry_point_operation_id.clone(),
            certified_program_ref: context.admission.certified_program_ref.clone(),
            admission_material_refs: context.admission.admission_material_refs.clone(),
            initial_bindings: context.admission.initial_bindings.clone(),
            genesis_semantic_state_digest: intent.genesis_semantic_state_digest.clone(),
        }),
        PrimaryIntent::Transition(intent) => {
            RunRecord::StateTransitionCommitted(materialize_transition(intent))
        }
        PrimaryIntent::Authorization(intent) => RunRecord::ExternalAccessAuthorized(
            materialize_authorization(&context.admission.run_id, intent)?,
        ),
        PrimaryIntent::Observation(intent) => {
            RunRecord::ExternalAccessObserved(materialize_observation(intent))
        }
    })
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
    let objects = materialize_artifacts(
        pending.artifacts(),
        pending.retained_objects(),
        context,
        retained_objects,
    )?;
    let coordinate = match (pending.tenant_fact_requirement(), tenant_frontier.as_ref()) {
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
    let mut records = vec![materialize_primary(context, pending.primary())?];
    if let Some(outcome_ref) = pending.closure() {
        records.push(RunRecord::RunClosed(RunClosed {
            outcome_ref: outcome_ref.clone(),
        }));
    }
    let candidate = CommitCandidate {
        run_id: context.admission.run_id.clone(),
        expected_head: previous_projection
            .as_ref()
            .map(|projection| projection.journal_head.clone()),
        append_request_id: append_request_id.clone(),
        tenant_fact_coordinate: coordinate,
        records,
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

pub(super) struct ComparisonPassed(());

pub(super) struct CompiledAppend {
    committed: CommittedBatch,
    run_projection: RunProjectionPlan,
    tenant_fact_plan: TenantFactProjectionPlan,
    fact_read_capability_spec: Option<FactScanPermitSpec>,
}

impl CompiledAppend {
    pub(super) const fn tenant_fact_plan(&self) -> &TenantFactProjectionPlan {
        &self.tenant_fact_plan
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        CommittedBatch,
        RunProjectionPlan,
        TenantFactProjectionPlan,
        Option<FactScanPermitSpec>,
    ) {
        (
            self.committed,
            self.run_projection,
            self.tenant_fact_plan,
            self.fact_read_capability_spec,
        )
    }
}

pub(super) struct ComparedReduction {
    reduced: Box<ReducedRunState>,
    compiled: CompiledAppend,
    obligations: Vec<SemanticObligation>,
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
        let primary_assignment = qualified
            .records
            .first()
            .map(|record| &record.record_ref)
            .ok_or_else(invalid)?;
        let (reduced, obligations) = recorded.bind_after_comparison(
            ComparisonPassed(()),
            primary_assignment,
            qualified.head.clone(),
        )?;
        Ok(Self {
            reduced,
            compiled: CompiledAppend {
                committed: preview.committed,
                run_projection: preview.run_projection,
                tenant_fact_plan: preview.tenant_fact_plan,
                fact_read_capability_spec: preview.fact_read_capability_spec,
            },
            obligations,
        })
    }

    pub(super) fn obligations(&self) -> &[SemanticObligation] {
        &self.obligations
    }

    pub(super) fn into_finalization_parts(
        self,
        _passed: super::obligations::DischargePassed,
    ) -> (Box<ReducedRunState>, CompiledAppend) {
        (self.reduced, self.compiled)
    }
}
