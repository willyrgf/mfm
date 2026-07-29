use std::collections::BTreeSet;

use mfm_canonical::{PlainCanonicalJsonBytes, RecoverabilityContractV2};
use mfm_capabilities::{
    BoundaryStage, FailureClass, SafeFailureClassifierDescriptor, SafeFailureClassifierRule,
    SafeFailureDiagnosticRule, SafeFailureOutcome, SafeFailureSizeRule,
};
use mfm_facts::{
    CanonicalFactPredicate, FactOrdering, FactProposal, FactSelectionLimit, FactSelectionQuery,
    FactSelectionRequest, FactSubject, FactTieBreak, ProposedFactValue,
};
use mfm_ids::{AppendRequestId, ContentRef, RunId, TenantScopeId};
use mfm_journal::v1::{
    ArtifactAdmissionMode, BindingDeltaEntryFields, FactSelectionCompletenessFields,
    FactSelectionScanContract, ObservationRef, ReadCapabilityBinding, SettlementFields,
    TenantFactFrontier, TransitionBodyFields, TransitionRef, ValueRef,
};
use mfm_spec::v1::{EntryPointContract, RetainedValueContract};

use super::{
    canonical_json, content_ref, field_path, fixture_error, semantic_type, stable_id,
    FixtureFactExecution, FixtureReadExecution, LegalAdmissionFixture,
};
use crate::v1::fact_scan::{
    abandon_fact_scan_after_first_page, scan_fact_selection_with_fact_limit,
    verify_middle_fact_omission_rejected, verify_three_fact_completion,
};
use crate::v1::{
    AdmissionSourceBackend, AppendOutcome, AuthorizationMaterial, CommittedJournalCommit,
    ConfiguredValueBackend, ExistingRunAppendMaterial, FactAttestationLoadVerifier,
    FactSelectionAuthorizationOutcome, FactSelectionStore, NewlyAppended, ObjectGraphProposal,
    PreparedJournalAppend, ProducedObjectRoot, ProducedOutputSlot, QualifiedSupportMember, Result,
    RunAccessAuthorityIssuer, RunJournalBackend, RunJournalStore, SettlementMaterial, StoreError,
    StoreIdentity, SupportBackend, TransitionMaterial, FACT_SELECTION_OPERATION_ID,
};

pub(in crate::v1) struct FactSelectionFixtureContracts {
    pub(in crate::v1) execution: FixtureReadExecution,
    pub(in crate::v1) routing_generation_ref: ContentRef,
    pub(in crate::v1) request_contract: RetainedValueContract,
    #[cfg(test)]
    pub(in crate::v1) response_contract: RetainedValueContract,
    #[cfg(test)]
    pub(in crate::v1) attestation_contract: RetainedValueContract,
    pub(in crate::v1) request: FactSelectionRequest,
}

pub(in crate::v1) fn fact_selection_contracts(
    discriminator: u8,
    descriptor_ref: Option<&ContentRef>,
    predicate: CanonicalFactPredicate,
    ordering: FactOrdering,
    limit: u32,
) -> Result<FactSelectionFixtureContracts> {
    let contract = RecoverabilityContractV2::embedded()?;
    let primitive_schema = contract
        .schema_id("mfm.primitive-canonical_value.v1")?
        .clone();
    let routing_contract = retained_contract(
        primitive_schema.clone(),
        "fact-routing",
        "fact-routing",
        discriminator,
        default_evidence_ref()?,
    )?;
    let routing_bytes = canonical_json(r#"{"routes":[]}"#)?;
    let (routing_member, routing_ref) =
        support_member("fact.routing", routing_bytes, routing_contract)?;

    let request_contract = retained_contract(
        contract.schema_id("mfm.fact-selection-request.v1")?.clone(),
        "fact-selection-request",
        "fact-selection-request",
        discriminator,
        default_evidence_ref()?,
    )?;
    let response_contract = retained_contract(
        contract
            .schema_id("mfm.fact-selection-response.v1")?
            .clone(),
        "fact-selection-response",
        "fact-selection-response",
        discriminator,
        routing_ref.clone(),
    )?;
    let attestation_contract = retained_contract(
        contract
            .schema_id("mfm.fact-selection-scan-attestation.v1")?
            .clone(),
        "fact-selection-attestation",
        "fact-selection-attestation",
        discriminator,
        routing_ref.clone(),
    )?;
    let scan_contract = FactSelectionScanContract::new(
        &stable_id(FACT_SELECTION_OPERATION_ID)?,
        &request_contract,
        &response_contract,
        &attestation_contract,
    )?;
    let scan_contract_value =
        PlainCanonicalJsonBytes::from_canonical_json_slice(scan_contract.as_bytes())
            .map_err(|_| fixture_error("fact scan contract is not canonical"))?;
    let scan_contract_retained = retained_contract(
        scan_contract.schema_id().clone(),
        "fact-scan-contract",
        "fact-scan-contract",
        discriminator,
        default_evidence_ref()?,
    )?;
    let expected_scan_ref = content_ref(&scan_contract_retained, scan_contract_value.as_bytes())?;
    let (scan_member, scan_ref) = support_member(
        "fact.scan_contract",
        scan_contract_value,
        scan_contract_retained,
    )?;
    if expected_scan_ref != scan_ref {
        return Err(fixture_error(
            "fact scan contract reference is inconsistent",
        ));
    }

    let classifier = SafeFailureClassifierDescriptor::new(
        routing_ref.clone(),
        None,
        vec![SafeFailureClassifierRule::new(
            stable_id("destination_unavailable")?,
            SafeFailureOutcome::DidNotEnter,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
            SafeFailureSizeRule::None,
            SafeFailureDiagnosticRule::Forbidden,
        )],
    )
    .map_err(|_| fixture_error("fact classifier is invalid"))?;
    let classifier_contract = retained_contract(
        SafeFailureClassifierDescriptor::schema_id()
            .map_err(|_| fixture_error("fact classifier schema is invalid"))?,
        "fact-classifier",
        "fact-classifier",
        discriminator,
        default_evidence_ref()?,
    )?;
    let (classifier_member, classifier_ref) = support_member(
        "fact.classifier",
        classifier
            .canonical()
            .map_err(|_| fixture_error("fact classifier is not canonical"))?,
        classifier_contract,
    )?;
    if classifier
        .content_ref()
        .map_err(|_| fixture_error("fact classifier reference is invalid"))?
        != classifier_ref
    {
        return Err(fixture_error("fact classifier reference is inconsistent"));
    }

    let binding = ReadCapabilityBinding::new(
        &scan_ref,
        &routing_ref,
        &classifier_ref,
        &routing_ref,
        &routing_ref,
        &routing_ref,
    )?;
    let binding_value = PlainCanonicalJsonBytes::from_canonical_json_slice(binding.as_bytes())
        .map_err(|_| fixture_error("fact binding is not canonical"))?;
    let binding_contract = retained_contract(
        binding.schema_id().clone(),
        "fact-binding",
        "fact-binding",
        discriminator,
        default_evidence_ref()?,
    )?;
    let (binding_member, binding_ref) =
        support_member("fact.binding", binding_value, binding_contract)?;
    if binding.content_ref()? != binding_ref {
        return Err(fixture_error("fact binding reference is inconsistent"));
    }

    let safe_failure_contract = retained_contract(
        primitive_schema,
        "fact-safe-failure",
        "fact-safe-failure",
        discriminator,
        default_evidence_ref()?,
    )?;
    let query = FactSelectionQuery::new(
        descriptor_ref
            .cloned()
            .unwrap_or_else(|| routing_ref.clone()),
        predicate,
        None,
        ordering,
        FactSelectionLimit::new(limit)
            .map_err(|_| fixture_error("fact selection limit is invalid"))?,
        FactTieBreak::FactIdentityAscending,
    )
    .map_err(|_| fixture_error("fact selection query is invalid"))?;
    let request = FactSelectionRequest::new(vec![query])
        .map_err(|_| fixture_error("fact selection request is invalid"))?;

    Ok(FactSelectionFixtureContracts {
        execution: FixtureReadExecution {
            capability_operation_id: stable_id(FACT_SELECTION_OPERATION_ID)?,
            capability_binding_ref: binding_ref,
            request_contract: request_contract.clone(),
            returned_contract: response_contract.clone(),
            safe_failure_contract,
            support_members: vec![
                routing_member,
                scan_member,
                classifier_member,
                binding_member,
            ],
        },
        routing_generation_ref: routing_ref,
        request_contract,
        #[cfg(test)]
        response_contract,
        #[cfg(test)]
        attestation_contract,
        request,
    })
}

/// Genuine producer and reserved consumer inputs shared by memory and PostgreSQL tests.
pub struct FactScanConformanceFixture {
    producer: LegalAdmissionFixture,
    late_producer: LegalAdmissionFixture,
    consumer: LegalAdmissionFixture,
    request_contract: RetainedValueContract,
    routing_generation_ref: ContentRef,
    descriptor_ref: ContentRef,
    subject_contract: RetainedValueContract,
    response_value_contract: RetainedValueContract,
    subject: PlainCanonicalJsonBytes,
    request: FactSelectionRequest,
}

impl FactScanConformanceFixture {
    /// Constructs one three-fact producer and one reserved consumer in the same tenant.
    pub fn new(
        store_identity: StoreIdentity,
        tenant_scope_id: TenantScopeId,
        producer_discriminator: u8,
        late_producer_discriminator: u8,
        consumer_discriminator: u8,
    ) -> Result<Self> {
        let primitive_schema = RecoverabilityContractV2::embedded()?
            .schema_id("mfm.primitive-canonical_value.v1")?
            .clone();
        let descriptor_contract = retained_contract(
            primitive_schema.clone(),
            "bounded-fact-descriptor",
            "bounded-fact-descriptor",
            producer_discriminator,
            default_evidence_ref()?,
        )?;
        let descriptor_bytes = canonical_json(r#"{"kind":"bounded-fact"}"#)?;
        let (descriptor_member, descriptor_ref) =
            support_member("facts.descriptor", descriptor_bytes, descriptor_contract)?;
        let evidence_ref = default_evidence_ref()?;
        let (evidence_member, retained_evidence_ref) = support_member(
            "facts.object_evidence_contract",
            canonical_json(r#"{"version":"mfm.component-object-evidence-contract.v1"}"#)?,
            retained_contract(
                evidence_ref.schema_id().clone(),
                "component-object-evidence-contract",
                "component-object-evidence-contract",
                producer_discriminator,
                evidence_ref.clone(),
            )?,
        )?;
        if retained_evidence_ref != evidence_ref {
            return Err(fixture_error("fact evidence reference is inconsistent"));
        }
        let subject_contract = retained_contract(
            primitive_schema.clone(),
            "bounded-fact-subject",
            "bounded-fact-subject",
            producer_discriminator,
            default_evidence_ref()?,
        )?;
        let response_value_contract = retained_contract(
            primitive_schema,
            "bounded-fact-response",
            "bounded-fact-response",
            producer_discriminator,
            default_evidence_ref()?,
        )?;
        let fact_execution = FixtureFactExecution {
            descriptor_ref: descriptor_ref.clone(),
            subject_contract: subject_contract.clone(),
            response_contract: response_value_contract.clone(),
            support_members: vec![evidence_member, descriptor_member],
        };
        let producer = LegalAdmissionFixture::for_store_in_tenant(
            store_identity.clone(),
            tenant_scope_id.clone(),
            producer_discriminator,
        )?
        .with_fact_execution(fact_execution.clone());
        let late_producer = LegalAdmissionFixture::for_store_in_tenant(
            store_identity.clone(),
            tenant_scope_id.clone(),
            late_producer_discriminator,
        )?
        .with_fact_execution(fact_execution);

        let subject = canonical_json(r#"{"scope":"bounded-fact-scan"}"#)?;
        let fact_subject = FactSubject::from_canonical_json(subject.as_bytes())
            .map_err(|_| fixture_error("fact subject is invalid"))?;
        let contracts = fact_selection_contracts(
            consumer_discriminator,
            Some(&descriptor_ref),
            CanonicalFactPredicate::exact_subject(&fact_subject)
                .map_err(|_| fixture_error("fact predicate is invalid"))?,
            FactOrdering::Descending,
            3,
        )?;
        let consumer = LegalAdmissionFixture::for_store_in_tenant(
            store_identity,
            tenant_scope_id,
            consumer_discriminator,
        )?
        .with_read_execution(contracts.execution);

        Ok(Self {
            producer,
            late_producer,
            consumer,
            request_contract: contracts.request_contract,
            routing_generation_ref: contracts.routing_generation_ref,
            descriptor_ref,
            subject_contract,
            response_value_contract,
            subject,
            request: contracts.request,
        })
    }

    /// Returns the legal three-fact producer admission fixture.
    pub const fn producer(&self) -> &LegalAdmissionFixture {
        &self.producer
    }

    /// Returns the producer whose publication is committed above the fixed barrier.
    pub const fn late_producer(&self) -> &LegalAdmissionFixture {
        &self.late_producer
    }

    /// Returns the reserved fact-selection consumer admission fixture.
    pub const fn consumer(&self) -> &LegalAdmissionFixture {
        &self.consumer
    }

    /// Builds three genuine same-slot proposals with one common subject and distinct responses.
    fn fact_proposals(&self) -> Result<Vec<FactProposal>> {
        (0..3)
            .map(|ordinal| {
                let response = canonical_json(&format!(r#"{{"ordinal":{ordinal}}}"#))?;
                FactProposal::new(
                    0,
                    self.descriptor_ref.clone(),
                    proposed_value(&self.subject_contract, self.subject.clone())?,
                    proposed_value(&self.response_value_contract, response)?,
                )
                .map_err(|_| fixture_error("fact proposal is invalid"))
            })
            .collect()
    }

    /// Exercises the complete bounded-scan, attestation, settlement, and replay contract.
    ///
    /// The caller must provision all three configured values before invoking this helper.
    pub async fn verify_on<B>(
        &self,
        store: &B,
        issuer: &RunAccessAuthorityIssuer,
    ) -> std::result::Result<(), <B as RunJournalBackend>::Error>
    where
        B: FactSelectionStore
            + RunJournalStore<Error = <B as RunJournalBackend>::Error>
            + SupportBackend
            + ConfiguredValueBackend
            + AdmissionSourceBackend,
    {
        let producer_run_id = admit_fixture(store, issuer, &self.producer).await?;
        let late_producer_run_id = admit_fixture(store, issuer, &self.late_producer).await?;
        let consumer_run_id = admit_fixture(store, issuer, &self.consumer).await?;

        let producing_transition_ref = settle_fact_producer(
            store,
            issuer,
            &self.producer,
            &producer_run_id,
            self.fact_proposals()
                .map_err(<B as RunJournalBackend>::Error::from)?,
            1,
            "first",
        )
        .await?;

        let abandoned = authorize_fact_selection(
            store,
            issuer,
            self,
            &consumer_run_id,
            "fact-conformance-abandoned",
        )
        .await?;
        if abandoned
            .frontier
            .fields()
            .map_err(StoreError::from)
            .map_err(<B as RunJournalBackend>::Error::from)?
            .fact_order
            != 1
        {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }
        abandon_fact_scan_after_first_page(store, abandoned.permit, 2, 1, 2).await?;

        let consumer_drive = issuer.authorize_drive(
            self.consumer.tenant_scope_id().clone(),
            consumer_run_id.clone(),
        );
        let abandoned_view = store
            .load_committed_journal(&consumer_drive)
            .await?
            .verify_recorded_history()
            .map_err(<B as RunJournalBackend>::Error::from)?;
        let rows = store
            .backend_load_fact_attestations(FactAttestationLoadVerifier::new(
                abandoned_view.store_identity().clone(),
                abandoned_view.tenant_scope_id().clone(),
                abandoned_view.run_id().clone(),
            ))
            .await?;
        if !rows.is_empty() {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }

        let completed_authorization = authorize_fact_selection(
            store,
            issuer,
            self,
            &consumer_run_id,
            "fact-conformance-completed",
        )
        .await?;
        if completed_authorization.frontier != abandoned.frontier {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }
        let authorization_ref = completed_authorization.authorization_ref;
        let request_ref = completed_authorization.request_ref;
        let frontier = completed_authorization.frontier;

        settle_fact_producer(
            store,
            issuer,
            &self.late_producer,
            &late_producer_run_id,
            self.fact_proposals()
                .map_err(<B as RunJournalBackend>::Error::from)?,
            2,
            "late",
        )
        .await?;

        let completed =
            scan_fact_selection_with_fact_limit(store, completed_authorization.permit, 2).await?;
        if completed.authorization_ref() != &authorization_ref || completed.frontier() != &frontier
        {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }
        verify_three_fact_completion(&completed, &producing_transition_ref)
            .map_err(<B as RunJournalBackend>::Error::from)?;

        let before_observation = store
            .load_committed_journal(&consumer_drive)
            .await?
            .verify_recorded_history()
            .map_err(<B as RunJournalBackend>::Error::from)?;
        let rows = store
            .backend_load_fact_attestations(FactAttestationLoadVerifier::new(
                before_observation.store_identity().clone(),
                before_observation.tenant_scope_id().clone(),
                before_observation.run_id().clone(),
            ))
            .await?;
        if !rows.is_empty() {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }
        let observation = store
            .prepare_append(
                &consumer_drive,
                &before_observation,
                AppendRequestId::new("fact-conformance-observation")
                    .map_err(StoreError::from)
                    .map_err(<B as RunJournalBackend>::Error::from)?,
                completed.into_observation_material(),
            )
            .map_err(<B as RunJournalBackend>::Error::from)?;
        let observation_commit = match store.append(&consumer_drive, observation).await? {
            AppendOutcome::NewlyAppended(NewlyAppended::Observation(committed)) => committed,
            AppendOutcome::NewlyAppended(
                NewlyAppended::RunAdmitted(_)
                | NewlyAppended::Transition(_)
                | NewlyAppended::Authorization(_),
            )
            | AppendOutcome::AlreadyCommitted(_)
            | AppendOutcome::Rejected(_)
            | AppendOutcome::OutcomeUnknown => {
                return Err(<B as RunJournalBackend>::Error::from(
                    StoreError::FactScanBindingMismatch,
                ));
            }
        };
        let [observation_record_ref] = observation_commit.record_refs() else {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        };
        let observation_ref = ObservationRef::new(observation_record_ref)
            .map_err(StoreError::from)
            .map_err(<B as RunJournalBackend>::Error::from)?;

        let observed_view = store
            .load_committed_journal(&consumer_drive)
            .await?
            .verify_recorded_history()
            .map_err(<B as RunJournalBackend>::Error::from)?;
        verify_committed_fact_observation_modes(
            observed_view
                .journal()
                .commits()
                .last()
                .ok_or(StoreError::FactScanBindingMismatch)
                .map_err(<B as RunJournalBackend>::Error::from)?,
        )
        .map_err(<B as RunJournalBackend>::Error::from)?;
        let rows = store
            .backend_load_fact_attestations(FactAttestationLoadVerifier::new(
                observed_view.store_identity().clone(),
                observed_view.tenant_scope_id().clone(),
                observed_view.run_id().clone(),
            ))
            .await?;
        let [row] = rows.as_slice() else {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        };
        if row.authorization_ref() != &authorization_ref
            || row.observation_ref() != &observation_ref
            || row.containing_journal_head() != observation_commit.journal_head()
        {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }
        store
            .verify_drive_fact_selection_observations(
                &consumer_drive,
                &observed_view,
                std::slice::from_ref(&observation_ref),
            )
            .await?;

        let [node] = observed_view.certified_spec().nodes() else {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        };
        let [output_slot] = node.settlement_contract().output_slots() else {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        };
        let frame = store
            .prepare_frame(&consumer_drive, &observed_view, node.node_id())
            .await?;
        let settled = store
            .prepare_append(
                &consumer_drive,
                &observed_view,
                AppendRequestId::new("fact-conformance-read-settled")
                    .map_err(StoreError::from)
                    .map_err(<B as RunJournalBackend>::Error::from)?,
                ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::ReadSettled {
                    prepared_frame: Box::new(frame),
                    immutable_request_ref: request_ref,
                    consumed_observation_ref: observation_ref,
                    settlement: SettlementMaterial::Succeeded {
                        output_roots: vec![ProducedOutputSlot::new(
                            output_slot.output_ordinal(),
                            output_slot.field_path().clone(),
                            ProducedObjectRoot::new(
                                output_slot.value_contract().clone(),
                                canonical_json(r#"{"result":"fact-consumer"}"#)
                                    .map_err(<B as RunJournalBackend>::Error::from)?,
                            ),
                        )],
                        fact_roots: Vec::new(),
                    },
                    object_graph: ObjectGraphProposal::empty(),
                })),
            )
            .map_err(<B as RunJournalBackend>::Error::from)?;
        match store.append(&consumer_drive, settled).await? {
            AppendOutcome::NewlyAppended(NewlyAppended::Transition(_)) => {}
            AppendOutcome::NewlyAppended(
                NewlyAppended::RunAdmitted(_)
                | NewlyAppended::Authorization(_)
                | NewlyAppended::Observation(_),
            )
            | AppendOutcome::AlreadyCommitted(_)
            | AppendOutcome::Rejected(_)
            | AppendOutcome::OutcomeUnknown => {
                return Err(<B as RunJournalBackend>::Error::from(
                    StoreError::FactScanBindingMismatch,
                ));
            }
        }

        let settled_view = store
            .load_committed_journal(&consumer_drive)
            .await?
            .verify_recorded_history()
            .map_err(<B as RunJournalBackend>::Error::from)?;
        verify_middle_fact_omission_rejected(store, &settled_view).await?;
        let replay =
            issuer.authorize_replay(self.consumer.tenant_scope_id().clone(), consumer_run_id);
        let completeness = store
            .verify_fact_selection_completeness(&replay, &settled_view)
            .await?;
        let [claim] = completeness.fact_selections() else {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        };
        match claim
            .fields()
            .map_err(StoreError::from)
            .map_err(<B as RunJournalBackend>::Error::from)?
        {
            FactSelectionCompletenessFields::SameStoreVerified {
                authorization_ref: retained_authorization,
                frontier: retained_frontier,
            } if retained_authorization == authorization_ref && *retained_frontier == frontier => {
                Ok(())
            }
            FactSelectionCompletenessFields::SameStoreVerified { .. }
            | FactSelectionCompletenessFields::Unverified { .. } => Err(
                <B as RunJournalBackend>::Error::from(StoreError::FactScanBindingMismatch),
            ),
        }
    }
}

struct AuthorizedFactScan {
    permit: crate::v1::FactScanPermit,
    authorization_ref: mfm_journal::v1::AuthorizationRef,
    request_ref: ValueRef,
    frontier: TenantFactFrontier,
}

async fn admit_fixture<B>(
    store: &B,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &LegalAdmissionFixture,
) -> std::result::Result<RunId, <B as RunJournalBackend>::Error>
where
    B: FactSelectionStore
        + RunJournalStore<Error = <B as RunJournalBackend>::Error>
        + SupportBackend
        + ConfiguredValueBackend
        + AdmissionSourceBackend,
{
    let prepared = fixture.prepare_on(store, issuer).await?;
    let (authority, append) = prepared.into_parts();
    match store.append_admission(&authority, append).await? {
        AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) => {
            Ok(admitted.run_id().clone())
        }
        AppendOutcome::NewlyAppended(
            NewlyAppended::Transition(_)
            | NewlyAppended::Authorization(_)
            | NewlyAppended::Observation(_),
        )
        | AppendOutcome::AlreadyCommitted(_)
        | AppendOutcome::Rejected(_)
        | AppendOutcome::OutcomeUnknown => Err(<B as RunJournalBackend>::Error::from(
            StoreError::FactScanBindingMismatch,
        )),
    }
}

async fn settle_fact_producer<B>(
    store: &B,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &LegalAdmissionFixture,
    run_id: &RunId,
    fact_roots: Vec<FactProposal>,
    expected_fact_order: u64,
    label: &str,
) -> std::result::Result<TransitionRef, <B as RunJournalBackend>::Error>
where
    B: FactSelectionStore + RunJournalStore<Error = <B as RunJournalBackend>::Error>,
{
    let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = store
        .load_committed_journal(&drive)
        .await?
        .verify_recorded_history()
        .map_err(<B as RunJournalBackend>::Error::from)?;
    let [node] = view.certified_spec().nodes() else {
        return Err(<B as RunJournalBackend>::Error::from(
            StoreError::FactScanBindingMismatch,
        ));
    };
    let [first_output_slot, second_output_slot] = node.settlement_contract().output_slots() else {
        return Err(<B as RunJournalBackend>::Error::from(
            StoreError::FactScanBindingMismatch,
        ));
    };
    let frame = store.prepare_frame(&drive, &view, node.node_id()).await?;
    let append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("fact-conformance-{label}-settlement"))
                .map_err(StoreError::from)
                .map_err(<B as RunJournalBackend>::Error::from)?,
            ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                prepared_frame: Box::new(frame),
                settlement: SettlementMaterial::Succeeded {
                    output_roots: vec![
                        ProducedOutputSlot::new(
                            first_output_slot.output_ordinal(),
                            first_output_slot.field_path().clone(),
                            ProducedObjectRoot::new(
                                first_output_slot.value_contract().clone(),
                                canonical_json(&format!(r#"{{"result":"{label}-producer-z"}}"#))
                                    .map_err(<B as RunJournalBackend>::Error::from)?,
                            ),
                        ),
                        ProducedOutputSlot::new(
                            second_output_slot.output_ordinal(),
                            second_output_slot.field_path().clone(),
                            ProducedObjectRoot::new(
                                second_output_slot.value_contract().clone(),
                                canonical_json(&format!(r#"{{"result":"{label}-producer-a"}}"#))
                                    .map_err(<B as RunJournalBackend>::Error::from)?,
                            ),
                        ),
                    ],
                    fact_roots,
                },
                object_graph: ObjectGraphProposal::empty(),
            })),
        )
        .map_err(<B as RunJournalBackend>::Error::from)?;
    verify_prepared_fact_settlement_ordering(&append, expected_fact_order == 1)
        .map_err(<B as RunJournalBackend>::Error::from)?;
    let committed = match store.append(&drive, append).await? {
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(committed)) => committed,
        AppendOutcome::NewlyAppended(
            NewlyAppended::RunAdmitted(_)
            | NewlyAppended::Authorization(_)
            | NewlyAppended::Observation(_),
        )
        | AppendOutcome::AlreadyCommitted(_)
        | AppendOutcome::Rejected(_)
        | AppendOutcome::OutcomeUnknown => {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }
    };
    let fact_frontier = committed
        .fact_frontier()
        .ok_or(StoreError::FactScanBindingMismatch)
        .map_err(<B as RunJournalBackend>::Error::from)?;
    if fact_frontier
        .fields()
        .map_err(StoreError::from)
        .map_err(<B as RunJournalBackend>::Error::from)?
        .fact_order
        != expected_fact_order
    {
        return Err(<B as RunJournalBackend>::Error::from(
            StoreError::FactScanBindingMismatch,
        ));
    }
    let closed = store
        .load_committed_journal(&drive)
        .await?
        .verify_recorded_history()
        .map_err(<B as RunJournalBackend>::Error::from)?;
    let output_ordinals = closed
        .node_outputs(node.node_id())
        .iter()
        .map(|binding| binding.fields().map(|fields| fields.output_ordinal))
        .collect::<mfm_journal::v1::Result<Vec<_>>>()
        .map_err(StoreError::from)
        .map_err(<B as RunJournalBackend>::Error::from)?;
    let fact_ordinals = closed
        .node_facts(node.node_id())
        .iter()
        .map(|emission| emission.fields().map(|fields| fields.emission_ordinal))
        .collect::<mfm_journal::v1::Result<Vec<_>>>()
        .map_err(StoreError::from)
        .map_err(<B as RunJournalBackend>::Error::from)?;
    if output_ordinals != [0, 1] || fact_ordinals != [0, 1, 2] {
        return Err(<B as RunJournalBackend>::Error::from(
            StoreError::FactScanBindingMismatch,
        ));
    }
    closed
        .node_terminal_transition_ref(node.node_id())
        .cloned()
        .ok_or(StoreError::FactScanBindingMismatch)
        .map_err(<B as RunJournalBackend>::Error::from)
}

fn verify_prepared_fact_settlement_ordering(
    append: &PreparedJournalAppend,
    require_fact_wire_reordering: bool,
) -> Result<()> {
    let PreparedJournalAppend::CommitTransition(append) = append else {
        return Err(StoreError::FactScanBindingMismatch);
    };
    let transition = append.transition().fields()?;
    let TransitionBodyFields::PureSettled { settlement, .. } = transition.body.fields()? else {
        return Err(StoreError::FactScanBindingMismatch);
    };
    let SettlementFields::Succeeded {
        output_bindings,
        fact_emissions,
    } = settlement.fields()?
    else {
        return Err(StoreError::FactScanBindingMismatch);
    };
    let semantic_output_ordinals = output_bindings
        .iter()
        .map(|binding| binding.fields().map(|fields| fields.output_ordinal))
        .collect::<mfm_journal::v1::Result<Vec<_>>>()?;
    let semantic_fact_ordinals = fact_emissions
        .iter()
        .map(|emission| emission.fields().map(|fields| fields.emission_ordinal))
        .collect::<mfm_journal::v1::Result<Vec<_>>>()?;
    if semantic_output_ordinals != [0, 1] || semantic_fact_ordinals != [0, 1, 2] {
        return Err(StoreError::FactScanBindingMismatch);
    }

    let mut wire_output_ordinals = Vec::new();
    let mut wire_fact_ordinals = Vec::new();
    for entry in transition.after.fields()?.binding_delta.entries()? {
        match entry.fields()? {
            BindingDeltaEntryFields::OutputBinding(binding) => {
                wire_output_ordinals.push(binding.fields()?.output_ordinal);
            }
            BindingDeltaEntryFields::FactBinding(emission) => {
                wire_fact_ordinals.push(emission.fields()?.emission_ordinal);
            }
            BindingDeltaEntryFields::NodePhaseChange { .. }
            | BindingDeltaEntryFields::PendingEffectInsert { .. }
            | BindingDeltaEntryFields::PendingEffectRemove { .. }
            | BindingDeltaEntryFields::PublicOutputChange(_)
            | BindingDeltaEntryFields::RunPhaseChange(_) => {}
        }
    }
    if wire_output_ordinals != [1, 0]
        || wire_fact_ordinals.len() != 3
        || (require_fact_wire_reordering && wire_fact_ordinals == [0, 1, 2])
    {
        return Err(StoreError::FactScanBindingMismatch);
    }
    Ok(())
}

fn verify_committed_fact_observation_modes(commit: &CommittedJournalCommit) -> Result<()> {
    let envelope = commit.envelope().fields()?;
    let bound = envelope
        .core
        .ordered_object_bindings
        .iter()
        .map(|binding| {
            binding
                .fields()
                .map(|fields| fields.value_ref.as_bytes().to_vec())
        })
        .collect::<mfm_journal::v1::Result<BTreeSet<_>>>()?;
    let mut products = 0;
    let mut imports = 0;
    for intent in envelope.core.artifact_admission_intents {
        let fields = intent.fields()?;
        let is_bound = bound.contains(fields.value_ref.as_bytes());
        match (fields.mode, is_bound) {
            (ArtifactAdmissionMode::AdmitOrVerifyExact, true) => products += 1,
            (ArtifactAdmissionMode::RequireExisting, false) => imports += 1,
            (ArtifactAdmissionMode::RequireExisting, true)
            | (ArtifactAdmissionMode::AdmitOrVerifyExact, false) => {
                return Err(StoreError::FactScanBindingMismatch);
            }
        }
    }
    if products != 2 || imports == 0 {
        return Err(StoreError::FactScanBindingMismatch);
    }
    Ok(())
}

async fn authorize_fact_selection<B>(
    store: &B,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &FactScanConformanceFixture,
    run_id: &RunId,
    append_request_id: &str,
) -> std::result::Result<AuthorizedFactScan, <B as RunJournalBackend>::Error>
where
    B: FactSelectionStore + RunJournalStore<Error = <B as RunJournalBackend>::Error>,
{
    let drive = issuer.authorize_drive(fixture.consumer.tenant_scope_id().clone(), run_id.clone());
    let view = store
        .load_committed_journal(&drive)
        .await?
        .verify_recorded_history()
        .map_err(<B as RunJournalBackend>::Error::from)?;
    let [node] = view.certified_spec().nodes() else {
        return Err(<B as RunJournalBackend>::Error::from(
            StoreError::FactScanBindingMismatch,
        ));
    };
    let frame = store.prepare_frame(&drive, &view, node.node_id()).await?;
    let append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(append_request_id)
                .map_err(StoreError::from)
                .map_err(<B as RunJournalBackend>::Error::from)?,
            ExistingRunAppendMaterial::Authorization(Box::new(AuthorizationMaterial::Read {
                prepared_frame: Box::new(frame),
                immutable_request_root: Box::new(ProducedObjectRoot::new(
                    fixture.request_contract.clone(),
                    PlainCanonicalJsonBytes::from_canonical_json_slice(
                        fixture.request.canonical_json(),
                    )
                    .map_err(|_| StoreError::JournalContract)
                    .map_err(<B as RunJournalBackend>::Error::from)?,
                )),
                routing_generation_ref: fixture.routing_generation_ref.clone(),
            })),
        )
        .map_err(<B as RunJournalBackend>::Error::from)?;
    let PreparedJournalAppend::AuthorizeExternalAccess(append) = append else {
        return Err(<B as RunJournalBackend>::Error::from(
            StoreError::FactScanBindingMismatch,
        ));
    };
    let permit = match store
        .append_fact_selection_authorization(&drive, append, fixture.request.clone())
        .await?
    {
        FactSelectionAuthorizationOutcome::NewlyAuthorized(permit) => *permit,
        FactSelectionAuthorizationOutcome::AlreadyCommitted(_)
        | FactSelectionAuthorizationOutcome::Rejected(_)
        | FactSelectionAuthorizationOutcome::OutcomeUnknown => {
            return Err(<B as RunJournalBackend>::Error::from(
                StoreError::FactScanBindingMismatch,
            ));
        }
    };
    let authorization_ref = permit.authorization_ref().clone();
    let request_ref = permit
        .authorization()
        .fields()
        .map_err(StoreError::from)
        .map_err(<B as RunJournalBackend>::Error::from)?
        .request_ref;
    let frontier = permit.frontier().clone();
    Ok(AuthorizedFactScan {
        permit,
        authorization_ref,
        request_ref,
        frontier,
    })
}

fn proposed_value(
    contract: &RetainedValueContract,
    canonical: PlainCanonicalJsonBytes,
) -> Result<ProposedFactValue> {
    ProposedFactValue::new(
        contract.schema_id().clone(),
        contract.semantic_type_id().clone(),
        contract.role().clone(),
        contract.media_type(),
        contract.evidence_contract_ref().clone(),
        canonical,
    )
    .map_err(|_| fixture_error("proposed fact value is invalid"))
}

fn retained_contract(
    schema_id: mfm_ids::SchemaId,
    semantic_name: &str,
    role: &str,
    discriminator: u8,
    evidence_contract_ref: ContentRef,
) -> Result<RetainedValueContract> {
    RetainedValueContract::new(
        schema_id,
        semantic_type(semantic_name, discriminator)?,
        stable_id(role)?,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|_| fixture_error("fact retained-value contract is invalid"))
}

fn default_evidence_ref() -> Result<ContentRef> {
    Ok(EntryPointContract::retained_contract()?
        .evidence_contract_ref()
        .clone())
}

fn support_member(
    path: &str,
    canonical: PlainCanonicalJsonBytes,
    contract: RetainedValueContract,
) -> Result<(QualifiedSupportMember, ContentRef)> {
    let reference = content_ref(&contract, canonical.as_bytes())?;
    Ok((
        QualifiedSupportMember::new(field_path(path)?, canonical, contract),
        reference,
    ))
}
