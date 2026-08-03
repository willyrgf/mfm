use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, InvocationIdentity, RunId,
    SchemaId, SchemaVersion, SemanticTypeId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    CommitCandidate, HistoryObject, LexicalValueRef, ObservationOutcome,
    PriorRunFactSourceManifest, RunClosed, RunRecord, StateOutcomeRef, StateTransitionCommitted,
    TenantFactCoordinate, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_spec::structured::{
    fan_out_join_contract_canonical_json, fan_out_join_contract_ref,
    lane_outcome_contract_canonical_json, lane_outcome_contract_ref, never_failure_contract_ref,
    retained_value_contract_ref, BlockTail, CertifiedComponentObject, CertifiedFailureBoundary,
    CertifiedProgramComponents, CertifiedProgramDocument, CertifiedProgramRoot,
    CertifiedStructuralBounds, ClosedSumContract, ClosedSumPayload, ClosedSumVariant,
    ExpandedBlock, ExpandedDeclaration, ExpandedFanOut, ExpandedFanOutLane, ExpandedMatch,
    ExpandedMatchArm, ExpandedStateBinding, ExpandedStructuredProgram, FailurePlan,
    FailurePlanIdentity, FailureScope, FailureScopeBinding, HandlerContinuation, LexicalProducer,
    LexicalSlot, NoFailureBoundary, ResultRole, SecretFreeImplementationManifest,
    SecretFreeImplementationManifestEntry, SemanticCallPath, SemanticPathSegment, StructuralPath,
    StructuralPathSegment, StructuredComponentKind, StructuredExecutionKind,
    StructuredFailureContract, StructuredLiveComponentContract, StructuredPublicContractRefs,
    StructuredSafeFailureDispositionContract, StructuredStateContract,
    StructuredStateExecutionContract,
};
use mfm_spec::CanonicalJsonValue;
use mfm_values::{
    EnumTagging, EnumVariantDescriptor, FieldDescriptor, RetainedValueContract, SchemaIdentity,
    SchemaKind, SchemaShape,
};

use super::backend::StructuredRunStore;
use super::fold::ProgramVerifier;
use super::*;

#[derive(Clone)]
struct FixtureProgramVerifier {
    entry_point: StableId,
    document: CertifiedProgramDocument,
    expanded: ExpandedStructuredProgram,
    value_schemas: BTreeMap<ContentRef, SchemaIdentity>,
}

impl ProgramVerifier for FixtureProgramVerifier {
    fn verify(
        &self,
        entry_point_id: &StableId,
        root: &CertifiedProgramRoot,
        _authored: &mfm_spec::CanonicalJsonValue,
    ) -> std::result::Result<Arc<VerifiedProgramData>, StructuredStoreError> {
        if entry_point_id != &self.entry_point || root != &self.document.root {
            return Err(StructuredStoreError::Certification);
        }
        Ok(Arc::new(VerifiedProgramData::new(
            self.document.clone(),
            self.expanded.clone(),
            self.value_schemas.clone(),
        )))
    }
}

struct NoPhysicalBindings;

impl PublicPhysicalBindingVerifier for NoPhysicalBindings {
    fn verify_authorization(
        &self,
        _context: &PhysicalBindingAuthorization<'_>,
        _certificate: &mfm_journal::structured::HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Err(StructuredStoreError::Certification)
    }

    fn verify_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &mfm_journal::structured::HistoryObject,
        _evidence: &mfm_journal::structured::HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Err(StructuredStoreError::Certification)
    }
}

struct AcceptPhysicalBindings;

impl PublicPhysicalBindingVerifier for AcceptPhysicalBindings {
    fn verify_authorization(
        &self,
        _context: &PhysicalBindingAuthorization<'_>,
        _certificate: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Ok(())
    }

    fn verify_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum PositiveReply {
    ExactNew,
    ExactExisting,
    SubstitutedNew,
    SubstitutedExisting,
}

struct PositiveReplyBackend {
    identity: StructuredStoreIdentity,
    reply: PositiveReply,
}

impl StructuredHistoryBackend for PositiveReplyBackend {
    fn identity(&self) -> &StructuredStoreIdentity {
        &self.identity
    }

    fn load<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async { Ok(None) })
    }

    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, mfm_journal::structured::TenantFactFrontier> {
        Box::pin(async move {
            Ok(mfm_journal::structured::TenantFactFrontier::new(
                self.identity.store_scope_id.clone(),
                self.identity.store_epoch,
                tenant_scope_id.clone(),
                0,
            ))
        })
    }

    fn scan_fact_publications<'a>(
        &'a self,
        _tenant_scope_id: &'a TenantScopeId,
        _first_order: u64,
        _through_order: u64,
        _maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn append<'a>(
        &'a self,
        batch: ValidatedBatch,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            let mut returned = batch.into_committed();
            if matches!(
                self.reply,
                PositiveReply::SubstitutedNew | PositiveReply::SubstitutedExisting
            ) {
                returned.append_request_id =
                    AppendRequestId::new("backend-substitution").expect("substituted append id");
            }
            Ok(match self.reply {
                PositiveReply::ExactNew | PositiveReply::SubstitutedNew => {
                    BackendAppendOutcome::NewlyCommitted(returned)
                }
                PositiveReply::ExactExisting | PositiveReply::SubstitutedExisting => {
                    BackendAppendOutcome::ExistingSame(returned)
                }
            })
        })
    }

    fn resolve_append<'a>(
        &'a self,
        _run_id: &'a RunId,
        _append_request_id: &'a AppendRequestId,
        _candidate_digest: &'a ContentDigest,
    ) -> StructuredBackendFuture<'a, Option<mfm_journal::structured::CommittedBatch>> {
        Box::pin(async { Ok(None) })
    }
}

#[derive(Clone, Copy)]
enum ResolutionCorruption {
    WrongStoreIdentity,
    MalformedEnvelope,
}

struct CorruptResolutionBackend {
    inner: StructuredMemoryBackend,
    corruption: ResolutionCorruption,
}

impl StructuredHistoryBackend for CorruptResolutionBackend {
    fn identity(&self) -> &StructuredStoreIdentity {
        self.inner.identity()
    }

    fn load<'a>(&'a self, run_id: &'a RunId) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        self.inner.load(run_id)
    }

    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, mfm_journal::structured::TenantFactFrontier> {
        self.inner.tenant_fact_frontier(tenant_scope_id)
    }

    fn scan_fact_publications<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        self.inner.scan_fact_publications(
            tenant_scope_id,
            first_order,
            through_order,
            maximum_items,
        )
    }

    fn append<'a>(
        &'a self,
        batch: ValidatedBatch,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome> {
        self.inner.append(batch)
    }

    fn resolve_append<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
        candidate_digest: &'a ContentDigest,
    ) -> StructuredBackendFuture<'a, Option<mfm_journal::structured::CommittedBatch>> {
        Box::pin(async move {
            let Some(mut batch) = self
                .inner
                .resolve_append(run_id, append_request_id, candidate_digest)
                .await?
            else {
                return Ok(None);
            };
            match self.corruption {
                ResolutionCorruption::WrongStoreIdentity => {
                    batch.store_scope_id =
                        StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "f".repeat(32)))
                            .expect("foreign store scope");
                }
                ResolutionCorruption::MalformedEnvelope => {
                    batch.head.run_sequence = batch.head.run_sequence.saturating_add(1);
                }
            }
            Ok(Some(batch))
        })
    }
}

struct Fixture {
    entry_point: StableId,
    document: CertifiedProgramDocument,
    expanded: ExpandedStructuredProgram,
    input: LexicalSlot,
    value_schema: SchemaIdentity,
    extra_value_schemas: BTreeMap<ContentRef, SchemaIdentity>,
}

struct HandledFailureRoleFixture {
    fixture: Fixture,
    pre: ExpandedStateBinding,
    handler: ExpandedStateBinding,
    normal_continuation: ExpandedStateBinding,
}

struct PropagatingFailurePostFixture {
    fixture: Fixture,
    post: ExpandedStateBinding,
    normal_continuation: ExpandedStateBinding,
}

#[tokio::test]
async fn backend_positive_replies_cannot_substitute_store_validated_content() {
    for (discriminator, reply) in [
        (10, PositiveReply::SubstitutedNew),
        (11, PositiveReply::SubstitutedExisting),
    ] {
        let fixture = zero_state_fixture(discriminator);
        let store = StructuredRunStore::new(
            PositiveReplyBackend {
                identity: store_identity(discriminator),
                reply,
            },
            Arc::new(verifier(&fixture)),
            Arc::new(NoPhysicalBindings),
        );
        let (writer, _reader) = store.split();
        assert_eq!(
            writer
                .admit_run(admission(
                    &fixture,
                    run_id(discriminator),
                    "substituted-positive-reply",
                ))
                .await
                .expect_err("substituted backend content must fail closed"),
            StructuredStoreError::InvalidHistory
        );
    }
}

#[tokio::test]
async fn exact_backend_positive_replies_are_normalized_from_the_retained_candidate() {
    for (discriminator, reply) in [
        (12, PositiveReply::ExactNew),
        (13, PositiveReply::ExactExisting),
    ] {
        let fixture = zero_state_fixture(discriminator);
        let store = StructuredRunStore::new(
            PositiveReplyBackend {
                identity: store_identity(discriminator),
                reply,
            },
            Arc::new(verifier(&fixture)),
            Arc::new(NoPhysicalBindings),
        );
        let (writer, _reader) = store.split();
        let attempt = writer
            .admit_run(admission(
                &fixture,
                run_id(discriminator),
                "exact-positive-reply",
            ))
            .await
            .expect("exact positive backend reply");
        assert!(attempt.committed().is_some());
    }
}

#[tokio::test]
async fn ambiguity_resolution_rejects_wrong_identity_and_malformed_envelopes() {
    for (discriminator, corruption) in [
        (14, ResolutionCorruption::WrongStoreIdentity),
        (15, ResolutionCorruption::MalformedEnvelope),
    ] {
        let fixture = zero_state_fixture(discriminator);
        let inner = StructuredMemoryBackend::new(store_identity(discriminator));
        inner
            .acknowledge_next_commit_as_unknown()
            .expect("inject acknowledgement loss");
        let store = StructuredRunStore::new(
            CorruptResolutionBackend { inner, corruption },
            Arc::new(verifier(&fixture)),
            Arc::new(NoPhysicalBindings),
        );
        let (writer, _reader) = store.split();
        let attempt = writer
            .admit_run(admission(
                &fixture,
                run_id(discriminator),
                "corrupt-resolution",
            ))
            .await
            .expect("ambiguous append");
        assert!(matches!(
            attempt.outcome(),
            BackendAppendOutcome::AcknowledgementUnknown
        ));
        assert!(matches!(
            writer
                .resolve_append(
                    &run_id(discriminator),
                    attempt.append_request_id(),
                    attempt.candidate_digest(),
                )
                .await,
            Err(StructuredStoreError::InvalidHistory)
        ));
    }
}

#[tokio::test]
async fn zero_state_admission_closes_atomically_and_resolves_lost_acknowledgement() {
    let fixture = zero_state_fixture(1);
    let backend = StructuredMemoryBackend::new(store_identity(1));
    backend
        .acknowledge_next_commit_as_unknown()
        .expect("inject acknowledgement loss");
    let store = StructuredRunStore::new(
        backend.clone(),
        Arc::new(verifier(&fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let run_id = run_id(1);
    let attempt = writer
        .admit_run(admission(&fixture, run_id.clone(), "admit-zero"))
        .await
        .expect("admit zero-state run");
    assert!(matches!(
        attempt.outcome(),
        BackendAppendOutcome::AcknowledgementUnknown
    ));

    let resolved = writer
        .resolve_append(
            &run_id,
            attempt.append_request_id(),
            attempt.candidate_digest(),
        )
        .await
        .expect("resolve append")
        .expect("committed append");
    assert_eq!(resolved.records.len(), 2);
    assert!(matches!(
        resolved.records[0].record,
        RunRecord::RunAdmitted(_)
    ));
    assert!(matches!(
        resolved.records[1].record,
        RunRecord::RunClosed(_)
    ));
    let RunRecord::RunAdmitted(admission_record) = &resolved.records[0].record else {
        unreachable!("admission record shape was checked")
    };
    let root_object = resolved
        .objects
        .iter()
        .find(|object| object.content_ref == admission_record.certified_program_root_ref)
        .expect("separately persisted certified root");
    assert_eq!(
        root_object.object_type.as_str(),
        "structured.certified_program_root"
    );
    assert_ne!(
        admission_record.certified_program_root_ref,
        admission_record.certified_program_ref
    );
    assert!(resolved.objects.iter().all(|object| {
        object.object_type.as_str() != "structured.certified_program_document"
            && object.canonical_json.len() < 16_777_216
    }));
    assert!(fixture.document.component_closure.iter().all(|component| {
        resolved
            .objects
            .iter()
            .any(|object| object.content_ref == component.content_ref)
    }));

    let verified = reader.load_verified(&run_id).await.expect("verified run");
    assert_eq!(verified.frontier(), &StructuredFrontier::Complete);
    assert!(matches!(verified.cursor(), ProgramCursor::Closed { .. }));

    let retry = writer
        .admit_run(admission(&fixture, run_id, "admit-zero"))
        .await
        .expect("retry exact admission");
    assert!(matches!(
        retry.outcome(),
        BackendAppendOutcome::ExistingSame(_)
    ));
}

#[tokio::test]
async fn pure_transition_and_root_closure_share_one_atomic_append() {
    let fixture = one_state_fixture(2);
    let store = StructuredRunStore::new(
        StructuredMemoryBackend::new(store_identity(2)),
        Arc::new(verifier(&fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let run_id = run_id(2);
    let admitted = writer
        .admit_run(admission(&fixture, run_id.clone(), "admit-state"))
        .await
        .expect("admit one-state run");
    let admission_batch = admitted.committed().expect("known admission");
    assert_eq!(admission_batch.records.len(), 1);

    let transition = StateTransitionProposal::success(
        AppendRequestId::new("settle-state").expect("append id"),
        ProposedCanonicalValue::from_json("8").expect("output"),
        mfm_facts::FactSet::empty(),
    );
    let committed = writer
        .commit_state_transition(
            writer.load_verified(&run_id).await.expect("open run"),
            &transition,
        )
        .await
        .expect("commit transition");
    let batch = committed.committed().expect("known transition");
    assert_eq!(batch.records.len(), 2);
    assert!(matches!(
        batch.records[0].record,
        RunRecord::StateTransitionCommitted(_)
    ));
    assert!(matches!(batch.records[1].record, RunRecord::RunClosed(_)));

    let verified = reader.load_verified(&run_id).await.expect("verified run");
    assert_eq!(verified.frontier(), &StructuredFrontier::Complete);
    assert_eq!(verified.journal_head(), &batch.head);
}

#[tokio::test]
async fn never_occurrences_reject_proposed_and_forged_failures_for_every_execution_kind() {
    for (discriminator, execution) in [
        (20, StructuredExecutionKind::Pure),
        (21, StructuredExecutionKind::Read),
        (22, StructuredExecutionKind::Effect),
    ] {
        let fixture = one_state_fixture_with_execution(discriminator, execution);
        let identity = store_identity(discriminator);
        let backend = StructuredMemoryBackend::new(identity.clone());
        let store = StructuredRunStore::new(
            backend.clone(),
            Arc::new(verifier(&fixture)),
            Arc::new(AcceptPhysicalBindings),
        );
        let (writer, reader) = store.split();
        let run_id = run_id(discriminator);
        writer
            .admit_run(admission(
                &fixture,
                run_id.clone(),
                "never-matrix-admission",
            ))
            .await
            .unwrap_or_else(|error| panic!("Never admission {discriminator}: {error:?}"));
        let verified = reader.load_verified(&run_id).await.expect("Never run");
        let StructuredFrontier::Actions(actions) = verified.frontier() else {
            panic!("one Never state must be actionable")
        };
        let [action] = actions.as_slice() else {
            panic!("one Never state action")
        };
        let action = action.clone();

        assert_eq!(
            writer
                .commit_state_transition(
                    verified,
                    &StateTransitionProposal::failure(
                        AppendRequestId::new(format!("never-failure-{discriminator}"))
                            .expect("failure append id"),
                        ProposedCanonicalValue::from_json("9").expect("failure value"),
                    ),
                )
                .await
                .expect_err("Never callback proposal must be rejected before append"),
            StructuredStoreError::CandidateRejected
        );
        let mut verified = reader
            .load_verified(&run_id)
            .await
            .expect("unchanged Never run");
        if execution != StructuredExecutionKind::Pure {
            let authorization = writer
                .authorize_access(
                    verified,
                    &AccessAuthorizationProposal::new(
                        AppendRequestId::new(format!("never-authorize-{discriminator}"))
                            .expect("authorization append id"),
                        action.input.clone(),
                        ProposedCanonicalValue::from_json("7").expect("access request"),
                        admission_object(
                            "fixture.physical-binding",
                            "fixture.physical-binding",
                            discriminator,
                        ),
                    ),
                )
                .await
                .expect("valid access authorization");
            let (authorization, authorized) = authorization
                .into_newly_appended_authorization()
                .expect("new authorization permit");
            let raw_after_authorization = backend
                .load(&run_id)
                .await
                .expect("raw authorized Never run")
                .expect("authorized Never run");
            assert_eq!(
                writer
                    .authorize_access(
                        reader
                            .load_verified(&run_id)
                            .await
                            .expect("authorized Never run"),
                        &AccessAuthorizationProposal::new(
                            AppendRequestId::new(format!(
                                "never-second-authorization-{discriminator}"
                            ))
                            .expect("second authorization append id"),
                            action.input.clone(),
                            ProposedCanonicalValue::from_json("7").expect("second request"),
                            admission_object(
                                "fixture.physical-binding",
                                "fixture.physical-binding",
                                discriminator,
                            ),
                        ),
                    )
                    .await
                    .expect_err("a second unresolved access must be rejected before append"),
                StructuredStoreError::CandidateRejected
            );
            assert_eq!(
                backend
                    .load(&run_id)
                    .await
                    .expect("raw Never run after rejected second authorization")
                    .expect("authorized Never run"),
                raw_after_authorization
            );
            let observation = writer
                .commit_observation(
                    authorized,
                    &AccessObservationProposal::new(
                        AppendRequestId::new(format!("never-observe-{discriminator}"))
                            .expect("observation append id"),
                        authorization.authorization_ref().clone(),
                        ProposedObservationOutcome::Returned(
                            ProposedCanonicalValue::from_json("8").expect("returned observation"),
                        ),
                    ),
                )
                .await
                .expect("valid access observation");
            verified = match observation {
                ObservationCommit::ExistingSame(verified) => *verified,
                ObservationCommit::Attempt(attempt) => {
                    let (_, verified) = (*attempt)
                        .into_committed_successor()
                        .expect("committed observation successor");
                    verified
                }
            };
        }
        let StructuredFrontier::Actions(actions) = verified.frontier() else {
            panic!("settleable Never state must remain actionable")
        };
        let [settleable] = actions.as_slice() else {
            panic!("settleable Never fixture must have one action")
        };
        let settleable = settleable.clone();
        let consumed_observation_ref = match &settleable.leaf {
            StateLeaf::Ready if execution == StructuredExecutionKind::Pure => None,
            StateLeaf::ObservedForSettlement {
                observation_ref, ..
            } => Some(observation_ref.clone()),
            leaf => panic!("unexpected Never settlement leaf: {leaf:?}"),
        };
        let mut raw = backend
            .load(&run_id)
            .await
            .expect("raw Never run")
            .expect("admitted Never run");
        assert_eq!(
            raw.batches.len(),
            if execution == StructuredExecutionKind::Pure {
                1
            } else {
                3
            },
            "candidate rejection must append nothing before valid access prefixes"
        );

        let transition = StateTransitionCommitted {
            occurrence_id: settleable.occurrence_id,
            occurrence_path_ref: settleable
                .occurrence_path
                .content_ref()
                .expect("occurrence path ref"),
            semantic_call_id: settleable.semantic_call_id,
            input: settleable.input.clone(),
            consumed_observation_ref,
            outcome_ref: settleable.input.value.value_ref.clone(),
            outcome: StateOutcomeRef::Failure(settleable.input),
            facts: Vec::new(),
            before_semantic_state_digest: verified.semantic_head().semantic_state_digest().clone(),
            after_semantic_state_digest: verified.semantic_head().semantic_state_digest().clone(),
        };
        let forged = super::fold::assign_candidate(
            &identity,
            CommitCandidate {
                run_id: run_id.clone(),
                expected_head: Some(raw.batches.last().expect("Never prefix").head.clone()),
                append_request_id: AppendRequestId::new(format!(
                    "forged-never-failure-{discriminator}"
                ))
                .expect("forged append id"),
                tenant_fact_coordinate: TenantFactCoordinate::None,
                records: vec![RunRecord::StateTransitionCommitted(transition)],
                objects: Vec::new(),
            },
        )
        .expect("well-formed hostile envelope");
        raw.batches.push(forged);
        assert_eq!(
            super::fold::verify_recorded_history(raw, &verifier(&fixture), &AcceptPhysicalBindings,)
                .expect_err("persisted Never failure must fail closed"),
            StructuredStoreError::InvalidHistory
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootProvenanceCase {
    AdmissionRoot,
    StateOutput,
    ArmValue,
    VariantPayload,
    FragmentInput,
    FragmentBoundary,
    FanOutJoin,
}

#[tokio::test]
async fn persisted_root_outcomes_bind_every_applicable_provenance_exactly() {
    let cases = [
        RootProvenanceCase::AdmissionRoot,
        RootProvenanceCase::StateOutput,
        RootProvenanceCase::ArmValue,
        RootProvenanceCase::VariantPayload,
        RootProvenanceCase::FragmentInput,
        RootProvenanceCase::FragmentBoundary,
        RootProvenanceCase::FanOutJoin,
    ];
    let mut discriminator = 80_u8;
    for case in cases {
        for closes_as_failure in [false, true] {
            if case == RootProvenanceCase::FanOutJoin && closes_as_failure {
                continue;
            }
            let root = root_provenance_fixture(discriminator, case, closes_as_failure);
            let identity = store_identity(discriminator);
            let backend = StructuredMemoryBackend::new(identity.clone());
            let store = StructuredRunStore::new(
                backend.clone(),
                Arc::new(verifier(&root.fixture)),
                Arc::new(NoPhysicalBindings),
            );
            let (writer, reader) = store.split();
            let run_id = run_id(discriminator);
            writer
                .admit_run(admission_with_json(
                    &root.fixture,
                    run_id.clone(),
                    "root-provenance-admission",
                    root.input_json,
                ))
                .await
                .unwrap_or_else(|error| {
                    panic!("valid {case:?} admission root {discriminator}: {error:?}")
                });
            if root.requires_transition {
                writer
                    .commit_state_transition(
                        reader
                            .load_verified(&run_id)
                            .await
                            .expect("open provenance run"),
                        &StateTransitionProposal::success(
                            AppendRequestId::new("root-provenance-transition")
                                .expect("transition append id"),
                            ProposedCanonicalValue::from_json("8").expect("state output value"),
                            mfm_facts::FactSet::empty(),
                        ),
                    )
                    .await
                    .expect("valid provenance transition");
            }
            let verified = reader
                .load_verified(&run_id)
                .await
                .expect("valid closed provenance run");
            assert_eq!(verified.frontier(), &StructuredFrontier::Complete);
            let raw = backend
                .load(&run_id)
                .await
                .expect("raw provenance history")
                .expect("persisted provenance history");
            if case == RootProvenanceCase::FanOutJoin {
                let [admission_batch] = raw.batches.as_slice() else {
                    panic!("admission-only fan-out must close in one batch");
                };
                assert_eq!(admission_batch.records.len(), 2);
                assert!(matches!(
                    admission_batch.records[0].record,
                    RunRecord::RunAdmitted(_)
                ));
                assert!(matches!(
                    admission_batch.records[1].record,
                    RunRecord::RunClosed(_)
                ));
                assert!(admission_batch.records.iter().all(|assigned| matches!(
                    assigned.record,
                    RunRecord::RunAdmitted(_) | RunRecord::RunClosed(_)
                )));
                let lane_objects = admission_batch
                    .objects
                    .iter()
                    .filter(|object| object.canonical_json.as_str() == r#"{"Success":7}"#)
                    .collect::<Vec<_>>();
                // Payload objects remain content-addressed; lane provenance is
                // carried by distinct LexicalValueRef.structural_origin values.
                assert_eq!(
                    lane_objects.len(),
                    1,
                    "two identical lane payloads share one content-addressed wrapper object"
                );
                let lane_origins = verified
                    .live_bindings()
                    .filter_map(|binding| binding.structural_origin.as_ref())
                    .filter(|origin| {
                        matches!(
                            origin,
                            mfm_journal::structured::StructuralValueOrigin::FanOutLane { .. }
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    lane_origins.len(),
                    2,
                    "each completed lane retains a structural origin"
                );
                match (lane_origins[0], lane_origins[1]) {
                    (
                        mfm_journal::structured::StructuralValueOrigin::FanOutLane {
                            lane_ordinal: first_ordinal,
                            source_value_ref: first_value,
                            ..
                        },
                        mfm_journal::structured::StructuralValueOrigin::FanOutLane {
                            lane_ordinal: second_ordinal,
                            source_value_ref: second_value,
                            ..
                        },
                    ) => {
                        assert_ne!(
                            first_ordinal, second_ordinal,
                            "identical payloads from distinct lanes keep distinct ordinals"
                        );
                        assert_eq!(
                            first_value, second_value,
                            "byte-identical lane payloads share content identity"
                        );
                    }
                    _ => panic!("expected FanOutLane structural origins"),
                }
                let join_objects = admission_batch
                    .objects
                    .iter()
                    .filter(|object| {
                        object.canonical_json.as_str()
                            == r#"{"head":{"Success":7},"tail":[{"Success":7}]}"#
                    })
                    .collect::<Vec<_>>();
                assert_eq!(join_objects.len(), 1, "one exact fan-out join object");
                for (attack, omitted_ref) in [
                    ("omit-lane-wrapper", lane_objects[0].content_ref.clone()),
                    ("omit-join-value", join_objects[0].content_ref.clone()),
                ] {
                    let forged =
                        forge_object_omission(raw.clone(), &identity, &omitted_ref, attack);
                    assert_eq!(
                        super::fold::verify_recorded_history(
                            forged,
                            &verifier(&root.fixture),
                            &NoPhysicalBindings,
                        )
                        .expect_err("omitted generated fan-out object must fail closed"),
                        StructuredStoreError::InvalidHistory
                    );
                }
            }
            let exact_outcome = closed_outcome_object(&raw);
            let exact_json = exact_outcome
                .decode::<serde_json::Value>()
                .expect("operation outcome JSON");
            let expected_variant = if closes_as_failure {
                "Failure"
            } else {
                "Success"
            };
            let exact_value = exact_json
                .get(expected_variant)
                .and_then(serde_json::Value::as_object)
                .expect("exact root variant and lexical value");
            assert_eq!(
                exact_value.get("slot_ref"),
                Some(
                    &serde_json::to_value(
                        root.root_slot.content_ref().expect("root slot reference")
                    )
                    .expect("slot reference JSON")
                ),
                "{case:?} must retain its exact producer recipe"
            );

            let mut attacks = hostile_provenance_slots(&root.root_slot)
                .into_iter()
                .map(|(attack, slot)| {
                    let hostile_slot_ref = slot.content_ref().expect("hostile provenance slot ref");
                    (
                        attack,
                        mutate_root_value(&exact_json, expected_variant, |value| {
                            value.insert(
                                "slot_ref".to_owned(),
                                serde_json::to_value(&hostile_slot_ref)
                                    .expect("hostile slot reference JSON"),
                            );
                        }),
                    )
                })
                .collect::<Vec<_>>();
            attacks.extend([
                (
                    "wrong-content",
                    mutate_root_value(&exact_json, expected_variant, |value| {
                        value.insert(
                            "value_ref".to_owned(),
                            serde_json::to_value(content_ref(
                                "fixture.unbound-root-content",
                                discriminator,
                            ))
                            .expect("unbound value reference JSON"),
                        );
                    }),
                ),
                (
                    "wrong-contract",
                    mutate_root_value(&exact_json, expected_variant, |value| {
                        value.insert(
                            "contract_ref".to_owned(),
                            serde_json::to_value(content_ref(
                                "fixture.foreign-root-contract",
                                discriminator,
                            ))
                            .expect("foreign contract reference JSON"),
                        );
                    }),
                ),
                (
                    "wrong-variant",
                    swap_root_variant(&exact_json, expected_variant),
                ),
            ]);
            for (attack, hostile_json) in attacks {
                let forged = forge_closed_outcome(
                    raw.clone(),
                    &identity,
                    exact_outcome,
                    hostile_json,
                    &format!("{case:?}-{expected_variant}-{attack}"),
                );
                assert_eq!(
                    super::fold::verify_recorded_history(
                        forged,
                        &verifier(&root.fixture),
                        &NoPhysicalBindings,
                    )
                    .expect_err("hostile root outcome must fail closed"),
                    StructuredStoreError::InvalidHistory
                );
            }
            discriminator = discriminator.checked_add(1).expect("fixture discriminator");
        }
    }
}

#[tokio::test]
async fn persisted_authorizations_reject_future_lane_binding_and_ordinal_substitution() {
    let (sequential, future_state) = two_state_read_fixture(96);
    let sequential_identity = store_identity(96);
    let sequential_backend = StructuredMemoryBackend::new(sequential_identity.clone());
    let sequential_store = StructuredRunStore::new(
        sequential_backend.clone(),
        Arc::new(verifier(&sequential)),
        Arc::new(AcceptPhysicalBindings),
    );
    let (writer, _) = sequential_store.split();
    let sequential_run = run_id(96);
    writer
        .admit_run(admission(
            &sequential,
            sequential_run.clone(),
            "future-authorization-admission",
        ))
        .await
        .expect("sequential Read admission");
    let first = writer
        .load_verified(&sequential_run)
        .await
        .expect("first Read action");
    let StructuredFrontier::Actions(actions) = first.frontier() else {
        panic!("first Read must be actionable")
    };
    let first_action = actions[0].clone();
    writer
        .authorize_access(
            first,
            &AccessAuthorizationProposal::new(
                AppendRequestId::new("valid-first-authorization").expect("authorization id"),
                first_action.input.clone(),
                ProposedCanonicalValue::from_json("7").expect("Read request"),
                admission_object("fixture.physical-binding", "fixture.physical-binding", 96),
            ),
        )
        .await
        .expect("valid first authorization");
    let sequential_raw = sequential_backend
        .load(&sequential_run)
        .await
        .expect("raw sequential authorization")
        .expect("sequential authorization prefix");
    super::fold::verify_recorded_history(
        sequential_raw.clone(),
        &verifier(&sequential),
        &AcceptPhysicalBindings,
    )
    .expect("valid sequential authorization prefix");

    let future_input_slot_ref = future_state.inputs[0]
        .content_ref()
        .expect("future state input slot ref");
    for (attack, forged) in [
        (
            "actual-future-occurrence",
            forge_last_record(sequential_raw.clone(), &sequential_identity, |record| {
                let RunRecord::ExternalAccessAuthorized(authorization) = record else {
                    panic!("authorization record")
                };
                authorization.occurrence_id = future_state.occurrence_id.clone();
                authorization.occurrence_path_ref = future_state
                    .occurrence_path
                    .content_ref()
                    .expect("future occurrence path ref");
                authorization.semantic_call_id = future_state.semantic_call_id.clone();
                authorization.state_input_ref.slot_ref = future_input_slot_ref.clone();
            }),
        ),
        (
            "wrong-binding",
            forge_last_record(sequential_raw.clone(), &sequential_identity, |record| {
                let RunRecord::ExternalAccessAuthorized(authorization) = record else {
                    panic!("authorization record")
                };
                authorization.state_input_ref.slot_ref =
                    content_ref("fixture.foreign-state-input", 96);
            }),
        ),
        (
            "wrong-attempt-ordinal",
            forge_last_record(sequential_raw, &sequential_identity, |record| {
                let RunRecord::ExternalAccessAuthorized(authorization) = record else {
                    panic!("authorization record")
                };
                authorization.attempt_ordinal = 1;
            }),
        ),
    ] {
        assert_eq!(
            super::fold::verify_recorded_history(
                forged,
                &verifier(&sequential),
                &AcceptPhysicalBindings,
            )
            .expect_err(attack),
            StructuredStoreError::InvalidHistory
        );
    }

    let (fan_out, lane_b) = two_lane_read_fixture(97);
    let fan_out_identity = store_identity(97);
    let fan_out_backend = StructuredMemoryBackend::new(fan_out_identity.clone());
    let fan_out_store = StructuredRunStore::new(
        fan_out_backend.clone(),
        Arc::new(verifier(&fan_out)),
        Arc::new(AcceptPhysicalBindings),
    );
    let (writer, _) = fan_out_store.split();
    let fan_out_run = run_id(97);
    writer
        .admit_run(admission(
            &fan_out,
            fan_out_run.clone(),
            "lane-authorization-admission",
        ))
        .await
        .expect("fan-out Read admission");
    let lane_a = writer
        .load_verified(&fan_out_run)
        .await
        .expect("first lane Read");
    let StructuredFrontier::Actions(actions) = lane_a.frontier() else {
        panic!("first lane Read must be actionable")
    };
    let lane_a_action = actions[0].clone();
    writer
        .authorize_access(
            lane_a,
            &AccessAuthorizationProposal::new(
                AppendRequestId::new("valid-lane-a-authorization").expect("authorization id"),
                lane_a_action.input,
                ProposedCanonicalValue::from_json("7").expect("lane Read request"),
                admission_object("fixture.physical-binding", "fixture.physical-binding", 97),
            ),
        )
        .await
        .expect("valid lane A authorization");
    let raw = fan_out_backend
        .load(&fan_out_run)
        .await
        .expect("raw lane authorization")
        .expect("lane authorization prefix");
    let forged = forge_last_record(raw, &fan_out_identity, |record| {
        let RunRecord::ExternalAccessAuthorized(authorization) = record else {
            panic!("authorization record")
        };
        authorization.occurrence_id = lane_b.occurrence_id.clone();
        authorization.occurrence_path_ref = lane_b
            .occurrence_path
            .content_ref()
            .expect("lane B path ref");
        authorization.semantic_call_id = lane_b.semantic_call_id.clone();
    });
    assert_eq!(
        super::fold::verify_recorded_history(forged, &verifier(&fan_out), &AcceptPhysicalBindings,)
            .expect_err("authorization cannot jump to another actual lane"),
        StructuredStoreError::InvalidHistory
    );
}

#[tokio::test]
async fn persisted_observations_reject_variant_schema_and_contract_substitution() {
    let (fixture, returned_ref, safe_failure_ref, safe_failure_schema) =
        distinct_read_contract_fixture(106);
    let identity = store_identity(106);
    let backend = StructuredMemoryBackend::new(identity.clone());
    let store = StructuredRunStore::new(
        backend.clone(),
        Arc::new(verifier(&fixture)),
        Arc::new(AcceptPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let run_id = run_id(106);
    writer
        .admit_run(admission(
            &fixture,
            run_id.clone(),
            "observation-substitution-admission",
        ))
        .await
        .expect("distinct Read admission");
    let verified = reader
        .load_verified(&run_id)
        .await
        .expect("distinct Read action");
    let StructuredFrontier::Actions(actions) = verified.frontier() else {
        panic!("distinct Read must be actionable")
    };
    let action = actions[0].clone();
    let authorization = writer
        .authorize_access(
            verified,
            &AccessAuthorizationProposal::new(
                AppendRequestId::new("distinct-read-authorization")
                    .expect("authorization append id"),
                action.input,
                ProposedCanonicalValue::from_json("7").expect("Read request"),
                admission_object("fixture.physical-binding", "fixture.physical-binding", 106),
            ),
        )
        .await
        .expect("distinct Read authorization");
    let (authorization, authorized) = authorization
        .into_newly_appended_authorization()
        .expect("new Read authorization");
    writer
        .commit_observation(
            authorized,
            &AccessObservationProposal::new(
                AppendRequestId::new("distinct-read-observation").expect("observation append id"),
                authorization.authorization_ref().clone(),
                ProposedObservationOutcome::Returned(
                    ProposedCanonicalValue::from_json("8").expect("returned value"),
                ),
            ),
        )
        .await
        .expect("valid returned observation");
    let raw = backend
        .load(&run_id)
        .await
        .expect("raw distinct Read")
        .expect("observed distinct Read");
    let returned_value = raw
        .batches
        .last()
        .and_then(|batch| batch.records.first())
        .and_then(|assigned| match &assigned.record {
            RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                ObservationOutcome::Returned { value } => Some(value.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("returned observation value");
    assert_eq!(returned_value.contract_ref, returned_ref);

    let returned_object = raw
        .batches
        .last()
        .expect("observation batch")
        .objects
        .iter()
        .find(|object| object.content_ref == returned_value.value_ref)
        .expect("returned value object");
    let wrong_schema_object = HistoryObject::new(
        returned_object.object_type.clone(),
        safe_failure_schema
            .schema_id()
            .expect("safe-failure schema id"),
        &returned_object.canonical_json,
    )
    .expect("wrong-schema value object");

    let attacks = [
        forge_last_record(raw.clone(), &identity, |record| {
            let RunRecord::ExternalAccessObserved(observation) = record else {
                panic!("observation record")
            };
            observation.outcome = ObservationOutcome::SafeFailure {
                value: returned_value.clone(),
            };
        }),
        forge_last_record(raw.clone(), &identity, |record| {
            let RunRecord::ExternalAccessObserved(observation) = record else {
                panic!("observation record")
            };
            let ObservationOutcome::Returned { value } = &mut observation.outcome else {
                panic!("returned observation")
            };
            value.contract_ref = safe_failure_ref.clone();
        }),
        forge_last_record_with_object(
            raw.clone(),
            &identity,
            wrong_schema_object,
            |record, object| {
                let RunRecord::ExternalAccessObserved(observation) = record else {
                    panic!("observation record")
                };
                let ObservationOutcome::Returned { value } = &mut observation.outcome else {
                    panic!("returned observation")
                };
                value.value_ref = object.content_ref.clone();
            },
        ),
        forge_last_record(raw, &identity, |record| {
            let RunRecord::ExternalAccessObserved(observation) = record else {
                panic!("observation record")
            };
            observation.outcome = ObservationOutcome::EntryUnknown {
                fault_code: StableId::new("fixture.read-entry-unknown").expect("fault code"),
            };
        }),
    ];
    for hostile in attacks {
        assert_eq!(
            super::fold::verify_recorded_history(
                hostile,
                &verifier(&fixture),
                &AcceptPhysicalBindings,
            )
            .expect_err("hostile observation substitution must fail closed"),
            StructuredStoreError::InvalidHistory
        );
    }
}

#[tokio::test]
async fn persisted_structure_rejects_skipped_roles_and_missing_or_premature_closure() {
    for (discriminator, skipped_role, labels) in [
        (100_u8, "pre", ["pre", "protected"]),
        (101, "post", ["failure-post", "handler"]),
        (102, "handler", ["handler", "continuation"]),
    ] {
        let fixture = sequential_named_state_fixture(discriminator, &labels);
        let identity = store_identity(discriminator);
        let backend = StructuredMemoryBackend::new(identity.clone());
        let store = StructuredRunStore::new(
            backend.clone(),
            Arc::new(verifier(&fixture)),
            Arc::new(NoPhysicalBindings),
        );
        let (writer, reader) = store.split();
        let run_id = run_id(discriminator);
        writer
            .admit_run(admission(
                &fixture,
                run_id.clone(),
                "skipped-role-admission",
            ))
            .await
            .expect("sequential role admission");
        let verified = reader
            .load_verified(&run_id)
            .await
            .expect("first structural role");
        let StructuredFrontier::Actions(actions) = verified.frontier() else {
            panic!("first structural role must be actionable")
        };
        let current = actions[0].clone();
        let ExpandedDeclaration::State(target) = &fixture.expanded.root.declarations[1] else {
            panic!("second structural role state")
        };
        let raw = backend
            .load(&run_id)
            .await
            .expect("raw role prefix")
            .expect("role admission prefix");
        let forged = super::fold::assign_candidate(
            &identity,
            CommitCandidate {
                run_id: run_id.clone(),
                expected_head: Some(raw.batches[0].head.clone()),
                append_request_id: AppendRequestId::new(format!("forged-skip-{skipped_role}"))
                    .expect("forged skip append id"),
                tenant_fact_coordinate: TenantFactCoordinate::None,
                records: vec![RunRecord::StateTransitionCommitted(
                    StateTransitionCommitted {
                        occurrence_id: target.occurrence_id.clone(),
                        occurrence_path_ref: target
                            .occurrence_path
                            .content_ref()
                            .expect("target path ref"),
                        semantic_call_id: target.semantic_call_id.clone(),
                        input: current.input.clone(),
                        consumed_observation_ref: None,
                        outcome_ref: current.input.value.value_ref.clone(),
                        outcome: StateOutcomeRef::Success(current.input),
                        facts: Vec::new(),
                        before_semantic_state_digest: verified
                            .semantic_head()
                            .semantic_state_digest()
                            .clone(),
                        after_semantic_state_digest: verified
                            .semantic_head()
                            .semantic_state_digest()
                            .clone(),
                    },
                )],
                objects: Vec::new(),
            },
        )
        .expect("well-formed skipped-role envelope");
        let mut hostile = raw;
        hostile.batches.push(forged);
        assert_eq!(
            super::fold::verify_recorded_history(
                hostile,
                &verifier(&fixture),
                &NoPhysicalBindings,
            )
            .expect_err("an ordinary structural role cannot be skipped"),
            StructuredStoreError::InvalidHistory
        );
    }

    let zero = zero_state_fixture(103);
    let zero_identity = store_identity(103);
    let zero_backend = StructuredMemoryBackend::new(zero_identity.clone());
    let zero_store = StructuredRunStore::new(
        zero_backend.clone(),
        Arc::new(verifier(&zero)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, _) = zero_store.split();
    let zero_run = run_id(103);
    writer
        .admit_run(admission(&zero, zero_run.clone(), "closed-zero-admission"))
        .await
        .expect("closed zero-state admission");
    let mut omitted = zero_backend
        .load(&zero_run)
        .await
        .expect("raw closed zero run")
        .expect("closed zero run");
    let original = omitted.batches.pop().expect("zero admission batch");
    let admission_record = original.records[0].record.clone();
    omitted.batches.push(
        super::fold::assign_candidate(
            &zero_identity,
            CommitCandidate {
                run_id: zero_run,
                expected_head: None,
                append_request_id: AppendRequestId::new("omitted-admission-closure")
                    .expect("omitted closure append id"),
                tenant_fact_coordinate: TenantFactCoordinate::None,
                records: vec![admission_record],
                objects: original.objects,
            },
        )
        .expect("well-formed omitted-closure envelope"),
    );
    assert_eq!(
        super::fold::verify_recorded_history(omitted, &verifier(&zero), &NoPhysicalBindings)
            .expect_err("derivable admission root requires adjacent closure"),
        StructuredStoreError::InvalidHistory
    );

    let transitioned = one_state_fixture(105);
    let transitioned_identity = store_identity(105);
    let transitioned_backend = StructuredMemoryBackend::new(transitioned_identity.clone());
    let transitioned_store = StructuredRunStore::new(
        transitioned_backend.clone(),
        Arc::new(verifier(&transitioned)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = transitioned_store.split();
    let transitioned_run = run_id(105);
    writer
        .admit_run(admission(
            &transitioned,
            transitioned_run.clone(),
            "transitioned-root-admission",
        ))
        .await
        .expect("open transition-derived root run");
    writer
        .commit_state_transition(
            reader
                .load_verified(&transitioned_run)
                .await
                .expect("transition-derived root action"),
            &StateTransitionProposal::success(
                AppendRequestId::new("transitioned-root-success").expect("transition append id"),
                ProposedCanonicalValue::from_json("8").expect("transition output"),
                mfm_facts::FactSet::empty(),
            ),
        )
        .await
        .expect("valid transition-derived root closure");
    let mut omitted_transition_closure = transitioned_backend
        .load(&transitioned_run)
        .await
        .expect("raw transition-derived root")
        .expect("closed transition-derived root");
    let original = omitted_transition_closure
        .batches
        .pop()
        .expect("transition and closure batch");
    let transition_record = original.records[0].record.clone();
    let RunRecord::RunClosed(RunClosed { .. }) = &original.records[1].record else {
        panic!("transition-derived root must close in the same batch")
    };
    let objects = original.objects;
    omitted_transition_closure.batches.push(
        super::fold::assign_candidate(
            &transitioned_identity,
            CommitCandidate {
                run_id: transitioned_run,
                expected_head: omitted_transition_closure
                    .batches
                    .last()
                    .map(|batch| batch.head.clone()),
                append_request_id: AppendRequestId::new("omitted-transition-closure")
                    .expect("omitted transition closure append id"),
                tenant_fact_coordinate: TenantFactCoordinate::None,
                records: vec![transition_record],
                objects,
            },
        )
        .expect("well-formed transition without closure envelope"),
    );
    assert_eq!(
        super::fold::verify_recorded_history(
            omitted_transition_closure,
            &verifier(&transitioned),
            &NoPhysicalBindings,
        )
        .expect_err("transition-derived root requires adjacent closure"),
        StructuredStoreError::InvalidHistory
    );

    let open = one_state_fixture(104);
    let open_identity = store_identity(104);
    let open_backend = StructuredMemoryBackend::new(open_identity.clone());
    let open_store = StructuredRunStore::new(
        open_backend.clone(),
        Arc::new(verifier(&open)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, _) = open_store.split();
    let open_run = run_id(104);
    writer
        .admit_run(admission(&open, open_run.clone(), "open-state-admission"))
        .await
        .expect("open state admission");
    let mut premature = open_backend
        .load(&open_run)
        .await
        .expect("raw open run")
        .expect("open run");
    let original = premature.batches.pop().expect("open admission batch");
    let mut records = original
        .records
        .iter()
        .map(|assigned| assigned.record.clone())
        .collect::<Vec<_>>();
    records.push(RunRecord::RunClosed(RunClosed {
        outcome_ref: content_ref("fixture.premature-operation-outcome", 104),
    }));
    premature.batches.push(
        super::fold::assign_candidate(
            &open_identity,
            CommitCandidate {
                run_id: open_run,
                expected_head: None,
                append_request_id: AppendRequestId::new("premature-admission-closure")
                    .expect("premature closure append id"),
                tenant_fact_coordinate: TenantFactCoordinate::None,
                records,
                objects: original.objects,
            },
        )
        .expect("well-formed premature-closure envelope"),
    );
    assert_eq!(
        super::fold::verify_recorded_history(premature, &verifier(&open), &NoPhysicalBindings)
            .expect_err("open state cannot carry RunClosed"),
        StructuredStoreError::InvalidHistory
    );
}

#[tokio::test]
async fn persisted_failure_plans_reject_skipped_actual_pre_post_and_handler_occurrences() {
    let handled = handled_failure_role_fixture(114);
    let identity = store_identity(114);
    let backend = StructuredMemoryBackend::new(identity.clone());
    let store = StructuredRunStore::new(
        backend.clone(),
        Arc::new(verifier(&handled.fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let handled_run = run_id(114);
    writer
        .admit_run(admission(
            &handled.fixture,
            handled_run.clone(),
            "actual-handled-plan-admission",
        ))
        .await
        .expect("admit actual handled failure plan");
    writer
        .commit_state_transition(
            reader
                .load_verified(&handled_run)
                .await
                .expect("protected state action"),
            &StateTransitionProposal::failure(
                AppendRequestId::new("actual-protected-failure")
                    .expect("protected failure append id"),
                ProposedCanonicalValue::from_json("7").expect("typed failure value"),
            ),
        )
        .await
        .expect("commit protected typed failure");
    let pre_frontier = reader
        .load_verified(&handled_run)
        .await
        .expect("actual pre-handler frontier");
    let StructuredFrontier::Actions(pre_actions) = pre_frontier.frontier() else {
        panic!("actual pre-handler state must be actionable")
    };
    assert_eq!(pre_actions[0].occurrence_id, handled.pre.occurrence_id);
    let raw_after_failure = backend
        .load(&handled_run)
        .await
        .expect("raw handled failure prefix")
        .expect("handled failure prefix");
    assert_eq!(
        super::fold::verify_recorded_history(
            forge_skipped_state_transition(
                raw_after_failure,
                &identity,
                &pre_frontier,
                &handled.handler,
                pre_actions[0].input.clone(),
                "skip-actual-pre",
            ),
            &verifier(&handled.fixture),
            &NoPhysicalBindings,
        )
        .expect_err("designated handler cannot skip its actual pre-handler block"),
        StructuredStoreError::InvalidHistory
    );

    writer
        .commit_state_transition(
            pre_frontier,
            &StateTransitionProposal::success(
                AppendRequestId::new("actual-pre-success").expect("pre append id"),
                ProposedCanonicalValue::from_json("7").expect("pre output"),
                mfm_facts::FactSet::empty(),
            ),
        )
        .await
        .expect("commit actual pre-handler state");
    let handler_frontier = reader
        .load_verified(&handled_run)
        .await
        .expect("designated handler frontier");
    let StructuredFrontier::Actions(handler_actions) = handler_frontier.frontier() else {
        panic!("designated failure handler must be actionable")
    };
    assert_eq!(
        handler_actions[0].occurrence_id,
        handled.handler.occurrence_id
    );
    let raw_after_pre = backend
        .load(&handled_run)
        .await
        .expect("raw pre-handler prefix")
        .expect("pre-handler prefix");
    assert_eq!(
        super::fold::verify_recorded_history(
            forge_skipped_state_transition(
                raw_after_pre,
                &identity,
                &handler_frontier,
                &handled.normal_continuation,
                handler_actions[0].input.clone(),
                "skip-designated-handler",
            ),
            &verifier(&handled.fixture),
            &NoPhysicalBindings,
        )
        .expect_err("normal continuation cannot skip the designated failure handler"),
        StructuredStoreError::InvalidHistory
    );

    let propagating = propagating_failure_post_fixture(115);
    let identity = store_identity(115);
    let backend = StructuredMemoryBackend::new(identity.clone());
    let store = StructuredRunStore::new(
        backend.clone(),
        Arc::new(verifier(&propagating.fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let post_run = run_id(115);
    writer
        .admit_run(admission(
            &propagating.fixture,
            post_run.clone(),
            "actual-failure-post-admission",
        ))
        .await
        .expect("admit actual propagation failure plan");
    writer
        .commit_state_transition(
            reader
                .load_verified(&post_run)
                .await
                .expect("propagating protected state"),
            &StateTransitionProposal::failure(
                AppendRequestId::new("propagating-protected-failure")
                    .expect("propagating failure append id"),
                ProposedCanonicalValue::from_json("7").expect("propagating failure value"),
            ),
        )
        .await
        .expect("commit propagation source failure");
    let post_frontier = reader
        .load_verified(&post_run)
        .await
        .expect("actual failure-post frontier");
    let StructuredFrontier::Actions(post_actions) = post_frontier.frontier() else {
        panic!("actual failure-post state must be actionable")
    };
    assert_eq!(
        post_actions[0].occurrence_id,
        propagating.post.occurrence_id
    );
    let raw_after_failure = backend
        .load(&post_run)
        .await
        .expect("raw propagation prefix")
        .expect("propagation prefix");
    assert_eq!(
        super::fold::verify_recorded_history(
            forge_skipped_state_transition(
                raw_after_failure,
                &identity,
                &post_frontier,
                &propagating.normal_continuation,
                post_actions[0].input.clone(),
                "skip-actual-failure-post",
            ),
            &verifier(&propagating.fixture),
            &NoPhysicalBindings,
        )
        .expect_err("normal continuation cannot skip the actual failure-post block"),
        StructuredStoreError::InvalidHistory
    );
}

#[tokio::test]
async fn persisted_transition_cannot_select_an_inactive_match_arm() {
    let (fixture, inactive_state) = branch_choice_fixture(105);
    let identity = store_identity(105);
    let backend = StructuredMemoryBackend::new(identity.clone());
    let store = StructuredRunStore::new(
        backend.clone(),
        Arc::new(verifier(&fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let run_id = run_id(105);
    writer
        .admit_run(admission_with_json(
            &fixture,
            run_id.clone(),
            "branch-choice-admission",
            r#"{"kind":"allow","value":7}"#,
        ))
        .await
        .expect("active Match-arm admission");
    let verified = reader
        .load_verified(&run_id)
        .await
        .expect("active Match arm");
    let StructuredFrontier::Actions(actions) = verified.frontier() else {
        panic!("active Match arm must be actionable")
    };
    let active = actions[0].clone();
    assert_ne!(active.occurrence_id, inactive_state.occurrence_id);
    let raw = backend
        .load(&run_id)
        .await
        .expect("raw Match prefix")
        .expect("Match admission prefix");
    let forged = super::fold::assign_candidate(
        &identity,
        CommitCandidate {
            run_id: run_id.clone(),
            expected_head: Some(raw.batches[0].head.clone()),
            append_request_id: AppendRequestId::new("forged-inactive-match-arm")
                .expect("forged branch append id"),
            tenant_fact_coordinate: TenantFactCoordinate::None,
            records: vec![RunRecord::StateTransitionCommitted(
                StateTransitionCommitted {
                    occurrence_id: inactive_state.occurrence_id.clone(),
                    occurrence_path_ref: inactive_state
                        .occurrence_path
                        .content_ref()
                        .expect("inactive state path ref"),
                    semantic_call_id: inactive_state.semantic_call_id,
                    input: active.input.clone(),
                    consumed_observation_ref: None,
                    outcome_ref: active.input.value.value_ref.clone(),
                    outcome: StateOutcomeRef::Success(active.input),
                    facts: Vec::new(),
                    before_semantic_state_digest: verified
                        .semantic_head()
                        .semantic_state_digest()
                        .clone(),
                    after_semantic_state_digest: verified
                        .semantic_head()
                        .semantic_state_digest()
                        .clone(),
                },
            )],
            objects: Vec::new(),
        },
    )
    .expect("well-formed inactive-arm envelope");
    let mut hostile = raw;
    hostile.batches.push(forged);
    assert_eq!(
        super::fold::verify_recorded_history(hostile, &verifier(&fixture), &NoPhysicalBindings,)
            .expect_err("inactive Match-arm state cannot become the cursor"),
        StructuredStoreError::InvalidHistory
    );
}

#[tokio::test]
async fn fresh_folds_resume_every_runtime_crash_boundary() {
    let (read, _, _, _) = distinct_read_contract_fixture(107);
    let read_backend = StructuredMemoryBackend::new(store_identity(107));
    let read_store = StructuredRunStore::new(
        read_backend.clone(),
        Arc::new(verifier(&read)),
        Arc::new(AcceptPhysicalBindings),
    );
    let (writer, reader) = read_store.split();
    let read_run = run_id(107);
    writer
        .admit_run(admission(&read, read_run.clone(), "crash-read-admission"))
        .await
        .expect("Read crash admission");
    assert!(matches!(
        fresh_frontier(&read_backend, &read, &read_run).await,
        StructuredFrontier::Actions(_)
    ));
    let verified = reader.load_verified(&read_run).await.expect("Read action");
    let StructuredFrontier::Actions(actions) = verified.frontier() else {
        panic!("Read must be actionable")
    };
    let action = actions[0].clone();
    let authorization = writer
        .authorize_access(
            verified,
            &AccessAuthorizationProposal::new(
                AppendRequestId::new("crash-read-authorization").expect("authorization append id"),
                action.input,
                ProposedCanonicalValue::from_json("7").expect("Read request"),
                admission_object("fixture.physical-binding", "fixture.physical-binding", 107),
            ),
        )
        .await
        .expect("Read authorization");
    let (authorization, authorized) = authorization
        .into_newly_appended_authorization()
        .expect("new Read authorization");
    assert_eq!(
        fresh_frontier(&read_backend, &read, &read_run).await,
        StructuredFrontier::WaitingReads,
        "authorization crash must resume before invocation"
    );
    let before_invocation = read_backend
        .load(&read_run)
        .await
        .expect("authorization prefix")
        .expect("authorized Read");
    assert_eq!(
        fresh_frontier(&read_backend, &read, &read_run).await,
        StructuredFrontier::WaitingReads,
        "invocation has no append boundary and resumes from authorization"
    );
    assert_eq!(
        read_backend
            .load(&read_run)
            .await
            .expect("post-invocation prefix")
            .expect("authorized Read"),
        before_invocation
    );
    writer
        .commit_observation(
            authorized,
            &AccessObservationProposal::new(
                AppendRequestId::new("crash-read-observation").expect("observation append id"),
                authorization.authorization_ref().clone(),
                ProposedObservationOutcome::Returned(
                    ProposedCanonicalValue::from_json("8").expect("Read returned value"),
                ),
            ),
        )
        .await
        .expect("Read observation");
    assert!(matches!(
        fresh_frontier(&read_backend, &read, &read_run).await,
        StructuredFrontier::Actions(_)
    ));
    writer
        .commit_state_transition(
            reader
                .load_verified(&read_run)
                .await
                .expect("observed Read"),
            &StateTransitionProposal::success(
                AppendRequestId::new("crash-read-settlement").expect("settlement append id"),
                ProposedCanonicalValue::from_json("9").expect("settled value"),
                mfm_facts::FactSet::empty(),
            ),
        )
        .await
        .expect("Read settlement and closure");
    assert_eq!(
        fresh_frontier(&read_backend, &read, &read_run).await,
        StructuredFrontier::Complete,
        "settlement and adjacent closure survive a fresh fold"
    );

    let handler = handled_failure_role_fixture(108);
    let handler_backend = StructuredMemoryBackend::new(store_identity(108));
    let handler_store = StructuredRunStore::new(
        handler_backend.clone(),
        Arc::new(verifier(&handler.fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = handler_store.split();
    let handler_run = run_id(108);
    writer
        .admit_run(admission(
            &handler.fixture,
            handler_run.clone(),
            "crash-handler-admission",
        ))
        .await
        .expect("handler admission");
    writer
        .commit_state_transition(
            reader
                .load_verified(&handler_run)
                .await
                .expect("protected action"),
            &StateTransitionProposal::failure(
                AppendRequestId::new("crash-handler-protected-failure")
                    .expect("protected failure append id"),
                ProposedCanonicalValue::from_json("7").expect("protected failure"),
            ),
        )
        .await
        .expect("protected transition");
    let StructuredFrontier::Actions(actions) =
        fresh_frontier(&handler_backend, &handler.fixture, &handler_run).await
    else {
        panic!("failure-plan crash must resume at its exact pre-handler state")
    };
    assert_eq!(actions[0].occurrence_id, handler.pre.occurrence_id);
    writer
        .commit_state_transition(
            reader
                .load_verified(&handler_run)
                .await
                .expect("pre-handler action"),
            &StateTransitionProposal::success(
                AppendRequestId::new("crash-handler-pre").expect("pre-handler append id"),
                ProposedCanonicalValue::from_json("7").expect("pre-handler output"),
                mfm_facts::FactSet::empty(),
            ),
        )
        .await
        .expect("pre-handler transition");
    let StructuredFrontier::Actions(actions) =
        fresh_frontier(&handler_backend, &handler.fixture, &handler_run).await
    else {
        panic!("pre-handler crash must resume at the designated handler")
    };
    assert_eq!(actions[0].occurrence_id, handler.handler.occurrence_id);
    writer
        .commit_state_transition(
            reader
                .load_verified(&handler_run)
                .await
                .expect("designated handler action"),
            &StateTransitionProposal::success(
                AppendRequestId::new("crash-designated-handler").expect("handler append id"),
                ProposedCanonicalValue::from_json(r#"{"kind":"Propagate","failure":7}"#)
                    .expect("handler route output"),
                mfm_facts::FactSet::empty(),
            ),
        )
        .await
        .expect("designated handler transition");
    assert_eq!(
        fresh_frontier(&handler_backend, &handler.fixture, &handler_run).await,
        StructuredFrontier::Complete,
        "handler transition and failure closure survive a fresh fold"
    );

    let (branch, _) = branch_choice_fixture(109);
    let branch_backend = StructuredMemoryBackend::new(store_identity(109));
    let branch_store = StructuredRunStore::new(
        branch_backend.clone(),
        Arc::new(verifier(&branch)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, _) = branch_store.split();
    let branch_run = run_id(109);
    writer
        .admit_run(admission_with_json(
            &branch,
            branch_run.clone(),
            "crash-branch-admission",
            r#"{"kind":"allow","value":7}"#,
        ))
        .await
        .expect("branch admission");
    let StructuredFrontier::Actions(actions) =
        fresh_frontier(&branch_backend, &branch, &branch_run).await
    else {
        panic!("branch selection crash must resume in the selected arm")
    };
    assert!(actions[0]
        .occurrence_path
        .segments()
        .iter()
        .any(|segment| matches!(
            segment,
            StructuralPathSegment::MatchArm { tag, .. } if tag == "allow"
        )));

    let (fan_out, _) = two_lane_read_fixture(110);
    let fan_out_backend = StructuredMemoryBackend::new(store_identity(110));
    let fan_out_store = StructuredRunStore::new(
        fan_out_backend.clone(),
        Arc::new(verifier(&fan_out)),
        Arc::new(AcceptPhysicalBindings),
    );
    let (writer, reader) = fan_out_store.split();
    let fan_out_run = run_id(110);
    writer
        .admit_run(admission(
            &fan_out,
            fan_out_run.clone(),
            "crash-fan-out-admission",
        ))
        .await
        .expect("fan-out admission");
    let mut authorization_refs = Vec::new();
    for lane in ["a", "b"] {
        let verified = reader
            .load_verified(&fan_out_run)
            .await
            .expect("fan-out lane action");
        let StructuredFrontier::Actions(actions) = verified.frontier() else {
            panic!("next fan-out lane must be actionable")
        };
        let action = actions[0].clone();
        let attempt = writer
            .authorize_access(
                verified,
                &AccessAuthorizationProposal::new(
                    AppendRequestId::new(format!("crash-fan-out-authorize-{lane}"))
                        .expect("lane authorization id"),
                    action.input,
                    ProposedCanonicalValue::from_json("7").expect("lane request"),
                    admission_object("fixture.physical-binding", "fixture.physical-binding", 110),
                ),
            )
            .await
            .expect("lane authorization");
        let (authorization, _) = attempt
            .into_newly_appended_authorization()
            .expect("new lane authorization");
        authorization_refs.push(authorization.authorization_ref().clone());
    }
    assert_eq!(
        fresh_frontier(&fan_out_backend, &fan_out, &fan_out_run).await,
        StructuredFrontier::WaitingReads
    );
    for (lane, authorization_ref) in authorization_refs.into_iter().enumerate() {
        let verified = reader
            .load_verified(&fan_out_run)
            .await
            .expect("fan-out observation prefix");
        writer
            .commit_observation(
                verified,
                &AccessObservationProposal::new(
                    AppendRequestId::new(format!("crash-fan-out-observe-{lane}"))
                        .expect("lane observation id"),
                    authorization_ref,
                    ProposedObservationOutcome::Returned(
                        ProposedCanonicalValue::from_json("8").expect("lane returned value"),
                    ),
                ),
            )
            .await
            .expect("lane observation");
        assert!(matches!(
            fresh_frontier(&fan_out_backend, &fan_out, &fan_out_run).await,
            StructuredFrontier::Actions(_)
        ));
        writer
            .commit_state_transition(
                reader
                    .load_verified(&fan_out_run)
                    .await
                    .expect("observed fan-out lane"),
                &StateTransitionProposal::success(
                    AppendRequestId::new(format!("crash-fan-out-settle-{lane}"))
                        .expect("lane settlement id"),
                    ProposedCanonicalValue::from_json("9").expect("lane settlement"),
                    mfm_facts::FactSet::empty(),
                ),
            )
            .await
            .expect("lane settlement");
    }
    assert_eq!(
        fresh_frontier(&fan_out_backend, &fan_out, &fan_out_run).await,
        StructuredFrontier::Complete,
        "final fan-out settlement and closure survive a fresh fold"
    );
}

struct RootProvenanceFixture {
    fixture: Fixture,
    root_slot: LexicalSlot,
    input_json: &'static str,
    requires_transition: bool,
}

fn two_state_read_fixture(discriminator: u8) -> (Fixture, ExpandedStateBinding) {
    let mut fixture =
        one_state_fixture_with_execution(discriminator, StructuredExecutionKind::Read);
    let implementation_manifest_ref = fixture
        .document
        .root
        .components
        .secret_free_implementation_manifest_closure_ref
        .clone();
    let access_components = access_component_sidecars(&fixture);
    let ExpandedDeclaration::State(first) = &fixture.expanded.root.declarations[0] else {
        panic!("one-state Read fixture")
    };
    let first = first.as_ref().clone();
    let label = StableId::new("future-state").expect("future state label");
    let occurrence_path = fixture
        .expanded
        .root
        .path
        .child(StructuralPathSegment::Declaration {
            label: label.clone(),
            ordinal: 1,
        })
        .expect("future state path");
    let occurrence_id = occurrence_path
        .occurrence_id()
        .expect("future occurrence id");
    let semantic_call_id = SemanticCallPath::new(vec![SemanticPathSegment {
        label: label.clone(),
        discriminator: None,
    }])
    .expect("future semantic path")
    .identity()
    .expect("future semantic call id");
    let output_slot = LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: fixture.input.contract_ref.clone(),
        producer: LexicalProducer::StateOutput {
            occurrence_id: occurrence_id.clone(),
            role: ResultRole::SuccessOutput,
        },
    };
    let future = ExpandedStateBinding {
        semantic_call_id,
        occurrence_id,
        occurrence_path,
        label,
        contract: first.contract.clone(),
        inputs: vec![first.output_slot.clone()],
        output_slot: output_slot.clone(),
        failure_boundary: first.failure_boundary.clone(),
    };
    fixture.expanded.root.declarations = vec![
        ExpandedDeclaration::State(Box::new(first)),
        ExpandedDeclaration::State(Box::new(future.clone())),
    ];
    fixture.expanded.root.tail = BlockTail::Normal(output_slot);
    let contract = value_contract(&fixture.value_schema, discriminator);
    fixture.document = document(&fixture.expanded, &contract, discriminator);
    fixture
        .document
        .root
        .components
        .secret_free_implementation_manifest_closure_ref = implementation_manifest_ref;
    fixture.document.component_closure.extend(access_components);
    (fixture, future)
}

fn branch_choice_fixture(discriminator: u8) -> (Fixture, ExpandedStateBinding) {
    let entry_point =
        StableId::new(format!("fixture.branch.{discriminator}")).expect("branch entry point");
    let root_path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: entry_point.clone(),
    }])
    .expect("branch root path");
    let selector_shape = SchemaShape::tagged_enum(
        EnumTagging::Internal {
            tag: "kind".to_owned(),
        },
        ["allow", "deny"]
            .into_iter()
            .map(|tag| {
                EnumVariantDescriptor::new(
                    tag,
                    SchemaShape::named_struct(vec![FieldDescriptor::required(
                        "value",
                        SchemaShape::UnsignedInteger { bits: 64 },
                    )])
                    .expect("branch payload shape"),
                )
            })
            .collect(),
    )
    .expect("branch selector shape");
    let selector_schema = shaped_value_schema(discriminator, "branch-selector", selector_shape);
    let payload_discriminator = discriminator
        .checked_add(64)
        .expect("payload discriminator");
    let payload_schema = value_schema(payload_discriminator);
    let selector_contract = value_contract(&selector_schema, discriminator);
    let payload_contract = value_contract(&payload_schema, payload_discriminator);
    let selector_ref = retained_value_contract_ref(&selector_contract).expect("selector ref");
    let payload_ref = retained_value_contract_ref(&payload_contract).expect("payload ref");
    let selector = admission_slot(&root_path, &selector_ref);
    let match_label = StableId::new("choice").expect("Match label");
    let match_path = root_path
        .child(StructuralPathSegment::Declaration {
            label: match_label.clone(),
            ordinal: 0,
        })
        .expect("Match path");
    let scope_id = StableId::new("fixture.branch-scope").expect("branch scope id");
    let mut arms = Vec::new();
    let mut arm_outputs = Vec::new();
    let mut inactive = None;
    for tag in ["allow", "deny"] {
        let arm_label = StableId::new(format!("{tag}-arm")).expect("arm label");
        let arm_path = match_path
            .child(StructuralPathSegment::MatchArm {
                label: arm_label.clone(),
                tag: tag.to_owned(),
            })
            .expect("arm path");
        let payload = LexicalSlot {
            lexical_path: arm_path.clone(),
            contract_ref: payload_ref.clone(),
            producer: LexicalProducer::VariantPayload {
                selector: Box::new(selector.clone()),
                canonical_tag: tag.to_owned(),
                payload_path: vec![StableId::new("value").expect("payload path")],
            },
        };
        let state_label = StableId::new(format!("{tag}-state")).expect("arm state label");
        let occurrence_path = arm_path
            .child(StructuralPathSegment::Declaration {
                label: state_label.clone(),
                ordinal: 0,
            })
            .expect("arm state path");
        let occurrence_id = occurrence_path.occurrence_id().expect("arm occurrence id");
        let semantic_call_id = SemanticCallPath::new(vec![SemanticPathSegment {
            label: state_label.clone(),
            discriminator: Some(arm_label.clone()),
        }])
        .expect("arm semantic path")
        .identity()
        .expect("arm semantic call id");
        let output = LexicalSlot {
            lexical_path: occurrence_path.clone(),
            contract_ref: payload_ref.clone(),
            producer: LexicalProducer::StateOutput {
                occurrence_id: occurrence_id.clone(),
                role: ResultRole::SuccessOutput,
            },
        };
        let state = ExpandedStateBinding {
            semantic_call_id,
            occurrence_id,
            occurrence_path,
            label: state_label,
            contract: StructuredStateContract::new(
                StableId::new("fixture.branch-state").expect("branch state id"),
                StructuredStateExecutionContract::Pure,
                payload_ref.clone(),
                payload_ref.clone(),
                StructuredFailureContract::Never,
                StructuredSafeFailureDispositionContract::NotApplicable {},
                None,
            )
            .expect("branch state contract"),
            inputs: vec![payload],
            output_slot: output.clone(),
            failure_boundary: mfm_spec::structured::CertifiedFailureBoundary::NoFailure(
                NoFailureBoundary {
                    never_contract_ref: never_failure_contract_ref().expect("Never ref"),
                },
            ),
        };
        if tag == "deny" {
            inactive = Some(state.clone());
        }
        arms.push(ExpandedMatchArm {
            canonical_tag: tag.to_owned(),
            label: arm_label,
            path: arm_path.clone(),
            body: ExpandedBlock {
                path: arm_path,
                failure_scope: FailureScopeBinding::Inherits {
                    scope_id: scope_id.clone(),
                    failure_contract: StructuredFailureContract::Never,
                },
                declarations: vec![ExpandedDeclaration::State(Box::new(state))],
                failure_exits: Vec::new(),
                tail: BlockTail::Normal(output.clone()),
            },
        });
        arm_outputs.push(output);
    }
    let match_output = LexicalSlot {
        lexical_path: match_path.clone(),
        contract_ref: payload_ref.clone(),
        producer: LexicalProducer::MatchMerge {
            match_path: match_path.clone(),
            declaration_ordered_arm_slots: arm_outputs,
        },
    };
    let selector_contract_table = ClosedSumContract::new(
        selector_ref.clone(),
        ["allow", "deny"]
            .into_iter()
            .map(|tag| ClosedSumVariant {
                canonical_tag: tag.to_owned(),
                payloads: vec![ClosedSumPayload {
                    payload_path: vec![StableId::new("value").expect("payload path")],
                    contract_ref: payload_ref.clone(),
                }],
            })
            .collect(),
    )
    .expect("branch selector contract");
    let expanded = ExpandedStructuredProgram {
        operation_id: entry_point.clone(),
        input_roots: vec![selector.clone()],
        output_contract_ref: payload_ref,
        failure_contract: StructuredFailureContract::Never,
        root: ExpandedBlock {
            path: root_path,
            failure_scope: FailureScopeBinding::Owns {
                scope: FailureScope {
                    scope_id,
                    failure_contract: StructuredFailureContract::Never,
                    default_mappers: Vec::new(),
                },
            },
            declarations: vec![ExpandedDeclaration::Match(Box::new(ExpandedMatch {
                label: match_label,
                path: match_path,
                selector,
                selector_contract: selector_contract_table,
                arms,
                output_slot: match_output.clone(),
            }))],
            failure_exits: Vec::new(),
            tail: BlockTail::Normal(match_output),
        },
    };
    let document = document(&expanded, &selector_contract, discriminator);
    let mut fixture = Fixture {
        entry_point,
        document,
        expanded,
        input: admission_slot(
            &StructuralPath::new(vec![StructuralPathSegment::Root {
                operation_id: StableId::new(format!("fixture.branch.{discriminator}"))
                    .expect("branch entry point"),
            }])
            .expect("branch root path"),
            &selector_ref,
        ),
        value_schema: selector_schema,
        extra_value_schemas: BTreeMap::new(),
    };
    add_retained_contract(&mut fixture, &payload_contract, payload_schema);
    (fixture, inactive.expect("inactive deny state"))
}

fn distinct_read_contract_fixture(
    discriminator: u8,
) -> (Fixture, ContentRef, ContentRef, SchemaIdentity) {
    let mut fixture = zero_state_fixture(discriminator);
    let request_ref = fixture.input.contract_ref.clone();
    let returned_discriminator = discriminator
        .checked_add(64)
        .expect("returned discriminator");
    let safe_failure_discriminator = discriminator
        .checked_add(96)
        .expect("safe-failure discriminator");
    let returned_schema = value_schema(returned_discriminator);
    let safe_failure_schema = value_schema(safe_failure_discriminator);
    let returned_contract = value_contract(&returned_schema, returned_discriminator);
    let safe_failure_contract = value_contract(&safe_failure_schema, safe_failure_discriminator);
    let returned_ref =
        retained_value_contract_ref(&returned_contract).expect("returned contract ref");
    let safe_failure_ref =
        retained_value_contract_ref(&safe_failure_contract).expect("safe-failure contract ref");
    let adapter_ref = content_ref("fixture.distinct-read-adapter", discriminator);
    let capability = StructuredLiveComponentContract::new_read_capability(
        StableId::new(format!("fixture.distinct-read-capability-{discriminator}"))
            .expect("distinct Read capability id"),
        request_ref.clone(),
        returned_ref.clone(),
        safe_failure_ref.clone(),
        adapter_ref.clone(),
    )
    .expect("distinct Read capability");
    let capability_ref = capability.content_ref().expect("capability ref");
    let label = StableId::new("distinct-read-state").expect("state label");
    let occurrence_path = fixture
        .expanded
        .root
        .path
        .child(StructuralPathSegment::Declaration {
            label: label.clone(),
            ordinal: 0,
        })
        .expect("state path");
    let occurrence_id = occurrence_path.occurrence_id().expect("occurrence id");
    let semantic_call_id = SemanticCallPath::new(vec![SemanticPathSegment {
        label: label.clone(),
        discriminator: None,
    }])
    .expect("semantic path")
    .identity()
    .expect("semantic call id");
    let state_contract = StructuredStateContract::new(
        StableId::new("fixture.distinct-read-state").expect("state id"),
        StructuredStateExecutionContract::Read {
            capability_contract_ref: capability_ref.clone(),
        },
        request_ref.clone(),
        request_ref.clone(),
        StructuredFailureContract::Never,
        StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {},
        None,
    )
    .expect("state contract");
    let output = LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: request_ref,
        producer: LexicalProducer::StateOutput {
            occurrence_id: occurrence_id.clone(),
            role: ResultRole::SuccessOutput,
        },
    };
    fixture.expanded.root.declarations =
        vec![ExpandedDeclaration::State(Box::new(ExpandedStateBinding {
            semantic_call_id,
            occurrence_id,
            occurrence_path,
            label,
            contract: state_contract.clone(),
            inputs: vec![fixture.input.clone()],
            output_slot: output.clone(),
            failure_boundary: mfm_spec::structured::CertifiedFailureBoundary::NoFailure(
                NoFailureBoundary {
                    never_contract_ref: never_failure_contract_ref().expect("Never ref"),
                },
            ),
        }))];
    fixture.expanded.root.tail = BlockTail::Normal(output);
    let input_contract = value_contract(&fixture.value_schema, discriminator);
    fixture.document = document(&fixture.expanded, &input_contract, discriminator);
    add_retained_contract(&mut fixture, &returned_contract, returned_schema);
    add_retained_contract(
        &mut fixture,
        &safe_failure_contract,
        safe_failure_schema.clone(),
    );
    fixture
        .document
        .component_closure
        .push(CertifiedComponentObject {
            object_type: StableId::new("structured.capability_contract")
                .expect("capability object type"),
            content_ref: capability_ref.clone(),
            value: CanonicalJsonValue::new(
                serde_json::to_value(capability).expect("capability JSON"),
            )
            .expect("capability canonical value"),
        });
    let implementation_manifest = SecretFreeImplementationManifest {
        entries: vec![
            SecretFreeImplementationManifestEntry {
                component_kind: StructuredComponentKind::State,
                semantic_contract_ref: state_contract.state_contract_ref,
                implementation_contract_ref: content_ref(
                    "fixture.distinct-state-implementation",
                    discriminator,
                ),
            },
            SecretFreeImplementationManifestEntry {
                component_kind: StructuredComponentKind::Capability,
                semantic_contract_ref: capability_ref,
                implementation_contract_ref: content_ref(
                    "fixture.distinct-capability-implementation",
                    discriminator,
                ),
            },
            SecretFreeImplementationManifestEntry {
                component_kind: StructuredComponentKind::Adapter,
                semantic_contract_ref: adapter_ref,
                implementation_contract_ref: content_ref(
                    "fixture.distinct-adapter-implementation",
                    discriminator,
                ),
            },
        ],
    };
    let implementation_object = fixture_component_object(
        "structured.secret_free_implementation_manifest",
        "fixture.distinct-secret-free-implementation-manifest",
        &implementation_manifest,
    );
    fixture
        .document
        .root
        .components
        .secret_free_implementation_manifest_closure_ref =
        implementation_object.content_ref.clone();
    fixture
        .document
        .component_closure
        .push(implementation_object);
    (fixture, returned_ref, safe_failure_ref, safe_failure_schema)
}

fn two_lane_read_fixture(discriminator: u8) -> (Fixture, ExpandedStateBinding) {
    let mut fixture =
        one_state_fixture_with_execution(discriminator, StructuredExecutionKind::Read);
    let implementation_manifest_ref = fixture
        .document
        .root
        .components
        .secret_free_implementation_manifest_closure_ref
        .clone();
    let access_components = access_component_sidecars(&fixture);
    let ExpandedDeclaration::State(template) = &fixture.expanded.root.declarations[0] else {
        panic!("one-state Read fixture")
    };
    let template = template.as_ref().clone();
    let contract = value_contract(&fixture.value_schema, discriminator);
    let contract_ref = fixture.input.contract_ref.clone();
    let group_path = fixture
        .expanded
        .root
        .path
        .child(StructuralPathSegment::Declaration {
            label: StableId::new("read-fan-out").expect("fan-out label"),
            ordinal: 0,
        })
        .expect("Read fan-out path");
    let lane_contract_ref =
        lane_outcome_contract_ref(&contract_ref, &StructuredFailureContract::Never)
            .expect("lane contract ref");
    let join_contract_ref =
        fan_out_join_contract_ref(&contract_ref, &StructuredFailureContract::Never)
            .expect("join contract ref");
    let mut lanes = Vec::new();
    let mut lane_slots = Vec::new();
    let mut lane_b_state = None;
    for (ordinal, key) in [(0_u32, "read-lane-a"), (1, "read-lane-b")] {
        let key = StableId::new(key).expect("Read lane key");
        let lane_path = group_path
            .child(StructuralPathSegment::FanOutLane {
                key: key.clone(),
                ordinal,
            })
            .expect("Read lane path");
        let label = StableId::new(format!("lane-state-{ordinal}")).expect("lane state label");
        let occurrence_path = lane_path
            .child(StructuralPathSegment::Declaration {
                label: label.clone(),
                ordinal: 0,
            })
            .expect("lane state path");
        let occurrence_id = occurrence_path.occurrence_id().expect("lane occurrence id");
        let semantic_call_id = SemanticCallPath::new(vec![SemanticPathSegment {
            label: label.clone(),
            discriminator: Some(key.clone()),
        }])
        .expect("lane semantic path")
        .identity()
        .expect("lane semantic id");
        let output_slot = LexicalSlot {
            lexical_path: occurrence_path.clone(),
            contract_ref: contract_ref.clone(),
            producer: LexicalProducer::StateOutput {
                occurrence_id: occurrence_id.clone(),
                role: ResultRole::SuccessOutput,
            },
        };
        let state = ExpandedStateBinding {
            semantic_call_id,
            occurrence_id,
            occurrence_path,
            label,
            contract: template.contract.clone(),
            inputs: vec![fixture.input.clone()],
            output_slot: output_slot.clone(),
            failure_boundary: template.failure_boundary.clone(),
        };
        if ordinal == 1 {
            lane_b_state = Some(state.clone());
        }
        let outcome_slot = LexicalSlot {
            lexical_path: lane_path.clone(),
            contract_ref: lane_contract_ref.clone(),
            producer: LexicalProducer::LaneOutcome {
                lane_path: lane_path.clone(),
                success_slot: Some(Box::new(output_slot.clone())),
                failure_slot: None,
            },
        };
        lanes.push(ExpandedFanOutLane {
            key,
            declaration_ordinal: ordinal,
            path: lane_path.clone(),
            body: ExpandedBlock {
                path: lane_path,
                failure_scope: never_scope(discriminator.wrapping_add(ordinal as u8 + 1)),
                declarations: vec![ExpandedDeclaration::State(Box::new(state))],
                failure_exits: Vec::new(),
                tail: BlockTail::Normal(output_slot),
            },
            outcome_slot: outcome_slot.clone(),
        });
        lane_slots.push(outcome_slot);
    }
    let join_slot = LexicalSlot {
        lexical_path: group_path.clone(),
        contract_ref: join_contract_ref.clone(),
        producer: LexicalProducer::FanOutJoin {
            group_path: group_path.clone(),
            declaration_ordered_lane_slots: lane_slots,
        },
    };
    fixture.expanded.output_contract_ref = join_contract_ref.clone();
    fixture.expanded.root.declarations =
        vec![ExpandedDeclaration::FanOut(Box::new(ExpandedFanOut {
            label: StableId::new("read-fan-out").expect("fan-out label"),
            path: group_path,
            lane_output_contract_ref: contract_ref.clone(),
            lane_failure_contract: StructuredFailureContract::Never,
            lanes,
            output_slot: join_slot.clone(),
        }))];
    fixture.expanded.root.tail = BlockTail::Normal(join_slot);
    fixture.document = document(&fixture.expanded, &contract, discriminator);
    fixture
        .document
        .root
        .components
        .secret_free_implementation_manifest_closure_ref = implementation_manifest_ref;
    fixture.document.component_closure.extend(access_components);
    add_fan_out_contract_components(&mut fixture.document, &contract_ref);
    (fixture, lane_b_state.expect("lane B state"))
}

fn access_component_sidecars(fixture: &Fixture) -> Vec<CertifiedComponentObject> {
    fixture
        .document
        .component_closure
        .iter()
        .filter(|component| {
            matches!(
                component.object_type.as_str(),
                "structured.capability_contract" | "structured.secret_free_implementation_manifest"
            )
        })
        .cloned()
        .collect()
}

fn add_fan_out_contract_components(
    document: &mut CertifiedProgramDocument,
    success_contract_ref: &ContentRef,
) {
    let failure_contract = StructuredFailureContract::Never;
    document.component_closure.extend([
        CertifiedComponentObject {
            object_type: StableId::new("structured.lane_outcome_contract")
                .expect("lane object type"),
            content_ref: lane_outcome_contract_ref(success_contract_ref, &failure_contract)
                .expect("lane contract ref"),
            value: CanonicalJsonValue::from_canonical_json(
                lane_outcome_contract_canonical_json(success_contract_ref, &failure_contract)
                    .expect("lane contract canonical")
                    .as_bytes(),
            )
            .expect("lane contract value"),
        },
        CertifiedComponentObject {
            object_type: StableId::new("structured.fan_out_join_contract")
                .expect("join object type"),
            content_ref: fan_out_join_contract_ref(success_contract_ref, &failure_contract)
                .expect("join contract ref"),
            value: CanonicalJsonValue::from_canonical_json(
                fan_out_join_contract_canonical_json(success_contract_ref, &failure_contract)
                    .expect("join contract canonical")
                    .as_bytes(),
            )
            .expect("join contract value"),
        },
    ]);
}

fn forge_last_record(
    mut raw: RawRunHistory,
    identity: &StructuredStoreIdentity,
    mutate: impl FnOnce(&mut RunRecord),
) -> RawRunHistory {
    let original = raw.batches.pop().expect("record-producing batch");
    let mut records = original
        .records
        .iter()
        .map(|assigned| assigned.record.clone())
        .collect::<Vec<_>>();
    mutate(records.first_mut().expect("record to mutate"));
    let forged = super::fold::assign_candidate(
        identity,
        CommitCandidate {
            run_id: raw.run_id.clone(),
            expected_head: raw.batches.last().map(|batch| batch.head.clone()),
            append_request_id: AppendRequestId::new("forged-structural-record")
                .expect("forged append id"),
            tenant_fact_coordinate: original.tenant_fact_coordinate,
            records,
            objects: original.objects,
        },
    )
    .expect("well-formed hostile record envelope");
    raw.batches.push(forged);
    raw
}

fn forge_last_record_with_object(
    mut raw: RawRunHistory,
    identity: &StructuredStoreIdentity,
    object: HistoryObject,
    mutate: impl FnOnce(&mut RunRecord, &HistoryObject),
) -> RawRunHistory {
    let original = raw.batches.pop().expect("record-producing batch");
    let mut records = original
        .records
        .iter()
        .map(|assigned| assigned.record.clone())
        .collect::<Vec<_>>();
    mutate(records.first_mut().expect("record to mutate"), &object);
    let mut objects = original.objects;
    objects.push(object);
    objects.sort_by(|left, right| left.content_ref.cmp(&right.content_ref));
    let forged = super::fold::assign_candidate(
        identity,
        CommitCandidate {
            run_id: raw.run_id.clone(),
            expected_head: raw.batches.last().map(|batch| batch.head.clone()),
            append_request_id: AppendRequestId::new("forged-structural-object-record")
                .expect("forged append id"),
            tenant_fact_coordinate: original.tenant_fact_coordinate,
            records,
            objects,
        },
    )
    .expect("well-formed hostile object envelope");
    raw.batches.push(forged);
    raw
}

fn forge_skipped_state_transition(
    mut raw: RawRunHistory,
    identity: &StructuredStoreIdentity,
    verified: &VerifiedStructuredRun,
    target: &ExpandedStateBinding,
    input: LexicalValueRef,
    append_id: &str,
) -> RawRunHistory {
    let prior_head = raw.batches.last().map(|batch| batch.head.clone());
    let semantic_digest = verified.semantic_head().semantic_state_digest().clone();
    let batch = super::fold::assign_candidate(
        identity,
        CommitCandidate {
            run_id: raw.run_id.clone(),
            expected_head: prior_head,
            append_request_id: AppendRequestId::new(append_id).expect("forged skip append id"),
            tenant_fact_coordinate: TenantFactCoordinate::None,
            records: vec![RunRecord::StateTransitionCommitted(
                StateTransitionCommitted {
                    occurrence_id: target.occurrence_id.clone(),
                    occurrence_path_ref: target
                        .occurrence_path
                        .content_ref()
                        .expect("skipped target path ref"),
                    semantic_call_id: target.semantic_call_id.clone(),
                    input: input.clone(),
                    consumed_observation_ref: None,
                    outcome_ref: input.value.value_ref.clone(),
                    outcome: StateOutcomeRef::Success(input),
                    facts: Vec::new(),
                    before_semantic_state_digest: semantic_digest.clone(),
                    after_semantic_state_digest: semantic_digest,
                },
            )],
            objects: Vec::new(),
        },
    )
    .expect("well-formed skipped-state envelope");
    raw.batches.push(batch);
    raw
}

fn root_provenance_fixture(
    discriminator: u8,
    case: RootProvenanceCase,
    closes_as_failure: bool,
) -> RootProvenanceFixture {
    match case {
        RootProvenanceCase::VariantPayload => {
            variant_payload_root_fixture(discriminator, closes_as_failure)
        }
        RootProvenanceCase::FanOutJoin => fan_out_join_root_fixture(discriminator),
        _ => {
            let mut fixture = if case == RootProvenanceCase::StateOutput {
                one_state_fixture(discriminator)
            } else {
                zero_state_fixture(discriminator)
            };
            let source = if case == RootProvenanceCase::StateOutput {
                match &fixture.expanded.root.tail {
                    BlockTail::Normal(slot) => slot.clone(),
                    BlockTail::ScopeFailure(_) => panic!("state fixture must have a normal tail"),
                }
            } else {
                fixture.input.clone()
            };
            let derived_path = fixture
                .expanded
                .root
                .path
                .child(StructuralPathSegment::Declaration {
                    label: StableId::new("provenance-boundary").expect("provenance label"),
                    ordinal: 0,
                })
                .expect("provenance path");
            let root_slot = match case {
                RootProvenanceCase::AdmissionRoot | RootProvenanceCase::StateOutput => source,
                RootProvenanceCase::ArmValue => LexicalSlot {
                    lexical_path: derived_path.clone(),
                    contract_ref: source.contract_ref.clone(),
                    producer: LexicalProducer::ArmValue {
                        selected_arm_path: derived_path,
                        source: Box::new(source),
                    },
                },
                RootProvenanceCase::FragmentInput => LexicalSlot {
                    lexical_path: derived_path.clone(),
                    contract_ref: source.contract_ref.clone(),
                    producer: LexicalProducer::FragmentInput {
                        boundary_id: derived_path
                            .fragment_boundary_id()
                            .expect("fragment boundary id"),
                        child_root_id: StableId::new("child-root").expect("child root id"),
                        source: Box::new(source),
                    },
                },
                RootProvenanceCase::FragmentBoundary => LexicalSlot {
                    lexical_path: derived_path.clone(),
                    contract_ref: source.contract_ref.clone(),
                    producer: LexicalProducer::FragmentBoundary {
                        boundary_id: derived_path
                            .fragment_boundary_id()
                            .expect("fragment boundary id"),
                        role: ResultRole::SuccessOutput,
                        source: Box::new(source),
                    },
                },
                RootProvenanceCase::VariantPayload | RootProvenanceCase::FanOutJoin => {
                    unreachable!("specialized provenance fixture")
                }
            };
            let contract = value_contract(&fixture.value_schema, discriminator);
            set_root_outcome(
                &mut fixture.expanded,
                root_slot.clone(),
                &contract,
                closes_as_failure,
            );
            fixture.document = document(&fixture.expanded, &contract, discriminator);
            RootProvenanceFixture {
                fixture,
                root_slot,
                input_json: "7",
                requires_transition: case == RootProvenanceCase::StateOutput,
            }
        }
    }
}

fn variant_payload_root_fixture(
    discriminator: u8,
    closes_as_failure: bool,
) -> RootProvenanceFixture {
    let entry_point =
        StableId::new(format!("fixture.variant.{discriminator}")).expect("variant entry point");
    let path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: entry_point.clone(),
    }])
    .expect("variant root path");
    let selector_shape = SchemaShape::tagged_enum(
        EnumTagging::Internal {
            tag: "kind".to_owned(),
        },
        vec![EnumVariantDescriptor::new(
            "allow",
            SchemaShape::named_struct(vec![FieldDescriptor::required(
                "value",
                SchemaShape::UnsignedInteger { bits: 64 },
            )])
            .expect("selector payload shape"),
        )],
    )
    .expect("selector shape");
    let selector_schema = shaped_value_schema(discriminator, "selector", selector_shape);
    let payload_discriminator = discriminator
        .checked_add(64)
        .expect("payload discriminator");
    let payload_schema = value_schema(payload_discriminator);
    let selector_contract = value_contract(&selector_schema, discriminator);
    let payload_contract = value_contract(&payload_schema, payload_discriminator);
    let selector_ref = retained_value_contract_ref(&selector_contract).expect("selector ref");
    let payload_ref = retained_value_contract_ref(&payload_contract).expect("payload ref");
    let input = admission_slot(&path, &selector_ref);
    let root_slot = LexicalSlot {
        lexical_path: path.clone(),
        contract_ref: payload_ref.clone(),
        producer: LexicalProducer::VariantPayload {
            selector: Box::new(input.clone()),
            canonical_tag: "allow".to_owned(),
            payload_path: vec![StableId::new("value").expect("payload field")],
        },
    };
    let mut expanded = ExpandedStructuredProgram {
        operation_id: entry_point.clone(),
        input_roots: vec![input.clone()],
        output_contract_ref: payload_ref,
        failure_contract: StructuredFailureContract::Never,
        root: ExpandedBlock {
            path,
            failure_scope: never_scope(discriminator),
            declarations: Vec::new(),
            failure_exits: Vec::new(),
            tail: BlockTail::Normal(root_slot.clone()),
        },
    };
    set_root_outcome(
        &mut expanded,
        root_slot.clone(),
        &payload_contract,
        closes_as_failure,
    );
    let document = document(&expanded, &selector_contract, discriminator);
    let mut fixture = Fixture {
        entry_point,
        document,
        expanded,
        input,
        value_schema: selector_schema,
        extra_value_schemas: BTreeMap::new(),
    };
    add_retained_contract(&mut fixture, &payload_contract, payload_schema);
    RootProvenanceFixture {
        fixture,
        root_slot,
        input_json: r#"{"kind":"allow","value":7}"#,
        requires_transition: false,
    }
}

fn fan_out_join_root_fixture(discriminator: u8) -> RootProvenanceFixture {
    let mut fixture = zero_state_fixture(discriminator);
    let contract = value_contract(&fixture.value_schema, discriminator);
    let contract_ref = fixture.input.contract_ref.clone();
    let group_path = fixture
        .expanded
        .root
        .path
        .child(StructuralPathSegment::Declaration {
            label: StableId::new("fan-out").expect("fan-out label"),
            ordinal: 0,
        })
        .expect("fan-out path");
    let lane_contract_ref =
        lane_outcome_contract_ref(&contract_ref, &StructuredFailureContract::Never)
            .expect("lane outcome contract ref");
    let join_contract_ref =
        fan_out_join_contract_ref(&contract_ref, &StructuredFailureContract::Never)
            .expect("fan-out join contract ref");
    let mut lanes = Vec::new();
    let mut lane_slots = Vec::new();
    for (ordinal, key) in [(0_u32, "lane-a"), (1, "lane-b")] {
        let key = StableId::new(key).expect("lane key");
        let lane_path = group_path
            .child(StructuralPathSegment::FanOutLane {
                key: key.clone(),
                ordinal,
            })
            .expect("lane path");
        let outcome_slot = LexicalSlot {
            lexical_path: lane_path.clone(),
            contract_ref: lane_contract_ref.clone(),
            producer: LexicalProducer::LaneOutcome {
                lane_path: lane_path.clone(),
                success_slot: Some(Box::new(fixture.input.clone())),
                failure_slot: None,
            },
        };
        lanes.push(ExpandedFanOutLane {
            key,
            declaration_ordinal: ordinal,
            path: lane_path.clone(),
            body: ExpandedBlock {
                path: lane_path,
                failure_scope: never_scope(discriminator.wrapping_add(ordinal as u8 + 1)),
                declarations: Vec::new(),
                failure_exits: Vec::new(),
                tail: BlockTail::Normal(fixture.input.clone()),
            },
            outcome_slot: outcome_slot.clone(),
        });
        lane_slots.push(outcome_slot);
    }
    let root_slot = LexicalSlot {
        lexical_path: group_path.clone(),
        contract_ref: join_contract_ref.clone(),
        producer: LexicalProducer::FanOutJoin {
            group_path: group_path.clone(),
            declaration_ordered_lane_slots: lane_slots,
        },
    };
    fixture.expanded.output_contract_ref = join_contract_ref.clone();
    fixture.expanded.root.declarations =
        vec![ExpandedDeclaration::FanOut(Box::new(ExpandedFanOut {
            label: StableId::new("fan-out").expect("fan-out label"),
            path: group_path,
            lane_output_contract_ref: contract_ref.clone(),
            lane_failure_contract: StructuredFailureContract::Never,
            lanes,
            output_slot: root_slot.clone(),
        }))];
    fixture.expanded.root.tail = BlockTail::Normal(root_slot.clone());
    fixture.document = document(&fixture.expanded, &contract, discriminator);
    fixture.document.component_closure.extend([
        CertifiedComponentObject {
            object_type: StableId::new("structured.lane_outcome_contract")
                .expect("lane object type"),
            content_ref: lane_contract_ref,
            value: CanonicalJsonValue::from_canonical_json(
                lane_outcome_contract_canonical_json(
                    &contract_ref,
                    &StructuredFailureContract::Never,
                )
                .expect("lane contract canonical")
                .as_bytes(),
            )
            .expect("lane contract value"),
        },
        CertifiedComponentObject {
            object_type: StableId::new("structured.fan_out_join_contract")
                .expect("join object type"),
            content_ref: join_contract_ref,
            value: CanonicalJsonValue::from_canonical_json(
                fan_out_join_contract_canonical_json(
                    &contract_ref,
                    &StructuredFailureContract::Never,
                )
                .expect("join contract canonical")
                .as_bytes(),
            )
            .expect("join contract value"),
        },
    ]);
    RootProvenanceFixture {
        fixture,
        root_slot,
        input_json: "7",
        requires_transition: false,
    }
}

fn set_root_outcome(
    expanded: &mut ExpandedStructuredProgram,
    root_slot: LexicalSlot,
    root_contract: &RetainedValueContract,
    closes_as_failure: bool,
) {
    if closes_as_failure {
        let failure_contract =
            StructuredFailureContract::typed(root_contract.clone()).expect("typed root failure");
        expanded.failure_contract = failure_contract.clone();
        expanded.root.failure_scope = FailureScopeBinding::Owns {
            scope: FailureScope {
                scope_id: StableId::new("fixture.typed-root-scope").expect("typed scope id"),
                failure_contract,
                default_mappers: Vec::new(),
            },
        };
        expanded.root.tail = BlockTail::ScopeFailure(root_slot);
    } else {
        expanded.failure_contract = StructuredFailureContract::Never;
        expanded.root.tail = BlockTail::Normal(root_slot);
    }
}

fn shaped_value_schema(discriminator: u8, name: &str, shape: SchemaShape) -> SchemaIdentity {
    SchemaIdentity::new(
        SchemaKind::Value,
        Some(fixture_semantic_type(discriminator)),
        format!("fixture.{name}"),
        SchemaVersion::new("1").expect("schema version"),
        shape,
    )
    .expect("shaped value schema")
}

fn add_retained_contract(
    fixture: &mut Fixture,
    contract: &RetainedValueContract,
    schema: SchemaIdentity,
) {
    let contract_ref = retained_value_contract_ref(contract).expect("retained contract ref");
    fixture
        .document
        .component_closure
        .push(CertifiedComponentObject {
            object_type: StableId::new("structured.data_contract").expect("data contract type"),
            content_ref: contract_ref.clone(),
            value: CanonicalJsonValue::from_canonical_json(
                contract
                    .canonical_json()
                    .expect("retained contract canonical")
                    .as_bytes(),
            )
            .expect("retained contract component"),
        });
    fixture.extra_value_schemas.insert(contract_ref, schema);
}

fn hostile_provenance_slots(exact: &LexicalSlot) -> Vec<(&'static str, LexicalSlot)> {
    let mut cross_kind = exact.clone();
    cross_kind.producer = match &exact.producer {
        LexicalProducer::AdmissionRoot { .. } => LexicalProducer::StateOutput {
            occurrence_id: foreign_provenance_path(exact, "cross-kind-state")
                .occurrence_id()
                .expect("cross-kind occurrence id"),
            role: ResultRole::SuccessOutput,
        },
        _ => LexicalProducer::AdmissionRoot {
            root_id: StableId::new("cross-kind-admission-root").expect("cross-kind admission root"),
        },
    };
    let mut attacks = vec![("cross-kind-producer", cross_kind)];
    attacks.extend(match &exact.producer {
        LexicalProducer::AdmissionRoot { .. } => {
            let mut wrong_root = exact.clone();
            let LexicalProducer::AdmissionRoot { root_id } = &mut wrong_root.producer else {
                unreachable!("cloned admission root")
            };
            *root_id = StableId::new("foreign-input-root").expect("foreign root id");
            vec![("wrong-admission-root", wrong_root)]
        }
        LexicalProducer::StateOutput { .. } => {
            let mut wrong_occurrence = exact.clone();
            let LexicalProducer::StateOutput { occurrence_id, .. } = &mut wrong_occurrence.producer
            else {
                unreachable!("cloned state output")
            };
            *occurrence_id = foreign_provenance_path(exact, "foreign-state")
                .occurrence_id()
                .expect("foreign occurrence id");
            let mut wrong_role = exact.clone();
            let LexicalProducer::StateOutput { role, .. } = &mut wrong_role.producer else {
                unreachable!("cloned state output")
            };
            *role = ResultRole::TypedFailure;
            vec![
                ("wrong-state-source", wrong_occurrence),
                ("wrong-state-role", wrong_role),
            ]
        }
        LexicalProducer::ArmValue { .. } => {
            let mut wrong_path = exact.clone();
            let LexicalProducer::ArmValue {
                selected_arm_path, ..
            } = &mut wrong_path.producer
            else {
                unreachable!("cloned arm value")
            };
            *selected_arm_path = selected_arm_path
                .child(StructuralPathSegment::MatchArm {
                    label: StableId::new("foreign-arm").expect("foreign arm label"),
                    tag: "foreign".to_owned(),
                })
                .expect("foreign arm path");
            let mut wrong_source = exact.clone();
            let LexicalProducer::ArmValue { source, .. } = &mut wrong_source.producer else {
                unreachable!("cloned arm value")
            };
            **source = foreign_source_slot(source);
            vec![
                ("wrong-arm-path", wrong_path),
                ("wrong-arm-source", wrong_source),
            ]
        }
        LexicalProducer::VariantPayload { .. } => {
            let mut wrong_selector = exact.clone();
            let LexicalProducer::VariantPayload { selector, .. } = &mut wrong_selector.producer
            else {
                unreachable!("cloned variant payload")
            };
            **selector = foreign_source_slot(selector);
            let mut wrong_tag = exact.clone();
            let LexicalProducer::VariantPayload { canonical_tag, .. } = &mut wrong_tag.producer
            else {
                unreachable!("cloned variant payload")
            };
            *canonical_tag = "deny".to_owned();
            let mut wrong_path = exact.clone();
            let LexicalProducer::VariantPayload { payload_path, .. } = &mut wrong_path.producer
            else {
                unreachable!("cloned variant payload")
            };
            *payload_path = vec![StableId::new("foreign").expect("foreign payload path")];
            vec![
                ("wrong-selector", wrong_selector),
                ("wrong-tag", wrong_tag),
                ("wrong-payload-path", wrong_path),
            ]
        }
        LexicalProducer::FragmentInput { .. } => {
            let mut wrong_boundary = exact.clone();
            let LexicalProducer::FragmentInput { boundary_id, .. } = &mut wrong_boundary.producer
            else {
                unreachable!("cloned fragment input")
            };
            *boundary_id = foreign_provenance_path(exact, "foreign-fragment")
                .fragment_boundary_id()
                .expect("foreign fragment boundary");
            let mut wrong_child = exact.clone();
            let LexicalProducer::FragmentInput { child_root_id, .. } = &mut wrong_child.producer
            else {
                unreachable!("cloned fragment input")
            };
            *child_root_id = StableId::new("foreign-child-root").expect("foreign child root");
            let mut wrong_source = exact.clone();
            let LexicalProducer::FragmentInput { source, .. } = &mut wrong_source.producer else {
                unreachable!("cloned fragment input")
            };
            **source = foreign_source_slot(source);
            vec![
                ("wrong-fragment-input-boundary", wrong_boundary),
                ("wrong-child-root", wrong_child),
                ("wrong-fragment-input-source", wrong_source),
            ]
        }
        LexicalProducer::FragmentBoundary { .. } => {
            let mut wrong_boundary = exact.clone();
            let LexicalProducer::FragmentBoundary { boundary_id, .. } =
                &mut wrong_boundary.producer
            else {
                unreachable!("cloned fragment boundary")
            };
            *boundary_id = foreign_provenance_path(exact, "foreign-boundary")
                .fragment_boundary_id()
                .expect("foreign boundary id");
            let mut wrong_role = exact.clone();
            let LexicalProducer::FragmentBoundary { role, .. } = &mut wrong_role.producer else {
                unreachable!("cloned fragment boundary")
            };
            *role = ResultRole::TypedFailure;
            let mut wrong_source = exact.clone();
            let LexicalProducer::FragmentBoundary { source, .. } = &mut wrong_source.producer
            else {
                unreachable!("cloned fragment boundary")
            };
            **source = foreign_source_slot(source);
            vec![
                ("wrong-fragment-boundary", wrong_boundary),
                ("wrong-fragment-role", wrong_role),
                ("wrong-fragment-source", wrong_source),
            ]
        }
        LexicalProducer::FanOutJoin { .. } => {
            let mut wrong_order = exact.clone();
            let LexicalProducer::FanOutJoin {
                declaration_ordered_lane_slots,
                ..
            } = &mut wrong_order.producer
            else {
                unreachable!("cloned fan-out join")
            };
            declaration_ordered_lane_slots.reverse();
            vec![("wrong-lane-order", wrong_order)]
        }
        producer => panic!("unsupported root provenance mutation: {producer:?}"),
    });
    attacks
}

fn foreign_source_slot(source: &LexicalSlot) -> LexicalSlot {
    let mut foreign = source.clone();
    match &mut foreign.producer {
        LexicalProducer::AdmissionRoot { root_id } => {
            *root_id = StableId::new("foreign-source-root").expect("foreign source root")
        }
        LexicalProducer::StateOutput { occurrence_id, .. } => {
            *occurrence_id = foreign_provenance_path(source, "foreign-source-state")
                .occurrence_id()
                .expect("foreign source occurrence")
        }
        _ => foreign.lexical_path = foreign_provenance_path(source, "foreign-source"),
    }
    foreign
}

fn foreign_provenance_path(slot: &LexicalSlot, label: &str) -> StructuralPath {
    slot.lexical_path
        .child(StructuralPathSegment::Declaration {
            label: StableId::new(label).expect("foreign provenance label"),
            ordinal: 7,
        })
        .expect("foreign provenance path")
}

fn mutate_root_value(
    exact: &serde_json::Value,
    variant: &str,
    mutate: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> serde_json::Value {
    let mut hostile = exact.clone();
    mutate(
        hostile
            .get_mut(variant)
            .and_then(serde_json::Value::as_object_mut)
            .expect("root lexical value"),
    );
    hostile
}

fn swap_root_variant(exact: &serde_json::Value, variant: &str) -> serde_json::Value {
    let mut hostile = exact.as_object().expect("root outcome object").clone();
    let value = hostile.remove(variant).expect("exact root variant");
    let foreign = if variant == "Success" {
        "Failure"
    } else {
        "Success"
    };
    hostile.insert(foreign.to_owned(), value);
    serde_json::Value::Object(hostile)
}

fn closed_outcome_object(raw: &RawRunHistory) -> &HistoryObject {
    let outcome_ref = raw
        .batches
        .last()
        .and_then(|batch| batch.records.last())
        .and_then(|assigned| match &assigned.record {
            RunRecord::RunClosed(closed) => Some(&closed.outcome_ref),
            _ => None,
        })
        .expect("adjacent RunClosed");
    raw.batches
        .iter()
        .flat_map(|batch| &batch.objects)
        .find(|object| &object.content_ref == outcome_ref)
        .expect("operation outcome object")
}

fn forge_closed_outcome(
    mut raw: RawRunHistory,
    identity: &StructuredStoreIdentity,
    exact_outcome: &HistoryObject,
    hostile_json: serde_json::Value,
    _attack: &str,
) -> RawRunHistory {
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&hostile_json).expect("hostile outcome JSON"),
    )
    .expect("canonical hostile outcome");
    let hostile_object = HistoryObject::new(
        exact_outcome.object_type.clone(),
        exact_outcome.content_ref.schema_id().clone(),
        canonical.as_str(),
    )
    .expect("hostile operation outcome object");
    let original = raw.batches.pop().expect("closure-producing batch");
    let mut records = original
        .records
        .iter()
        .map(|assigned| assigned.record.clone())
        .collect::<Vec<_>>();
    let RunRecord::RunClosed(RunClosed { outcome_ref }) =
        records.last_mut().expect("closure record")
    else {
        panic!("last record must close the run")
    };
    *outcome_ref = hostile_object.content_ref.clone();
    let mut objects = original.objects;
    objects.push(hostile_object);
    objects.sort_by(|left, right| left.content_ref.cmp(&right.content_ref));
    let expected_head = raw.batches.last().map(|batch| batch.head.clone());
    let forged = super::fold::assign_candidate(
        identity,
        CommitCandidate {
            run_id: raw.run_id.clone(),
            expected_head,
            append_request_id: AppendRequestId::new("forged-root-outcome")
                .expect("forged append id"),
            tenant_fact_coordinate: original.tenant_fact_coordinate,
            records,
            objects,
        },
    )
    .expect("well-formed hostile closure envelope");
    raw.batches.push(forged);
    raw
}

fn forge_object_omission(
    mut raw: RawRunHistory,
    identity: &StructuredStoreIdentity,
    omitted_ref: &ContentRef,
    append_id: &str,
) -> RawRunHistory {
    let original = raw.batches.pop().expect("object-producing batch");
    let records = original
        .records
        .iter()
        .map(|assigned| assigned.record.clone())
        .collect();
    let original_object_count = original.objects.len();
    let objects = original
        .objects
        .into_iter()
        .filter(|object| &object.content_ref != omitted_ref)
        .collect::<Vec<_>>();
    assert_eq!(objects.len() + 1, original_object_count);
    let forged = super::fold::assign_candidate(
        identity,
        CommitCandidate {
            run_id: raw.run_id.clone(),
            expected_head: raw.batches.last().map(|batch| batch.head.clone()),
            append_request_id: AppendRequestId::new(append_id).expect("omission append id"),
            tenant_fact_coordinate: original.tenant_fact_coordinate,
            records,
            objects,
        },
    )
    .expect("well-formed omission envelope");
    raw.batches.push(forged);
    raw
}

#[tokio::test]
async fn incremental_successors_match_fresh_full_folds_after_every_prefix() {
    let fixture = sequential_state_fixture(6, 4);
    let store = StructuredRunStore::new(
        StructuredMemoryBackend::new(store_identity(6)),
        Arc::new(verifier(&fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, reader) = store.split();
    let run_id = run_id(6);
    writer
        .admit_run(admission(&fixture, run_id.clone(), "equivalence-admit"))
        .await
        .expect("admit sequential run");
    let mut incremental = reader
        .load_verified(&run_id)
        .await
        .expect("initial full fold");

    for ordinal in 0_u32..4 {
        let attempt = writer
            .commit_state_transition(
                incremental,
                &StateTransitionProposal::success(
                    AppendRequestId::new(format!("equivalence-transition-{ordinal}"))
                        .expect("transition append id"),
                    ProposedCanonicalValue::from_value(&(8_u64 + u64::from(ordinal)))
                        .expect("transition value"),
                    mfm_facts::FactSet::empty(),
                ),
            )
            .await
            .expect("incremental transition");
        let (_, successor) = attempt
            .into_committed_successor()
            .expect("known committed successor");
        let full = reader
            .load_verified(&run_id)
            .await
            .expect("fresh full fold after transition");
        assert!(
            super::fold::verified_runs_equivalent(&successor, &full),
            "incremental and full folds differ after prefix {}",
            ordinal + 2
        );
        incremental = successor;
    }
    assert_eq!(incremental.frontier(), &StructuredFrontier::Complete);
}

#[tokio::test]
async fn retained_schema_shape_is_authoritative_before_atomic_append() {
    let fixture = zero_state_fixture(3);
    let backend = StructuredMemoryBackend::new(store_identity(3));
    let store = StructuredRunStore::new(
        backend.clone(),
        Arc::new(verifier(&fixture)),
        Arc::new(NoPhysicalBindings),
    );
    let (writer, _reader) = store.split();
    let run_id = run_id(3);
    let request = admission_with_json(
        &fixture,
        run_id.clone(),
        "admit-wrong-shape",
        "\"not-an-integer\"",
    );
    let error = writer
        .admit_run(request)
        .await
        .expect_err("wrong retained shape must be rejected");
    assert_eq!(error, StructuredStoreError::CandidateRejected);
    assert!(backend
        .load(&run_id)
        .await
        .expect("load after rejected append")
        .is_none());
}

fn verifier(fixture: &Fixture) -> FixtureProgramVerifier {
    let mut value_schemas = fixture.extra_value_schemas.clone();
    value_schemas.insert(
        fixture.input.contract_ref.clone(),
        fixture.value_schema.clone(),
    );
    FixtureProgramVerifier {
        entry_point: fixture.entry_point.clone(),
        document: fixture.document.clone(),
        expanded: fixture.expanded.clone(),
        value_schemas,
    }
}

async fn fresh_frontier(
    backend: &StructuredMemoryBackend,
    fixture: &Fixture,
    run_id: &RunId,
) -> StructuredFrontier {
    let store = StructuredRunStore::new(
        backend.clone(),
        Arc::new(verifier(fixture)),
        Arc::new(AcceptPhysicalBindings),
    );
    let (_, reader) = store.split();
    reader
        .load_verified(run_id)
        .await
        .expect("fresh verified prefix")
        .frontier()
        .clone()
}

fn admission(fixture: &Fixture, run_id: RunId, append_id: &str) -> StructuredAdmissionRequest {
    admission_with_json(fixture, run_id, append_id, "7")
}

fn admission_with_json(
    fixture: &Fixture,
    run_id: RunId,
    append_id: &str,
    value: &str,
) -> StructuredAdmissionRequest {
    StructuredAdmissionRequest::new(
        run_id,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32))).expect("tenant"),
        InvocationIdentity::new("00000000-0000-4000-8000-000000000001").expect("invocation"),
        fixture.entry_point.clone(),
        fixture.document.clone(),
        admission_material(9),
        vec![ProposedCanonicalValue::from_json(value).expect("input")],
        AppendRequestId::new(append_id).expect("append id"),
    )
}

fn admission_material(discriminator: u8) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "fixture.admission-configuration",
            discriminator,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "fixture.admission-context",
            discriminator,
        ),
        PriorRunFactSourceManifest::new(Vec::new())
            .expect("empty prior-run source manifest")
            .to_history_object()
            .expect("prior-run source object"),
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "fixture.admission-routing",
            discriminator,
        ),
        Vec::new(),
    )
    .expect("admission material")
}

fn admission_object(object_type: &str, schema_name: &str, discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        StableId::new(object_type).expect("object type"),
        SchemaId::new(
            schema_name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, schema_name.as_bytes()[0]]),
        )
        .expect("admission schema"),
        "{\"entries\":[]}",
    )
    .expect("admission object")
}

fn zero_state_fixture(discriminator: u8) -> Fixture {
    let entry_point = StableId::new(format!("fixture.zero.{discriminator}")).expect("entry point");
    let path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: entry_point.clone(),
    }])
    .expect("root path");
    let value_schema = value_schema(discriminator);
    let contract = value_contract(&value_schema, discriminator);
    let contract_ref = retained_value_contract_ref(&contract).expect("contract ref");
    let input = admission_slot(&path, &contract_ref);
    let expanded = ExpandedStructuredProgram {
        operation_id: entry_point.clone(),
        input_roots: vec![input.clone()],
        output_contract_ref: contract_ref.clone(),
        failure_contract: StructuredFailureContract::Never,
        root: ExpandedBlock {
            path,
            failure_scope: never_scope(discriminator),
            declarations: Vec::new(),
            failure_exits: Vec::new(),
            tail: BlockTail::Normal(input.clone()),
        },
    };
    let document = document(&expanded, &contract, discriminator);
    Fixture {
        entry_point,
        document,
        expanded,
        input,
        value_schema,
        extra_value_schemas: BTreeMap::new(),
    }
}

fn one_state_fixture(discriminator: u8) -> Fixture {
    one_state_fixture_with_execution(discriminator, StructuredExecutionKind::Pure)
}

fn one_state_fixture_with_execution(
    discriminator: u8,
    execution_kind: StructuredExecutionKind,
) -> Fixture {
    let mut fixture = zero_state_fixture(discriminator);
    let root_path = fixture.expanded.root.path.clone();
    let label = StableId::new("only-state").expect("label");
    let occurrence_path = root_path
        .child(StructuralPathSegment::Declaration {
            label: label.clone(),
            ordinal: 0,
        })
        .expect("occurrence path");
    let occurrence_id = occurrence_path.occurrence_id().expect("occurrence id");
    let semantic_call_id = SemanticCallPath::new(vec![SemanticPathSegment {
        label: label.clone(),
        discriminator: None,
    }])
    .expect("semantic path")
    .identity()
    .expect("semantic call id");
    let contract_ref = fixture.input.contract_ref.clone();
    let adapter_ref = content_ref("fixture.adapter-contract", discriminator);
    let capability = match execution_kind {
        StructuredExecutionKind::Pure => None,
        StructuredExecutionKind::Read => Some(
            StructuredLiveComponentContract::new_read_capability(
                StableId::new(format!("fixture.read-capability-{discriminator}"))
                    .expect("Read capability id"),
                contract_ref.clone(),
                contract_ref.clone(),
                contract_ref.clone(),
                adapter_ref.clone(),
            )
            .expect("Read capability contract"),
        ),
        StructuredExecutionKind::Effect => Some(
            StructuredLiveComponentContract::new_effect_capability_no_refresh(
                StableId::new(format!("fixture.effect-capability-{discriminator}"))
                    .expect("Effect capability id"),
                contract_ref.clone(),
                contract_ref.clone(),
                contract_ref.clone(),
                adapter_ref.clone(),
            )
            .expect("Effect capability contract"),
        ),
    };
    let execution = match &capability {
        None => StructuredStateExecutionContract::Pure,
        Some(capability) => match execution_kind {
            StructuredExecutionKind::Read => StructuredStateExecutionContract::Read {
                capability_contract_ref: capability.content_ref().expect("Read capability ref"),
            },
            StructuredExecutionKind::Effect => StructuredStateExecutionContract::Effect {
                capability_contract_ref: capability.content_ref().expect("Effect capability ref"),
            },
            StructuredExecutionKind::Pure => unreachable!("Pure has no capability"),
        },
    };
    let safe_failure_disposition = match execution_kind {
        StructuredExecutionKind::Pure => StructuredSafeFailureDispositionContract::NotApplicable {},
        StructuredExecutionKind::Read | StructuredExecutionKind::Effect => {
            StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {}
        }
    };
    let state_contract = StructuredStateContract::new(
        StableId::new(format!("fixture.{execution_kind:?}-state").to_ascii_lowercase())
            .expect("state id"),
        execution,
        contract_ref.clone(),
        contract_ref.clone(),
        StructuredFailureContract::Never,
        safe_failure_disposition,
        None,
    )
    .expect("state contract");
    let state_contract_ref = state_contract.state_contract_ref.clone();
    let output = LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: contract_ref.clone(),
        producer: LexicalProducer::StateOutput {
            occurrence_id: occurrence_id.clone(),
            role: ResultRole::SuccessOutput,
        },
    };
    fixture.expanded.root.declarations =
        vec![ExpandedDeclaration::State(Box::new(ExpandedStateBinding {
            semantic_call_id,
            occurrence_id,
            occurrence_path,
            label,
            contract: state_contract,
            inputs: vec![fixture.input.clone()],
            output_slot: output.clone(),
            failure_boundary: mfm_spec::structured::CertifiedFailureBoundary::NoFailure(
                NoFailureBoundary {
                    never_contract_ref: never_failure_contract_ref().expect("never ref"),
                },
            ),
        }))];
    fixture.expanded.root.tail = BlockTail::Normal(output);
    let contract = value_contract(&fixture.value_schema, discriminator);
    fixture.document = document(&fixture.expanded, &contract, discriminator);
    if let Some(capability) = capability {
        let capability_ref = capability.content_ref().expect("capability ref");
        fixture
            .document
            .component_closure
            .push(CertifiedComponentObject {
                object_type: StableId::new("structured.capability_contract")
                    .expect("capability object type"),
                content_ref: capability_ref.clone(),
                value: CanonicalJsonValue::new(
                    serde_json::to_value(capability).expect("capability JSON"),
                )
                .expect("capability canonical value"),
            });
        let implementation_manifest = SecretFreeImplementationManifest {
            entries: vec![
                SecretFreeImplementationManifestEntry {
                    component_kind: StructuredComponentKind::State,
                    semantic_contract_ref: state_contract_ref,
                    implementation_contract_ref: content_ref(
                        "fixture.state-implementation",
                        discriminator,
                    ),
                },
                SecretFreeImplementationManifestEntry {
                    component_kind: StructuredComponentKind::Capability,
                    semantic_contract_ref: capability_ref,
                    implementation_contract_ref: content_ref(
                        "fixture.capability-implementation",
                        discriminator,
                    ),
                },
                SecretFreeImplementationManifestEntry {
                    component_kind: StructuredComponentKind::Adapter,
                    semantic_contract_ref: adapter_ref,
                    implementation_contract_ref: content_ref(
                        "fixture.adapter-implementation",
                        discriminator,
                    ),
                },
            ],
        };
        let implementation_object = fixture_component_object(
            "structured.secret_free_implementation_manifest",
            "fixture.secret-free-implementation-manifest",
            &implementation_manifest,
        );
        fixture
            .document
            .root
            .components
            .secret_free_implementation_manifest_closure_ref =
            implementation_object.content_ref.clone();
        fixture
            .document
            .component_closure
            .push(implementation_object);
    }
    fixture
}

fn sequential_state_fixture(discriminator: u8, state_count: u32) -> Fixture {
    let labels = (0..state_count)
        .map(|ordinal| format!("state-{ordinal}"))
        .collect::<Vec<_>>();
    let labels = labels.iter().map(String::as_str).collect::<Vec<_>>();
    sequential_named_state_fixture(discriminator, &labels)
}

fn handled_failure_role_fixture(discriminator: u8) -> HandledFailureRoleFixture {
    let mut fixture =
        sequential_named_state_fixture(discriminator, &["protected", "normal-continuation"]);
    let contract = value_contract(&fixture.value_schema, discriminator);
    let contract_ref = retained_value_contract_ref(&contract).expect("value contract ref");
    let handler_route_schema = shaped_value_schema(
        discriminator.wrapping_add(64),
        "failure-handler-route",
        SchemaShape::tagged_enum(
            EnumTagging::Internal {
                tag: "kind".to_owned(),
            },
            vec![EnumVariantDescriptor::new(
                "Propagate",
                SchemaShape::named_struct(vec![FieldDescriptor::required(
                    "failure",
                    SchemaShape::UnsignedInteger { bits: 64 },
                )])
                .expect("failure handler payload shape"),
            )],
        )
        .expect("failure handler route shape"),
    );
    let handler_route_value_contract =
        value_contract(&handler_route_schema, discriminator.wrapping_add(64));
    let handler_route_ref = retained_value_contract_ref(&handler_route_value_contract)
        .expect("failure handler route contract ref");
    let typed_failure =
        StructuredFailureContract::typed(contract.clone()).expect("typed failure contract");
    let scope_id =
        StableId::new(format!("fixture.failure-scope.{discriminator}")).expect("failure scope id");
    fixture.expanded.failure_contract = typed_failure.clone();
    fixture.expanded.root.failure_scope = FailureScopeBinding::Owns {
        scope: FailureScope {
            scope_id: scope_id.clone(),
            failure_contract: typed_failure.clone(),
            default_mappers: Vec::new(),
        },
    };

    let (pre, handler, normal_continuation, root_failure_slot) = {
        let [ExpandedDeclaration::State(protected), ExpandedDeclaration::State(continuation)] =
            fixture.expanded.root.declarations.as_mut_slice()
        else {
            panic!("handled failure fixture requires protected and continuation states")
        };
        let failure_source = LexicalSlot {
            lexical_path: protected.occurrence_path.clone(),
            contract_ref: contract_ref.clone(),
            producer: LexicalProducer::StateOutput {
                occurrence_id: protected.occurrence_id.clone(),
                role: ResultRole::TypedFailure,
            },
        };
        protected.contract = StructuredStateContract::new(
            protected.contract.semantic_state_id.clone(),
            protected.contract.execution.clone(),
            protected.contract.input_contract_ref.clone(),
            protected.contract.output_contract_ref.clone(),
            typed_failure.clone(),
            StructuredSafeFailureDispositionContract::NotApplicable {},
            None,
        )
        .expect("fallible protected state contract");
        let plan_path = protected
            .occurrence_path
            .child(StructuralPathSegment::FailurePlan {
                label: StableId::new("actual-handler-plan").expect("handler plan label"),
            })
            .expect("handler plan path");
        let pre = relocated_pure_state(
            continuation,
            &plan_path,
            "actual-failure-pre",
            0,
            failure_source.clone(),
        );
        let mut handler = relocated_pure_state(
            continuation,
            &plan_path,
            "actual-failure-handler",
            1,
            pre.output_slot.clone(),
        );
        handler.contract = StructuredStateContract::new(
            handler.contract.semantic_state_id.clone(),
            handler.contract.execution.clone(),
            handler.contract.input_contract_ref.clone(),
            handler_route_ref.clone(),
            StructuredFailureContract::Never,
            StructuredSafeFailureDispositionContract::NotApplicable {},
            None,
        )
        .expect("designated handler contract");
        handler.output_slot.contract_ref = handler_route_ref.clone();
        let payload_path = vec![StableId::new("failure").expect("failure payload path")];
        let route_contract = ClosedSumContract::new(
            handler_route_ref,
            vec![ClosedSumVariant {
                canonical_tag: "Propagate".to_owned(),
                payloads: vec![ClosedSumPayload {
                    payload_path: payload_path.clone(),
                    contract_ref: contract_ref.clone(),
                }],
            }],
        )
        .expect("default propagation route contract");
        let payload_slot = LexicalSlot {
            lexical_path: plan_path.clone(),
            contract_ref: contract_ref.clone(),
            producer: LexicalProducer::VariantPayload {
                selector: Box::new(handler.output_slot.clone()),
                canonical_tag: "Propagate".to_owned(),
                payload_path,
            },
        };
        let plan_id = FailurePlanIdentity {
            source_semantic_call_id: protected.semantic_call_id.clone(),
            source_slot: failure_source.clone(),
            plan_path: plan_path.clone(),
        }
        .derive()
        .expect("handled failure plan id");
        protected.failure_boundary = CertifiedFailureBoundary::Typed {
            failure_contract: Box::new(typed_failure.clone()),
            source_slot: failure_source.clone(),
            plan: Box::new(FailurePlan::Handled {
                plan_id,
                plan_path: plan_path.clone(),
                source_slot: failure_source.clone(),
                before_handler: Box::new(ExpandedBlock {
                    path: plan_path,
                    failure_scope: FailureScopeBinding::Inherits {
                        scope_id,
                        failure_contract: typed_failure.clone(),
                    },
                    declarations: vec![ExpandedDeclaration::State(Box::new(pre.clone()))],
                    failure_exits: Vec::new(),
                    tail: BlockTail::Normal(pre.output_slot.clone()),
                }),
                handler: Box::new(handler.clone()),
                continuation: Box::new(HandlerContinuation::DefaultPropagation {
                    handler_output_slot: handler.output_slot.clone(),
                    route_contract,
                    payload_slot: Box::new(payload_slot.clone()),
                    failure_tail: Box::new(payload_slot),
                }),
            }),
        };
        (pre, handler, continuation.as_ref().clone(), failure_source)
    };
    fixture.expanded.root.failure_exits = vec![root_failure_slot];
    fixture.document = document(&fixture.expanded, &contract, discriminator);
    add_retained_contract(
        &mut fixture,
        &handler_route_value_contract,
        handler_route_schema,
    );
    HandledFailureRoleFixture {
        fixture,
        pre,
        handler,
        normal_continuation,
    }
}

fn propagating_failure_post_fixture(discriminator: u8) -> PropagatingFailurePostFixture {
    let mut fixture =
        sequential_named_state_fixture(discriminator, &["protected", "normal-continuation"]);
    let contract = value_contract(&fixture.value_schema, discriminator);
    let contract_ref = retained_value_contract_ref(&contract).expect("value contract ref");
    let typed_failure =
        StructuredFailureContract::typed(contract.clone()).expect("typed failure contract");
    let scope_id = StableId::new(format!("fixture.failure-post-scope.{discriminator}"))
        .expect("failure-post scope id");
    fixture.expanded.failure_contract = typed_failure.clone();
    fixture.expanded.root.failure_scope = FailureScopeBinding::Owns {
        scope: FailureScope {
            scope_id: scope_id.clone(),
            failure_contract: typed_failure.clone(),
            default_mappers: Vec::new(),
        },
    };
    let (post, normal_continuation, root_failure_slot) = {
        let [ExpandedDeclaration::State(protected), ExpandedDeclaration::State(continuation)] =
            fixture.expanded.root.declarations.as_mut_slice()
        else {
            panic!("failure-post fixture requires protected and continuation states")
        };
        let failure_source = LexicalSlot {
            lexical_path: protected.occurrence_path.clone(),
            contract_ref: contract_ref.clone(),
            producer: LexicalProducer::StateOutput {
                occurrence_id: protected.occurrence_id.clone(),
                role: ResultRole::TypedFailure,
            },
        };
        protected.contract = StructuredStateContract::new(
            protected.contract.semantic_state_id.clone(),
            protected.contract.execution.clone(),
            protected.contract.input_contract_ref.clone(),
            protected.contract.output_contract_ref.clone(),
            typed_failure.clone(),
            StructuredSafeFailureDispositionContract::NotApplicable {},
            None,
        )
        .expect("fallible propagation source contract");
        let plan_path = protected
            .occurrence_path
            .child(StructuralPathSegment::FailurePlan {
                label: StableId::new("actual-failure-post-plan").expect("failure-post label"),
            })
            .expect("failure-post plan path");
        let post = relocated_pure_state(
            continuation,
            &plan_path,
            "actual-failure-post",
            0,
            failure_source.clone(),
        );
        let boundary_id = plan_path
            .fragment_boundary_id()
            .expect("failure-post boundary id");
        let boundary_slot = LexicalSlot {
            lexical_path: plan_path.clone(),
            contract_ref: contract_ref.clone(),
            producer: LexicalProducer::FragmentBoundary {
                boundary_id: boundary_id.clone(),
                role: ResultRole::TypedFailure,
                source: Box::new(post.output_slot.clone()),
            },
        };
        let plan_id = FailurePlanIdentity {
            source_semantic_call_id: protected.semantic_call_id.clone(),
            source_slot: failure_source.clone(),
            plan_path: plan_path.clone(),
        }
        .derive()
        .expect("propagation failure plan id");
        protected.failure_boundary = CertifiedFailureBoundary::Typed {
            failure_contract: Box::new(typed_failure.clone()),
            source_slot: failure_source.clone(),
            plan: Box::new(FailurePlan::Propagate {
                plan_id,
                plan_path: plan_path.clone(),
                source_slot: failure_source,
                before_boundary: Box::new(ExpandedBlock {
                    path: plan_path,
                    failure_scope: FailureScopeBinding::Inherits {
                        scope_id,
                        failure_contract: typed_failure.clone(),
                    },
                    declarations: vec![ExpandedDeclaration::State(Box::new(post.clone()))],
                    failure_exits: Vec::new(),
                    tail: BlockTail::Normal(post.output_slot.clone()),
                }),
                mapping_chain: Vec::new(),
                boundary_id,
                boundary_slot: Box::new(boundary_slot.clone()),
            }),
        };
        (post, continuation.as_ref().clone(), boundary_slot)
    };
    fixture.expanded.root.failure_exits = vec![root_failure_slot];
    fixture.document = document(&fixture.expanded, &contract, discriminator);
    PropagatingFailurePostFixture {
        fixture,
        post,
        normal_continuation,
    }
}

fn relocated_pure_state(
    template: &ExpandedStateBinding,
    parent_path: &StructuralPath,
    label: &str,
    ordinal: u32,
    input: LexicalSlot,
) -> ExpandedStateBinding {
    let label = StableId::new(label).expect("relocated state label");
    let occurrence_path = parent_path
        .child(StructuralPathSegment::Declaration {
            label: label.clone(),
            ordinal,
        })
        .expect("relocated state path");
    let occurrence_id = occurrence_path
        .occurrence_id()
        .expect("relocated occurrence id");
    let semantic_call_id = SemanticCallPath::new(vec![SemanticPathSegment {
        label: label.clone(),
        discriminator: None,
    }])
    .expect("relocated semantic path")
    .identity()
    .expect("relocated semantic call id");
    let output_slot = LexicalSlot {
        lexical_path: occurrence_path.clone(),
        contract_ref: template.contract.output_contract_ref.clone(),
        producer: LexicalProducer::StateOutput {
            occurrence_id: occurrence_id.clone(),
            role: ResultRole::SuccessOutput,
        },
    };
    ExpandedStateBinding {
        semantic_call_id,
        occurrence_id,
        occurrence_path,
        label,
        contract: template.contract.clone(),
        inputs: vec![input],
        output_slot,
        failure_boundary: CertifiedFailureBoundary::NoFailure(NoFailureBoundary {
            never_contract_ref: never_failure_contract_ref().expect("Never contract ref"),
        }),
    }
}

fn sequential_named_state_fixture(discriminator: u8, labels: &[&str]) -> Fixture {
    let mut fixture = zero_state_fixture(discriminator);
    let root_path = fixture.expanded.root.path.clone();
    let contract_ref = fixture.input.contract_ref.clone();
    let mut input = fixture.input.clone();
    let mut declarations = Vec::new();
    for (ordinal, label) in labels.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).expect("state ordinal");
        let label = StableId::new(*label).expect("state label");
        let occurrence_path = root_path
            .child(StructuralPathSegment::Declaration {
                label: label.clone(),
                ordinal,
            })
            .expect("occurrence path");
        let occurrence_id = occurrence_path.occurrence_id().expect("occurrence id");
        let semantic_call_id = SemanticCallPath::new(vec![SemanticPathSegment {
            label: label.clone(),
            discriminator: None,
        }])
        .expect("semantic path")
        .identity()
        .expect("semantic call id");
        let output = LexicalSlot {
            lexical_path: occurrence_path.clone(),
            contract_ref: contract_ref.clone(),
            producer: LexicalProducer::StateOutput {
                occurrence_id: occurrence_id.clone(),
                role: ResultRole::SuccessOutput,
            },
        };
        declarations.push(ExpandedDeclaration::State(Box::new(ExpandedStateBinding {
            semantic_call_id,
            occurrence_id,
            occurrence_path,
            label,
            contract: StructuredStateContract::new(
                StableId::new("fixture.pure-state").expect("state id"),
                StructuredStateExecutionContract::Pure,
                contract_ref.clone(),
                contract_ref.clone(),
                StructuredFailureContract::Never,
                StructuredSafeFailureDispositionContract::NotApplicable {},
                None,
            )
            .expect("state contract"),
            inputs: vec![input],
            output_slot: output.clone(),
            failure_boundary: mfm_spec::structured::CertifiedFailureBoundary::NoFailure(
                NoFailureBoundary {
                    never_contract_ref: never_failure_contract_ref().expect("never ref"),
                },
            ),
        })));
        input = output;
    }
    fixture.expanded.root.declarations = declarations;
    fixture.expanded.root.tail = BlockTail::Normal(input);
    let contract = value_contract(&fixture.value_schema, discriminator);
    fixture.document = document(&fixture.expanded, &contract, discriminator);
    fixture
}

fn document(
    expanded: &ExpandedStructuredProgram,
    contract: &RetainedValueContract,
    discriminator: u8,
) -> CertifiedProgramDocument {
    let contract_ref = retained_value_contract_ref(contract).expect("contract ref");
    let canonical = contract.canonical_json().expect("contract canonical");
    let component = CertifiedComponentObject {
        object_type: StableId::new("structured.data_contract").expect("object type"),
        content_ref: contract_ref.clone(),
        value: CanonicalJsonValue::from_canonical_json(canonical.as_bytes())
            .expect("component value"),
    };
    let authored_value = CanonicalJsonValue::new(serde_json::json!({
        "fixture": discriminator,
    }))
    .expect("authored fixture value");
    let authored_canonical = authored_value
        .canonical_json()
        .expect("authored fixture canonical bytes");
    let authored_ref = ContentRef::new(
        SchemaId::new(
            "mfm.authored-structured-program",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.authored-structured-program.v1"),
        )
        .expect("authored fixture schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(authored_canonical.as_bytes()),
        ),
    )
    .expect("authored fixture reference");
    let authored_component = CertifiedComponentObject {
        object_type: StableId::new("structured.authored_program").expect("authored object type"),
        content_ref: authored_ref.clone(),
        value: authored_value,
    };
    let placeholder = content_ref("fixture.placeholder", discriminator);
    CertifiedProgramDocument {
        root: CertifiedProgramRoot {
            components: CertifiedProgramComponents {
                certified_program_contract_ref: placeholder.clone(),
                entry_point_contract_ref: placeholder.clone(),
                qualified_entry_point_admission_policy_ref: placeholder.clone(),
                authored_program_ref: authored_ref,
                expanded_program_ref: expanded.content_ref().expect("expanded ref"),
                expansion_profile_ref: placeholder.clone(),
                expansion_proof_ref: placeholder.clone(),
                policy_coverage_proof_ref: placeholder.clone(),
                public_input_output_failure_contract_refs: StructuredPublicContractRefs {
                    input_contract_refs: expanded
                        .input_roots
                        .iter()
                        .map(|slot| slot.contract_ref.clone())
                        .collect(),
                    output_contract_ref: expanded.output_contract_ref.clone(),
                    failure_contract_ref: expanded
                        .failure_contract
                        .contract_ref()
                        .expect("root failure contract ref"),
                },
                certified_structural_bounds: CertifiedStructuralBounds {
                    max_occurrences: 8,
                    max_declarations: 8,
                    max_lanes: 8,
                    max_fan_out_depth: 2,
                    max_branch_depth: 8,
                },
                state_capability_adapter_signer_resource_manifest_closure_ref: placeholder.clone(),
                secret_free_implementation_manifest_closure_ref: placeholder.clone(),
                certification_predicate_set_ref: placeholder,
            },
            canonical_component_closure_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(&[discriminator, 9]),
            ),
        },
        component_closure: vec![authored_component, component],
    }
}

fn fixture_component_object<T: serde::Serialize>(
    object_type: &str,
    schema_name: &str,
    value: &T,
) -> CertifiedComponentObject {
    let value = CanonicalJsonValue::new(serde_json::to_value(value).expect("component JSON"))
        .expect("canonical component value");
    let canonical = value.canonical_json().expect("component canonical JSON");
    let schema = SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("fixture.schema:{schema_name}:1").as_bytes()),
    )
    .expect("component schema");
    CertifiedComponentObject {
        object_type: StableId::new(object_type).expect("component object type"),
        content_ref: ContentRef::new(
            schema,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(canonical.as_bytes()),
            ),
        )
        .expect("component content ref"),
        value,
    }
}

fn admission_slot(path: &StructuralPath, contract_ref: &ContentRef) -> LexicalSlot {
    LexicalSlot {
        lexical_path: path.clone(),
        contract_ref: contract_ref.clone(),
        producer: LexicalProducer::AdmissionRoot {
            root_id: StableId::new("input").expect("input id"),
        },
    }
}

fn never_scope(discriminator: u8) -> FailureScopeBinding {
    FailureScopeBinding::Owns {
        scope: FailureScope {
            scope_id: StableId::new(format!("fixture.scope.{discriminator}")).expect("scope id"),
            failure_contract: StructuredFailureContract::Never,
            default_mappers: Vec::new(),
        },
    }
}

fn value_contract(schema: &SchemaIdentity, discriminator: u8) -> RetainedValueContract {
    RetainedValueContract::new(
        schema.schema_id().expect("schema"),
        fixture_semantic_type(discriminator),
        StableId::new("fixture.integer").expect("role"),
        "application/json",
        content_ref("fixture.evidence", discriminator),
    )
    .expect("retained contract")
}

fn value_schema(discriminator: u8) -> SchemaIdentity {
    SchemaIdentity::new(
        SchemaKind::Value,
        Some(fixture_semantic_type(discriminator)),
        "fixture.integer",
        SchemaVersion::new("1").expect("schema version"),
        SchemaShape::UnsignedInteger { bits: 64 },
    )
    .expect("schema identity")
}

fn fixture_semantic_type(discriminator: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.fixture",
        "integer",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[discriminator, 2]),
    )
    .expect("semantic type")
}

fn content_ref(name: &str, discriminator: u8) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, 3]),
        )
        .expect("content schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(&[discriminator, 4]),
        ),
    )
    .expect("content ref")
}

fn store_identity(discriminator: u8) -> StructuredStoreIdentity {
    StructuredStoreIdentity {
        store_scope_id: StoreScopeId::new(format!(
            "{}{:032x}",
            StoreScopeId::PREFIX,
            discriminator
        ))
        .expect("store scope"),
        store_epoch: StoreEpoch::new(1),
    }
}

fn run_id(discriminator: u8) -> RunId {
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[discriminator, 5]),
    )
}
