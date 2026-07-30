use mfm_canonical::{
    sha256_digest_bytes, CanonicalValue, PlainCanonicalJsonBytes, RecoverabilityContract,
};
use mfm_capabilities::{
    BoundaryStage, CoarseSizeClass, FailureClass, SafeFailureClassifierDescriptor,
    SafeFailureClassifierRule, SafeFailureDiagnosticConstraint, SafeFailureDiagnosticRule,
    SafeFailureOutcome, SafeFailureSizeRule,
};
use mfm_executor::{
    reference_safe_failure, verify_ensure_result, CanonicalExecutorRequest, CommittedEffectRequest,
    EffectExecutorOutcome, EffectIdentity, Ensure, EvidenceBounds, ExecutorBinding,
    ExecutorContractDescriptor, ExecutorDeployment, ExecutorEnsureResultClaim, ExecutorError,
    ExecutorRetainedClosureClaim, ExecutorRetainedClosureContract, ExecutorRetainedValue,
    ExecutorRetainedValueRelation, KeyedExecutorLedger, MemoryConvergentDestination,
    MemoryExecutorStore, ReferenceContract, ReferenceExecutor, ReferenceFailureCode,
    ReferenceRequest, SchemaQualifiedCanonicalValue, VerifiedExecutorBinding,
};
use mfm_facts::{
    CanonicalFactPredicate, FactOrdering, FactSelectionLimit, FactSelectionQuery,
    FactSelectionRequest, FactTieBreak,
};
use mfm_ids::{
    AppendRequestId, ContentRef, DigestAlgorithm, FactQueryDigest, FieldPath, RequestDigest,
    SchemaId, SchemaVersion, SemanticTypeId, StableId,
};
use mfm_journal::{
    ArtifactAdmissionIntent, ArtifactAdmissionMode, AuthorityUse, BatchPurpose,
    CandidateRecordEnvelope, CapabilityBindingRef, CommitCandidatePreimage, CommitDigestPreimage,
    CommitEnvelope, ExecutorEnsureResult, ExecutorEnsureResultFields, ExternalAccessObserved,
    FactSelectionResponse, FactSelectionScanAttestation, JournalPredecessor, NonDomainDisposition,
    NonDomainEntryStatus, NonDomainFailure, NonDomainFailureCode, ObjectPathBinding,
    ObservationOutcome, ObservationRef, ProducerBinding, ProducerBindingFields,
    ReadCapabilityBinding, RecordHashPreimage, RecordIdPreimage, RecordLogicalKey,
    RunJournalRecord, RunJournalRecordFields, RunPhase, SafeFailure, TenantFactCoordinate,
    TenantFactFrontier, TerminalEffectEvidence, TransitionRef,
};
use mfm_spec::{EntryPointContract, RetainedValueContract};
use mfm_values::{
    EnumTagging, EnumVariantDescriptor, FieldDescriptor, SchemaIdentity, SchemaKind, SchemaShape,
};

use super::objects::derive_value_ref;
use super::test_support::fact_scan::fact_selection_contracts;
use super::test_support::{FixtureEffectExecution, FixtureReadExecution, LegalAdmissionFixture};
use super::{
    open_in_memory, verify_offline_recorded_history, AppendOutcome, AuthorizationMaterial,
    CommittedJournalCommit, CommittedJournalRecord, CommittedObject, ExactResolution,
    ExistingRunAppendMaterial, FactSelectionAuthorizationOutcome, InMemoryRunJournalBackend,
    NewlyAppended, ObjectGraphProposal, ObservationMaterial, PreparedJournalAppend,
    ProducedObjectRoot, ProducedOutputSlot, QualifiedSupportMember, ReadObservationMaterial,
    RunHistoryWriter, SafeFailureMetadata, SettlementMaterial, StoreError, StoreIdentity,
    TransitionMaterial,
};

const VALID_DIAGNOSTIC: &str = concat!(
    r#"{"detail":{"code":-1,"reason":{"kind":"malformed"},"#,
    r#""status":500},"kind":"response_invalid"}"#
);

struct ReadFixtureContracts {
    execution: FixtureReadExecution,
    routing_generation_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    diagnostic_contract: RetainedValueContract,
    metadata: SafeFailureMetadata,
    did_not_enter_metadata: Option<SafeFailureMetadata>,
}

fn canonical(value: &str) -> PlainCanonicalJsonBytes {
    PlainCanonicalJsonBytes::from_json_str(value).expect("canonical JSON")
}

fn field_path(value: &str) -> FieldPath {
    FieldPath::new(value).expect("field path")
}

fn stable_id(value: &str) -> StableId {
    StableId::new(value).expect("stable id")
}

fn semantic_type(name: &str) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.store-test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.store-test:{name}").as_bytes()),
    )
    .expect("semantic type")
}

fn retained_contract(
    schema_id: SchemaId,
    semantic_name: &str,
    role: &str,
) -> RetainedValueContract {
    retained_contract_with_media(schema_id, semantic_name, role, "application/json")
}

fn retained_contract_with_media(
    schema_id: SchemaId,
    semantic_name: &str,
    role: &str,
    media_type: &str,
) -> RetainedValueContract {
    RetainedValueContract::new(
        schema_id,
        semantic_type(semantic_name),
        stable_id(role),
        media_type,
        EntryPointContract::retained_contract()
            .expect("entry-point contract")
            .evidence_contract_ref()
            .clone(),
    )
    .expect("retained contract")
}

fn content_ref(contract: &RetainedValueContract, bytes: &[u8]) -> ContentRef {
    ContentRef::new(
        contract.schema_id().clone(),
        RecoverabilityContract::embedded()
            .expect("recoverability contract")
            .raw_content_digest(bytes),
    )
    .expect("content ref")
}

fn support_member(
    path: &str,
    canonical: PlainCanonicalJsonBytes,
    contract: RetainedValueContract,
) -> (QualifiedSupportMember, ContentRef) {
    let reference = content_ref(&contract, canonical.as_bytes());
    (
        QualifiedSupportMember::new(field_path(path), canonical, contract),
        reference,
    )
}

fn executor_support_member(
    path: &str,
    semantic_name: &str,
    role: &str,
    validated: &mfm_canonical::ValidatedCanonicalValue,
) -> (QualifiedSupportMember, ContentRef) {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(validated.as_bytes())
        .expect("executor support bytes");
    support_member(
        path,
        canonical,
        retained_contract(validated.schema_id().clone(), semantic_name, role),
    )
}

struct EffectFixtureContracts {
    execution: FixtureEffectExecution,
    binding: VerifiedExecutorBinding,
    routing_ref: ContentRef,
    request_contract: RetainedValueContract,
}

fn effect_fixture_contracts(tenant_scope_id: &mfm_ids::TenantScopeId) -> EffectFixtureContracts {
    let contract = RecoverabilityContract::embedded().expect("recoverability contract");
    let primitive_schema = contract
        .schema_id("mfm.primitive-canonical_value.v1")
        .expect("primitive schema")
        .clone();
    let routing_contract = retained_contract(primitive_schema, "effect-routing", "effect-routing");
    let (routing_member, routing_ref) = support_member(
        "effect.routing",
        canonical(r#"{"routes":[]}"#),
        routing_contract,
    );

    let request_contract = retained_contract(
        contract
            .schema_id("mfm.executor-reference-queue-request.v1")
            .expect("request schema")
            .clone(),
        "effect-request",
        "effect-request",
    );
    let safe_failure_contract = retained_contract(
        contract
            .schema_id("mfm.safe-failure.v1")
            .expect("safe-failure schema")
            .clone(),
        "effect-safe-failure",
        "effect-safe-failure",
    );
    let ensure_result_contract = retained_contract(
        contract
            .schema_id("mfm.executor-ensure-result.v1")
            .expect("ensure-result schema")
            .clone(),
        "effect-ensure-result",
        "effect-ensure-result",
    );
    let delivery_audit_contract = retained_contract(
        contract
            .schema_id("mfm.executor-delivery-frontier.v2")
            .expect("delivery-frontier schema")
            .clone(),
        "effect-delivery-audit",
        "effect-delivery-audit",
    );
    let executor_frontier_contract = retained_contract(
        contract
            .schema_id("mfm.executor-delivery-frontier.v2")
            .expect("executor-frontier schema")
            .clone(),
        "effect-executor-frontier",
        "effect-executor-frontier",
    );
    let terminal_evidence_contract = retained_contract(
        contract
            .schema_id("mfm.terminal-effect-evidence.v1")
            .expect("terminal-evidence schema")
            .clone(),
        "effect-terminal-evidence",
        "effect-terminal-evidence",
    );
    let terminal_tombstone_contract = retained_contract(
        contract
            .schema_id("mfm.executor-terminal-tombstone.v2")
            .expect("terminal-tombstone schema")
            .clone(),
        "effect-terminal-tombstone",
        "effect-terminal-tombstone",
    );
    let terminal_proof_contract = retained_contract(
        contract
            .schema_id("mfm.executor-reference-terminal-proof.v2")
            .expect("terminal-proof schema")
            .clone(),
        "effect-terminal-proof",
        "effect-terminal-proof",
    );
    let domain_result_contract = retained_contract(
        contract
            .schema_id("mfm.executor-reference-queue-result.v2")
            .expect("domain-result schema")
            .clone(),
        "effect-domain-result",
        "effect-domain-result",
    );
    let closure_contract = ExecutorRetainedClosureContract::new(
        ensure_result_contract.clone(),
        delivery_audit_contract,
        executor_frontier_contract,
        terminal_evidence_contract.clone(),
        terminal_tombstone_contract,
        terminal_proof_contract,
        domain_result_contract.clone(),
    )
    .expect("effect retained closure");
    let deployment = ExecutorDeployment::new(
        routing_ref.clone(),
        routing_ref.clone(),
        tenant_scope_id.clone(),
        routing_ref.clone(),
        None,
    )
    .expect("executor deployment");
    let descriptor = ExecutorContractDescriptor::new(
        routing_ref.clone(),
        request_contract.clone(),
        safe_failure_contract,
        closure_contract,
        routing_ref.clone(),
        routing_ref.clone(),
        EvidenceBounds::new(8, 256, 4_000_000, 16_384, 2, 16_384).expect("evidence bounds"),
        None,
        Vec::new(),
    )
    .expect("executor contract");
    let binding = ExecutorBinding::new(
        descriptor.reference().expect("executor contract ref"),
        routing_ref.clone(),
        deployment.reference().expect("deployment ref"),
    )
    .expect("executor binding");
    let verified = VerifiedExecutorBinding::verify(
        binding.clone(),
        descriptor.clone(),
        deployment.clone(),
        None,
        tenant_scope_id,
    )
    .expect("verified executor binding");

    let descriptor_validated = descriptor.validated().expect("executor contract bytes");
    let (descriptor_member, descriptor_ref) = executor_support_member(
        "effect.contract",
        "effect-contract",
        "effect-contract",
        &descriptor_validated,
    );
    assert_eq!(
        descriptor.reference().expect("executor contract ref"),
        descriptor_ref
    );
    let deployment_validated = deployment.validated().expect("executor deployment bytes");
    let (deployment_member, deployment_ref) = executor_support_member(
        "effect.deployment",
        "effect-deployment",
        "effect-deployment",
        &deployment_validated,
    );
    assert_eq!(
        deployment
            .reference()
            .expect("deployment ref")
            .as_content_ref(),
        &deployment_ref
    );
    let binding_validated = binding.validated().expect("executor binding bytes");
    let (binding_member, binding_ref) = executor_support_member(
        "effect.binding",
        "effect-binding",
        "effect-binding",
        &binding_validated,
    );
    assert_eq!(verified.binding_ref().as_content_ref(), &binding_ref);

    EffectFixtureContracts {
        execution: FixtureEffectExecution {
            executor_operation_id: stable_id("mfm.store-test/effect"),
            executor_binding_ref: binding_ref,
            request_contract: request_contract.clone(),
            ensure_result_contract,
            terminal_evidence_contract,
            domain_result_contract,
            support_members: vec![
                routing_member,
                descriptor_member,
                deployment_member,
                binding_member,
            ],
        },
        binding: verified,
        routing_ref,
        request_contract,
    }
}

fn diagnostic_identity() -> SchemaIdentity {
    let reason = SchemaShape::tagged_enum(
        EnumTagging::Internal {
            tag: "kind".to_owned(),
        },
        vec![
            EnumVariantDescriptor::new("invalid_result", SchemaShape::Unit),
            EnumVariantDescriptor::new("malformed", SchemaShape::Unit),
        ],
    )
    .expect("reason enum");
    let response = SchemaShape::named_struct(vec![
        FieldDescriptor::required("code", SchemaShape::SignedInteger { bits: 64 }),
        FieldDescriptor::required("reason", reason),
        FieldDescriptor::required("status", SchemaShape::UnsignedInteger { bits: 16 }),
    ])
    .expect("response shape");
    SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic_type("read-diagnostic")),
        "mfm.store-test.read_diagnostic",
        SchemaVersion::new("1").expect("schema version"),
        SchemaShape::tagged_enum(
            EnumTagging::Adjacent {
                tag: "kind".to_owned(),
                content: "detail".to_owned(),
            },
            vec![EnumVariantDescriptor::new("response_invalid", response)],
        )
        .expect("diagnostic enum"),
    )
    .expect("diagnostic identity")
}

fn read_fixture_contracts(with_diagnostic: bool) -> ReadFixtureContracts {
    let primitive_schema = RecoverabilityContract::embedded()
        .expect("recoverability contract")
        .schema_id("mfm.primitive-canonical_value.v1")
        .expect("primitive schema")
        .clone();
    let routing_contract =
        retained_contract(primitive_schema.clone(), "routing-catalog", "read-routing");
    let routing_bytes = canonical(r#"{"routes":[]}"#);
    let (routing_member, routing_ref) =
        support_member("read.routing", routing_bytes, routing_contract);

    let diagnostic_identity = with_diagnostic.then(diagnostic_identity);
    let diagnostic_contract = retained_contract(
        diagnostic_identity
            .as_ref()
            .map(|identity| identity.schema_id().expect("diagnostic schema"))
            .unwrap_or_else(|| primitive_schema.clone()),
        if with_diagnostic {
            "read-diagnostic"
        } else {
            "read-diagnostic-value"
        },
        "read-diagnostic",
    );
    let (code, outcome, failure_class, boundary_stage, coarse_size, diagnostic_rule) =
        if with_diagnostic {
            (
                stable_id("response_invalid"),
                SafeFailureOutcome::Indeterminate,
                FailureClass::UnrepresentableResponse,
                BoundaryStage::BoundaryObservation,
                SafeFailureSizeRule::FromFailure,
                SafeFailureDiagnosticRule::required(vec![SafeFailureDiagnosticConstraint::new(
                    field_path("kind"),
                    vec!["response_invalid".to_owned()],
                )
                .expect("diagnostic constraint")])
                .expect("diagnostic rule"),
            )
        } else {
            (
                stable_id("destination_unavailable"),
                SafeFailureOutcome::DidNotEnter,
                FailureClass::Transport,
                BoundaryStage::BeforeBoundaryEntry,
                SafeFailureSizeRule::None,
                SafeFailureDiagnosticRule::Forbidden,
            )
        };
    let mut classifier_rules = vec![SafeFailureClassifierRule::new(
        code.clone(),
        outcome,
        failure_class,
        boundary_stage,
        coarse_size,
        diagnostic_rule,
    )];
    let did_not_enter_metadata = with_diagnostic.then(|| {
        let code = stable_id("destination_unavailable");
        classifier_rules.push(SafeFailureClassifierRule::new(
            code.clone(),
            SafeFailureOutcome::DidNotEnter,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
            SafeFailureSizeRule::None,
            SafeFailureDiagnosticRule::Forbidden,
        ));
        SafeFailureMetadata::new(
            routing_ref.clone(),
            code,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
            None,
        )
    });
    let classifier = SafeFailureClassifierDescriptor::new(
        routing_ref.clone(),
        diagnostic_identity,
        classifier_rules,
    )
    .expect("classifier");
    let classifier_bytes = classifier.canonical().expect("classifier bytes");
    let classifier_contract = retained_contract(
        SafeFailureClassifierDescriptor::schema_id().expect("classifier schema"),
        "read-classifier",
        "read-classifier",
    );
    let (classifier_member, classifier_ref) =
        support_member("read.classifier", classifier_bytes, classifier_contract);
    assert_eq!(
        classifier.content_ref().expect("classifier ref"),
        classifier_ref
    );

    let binding = ReadCapabilityBinding::new(
        &routing_ref,
        &routing_ref,
        &classifier_ref,
        &routing_ref,
        &routing_ref,
        &routing_ref,
    )
    .expect("read binding");
    let binding_bytes = PlainCanonicalJsonBytes::from_canonical_json_slice(binding.as_bytes())
        .expect("binding bytes");
    let binding_contract =
        retained_contract(binding.schema_id().clone(), "read-binding", "read-binding");
    let (binding_member, binding_ref) =
        support_member("read.binding", binding_bytes, binding_contract);
    assert_eq!(binding.content_ref().expect("binding ref"), binding_ref);

    let request_contract =
        retained_contract(primitive_schema.clone(), "read-request", "read-request");
    let returned_contract = retained_contract(primitive_schema, "read-returned", "read-returned");
    let metadata = SafeFailureMetadata::new(
        routing_ref.clone(),
        code,
        failure_class,
        boundary_stage,
        with_diagnostic.then_some(CoarseSizeClass::UpTo16Kib),
    );
    let execution = FixtureReadExecution {
        capability_operation_id: stable_id("mfm.store-test/read"),
        capability_binding_ref: binding_ref,
        request_contract: request_contract.clone(),
        returned_contract: returned_contract.clone(),
        safe_failure_contract: diagnostic_contract.clone(),
        support_members: vec![routing_member, classifier_member, binding_member],
    };
    ReadFixtureContracts {
        execution,
        routing_generation_ref: routing_ref,
        request_contract,
        returned_contract,
        diagnostic_contract,
        metadata,
        did_not_enter_metadata,
    }
}

struct AuthorizedRead {
    store: RunHistoryWriter<InMemoryRunJournalBackend>,
    issuer: super::RunAccessAuthorityIssuer,
    store_identity: StoreIdentity,
    fixture: LegalAdmissionFixture,
    run_id: mfm_ids::RunId,
    authorization_ref: mfm_journal::AuthorizationRef,
    request_ref: mfm_journal::ValueRef,
    request_contract: RetainedValueContract,
    routing_generation_ref: ContentRef,
    returned_contract: RetainedValueContract,
    diagnostic_contract: RetainedValueContract,
    metadata: SafeFailureMetadata,
    did_not_enter_metadata: Option<SafeFailureMetadata>,
}

async fn authorize_read(with_diagnostic: bool, discriminator: u8) -> AuthorizedRead {
    authorize_read_with(read_fixture_contracts(with_diagnostic), discriminator).await
}

async fn authorize_read_with(contracts: ReadFixtureContracts, discriminator: u8) -> AuthorizedRead {
    let routing_generation_ref = contracts.routing_generation_ref.clone();
    let request_contract = contracts.request_contract.clone();
    let returned_contract = contracts.returned_contract.clone();
    let diagnostic_contract = contracts.diagnostic_contract.clone();
    let metadata = contracts.metadata.clone();
    let did_not_enter_metadata = contracts.did_not_enter_metadata.clone();
    let fixture = LegalAdmissionFixture::new(discriminator)
        .expect("fixture")
        .with_read_execution(contracts.execution);
    let store_identity = fixture.store_identity().clone();
    let (store, issuer) = open_in_memory(store_identity.clone());
    fixture
        .provision_in_memory(&store)
        .expect("configured value");
    let support = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fixture");
    let (store, reader) = store.split();
    let prepared = fixture
        .prepare_on(&store, &reader, &issuer, &support)
        .await
        .expect("prepare admission");
    let (admit, append) = prepared.into_parts();
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = store
        .append_admission(&admit, append)
        .await
        .expect("append admission")
    else {
        panic!("new admission");
    };
    let run_id = admitted.run_id().clone();
    let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = store
        .load_for_drive(&drive)
        .await
        .expect("load admitted run")
        .verify_recorded_history()
        .expect("verify admitted run");
    let node_id = view.certified_spec().nodes()[0].node_id().clone();
    let frame = store
        .prepare_frame(&drive, &view, &node_id)
        .await
        .expect("prepare read frame");
    let append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("read-authorize-{discriminator:02x}"))
                .expect("append request"),
            ExistingRunAppendMaterial::Authorization(Box::new(AuthorizationMaterial::Read {
                prepared_frame: Box::new(frame),
                immutable_request_root: Box::new(ProducedObjectRoot::new(
                    request_contract.clone(),
                    canonical("{}"),
                )),
                routing_generation_ref: routing_generation_ref.clone(),
            })),
        )
        .expect("prepare authorization");
    let AppendOutcome::NewlyAppended(NewlyAppended::Authorization(authorization)) = store
        .append(&drive, append)
        .await
        .expect("append authorization")
    else {
        panic!("new authorization");
    };
    let authorization_ref = authorization.authorization_ref().clone();
    let request_ref = authorization
        .authorization()
        .fields()
        .expect("authorization fields")
        .request_ref;
    drop(authorization);
    AuthorizedRead {
        store,
        issuer,
        store_identity,
        fixture,
        run_id,
        authorization_ref,
        request_ref,
        request_contract,
        routing_generation_ref,
        returned_contract,
        diagnostic_contract,
        metadata,
        did_not_enter_metadata,
    }
}

struct AuthorizedEffect {
    store: RunHistoryWriter<InMemoryRunJournalBackend>,
    issuer: super::RunAccessAuthorityIssuer,
    store_identity: StoreIdentity,
    fixture: LegalAdmissionFixture,
    run_id: mfm_ids::RunId,
    authorization_ref: mfm_journal::AuthorizationRef,
    identity: EffectIdentity,
    binding: VerifiedExecutorBinding,
    safe_failure_contract_ref: ContentRef,
    committed_request: CommittedEffectRequest<ReferenceRequest>,
}

async fn authorize_effect(discriminator: u8) -> AuthorizedEffect {
    let fixture = LegalAdmissionFixture::new(discriminator).expect("fixture");
    let contracts = effect_fixture_contracts(fixture.tenant_scope_id());
    let binding = contracts.binding.clone();
    let safe_failure_contract_ref = contracts.routing_ref.clone();
    let request_contract = contracts.request_contract.clone();
    let fixture = fixture.with_effect_execution(contracts.execution);
    let store_identity = fixture.store_identity().clone();
    let (store, issuer) = open_in_memory(store_identity.clone());
    fixture
        .provision_in_memory(&store)
        .expect("configured value");
    let support = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fixture");
    let (store, reader) = store.split();
    let prepared = fixture
        .prepare_on(&store, &reader, &issuer, &support)
        .await
        .expect("prepare admission");
    let (admit, append) = prepared.into_parts();
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = store
        .append_admission(&admit, append)
        .await
        .expect("append admission")
    else {
        panic!("new admission");
    };
    let run_id = admitted.run_id().clone();
    let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = store
        .load_for_drive(&drive)
        .await
        .expect("load admitted run")
        .verify_recorded_history()
        .expect("verify admitted run");
    let node_id = view.certified_spec().nodes()[0].node_id().clone();
    let frame = store
        .prepare_frame(&drive, &view, &node_id)
        .await
        .expect("prepare effect frame");
    let configured_ref = fixture
        .configured_binding()
        .fields()
        .expect("configured binding")
        .value_ref;
    let payload_ref = RecoverabilityContract::embedded()
        .expect("recoverability contract")
        .strict_decode("mfm.value-ref.v1", configured_ref.as_bytes())
        .expect("configured value ref");
    let request =
        ReferenceRequest::new("store-test-effect", payload_ref).expect("reference request");
    let committed_request = CommittedEffectRequest::new(
        binding.binding_ref().clone(),
        fixture.tenant_scope_id().clone(),
        store_identity.store_scope_id(),
        &run_id,
        &node_id,
        request,
    )
    .expect("committed effect request");
    let identity = committed_request.identity().clone();
    let semantic_request = PlainCanonicalJsonBytes::from_canonical_json_slice(
        committed_request.request().canonical_request().as_bytes(),
    )
    .expect("semantic request");
    let append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("effect-request-{discriminator:02x}"))
                .expect("append request"),
            ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::EffectRequested {
                prepared_frame: Box::new(frame),
                semantic_request_root: Box::new(ProducedObjectRoot::new(
                    request_contract,
                    semantic_request,
                )),
                effect_key: identity.effect_key().clone(),
                request_digest: identity.request_digest().clone(),
                executor_binding_ref: CapabilityBindingRef::new(
                    binding.binding_ref().as_content_ref(),
                )
                .expect("capability binding ref"),
                object_graph: ObjectGraphProposal::empty(),
            })),
        )
        .expect("prepare effect request");
    let AppendOutcome::NewlyAppended(NewlyAppended::Transition(transition)) = store
        .append(&drive, append)
        .await
        .expect("append effect request")
    else {
        panic!("new effect request");
    };
    let request_transition_ref =
        TransitionRef::new(&transition.record_refs()[0]).expect("request transition ref");
    let view = store
        .load_for_drive(&drive)
        .await
        .expect("load effect request")
        .verify_recorded_history()
        .expect("verify effect request");
    let append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("effect-authorize-{discriminator:02x}"))
                .expect("append request"),
            ExistingRunAppendMaterial::Authorization(Box::new(
                AuthorizationMaterial::EnsureEffect {
                    request_transition_ref,
                },
            )),
        )
        .expect("prepare effect authorization");
    let AppendOutcome::NewlyAppended(NewlyAppended::Authorization(authorization)) = store
        .append(&drive, append)
        .await
        .expect("append effect authorization")
    else {
        panic!("new effect authorization");
    };
    let authorization_ref = authorization.authorization_ref().clone();
    drop(authorization);
    AuthorizedEffect {
        store,
        issuer,
        store_identity,
        fixture,
        run_id,
        authorization_ref,
        identity,
        binding,
        safe_failure_contract_ref,
        committed_request,
    }
}

struct CommittedFactSelection {
    store: RunHistoryWriter<InMemoryRunJournalBackend>,
    issuer: super::RunAccessAuthorityIssuer,
    store_identity: StoreIdentity,
    fixture: LegalAdmissionFixture,
    run_id: mfm_ids::RunId,
    authorization_ref: mfm_journal::AuthorizationRef,
    response_contract: RetainedValueContract,
    attestation_contract: RetainedValueContract,
}

async fn commit_fact_selection(discriminator: u8) -> CommittedFactSelection {
    let contracts = fact_selection_contracts(
        discriminator,
        None,
        CanonicalFactPredicate::from_canonical_value(CanonicalValue::String(
            "no-selected-facts".to_owned(),
        ))
        .expect("fact predicate"),
        FactOrdering::Ascending,
        1,
    )
    .expect("fact fixture contracts");
    let routing_generation_ref = contracts.routing_generation_ref;
    let request_contract = contracts.request_contract;
    let response_contract = contracts.response_contract;
    let attestation_contract = contracts.attestation_contract;
    let request = contracts.request;
    let mismatched_request = FactSelectionRequest::new(vec![FactSelectionQuery::new(
        routing_generation_ref.clone(),
        CanonicalFactPredicate::from_canonical_value(CanonicalValue::String(
            "different-request".to_owned(),
        ))
        .expect("different predicate"),
        None,
        FactOrdering::Ascending,
        FactSelectionLimit::new(1).expect("fact limit"),
        FactTieBreak::FactIdentityAscending,
    )
    .expect("different fact query")])
    .expect("different fact request");
    let fixture = LegalAdmissionFixture::new(discriminator)
        .expect("fixture")
        .with_read_execution(contracts.execution);
    let store_identity = fixture.store_identity().clone();
    let (store, issuer) = open_in_memory(store_identity.clone());
    fixture
        .provision_in_memory(&store)
        .expect("configured value");
    let support = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fixture");
    let (store, reader) = store.split();
    let prepared = fixture
        .prepare_on(&store, &reader, &issuer, &support)
        .await
        .expect("prepare admission");
    let (admit, append) = prepared.into_parts();
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = store
        .append_admission(&admit, append)
        .await
        .expect("append admission")
    else {
        panic!("new admission");
    };
    let run_id = admitted.run_id().clone();
    let drive = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = store
        .load_for_drive(&drive)
        .await
        .expect("load admitted run")
        .verify_recorded_history()
        .expect("verify admitted run");
    let node_id = view.certified_spec().nodes()[0].node_id().clone();
    let frame = store
        .prepare_frame(&drive, &view, &node_id)
        .await
        .expect("prepare fact frame");
    let mismatched_append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("fact-authorize-mismatch-{discriminator:02x}"))
                .expect("append request"),
            ExistingRunAppendMaterial::Authorization(Box::new(AuthorizationMaterial::Read {
                prepared_frame: Box::new(frame),
                immutable_request_root: Box::new(ProducedObjectRoot::new(
                    request_contract.clone(),
                    PlainCanonicalJsonBytes::from_canonical_json_slice(request.canonical_json())
                        .expect("fact request bytes"),
                )),
                routing_generation_ref: routing_generation_ref.clone(),
            })),
        )
        .expect("prepare mismatched fact authorization");
    let PreparedJournalAppend::AuthorizeExternalAccess(mismatched_append) = mismatched_append
    else {
        panic!("fact authorization append");
    };
    let mismatch_error = match store
        .append_fact_selection_authorization(&drive, mismatched_append, mismatched_request)
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("distinct fact request unexpectedly authorized"),
    };
    assert_eq!(mismatch_error, StoreError::FactScanBindingMismatch);
    let frame = store
        .prepare_frame(&drive, &view, &node_id)
        .await
        .expect("prepare fact frame");
    let append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("fact-authorize-{discriminator:02x}"))
                .expect("append request"),
            ExistingRunAppendMaterial::Authorization(Box::new(AuthorizationMaterial::Read {
                prepared_frame: Box::new(frame),
                immutable_request_root: Box::new(ProducedObjectRoot::new(
                    request_contract,
                    PlainCanonicalJsonBytes::from_canonical_json_slice(request.canonical_json())
                        .expect("fact request bytes"),
                )),
                routing_generation_ref,
            })),
        )
        .expect("prepare fact authorization");
    let PreparedJournalAppend::AuthorizeExternalAccess(append) = append else {
        panic!("fact authorization append");
    };
    let FactSelectionAuthorizationOutcome::NewlyAuthorized(permit) = store
        .append_fact_selection_authorization(&drive, append, request)
        .await
        .expect("append fact authorization")
    else {
        panic!("new fact authorization");
    };
    let authorization_ref = permit.authorization_ref().clone();
    let completed = store
        .scan_fact_selection(*permit)
        .await
        .expect("complete fact scan");
    let view = store
        .load_for_drive(&drive)
        .await
        .expect("load fact authorization")
        .verify_recorded_history()
        .expect("verify fact authorization");
    let append = store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("fact-observe-{discriminator:02x}"))
                .expect("append request"),
            ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::FactSelection {
                sealed: Box::new(completed.seal()),
            })),
        )
        .expect("prepare fact observation");
    assert!(matches!(
        store
            .append(&drive, append)
            .await
            .expect("append fact observation"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    store
        .load_for_drive(&drive)
        .await
        .expect("reload fact observation")
        .verify_recorded_history()
        .expect("replay fact observation");
    CommittedFactSelection {
        store,
        issuer,
        store_identity,
        fixture,
        run_id,
        authorization_ref,
        response_contract,
        attestation_contract,
    }
}

async fn verified_pending_effect(fixture: &AuthorizedEffect) -> mfm_executor::VerifiedEnsureResult {
    let ledger = KeyedExecutorLedger::new(
        MemoryExecutorStore::new(&fixture.binding),
        fixture.binding.clone(),
    )
    .expect("executor ledger");
    let effect = ledger
        .bind_effect(&fixture.identity)
        .await
        .expect("bind effect");
    let audit = effect.delivery_audit().clone();
    let head = audit.head_ref().expect("delivery audit head");
    let closure = ExecutorRetainedClosureClaim::from_delivery_audit(&audit, &fixture.binding)
        .expect("retained closure");
    verify_ensure_result(
        fixture.identity.clone(),
        &fixture.binding,
        ExecutorEnsureResultClaim::pending(head),
        closure,
    )
    .expect("verified pending result")
}

async fn verified_terminal_effect(
    fixture: &AuthorizedEffect,
) -> mfm_executor::VerifiedEnsureResult {
    let destination = MemoryConvergentDestination::new();
    destination
        .activate_generation(fixture.safe_failure_contract_ref.clone())
        .expect("activate executor generation");
    let ledger = KeyedExecutorLedger::new(
        MemoryExecutorStore::new(&fixture.binding),
        fixture.binding.clone(),
    )
    .expect("executor ledger");
    let reviewed_operation = RecoverabilityContract::embedded()
        .expect("recoverability contract")
        .encode(
            "mfm.primitive-stable_id.v1",
            &CanonicalValue::String("mfm.store-test/enqueue".to_owned()),
        )
        .expect("reviewed enqueue operation");
    let reference_contract = ReferenceContract::new(
        fixture.safe_failure_contract_ref.clone(),
        SchemaQualifiedCanonicalValue::from_validated(&reviewed_operation)
            .expect("reviewed enqueue value"),
        "applied",
        fixture.safe_failure_contract_ref.clone(),
        fixture.safe_failure_contract_ref.clone(),
    )
    .expect("reference contract");
    let executor = ReferenceExecutor::new(ledger, destination, reference_contract);
    executor
        .drive(&fixture.committed_request)
        .await
        .expect("terminal executor result")
}

async fn append_effect_result(
    fixture: &AuthorizedEffect,
    append_request_id: &str,
    result: mfm_executor::VerifiedEnsureResult,
) {
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized effect")
        .verify_recorded_history()
        .expect("verify authorized effect");
    let append = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(append_request_id).expect("append request"),
            ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::EnsureEffect {
                authorization_ref: fixture.authorization_ref.clone(),
                outcome: Box::new(EffectExecutorOutcome::returned(result)),
            })),
        )
        .expect("prepare returned effect observation");
    assert!(matches!(
        fixture
            .store
            .append(&drive, append)
            .await
            .expect("append returned effect observation"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
}

fn append_raw_diagnostic_observation(
    store_identity: &StoreIdentity,
    authorization_ref: &mfm_journal::AuthorizationRef,
    diagnostic_contract: &RetainedValueContract,
    metadata: &SafeFailureMetadata,
    commits: &mut Vec<CommittedJournalCommit>,
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
) {
    let prior = commits.last().expect("authorization commit");
    let prior_envelope = prior.envelope().fields().expect("prior envelope");
    let predecessor =
        JournalPredecessor::journal_head(&prior.envelope().journal_head().expect("prior head"))
            .expect("predecessor");
    let run_sequence = prior_envelope
        .core
        .run_sequence
        .checked_add(1)
        .expect("run sequence");
    let producer = ProducerBinding::external_observation(
        authorization_ref,
        &field_path("outcome.safe_failure.diagnostic_ref"),
    )
    .expect("diagnostic producer");
    let diagnostic_ref =
        derive_value_ref(diagnostic_contract, &producer, bytes).expect("diagnostic ref");
    let failure = SafeFailure::new(
        metadata.safe_failure_contract_ref(),
        metadata.stable_code(),
        metadata.failure_class(),
        metadata.boundary_stage(),
        metadata.coarse_size_class(),
        Some(&diagnostic_ref),
    )
    .expect("safe failure");
    let outcome = ObservationOutcome::indeterminate(&failure).expect("outcome");
    let observation =
        ExternalAccessObserved::new(authorization_ref, &outcome, None).expect("observation");
    let payload =
        RunJournalRecord::external_access_observed(&observation).expect("observation record");
    let logical_key = RecordLogicalKey::observation(authorization_ref).expect("logical key");
    let candidate =
        CandidateRecordEnvelope::new(0, payload.schema_id(), &logical_key, &payload, false)
            .expect("candidate");
    let reference = diagnostic_ref.fields().expect("diagnostic fields");
    let bindings = vec![ObjectPathBinding::new(
        0,
        &field_path("outcome.safe_failure.diagnostic_ref"),
        AuthorityUse::ProducedHere,
        &diagnostic_ref,
        &reference.evidence_contract_ref,
    )
    .expect("object binding")];
    let intents = vec![ArtifactAdmissionIntent::new(
        &diagnostic_ref,
        &reference.evidence_contract_ref,
        ArtifactAdmissionMode::AdmitOrVerifyExact,
    )
    .expect("admission intent")];
    let candidate_digest = CommitCandidatePreimage::new(
        &predecessor,
        BatchPurpose::ExternalAccessObservation,
        std::slice::from_ref(&candidate),
        &bindings,
        &intents,
    )
    .expect("candidate preimage")
    .candidate_digest()
    .expect("candidate digest");
    let record_hash = RecordHashPreimage::from_candidate(&candidate)
        .expect("record hash preimage")
        .record_hash()
        .expect("record hash");
    let record_id = RecordIdPreimage::new(
        store_identity.store_scope_id(),
        &prior_envelope.core.run_id,
        run_sequence,
        0,
        &record_hash,
    )
    .expect("record id preimage")
    .record_id()
    .expect("record id");
    let coordinate = TenantFactCoordinate::none().expect("fact coordinate");
    let append_request_id =
        AppendRequestId::new("raw-hostile-observation").expect("append request");
    let commit_digest = CommitDigestPreimage::new(
        store_identity.store_scope_id(),
        &prior_envelope.core.run_id,
        run_sequence,
        &predecessor,
        &append_request_id,
        &candidate_digest,
        &coordinate,
        std::slice::from_ref(&record_hash),
        &bindings,
        &intents,
    )
    .expect("commit preimage")
    .commit_digest()
    .expect("commit digest");
    let envelope = CommitEnvelope::new(
        store_identity.store_scope_id(),
        &prior_envelope.core.run_id,
        run_sequence,
        &predecessor,
        &append_request_id,
        &candidate_digest,
        &commit_digest,
        &coordinate,
        std::slice::from_ref(&record_hash),
        &bindings,
        &intents,
        prior_envelope.committed_at + 1,
    )
    .expect("commit envelope");
    commits.push(CommittedJournalCommit::from_persisted(
        envelope,
        vec![CommittedJournalRecord::from_persisted(
            record_id,
            record_hash,
            candidate,
        )],
    ));
    objects.push(
        CommittedObject::from_persisted(diagnostic_ref, bytes.to_vec()).expect("diagnostic object"),
    );
}

fn observation_material(
    fixture: &AuthorizedRead,
    diagnostic: Option<PlainCanonicalJsonBytes>,
) -> ExistingRunAppendMaterial {
    ExistingRunAppendMaterial::Observation(Box::new(read_observation_material(fixture, diagnostic)))
}

fn read_observation_material(
    fixture: &AuthorizedRead,
    diagnostic: Option<PlainCanonicalJsonBytes>,
) -> ObservationMaterial {
    let outcome = match diagnostic {
        Some(diagnostic) => ReadObservationMaterial::Indeterminate {
            diagnostic_root: Some(ProducedObjectRoot::new(
                fixture.diagnostic_contract.clone(),
                diagnostic,
            )),
            metadata: fixture.metadata.clone(),
        },
        None => ReadObservationMaterial::DidNotEnter {
            diagnostic_root: None,
            metadata: fixture.metadata.clone(),
        },
    };
    ObservationMaterial::Read {
        authorization_ref: fixture.authorization_ref.clone(),
        outcome: Box::new(outcome),
    }
}

#[derive(Debug, Clone, Copy)]
enum LateObservationCase {
    Returned,
    DidNotEnter,
    Indeterminate,
    NonDomainFailure,
}

async fn append_additional_read_authorization(
    fixture: &AuthorizedRead,
    label: &str,
) -> mfm_journal::AuthorizationRef {
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load read for additional authorization")
        .verify_recorded_history()
        .expect("verify read for additional authorization");
    let node_id = view.certified_spec().nodes()[0].node_id().clone();
    let frame = fixture
        .store
        .prepare_frame(&drive, &view, &node_id)
        .await
        .expect("prepare additional read frame");
    let append = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(label).expect("additional authorization append id"),
            ExistingRunAppendMaterial::Authorization(Box::new(AuthorizationMaterial::Read {
                prepared_frame: Box::new(frame),
                immutable_request_root: Box::new(ProducedObjectRoot::new(
                    fixture.request_contract.clone(),
                    canonical("{}"),
                )),
                routing_generation_ref: fixture.routing_generation_ref.clone(),
            })),
        )
        .expect("prepare additional read authorization");
    let AppendOutcome::NewlyAppended(NewlyAppended::Authorization(authorization)) = fixture
        .store
        .append(&drive, append)
        .await
        .expect("append additional read authorization")
    else {
        panic!("additional read authorization must be new")
    };
    authorization.authorization_ref().clone()
}

async fn append_returned_read_observation(
    fixture: &AuthorizedRead,
    authorization_ref: &mfm_journal::AuthorizationRef,
    label: &str,
) -> ObservationRef {
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load read for returned observation")
        .verify_recorded_history()
        .expect("verify read for returned observation");
    let append = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(label).expect("returned observation append id"),
            ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::Read {
                authorization_ref: authorization_ref.clone(),
                outcome: Box::new(ReadObservationMaterial::Returned {
                    returned_root: ProducedObjectRoot::new(
                        fixture.returned_contract.clone(),
                        canonical(r#"{"value":"returned"}"#),
                    ),
                }),
            })),
        )
        .expect("prepare returned read observation");
    let AppendOutcome::NewlyAppended(NewlyAppended::Observation(committed)) = fixture
        .store
        .append(&drive, append)
        .await
        .expect("append returned read observation")
    else {
        panic!("returned read observation must be new")
    };
    ObservationRef::new(&committed.record_refs()[0]).expect("observation ref")
}

async fn close_read_run(fixture: &AuthorizedRead, observation_ref: ObservationRef, label: &str) {
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load read for settlement")
        .verify_recorded_history()
        .expect("verify read for settlement");
    let [node] = view.certified_spec().nodes() else {
        panic!("one-node fixture")
    };
    let [output_slot] = node.settlement_contract().output_slots() else {
        panic!("one-output fixture")
    };
    let frame = fixture
        .store
        .prepare_frame(&drive, &view, node.node_id())
        .await
        .expect("prepare settlement frame");
    let append = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new(label).expect("settlement append id"),
            ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::ReadSettled {
                prepared_frame: Box::new(frame),
                immutable_request_ref: fixture.request_ref.clone(),
                consumed_observation_ref: observation_ref,
                settlement: SettlementMaterial::Succeeded {
                    output_roots: vec![ProducedOutputSlot::new(
                        output_slot.output_ordinal(),
                        output_slot.field_path().clone(),
                        ProducedObjectRoot::new(
                            output_slot.value_contract().clone(),
                            canonical(r#"{"result":"late-tail-close"}"#),
                        ),
                    )],
                    fact_roots: Vec::new(),
                },
                object_graph: ObjectGraphProposal::empty(),
            })),
        )
        .expect("prepare closing settlement");
    assert!(matches!(
        fixture
            .store
            .append(&drive, append)
            .await
            .expect("append closing settlement"),
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
    ));
}

fn late_read_observation_material(
    fixture: &AuthorizedRead,
    authorization_ref: &mfm_journal::AuthorizationRef,
    case: LateObservationCase,
) -> ObservationMaterial {
    let outcome = match case {
        LateObservationCase::Returned => ReadObservationMaterial::Returned {
            returned_root: ProducedObjectRoot::new(
                fixture.returned_contract.clone(),
                canonical(r#"{"value":"late-returned"}"#),
            ),
        },
        LateObservationCase::DidNotEnter => ReadObservationMaterial::DidNotEnter {
            diagnostic_root: None,
            metadata: fixture
                .did_not_enter_metadata
                .clone()
                .unwrap_or_else(|| fixture.metadata.clone()),
        },
        LateObservationCase::Indeterminate => ReadObservationMaterial::Indeterminate {
            diagnostic_root: Some(ProducedObjectRoot::new(
                fixture.diagnostic_contract.clone(),
                canonical(VALID_DIAGNOSTIC),
            )),
            metadata: fixture.metadata.clone(),
        },
        LateObservationCase::NonDomainFailure => ReadObservationMaterial::NonDomainFailure {
            failure: NonDomainFailure::new(
                NonDomainEntryStatus::MayHaveEntered,
                NonDomainDisposition::IntegrityBlocked,
                NonDomainFailureCode::AdapterContractViolation,
            )
            .expect("read-layer non-domain failure"),
        },
    };
    ObservationMaterial::Read {
        authorization_ref: authorization_ref.clone(),
        outcome: Box::new(outcome),
    }
}

#[tokio::test]
async fn every_legal_observation_outcome_closes_one_preclosure_authorization_after_closure() {
    let fixture = authorize_read(true, 70).await;
    let cases = [
        LateObservationCase::Returned,
        LateObservationCase::DidNotEnter,
        LateObservationCase::Indeterminate,
        LateObservationCase::NonDomainFailure,
    ];
    let mut late_authorizations = Vec::new();
    for index in 0..cases.len() {
        late_authorizations.push(
            append_additional_read_authorization(&fixture, &format!("late-tail-authorize-{index}"))
                .await,
        );
    }
    let consumed_observation = append_returned_read_observation(
        &fixture,
        &fixture.authorization_ref,
        "late-tail-observe-consumed",
    )
    .await;
    close_read_run(&fixture, consumed_observation, "late-tail-close").await;

    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let closed = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load closed run")
        .verify_recorded_history()
        .expect("verify closed run");
    assert_eq!(closed.run_phase(), RunPhase::Closed);
    assert_eq!(closed.unobserved_authorizations().count(), cases.len());
    let semantic_head = closed.semantic_head().clone();
    let mut prior_journal_head = closed.journal_head().clone();

    for (index, (case, late_authorization_ref)) in
        cases.into_iter().zip(late_authorizations).enumerate()
    {
        let current = fixture
            .store
            .load_for_drive(&drive)
            .await
            .expect("load current late tail")
            .verify_recorded_history()
            .expect("verify current late tail");
        let append =
            fixture
                .store
                .prepare_append(
                    &drive,
                    &current,
                    AppendRequestId::new(format!("late-tail-observe-{index}"))
                        .expect("late observation append id"),
                    ExistingRunAppendMaterial::Observation(Box::new(
                        late_read_observation_material(&fixture, &late_authorization_ref, case),
                    )),
                )
                .expect("prepare legal late observation");
        assert!(matches!(
            fixture
                .store
                .append(&drive, append)
                .await
                .expect("append legal late observation"),
            AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
        ));

        let observed = fixture
            .store
            .load_for_drive(&drive)
            .await
            .expect("reload late observation")
            .verify_recorded_history()
            .expect("verify late observation");
        assert_eq!(observed.run_phase(), RunPhase::Closed);
        assert_eq!(observed.semantic_head(), &semantic_head);
        assert_ne!(observed.journal_head(), &prior_journal_head);
        prior_journal_head = observed.journal_head().clone();
        assert_eq!(
            observed.unobserved_authorizations().count(),
            cases.len() - index - 1
        );

        let duplicate_error = match fixture.store.prepare_append(
            &drive,
            &observed,
            AppendRequestId::new(format!("late-tail-duplicate-{index}"))
                .expect("duplicate append id"),
            ExistingRunAppendMaterial::Observation(Box::new(late_read_observation_material(
                &fixture,
                &late_authorization_ref,
                case,
            ))),
        ) {
            Err(error) => error,
            Ok(_) => panic!("one authorization cannot receive a second late observation"),
        };
        assert_eq!(duplicate_error, StoreError::ObservationAlreadyCommitted);
    }
}

#[tokio::test]
async fn exact_resolution_closes_an_identical_observation_before_and_after_acknowledgement() {
    let fixture = authorize_read(false, 58).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized run")
        .verify_recorded_history()
        .expect("verify authorized run");
    let prepared = fixture
        .store
        .prepare_observation_resolution(
            &drive,
            &view,
            AppendRequestId::new("exact-identical-observation").expect("append request"),
            read_observation_material(&fixture, None),
        )
        .expect("prepare exact observation");
    let logical = prepared
        .logical_observation_identity()
        .expect("logical identity")
        .expect("observation identity");
    let physical = prepared.physical_identity();
    assert_eq!(
        view.resolve_logical_observation(&logical)
            .expect("resolve absent logical"),
        ExactResolution::Absent
    );
    let absent = view
        .resolve_physical_append(&physical)
        .expect("resolve absent physical");
    assert_eq!(absent.exact(), &ExactResolution::Absent);
    assert_eq!(absent.current_head(), view.journal_head());

    assert!(matches!(
        fixture
            .store
            .append(&drive, prepared)
            .await
            .expect("append observation"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    let committed_view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload observed run")
        .verify_recorded_history()
        .expect("verify observed run");
    let ExactResolution::Identical(resolved) = committed_view
        .resolve_logical_observation(&logical)
        .expect("resolve committed logical")
    else {
        panic!("logical observation must resolve identically");
    };
    assert_eq!(resolved.journal_head(), committed_view.journal_head());
    let physical_resolution = committed_view
        .resolve_physical_append(&physical)
        .expect("resolve committed physical");
    assert!(matches!(
        physical_resolution.exact(),
        ExactResolution::Identical(_)
    ));
    assert_eq!(
        physical_resolution.current_head(),
        committed_view.journal_head()
    );
}

#[tokio::test]
async fn exact_resolution_prefers_logical_identity_across_stale_physical_attempts() {
    let fixture = authorize_read(false, 59).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized run")
        .verify_recorded_history()
        .expect("verify authorized run");
    let stale = fixture
        .store
        .prepare_observation_resolution(
            &drive,
            &view,
            AppendRequestId::new("stale-physical-observation").expect("append request"),
            read_observation_material(&fixture, None),
        )
        .expect("prepare stale candidate");
    let logical = stale
        .logical_observation_identity()
        .expect("logical identity")
        .expect("observation identity");
    let physical = stale.physical_identity();
    let competing = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new("winning-physical-observation").expect("append request"),
            observation_material(&fixture, None),
        )
        .expect("prepare competing candidate");
    assert!(matches!(
        fixture
            .store
            .append(&drive, competing)
            .await
            .expect("append competing candidate"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    let committed_view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload observed run")
        .verify_recorded_history()
        .expect("verify observed run");
    assert!(matches!(
        committed_view
            .resolve_logical_observation(&logical)
            .expect("resolve logical observation"),
        ExactResolution::Identical(_)
    ));
    let stale_resolution = committed_view
        .resolve_physical_append(&physical)
        .expect("resolve stale physical candidate");
    assert_eq!(stale_resolution.exact(), &ExactResolution::Absent);
    assert_ne!(stale_resolution.current_head(), view.journal_head());
}

#[tokio::test]
async fn exact_resolution_rejects_different_logical_and_physical_content() {
    let fixture = authorize_read(false, 60).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized run")
        .verify_recorded_history()
        .expect("verify authorized run");
    let append_request_id =
        AppendRequestId::new("conflicting-observation").expect("append request");
    let expected = fixture
        .store
        .prepare_observation_resolution(
            &drive,
            &view,
            append_request_id.clone(),
            read_observation_material(&fixture, None),
        )
        .expect("prepare expected candidate");
    let logical = expected
        .logical_observation_identity()
        .expect("logical identity")
        .expect("observation identity");
    let physical = expected.physical_identity();
    let conflicting = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            append_request_id,
            ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::Read {
                authorization_ref: fixture.authorization_ref.clone(),
                outcome: Box::new(ReadObservationMaterial::Returned {
                    returned_root: ProducedObjectRoot::new(
                        fixture.returned_contract.clone(),
                        canonical(r#"{"value":"different"}"#),
                    ),
                }),
            })),
        )
        .expect("prepare conflicting candidate");
    assert!(matches!(
        fixture
            .store
            .append(&drive, conflicting)
            .await
            .expect("append conflicting candidate"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    let committed_view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload conflicting run")
        .verify_recorded_history()
        .expect("verify conflicting run");
    assert_eq!(
        committed_view
            .resolve_logical_observation(&logical)
            .expect("resolve logical conflict"),
        ExactResolution::Conflict
    );
    assert_eq!(
        committed_view
            .resolve_physical_append(&physical)
            .expect("resolve physical conflict")
            .exact(),
        &ExactResolution::Conflict
    );
}

#[test]
fn sequence_reservation_keeps_one_slot_for_every_unmatched_authorization() {
    use super::journal::validate_successor_sequence_capacity;

    assert_eq!(
        validate_successor_sequence_capacity(u64::MAX, 0, 0),
        Err(StoreError::SequenceOverflow)
    );
    assert_eq!(
        validate_successor_sequence_capacity(u64::MAX - 1, 0, 1),
        Err(StoreError::SequenceOverflow)
    );
    assert_eq!(
        validate_successor_sequence_capacity(u64::MAX - 2, 0, 1),
        Ok(())
    );
    assert_eq!(
        validate_successor_sequence_capacity(u64::MAX - 1, 1, -1),
        Ok(())
    );
    assert_eq!(
        validate_successor_sequence_capacity(u64::MAX - 1, 1, 0),
        Err(StoreError::SequenceOverflow)
    );
    assert_eq!(
        validate_successor_sequence_capacity(u64::MAX - 2, 1, 0),
        Ok(())
    );
}

fn replace_observation_diagnostic(
    store_identity: &StoreIdentity,
    diagnostic_contract: &RetainedValueContract,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
) {
    let last = commits.last().expect("observation commit").clone();
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().fields().expect("candidate");
    let RunJournalRecordFields::ExternalAccessObserved(observation) =
        candidate.payload.fields().expect("record")
    else {
        panic!("observation record");
    };
    let observation_fields = observation.fields().expect("observation");
    let mfm_journal::ObservationOutcomeFields::Indeterminate { safe_failure } =
        observation_fields.outcome.fields().expect("outcome")
    else {
        panic!("indeterminate observation");
    };
    let failure = safe_failure.fields().expect("safe failure");
    let old_ref = failure.diagnostic_ref.expect("diagnostic ref");
    let producer = old_ref.fields().expect("value ref").producer_binding;
    let new_ref =
        derive_value_ref(diagnostic_contract, &producer, bytes).expect("recomputed value ref");
    assert_ne!(new_ref, old_ref);

    let new_failure = SafeFailure::new(
        &failure.safe_failure_contract_ref,
        &failure.stable_code,
        failure.failure_class,
        failure.boundary_stage,
        failure.coarse_size_class,
        Some(&new_ref),
    )
    .expect("rewritten safe failure");
    let new_outcome = ObservationOutcome::indeterminate(&new_failure).expect("rewritten outcome");
    let new_observation =
        ExternalAccessObserved::new(&observation_fields.authorization_ref, &new_outcome, None)
            .expect("rewritten observation");
    let new_payload =
        RunJournalRecord::external_access_observed(&new_observation).expect("rewritten record");
    replace_single_observation_object(
        store_identity,
        commits,
        objects,
        &old_ref,
        new_ref,
        bytes,
        new_payload,
    );
}

fn replace_single_observation_object(
    store_identity: &StoreIdentity,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    old_ref: &mfm_journal::ValueRef,
    new_ref: mfm_journal::ValueRef,
    bytes: &[u8],
    new_payload: RunJournalRecord,
) {
    replace_observation_objects(
        store_identity,
        commits,
        objects,
        vec![(old_ref.clone(), new_ref, bytes.to_vec(), 1)],
        new_payload,
    );
}

fn replace_observation_objects(
    store_identity: &StoreIdentity,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    replacements: Vec<(mfm_journal::ValueRef, mfm_journal::ValueRef, Vec<u8>, usize)>,
    new_payload: RunJournalRecord,
) {
    let last = commits.last().expect("observation commit").clone();
    let envelope = last.envelope().fields().expect("envelope");
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().fields().expect("candidate");
    let new_candidate = CandidateRecordEnvelope::new(
        candidate.ordinal,
        &candidate.schema_id,
        &candidate.logical_key,
        &new_payload,
        candidate.emits_facts,
    )
    .expect("rewritten candidate");

    let mut binding_replacements = vec![0; replacements.len()];
    let mut bindings = envelope
        .core
        .ordered_object_bindings
        .iter()
        .map(|binding| {
            let fields = binding.fields().expect("object binding");
            let replacement_index = replacements
                .iter()
                .position(|(old_ref, _, _, _)| fields.value_ref == *old_ref);
            let (value_ref, evidence_contract_ref) = replacement_index
                .map(|index| {
                    binding_replacements[index] += 1;
                    let new_ref = &replacements[index].1;
                    let new_fields = new_ref.fields().expect("replacement value ref");
                    (new_ref, new_fields.evidence_contract_ref)
                })
                .unwrap_or((&fields.value_ref, fields.evidence_contract_ref.clone()));
            ObjectPathBinding::new(
                fields.record_ordinal,
                &fields.field_path,
                fields.authority_use,
                value_ref,
                &evidence_contract_ref,
            )
            .expect("rewritten binding")
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let mut intent_replacements = vec![0; replacements.len()];
    let mut intents = envelope
        .core
        .artifact_admission_intents
        .iter()
        .map(|intent| {
            let fields = intent.fields().expect("admission intent");
            let replacement_index = replacements
                .iter()
                .position(|(old_ref, _, _, _)| fields.value_ref == *old_ref);
            let (value_ref, evidence_contract_ref) = replacement_index
                .map(|index| {
                    intent_replacements[index] += 1;
                    let new_ref = &replacements[index].1;
                    (
                        new_ref,
                        new_ref
                            .fields()
                            .expect("replacement value ref")
                            .evidence_contract_ref,
                    )
                })
                .unwrap_or((&fields.value_ref, fields.evidence_contract_ref.clone()));
            ArtifactAdmissionIntent::new(value_ref, &evidence_contract_ref, fields.mode)
                .expect("rewritten intent")
        })
        .collect::<Vec<_>>();
    intents.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    assert!(binding_replacements
        .iter()
        .zip(&replacements)
        .all(|(count, (_, _, _, expected))| count == expected));
    assert!(intent_replacements.iter().all(|count| *count == 1));

    let candidate_digest = CommitCandidatePreimage::new(
        &envelope.core.predecessor,
        BatchPurpose::ExternalAccessObservation,
        std::slice::from_ref(&new_candidate),
        &bindings,
        &intents,
    )
    .expect("candidate preimage")
    .candidate_digest()
    .expect("candidate digest");
    let record_hash = RecordHashPreimage::from_candidate(&new_candidate)
        .expect("record hash preimage")
        .record_hash()
        .expect("record hash");
    let record_id = RecordIdPreimage::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        candidate.ordinal,
        &record_hash,
    )
    .expect("record id preimage")
    .record_id()
    .expect("record id");
    let record =
        CommittedJournalRecord::from_persisted(record_id, record_hash.clone(), new_candidate);
    let commit_digest = CommitDigestPreimage::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &bindings,
        &intents,
    )
    .expect("commit preimage")
    .commit_digest()
    .expect("commit digest");
    let envelope = CommitEnvelope::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &commit_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &bindings,
        &intents,
        envelope.committed_at,
    )
    .expect("rewritten envelope");
    *commits.last_mut().expect("observation commit") =
        CommittedJournalCommit::from_persisted(envelope, vec![record]);

    for (old_ref, new_ref, bytes, _) in replacements {
        let prior_len = objects.len();
        objects.retain(|object| object.value_ref() != &old_ref);
        assert_eq!(objects.len() + 1, prior_len);
        objects.push(
            CommittedObject::from_persisted(new_ref, bytes).expect("rewritten observation object"),
        );
    }
}

fn replace_observation_result(
    store_identity: &StoreIdentity,
    replacement_contract: &RetainedValueContract,
    replacement_producer: ProducerBinding,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
) {
    let last = commits.last().expect("observation commit").clone();
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().fields().expect("candidate");
    let RunJournalRecordFields::ExternalAccessObserved(observation) =
        candidate.payload.fields().expect("record")
    else {
        panic!("observation record");
    };
    let observation_fields = observation.fields().expect("observation");
    let mfm_journal::ObservationOutcomeFields::Returned {
        result_ref: old_ref,
    } = observation_fields.outcome.fields().expect("outcome")
    else {
        panic!("returned observation");
    };
    let old_object = objects
        .iter()
        .find(|object| object.value_ref() == &old_ref)
        .expect("returned object");
    let bytes = old_object.bytes().to_vec();
    let new_ref = derive_value_ref(replacement_contract, &replacement_producer, &bytes)
        .expect("replacement result ref");
    assert_ne!(new_ref, old_ref);
    let outcome = ObservationOutcome::returned(&new_ref).expect("replacement outcome");
    let observation =
        ExternalAccessObserved::new(&observation_fields.authorization_ref, &outcome, None)
            .expect("replacement observation");
    let payload =
        RunJournalRecord::external_access_observed(&observation).expect("replacement record");
    replace_single_observation_object(
        store_identity,
        commits,
        objects,
        &old_ref,
        new_ref,
        &bytes,
        payload,
    );
}

fn replace_effect_pending_delivery_ref(
    store_identity: &StoreIdentity,
    binding: &VerifiedExecutorBinding,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    replacement_contract: &RetainedValueContract,
    replacement_path_prefix: &str,
) {
    let last = commits.last().expect("observation commit").clone();
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().fields().expect("candidate");
    let RunJournalRecordFields::ExternalAccessObserved(observation) =
        candidate.payload.fields().expect("record")
    else {
        panic!("observation record");
    };
    let observation_fields = observation.fields().expect("observation");
    let mfm_journal::ObservationOutcomeFields::Returned {
        result_ref: old_result_ref,
    } = observation_fields.outcome.fields().expect("outcome")
    else {
        panic!("returned observation");
    };
    let result_bytes = objects
        .iter()
        .find(|object| object.value_ref() == &old_result_ref)
        .expect("ensure-result object")
        .bytes()
        .to_vec();
    let result = ExecutorEnsureResult::strict_decode(&result_bytes).expect("ensure-result bytes");
    let ExecutorEnsureResultFields::Pending { delivery_audit_ref } =
        result.fields().expect("ensure-result fields")
    else {
        panic!("pending ensure result");
    };
    let delivery_bytes = objects
        .iter()
        .find(|object| object.value_ref() == &delivery_audit_ref)
        .expect("delivery-audit object")
        .bytes()
        .to_vec();
    let digest = delivery_audit_ref
        .fields()
        .expect("delivery-audit ref")
        .content_digest
        .digest()
        .to_owned();
    let replacement_producer = ProducerBinding::external_observation(
        &observation_fields.authorization_ref,
        &field_path(&format!("{replacement_path_prefix}.{digest}")),
    )
    .expect("replacement retained producer");
    let replacement_delivery_ref =
        derive_value_ref(replacement_contract, &replacement_producer, &delivery_bytes)
            .expect("replacement delivery ref");
    assert_ne!(replacement_delivery_ref, delivery_audit_ref);
    let replacement_result =
        ExecutorEnsureResult::pending(&replacement_delivery_ref).expect("replacement result");
    let result_producer = old_result_ref
        .fields()
        .expect("result ref")
        .producer_binding;
    let ensure_contract = binding
        .contract()
        .retained_closure_contract()
        .ensure_result_contract();
    let replacement_result_ref = derive_value_ref(
        ensure_contract,
        &result_producer,
        replacement_result.as_bytes(),
    )
    .expect("replacement result ref");
    let outcome =
        ObservationOutcome::returned(&replacement_result_ref).expect("replacement outcome");
    let observation =
        ExternalAccessObserved::new(&observation_fields.authorization_ref, &outcome, None)
            .expect("replacement observation");
    let payload =
        RunJournalRecord::external_access_observed(&observation).expect("replacement record");
    replace_observation_objects(
        store_identity,
        commits,
        objects,
        vec![
            (
                delivery_audit_ref,
                replacement_delivery_ref,
                delivery_bytes,
                0,
            ),
            (
                old_result_ref,
                replacement_result_ref,
                replacement_result.as_bytes().to_vec(),
                1,
            ),
        ],
        payload,
    );
}

enum TerminalEffectMutation {
    OuterEvidenceWrongPath,
    TombstoneWrongPath,
    WrongRequestDigest,
}

fn replace_effect_terminal_chain(
    store_identity: &StoreIdentity,
    binding: &VerifiedExecutorBinding,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    mutation: TerminalEffectMutation,
) {
    let last = commits.last().expect("observation commit").clone();
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().fields().expect("candidate");
    let RunJournalRecordFields::ExternalAccessObserved(observation) =
        candidate.payload.fields().expect("record")
    else {
        panic!("observation record");
    };
    let observation_fields = observation.fields().expect("observation");
    let mfm_journal::ObservationOutcomeFields::Returned {
        result_ref: old_result_ref,
    } = observation_fields.outcome.fields().expect("outcome")
    else {
        panic!("returned observation");
    };
    let old_result_bytes = objects
        .iter()
        .find(|object| object.value_ref() == &old_result_ref)
        .expect("ensure-result object")
        .bytes()
        .to_vec();
    let result =
        ExecutorEnsureResult::strict_decode(&old_result_bytes).expect("ensure-result bytes");
    let ExecutorEnsureResultFields::Terminal {
        evidence_ref: old_evidence_ref,
    } = result.fields().expect("ensure-result fields")
    else {
        panic!("terminal ensure result");
    };
    let old_evidence_bytes = objects
        .iter()
        .find(|object| object.value_ref() == &old_evidence_ref)
        .expect("terminal-evidence object")
        .bytes()
        .to_vec();
    let evidence = TerminalEffectEvidence::strict_decode(&old_evidence_bytes)
        .expect("terminal-evidence bytes");
    let fields = evidence.fields().expect("terminal-evidence fields");
    let contracts = binding.contract().retained_closure_contract();
    let mut replacements = Vec::new();

    let (new_evidence_ref, new_evidence_bytes) = match mutation {
        TerminalEffectMutation::OuterEvidenceWrongPath => {
            let digest = old_evidence_ref
                .fields()
                .expect("terminal-evidence ref")
                .content_digest
                .digest()
                .to_owned();
            let producer = ProducerBinding::external_observation(
                &observation_fields.authorization_ref,
                &field_path(&format!("executor.frontier.{digest}")),
            )
            .expect("wrong terminal-evidence producer");
            let reference = derive_value_ref(
                contracts.terminal_evidence_contract(),
                &producer,
                &old_evidence_bytes,
            )
            .expect("wrong terminal-evidence ref");
            (reference, old_evidence_bytes.clone())
        }
        TerminalEffectMutation::TombstoneWrongPath => {
            let tombstone_bytes = objects
                .iter()
                .find(|object| object.value_ref() == &fields.terminal_tombstone_ref)
                .expect("terminal-tombstone object")
                .bytes()
                .to_vec();
            let digest = fields
                .terminal_tombstone_ref
                .fields()
                .expect("terminal-tombstone ref")
                .content_digest
                .digest()
                .to_owned();
            let producer = ProducerBinding::external_observation(
                &observation_fields.authorization_ref,
                &field_path(&format!("executor.domain_evidence.{digest}")),
            )
            .expect("wrong tombstone producer");
            let tombstone_ref = derive_value_ref(
                contracts.terminal_tombstone_contract(),
                &producer,
                &tombstone_bytes,
            )
            .expect("wrong tombstone ref");
            let replacement = TerminalEffectEvidence::new(
                &fields.executor_binding_ref,
                &fields.effect_key,
                &fields.request_digest,
                &fields.delivery_audit_ref,
                &tombstone_ref,
                &fields.external_operation_identity,
                &fields.terminal_outcome,
                &fields.assurance_policy_ref,
                &fields.proof_basis,
                &fields.domain_evidence_ref,
            )
            .expect("replacement terminal evidence");
            let evidence_producer = old_evidence_ref
                .fields()
                .expect("terminal-evidence ref")
                .producer_binding;
            let evidence_ref = derive_value_ref(
                contracts.terminal_evidence_contract(),
                &evidence_producer,
                replacement.as_bytes(),
            )
            .expect("replacement terminal-evidence ref");
            replacements.push((
                fields.terminal_tombstone_ref,
                tombstone_ref,
                tombstone_bytes,
                0,
            ));
            (evidence_ref, replacement.as_bytes().to_vec())
        }
        TerminalEffectMutation::WrongRequestDigest => {
            let request_digest =
                RequestDigest::from_digest(sha256_digest_bytes(b"wrong effect request"));
            let replacement = TerminalEffectEvidence::new(
                &fields.executor_binding_ref,
                &fields.effect_key,
                &request_digest,
                &fields.delivery_audit_ref,
                &fields.terminal_tombstone_ref,
                &fields.external_operation_identity,
                &fields.terminal_outcome,
                &fields.assurance_policy_ref,
                &fields.proof_basis,
                &fields.domain_evidence_ref,
            )
            .expect("replacement terminal evidence");
            let evidence_producer = old_evidence_ref
                .fields()
                .expect("terminal-evidence ref")
                .producer_binding;
            let evidence_ref = derive_value_ref(
                contracts.terminal_evidence_contract(),
                &evidence_producer,
                replacement.as_bytes(),
            )
            .expect("replacement terminal-evidence ref");
            (evidence_ref, replacement.as_bytes().to_vec())
        }
    };
    assert_ne!(new_evidence_ref, old_evidence_ref);
    let replacement_result =
        ExecutorEnsureResult::terminal(&new_evidence_ref).expect("replacement result");
    let result_producer = old_result_ref
        .fields()
        .expect("ensure-result ref")
        .producer_binding;
    let replacement_result_ref = derive_value_ref(
        contracts.ensure_result_contract(),
        &result_producer,
        replacement_result.as_bytes(),
    )
    .expect("replacement result ref");
    let outcome =
        ObservationOutcome::returned(&replacement_result_ref).expect("replacement outcome");
    let replacement_observation =
        ExternalAccessObserved::new(&observation_fields.authorization_ref, &outcome, None)
            .expect("replacement observation");
    let payload = RunJournalRecord::external_access_observed(&replacement_observation)
        .expect("replacement record");
    replacements.extend([
        (old_evidence_ref, new_evidence_ref, new_evidence_bytes, 0),
        (
            old_result_ref,
            replacement_result_ref,
            replacement_result.as_bytes().to_vec(),
            1,
        ),
    ]);
    replace_observation_objects(store_identity, commits, objects, replacements, payload);
}

fn remove_unbound_observation_object(
    store_identity: &StoreIdentity,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    producer_path_prefix: &str,
) {
    let removed_ref = objects
        .iter()
        .find_map(|object| {
            let fields = object.value_ref().fields().ok()?;
            let ProducerBindingFields::ExternalObservation { field_path, .. } =
                fields.producer_binding.fields().ok()?
            else {
                return None;
            };
            field_path
                .as_str()
                .starts_with(producer_path_prefix)
                .then(|| object.value_ref().clone())
        })
        .expect("unbound observation object");
    let last = commits.last().expect("observation commit").clone();
    let envelope = last.envelope().fields().expect("envelope");
    assert!(envelope
        .core
        .ordered_object_bindings
        .iter()
        .all(|binding| binding.fields().expect("binding").value_ref != removed_ref));
    let intents = envelope
        .core
        .artifact_admission_intents
        .iter()
        .filter(|intent| intent.fields().expect("intent").value_ref != removed_ref)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        intents.len() + 1,
        envelope.core.artifact_admission_intents.len()
    );
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().clone();
    let candidate_digest = CommitCandidatePreimage::new(
        &envelope.core.predecessor,
        BatchPurpose::ExternalAccessObservation,
        std::slice::from_ref(&candidate),
        &envelope.core.ordered_object_bindings,
        &intents,
    )
    .expect("candidate preimage")
    .candidate_digest()
    .expect("candidate digest");
    let record_hash = record.record_hash().clone();
    let commit_digest = CommitDigestPreimage::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &envelope.core.ordered_object_bindings,
        &intents,
    )
    .expect("commit preimage")
    .commit_digest()
    .expect("commit digest");
    let rewritten_envelope = CommitEnvelope::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &commit_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &envelope.core.ordered_object_bindings,
        &intents,
        envelope.committed_at,
    )
    .expect("rewritten envelope");
    *commits.last_mut().expect("observation commit") =
        CommittedJournalCommit::from_persisted(rewritten_envelope, vec![record.clone()]);
    let object_count = objects.len();
    objects.retain(|object| object.value_ref() != &removed_ref);
    assert_eq!(objects.len() + 1, object_count);
}

fn add_unbound_observation_object(
    store_identity: &StoreIdentity,
    authorization_ref: &mfm_journal::AuthorizationRef,
    contract: &RetainedValueContract,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
) {
    add_unbound_observation_object_at_path(
        store_identity,
        authorization_ref,
        contract,
        commits,
        objects,
        bytes,
        UnboundObservationIntent {
            producer_path: "outcome.unbound_extra",
            mode: ArtifactAdmissionMode::AdmitOrVerifyExact,
        },
    );
}

fn add_unbound_preexisting_observation_object(
    store_identity: &StoreIdentity,
    authorization_ref: &mfm_journal::AuthorizationRef,
    contract: &RetainedValueContract,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
) {
    add_unbound_observation_object_at_path(
        store_identity,
        authorization_ref,
        contract,
        commits,
        objects,
        bytes,
        UnboundObservationIntent {
            producer_path: "outcome.unbound_preexisting",
            mode: ArtifactAdmissionMode::RequireExisting,
        },
    );
}

struct UnboundObservationIntent<'a> {
    producer_path: &'a str,
    mode: ArtifactAdmissionMode,
}

fn add_unbound_observation_object_at_path(
    store_identity: &StoreIdentity,
    authorization_ref: &mfm_journal::AuthorizationRef,
    contract: &RetainedValueContract,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
    intent: UnboundObservationIntent<'_>,
) {
    let producer =
        ProducerBinding::external_observation(authorization_ref, &field_path(intent.producer_path))
            .expect("extra producer");
    add_unbound_observation_authority(
        store_identity,
        contract,
        commits,
        objects,
        bytes,
        producer,
        intent.mode,
    );
}

fn add_unbound_observation_authority(
    store_identity: &StoreIdentity,
    contract: &RetainedValueContract,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
    producer: ProducerBinding,
    mode: ArtifactAdmissionMode,
) {
    let extra_ref = derive_value_ref(contract, &producer, bytes).expect("extra ref");
    let extra_fields = extra_ref.fields().expect("extra fields");
    let last = commits.last().expect("observation commit").clone();
    let envelope = last.envelope().fields().expect("envelope");
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().clone();
    let mut intents = envelope.core.artifact_admission_intents.clone();
    intents.push(
        ArtifactAdmissionIntent::new(&extra_ref, &extra_fields.evidence_contract_ref, mode)
            .expect("extra intent"),
    );
    intents.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let candidate_digest = CommitCandidatePreimage::new(
        &envelope.core.predecessor,
        BatchPurpose::ExternalAccessObservation,
        std::slice::from_ref(&candidate),
        &envelope.core.ordered_object_bindings,
        &intents,
    )
    .expect("candidate preimage")
    .candidate_digest()
    .expect("candidate digest");
    let record_hash = record.record_hash().clone();
    let commit_digest = CommitDigestPreimage::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &envelope.core.ordered_object_bindings,
        &intents,
    )
    .expect("commit preimage")
    .commit_digest()
    .expect("commit digest");
    let rewritten_envelope = CommitEnvelope::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &commit_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &envelope.core.ordered_object_bindings,
        &intents,
        envelope.committed_at,
    )
    .expect("rewritten envelope");
    *commits.last_mut().expect("observation commit") =
        CommittedJournalCommit::from_persisted(rewritten_envelope, vec![record.clone()]);
    objects.push(CommittedObject::from_persisted(extra_ref, bytes.to_vec()).expect("extra object"));
}

fn inject_effect_safe_failure_diagnostic(
    store_identity: &StoreIdentity,
    diagnostic_contract: &RetainedValueContract,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
) {
    let last = commits.last().expect("observation commit").clone();
    let envelope = last.envelope().fields().expect("envelope");
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate_fields = record.candidate().fields().expect("candidate");
    let RunJournalRecordFields::ExternalAccessObserved(observation) =
        candidate_fields.payload.fields().expect("record")
    else {
        panic!("observation record");
    };
    let observation_fields = observation.fields().expect("observation");
    let mfm_journal::ObservationOutcomeFields::DidNotEnter { safe_failure } =
        observation_fields.outcome.fields().expect("outcome")
    else {
        panic!("did-not-enter observation");
    };
    let failure = safe_failure.fields().expect("safe failure");
    assert!(failure.diagnostic_ref.is_none());
    let producer = ProducerBinding::external_observation(
        &observation_fields.authorization_ref,
        &field_path("outcome.safe_failure.diagnostic_ref"),
    )
    .expect("diagnostic producer");
    let diagnostic_ref =
        derive_value_ref(diagnostic_contract, &producer, bytes).expect("diagnostic ref");
    let diagnostic_fields = diagnostic_ref.fields().expect("diagnostic fields");
    let replacement_failure = SafeFailure::new(
        &failure.safe_failure_contract_ref,
        &failure.stable_code,
        failure.failure_class,
        failure.boundary_stage,
        failure.coarse_size_class,
        Some(&diagnostic_ref),
    )
    .expect("replacement safe failure");
    let replacement_outcome =
        ObservationOutcome::did_not_enter(&replacement_failure).expect("replacement outcome");
    let replacement_observation = ExternalAccessObserved::new(
        &observation_fields.authorization_ref,
        &replacement_outcome,
        None,
    )
    .expect("replacement observation");
    let replacement_payload = RunJournalRecord::external_access_observed(&replacement_observation)
        .expect("replacement record");
    let replacement_candidate = CandidateRecordEnvelope::new(
        candidate_fields.ordinal,
        &candidate_fields.schema_id,
        &candidate_fields.logical_key,
        &replacement_payload,
        candidate_fields.emits_facts,
    )
    .expect("replacement candidate");
    let mut bindings = envelope.core.ordered_object_bindings.clone();
    bindings.push(
        ObjectPathBinding::new(
            candidate_fields.ordinal,
            &field_path("outcome.safe_failure.diagnostic_ref"),
            AuthorityUse::ProducedHere,
            &diagnostic_ref,
            &diagnostic_fields.evidence_contract_ref,
        )
        .expect("diagnostic binding"),
    );
    bindings.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let mut intents = envelope.core.artifact_admission_intents.clone();
    intents.push(
        ArtifactAdmissionIntent::new(
            &diagnostic_ref,
            &diagnostic_fields.evidence_contract_ref,
            ArtifactAdmissionMode::AdmitOrVerifyExact,
        )
        .expect("diagnostic intent"),
    );
    intents.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    let candidate_digest = CommitCandidatePreimage::new(
        &envelope.core.predecessor,
        BatchPurpose::ExternalAccessObservation,
        std::slice::from_ref(&replacement_candidate),
        &bindings,
        &intents,
    )
    .expect("candidate preimage")
    .candidate_digest()
    .expect("candidate digest");
    let record_hash = RecordHashPreimage::from_candidate(&replacement_candidate)
        .expect("record hash preimage")
        .record_hash()
        .expect("record hash");
    let record_id = RecordIdPreimage::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        candidate_fields.ordinal,
        &record_hash,
    )
    .expect("record id preimage")
    .record_id()
    .expect("record id");
    let replacement_record = CommittedJournalRecord::from_persisted(
        record_id,
        record_hash.clone(),
        replacement_candidate,
    );
    let commit_digest = CommitDigestPreimage::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &bindings,
        &intents,
    )
    .expect("commit preimage")
    .commit_digest()
    .expect("commit digest");
    let replacement_envelope = CommitEnvelope::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &commit_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &bindings,
        &intents,
        envelope.committed_at,
    )
    .expect("replacement envelope");
    *commits.last_mut().expect("observation commit") =
        CommittedJournalCommit::from_persisted(replacement_envelope, vec![replacement_record]);
    objects.push(
        CommittedObject::from_persisted(diagnostic_ref, bytes.to_vec()).expect("diagnostic object"),
    );
}

#[derive(Clone, Copy)]
enum FactObservationMutation {
    ResponseFrontier,
    AttestationRequestDigest,
}

fn replace_fact_observation(
    fixture: &CommittedFactSelection,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    mutation: FactObservationMutation,
) {
    let last = commits.last().expect("observation commit").clone();
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().fields().expect("candidate");
    let RunJournalRecordFields::ExternalAccessObserved(observation) =
        candidate.payload.fields().expect("record")
    else {
        panic!("observation record");
    };
    let observation_fields = observation.fields().expect("observation");
    let mfm_journal::ObservationOutcomeFields::Returned {
        result_ref: old_response_ref,
    } = observation_fields.outcome.fields().expect("outcome")
    else {
        panic!("returned observation");
    };
    let old_attestation_ref = observation_fields
        .fact_selection_scan_attestation_ref
        .expect("scan attestation");
    let response = FactSelectionResponse::strict_decode(
        objects
            .iter()
            .find(|object| object.value_ref() == &old_response_ref)
            .expect("response object")
            .bytes(),
    )
    .expect("response bytes");
    let response_fields = response.fields().expect("response fields");
    let attestation = FactSelectionScanAttestation::strict_decode(
        objects
            .iter()
            .find(|object| object.value_ref() == &old_attestation_ref)
            .expect("attestation object")
            .bytes(),
    )
    .expect("attestation bytes");
    let attestation_fields = attestation.fields().expect("attestation fields");
    let response_producer = old_response_ref
        .fields()
        .expect("response ref")
        .producer_binding;
    let attestation_producer = old_attestation_ref
        .fields()
        .expect("attestation ref")
        .producer_binding;

    let mut replacements = Vec::new();
    let (response_ref, frontier) = match mutation {
        FactObservationMutation::ResponseFrontier => {
            let frontier_fields = response_fields.frontier.fields().expect("frontier fields");
            let frontier = TenantFactFrontier::new(
                &frontier_fields.store_scope_id,
                frontier_fields.store_epoch,
                &frontier_fields.tenant_scope_id,
                frontier_fields.fact_order + 1,
            )
            .expect("replacement frontier");
            let replacement = FactSelectionResponse::new(
                &response_fields.request_digest,
                &frontier,
                &response_fields.results,
            )
            .expect("replacement response");
            let response_ref = derive_value_ref(
                &fixture.response_contract,
                &response_producer,
                replacement.as_bytes(),
            )
            .expect("replacement response ref");
            replacements.push((
                old_response_ref,
                response_ref.clone(),
                replacement.as_bytes().to_vec(),
                1,
            ));
            (response_ref, frontier)
        }
        FactObservationMutation::AttestationRequestDigest => {
            (old_response_ref.clone(), response_fields.frontier)
        }
    };
    let request_digest = match mutation {
        FactObservationMutation::ResponseFrontier => attestation_fields.request_digest,
        FactObservationMutation::AttestationRequestDigest => {
            FactQueryDigest::from_digest(sha256_digest_bytes(b"wrong fact-selection request"))
        }
    };
    let replacement_attestation = FactSelectionScanAttestation::new(
        &attestation_fields.store_scope_id,
        &attestation_fields.store_epoch,
        &attestation_fields.tenant_scope_id,
        &attestation_fields.authorization_ref,
        &request_digest,
        &frontier,
        &response_ref,
        &attestation_fields.response_closure_digest,
    )
    .expect("replacement attestation");
    let replacement_attestation_ref = derive_value_ref(
        &fixture.attestation_contract,
        &attestation_producer,
        replacement_attestation.as_bytes(),
    )
    .expect("replacement attestation ref");
    replacements.push((
        old_attestation_ref,
        replacement_attestation_ref.clone(),
        replacement_attestation.as_bytes().to_vec(),
        1,
    ));
    let outcome = ObservationOutcome::returned(&response_ref).expect("replacement outcome");
    let replacement_observation = ExternalAccessObserved::new(
        &observation_fields.authorization_ref,
        &outcome,
        Some(&replacement_attestation_ref),
    )
    .expect("replacement observation");
    let payload = RunJournalRecord::external_access_observed(&replacement_observation)
        .expect("replacement record");
    replace_observation_objects(
        &fixture.store_identity,
        commits,
        objects,
        replacements,
        payload,
    );
}

fn rewrite_last_observation_intents(
    store_identity: &StoreIdentity,
    commits: &mut [CommittedJournalCommit],
    intents: Vec<ArtifactAdmissionIntent>,
) {
    let last = commits.last().expect("observation commit").clone();
    let envelope = last.envelope().fields().expect("envelope");
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().clone();
    let candidate_digest = CommitCandidatePreimage::new(
        &envelope.core.predecessor,
        BatchPurpose::ExternalAccessObservation,
        std::slice::from_ref(&candidate),
        &envelope.core.ordered_object_bindings,
        &intents,
    )
    .expect("candidate preimage")
    .candidate_digest()
    .expect("candidate digest");
    let record_hash = record.record_hash().clone();
    let commit_digest = CommitDigestPreimage::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &envelope.core.ordered_object_bindings,
        &intents,
    )
    .expect("commit preimage")
    .commit_digest()
    .expect("commit digest");
    let rewritten_envelope = CommitEnvelope::new(
        store_identity.store_scope_id(),
        &envelope.core.run_id,
        envelope.core.run_sequence,
        &envelope.core.predecessor,
        &envelope.core.append_request_id,
        &candidate_digest,
        &commit_digest,
        &envelope.core.tenant_fact_coordinate,
        std::slice::from_ref(&record_hash),
        &envelope.core.ordered_object_bindings,
        &intents,
        envelope.committed_at,
    )
    .expect("rewritten envelope");
    *commits.last_mut().expect("observation commit") =
        CommittedJournalCommit::from_persisted(rewritten_envelope, vec![record.clone()]);
}

fn rewrite_fact_require_existing_intent(
    fixture: &CommittedFactSelection,
    commits: &mut [CommittedJournalCommit],
    replacement: &mfm_journal::ValueRef,
) {
    let last = commits.last().expect("observation commit").clone();
    let envelope = last.envelope().fields().expect("envelope");
    let required = envelope
        .core
        .artifact_admission_intents
        .iter()
        .filter(|intent| {
            intent.fields().expect("intent").mode == ArtifactAdmissionMode::RequireExisting
        })
        .collect::<Vec<_>>();
    assert_eq!(required.len(), 1, "empty scan has one evidence dependency");
    let old_ref = required[0].fields().expect("intent").value_ref;
    assert_ne!(&old_ref, replacement, "replacement fact import");
    let replacement_fields = replacement.fields().expect("replacement ref");
    let mut intents = envelope
        .core
        .artifact_admission_intents
        .iter()
        .filter(|intent| intent.fields().expect("intent").value_ref != old_ref)
        .cloned()
        .collect::<Vec<_>>();
    intents.push(
        ArtifactAdmissionIntent::new(
            replacement,
            &replacement_fields.evidence_contract_ref,
            ArtifactAdmissionMode::RequireExisting,
        )
        .expect("replacement intent"),
    );
    intents.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    rewrite_last_observation_intents(&fixture.store_identity, commits, intents);
}

fn substitute_fact_require_existing_intent_with_seen_object(
    fixture: &CommittedFactSelection,
    commits: &mut [CommittedJournalCommit],
    objects: &[CommittedObject],
) {
    let last = commits.last().expect("observation commit");
    let envelope = last.envelope().fields().expect("envelope");
    let replacement = objects
        .iter()
        .find(|object| {
            !envelope
                .core
                .artifact_admission_intents
                .iter()
                .any(|intent| intent.fields().expect("intent").value_ref == *object.value_ref())
                && matches!(
                    object
                        .value_ref()
                        .fields()
                        .and_then(|fields| fields.producer_binding.fields()),
                    Ok(ProducerBindingFields::ConfiguredValue { .. })
                )
        })
        .expect("visible configured replacement")
        .value_ref()
        .clone();
    rewrite_fact_require_existing_intent(fixture, commits, &replacement);
}

fn substitute_fact_require_existing_intent_with_first_use_unbound_object(
    fixture: &CommittedFactSelection,
    commits: &mut [CommittedJournalCommit],
    objects: &mut Vec<CommittedObject>,
    bytes: &[u8],
) {
    let producer = ProducerBinding::qualified_support(
        fixture.fixture.qualification_scope_id(),
        &field_path("fact_scan.unbound_substitution"),
    )
    .expect("first-use substitution producer");
    let replacement =
        derive_value_ref(&fixture.response_contract, &producer, bytes).expect("replacement ref");
    assert!(
        objects
            .iter()
            .all(|object| object.value_ref() != &replacement),
        "replacement must be first-use"
    );
    assert!(
        commits.iter().all(|commit| {
            let envelope = commit.envelope().fields().expect("envelope");
            envelope
                .core
                .artifact_admission_intents
                .iter()
                .all(|intent| intent.fields().expect("intent").value_ref != replacement)
                && envelope
                    .core
                    .ordered_object_bindings
                    .iter()
                    .all(|binding| binding.fields().expect("binding").value_ref != replacement)
        }),
        "replacement authority must be absent from prior physical history"
    );
    rewrite_fact_require_existing_intent(fixture, commits, &replacement);
    let last = commits.last().expect("observation commit");
    let envelope = last.envelope().fields().expect("envelope");
    assert!(
        envelope
            .core
            .ordered_object_bindings
            .iter()
            .all(|binding| binding.fields().expect("binding").value_ref != replacement),
        "replacement RequireExisting authority must remain unbound"
    );
    objects.push(
        CommittedObject::from_persisted(replacement, bytes.to_vec())
            .expect("first-use replacement object"),
    );
}

#[tokio::test]
async fn candidate_and_replay_reject_recomputed_malformed_read_diagnostics() {
    let fixture = authorize_read(true, 40).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized run")
        .verify_recorded_history()
        .expect("verify authorized run");
    let hostile = [
        (
            "wrong-u16",
            r#"{"detail":{"code":-1,"reason":{"kind":"malformed"},"status":-1},"kind":"response_invalid"}"#,
        ),
        (
            "wrong-i64",
            r#"{"detail":{"code":9223372036854775808,"reason":{"kind":"malformed"},"status":500},"kind":"response_invalid"}"#,
        ),
        (
            "extra-field",
            r#"{"detail":{"code":-1,"extra":true,"reason":{"kind":"malformed"},"status":500},"kind":"response_invalid"}"#,
        ),
        (
            "missing-field",
            r#"{"detail":{"code":-1,"reason":{"kind":"malformed"}},"kind":"response_invalid"}"#,
        ),
        (
            "wrong-tag",
            r#"{"detail":{"code":-1,"reason":{"kind":"malformed"},"status":500},"kind":"unknown"}"#,
        ),
        (
            "wrong-nested-kind",
            r#"{"detail":{"code":-1,"reason":{"kind":"unknown"},"status":500},"kind":"response_invalid"}"#,
        ),
    ];
    for (index, (name, bytes)) in hostile.iter().enumerate() {
        let error = match fixture.store.prepare_append(
            &drive,
            &view,
            AppendRequestId::new(format!("hostile-candidate-{index}")).expect("append request"),
            observation_material(&fixture, Some(canonical(bytes))),
        ) {
            Err(error) => error,
            Ok(_) => panic!("{name} candidate unexpectedly prepared"),
        };
        assert_eq!(
            error,
            StoreError::PersistedMismatch {
                field: "read_safe_failure"
            },
            "{name}"
        );
    }

    let valid = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new("valid-diagnostic-observation").expect("append request"),
            observation_material(&fixture, Some(canonical(VALID_DIAGNOSTIC))),
        )
        .expect("prepare valid observation");
    assert!(matches!(
        fixture
            .store
            .append(&drive, valid)
            .await
            .expect("append valid observation"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    let (tenant, commits, objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded run");
    for (name, bytes) in hostile {
        let mut hostile_commits = commits.clone();
        let mut hostile_objects = objects.clone();
        replace_observation_diagnostic(
            &fixture.store_identity,
            &fixture.diagnostic_contract,
            &mut hostile_commits,
            &mut hostile_objects,
            canonical(bytes).as_bytes(),
        );
        let error = match verify_offline_recorded_history(
            fixture.store_identity.clone(),
            tenant.clone(),
            fixture.run_id.clone(),
            hostile_commits,
            hostile_objects,
        ) {
            Err(error) => error,
            Ok(_) => panic!("{name} replay unexpectedly verified"),
        };
        assert_eq!(
            error,
            StoreError::PersistedMismatch {
                field: "read_safe_failure"
            },
            "{name}"
        );
    }
}

#[tokio::test]
async fn candidate_and_replay_enforce_returned_contract_producer_path_and_full_value_ref() {
    let fixture = authorize_read(false, 44).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized run")
        .verify_recorded_history()
        .expect("verify authorized run");
    let wrong_contract = retained_contract(
        fixture.returned_contract.schema_id().clone(),
        "read-returned",
        "wrong-read-returned-role",
    );
    let candidate_error = match fixture.store.prepare_append(
        &drive,
        &view,
        AppendRequestId::new("wrong-returned-contract").expect("append request"),
        ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::Read {
            authorization_ref: fixture.authorization_ref.clone(),
            outcome: Box::new(ReadObservationMaterial::Returned {
                returned_root: ProducedObjectRoot::new(
                    wrong_contract.clone(),
                    canonical(r#"{"value":"returned"}"#),
                ),
            }),
        })),
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong-contract returned candidate unexpectedly prepared"),
    };
    assert!(matches!(
        candidate_error,
        StoreError::InvalidPreparedAppend {
            purpose: "read_observation",
            ..
        }
    ));

    let append = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new("valid-returned-observation").expect("append request"),
            ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::Read {
                authorization_ref: fixture.authorization_ref.clone(),
                outcome: Box::new(ReadObservationMaterial::Returned {
                    returned_root: ProducedObjectRoot::new(
                        fixture.returned_contract.clone(),
                        canonical(r#"{"value":"returned"}"#),
                    ),
                }),
            })),
        )
        .expect("prepare returned observation");
    assert!(matches!(
        fixture
            .store
            .append(&drive, append)
            .await
            .expect("append returned observation"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    let (tenant, commits, objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded returned run");

    let mut wrong_path_commits = commits.clone();
    let mut wrong_path_objects = objects.clone();
    replace_observation_result(
        &fixture.store_identity,
        &fixture.returned_contract,
        ProducerBinding::external_observation(
            &fixture.authorization_ref,
            &field_path("outcome.wrong_result_ref"),
        )
        .expect("wrong-path producer"),
        &mut wrong_path_commits,
        &mut wrong_path_objects,
    );
    assert_eq!(
        match verify_offline_recorded_history(
            fixture.store_identity.clone(),
            tenant.clone(),
            fixture.run_id.clone(),
            wrong_path_commits,
            wrong_path_objects,
        ) {
            Err(error) => error,
            Ok(_) => panic!("wrong-path replay unexpectedly verified"),
        },
        StoreError::PersistedMismatch {
            field: "observation_producer"
        }
    );

    let mut wrong_contract_commits = commits.clone();
    let mut wrong_contract_objects = objects.clone();
    replace_observation_result(
        &fixture.store_identity,
        &wrong_contract,
        ProducerBinding::external_observation(
            &fixture.authorization_ref,
            &field_path("outcome.result_ref"),
        )
        .expect("result producer"),
        &mut wrong_contract_commits,
        &mut wrong_contract_objects,
    );
    assert!(verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        wrong_contract_commits,
        wrong_contract_objects,
    )
    .is_err());

    let mut preexisting_commits = commits.clone();
    let mut preexisting_objects = objects.clone();
    add_unbound_preexisting_observation_object(
        &fixture.store_identity,
        &fixture.authorization_ref,
        &fixture.returned_contract,
        &mut preexisting_commits,
        &mut preexisting_objects,
        canonical(r#"{"preexisting":true}"#).as_bytes(),
    );
    assert!(matches!(
        verify_offline_recorded_history(
            fixture.store_identity.clone(),
            tenant.clone(),
            fixture.run_id.clone(),
            preexisting_commits,
            preexisting_objects,
        ),
        Err(StoreError::MissingObjectAuthority { .. })
    ));

    let mut extra_commits = commits;
    let mut extra_objects = objects;
    add_unbound_observation_object(
        &fixture.store_identity,
        &fixture.authorization_ref,
        &fixture.returned_contract,
        &mut extra_commits,
        &mut extra_objects,
        canonical(r#"{"extra":true}"#).as_bytes(),
    );
    assert_eq!(
        match verify_offline_recorded_history(
            fixture.store_identity,
            tenant,
            fixture.run_id,
            extra_commits,
            extra_objects,
        ) {
            Err(error) => error,
            Ok(_) => panic!("extra-object replay unexpectedly verified"),
        },
        StoreError::PersistedMismatch {
            field: "observation_produced_values"
        }
    );
}

#[tokio::test]
async fn classifier_without_a_diagnostic_identity_accepts_a_forbidden_diagnostic_rule() {
    let fixture = authorize_read(false, 41).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized run")
        .verify_recorded_history()
        .expect("verify authorized run");
    let append = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new("no-diagnostic-observation").expect("append request"),
            observation_material(&fixture, None),
        )
        .expect("prepare no-diagnostic observation");
    assert!(matches!(
        fixture
            .store
            .append(&drive, append)
            .await
            .expect("append no-diagnostic observation"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload no-diagnostic run")
        .verify_recorded_history()
        .expect("verify no-diagnostic classifier branch");
}

async fn assert_diagnostic_contract_bridge_mismatch(
    contracts: ReadFixtureContracts,
    discriminator: u8,
) {
    let fixture = authorize_read_with(contracts, discriminator).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized run")
        .verify_recorded_history()
        .expect("verify authorized run");
    let candidate_error = match fixture.store.prepare_append(
        &drive,
        &view,
        AppendRequestId::new(format!("bridge-mismatch-{discriminator}")).expect("append request"),
        observation_material(&fixture, Some(canonical(VALID_DIAGNOSTIC))),
    ) {
        Err(error) => error,
        Ok(_) => panic!("bridge-mismatched candidate unexpectedly prepared"),
    };
    assert_eq!(
        candidate_error,
        StoreError::PersistedMismatch {
            field: "read_classifier"
        }
    );

    let (tenant, mut commits, mut objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded authorization");
    append_raw_diagnostic_observation(
        &fixture.store_identity,
        &fixture.authorization_ref,
        &fixture.diagnostic_contract,
        &fixture.metadata,
        &mut commits,
        &mut objects,
        canonical(VALID_DIAGNOSTIC).as_bytes(),
    );
    let replay_error = match verify_offline_recorded_history(
        fixture.store_identity,
        tenant,
        fixture.run_id,
        commits,
        objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("bridge-mismatched replay unexpectedly verified"),
    };
    assert_eq!(
        replay_error,
        StoreError::PersistedMismatch {
            field: "read_classifier"
        }
    );
}

#[tokio::test]
async fn candidate_and_replay_require_the_complete_diagnostic_identity_contract_bridge() {
    let mut semantic_mismatch = read_fixture_contracts(true);
    let semantic_contract = retained_contract(
        semantic_mismatch.diagnostic_contract.schema_id().clone(),
        "wrong-read-diagnostic",
        "read-diagnostic",
    );
    semantic_mismatch.execution.safe_failure_contract = semantic_contract.clone();
    semantic_mismatch.diagnostic_contract = semantic_contract;
    assert_diagnostic_contract_bridge_mismatch(semantic_mismatch, 42).await;

    let mut media_mismatch = read_fixture_contracts(true);
    let media_contract = retained_contract_with_media(
        media_mismatch.diagnostic_contract.schema_id().clone(),
        "read-diagnostic",
        "read-diagnostic",
        "application/problem+json",
    );
    media_mismatch.execution.safe_failure_contract = media_contract.clone();
    media_mismatch.diagnostic_contract = media_contract;
    assert_diagnostic_contract_bridge_mismatch(media_mismatch, 43).await;
}

#[tokio::test]
async fn effect_returned_candidate_commits_and_replays_with_the_exact_retained_closure() {
    let fixture = authorize_effect(44).await;
    let verified = verified_pending_effect(&fixture).await;
    append_effect_result(&fixture, "effect-returned-observation", verified).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload returned effect")
        .verify_recorded_history()
        .expect("replay returned effect");
}

#[tokio::test]
async fn effect_pending_seals_candidates_and_rejects_recomputed_wrong_nested_full_refs() {
    let fixture = authorize_effect(46).await;
    let ledger = KeyedExecutorLedger::new(
        MemoryExecutorStore::new(&fixture.binding),
        fixture.binding.clone(),
    )
    .expect("executor ledger");
    let effect = ledger
        .bind_effect(&fixture.identity)
        .await
        .expect("bind effect");
    let audit = effect.delivery_audit().clone();
    let head = audit.head_ref().expect("delivery audit head");
    let closure = ExecutorRetainedClosureClaim::from_delivery_audit(&audit, &fixture.binding)
        .expect("retained closure");
    let head_value = SchemaQualifiedCanonicalValue::from_validated(
        &audit
            .frontiers()
            .last()
            .expect("delivery frontier")
            .validated()
            .expect("delivery frontier bytes"),
    )
    .expect("delivery frontier value");
    let wrong_relation = ExecutorRetainedValue::new(
        ExecutorRetainedValueRelation::ExecutorFrontier,
        fixture
            .binding
            .contract()
            .retained_closure_contract()
            .executor_frontier_contract()
            .clone(),
        head_value,
    )
    .expect("wrong retained relation");
    let hostile_claim = ExecutorRetainedClosureClaim::new(
        closure
            .into_members()
            .chain(std::iter::once(wrong_relation)),
    )
    .expect("hostile closure claim");
    assert_eq!(
        verify_ensure_result(
            fixture.identity.clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::pending(head),
            hostile_claim,
        )
        .expect_err("sealed verifier must reject a hostile candidate"),
        ExecutorError::FrontierFork
    );

    let verified = verified_pending_effect(&fixture).await;
    append_effect_result(&fixture, "effect-pending-hostile-base", verified).await;
    let (tenant, commits, objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded pending effect");
    let contracts = fixture.binding.contract().retained_closure_contract();

    let mut wrong_path_commits = commits.clone();
    let mut wrong_path_objects = objects.clone();
    replace_effect_pending_delivery_ref(
        &fixture.store_identity,
        &fixture.binding,
        &mut wrong_path_commits,
        &mut wrong_path_objects,
        contracts.delivery_audit_contract(),
        "executor.frontier",
    );
    let wrong_path_error = match verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        wrong_path_commits,
        wrong_path_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong retained path unexpectedly replayed"),
    };
    assert_eq!(
        wrong_path_error,
        StoreError::PersistedMismatch {
            field: "effect_retained_relation"
        }
    );

    let mut wrong_contract_commits = commits;
    let mut wrong_contract_objects = objects;
    replace_effect_pending_delivery_ref(
        &fixture.store_identity,
        &fixture.binding,
        &mut wrong_contract_commits,
        &mut wrong_contract_objects,
        contracts.executor_frontier_contract(),
        "executor.delivery_audit",
    );
    let wrong_contract_error = match verify_offline_recorded_history(
        fixture.store_identity,
        tenant,
        fixture.run_id,
        wrong_contract_commits,
        wrong_contract_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong retained contract unexpectedly replayed"),
    };
    assert_eq!(
        wrong_contract_error,
        StoreError::InvalidObjectAuthority {
            message: "retained value disagrees with its certified contract"
        }
    );
}

#[tokio::test]
async fn effect_terminal_candidate_commits_and_replays_every_retained_relation() {
    let fixture = authorize_effect(45).await;
    let verified = verified_terminal_effect(&fixture).await;
    assert!(matches!(verified.outcome(), Ensure::Terminal { .. }));
    let relations = verified
        .retained_closure()
        .members()
        .map(ExecutorRetainedValue::relation)
        .collect::<Vec<_>>();
    for relation in [
        ExecutorRetainedValueRelation::DeliveryAudit,
        ExecutorRetainedValueRelation::ExecutorFrontier,
        ExecutorRetainedValueRelation::TerminalTombstone,
        ExecutorRetainedValueRelation::TerminalProof,
        ExecutorRetainedValueRelation::DomainEvidence,
    ] {
        assert!(
            relations.contains(&relation),
            "terminal fixture lacks {relation:?}"
        );
    }
    append_effect_result(&fixture, "effect-terminal-observation", verified).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload terminal effect")
        .verify_recorded_history()
        .expect("replay terminal effect");
}

#[tokio::test]
async fn effect_terminal_replay_rejects_a_recomputed_outer_evidence_path() {
    let fixture = authorize_effect(47).await;
    let verified = verified_terminal_effect(&fixture).await;
    append_effect_result(&fixture, "effect-terminal-hostile-base", verified).await;
    let (tenant, mut commits, mut objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded terminal effect");
    replace_effect_terminal_chain(
        &fixture.store_identity,
        &fixture.binding,
        &mut commits,
        &mut objects,
        TerminalEffectMutation::OuterEvidenceWrongPath,
    );
    let error = match verify_offline_recorded_history(
        fixture.store_identity,
        tenant,
        fixture.run_id,
        commits,
        objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong outer evidence path unexpectedly replayed"),
    };
    assert_eq!(
        error,
        StoreError::PersistedMismatch {
            field: "observation_producer"
        }
    );
}

#[tokio::test]
async fn effect_terminal_seals_and_replays_exact_identity_and_complete_closure() {
    let fixture = authorize_effect(48).await;
    let verified = verified_terminal_effect(&fixture).await;
    let terminal_claim = match verified.outcome() {
        Ensure::Terminal { evidence } => evidence.claim().clone(),
        Ensure::Pending { .. } => panic!("terminal result"),
    };
    let members = verified
        .retained_closure()
        .members()
        .cloned()
        .collect::<Vec<_>>();
    let incomplete = members
        .iter()
        .filter(|member| member.relation() != ExecutorRetainedValueRelation::TerminalProof)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        verify_ensure_result(
            fixture.identity.clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(terminal_claim.clone()),
            ExecutorRetainedClosureClaim::new(incomplete).expect("incomplete closure claim"),
        )
        .expect_err("sealed verifier must reject an incomplete candidate"),
        ExecutorError::RetainedClosureIncomplete
    );

    let extra_bytes = canonical(
        r#"{"destination_key":"mfm.store-test/extra","kind":"enqueued","queue_position":"2"}"#,
    );
    let extra_validated = RecoverabilityContract::embedded()
        .expect("recoverability contract")
        .strict_decode(
            "mfm.executor-reference-queue-result.v2",
            extra_bytes.as_bytes(),
        )
        .expect("extra domain evidence");
    let extra_member = ExecutorRetainedValue::new(
        ExecutorRetainedValueRelation::DomainEvidence,
        fixture
            .binding
            .contract()
            .retained_closure_contract()
            .domain_evidence_contract()
            .clone(),
        SchemaQualifiedCanonicalValue::from_validated(&extra_validated)
            .expect("extra domain value"),
    )
    .expect("extra domain member");
    assert_eq!(
        verify_ensure_result(
            fixture.identity.clone(),
            &fixture.binding,
            ExecutorEnsureResultClaim::terminal(terminal_claim),
            ExecutorRetainedClosureClaim::new(
                members.iter().cloned().chain(std::iter::once(extra_member)),
            )
            .expect("extra closure claim"),
        )
        .expect_err("sealed verifier must reject an extra candidate member"),
        ExecutorError::RetainedClosureExtra
    );

    append_effect_result(&fixture, "effect-terminal-exactness-base", verified).await;
    let (tenant, commits, objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded terminal effect");

    let mut tombstone_commits = commits.clone();
    let mut tombstone_objects = objects.clone();
    replace_effect_terminal_chain(
        &fixture.store_identity,
        &fixture.binding,
        &mut tombstone_commits,
        &mut tombstone_objects,
        TerminalEffectMutation::TombstoneWrongPath,
    );
    let tombstone_error = match verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        tombstone_commits,
        tombstone_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong tombstone path unexpectedly replayed"),
    };
    assert_eq!(
        tombstone_error,
        StoreError::PersistedMismatch {
            field: "effect_retained_relation"
        }
    );

    let mut identity_commits = commits.clone();
    let mut identity_objects = objects.clone();
    replace_effect_terminal_chain(
        &fixture.store_identity,
        &fixture.binding,
        &mut identity_commits,
        &mut identity_objects,
        TerminalEffectMutation::WrongRequestDigest,
    );
    let identity_error = match verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        identity_commits,
        identity_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong terminal identity unexpectedly replayed"),
    };
    assert_eq!(
        identity_error,
        StoreError::PersistedMismatch {
            field: "effect_terminal_identity"
        }
    );

    let mut missing_commits = commits.clone();
    let mut missing_objects = objects.clone();
    remove_unbound_observation_object(
        &fixture.store_identity,
        &mut missing_commits,
        &mut missing_objects,
        "executor.terminal_proof.",
    );
    let missing_error = match verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        missing_commits,
        missing_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("missing terminal proof unexpectedly replayed"),
    };
    assert_eq!(
        missing_error,
        StoreError::PersistedMismatch {
            field: "effect_retained_closure"
        }
    );

    let mut extra_commits = commits;
    let mut extra_objects = objects;
    let domain_contract = fixture
        .binding
        .contract()
        .retained_closure_contract()
        .domain_evidence_contract();
    let extra_ref = content_ref(domain_contract, extra_bytes.as_bytes());
    add_unbound_observation_object_at_path(
        &fixture.store_identity,
        &fixture.authorization_ref,
        domain_contract,
        &mut extra_commits,
        &mut extra_objects,
        extra_bytes.as_bytes(),
        UnboundObservationIntent {
            producer_path: &format!(
                "executor.domain_evidence.{}",
                extra_ref.content_digest().digest()
            ),
            mode: ArtifactAdmissionMode::AdmitOrVerifyExact,
        },
    );
    let extra_error = match verify_offline_recorded_history(
        fixture.store_identity,
        tenant,
        fixture.run_id,
        extra_commits,
        extra_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("extra domain evidence unexpectedly replayed"),
    };
    assert_eq!(
        extra_error,
        StoreError::PersistedMismatch {
            field: "effect_retained_closure"
        }
    );
}

#[tokio::test]
async fn effect_safe_failure_requires_the_exact_contract_and_zero_produced_values() {
    let fixture = authorize_effect(49).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("load authorized effect")
        .verify_recorded_history()
        .expect("verify authorized effect");
    let wrong_contract = fixture
        .binding
        .contract()
        .reference()
        .expect("wrong safe-failure contract");
    let wrong_failure = reference_safe_failure(
        wrong_contract,
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("wrong-contract safe failure");
    let wrong_outcome =
        EffectExecutorOutcome::did_not_enter(wrong_failure).expect("wrong-contract outcome");
    let candidate_error = match fixture.store.prepare_append(
        &drive,
        &view,
        AppendRequestId::new("effect-safe-failure-wrong-contract").expect("append request"),
        ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::EnsureEffect {
            authorization_ref: fixture.authorization_ref.clone(),
            outcome: Box::new(wrong_outcome),
        })),
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong safe-failure contract unexpectedly prepared"),
    };
    assert_eq!(
        candidate_error,
        StoreError::PersistedMismatch {
            field: "effect_safe_failure"
        }
    );

    let failure = reference_safe_failure(
        fixture.safe_failure_contract_ref.clone(),
        ReferenceFailureCode::DestinationUnavailable,
        FailureClass::Transport,
        BoundaryStage::BeforeBoundaryEntry,
    )
    .expect("safe failure");
    let outcome = EffectExecutorOutcome::did_not_enter(failure).expect("safe-failure outcome");
    let append = fixture
        .store
        .prepare_append(
            &drive,
            &view,
            AppendRequestId::new("effect-safe-failure-valid").expect("append request"),
            ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::EnsureEffect {
                authorization_ref: fixture.authorization_ref.clone(),
                outcome: Box::new(outcome),
            })),
        )
        .expect("prepare safe-failure observation");
    assert!(matches!(
        fixture
            .store
            .append(&drive, append)
            .await
            .expect("append safe-failure observation"),
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
    ));
    fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload safe-failure effect")
        .verify_recorded_history()
        .expect("replay safe-failure effect");

    let (tenant, mut commits, mut objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded safe-failure effect");
    let diagnostic_contract = retained_contract(
        RecoverabilityContract::embedded()
            .expect("recoverability contract")
            .schema_id("mfm.primitive-canonical_value.v1")
            .expect("diagnostic schema")
            .clone(),
        "effect-injected-diagnostic",
        "effect-injected-diagnostic",
    );
    inject_effect_safe_failure_diagnostic(
        &fixture.store_identity,
        &diagnostic_contract,
        &mut commits,
        &mut objects,
        canonical("{}").as_bytes(),
    );
    let replay_error = match verify_offline_recorded_history(
        fixture.store_identity,
        tenant,
        fixture.run_id,
        commits,
        objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("diagnostic-bearing effect failure unexpectedly replayed"),
    };
    assert_eq!(
        replay_error,
        StoreError::PersistedMismatch {
            field: "effect_safe_failure"
        }
    );
}

#[tokio::test]
async fn empty_frontier_fact_selection_candidate_commits_and_replays() {
    let fixture = commit_fact_selection(50).await;
    let drive = fixture.issuer.authorize_drive(
        fixture.fixture.tenant_scope_id().clone(),
        fixture.run_id.clone(),
    );
    let view = fixture
        .store
        .load_for_drive(&drive)
        .await
        .expect("reload fact selection")
        .verify_recorded_history()
        .expect("replay fact selection");
    assert!(view
        .authorizations()
        .any(|(reference, _)| reference == &fixture.authorization_ref));
}

#[tokio::test]
async fn empty_frontier_fact_selection_replay_enforces_frontier_attestation_and_exact_objects() {
    let fixture = commit_fact_selection(51).await;
    let (tenant, commits, objects) = fixture
        .store
        .recorded_run_for_observation_test(&fixture.run_id)
        .expect("recorded fact selection");

    let mut frontier_commits = commits.clone();
    let mut frontier_objects = objects.clone();
    replace_fact_observation(
        &fixture,
        &mut frontier_commits,
        &mut frontier_objects,
        FactObservationMutation::ResponseFrontier,
    );
    let frontier_error = match verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        frontier_commits,
        frontier_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong response frontier unexpectedly replayed"),
    };
    assert_eq!(frontier_error, StoreError::FactScanBindingMismatch);

    let mut attestation_commits = commits.clone();
    let mut attestation_objects = objects.clone();
    replace_fact_observation(
        &fixture,
        &mut attestation_commits,
        &mut attestation_objects,
        FactObservationMutation::AttestationRequestDigest,
    );
    let attestation_error = match verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        attestation_commits,
        attestation_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("wrong attestation request unexpectedly replayed"),
    };
    assert_eq!(attestation_error, StoreError::FactScanBindingMismatch);

    let last = commits.last().expect("observation commit");
    let [record] = last.records() else {
        panic!("one observation record");
    };
    let candidate = record.candidate().fields().expect("candidate");
    let RunJournalRecordFields::ExternalAccessObserved(observation) =
        candidate.payload.fields().expect("record")
    else {
        panic!("observation record");
    };
    let mfm_journal::ObservationOutcomeFields::Returned {
        result_ref: response_ref,
    } = observation
        .fields()
        .expect("observation")
        .outcome
        .fields()
        .expect("outcome")
    else {
        panic!("returned response");
    };
    let response_bytes = objects
        .iter()
        .find(|object| object.value_ref() == &response_ref)
        .expect("response object")
        .bytes()
        .to_vec();
    let mut first_use_substitution_commits = commits.clone();
    let mut first_use_substitution_objects = objects.clone();
    substitute_fact_require_existing_intent_with_first_use_unbound_object(
        &fixture,
        &mut first_use_substitution_commits,
        &mut first_use_substitution_objects,
        &response_bytes,
    );
    assert_eq!(
        match verify_offline_recorded_history(
            fixture.store_identity.clone(),
            tenant.clone(),
            fixture.run_id.clone(),
            first_use_substitution_commits,
            first_use_substitution_objects,
        ) {
            Err(error) => error,
            Ok(_) => panic!("first-use unbound fact substitution unexpectedly replayed"),
        },
        StoreError::InvalidSourceClosure
    );

    let mut imported_extra_commits = commits.clone();
    let mut imported_extra_objects = objects.clone();
    add_unbound_observation_authority(
        &fixture.store_identity,
        &fixture.response_contract,
        &mut imported_extra_commits,
        &mut imported_extra_objects,
        &response_bytes,
        ProducerBinding::qualified_support(
            fixture.fixture.qualification_scope_id(),
            &field_path("fact_scan.unbound_import"),
        )
        .expect("first-use imported producer"),
        ArtifactAdmissionMode::RequireExisting,
    );
    assert_eq!(
        match verify_offline_recorded_history(
            fixture.store_identity.clone(),
            tenant.clone(),
            fixture.run_id.clone(),
            imported_extra_commits,
            imported_extra_objects,
        ) {
            Err(error) => error,
            Ok(_) => panic!("first-use extra fact import unexpectedly replayed"),
        },
        StoreError::InvalidSourceClosure
    );

    let mut extra_commits = commits.clone();
    let mut extra_objects = objects.clone();
    add_unbound_observation_object(
        &fixture.store_identity,
        &fixture.authorization_ref,
        &fixture.response_contract,
        &mut extra_commits,
        &mut extra_objects,
        &response_bytes,
    );
    let extra_error = match verify_offline_recorded_history(
        fixture.store_identity.clone(),
        tenant.clone(),
        fixture.run_id.clone(),
        extra_commits,
        extra_objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("extra produced response unexpectedly replayed"),
    };
    assert_eq!(extra_error, StoreError::FactScanBindingMismatch);

    let mut closure_commits = commits;
    substitute_fact_require_existing_intent_with_seen_object(
        &fixture,
        &mut closure_commits,
        &objects,
    );
    let closure_error = match verify_offline_recorded_history(
        fixture.store_identity,
        tenant,
        fixture.run_id,
        closure_commits,
        objects,
    ) {
        Err(error) => error,
        Ok(_) => panic!("substituted fact closure unexpectedly replayed"),
    };
    assert_eq!(closure_error, StoreError::InvalidSourceClosure);
}
