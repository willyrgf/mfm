use super::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use ed25519_dalek::SigningKey;
use mfm_artifact_capabilities::{ArtifactReadProvider, ArtifactReadRequest, VerifiedArtifactBytes};
use mfm_canonical::{sha256_digest_bytes, CanonicalValue};
use mfm_capabilities::{
    CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor, CapabilitySpec, EffectSpec,
    ExternalMutationAuthorityRole, ManagedPlatformWrite, ReadExternalRole,
};
use mfm_ids::{
    ArtifactId, DigestBytes, EffectKind, EffectVersion, EventId, SchemaId, ScopeId, SeedId,
    SemanticTypeId, SideEffectPairId, StateKind, StateVersion, TrustScopeId,
};
use mfm_manual_auth::{
    ManualAuthorizationSignatureBytes, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, IdempotencyKey, PublicOutputKey,
    PureState, ReadState, RemediationNodeParams, ResourceClaim, RootBuilder, ScopeKey,
    SideEffectNodeParams, SideEffectSagaPolicy, SideEffectState, StateKey, StateRegistryBuilder,
    StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, PublicOutputs};
use mfm_store::v1::{
    test_support::{
        event_id_for_envelope_inputs_for_test as test_event_id_for_envelope_inputs,
        fact_query_receipt_trust_root_for_test as test_store_fact_query_receipt_trust_root,
        prepared_commit_bundle_from_plan as test_bundle_from_plan,
        prepared_commit_plan_for_test as test_prepared_commit_plan,
        signed_fact_query_receipt_for_test as test_signed_fact_query_receipt,
        SignedFactQueryReceiptFixtureInputForTest,
    },
    RunEventStore,
};
use serde::{Deserialize, Serialize};

use crate::commit::{CommitPlanner, RunnerOutputCommitInput};

const D0: DigestBytes = DigestBytes::from_array([0x10; 32]);
const D1: DigestBytes = DigestBytes::from_array([0x11; 32]);
const D2: DigestBytes = DigestBytes::from_array([0x12; 32]);
const D3: DigestBytes = DigestBytes::from_array([0x13; 32]);
const D4: DigestBytes = DigestBytes::from_array([0x14; 32]);
const D5: DigestBytes = DigestBytes::from_array([0x15; 32]);
const D6: DigestBytes = DigestBytes::from_array([0x16; 32]);
const D7: DigestBytes = DigestBytes::from_array([0x17; 32]);
const D8: DigestBytes = DigestBytes::from_array([0x18; 32]);
const D9: DigestBytes = DigestBytes::from_array([0x19; 32]);
const APPLY_SIDE_EFFECT_RUNNER: &str = "apply_side_effect";
const READ_EXTERNAL_RUNNER: &str = "read_external";

fn fixture_value_semantic_id() -> SemanticTypeId {
    SemanticTypeId::new("mfm.test", "value", "1", DigestAlgorithm::Sha256JcsV1, D9)
        .expect("semantic")
}

fn fixture_value_schema_id() -> SchemaId {
    SchemaId::new("mfm.test.value", "1", DigestAlgorithm::Sha256JcsV1, DA).expect("schema")
}

fn fixture_trust_scope_id() -> TrustScopeId {
    TrustScopeId::new("mfm.trust_scope.v1:10101010101010101010101010101010")
        .expect("test trust scope")
}

fn synthetic_side_effect_pair_id(byte: u8) -> SideEffectPairId {
    SideEffectPairId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn test_fact_descriptor() -> mfm_facts::FactDescriptor {
    <RuntimeTestFact as mfm_program::MfmFactType>::descriptor().expect("fact descriptor")
}

fn test_fact_descriptor_with_kind(kind: &str) -> mfm_facts::FactDescriptor {
    let descriptor = test_fact_descriptor();
    mfm_facts::FactDescriptor::new(
        mfm_facts::FactKind::new(kind).expect("fact kind"),
        descriptor.descriptor_schema_id().clone(),
        descriptor.subject_schema_id().clone(),
        descriptor.response_schema_id().clone(),
        descriptor.fields().to_vec(),
        descriptor.orderings().to_vec(),
    )
    .expect("fact descriptor")
}

fn test_fact_key(subject_amount: u64) -> mfm_facts::FactKey {
    test_fact_subject_evidence(subject_amount)
        .fact_key()
        .clone()
}

fn test_fact_descriptor_hash() -> ContentDigest {
    mfm_facts::fact_descriptor_hash(&test_fact_descriptor()).expect("descriptor hash")
}

fn test_fact_query_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x52; 32])
}

fn test_fact_query_trust_root() -> store::FactQueryReceiptTrustRoot {
    let key = test_fact_query_signing_key();
    test_store_fact_query_receipt_trust_root(
        &key,
        mfm_facts::StoreIdentity::new("store.default").expect("store identity"),
        mfm_facts::StoreKeyId::new("key.default").expect("store key id"),
    )
}

fn test_fact_query_evidence() -> mfm_facts::FactQueryEvidence {
    test_fact_query_evidence_with_returned_refs(Vec::new())
}

fn test_fact_query_evidence_with_returned_refs(
    returned_refs: Vec<mfm_facts::InternalFactRef>,
) -> mfm_facts::FactQueryEvidence {
    let query_scope = mfm_facts::FactQueryScope::new(
        mfm_facts::FactAudience::Platform,
        mfm_facts::FactVisibilityScope::Default,
    );
    let store_scope = mfm_facts::StoreScopeRef::new("default").expect("store scope");
    let ordering = mfm_facts::FactOrderingPolicy::new(
        mfm_facts::FactOrderingName::new("metadata.store_order.asc").expect("ordering"),
        vec![mfm_facts::FactOrderingTerm::new(
            mfm_facts::FactFieldId::new("metadata.store_order").expect("field id"),
            mfm_facts::SortDirection::Ascending,
            mfm_facts::NullOrdering::Last,
            true,
        )],
    )
    .expect("fact ordering");
    let canonical_query = mfm_canonical::CanonicalJsonBytes::from_value(
        &mfm_canonical::CanonicalValue::object([(
            "kind",
            mfm_canonical::CanonicalValue::String("chain.head".to_owned()),
        )])
        .expect("canonical query value"),
    );
    let plan = mfm_facts::CanonicalFactQueryPlan::new(
        store_scope.clone(),
        query_scope.clone(),
        mfm_facts::FactQueryCompilerVersion::new(mfm_facts::FACT_QUERY_COMPILER_VERSION)
            .expect("query compiler version"),
        mfm_facts::FactCanonicalizerVersion::new(mfm_facts::FACT_QUERY_CANONICALIZER_VERSION)
            .expect("query canonicalizer version"),
        test_fact_descriptor_hash(),
        mfm_facts::ScopeDecisionEvidence::new(content(0x42)),
        canonical_query,
        ordering,
        Some(10),
    )
    .expect("query plan");
    let frontier = mfm_facts::StoreReadFrontier::new(
        store_scope,
        query_scope,
        mfm_facts::DescriptorCatalogWatermark::new(1),
        mfm_facts::FactProjectionGeneration::new(1),
        10,
        mfm_facts::StoreCommitWatermark::new(10),
    );
    let rows = returned_refs
        .into_iter()
        .map(|fact_ref| mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new()))
        .collect::<Vec<_>>();
    let plan_hash = mfm_facts::fact_query_plan_hash(&plan).expect("plan hash");
    let key = test_fact_query_signing_key();
    let receipt = test_signed_fact_query_receipt(SignedFactQueryReceiptFixtureInputForTest {
        plan_hash: &plan_hash,
        key: &key,
        store_identity: mfm_facts::StoreIdentity::new("store.default").expect("store identity"),
        key_id: mfm_facts::StoreKeyId::new("key.default").expect("store key id"),
        read_frontier: frontier,
        rows: &rows,
        include_returned_field_summaries: false,
        limit: plan.limit(),
    });
    let selection = mfm_facts::FactSelectionEvidence::new(content(0x45), Vec::new(), None)
        .expect("selection evidence");
    mfm_facts::FactQueryEvidence::new(plan, receipt, selection)
}

#[allow(clippy::too_many_arguments)]
fn test_fact_claim(
    subject_amount: u64,
    request_schema_id: SchemaId,
    request_hash: ContentDigest,
    response_evidence: &store::ArtifactEvidenceRef,
    capability_kind: CapabilityKind,
    capability_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
) -> mfm_facts::FactClaim {
    test_fact_claim_for_descriptor(
        &test_fact_descriptor(),
        subject_amount,
        request_schema_id,
        request_hash,
        response_evidence,
        capability_kind,
        capability_version,
        adapter_kind,
        adapter_version,
    )
}

#[allow(clippy::too_many_arguments)]
fn test_fact_claim_for_descriptor(
    descriptor: &mfm_facts::FactDescriptor,
    subject_amount: u64,
    request_schema_id: SchemaId,
    request_hash: ContentDigest,
    response_evidence: &store::ArtifactEvidenceRef,
    capability_kind: CapabilityKind,
    capability_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: descriptor.fact_kind().clone(),
        fact_descriptor_hash: mfm_facts::fact_descriptor_hash(descriptor).expect("descriptor hash"),
        subject: test_fact_subject_evidence_for_descriptor(descriptor, subject_amount),
        observed_at: Some("2026-01-02T03:04:05Z".to_owned()),
        request: Some(mfm_facts::FactRequestEvidence::new(
            request_schema_id,
            request_hash,
        )),
        response: mfm_facts::FactResponseEvidence::new(
            response_evidence
                .schema_id
                .clone()
                .expect("fact response schema"),
            response_evidence.digest.clone(),
            response_evidence.artifact_id.clone(),
            response_evidence
                .evidence_hash()
                .expect("fact response evidence hash"),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            capability_kind,
            capability_version,
            adapter_kind,
            adapter_version,
        ),
    })
    .expect("fact claim")
}

fn test_fact_subject_evidence(subject_amount: u64) -> mfm_facts::FactSubjectEvidence {
    test_fact_subject_evidence_for_descriptor(&test_fact_descriptor(), subject_amount)
}

fn test_fact_subject_evidence_for_descriptor(
    descriptor: &mfm_facts::FactDescriptor,
    subject_amount: u64,
) -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactSubjectValueV1::new(
        mfm_facts::FactFieldId::new("subject.amount").expect("field"),
        mfm_facts::FactFieldValueType::UnsignedInteger,
        mfm_facts::FactCanonicalScalar::UnsignedInteger(subject_amount),
    )
    .expect("subject value")])
    .expect("subject material");
    let namespace = mfm_facts::fact_subject_namespace(descriptor).expect("fact subject namespace");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(&namespace).expect("fact subject namespace hash");
    mfm_facts::FactSubjectEvidence::from_material(namespace_hash, &material)
        .expect("subject evidence")
}

fn test_fact_response_artifact(
    node: &spec::NodeSpec,
    amount: u64,
) -> (store::ArtifactEvidenceRef, Vec<u8>) {
    let bytes =
        canonical_json(serde_json::json!({ "amount": amount })).expect("fact response json");
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(
            <CertifierValue as mfm_values::MfmValue>::schema_id().expect("fact response schema"),
        ),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    (evidence, bytes.to_vec())
}

fn test_returned_fact_authority(
    fixture: &Fixture,
    node: &spec::NodeSpec,
) -> (
    mfm_facts::InternalFactRef,
    store::FactDescriptorProjection,
    store::FactRecordProjection,
    store::FactIndexProjection,
    Vec<store::FactIndexTermProjection>,
) {
    let descriptor = test_fact_descriptor();
    let source_event_id = EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x77; 32]),
    );
    let descriptor_fixture = store::test_support::fact_descriptor_projection_fixture_for_test(
        descriptor.clone(),
        EventId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x78; 32]),
        ),
    )
    .expect("descriptor projection fixture");
    let fact_fixture = store::test_support::fact_projection_fixture_for_test(
        &descriptor,
        descriptor_fixture.descriptor_hash.clone(),
        store::test_support::FactProjectionFixtureInputForTest {
            run_id: fixture.run_id.clone(),
            source_seq: 2,
            source_ordinal: 0,
            source_event_id,
            node_id: node.node_id.clone(),
            attempt_id: AttemptId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x79; 32]),
            ),
            commit_id: store::CommitKey::new("test-returned-fact").expect("commit key"),
            store_commit_order: 2,
            recorded_at: "2026-07-01T00:00:00Z".to_owned(),
            observed_at: Some("2026-07-01T00:01:00Z".to_owned()),
            visibility: mfm_facts::FactVisibility::indexed_default(
                mfm_facts::FactAudience::Platform,
            ),
            subject: CanonicalValue::object([("amount", CanonicalValue::Unsigned(17))])
                .expect("fact subject"),
            response: CanonicalValue::object([("amount", CanonicalValue::Unsigned(23))])
                .expect("fact response"),
            request: None,
            response_schema_id: <CertifierValue as mfm_values::MfmValue>::schema_id()
                .expect("fact response schema"),
            response_artifact_id: None,
            producer: mfm_facts::FactProducerProvenance::new(
                fixture.cap_kind.clone(),
                fixture.cap_version.clone(),
                fixture.adapter_kind.clone(),
                fixture.adapter_version.clone(),
            ),
        },
    )
    .expect("fact projection fixture");
    let index_projection = fact_fixture.index.expect("indexed fact projection");
    let fact_ref = index_projection.internal_ref().expect("internal fact ref");
    (
        fact_ref,
        descriptor_fixture.projection,
        fact_fixture.record,
        index_projection,
        fact_fixture.terms,
    )
}

fn projection_snapshot_with_returned_fact_authority(
    base: &store::ProjectionSnapshot,
    descriptor: store::FactDescriptorProjection,
    record: store::FactRecordProjection,
    index: store::FactIndexProjection,
    terms: Vec<store::FactIndexTermProjection>,
) -> store::ProjectionSnapshot {
    let mut parts = store::ProjectionSnapshotParts::from_snapshot(base);
    parts
        .fact_descriptors
        .insert(descriptor.descriptor_hash.clone(), descriptor);
    parts
        .fact_records
        .insert(record.fact_claim_id.clone(), record);
    parts
        .fact_index_entries
        .insert(index.fact_claim_id.clone(), index);
    parts.fact_term_entries.extend(
        terms
            .into_iter()
            .map(|term| ((term.fact_claim_id.clone(), term.field_id.clone()), term)),
    );
    store::ProjectionSnapshot::from_parts(parts)
        .expect("projection snapshot with returned fact authority")
}
fn fixture_side_effect_pair_id(fixture: &Fixture, node: &spec::NodeSpec) -> SideEffectPairId {
    match &node.framework {
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => verify.pair_id.clone(),
        _ => fixture
            .runtime_spec
            .side_effect_pair_for_submit_node(&node.node_id)
            .cloned()
            .expect("certified side-effect pair"),
    }
}

fn runtime_spec_terminal_policies(
    runtime_spec: &CertifiedRuntimeSpec,
) -> store::SideEffectTerminalPolicies {
    store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())
        .expect("certified side-effect terminal policies")
}

fn fixture_terminal_policies(fixture: &Fixture) -> store::SideEffectTerminalPolicies {
    runtime_spec_terminal_policies(&fixture.runtime_spec)
}

macro_rules! assert_side_effect_binding {
    ($payload:expr, $ledger_key:expr, $invocation_epoch:expr) => {{
        assert_eq!($payload.ledger_key, $ledger_key);
        assert_eq!($payload.invocation_epoch, $invocation_epoch);
    }};
}

macro_rules! delegate_execution_claim_store_to_inner {
    ($ty:ty) => {
        impl store::ExecutionClaimStore for $ty {
            type Error = store::StoreError;

            fn acquire_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
                self.inner.acquire_execution_claim(run_id, token)
            }

            fn execution_claim_status<'a>(
                &'a self,
                run_id: &'a RunId,
            ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
                self.inner.execution_claim_status(run_id)
            }

            fn renew_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
                self.inner.renew_execution_claim(run_id, token)
            }

            fn release_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                self.inner.release_execution_claim(run_id, token)
            }

            fn expired_execution_claims<'a>(
                &'a self,
            ) -> store::AsyncStoreFuture<'a, Vec<store::ExpiredExecutionClaim>, Self::Error> {
                self.inner.expired_execution_claims()
            }

            fn reap_expired_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                self.inner.reap_expired_execution_claim(run_id, token)
            }
        }
    };
}

macro_rules! delegate_execution_claim_store_to_refcell_inner {
    ($ty:ty) => {
        impl store::ExecutionClaimStore for $ty {
            type Error = store::StoreError;

            fn acquire_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
                let result = block_on_ready(
                    self.inner
                        .borrow_mut()
                        .acquire_execution_claim(run_id, token),
                );
                Box::pin(std::future::ready(result))
            }

            fn execution_claim_status<'a>(
                &'a self,
                run_id: &'a RunId,
            ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
                let result = block_on_ready(self.inner.borrow().execution_claim_status(run_id));
                Box::pin(std::future::ready(result))
            }

            fn renew_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
                let result =
                    block_on_ready(self.inner.borrow_mut().renew_execution_claim(run_id, token));
                Box::pin(std::future::ready(result))
            }

            fn release_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                let result = block_on_ready(
                    self.inner
                        .borrow_mut()
                        .release_execution_claim(run_id, token),
                );
                Box::pin(std::future::ready(result))
            }

            fn expired_execution_claims<'a>(
                &'a self,
            ) -> store::AsyncStoreFuture<'a, Vec<store::ExpiredExecutionClaim>, Self::Error> {
                let result = block_on_ready(self.inner.borrow().expired_execution_claims());
                Box::pin(std::future::ready(result))
            }

            fn reap_expired_execution_claim<'a>(
                &'a self,
                run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                let result = block_on_ready(
                    self.inner
                        .borrow_mut()
                        .reap_expired_execution_claim(run_id, token),
                );
                Box::pin(std::future::ready(result))
            }
        }
    };
}

fn derive_fixture_saga(
    fixture: &Fixture,
    projection: store::ProjectionSnapshot,
) -> store::SagaProjection {
    projection
        .derive_saga_projection(
            &fixture.run_id,
            &fixture.runtime_spec.spec().saga,
            &fixture_terminal_policies(fixture),
        )
        .expect("saga projection")
}

fn side_effect_pair_fields_for_purpose(
    runtime_spec: &CertifiedRuntimeSpec,
    node_id: &NodeId,
    ledger_purpose: &events::SideEffectLedgerPurpose,
    role: events::SideEffectPairRole,
) -> (SideEffectPairId, events::SideEffectPairRole) {
    match ledger_purpose {
        events::SideEffectLedgerPurpose::Forward
        | events::SideEffectLedgerPurpose::Remediation { .. } => {
            let pair_id = runtime_spec
                .side_effect_pair_for_submit_node(node_id)
                .cloned()
                .expect("certified side-effect pair");
            (pair_id, role)
        }
    }
}

fn side_effect_pair_fields_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    ledger_purpose: &events::SideEffectLedgerPurpose,
    role: events::SideEffectPairRole,
) -> (SideEffectPairId, events::SideEffectPairRole) {
    side_effect_pair_fields_for_purpose(
        ctx.runtime_spec(),
        &ctx.node().node_id,
        ledger_purpose,
        role,
    )
}

fn run_identity_material(runtime_spec: &CertifiedRuntimeSpec) -> events::RunIdentityMaterialV1 {
    run_identity_material_with_distinct(runtime_spec, None)
}

fn run_identity_material_with_distinct(
    runtime_spec: &CertifiedRuntimeSpec,
    distinct_run_key_digest: Option<ContentDigest>,
) -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash: runtime_spec.spec_hash().clone(),
        trust_scope_id: fixture_trust_scope_id(),
        distinct_run_key_digest,
    }
}

fn fixture_run_identity_material(fixture: &Fixture) -> events::RunIdentityMaterialV1 {
    run_identity_material_with_distinct(
        &fixture.runtime_spec,
        fixture.distinct_run_key_digest.clone(),
    )
}

fn refresh_fixture_run_id(fixture: &mut Fixture) {
    let distinct_run_key_digest = fixture.distinct_run_key_digest.clone();
    refresh_fixture_run_id_with_distinct(fixture, distinct_run_key_digest);
}

fn refresh_fixture_run_id_with_distinct(
    fixture: &mut Fixture,
    distinct_run_key_digest: Option<ContentDigest>,
) {
    fixture.distinct_run_key_digest = distinct_run_key_digest.clone();
    fixture.run_id =
        run_identity_material_with_distinct(&fixture.runtime_spec, distinct_run_key_digest)
            .derive_run_id()
            .expect("fixture run id");
}

macro_rules! store_typed_commit_request {
    (
        run_id: $run_id:expr,
        expected_next_seq: $expected_next_seq:expr,
        commit_key: $commit_key:expr,
        payloads: $payloads:expr,
        required_artifacts: $required_artifacts:expr,
        preconditions: $preconditions:expr $(,)?
    ) => {
        store::CommitRequest::from_payloads(
            $run_id,
            $expected_next_seq,
            $commit_key,
            $payloads,
            $required_artifacts,
            $preconditions,
        )
        .expect("typed commit request")
    };
}

trait TestPreparedCommitExt {
    fn append_prepared_commit(
        &mut self,
        request: store::CommitRequest,
    ) -> store::Result<store::CommitOutcome>;
}

impl TestPreparedCommitExt for TestTypedRunStore {
    fn append_prepared_commit(
        &mut self,
        request: store::CommitRequest,
    ) -> store::Result<store::CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_test_commit_plan(plan)
    }
}

fn validate_runtime_stream_for_tests(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    RuntimeRunView::from_stream(runtime_spec, run_id, stream).map(|_| ())
}

fn block_on_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test in-memory store future unexpectedly pending"),
    }
}

#[derive(Clone, Default)]
struct RunnerKitArtifactProvider {
    artifacts: BTreeMap<ArtifactId, (Vec<u8>, mfm_artifact_capabilities::ArtifactEvidenceRef)>,
}

impl RunnerKitArtifactProvider {
    fn new(artifacts: Vec<(Vec<u8>, mfm_artifact_capabilities::ArtifactEvidenceRef)>) -> Self {
        Self {
            artifacts: artifacts
                .into_iter()
                .map(|(bytes, evidence)| (evidence.artifact_id.clone(), (bytes, evidence)))
                .collect(),
        }
    }
}

impl ArtifactReadProvider for RunnerKitArtifactProvider {
    fn read_artifact<'a>(
        &'a self,
        request: &'a ArtifactReadRequest,
    ) -> mfm_artifact_capabilities::ArtifactReadFuture<'a> {
        Box::pin(async move {
            let Some((bytes, evidence)) = self.artifacts.get(request.artifact_id()) else {
                return Err(mfm_artifact_capabilities::ArtifactReadError::NotFound {
                    artifact_id: Box::new(request.artifact_id().clone()),
                });
            };
            VerifiedArtifactBytes::new(bytes.clone(), evidence.clone(), request)
        })
    }
}

#[derive(Clone, Default)]
struct TestTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
}

impl TestTypedRunStore {
    fn new() -> Self {
        Self::default()
    }

    fn append_test_commit_plan(
        &self,
        plan: store::PreparedCommitPlan,
    ) -> store::Result<store::CommitOutcome> {
        self.inner
            .seed_artifact_evidence_for_test(plan.admitted_artifacts())?;
        let bundle = test_bundle_from_plan(plan)?;
        block_on_ready(self.inner.append_prepared_commit_bundle(bundle))
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        block_on_ready(self.inner.load_run_stream(run_id)).expect("test store read")
    }

    fn run_admitted(&self, run_id: &RunId) -> Box<events::RunAdmitted> {
        self.load_run_stream(run_id)
            .into_iter()
            .find_map(|event| match event.payload().clone() {
                events::KernelEventPayload::RunAdmitted(payload) => Some(payload),
                _ => None,
            })
            .expect("RunAdmitted payload")
    }

    fn assert_run_stream_len(&self, run_id: &RunId, expected: usize) {
        assert_eq!(self.load_run_stream(run_id).len(), expected);
    }

    fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        block_on_ready(self.inner.expected_next_seq(run_id)).expect("test store next seq")
    }

    fn projection_snapshot(&self) -> store::ProjectionSnapshot {
        self.inner
            .projection_snapshot()
            .expect("test store projection")
    }
}

impl store::RunEventStore for TestTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.inner.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

delegate_execution_claim_store_to_inner!(TestTypedRunStore);

impl store::RetainedArtifactReadProvider for TestTypedRunStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        self.inner.read_retained_artifact(requirement)
    }
}

#[derive(Clone)]
struct RecordedPreparedCommit {
    seq: store::StreamSeq,
    commit_key: store::CommitKey,
    payloads: Vec<events::KernelEventPayload>,
    admitted_artifacts: Vec<store::ArtifactEvidenceRef>,
}

struct RecordingTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
    commits: Arc<Mutex<Vec<RecordedPreparedCommit>>>,
}

impl RecordingTypedRunStore {
    fn new() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            commits: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn commits(&self) -> Vec<RecordedPreparedCommit> {
        self.commits
            .lock()
            .expect("recording store commits lock")
            .clone()
    }

    async fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.inner
            .load_run_stream(run_id)
            .await
            .expect("recording store read")
    }

    async fn projection_snapshot(&self, run_id: &RunId) -> store::ProjectionSnapshot {
        self.inner
            .status_projection_snapshot(run_id)
            .await
            .expect("recording store projection")
    }
}

struct StaleOnceTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
    stale_terminal_injected: Mutex<bool>,
}

impl StaleOnceTypedRunStore {
    fn new() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            stale_terminal_injected: Mutex::new(false),
        }
    }

    async fn projection_snapshot(&self, run_id: &RunId) -> store::ProjectionSnapshot {
        self.inner
            .status_projection_snapshot(run_id)
            .await
            .expect("stale-once store projection")
    }
}

impl store::RunEventStore for RecordingTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let payloads = bundle.request().payloads().to_vec();
        let admitted_artifacts = bundle.admitted_artifacts().to_vec();
        Box::pin(async move {
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            let outcome = self.inner.append_prepared_commit_bundle(bundle).await?;
            if let store::CommitOutcome::Appended(batch) = &outcome {
                self.commits
                    .lock()
                    .map_err(|_| {
                        store::StoreError::Event("recording store lock poisoned".to_owned())
                    })?
                    .push(RecordedPreparedCommit {
                        seq: batch.seq(),
                        commit_key: batch.commit_key().clone(),
                        payloads,
                        admitted_artifacts,
                    });
            }
            Ok(outcome)
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.inner.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

delegate_execution_claim_store_to_inner!(RecordingTypedRunStore);

impl store::RunEventStore for StaleOnceTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            let is_run_start = bundle
                .request()
                .payloads()
                .iter()
                .any(|payload| matches!(payload, events::KernelEventPayload::RunAdmitted(_)));
            let has_terminal = bundle.request().payloads().iter().any(|payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::StateAttemptCompleted(_)
                        | events::KernelEventPayload::StateAttemptFailed(_)
                        | events::KernelEventPayload::StateAttemptInterrupted(_)
                )
            });
            let should_inject = {
                let mut injected = self.stale_terminal_injected.lock().map_err(|_| {
                    store::StoreError::Event("stale-once store lock poisoned".to_owned())
                })?;
                let should_inject = !*injected && !is_run_start && has_terminal;
                if should_inject {
                    *injected = true;
                }
                should_inject
            };
            if should_inject {
                let expected = bundle.request().expected_next_seq();
                let run_id = bundle.request().run_id().clone();
                self.inner
                    .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
                self.inner.append_prepared_commit_bundle(bundle).await?;
                return Err(store::StoreError::StaleExpectedNextSeq {
                    expected,
                    actual: self.inner.expected_next_seq(&run_id).await?,
                });
            }
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.inner.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

delegate_execution_claim_store_to_inner!(StaleOnceTypedRunStore);

type TestArtifactMap = BTreeMap<ArtifactId, (Vec<u8>, store::ArtifactEvidenceRef)>;

#[derive(Clone, Default)]
struct TestRuntimeArtifactStore {
    artifacts: Arc<Mutex<TestArtifactMap>>,
}

impl store::RetainedArtifactReadProvider for TestRuntimeArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let (bytes, evidence) = self
                .artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .get(&requirement.artifact_id)
                .cloned()
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRunArtifactBytes::new(bytes, evidence, requirement)
        })
    }
}

#[derive(Clone)]
struct FilteringRuntimeArtifactStore {
    source: TestTypedRunStore,
    missing_artifacts: Arc<Mutex<BTreeSet<ArtifactId>>>,
}

impl FilteringRuntimeArtifactStore {
    fn new(source: TestTypedRunStore) -> Self {
        Self {
            source,
            missing_artifacts: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    fn hide_artifact(&self, artifact_id: ArtifactId) {
        self.missing_artifacts
            .lock()
            .expect("filtering artifact store")
            .insert(artifact_id);
    }
}

impl store::RetainedArtifactReadProvider for FilteringRuntimeArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            if self
                .missing_artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .contains(&requirement.artifact_id)
            {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            }
            self.source.read_retained_artifact(requirement).await
        })
    }
}

fn test_scheduler(registry: ErasedRunnerRegistry) -> SerialTypedScheduler {
    test_scheduler_with_artifacts(registry, Arc::new(TestRuntimeArtifactStore::default()))
}

fn test_scheduler_with_artifacts(
    registry: ErasedRunnerRegistry,
    artifact_store: Arc<dyn RuntimeArtifactStore>,
) -> SerialTypedScheduler {
    SerialTypedScheduler::new(registry, artifact_store)
}

fn fixture_scheduler(registry: ErasedRunnerRegistry, fixture: &Fixture) -> SerialTypedScheduler {
    test_scheduler(register_fixture_capabilities(registry, fixture))
}

fn register_read_external_fixture_runner(registry: &mut ErasedRunnerRegistry, fixture: &Fixture) {
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            READ_EXTERNAL_RUNNER,
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding read");
}

fn register_default_fixture_read_runner(registry: &mut ErasedRunnerRegistry, fixture: &Fixture) {
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
}

fn register_default_fixture_pure_runner(registry: &mut ErasedRunnerRegistry, fixture: &Fixture) {
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
}

fn fixture_registry_with_first_runner<R: ErasedNodeRunner + 'static>(
    fixture: &Fixture,
    runner_name: &'static str,
    runner: R,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(fixture.descriptor_a.clone(), runner_name, runner))
        .expect("binding a");
    register_default_fixture_read_runner(&mut registry, fixture);
    registry
}

fn side_effect_driver_registry_with_submission_decision(
    fixture: &Fixture,
    decision: TestSubmissionDecision,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture).with_submission_decision(decision),
        ))
        .expect("binding side effect");
    register_read_external_fixture_runner(&mut registry, fixture);
    registry
}

macro_rules! assert_drive {
    ($scheduler:ident, $store:ident, $fixture:ident, $status:ident, $message:literal $(,)?) => {
        assert_eq!(
            drive_fixture_once(&$scheduler, &mut $store, &$fixture)
                .await
                .expect($message),
            SchedulerStatus::$status
        );
    };
}

macro_rules! drive_ok {
    ($scheduler:ident, $store:ident, $fixture:ident, $message:literal $(,)?) => {
        drive_fixture_once(&$scheduler, &mut $store, &$fixture)
            .await
            .expect($message)
    };
}

macro_rules! assert_first_node_invalid_after_drive {
    ($scheduler:ident, $store:ident, $fixture:ident, $message:literal $(,)?) => {{
        assert_drive!($scheduler, $store, $fixture, Advanced, $message);
        let node = node_by_output(&$fixture, &$fixture.cell_a);
        assert_node_failed_with_code(&$store, &node.node_id, "runner_output_invalid");
    }};
}

const TOUCHED_SET_EVIDENCE_ERR: &str = "without touched-set evidence";
const EXACT_TOUCHED_SET_CLAIM_ERR: &str = "without an exact-touched-set certified resource claim";

fn register_fixture_capabilities(
    mut registry: ErasedRunnerRegistry,
    fixture: &Fixture,
) -> ErasedRunnerRegistry {
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_side_effect_verify_fixture_runner(&mut registry, fixture);
    registry
}

fn register_spec_capabilities(
    registry: &mut ErasedRunnerRegistry,
    runtime_spec: &CertifiedRuntimeSpec,
) {
    register_spec_capabilities_with_adapter_executable(
        registry,
        runtime_spec,
        test_adapter_executable_identity(),
    );
}

fn register_spec_capabilities_with_adapter_executable(
    registry: &mut ErasedRunnerRegistry,
    runtime_spec: &CertifiedRuntimeSpec,
    adapter_executable: events::ExecutableIdentity,
) {
    let implementation_id =
        CapabilityImplementationId::new("mfm.test.capability").expect("capability implementation");
    for node in runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(runtime_spec.spec().remediations.values())
    {
        registry
            .register_capability_set(&node.capability_bindings, implementation_id.clone())
            .expect("capability binding");
        for adapter in &node.adapter_bindings {
            registry
                .register_adapter_executable(AdapterExecutableBinding::new(
                    adapter.adapter_kind.clone(),
                    adapter.adapter_version.clone(),
                    adapter_executable.clone(),
                ))
                .expect("adapter executable binding");
        }
    }
}

fn test_adapter_executable_identity() -> events::ExecutableIdentity {
    let factory_id = events::RunnerFactoryId::new("test_adapter").expect("factory");
    events::ExecutableIdentity {
        factory_id,
        cargo_package_digest: content(0xe3),
        binary_digest: content(0xe4),
        nix_derivation_hash: None,
        nix_output_hash: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "value",
    version = "1",
    schema = "mfm.runtime.test.value"
)]
struct CertifierValue {
    amount: u64,
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "fact",
    version = "1",
    schema = "mfm.runtime.test.fact"
)]
#[mfm_fact(kind = "mfm.runtime.test.fact")]
#[mfm_fact(field(
    id = "subject.amount",
    source = "subject",
    path = "amount",
    value_type = "unsigned_integer",
    operator = "equal",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.amount",
    source = "result",
    path = "amount",
    value_type = "unsigned_integer",
    operators(equal, greater_than),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(ordering(
    name = "result.amount.asc",
    term(field = "result.amount", direction = "ascending", nulls = "last")
))]
struct RuntimeTestFact {
    subject: CertifierValue,
    response: CertifierValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FixtureOutputValue {
    amount: u64,
    node_id: String,
    attempt_id: String,
}

fn fixture_output_value(
    amount: u64,
    node_id: impl Into<String>,
    attempt_id: impl Into<String>,
) -> FixtureOutputValue {
    FixtureOutputValue {
        amount,
        node_id: node_id.into(),
        attempt_id: attempt_id.into(),
    }
}

impl mfm_values::MfmValue for FixtureOutputValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        <CertifierValue as mfm_values::MfmValue>::schema_descriptor()
    }

    fn schema_id() -> mfm_values::Result<SchemaId> {
        Ok(fixture_value_schema_id())
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        Ok(fixture_value_semantic_id())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.test",
    name = "side_effect_evidence",
    version = "1",
    schema = "mfm.runtime.test.side_effect_evidence"
)]
struct FixtureSideEffectEvidence {
    amount: u64,
    node_id: String,
    attempt_id: String,
}

fn fixture_side_effect_evidence(
    amount: u64,
    node_id: impl Into<String>,
    attempt_id: impl Into<String>,
) -> FixtureSideEffectEvidence {
    FixtureSideEffectEvidence {
        amount,
        node_id: node_id.into(),
        attempt_id: attempt_id.into(),
    }
}

fn fixture_side_effect_evidence_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    amount: u64,
) -> FixtureSideEffectEvidence {
    fixture_side_effect_evidence(
        amount,
        ctx.node().node_id.as_str(),
        ctx.attempt_id().as_str(),
    )
}

impl From<&FixtureSideEffectEvidence> for FixtureOutputValue {
    fn from(evidence: &FixtureSideEffectEvidence) -> Self {
        fixture_output_value(evidence.amount, &evidence.node_id, &evidence.attempt_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct CertifierConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.runtime.test.public_outputs")]
struct CertifierPublicOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, CertifierValue>,
}

struct CertifierState {
    config: CertifierConfig,
}

impl StateSpec for CertifierState {
    type Config = CertifierConfig;
    type Input = CertifierValue;
    type Output = CertifierValue;
    type Effect = mfm_effects::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.runtime.test",
            "multiply",
            DigestAlgorithm::Sha256JcsV1,
            D1,
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.runtime.test.multiply.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.runtime.test.multiply"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for CertifierState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(CertifierValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.runtime.test.fixture_outputs")]
struct FixturePublicOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, FixtureOutputValue>,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.runtime.test.dual_fixture_outputs")]
struct DualFixturePublicOutputs<'p, 's> {
    result: mfm_program::Handle<'p, 's, FixtureOutputValue>,
    side_effect: mfm_program::Handle<'p, 's, FixtureOutputValue>,
}

struct RuntimeReadCap;

impl CapabilitySpec for RuntimeReadCap {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new("mfm.test", "read-db", DigestAlgorithm::Sha256JcsV1, D0)
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.cap.read_db.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "read-db"
    }
}

struct RuntimeMutationCap;

impl CapabilitySpec for RuntimeMutationCap {
    type Role = ExternalMutationAuthorityRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        Ok(side_effect_capability_kind())
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        Ok(side_effect_capability_version())
    }

    fn name() -> &'static str {
        "external-mutation"
    }
}

fn runtime_adapter_binding() -> AdapterBindingSpec {
    AdapterBindingSpec {
        adapter_kind: AdapterKind::new("mfm.test", "adapter", DigestAlgorithm::Sha256JcsV1, D1)
            .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.adapter.v1").expect("adapter version"),
    }
}

fn runtime_state_kind(name: &str, digest: DigestBytes) -> mfm_program::Result<StateKind> {
    StateKind::new(
        "mfm.runtime.test",
        name,
        DigestAlgorithm::Sha256JcsV1,
        digest,
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

macro_rules! impl_runtime_read_state {
    ($state:ident, $input:ty, $kind:literal, $version:literal, $name:literal, $digest:expr) => {
        struct $state {
            config: CertifierConfig,
        }

        impl StateSpec for $state {
            type Config = CertifierConfig;
            type Input = $input;
            type Output = FixtureOutputValue;
            type Effect = mfm_effects::ReadExternal;
            type Caps = (RuntimeReadCap,);

            fn kind() -> mfm_program::Result<StateKind> {
                runtime_state_kind($kind, $digest)
            }

            fn version() -> mfm_program::Result<StateVersion> {
                StateVersion::new($version)
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                $name
            }

            fn new(
                config: mfm_program::ValidatedConfig<Self::Config>,
            ) -> mfm_program::Result<Self> {
                Ok(Self {
                    config: config.into_inner(),
                })
            }
        }

        impl ReadState for $state {
            type RunFuture<'a> = std::future::Ready<StateResult<Self::Output>>;

            fn run<'a>(
                &'a self,
                _input: Self::Input,
                _caps: &'a Self::Caps,
            ) -> Self::RunFuture<'a> {
                std::future::ready(Ok(fixture_output_value(
                    self.config.multiplier,
                    $name,
                    "typed-read",
                )))
            }
        }
    };
}

macro_rules! impl_runtime_side_effect_state {
    ($state:ident, $input:ty, $kind:literal, $version:literal, $name:literal, $digest:expr) => {
        struct $state {
            config: CertifierConfig,
        }

        impl StateSpec for $state {
            type Config = CertifierConfig;
            type Input = $input;
            type Output = FixtureOutputValue;
            type Effect = mfm_effects::ApplySideEffect;
            type Caps = (RuntimeMutationCap,);

            fn kind() -> mfm_program::Result<StateKind> {
                runtime_state_kind($kind, $digest)
            }

            fn version() -> mfm_program::Result<StateVersion> {
                StateVersion::new($version)
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
            }

            fn name() -> &'static str {
                $name
            }

            fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
                Ok(vec![runtime_adapter_binding()])
            }

            fn new(
                config: mfm_program::ValidatedConfig<Self::Config>,
            ) -> mfm_program::Result<Self> {
                Ok(Self {
                    config: config.into_inner(),
                })
            }
        }

        impl SideEffectState for $state {
            type Intent = FixtureSideEffectEvidence;
            type IdempotencyInput = FixtureSideEffectEvidence;
            type Submission = FixtureSideEffectEvidence;
            type Receipt = FixtureSideEffectEvidence;
            type Confirmation = FixtureSideEffectEvidence;
            type SubmitFuture<'a> = std::future::Ready<StateResult<Self::Submission>>;

            fn prepare_intent(&self, input: &Self::Input) -> StateResult<Self::Intent> {
                let amount = serde_json::to_value(input)
                    .ok()
                    .and_then(|value| value.get("amount").and_then(serde_json::Value::as_u64))
                    .unwrap_or(self.config.multiplier);
                Ok(fixture_side_effect_evidence(amount, $name, "typed-intent"))
            }

            fn idempotency_input(
                &self,
                _input: &Self::Input,
                intent: &Self::Intent,
            ) -> StateResult<Self::IdempotencyInput> {
                Ok(intent.clone())
            }

            fn submit<'a>(
                &'a self,
                intent: &'a Self::Intent,
                _key: &'a IdempotencyKey<Self::IdempotencyInput>,
                _caps: &'a Self::Caps,
            ) -> Self::SubmitFuture<'a> {
                std::future::ready(Ok(intent.clone()))
            }

            fn output_from_receipt(
                &self,
                _input: &Self::Input,
                _intent: &Self::Intent,
                receipt: &Self::Receipt,
            ) -> StateResult<Self::Output> {
                Ok(receipt.into())
            }

            fn output_from_confirmation(
                &self,
                _input: &Self::Input,
                _intent: &Self::Intent,
                confirmation: &Self::Confirmation,
            ) -> StateResult<Self::Output> {
                Ok(confirmation.into())
            }
        }
    };
}

impl_runtime_side_effect_state!(
    RuntimeSubmitAState,
    CertifierValue,
    "submit-a",
    "mfm.runtime.test.submit_a.v1",
    "mfm.runtime.test.submit_a",
    DigestBytes::from_array([0xa1; 32])
);
impl_runtime_side_effect_state!(
    RuntimeSubmitBState,
    FixtureOutputValue,
    "submit-b",
    "mfm.runtime.test.submit_b.v1",
    "mfm.runtime.test.submit_b",
    DigestBytes::from_array([0xa2; 32])
);
impl_runtime_read_state!(
    RuntimeReadState,
    FixtureOutputValue,
    "read-output",
    "mfm.runtime.test.read_output.v1",
    "mfm.runtime.test.read_output",
    DigestBytes::from_array([0xa3; 32])
);
impl_runtime_read_state!(
    RuntimeSeedReadState,
    CertifierValue,
    "read-seed",
    "mfm.runtime.test.read_seed.v1",
    "mfm.runtime.test.read_seed",
    DigestBytes::from_array([0xa4; 32])
);

struct RuntimeTailState {
    config: CertifierConfig,
}

impl StateSpec for RuntimeTailState {
    type Config = CertifierConfig;
    type Input = FixtureOutputValue;
    type Output = FixtureOutputValue;
    type Effect = mfm_effects::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        runtime_state_kind("tail", DigestBytes::from_array([0xa5; 32]))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.runtime.test.tail.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.runtime.test.tail"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for RuntimeTailState {
    fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
        Ok(fixture_output_value(
            input.amount * self.config.multiplier,
            input.node_id,
            input.attempt_id,
        ))
    }
}

fn certifier_backed_runtime_authority() -> (
    mfm_certify::CertifiedTypedSpec,
    mfm_certify::CertificationRegistry,
) {
    let mut states = StateRegistryBuilder::new();
    let registered = states
        .register::<CertifierState>()
        .expect("state registration");
    let mut registry = mfm_certify::CertificationRegistry::new();
    registry
        .register_state(&registered)
        .expect("certification registry");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial").expect("seed key"),
                CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed"),
            )?;
            let result = root.scope().state::<CertifierState, _>(
                StateKey::new("multiply-state")?,
                CertifierConfig { multiplier: 3 },
                seed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &CertifierPublicOutputs { result },
            )
        },
    )
    .expect("program draft");
    (
        mfm_certify::certify_program_draft(&draft).expect("certified program"),
        registry,
    )
}

const DA: DigestBytes = DigestBytes::from_array([0x1a; 32]);
const DB: DigestBytes = DigestBytes::from_array([0x1b; 32]);
const DC: DigestBytes = DigestBytes::from_array([0x1c; 32]);
const DD: DigestBytes = DigestBytes::from_array([0x1d; 32]);
const DE: DigestBytes = DigestBytes::from_array([0x1e; 32]);
const DF: DigestBytes = DigestBytes::from_array([0x1f; 32]);
const TEST_CONFIG_BYTES: &[u8] = b"{}";
const TEST_SEED_BYTES: &[u8] = br#"{"seed":true}"#;
const CERTIFIER_SEED_BYTES: &[u8] = br#"{"amount":2}"#;
const CONFIG_MULTIPLIER_1_BYTES: &[u8] = br#"{"multiplier":1}"#;
const CONFIG_MULTIPLIER_3_BYTES: &[u8] = br#"{"multiplier":3}"#;
const CONFIG_MULTIPLIER_5_BYTES: &[u8] = br#"{"multiplier":5}"#;
const CONFIG_MULTIPLIER_7_BYTES: &[u8] = br#"{"multiplier":7}"#;

#[derive(Clone)]
struct Fixture {
    runtime_spec: CertifiedRuntimeSpec,
    run_id: RunId,
    distinct_run_key_digest: Option<ContentDigest>,
    seed_ref: events::SeedCellRef,
    descriptor_a: DescriptorId,
    descriptor_b: DescriptorId,
    descriptor_c: Option<DescriptorId>,
    render_node: NodeId,
    render_cell: CellId,
    cell_a: CellId,
    cell_b: CellId,
    cell_c: Option<CellId>,
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
}

struct RecordingRunner {
    expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
}

impl ErasedNodeRunner for RecordingRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            for (kind, version) in &self.expected_caps {
                assert!(ctx.caps().contains(kind, version));
            }
            let output_cell = ctx.node().output_cell.clone();
            let cell = match ctx.inputs().root.clone() {
                MaterializedInputNode::Cell(cell) => cell,
                MaterializedInputNode::Unit
                | MaterializedInputNode::Tuple(_)
                | MaterializedInputNode::Struct(_)
                | MaterializedInputNode::Vec(_)
                | MaterializedInputNode::NonEmptyVec(_) => {
                    panic!("expected cell input")
                }
            };
            assert!(matches!(
                cell.terminal,
                MaterializedCellTerminal::Seed { .. } | MaterializedCellTerminal::Produced { .. }
            ));
            let certified_cell = ctx.projections().cell_terminal(&output_cell).is_none();
            assert!(certified_cell);
            let artifact = store::ArtifactEvidenceRef {
                artifact_id: self.output_artifact.clone(),
                digest: self.output_digest.clone(),
                byte_len: 17,
                media_type: spec::MediaType::new("application/json").expect("media"),
                schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                producer_node_id: Some(ctx.node().node_id.clone()),
                producer_seed_id: None,
                artifact_role: events::ArtifactRole::StateOutput,
            };
            let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: Vec::new(),
                payloads: vec![RunnerEventPayload::CellProduced(events::CellProduced {
                    spec_hash: ctx.spec_hash().clone(),
                    node_id: ctx.node().node_id.clone(),
                    cell_id: ctx.node().output_cell.clone(),
                    scope_id: ctx.node().scope_id.clone(),
                    attempt_id: ctx.attempt_id().clone(),
                    semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
                    schema_id: ctx.descriptor().output_schema_id.clone(),
                    value_lineage: ctx.output_cell().value_lineage.clone(),
                    artifact_id: self.output_artifact.clone(),
                    content_digest: self.output_digest.clone(),
                    producer_state_kind: Some(ctx.node().state_kind.clone()),
                    producer_state_version: Some(ctx.node().state_version.clone()),
                })],
            })
        })
    }
}

struct BlockingRunner;

impl ErasedNodeRunner for BlockingRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::Blocked(format!(
                "node {} is failed manually in this fixture",
                ctx.node().node_id
            )))
        })
    }
}

struct ErrorRunner {
    error: RuntimeError,
}

impl ErasedNodeRunner for ErrorRunner {
    fn run_erased<'a>(&'a self, _ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { Err(self.error.clone()) })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct RunnerKitEmptyConfig {}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct RunnerKitStructInput {
    left: CertifierValue,
    right: Vec<CertifierValue>,
}

fn runner_kit_config_artifact(
    node: &spec::NodeSpec,
    bytes: &[u8],
) -> mfm_artifact_capabilities::ArtifactEvidenceRef {
    assert_eq!(node.config_ref.digest, digest_for_bytes(bytes));
    mfm_artifact_capabilities::ArtifactEvidenceRef {
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: node.config_ref.digest.clone(),
        byte_len: node.config_ref.byte_len,
        media_type: node.config_ref.media_type.clone(),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    }
}

fn runner_kit_value_artifact(
    bytes: &[u8],
    artifact_role: events::ArtifactRole,
    producer_node_id: Option<NodeId>,
    producer_seed_id: Option<SeedId>,
) -> mfm_artifact_capabilities::ArtifactEvidenceRef {
    let digest = digest_for_bytes(bytes);
    mfm_artifact_capabilities::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(fixture_value_schema_id()),
        semantic_type_id: Some(fixture_value_semantic_id()),
        producer_node_id,
        producer_seed_id,
        artifact_role,
    }
}

fn runner_kit_input_cell(
    evidence: &mfm_artifact_capabilities::ArtifactEvidenceRef,
    terminal: MaterializedCellTerminal,
) -> MaterializedInputNode {
    MaterializedInputNode::Cell(Box::new(MaterializedCell {
        cell_id: CellId::from_digest(evidence.digest.algorithm(), *evidence.digest.digest()),
        schema_id: evidence.schema_id.clone().expect("schema id"),
        semantic_type_id: evidence.semantic_type_id.clone().expect("semantic id"),
        value_lineage: spec::ValueLineageRef {
            lineage_digest: evidence.digest.clone(),
        },
        terminal,
    }))
}

fn runner_kit_skipped_cell() -> MaterializedInputNode {
    MaterializedInputNode::Cell(Box::new(MaterializedCell {
        cell_id: CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D8),
        schema_id: fixture_value_schema_id(),
        semantic_type_id: fixture_value_semantic_id(),
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0x78),
        },
        terminal: MaterializedCellTerminal::Skipped {
            skip_reason: events::SkipReason {
                code: events::ErrorCode::new("runner_kit_skip").expect("skip code"),
                safe_message: "input was skipped".to_owned(),
            },
        },
    }))
}

#[test]
fn runner_kit_loads_config_and_materialized_inputs() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let left_bytes = br#"{"amount":4}"#;
    let right_a_bytes = br#"{"amount":7}"#;
    let right_b_bytes = br#"{"amount":9}"#;

    let config_evidence = runner_kit_config_artifact(node, TEST_CONFIG_BYTES);
    let left_evidence = runner_kit_value_artifact(
        left_bytes,
        events::ArtifactRole::StateOutput,
        Some(node.node_id.clone()),
        None,
    );
    let right_a_evidence = runner_kit_value_artifact(
        right_a_bytes,
        events::ArtifactRole::SeedInput,
        None,
        Some(fixture.seed_ref.seed_id.clone()),
    );
    let right_b_evidence = runner_kit_value_artifact(
        right_b_bytes,
        events::ArtifactRole::StateOutput,
        Some(node.node_id.clone()),
        None,
    );
    let artifacts = RunnerKitArtifactProvider::new(vec![
        (TEST_CONFIG_BYTES.to_vec(), config_evidence),
        (left_bytes.to_vec(), left_evidence.clone()),
        (right_a_bytes.to_vec(), right_a_evidence.clone()),
        (right_b_bytes.to_vec(), right_b_evidence.clone()),
    ]);

    let config = block_on_ready(load_runner_config_for_node::<RunnerKitEmptyConfig>(
        node, &artifacts,
    ))
    .expect("runner config");
    assert_eq!(config.into_inner(), RunnerKitEmptyConfig {});

    let left_node = runner_kit_input_cell(
        &left_evidence,
        MaterializedCellTerminal::Produced {
            producer_node_id: node.node_id.clone(),
            artifact_id: left_evidence.artifact_id.clone(),
            content_digest: left_evidence.digest.clone(),
        },
    );
    let right_a_node = runner_kit_input_cell(
        &right_a_evidence,
        MaterializedCellTerminal::Seed {
            seed_id: fixture.seed_ref.seed_id.clone(),
            artifact_id: right_a_evidence.artifact_id.clone(),
            content_digest: right_a_evidence.digest.clone(),
        },
    );
    let right_b_node = runner_kit_input_cell(
        &right_b_evidence,
        MaterializedCellTerminal::Produced {
            producer_node_id: node.node_id.clone(),
            artifact_id: right_b_evidence.artifact_id.clone(),
            content_digest: right_b_evidence.digest.clone(),
        },
    );
    let inputs = MaterializedInputs {
        input_schema_id: fixture_value_schema_id(),
        root: MaterializedInputNode::Struct(vec![
            NamedMaterializedInput {
                field_path: spec::PublicFieldPath::new("left").expect("field path"),
                node: left_node.clone(),
            },
            NamedMaterializedInput {
                field_path: spec::PublicFieldPath::new("right").expect("field path"),
                node: MaterializedInputNode::Vec(vec![right_a_node.clone(), right_b_node.clone()]),
            },
        ]),
    };

    let decoded = block_on_ready(load_materialized_struct_input::<RunnerKitStructInput>(
        &inputs, &artifacts,
    ))
    .expect("struct input");
    assert_eq!(decoded.left, CertifierValue { amount: 4 });
    assert_eq!(
        decoded.right,
        vec![CertifierValue { amount: 7 }, CertifierValue { amount: 9 }]
    );

    let left = block_on_ready(load_materialized_struct_field_value::<CertifierValue>(
        &inputs, "left", &artifacts,
    ))
    .expect("struct field");
    assert_eq!(left, CertifierValue { amount: 4 });

    let non_empty_inputs = MaterializedInputs {
        input_schema_id: fixture_value_schema_id(),
        root: MaterializedInputNode::NonEmptyVec(vec![right_a_node, right_b_node]),
    };
    let values = block_on_ready(load_non_empty_materialized_input::<CertifierValue>(
        &non_empty_inputs,
        &artifacts,
    ))
    .expect("non-empty input");
    assert_eq!(
        values.values(),
        &[CertifierValue { amount: 7 }, CertifierValue { amount: 9 }]
    );
}

#[test]
fn runner_kit_rejects_skipped_materialized_input_cell() {
    let artifacts = RunnerKitArtifactProvider::default();
    let error = block_on_ready(load_materialized_node_value::<CertifierValue>(
        &runner_kit_skipped_cell(),
        &artifacts,
    ))
    .expect_err("skipped cells cannot be loaded");

    assert!(matches!(
        error,
        RuntimeError::InvalidRunnerOutput(message) if message.contains("skipped")
    ));
}

#[test]
fn runner_kit_builders_create_context_bound_artifacts_payloads_and_output() {
    let fixture = fixture();
    let fact_descriptor_ref =
        mfm_program::fact_descriptor_ref::<RuntimeTestFact>().expect("fact descriptor ref");
    let mut node = node_by_output(&fixture, &fixture.cell_a).clone();
    node.fact_descriptor_allowlist = vec![fact_descriptor_ref.clone()];

    with_runner_erased_ctx_for_node(&fixture, &node, |ctx| {
        let artifacts = RunnerArtifactBuilder::new(&ctx);
        let payloads = RunnerPayloadBuilder::new(&ctx);
        let value = CertifierValue { amount: 42 };
        let binding = RunnerCapabilityBinding {
            capability_kind: fixture.cap_kind.clone(),
            capability_version: fixture.cap_version.clone(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
        };

        let state = artifacts.state_output(&value).expect("state output");
        assert_eq!(
            state.evidence().artifact_role,
            events::ArtifactRole::StateOutput
        );
        assert_eq!(
            state.evidence().producer_node_id.as_ref(),
            Some(&ctx.node().node_id)
        );
        assert_eq!(
            artifacts.content_digest(&value).expect("content digest"),
            state.evidence().digest
        );

        let cell = payloads.cell_produced(&state).expect("cell payload");
        match &cell {
            RunnerEventPayload::CellProduced(payload) => {
                assert_eq!(payload.spec_hash, *ctx.spec_hash());
                assert_eq!(payload.node_id, ctx.node().node_id);
                assert_eq!(payload.cell_id, ctx.node().output_cell);
                assert_eq!(payload.schema_id, ctx.output_cell().schema_id);
                assert_eq!(payload.semantic_type_id, ctx.output_cell().semantic_type_id);
                assert_eq!(payload.artifact_id, state.evidence().artifact_id);
                assert_eq!(payload.content_digest, state.evidence().digest);
            }
            _ => panic!("expected cell produced payload"),
        }

        let response = artifacts.fact_response(&value).expect("fact response");
        assert_eq!(
            response.evidence().artifact_role,
            events::ArtifactRole::FactResponse
        );
        assert_eq!(
            response.evidence().producer_node_id.as_ref(),
            Some(&ctx.node().node_id)
        );

        let fact = RuntimeTestFact {
            subject: CertifierValue { amount: 7 },
            response: CertifierValue { amount: 9 },
        };
        let expected_descriptor =
            <RuntimeTestFact as mfm_program::MfmFactType>::descriptor().expect("fact descriptor");
        let expected_descriptor_hash =
            mfm_facts::fact_descriptor_hash(&expected_descriptor).expect("descriptor hash");
        assert_eq!(
            fact_descriptor_ref.descriptor_hash,
            expected_descriptor_hash
        );

        let mut fact_output = RunnerOutputBuilder::new(&ctx);
        let staged_fact = fact_output
            .record_fact(
                FactRecordInput::new(
                    fact,
                    mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
                )
                .observed_at("2026-01-02T03:04:05Z"),
                binding.clone(),
            )
            .expect("record typed fact");
        let fact_output = fact_output.finish();
        assert_eq!(fact_output.staged_artifacts.len(), 1);
        assert_eq!(fact_output.staged_retention_refs.len(), 0);
        assert_eq!(fact_output.payloads.len(), 1);
        let fact_artifact = &fact_output.staged_artifacts[0];
        assert_eq!(
            fact_artifact.evidence().artifact_role,
            events::ArtifactRole::FactResponse
        );
        assert_eq!(
            fact_artifact.evidence().artifact_id,
            *staged_fact.response_artifact_id()
        );
        assert_eq!(
            fact_artifact.evidence().digest,
            *staged_fact.response_hash()
        );
        match &fact_output.payloads[0] {
            RunnerEventPayload::FactRecorded(recorded) => {
                let payload = recorded.payload();
                assert_eq!(payload.spec_hash, *ctx.spec_hash());
                assert_eq!(payload.node_id, ctx.node().node_id);
                assert_eq!(payload.attempt_id, *ctx.attempt_id());
                assert_eq!(
                    payload.claim.fact_descriptor_hash(),
                    &expected_descriptor_hash
                );
                assert_eq!(payload.claim.subject().fact_key(), staged_fact.fact_key());
                assert_eq!(payload.claim.observed_at(), Some("2026-01-02T03:04:05Z"));
                assert!(payload.claim.request().is_none());
                assert_eq!(
                    payload.claim.response().response_schema_id(),
                    &<CertifierValue as mfm_values::MfmValue>::schema_id()
                        .expect("response schema")
                );
                assert_eq!(
                    payload.claim.response().response_hash(),
                    staged_fact.response_hash()
                );
                assert_eq!(
                    payload.claim.response().artifact_id(),
                    staged_fact.response_artifact_id()
                );
                assert_eq!(
                    payload.claim.response().artifact_evidence_hash(),
                    &fact_artifact
                        .evidence()
                        .evidence_hash()
                        .expect("artifact evidence hash")
                );
                assert_eq!(
                    payload.claim.producer().capability_kind(),
                    &fixture.cap_kind
                );
                assert_eq!(
                    payload.claim.producer().capability_version(),
                    &fixture.cap_version
                );
                assert_eq!(
                    payload.claim.producer().adapter_kind(),
                    &fixture.adapter_kind
                );
                assert_eq!(
                    payload.claim.producer().adapter_version(),
                    &fixture.adapter_version
                );
            }
            _ => panic!("expected fact recorded payload"),
        }

        let composed_fact = RuntimeTestFact {
            subject: CertifierValue { amount: 8 },
            response: CertifierValue { amount: 10 },
        };
        let mut composed_output = RunnerOutputBuilder::new(&ctx);
        composed_output
            .state_output_and_record_fact(
                FactRecordInput::new(
                    composed_fact,
                    mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
                ),
                binding.clone(),
            )
            .expect("state output and fact record");
        let composed_output = composed_output.finish();
        assert_eq!(composed_output.staged_artifacts.len(), 2);
        assert_eq!(composed_output.staged_retention_refs.len(), 1);
        assert_eq!(composed_output.payloads.len(), 2);
        assert_eq!(
            composed_output.staged_artifacts[0].evidence().artifact_role,
            events::ArtifactRole::StateOutput
        );
        assert_eq!(
            composed_output.staged_artifacts[1].evidence().artifact_role,
            events::ArtifactRole::FactResponse
        );
        assert!(matches!(
            composed_output.payloads[0],
            RunnerEventPayload::FactRecorded(_)
        ));
        assert!(matches!(
            composed_output.payloads[1],
            RunnerEventPayload::CellProduced(_)
        ));

        let query_evidence = test_fact_query_evidence();
        let expected_query_evidence_hash =
            mfm_facts::fact_query_evidence_hash(&query_evidence).expect("query evidence hash");
        let expected_query_evidence_schema =
            mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema");
        let mut query_output = RunnerOutputBuilder::new(&ctx);
        let staged_query_evidence = query_output
            .record_fact_query_evidence(query_evidence, &test_fact_query_trust_root())
            .expect("record fact query evidence");
        let query_output = query_output.finish();
        assert_eq!(query_output.staged_artifacts.len(), 1);
        assert_eq!(query_output.staged_retention_refs.len(), 1);
        assert!(query_output.payloads.is_empty());
        let query_artifact = &query_output.staged_artifacts[0];
        assert_eq!(
            query_artifact.evidence().artifact_role,
            events::ArtifactRole::FactQueryEvidence
        );
        assert_eq!(
            query_artifact.evidence().schema_id.as_ref(),
            Some(&expected_query_evidence_schema)
        );
        assert!(query_artifact.evidence().semantic_type_id.is_none());
        assert_eq!(
            query_artifact.evidence().digest,
            expected_query_evidence_hash
        );
        assert_eq!(
            query_artifact.evidence().artifact_id,
            *staged_query_evidence.artifact_id()
        );
        assert_eq!(
            query_artifact
                .evidence()
                .evidence_hash()
                .expect("query artifact evidence hash"),
            *staged_query_evidence.evidence_hash()
        );
        let expected_query_retention = events::RetentionRef {
            artifact_id: query_artifact.evidence().artifact_id.clone(),
            role: events::ArtifactRole::FactQueryEvidence,
            content_digest: query_artifact.evidence().digest.clone(),
        };
        assert_eq!(
            query_output.staged_retention_refs[0].refs(),
            &[expected_query_retention]
        );

        let ledger_key =
            events::SideEffectLedgerKey::new("mfm.test.runner_kit.ledger").expect("ledger key");
        let side_effect = RunnerSideEffectBinding {
            ledger_key: ledger_key.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: synthetic_side_effect_pair_id(0x31),
            invocation_epoch: 1,
        };
        let idempotency_key =
            events::IdempotencyKeyRef::new("mfm.test.runner_kit.idem").expect("idempotency key");
        let owner =
            events::RunnerInvocationId::new("mfm.test.runner_kit.owner").expect("claim owner");
        let next_owner =
            events::RunnerInvocationId::new("mfm.test.runner_kit.owner.next").expect("claim owner");
        let token = events::side_effect::ClaimFencingToken::new("mfm.test.runner_kit.token")
            .expect("token");
        let next_token =
            events::side_effect::ClaimFencingToken::new("mfm.test.runner_kit.token.next")
                .expect("next token");
        let verifier =
            events::ReplayVerifierId::new("mfm.test.runner_kit.verifier").expect("verifier");

        let intent = artifacts
            .side_effect_intent(&value)
            .expect("side-effect intent");
        let intent_payload = payloads
            .side_effect_intent_persisted(
                side_effect.clone(),
                &intent,
                &value,
                idempotency_key,
                binding,
            )
            .expect("intent payload");
        match &intent_payload {
            RunnerEventPayload::SideEffectIntentPersisted(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(payload.intent_hash, intent.evidence().digest);
                assert_eq!(payload.intent_artifact_id, intent.evidence().artifact_id);
                assert_eq!(payload.idempotency_input_hash, state.evidence().digest);
            }
            _ => panic!("expected side-effect intent payload"),
        }

        let claim = payloads.side_effect_claimed(
            side_effect.clone(),
            RunnerClaimBinding {
                claim_owner: owner.clone(),
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            },
        );
        match &claim {
            RunnerEventPayload::SideEffectClaimed(payload) => {
                assert_eq!(payload.claim_owner, owner);
                assert_eq!(payload.claim_generation, 1);
                assert_eq!(payload.claim_fencing_token, token);
            }
            _ => panic!("expected side-effect claimed payload"),
        }

        let takeover = payloads.side_effect_claim_taken_over(
            side_effect.clone(),
            RunnerClaimTakeoverBinding {
                previous_claim_owner: owner.clone(),
                new_claim_owner: next_owner.clone(),
                previous_claim_generation: 1,
                claim_generation: 2,
                claim_fencing_token: next_token.clone(),
            },
        );
        match &takeover {
            RunnerEventPayload::SideEffectClaimTakenOver(payload) => {
                assert_eq!(payload.previous_claim_owner, owner);
                assert_eq!(payload.new_claim_owner, next_owner);
                assert_eq!(payload.previous_claim_generation, 1);
                assert_eq!(payload.claim_generation, 2);
                assert_eq!(payload.claim_fencing_token, next_token);
            }
            _ => panic!("expected side-effect claim takeover payload"),
        }

        let prepared = artifacts
            .prepared_invocation(&serde_json::json!({"prepared": true}))
            .expect("prepared invocation");
        assert_eq!(
            prepared.evidence().artifact_role,
            events::ArtifactRole::PreparedInvocation
        );
        assert!(prepared.evidence().schema_id.is_none());
        let prepared_payload = payloads
            .side_effect_invocation_prepared(
                side_effect.clone(),
                Some(&prepared),
                RunnerPreparedInvocationBinding {
                    claim_generation: 2,
                    claim_fencing_token: next_token.clone(),
                    resource_key: None,
                },
            )
            .expect("prepared payload");
        match &prepared_payload {
            RunnerEventPayload::SideEffectInvocationPrepared(payload) => {
                assert_eq!(
                    payload.prepared_artifact_id.as_ref(),
                    Some(&prepared.evidence().artifact_id)
                );
                assert_eq!(
                    payload.prepared_hash.as_ref(),
                    Some(&prepared.evidence().digest)
                );
                assert_eq!(payload.claim_generation, 2);
            }
            _ => panic!("expected side-effect invocation prepared payload"),
        }

        let started = payloads.side_effect_invocation_started(
            side_effect.clone(),
            RunnerClaimBinding {
                claim_owner: next_owner.clone(),
                claim_generation: 2,
                claim_fencing_token: next_token.clone(),
            },
        );
        match &started {
            RunnerEventPayload::SideEffectInvocationStarted(payload) => {
                assert_eq!(payload.claim_owner, next_owner);
                assert_eq!(payload.claim_generation, 2);
                assert_eq!(payload.claim_fencing_token, next_token);
            }
            _ => panic!("expected side-effect invocation started payload"),
        }

        let not_submitted = artifacts
            .not_submitted_proof(&value)
            .expect("not-submitted proof");
        let not_submitted_payload = payloads
            .side_effect_not_submitted_proven(side_effect.clone(), &not_submitted)
            .expect("not-submitted payload");
        match &not_submitted_payload {
            RunnerEventPayload::SideEffectNotSubmittedProven(payload) => {
                assert_eq!(payload.proof_hash, not_submitted.evidence().digest);
                assert_eq!(
                    payload.proof_artifact_id,
                    not_submitted.evidence().artifact_id
                );
            }
            _ => panic!("expected side-effect not-submitted payload"),
        }

        let submission = artifacts.submission(&value).expect("submission");
        let submission_payload = payloads
            .side_effect_submission_observed(side_effect.clone(), &submission)
            .expect("submission payload");
        match &submission_payload {
            RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
                assert_eq!(payload.submission_hash, submission.evidence().digest);
                assert_eq!(
                    payload.submission_artifact_id,
                    submission.evidence().artifact_id
                );
            }
            _ => panic!("expected side-effect submission payload"),
        }

        let submission_unknown = artifacts
            .submission_unknown(&value)
            .expect("submission unknown evidence");
        let submission_unknown_payload = payloads
            .side_effect_submission_unknown(side_effect.clone(), &submission_unknown)
            .expect("submission unknown payload");
        match &submission_unknown_payload {
            RunnerEventPayload::SideEffectSubmissionUnknown(payload) => {
                assert_eq!(payload.evidence_hash, submission_unknown.evidence().digest);
                assert_eq!(
                    payload.evidence_artifact_id,
                    submission_unknown.evidence().artifact_id
                );
            }
            _ => panic!("expected side-effect submission unknown payload"),
        }

        let receipt = artifacts.receipt(&value).expect("receipt");
        let receipt_payload = payloads
            .side_effect_receipt_observed(side_effect.clone(), &receipt, verifier.clone(), None)
            .expect("receipt payload");
        match &receipt_payload {
            RunnerEventPayload::SideEffectReceiptObserved(payload) => {
                assert_eq!(payload.receipt_hash, receipt.evidence().digest);
                assert_eq!(payload.receipt_artifact_id, receipt.evidence().artifact_id);
                assert_eq!(payload.replay_verifier_id, verifier);
            }
            _ => panic!("expected side-effect receipt payload"),
        }

        let confirmation = artifacts.confirmation(&value).expect("confirmation");
        let confirmation_payload = payloads
            .side_effect_confirmation_observed(
                side_effect.clone(),
                &confirmation,
                verifier.clone(),
                None,
            )
            .expect("confirmation payload");
        match &confirmation_payload {
            RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
                assert_eq!(payload.confirmation_hash, confirmation.evidence().digest);
                assert_eq!(
                    payload.confirmation_artifact_id,
                    confirmation.evidence().artifact_id
                );
                assert_eq!(payload.replay_verifier_id, verifier);
            }
            _ => panic!("expected side-effect confirmation payload"),
        }

        let ambiguity = artifacts
            .ambiguity_evidence(&value)
            .expect("ambiguity evidence");
        let ambiguity_payload = payloads
            .side_effect_ambiguous(
                side_effect.clone(),
                events::SideEffectPairRole::Verify,
                events::AmbiguityCode::new("runner_kit_test").expect("ambiguity code"),
                &ambiguity,
            )
            .expect("ambiguity payload");
        match &ambiguity_payload {
            RunnerEventPayload::SideEffectAmbiguous(payload) => {
                assert_eq!(payload.evidence_hash, ambiguity.evidence().digest);
                assert_eq!(
                    payload.evidence_artifact_id,
                    ambiguity.evidence().artifact_id
                );
            }
            _ => panic!("expected side-effect ambiguous payload"),
        }

        let failed = payloads.side_effect_failed(
            side_effect.clone(),
            events::SideEffectPairRole::Verify,
            events::side_effect::FailurePhase::BeforeInvocationStarted,
            true,
            events::MfmErrorInfo::new(
                events::ErrorCode::new("runner_kit_failure").expect("error code"),
                events::ErrorCategory::Runtime,
                true,
                "runner kit failure",
            )
            .expect("error info"),
        );
        match &failed {
            RunnerEventPayload::SideEffectFailed(payload) => {
                assert_eq!(payload.ledger_key, ledger_key);
                assert_eq!(
                    payload.failure_phase,
                    events::side_effect::FailurePhase::BeforeInvocationStarted
                );
                assert!(payload.retryable);
                assert_eq!(payload.error.safe_message, "runner kit failure");
            }
            _ => panic!("expected side-effect failed payload"),
        }

        let role_mismatch = payloads
            .cell_produced(&response)
            .expect_err("fact response cannot produce a cell");
        assert!(matches!(
            role_mismatch,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("fact_response")
                    && message.contains("state_output")
        ));

        let mut state_output = RunnerOutputBuilder::new(&ctx);
        state_output
            .stage_attempt_artifact(&state)
            .expect("stage state output")
            .retain_runtime_evidence(&state)
            .payload(cell.clone());
        let state_output = state_output.finish();
        assert_eq!(state_output.staged_artifacts.len(), 1);
        assert_eq!(state_output.staged_retention_refs.len(), 1);
        assert_eq!(state_output.payloads, vec![cell]);

        let mut side_effect_output = RunnerOutputBuilder::new(&ctx);
        side_effect_output
            .stage_side_effect_runtime_evidence(&intent, &side_effect)
            .expect("stage side-effect runtime evidence")
            .payload(intent_payload);
        let side_effect_output = side_effect_output.finish();
        assert_eq!(side_effect_output.staged_artifacts.len(), 1);
        assert_eq!(side_effect_output.staged_retention_refs.len(), 1);
        assert_eq!(side_effect_output.payloads.len(), 1);
    });
}

#[tokio::test]
async fn fact_query_evidence_prepares_private_artifact_reference_without_fact_record() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = started_fixture_store(&scheduler, &fixture).await;
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    let projections = store.projection_snapshot().clone();
    let run_stream = store.load_run_stream(&fixture.run_id);
    let committed =
        store::CommittedRunStream::from_events(fixture.run_id.clone(), run_stream.clone())
            .expect("committed stream");
    let view = RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("runtime view");
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let caps =
        CertifiedRuntimeCapabilities::new(node.node_id.clone(), node.capability_bindings.clone());
    let recorded_facts = RecordedFacts::default();
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id: &attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps,
        recorded_facts,
        projections: &projections,
        run_stream: &run_stream,
        view: &view,
    };
    let ctx = ErasedRunCtx::from_prepared(&invocation);
    let output_bytes = br#"{"amount":11}"#.to_vec();
    let state_evidence =
        state_output_artifact_for_bytes(ctx.node(), ctx.descriptor(), &output_bytes);
    let state_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, output_bytes, state_evidence.clone())
            .expect("stage state output");
    let mut query_output = RunnerOutputBuilder::new(&ctx);
    query_output
        .record_fact_query_evidence(test_fact_query_evidence(), &test_fact_query_trust_root())
        .expect("record query evidence");
    let query_output = query_output.finish();
    let mut staged_artifacts = vec![state_artifact];
    staged_artifacts.extend(query_output.staged_artifacts);
    let mut tampered_retention_refs = query_output.staged_retention_refs.clone();
    tampered_retention_refs[0].refs = vec![retention_ref_for_artifact(&state_evidence)];
    let tampered_output = fact_query_terminal_output(
        &ctx,
        &state_evidence,
        staged_artifacts.clone(),
        tampered_retention_refs,
    );
    let tampered_error = match prepare_runner_output_for_invocation(&invocation, tampered_output) {
        Ok(_) => panic!("missing query evidence retention authority rejects at commit prep"),
        Err(error) => error,
    };
    assert!(matches!(
        tampered_error,
        RuntimeError::InvalidRunnerOutput(message)
            if message.contains("missing query evidence artifact")
    ));

    let output = fact_query_terminal_output(
        &ctx,
        &state_evidence,
        staged_artifacts,
        query_output.staged_retention_refs,
    );
    let prepared =
        prepare_runner_output_for_invocation(&invocation, output).expect("prepare runner output");
    let query_reference = prepared
        .commit
        .request()
        .payloads()
        .iter()
        .find_map(|payload| match payload {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::FactQueryEvidence =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("fact query evidence artifact reference");
    assert_eq!(
        query_reference.artifact_ref.schema_id,
        mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema")
    );
    assert!(query_reference.artifact_ref.semantic_type_id.is_none());
    assert!(prepared
        .commit
        .request()
        .payloads()
        .iter()
        .any(|payload| matches!(
            payload,
            events::KernelEventPayload::RetentionRefsAppended(payload)
                if payload.reason == events::RetentionReason::RuntimeEvidence
                    && payload.refs.iter().any(|reference| {
                        reference.artifact_id == query_reference.artifact_ref.artifact_id
                            && reference.role == events::ArtifactRole::FactQueryEvidence
                            && reference.content_digest
                                == query_reference.artifact_ref.content_digest
                    })
        )));
    assert!(!prepared
        .commit
        .request()
        .payloads()
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::FactRecorded(_))));
}

#[tokio::test]
async fn fact_query_evidence_retains_non_empty_returned_fact_authority() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = started_fixture_store(&scheduler, &fixture).await;
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    let projections = store.projection_snapshot().clone();
    let (fact_ref, descriptor_projection, record_projection, index_projection, term_projections) =
        test_returned_fact_authority(&fixture, node);
    let projections = projection_snapshot_with_returned_fact_authority(
        &projections,
        descriptor_projection.clone(),
        record_projection,
        index_projection.clone(),
        term_projections,
    );
    let run_stream = store.load_run_stream(&fixture.run_id);
    let committed =
        store::CommittedRunStream::from_events(fixture.run_id.clone(), run_stream.clone())
            .expect("committed stream");
    let view = RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("runtime view");
    let view = RuntimeRunView {
        projections: projections.clone(),
        ..view
    };
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let caps =
        CertifiedRuntimeCapabilities::new(node.node_id.clone(), node.capability_bindings.clone());
    let recorded_facts = RecordedFacts::default();
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id: &attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps,
        recorded_facts,
        projections: &projections,
        run_stream: &run_stream,
        view: &view,
    };
    let ctx = ErasedRunCtx::from_prepared(&invocation);
    let output_bytes = br#"{"amount":11}"#.to_vec();
    let state_evidence =
        state_output_artifact_for_bytes(ctx.node(), ctx.descriptor(), &output_bytes);
    let state_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, output_bytes, state_evidence.clone())
            .expect("stage state output");
    let query_evidence = test_fact_query_evidence_with_returned_refs(vec![fact_ref.clone()]);
    let mut query_output = RunnerOutputBuilder::new(&ctx);
    query_output
        .record_fact_query_evidence(query_evidence, &test_fact_query_trust_root())
        .expect("record query evidence");
    let query_output = query_output.finish();
    let mut staged_artifacts = vec![state_artifact];
    staged_artifacts.extend(query_output.staged_artifacts);
    let mut tampered_retention_refs = query_output.staged_retention_refs.clone();
    tampered_retention_refs[0]
        .refs
        .retain(|reference| reference.role != events::ArtifactRole::FactDescriptor);
    let tampered_output = fact_query_terminal_output(
        &ctx,
        &state_evidence,
        staged_artifacts.clone(),
        tampered_retention_refs,
    );
    let tampered_error = match prepare_runner_output_for_invocation(&invocation, tampered_output) {
        Ok(_) => panic!("missing descriptor retention authority rejects at commit prep"),
        Err(error) => error,
    };
    assert!(matches!(
        tampered_error,
        RuntimeError::InvalidRunnerOutput(message)
            if message.contains("missing descriptor artifact authority")
    ));

    let output = fact_query_terminal_output(
        &ctx,
        &state_evidence,
        staged_artifacts,
        query_output.staged_retention_refs,
    );
    let prepared = prepare_runner_output_for_invocation(&invocation, output)
        .expect("prepare runner output with returned fact query refs");
    let query_reference = prepared
        .commit
        .request()
        .payloads()
        .iter()
        .find_map(|payload| match payload {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::FactQueryEvidence =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("fact query evidence artifact reference");
    let retained_refs = prepared
        .commit
        .request()
        .payloads()
        .iter()
        .find_map(|payload| match payload {
            events::KernelEventPayload::RetentionRefsAppended(payload)
                if payload.reason == events::RetentionReason::RuntimeEvidence =>
            {
                Some(payload.refs.as_slice())
            }
            _ => None,
        })
        .expect("runtime evidence retention refs");

    assert!(retained_refs.contains(&events::RetentionRef {
        artifact_id: query_reference.artifact_ref.artifact_id.clone(),
        role: events::ArtifactRole::FactQueryEvidence,
        content_digest: query_reference.artifact_ref.content_digest.clone(),
    }));
    assert!(retained_refs.contains(&events::RetentionRef {
        artifact_id: descriptor_projection.descriptor_artifact_id,
        role: events::ArtifactRole::FactDescriptor,
        content_digest: descriptor_projection.descriptor_hash,
    }));
    assert!(retained_refs.contains(&events::RetentionRef {
        artifact_id: index_projection.artifact_id,
        role: events::ArtifactRole::FactResponse,
        content_digest: index_projection.response_hash,
    }));
    assert_eq!(retained_refs.len(), 3);
    assert!(!prepared
        .commit
        .request()
        .payloads()
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::FactRecorded(_))));
}

#[test]
fn side_effect_evidence_builder_prepares_and_stages_claimed_invocation() {
    let fixture = fixture();
    let ledger_key = events::SideEffectLedgerKey::new("mfm.test.side_effect_builder.ledger")
        .expect("ledger key");
    let owner =
        events::RunnerInvocationId::new("mfm.test.side_effect_builder.owner").expect("claim owner");
    let token = events::side_effect::ClaimFencingToken::new("mfm.test.side_effect_builder.token")
        .expect("token");
    let idempotency_key = events::IdempotencyKeyRef::new("mfm.test.side_effect_builder.idem")
        .expect("idempotency key");
    let capability_binding = RunnerCapabilityBinding {
        capability_kind: fixture.cap_kind.clone(),
        capability_version: fixture.cap_version.clone(),
        adapter_kind: fixture.adapter_kind.clone(),
        adapter_version: fixture.adapter_version.clone(),
    };
    let intent = CertifierValue { amount: 7 };
    let idempotency = CertifierValue { amount: 11 };
    let prepared = serde_json::json!({"prepared": true});

    with_runner_erased_ctx(&fixture, &fixture.cell_a, |ctx| {
        let output = SideEffectEvidenceBuilder::new(&ctx)
            .prepare_invocation_and_start(SideEffectPreparedInvocationEvidence {
                side_effect: RunnerSideEffectBinding {
                    ledger_key: ledger_key.clone(),
                    ledger_purpose: events::SideEffectLedgerPurpose::Forward,
                    pair_id: synthetic_side_effect_pair_id(0x32),
                    invocation_epoch: 3,
                },
                claim: RuntimeSideEffectClaimAuthority {
                    claim_owner: owner.clone(),
                    claim_generation: 9,
                    claim_fencing_token: token.clone(),
                    resource_key: None,
                },
                intent: &intent,
                idempotency: &idempotency,
                idempotency_key: idempotency_key.clone(),
                capability_binding,
                prepared_invocation: &prepared,
            })
            .expect("prepare side-effect evidence");

        assert_eq!(output.staged_artifacts.len(), 2);
        assert_eq!(output.staged_retention_refs.len(), 2);
        assert_eq!(output.payloads.len(), 4);
        match &output.payloads[0] {
            RunnerEventPayload::SideEffectIntentPersisted(payload) => {
                assert_side_effect_binding!(payload, ledger_key, 3);
                assert_eq!(
                    payload.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                );
                assert_eq!(payload.idempotency_key, idempotency_key);
                assert_eq!(payload.capability_kind, fixture.cap_kind);
                assert_eq!(payload.adapter_kind, fixture.adapter_kind);
            }
            other => panic!("expected side-effect intent payload: {other:?}"),
        }
        match &output.payloads[1] {
            RunnerEventPayload::SideEffectClaimed(payload) => {
                assert_eq!(payload.claim_owner, owner);
                assert_eq!(payload.claim_generation, 9);
                assert_eq!(payload.claim_fencing_token, token);
            }
            other => panic!("expected side-effect claimed payload: {other:?}"),
        }
        match &output.payloads[2] {
            RunnerEventPayload::SideEffectInvocationPrepared(payload) => {
                assert!(payload.prepared_artifact_id.is_some());
                assert!(payload.prepared_hash.is_some());
                assert_eq!(payload.claim_generation, 9);
                assert_eq!(payload.claim_fencing_token, token);
            }
            other => panic!("expected side-effect prepared payload: {other:?}"),
        }
        match &output.payloads[3] {
            RunnerEventPayload::SideEffectInvocationStarted(payload) => {
                assert_eq!(payload.claim_owner, owner);
                assert_eq!(payload.claim_generation, 9);
                assert_eq!(payload.claim_fencing_token, token);
            }
            other => panic!("expected side-effect started payload: {other:?}"),
        }
    });
}

#[test]
fn side_effect_evidence_builder_builds_progress_evidence_with_replay_and_resources() {
    let fixture = fixture();
    let ledger_key = events::SideEffectLedgerKey::new("mfm.test.side_effect_builder.progress")
        .expect("ledger key");
    let verifier =
        events::ReplayVerifierId::new("mfm.test.side_effect_builder.verifier").expect("verifier");
    let touched_set = events::ResourceTouchedSetEvidence {
        namespace: exact_touched_set_resource_namespace(),
        evidence_schema_id: fixture.seed_ref.schema_id.clone(),
        evidence_hash: content(0xd1),
        evidence_artifact_id: artifact(0xd2),
    };
    let value = CertifierValue { amount: 17 };

    with_runner_erased_ctx(&fixture, &fixture.cell_a, |ctx| {
        let builder = SideEffectEvidenceBuilder::new(&ctx);
        let side_effect = RunnerSideEffectBinding {
            ledger_key: ledger_key.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: synthetic_side_effect_pair_id(0x33),
            invocation_epoch: 4,
        };

        match single_side_effect_payload(
            &builder
                .submission_observed(side_effect.clone(), None, &value)
                .expect("submission evidence"),
        ) {
            RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
                assert_side_effect_binding!(payload, ledger_key, 4);
            }
            other => panic!("expected submission observed payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .submission_unknown(side_effect.clone(), None, &value)
                .expect("submission unknown evidence"),
        ) {
            RunnerEventPayload::SideEffectSubmissionUnknown(payload) => {
                assert_side_effect_binding!(payload, ledger_key, 4);
            }
            other => panic!("expected submission unknown payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .not_submitted_proven(side_effect.clone(), None, &value)
                .expect("not-submitted evidence"),
        ) {
            RunnerEventPayload::SideEffectNotSubmittedProven(payload) => {
                assert_side_effect_binding!(payload, ledger_key, 4);
            }
            other => panic!("expected not-submitted payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .receipt_observed(
                    side_effect.clone(),
                    &value,
                    SideEffectReplayEvidence {
                        replay_verifier_id: verifier.clone(),
                        resource_touched_set: Some(touched_set.clone()),
                    },
                )
                .expect("receipt evidence"),
        ) {
            RunnerEventPayload::SideEffectReceiptObserved(payload) => {
                assert_side_effect_binding!(payload, ledger_key, 4);
                assert_eq!(payload.replay_verifier_id, verifier);
                assert_eq!(payload.resource_touched_set.as_ref(), Some(&touched_set));
            }
            other => panic!("expected receipt payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .confirmation_observed(
                    side_effect.clone(),
                    &value,
                    SideEffectReplayEvidence {
                        replay_verifier_id: verifier.clone(),
                        resource_touched_set: Some(touched_set.clone()),
                    },
                )
                .expect("confirmation evidence"),
        ) {
            RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
                assert_side_effect_binding!(payload, ledger_key, 4);
                assert_eq!(payload.replay_verifier_id, verifier);
                assert_eq!(payload.resource_touched_set.as_ref(), Some(&touched_set));
            }
            other => panic!("expected confirmation payload: {other:?}"),
        }
        match single_side_effect_payload(
            &builder
                .ambiguous(
                    side_effect,
                    events::SideEffectPairRole::Verify,
                    None,
                    events::AmbiguityCode::new("side_effect_builder_test").expect("ambiguity code"),
                    &value,
                )
                .expect("ambiguity evidence"),
        ) {
            RunnerEventPayload::SideEffectAmbiguous(payload) => {
                assert_side_effect_binding!(payload, ledger_key, 4);
            }
            other => panic!("expected ambiguity payload: {other:?}"),
        }
    });
}

#[derive(Clone)]
enum TestSubmissionDecision {
    Observed,
    Unknown,
    NotSubmitted,
    Ambiguous,
}

#[derive(Clone)]
struct TestSideEffectDriverCallbacks {
    cap_kind: CapabilityKind,
    cap_version: CapabilityVersion,
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    submission_decision: TestSubmissionDecision,
}

impl TestSideEffectDriverCallbacks {
    fn new(fixture: &Fixture) -> Self {
        Self {
            cap_kind: side_effect_capability_kind(),
            cap_version: side_effect_capability_version(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
            submission_decision: TestSubmissionDecision::Observed,
        }
    }

    fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.submission_decision = decision;
        self
    }

    fn intent_plan_for(
        &self,
        node_id: String,
        attempt_id: String,
    ) -> Result<SideEffectIntentPlan<FixtureSideEffectEvidence, FixtureSideEffectEvidence>> {
        Ok(SideEffectIntentPlan {
            intent: fixture_side_effect_evidence(21, node_id.clone(), attempt_id.clone()),
            idempotency: fixture_side_effect_evidence(34, node_id, attempt_id),
            idempotency_key: events::IdempotencyKeyRef::new("mfm.test.driver.idem")
                .expect("idempotency key"),
            capability_binding: RunnerCapabilityBinding {
                capability_kind: self.cap_kind.clone(),
                capability_version: self.cap_version.clone(),
                adapter_kind: self.adapter_kind.clone(),
                adapter_version: self.adapter_version.clone(),
            },
        })
    }
}

impl SideEffectDriverCallbacks for TestSideEffectDriverCallbacks {
    type Intent = FixtureSideEffectEvidence;
    type Idempotency = FixtureSideEffectEvidence;
    type PreparedInvocation = serde_json::Value;
    type Submission = FixtureSideEffectEvidence;
    type SubmissionUnknownEvidence = FixtureSideEffectEvidence;
    type NotSubmittedProof = FixtureSideEffectEvidence;
    type AmbiguityEvidence = FixtureSideEffectEvidence;

    fn intent_and_idempotency<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
    ) -> SideEffectDriverFuture<'a, SideEffectIntentPlan<Self::Intent, Self::Idempotency>> {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move { self.intent_plan_for(node_id, attempt_id) })
    }

    fn prepare_invocation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _plan: &'a SideEffectIntentPlan<Self::Intent, Self::Idempotency>,
    ) -> SideEffectDriverFuture<'a, SideEffectPreparedInvocationPlan<Self::PreparedInvocation>>
    {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(SideEffectPreparedInvocationPlan::with_prepared_invocation(
                serde_json::json!({
                    "attempt_id": attempt_id,
                    "node_id": node_id,
                    "prepared": true
                }),
            ))
        })
    }

    fn reconstruct_prepared_invocation<'a, 'ctx>(
        &'a self,
        _ctx: &'a ErasedRunCtx<'ctx>,
        prepared: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::PreparedInvocation> {
        let prepared_artifact_id = prepared.artifact_id.clone();
        Box::pin(async move {
            Ok(serde_json::json!({
                "prepared_artifact_id": prepared_artifact_id.as_str()
            }))
        })
    }

    fn submit_or_recover_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _action: SideEffectProtocolAction,
        _prepared: Option<Self::PreparedInvocation>,
    ) -> SideEffectSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::SubmissionUnknownEvidence,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    > {
        let decision = self.submission_decision.clone();
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(match decision {
                TestSubmissionDecision::Observed => SideEffectSubmissionDecision::Observed(
                    fixture_side_effect_evidence(55, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::Unknown => SideEffectSubmissionDecision::Unknown(
                    fixture_side_effect_evidence(56, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::NotSubmitted => SideEffectSubmissionDecision::NotSubmitted(
                    fixture_side_effect_evidence(57, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::Ambiguous => SideEffectSubmissionDecision::Ambiguous {
                    ambiguity_code: events::AmbiguityCode::new("mfm_test_driver_ambiguous")
                        .expect("ambiguity code"),
                    evidence: fixture_side_effect_evidence(58, node_id, attempt_id),
                },
            })
        })
    }
}

impl SideEffectVerifyCallbacks for TestSideEffectDriverCallbacks {
    type Submission = FixtureSideEffectEvidence;
    type Receipt = FixtureSideEffectEvidence;
    type Confirmation = FixtureSideEffectEvidence;
    type Output = FixtureOutputValue;
    type NotSubmittedProof = FixtureSideEffectEvidence;
    type AmbiguityEvidence = FixtureSideEffectEvidence;

    fn recover_unknown_submission<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _prepared_invocation: Option<&'a store::SideEffectArtifactProjection>,
    ) -> SideEffectUnknownSubmissionDecisionFuture<
        'a,
        Self::Submission,
        Self::NotSubmittedProof,
        Self::AmbiguityEvidence,
    > {
        let decision = self.submission_decision.clone();
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(match decision {
                TestSubmissionDecision::Observed => SideEffectUnknownSubmissionDecision::Observed(
                    fixture_side_effect_evidence(55, node_id.clone(), attempt_id.clone()),
                ),
                TestSubmissionDecision::Unknown => {
                    SideEffectUnknownSubmissionDecision::StillUnknown
                }
                TestSubmissionDecision::NotSubmitted => {
                    SideEffectUnknownSubmissionDecision::NotSubmitted(fixture_side_effect_evidence(
                        57,
                        node_id.clone(),
                        attempt_id.clone(),
                    ))
                }
                TestSubmissionDecision::Ambiguous => {
                    SideEffectUnknownSubmissionDecision::Ambiguous {
                        ambiguity_code: events::AmbiguityCode::new("mfm_test_driver_ambiguous")
                            .expect("ambiguity code"),
                        evidence: fixture_side_effect_evidence(58, node_id, attempt_id),
                    }
                }
            })
        })
    }

    fn read_receipt<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _submission: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Receipt>> {
        let evidence = fixture_side_effect_evidence_for_ctx(ctx, 89);
        Box::pin(async move {
            Ok(SideEffectObservedEvidence {
                evidence,
                replay: test_driver_replay_evidence(),
            })
        })
    }

    fn build_confirmation<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, SideEffectObservedEvidence<Self::Confirmation>> {
        let evidence = fixture_side_effect_evidence_for_ctx(ctx, 144);
        Box::pin(async move {
            Ok(SideEffectObservedEvidence {
                evidence,
                replay: test_driver_replay_evidence(),
            })
        })
    }

    fn map_receipt_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _receipt: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let output =
            fixture_output_value(233, ctx.node().node_id.as_str(), ctx.attempt_id().as_str());
        Box::pin(async move { Ok(output) })
    }

    fn map_confirmation_to_output<'a, 'ctx>(
        &'a self,
        ctx: &'a ErasedRunCtx<'ctx>,
        _submit_node: &'a spec::NodeSpec,
        _submit_inputs: &'a MaterializedInputs,
        _confirmation: &'a store::SideEffectArtifactProjection,
    ) -> SideEffectDriverFuture<'a, Self::Output> {
        let output =
            fixture_output_value(233, ctx.node().node_id.as_str(), ctx.attempt_id().as_str());
        Box::pin(async move { Ok(output) })
    }
}

fn test_driver_replay_evidence() -> SideEffectReplayEvidence {
    SideEffectReplayEvidence {
        replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.verifier")
            .expect("verifier"),
        resource_touched_set: None,
    }
}

fn test_driver_resource_key_for_node(node: &spec::NodeSpec) -> Option<events::ResourceKeyEvidence> {
    let Some(spec::SideEffectContractSpec {
        resource_claim:
            spec::ResourceClaimSpec::Exclusive {
                namespace,
                key_schema,
            },
        ..
    }) = &node.side_effect
    else {
        return None;
    };
    Some(events::ResourceKeyEvidence {
        namespace: namespace.clone(),
        key_schema_id: key_schema.clone(),
        key: events::ResourceKey::new("mfm.test.driver.shared-resource").expect("resource key"),
    })
}

#[tokio::test]
async fn side_effect_driver_prepares_and_starts_one_step() {
    let fixture = fixture_with_first_side_effect_state();
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output = drive_side_effect_driver_empty(&fixture, &fixture.cell_a, &callbacks)
        .await
        .expect("driver output");

    assert_eq!(output.staged_artifacts.len(), 2);
    assert_eq!(output.payloads.len(), 4);
    assert!(matches!(
        output.payloads[0],
        RunnerEventPayload::SideEffectIntentPersisted(_)
    ));
    assert!(matches!(
        output.payloads[1],
        RunnerEventPayload::SideEffectClaimed(_)
    ));
    assert!(matches!(
        output.payloads[2],
        RunnerEventPayload::SideEffectInvocationPrepared(_)
    ));
    assert!(matches!(
        output.payloads[3],
        RunnerEventPayload::SideEffectInvocationStarted(_)
    ));
    match &output.payloads[0] {
        RunnerEventPayload::SideEffectIntentPersisted(payload) => {
            assert_eq!(
                payload.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            );
            assert_eq!(payload.invocation_epoch, 1);
        }
        other => panic!("expected intent payload: {other:?}"),
    }
}

#[tokio::test]
async fn side_effect_driver_preserves_concrete_exclusive_resource_key_across_runs() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let mut peer = fixture.clone();
    refresh_fixture_run_id_with_distinct(&mut peer, Some(content(0xf6)));
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();

    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start first run");
    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "first run claims resource lane"
    );
    let first_stream = store.load_run_stream(&fixture.run_id);
    let claim_seq = first_stream
        .iter()
        .find_map(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ResourceLaneClaimed(_)
            )
            .then_some(event.seq())
        })
        .expect("first run recorded resource lane claim");
    let prepared_seq = first_stream.iter().find_map(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectInvocationPrepared(_)
        )
        .then_some(event.seq())
    });
    assert!(
        prepared_seq.is_none_or(|prepared_seq| claim_seq < prepared_seq),
        "exclusive invocation prepare must be committed after ResourceLaneClaimed"
    );
    let resource_key = first_stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ResourceLaneClaimed(payload) => {
                Some(payload.resource_key.clone())
            }
            _ => None,
        })
        .expect("first run recorded resource key");
    assert_eq!(resource_key.key.as_str(), "mfm.test.driver.shared-resource");
    let lane_key = store::ResourceLaneKey::from_evidence(&resource_key);
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());

    start_fixture_run(&scheduler, &mut store, &peer, vec![peer.seed_ref.clone()])
        .await
        .expect("start peer run");
    assert_eq!(
        drive_once(&scheduler, &mut store, &peer.runtime_spec, &peer.run_id)
            .await
            .expect("peer run starts attempt before observing lane block"),
        SchedulerStatus::Advanced
    );
    assert_eq!(
        drive_once(&scheduler, &mut store, &peer.runtime_spec, &peer.run_id)
            .await
            .expect("peer run blocks on same resource lane"),
        SchedulerStatus::Blocked
    );
    assert!(
        store
            .load_run_stream(&peer.run_id)
            .iter()
            .all(|event| !matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectInvocationPrepared(_)
            )),
        "peer run must not prepare while the cross-run resource lane is held"
    );
}

#[tokio::test]
async fn exclusive_side_effect_prepare_failure_after_claim_terminalizes_attempt() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        FailingAfterPreclaimRunner::new(&fixture),
    ));
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "exclusive run terminalizes failed resource-lane claim"
    );
    let node = node_by_output(&fixture, &fixture.cell_a);
    let stream = store.load_run_stream(&fixture.run_id);
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ResourceLaneClaimed(_)
        )),
        "failed exclusive attempt records the resource lane claim"
    );
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ResourceLaneReleased(_)
        )),
        "failed exclusive attempt releases the resource lane"
    );
    assert!(
        stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectFailed(_)
        )),
        "failed exclusive attempt records terminal side-effect evidence"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
    assert!(
        store
            .projection_snapshot()
            .resource_lanes()
            .next()
            .is_none(),
        "terminal attempt failure releases the exclusive resource lane"
    );
}

#[tokio::test]
async fn runner_block_leaves_started_attempt_open_without_failure() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let mut registry = fixture_registry_with_first_runner(&fixture, "pure", BlockingRunner);
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    let scheduler = test_scheduler(registry);
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    assert_drive!(
        scheduler,
        store,
        fixture,
        Blocked,
        "blocked runner leaves attempt open"
    );
    let stream = store.load_run_stream(&fixture.run_id);
    assert!(stream.iter().any(|event| matches!(
        event.payload(),
        events::KernelEventPayload::StateAttemptStarted(payload)
            if payload.node_id == node.node_id
    )));
    assert!(stream.iter().all(|event| !matches!(
        event.payload(),
        events::KernelEventPayload::StateAttemptFailed(payload)
            if payload.node_id == node.node_id
    )));
}

#[tokio::test]
async fn side_effect_driver_submits_from_started_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-submit",
        "sidefx-driver-submit",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert_eq!(output.payloads.len(), 1);
    match &output.payloads[0] {
        RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
            assert_side_effect_binding!(payload, ledger_key, 1);
        }
        other => panic!("expected submission payload: {other:?}"),
    }
}

#[tokio::test]
async fn side_effect_driver_persists_submission_recovery_decisions() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, _ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-recovery",
        "sidefx-driver-recovery",
    );

    for (decision, expected) in [
        (
            TestSubmissionDecision::Unknown,
            "side_effect.submission_unknown",
        ),
        (
            TestSubmissionDecision::NotSubmitted,
            "side_effect.not_submitted_proven",
        ),
        (TestSubmissionDecision::Ambiguous, "side_effect.ambiguous"),
    ] {
        let callbacks =
            TestSideEffectDriverCallbacks::new(&fixture).with_submission_decision(decision);
        let output =
            drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
                .await
                .expect("driver output");
        let actual = output
            .payloads
            .iter()
            .find_map(|payload| match payload {
                RunnerEventPayload::SideEffectSubmissionUnknown(_) => {
                    Some("side_effect.submission_unknown")
                }
                RunnerEventPayload::SideEffectNotSubmittedProven(_) => {
                    Some("side_effect.not_submitted_proven")
                }
                RunnerEventPayload::SideEffectAmbiguous(_) => Some("side_effect.ambiguous"),
                RunnerEventPayload::ResourceLaneReleaseIntent(_) => None,
                _ => None,
            })
            .unwrap_or_else(|| panic!("unexpected recovery payloads: {:?}", output.payloads));
        assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn side_effect_submission_unknown_keeps_exclusive_resource_lane_held() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let pair_id = fixture_side_effect_pair_id(&fixture, node);
    let (attempt_id, _ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-unknown-lane",
        "sidefx-driver-unknown-lane",
    );
    let before_unknown = store.projection_snapshot();
    let (lane_key, held_lane) =
        active_resource_lane_for_pair(&before_unknown, &fixture.run_id, &pair_id)
            .expect("exclusive lane must be held before submission recovery");
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture)
        .with_submission_decision(TestSubmissionDecision::Unknown);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");
    assert!(output
        .payloads
        .iter()
        .any(|payload| matches!(payload, RunnerEventPayload::SideEffectSubmissionUnknown(_))));
    assert!(output
        .payloads
        .iter()
        .all(|payload| !matches!(payload, RunnerEventPayload::ResourceLaneReleaseIntent(_))));
    append_erased_runner_output(
        &mut store,
        &fixture.run_id,
        "sidefx-driver-unknown-lane-output",
        output,
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("attempt logical key")],
            required_side_effect_states: vec![store::SideEffectStatePrecondition {
                pair_id: pair_id.clone(),
                required: store::RequiredSideEffectState::InvocationStarted,
            }],
            certified_run_authority: Some(
                store::CertifiedRunStoreAuthority::from_spec(
                    fixture.run_id.clone(),
                    fixture.runtime_spec.spec(),
                )
                .expect("certified run authority"),
            ),
            ..store::CommitPreconditions::default()
        },
    );

    let after_unknown = store.projection_snapshot();
    let projection = after_unknown
        .side_effect_for_pair(&fixture.run_id, &pair_id)
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::SubmissionUnknown { .. }
    ));
    let lane_after_unknown = after_unknown
        .resource_lane(&lane_key)
        .expect("submission-unknown phase must keep the resource lane held");
    assert_eq!(lane_after_unknown.holder, held_lane.holder);
    assert_eq!(lane_after_unknown.claim_id, held_lane.claim_id);
}

#[tokio::test]
async fn side_effect_verify_driver_maps_receipt_to_state_output() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (submit_attempt_id, ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-output",
        "sidefx-driver-output",
    );
    append_synthetic_submission_observed(
        &mut store,
        &fixture,
        node,
        &submit_attempt_id,
        &ledger_key,
        "sidefx-driver-output-submission",
    );
    append_synthetic_receipt_observed(
        &mut store,
        &fixture,
        node,
        &submit_attempt_id,
        &ledger_key,
        "sidefx-driver-output-receipt",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);
    let verify_node = side_effect_verify_node_for_submit(&fixture, node);
    let verify_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &verify_node.node_id,
        1,
    )
    .expect("verify attempt id");

    let output = drive_side_effect_verify_driver_from_store(
        &fixture,
        &store,
        verify_node,
        &verify_attempt_id,
        &callbacks,
    )
    .await
    .expect("driver output");

    assert_eq!(output.staged_artifacts.len(), 1);
    assert_eq!(output.payloads.len(), 1);
    assert!(matches!(
        output.payloads[0],
        RunnerEventPayload::CellProduced(_)
    ));
}

#[tokio::test]
async fn side_effect_receipt_verification_releases_exclusive_resource_lane() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;
    let submit_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let verify_node = side_effect_verify_node_for_submit(&fixture, &submit_node).clone();
    let pair_id = fixture_side_effect_pair_id(&fixture, &submit_node);
    let mut saw_lane_claimed = false;
    let mut released_with_receipt_before_output = false;
    let mut terminal_output_after_release = false;

    for _ in 0..10 {
        assert_ne!(
            drive_fixture_once(&scheduler, &mut store, &fixture)
                .await
                .expect("advance receipt verification"),
            SchedulerStatus::Blocked
        );
        let snapshot = store.projection_snapshot();
        let active_lane = active_resource_lane_for_pair(&snapshot, &fixture.run_id, &pair_id);
        saw_lane_claimed |= active_lane.is_some();
        let side_effect = snapshot
            .side_effect_for_pair(&fixture.run_id, &pair_id)
            .expect("side-effect projection");
        if matches!(
            side_effect.phase,
            store::SideEffectPhase::ReceiptObserved { .. }
        ) && active_lane.is_none()
            && snapshot.cell_terminal(&verify_node.output_cell).is_none()
        {
            released_with_receipt_before_output = true;
        }
        if released_with_receipt_before_output
            && active_lane.is_none()
            && snapshot.cell_terminal(&verify_node.output_cell).is_some()
        {
            terminal_output_after_release = true;
            break;
        }
    }

    assert!(saw_lane_claimed, "exclusive side effect must claim a lane");
    assert!(
        released_with_receipt_before_output,
        "receipt evidence should release the lane before terminal output"
    );
    assert!(
        terminal_output_after_release,
        "receipt-level terminal output should be produced after lane release"
    );
}

#[tokio::test]
async fn side_effect_driver_rejects_ambiguous_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-ambiguous",
        "sidefx-driver-ambiguous",
    );
    append_synthetic_ambiguous(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        &attempt_id,
        &ledger_key,
        "sidefx-driver-ambiguous-terminal",
    );
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    assert!(matches!(
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks).await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("unsupported side-effect driver phase: ambiguous")
    ));
}

#[tokio::test]
async fn side_effect_driver_starts_and_submits_from_prepared_projection() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-driver-unsupported",
        "sidefx-driver-unsupported",
    );
    let pair_id = fixture_side_effect_pair_id(&fixture, node);
    let callbacks = TestSideEffectDriverCallbacks::new(&fixture);

    let output =
        drive_side_effect_driver_from_store(&fixture, &store, node, &attempt_id, &callbacks)
            .await
            .expect("driver output");

    assert_eq!(output.staged_artifacts.len(), 1);
    assert_eq!(output.payloads.len(), 2);
    match &output.payloads[0] {
        RunnerEventPayload::SideEffectInvocationStarted(payload) => {
            assert_side_effect_binding!(payload, ledger_key, 1);
        }
        other => panic!("expected invocation started payload: {other:?}"),
    }
    match &output.payloads[1] {
        RunnerEventPayload::SideEffectSubmissionObserved(payload) => {
            assert_side_effect_binding!(payload, ledger_key, 1);
        }
        other => panic!("expected submission payload: {other:?}"),
    }

    let required_artifacts = output
        .staged_artifacts
        .iter()
        .map(|artifact| artifact.evidence().clone())
        .collect::<Vec<_>>();
    let payloads = output
        .payloads
        .into_iter()
        .map(events::KernelEventPayload::from)
        .collect::<Vec<_>>();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("sidefx-driver-prepared-resume")
                .expect("commit key"),
            payloads: payloads,
            required_artifacts: required_artifacts,
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    pair_id: pair_id.clone(),
                    required: store::RequiredSideEffectState::InvocationPrepared,
                }],
                certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                    fixture.run_id.clone(),
                    fixture.runtime_spec.spec(),
                )
                .expect("certified run authority")),
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append prepared recovery output");

    let projection_snapshot = store.projection_snapshot();
    let projection = projection_snapshot
        .side_effect_state_for_pair(&fixture.run_id, &pair_id)
        .expect("side-effect state")
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase(),
        store::SideEffectLedgerPhase::SubmissionKnown {
            status: store::SideEffectSubmissionState::Observed { .. },
            ..
        }
    ));
}

macro_rules! with_prepared_runner_ctx {
    (
        $fixture:expr,
        $node:expr,
        $attempt_id:expr,
        $projections:expr,
        $run_stream:expr,
        |$ctx:ident| $body:block $(,)?
    ) => {{
        let invocation_node = $node;
        let descriptor = $fixture
            .runtime_spec
            .state_descriptor_for_node(invocation_node)
            .expect("state descriptor");
        let output_cell = $fixture
            .runtime_spec
            .cell(&invocation_node.output_cell)
            .expect("output cell");
        let config_artifact =
            config_artifact(&$fixture.runtime_spec, &invocation_node.config_ref).evidence;
        let projections = $projections;
        let run_stream = $run_stream;
        let committed =
            store::CommittedRunStream::from_events($fixture.run_id.clone(), run_stream.clone())
                .expect("prepared runner committed stream");
        let view = RuntimeRunView::from_committed_stream(&$fixture.runtime_spec, &committed)
            .expect("prepared runner view");
        let invocation = PreparedRunnerInvocation {
            runtime_spec: &$fixture.runtime_spec,
            run_id: &$fixture.run_id,
            spec_hash: $fixture.runtime_spec.spec_hash(),
            node: invocation_node,
            descriptor,
            output_cell,
            attempt_id: $attempt_id,
            attempt_no: 1,
            config_artifact,
            inputs: MaterializedInputs {
                input_schema_id: invocation_node.input_bindings.input_schema_id.clone(),
                root: MaterializedInputNode::Unit,
            },
            caps: CertifiedRuntimeCapabilities::new(
                invocation_node.node_id.clone(),
                invocation_node.capability_bindings.clone(),
            ),
            recorded_facts: RecordedFacts::default(),
            projections: &projections,
            run_stream: &run_stream,
            view: &view,
        };
        let $ctx = ErasedRunCtx::from_prepared(&invocation);
        $body
    }};
}

fn started_fixture_projection_and_stream(
    fixture: &Fixture,
) -> (store::ProjectionSnapshot, Vec<store::KernelEventEnvelope>) {
    let has_side_effect_nodes = fixture.runtime_spec.spec().nodes.iter().any(|node| {
        node.side_effect.is_some()
            || matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
            )
    });
    let registry = if has_side_effect_nodes {
        registered_side_effect_fixture_runners(fixture)
    } else {
        registered_fixture_runners(fixture)
    };
    let scheduler = test_scheduler(registry);
    let mut store = TestTypedRunStore::new();
    block_on_ready(start_fixture_run(
        &scheduler,
        &mut store,
        fixture,
        vec![fixture.seed_ref.clone()],
    ))
    .expect("start fixture for prepared runner context");
    (
        store.projection_snapshot().clone(),
        store.load_run_stream(&fixture.run_id),
    )
}

fn with_runner_erased_ctx<R, F>(fixture: &Fixture, cell_id: &CellId, test: F) -> R
where
    F: for<'a> FnOnce(ErasedRunCtx<'a>) -> R,
{
    let node = node_by_output(fixture, cell_id);
    with_runner_erased_ctx_for_node(fixture, node, test)
}

fn with_runner_erased_ctx_for_node<R, F>(fixture: &Fixture, node: &spec::NodeSpec, test: F) -> R
where
    F: for<'a> FnOnce(ErasedRunCtx<'a>) -> R,
{
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let (projections, run_stream) = started_fixture_projection_and_stream(fixture);
    with_prepared_runner_ctx!(fixture, node, &attempt_id, projections, run_stream, |ctx| {
        test(ctx)
    },)
}

async fn drive_side_effect_driver_empty<C>(
    fixture: &Fixture,
    cell_id: &CellId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectDriverCallbacks + ?Sized,
{
    let node = node_by_output(fixture, cell_id);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let (projections, run_stream) = started_fixture_projection_and_stream(fixture);
    with_prepared_runner_ctx!(fixture, node, &attempt_id, projections, run_stream, |ctx| {
        SideEffectDriver::drive(ctx, callbacks).await
    },)
}

async fn drive_side_effect_driver_from_store<C>(
    fixture: &Fixture,
    store: &TestTypedRunStore,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectDriverCallbacks + ?Sized,
{
    with_prepared_runner_ctx!(
        fixture,
        node,
        attempt_id,
        store.projection_snapshot().clone(),
        store.load_run_stream(&fixture.run_id),
        |ctx| { SideEffectDriver::drive(ctx, callbacks).await },
    )
}

async fn drive_side_effect_verify_driver_from_store<C>(
    fixture: &Fixture,
    store: &TestTypedRunStore,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectVerifyCallbacks + ?Sized,
{
    with_prepared_runner_ctx!(
        fixture,
        node,
        attempt_id,
        store.projection_snapshot().clone(),
        store.load_run_stream(&fixture.run_id),
        |ctx| { SideEffectVerifyDriver::drive(ctx, callbacks).await },
    )
}

fn single_side_effect_payload(output: &ErasedRunnerOutput) -> &RunnerEventPayload {
    assert_eq!(output.staged_artifacts.len(), 1);
    assert_eq!(output.staged_retention_refs.len(), 1);
    assert_eq!(output.payloads.len(), 1);
    &output.payloads[0]
}

#[test]
fn runner_registration_builder_preserves_explicit_binding_authority() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_b);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let factory_id = events::RunnerFactoryId::new("read").expect("factory");
    let executable = events::ExecutableIdentity {
        factory_id: factory_id.clone(),
        cargo_package_digest: content(0xe1),
        binary_digest: content(0xe2),
        nix_derivation_hash: None,
        nix_output_hash: None,
    };
    let implementation_id = CapabilityImplementationId::new("mfm.test.runner-kit-registration")
        .expect("implementation id");
    let mut registry = ErasedRunnerRegistry::new();

    RunnerRegistrationBuilder::new(&mut registry, implementation_id.clone())
        .register_descriptor(
            node.descriptor_id.clone(),
            &node.capability_bindings,
            factory_id.clone(),
            executable.clone(),
            Arc::new(RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            }),
        )
        .expect("runner registration");

    let binding = registry
        .resolve(&fixture.runtime_spec, node, descriptor)
        .expect("registered runner");
    assert_eq!(binding.factory_id(), &factory_id);
    assert_eq!(binding.executable(), &executable);
    let capabilities = registry
        .resolve_capability_implementations(node)
        .expect("capability implementations");
    assert_eq!(
        capabilities.len(),
        node.capability_bindings.capabilities.len()
    );
    for binding in capabilities {
        assert_eq!(binding.implementation_id(), &implementation_id);
    }

    let typed_fixture = fixture_with_first_side_effect_state();
    let typed_node = node_by_output(&typed_fixture, &typed_fixture.cell_b);
    let typed_descriptor = typed_fixture
        .runtime_spec
        .state_descriptor_for_node(typed_node)
        .expect("typed state descriptor");
    let expected_descriptor =
        mfm_program::registered_state_descriptor::<RuntimeReadState>().expect("typed descriptor");
    let typed_factory = events::RunnerFactoryId::new("read_external").expect("typed factory");
    let typed_executable = events::ExecutableIdentity {
        factory_id: typed_factory.clone(),
        cargo_package_digest: content(0xe3),
        binary_digest: content(0xe4),
        nix_derivation_hash: None,
        nix_output_hash: None,
    };
    let typed_implementation_id =
        CapabilityImplementationId::new("mfm.test.runner-kit-typed-registration")
            .expect("typed implementation id");
    let mut typed_registry = ErasedRunnerRegistry::new();
    let registered_descriptor =
        RunnerRegistrationBuilder::new(&mut typed_registry, typed_implementation_id.clone())
            .register_state_descriptor::<RuntimeReadState>(
                typed_factory.clone(),
                typed_executable.clone(),
                Arc::new(RecordingRunner {
                    expected_caps: vec![(
                        typed_fixture.cap_kind.clone(),
                        typed_fixture.cap_version.clone(),
                    )],
                    output_artifact: artifact(0xd1),
                    output_digest: content(0xd2),
                }),
            )
            .expect("typed runner registration");
    assert_eq!(
        registered_descriptor.descriptor_id(),
        expected_descriptor.descriptor_id()
    );
    assert_eq!(
        &typed_descriptor.descriptor_id,
        expected_descriptor.descriptor_id()
    );
    let typed_binding = typed_registry
        .resolve(&typed_fixture.runtime_spec, typed_node, typed_descriptor)
        .expect("registered typed runner");
    assert_eq!(typed_binding.factory_id(), &typed_factory);
    assert_eq!(typed_binding.executable(), &typed_executable);
    let typed_capabilities = typed_registry
        .resolve_capability_implementations(typed_node)
        .expect("typed capability implementations");
    assert_eq!(
        typed_capabilities.len(),
        typed_node.capability_bindings.capabilities.len()
    );
    for binding in typed_capabilities {
        assert_eq!(binding.implementation_id(), &typed_implementation_id);
    }

    let wrong_factory = events::RunnerFactoryId::new("pure").expect("factory");
    let mut mismatch_registry = ErasedRunnerRegistry::new();
    let error = match RunnerRegistrationBuilder::new(
        &mut mismatch_registry,
        CapabilityImplementationId::new("mfm.test.runner-kit-registration-mismatch")
            .expect("implementation id"),
    )
    .register_descriptor(
        node.descriptor_id.clone(),
        &node.capability_bindings,
        factory_id,
        events::ExecutableIdentity {
            factory_id: wrong_factory,
            ..executable
        },
        Arc::new(RecordingRunner {
            expected_caps: Vec::new(),
            output_artifact: artifact(0xc1),
            output_digest: content(0xc2),
        }),
    ) {
        Ok(_) => panic!("factory mismatch should be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        RuntimeError::RunnerBinding(message)
            if message.contains("does not match binding factory")
    ));
}

#[tokio::test]
async fn serial_scheduler_runs_nodes_in_certified_topological_order() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    assert_drive!(scheduler, store, fixture, Advanced, "drive a");
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_none());
    assert_drive!(scheduler, store, fixture, Advanced, "drive b");
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_some());
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=2 failed=0 interrupted=0 total=2] cells=2 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn scheduler_completes_run_after_public_output_evidence() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    assert_eq!(
        drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
            .await
            .expect("drive to public output"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.render_cell)
        .is_some());
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Completed attempts[started=0 completed=5 failed=0 interrupted=0 total=5] cells=5 side_effects=0 lanes[run=0 total=0] public_outputs=1 retentions=1"
    );

    let stream = store.load_run_stream(&fixture.run_id);
    let public_output_pos = stream
        .iter()
        .position(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        })
        .expect("public output produced");
    let completed_pos = stream
        .iter()
        .position(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
        .expect("run completed");
    assert!(public_output_pos < completed_pos);

    let public_event = &stream[public_output_pos];
    let public_payload = match public_event.payload() {
        events::KernelEventPayload::PublicOutputProduced(payload) => payload,
        _ => unreachable!("checked above"),
    };
    assert_eq!(public_payload.node_id, fixture.render_node);
    assert_eq!(public_payload.receipt_cell_id, fixture.render_cell);
    assert!(public_payload.rendered_artifact_id.is_none());

    let completed_payload = match stream[completed_pos].payload() {
        events::KernelEventPayload::RunCompleted(payload) => payload,
        _ => unreachable!("checked above"),
    };
    assert_eq!(
        completed_payload.outcome,
        events::RunCompletionOutcome::Completed(Box::new(events::PublicOutputCompletionEvidence {
            public_output_schema_id: fixture
                .runtime_spec
                .spec()
                .public_outputs
                .public_schema_id
                .clone(),
            public_output_event_id: public_event.event_id().clone(),
        }))
    );
    assert_eq!(completed_pos, stream.len() - 1);

    let complete_node = certified_complete_run_node(&fixture.runtime_spec).expect("complete node");
    let completion_seq = stream[completed_pos].seq();
    let completion_key = stream[completed_pos].commit_key().clone();
    let completion_commit = stream
        .iter()
        .filter(|event| event.seq() == completion_seq && event.commit_key() == &completion_key)
        .collect::<Vec<_>>();
    assert_eq!(completion_commit.len(), 4);
    let complete_attempt = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == complete_node.node_id && event.seq() < completion_seq =>
            {
                Some(payload.attempt_id.clone())
            }
            _ => None,
        })
        .expect("complete attempt started");
    let complete_receipt = completion_commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == complete_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("complete receipt cell");
    assert_eq!(complete_receipt.attempt_id, complete_attempt);
    assert_eq!(complete_receipt.cell_id, complete_node.output_cell);
    assert!(completion_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == complete_node.node_id
                    && payload.attempt_id == complete_attempt
                    && payload.output_cell_id == complete_node.output_cell
        )
    }));
    assert!(completion_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&complete_node.node_id)
                    && payload.attempt_id.as_ref() == Some(&complete_attempt)
                    && payload.artifact_ref.artifact_id == complete_receipt.artifact_id
                    && payload.artifact_ref.content_digest == complete_receipt.content_digest
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput
        )
    }));
    let stream_len = stream.len();
    assert_drive!(
        scheduler,
        store,
        fixture,
        PublicOutputProjected,
        "drive completed run"
    );
    store.assert_run_stream_len(&fixture.run_id, stream_len);
}

#[tokio::test]
async fn no_second_authority_full_run_stages_and_admits_first_artifact_references() {
    struct InlineRecordingRunner {
        expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for InlineRecordingRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                for (kind, version) in &self.expected_caps {
                    assert!(ctx.caps().contains(kind, version));
                }
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        artifact.artifact_id.clone(),
                        artifact.digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            InlineRecordingRunner {
                expected_caps: Vec::new(),
                output_bytes: br#"{"node":"a"}"#.to_vec(),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            InlineRecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_bytes: br#"{"node":"b"}"#.to_vec(),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry, &fixture),
        Arc::new(TestRuntimeArtifactStore {
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let store = RecordingTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");

    assert_eq!(
        drive_until_blocked_with_claim(&scheduler, &store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive full representative run"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store
            .projection_snapshot(&fixture.run_id)
            .await
            .run_state(&fixture.run_id),
        store::RunState::Completed
    );

    let stream = store.load_run_stream(&fixture.run_id).await;
    validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("representative stream validates");
    assert_every_certified_node_has_attempt(&fixture.runtime_spec, &stream);
    assert!(node_by_output(&fixture, &fixture.cell_a)
        .framework
        .is_none());
    assert!(matches!(
        &node_by_output(&fixture, &fixture.render_cell).framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    ));
    assert!(matches!(
        &certified_retention_manifest_node(&fixture.runtime_spec)
            .expect("retention node")
            .framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    ));
    assert!(matches!(
        &certified_complete_run_node(&fixture.runtime_spec)
            .expect("complete node")
            .framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    ));

    let mut first_reference_by_artifact = BTreeMap::<ArtifactId, usize>::new();
    let commits = store.commits();
    for (commit_index, commit) in commits.iter().enumerate() {
        let commit_references = commit
            .payloads
            .iter()
            .flat_map(referenced_artifact_ids_for_payload)
            .collect::<BTreeSet<_>>();
        assert!(
            !commit_references.is_empty() || commit.admitted_artifacts.is_empty(),
            "commit {} admitted artifacts without same-commit references",
            commit.commit_key
        );
        for admitted in &commit.admitted_artifacts {
            assert!(
                commit_references.contains(&admitted.artifact_id),
                "commit {} admitted unreferenced artifact {}",
                commit.commit_key,
                admitted.artifact_id
            );
        }
        for artifact_id in commit_references {
            first_reference_by_artifact
                .entry(artifact_id)
                .or_insert(commit_index);
        }
    }

    assert!(
        !first_reference_by_artifact.is_empty(),
        "representative run should reference artifacts"
    );
    for (artifact_id, commit_index) in first_reference_by_artifact {
        let commit = &commits[commit_index];
        assert!(
            commit
                .admitted_artifacts
                .iter()
                .any(|admitted| admitted.artifact_id == artifact_id),
            "artifact {artifact_id} was first referenced by {} at seq {} but not admitted there",
            commit.commit_key,
            commit.seq.as_u64()
        );
    }
}

#[tokio::test]
async fn run_launch_commits_single_admission_root_and_admits_launch_artifacts() {
    let fixture = fixture();
    let scheduler = test_scheduler_with_artifacts(
        registered_fixture_runners(&fixture),
        Arc::new(TestRuntimeArtifactStore {
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let store = started_fixture_store(&scheduler, &fixture).await;

    let stream = store.load_run_stream(&fixture.run_id);
    let first = stream.first().expect("stream event");
    let admission_commit = stream
        .iter()
        .take_while(|event| event.seq() == first.seq() && event.commit_key() == first.commit_key())
        .collect::<Vec<_>>();
    assert_eq!(admission_commit.len(), 1);
    assert_eq!(admission_commit[0].ordinal(), store::CommitOrdinal::new(0));

    let run_admitted = match admission_commit[0].payload() {
        events::KernelEventPayload::RunAdmitted(payload) => payload,
        _ => panic!("first admission event must be RunAdmitted"),
    };
    assert_eq!(
        run_admitted.spec_artifact.role,
        events::ArtifactRole::TypedExecutionSpec
    );
    assert_eq!(
        run_admitted.certificate_artifact.role,
        events::ArtifactRole::TypedSpecCertificate
    );
    assert!(run_admitted
        .config_artifacts
        .iter()
        .all(|artifact| artifact.role == events::ArtifactRole::TypedConfig));
    assert_eq!(
        fixture.seed_ref.seed_artifact.role,
        events::ArtifactRole::SeedInput
    );
    assert!(stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(_)
                | events::KernelEventPayload::StateAttemptCompleted(_)
                | events::KernelEventPayload::CellProduced(_)
                | events::KernelEventPayload::ArtifactReferenced(_)
                | events::KernelEventPayload::RetentionRefsAppended(_)
        )
    }));
}

#[tokio::test]
async fn scheduler_binds_staged_retention_refs_and_projects_manifest() {
    let fixture = fixture_with_retention_lifecycle_node();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let projection_snapshot = store.projection_snapshot();
    let start_retention = projection_snapshot
        .retention(&fixture.run_id)
        .expect("run-start retention");
    assert!(start_retention
        .refs
        .contains_key(&fixture.seed_ref.seed_artifact.artifact_id));

    let status = drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
        .await
        .expect("drive to completion");
    assert_eq!(status, SchedulerStatus::PublicOutputProjected);
    let render_receipt_artifact = match store
        .projection_snapshot()
        .cell_terminal(&fixture.render_cell)
        .expect("render receipt cell")
    {
        store::CellTerminalProjection::Produced { artifact_id, .. } => artifact_id.clone(),
        terminal => panic!("unexpected render terminal: {terminal:?}"),
    };
    assert!(store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .expect("runtime retention")
        .refs
        .contains_key(&render_receipt_artifact));

    let projection_snapshot = store.projection_snapshot();
    let projection = projection_snapshot
        .retention(&fixture.run_id)
        .expect("retention projection");
    let manifest = projection.manifest.as_ref().expect("manifest");
    assert_eq!(manifest.manifest_seq, 1);
    assert!(projection.refs.contains_key(&manifest.manifest_artifact_id));

    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    let stream = store.load_run_stream(&fixture.run_id);
    let manifest_event = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == manifest.manifest_artifact_id =>
            {
                Some((event.seq(), payload))
            }
            _ => None,
        })
        .expect("manifest event");
    let retention_seq = manifest_event.0;
    assert_eq!(manifest_event.1.manifest_digest, manifest.manifest_digest);
    let receipt = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if event.seq() == retention_seq && payload.node_id == retention_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("retention receipt cell");
    assert_eq!(receipt.cell_id, retention_node.output_cell);
    assert!(stream.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(payload)
                if event.seq() < retention_seq
                    && payload.node_id == retention_node.node_id
                    && payload.attempt_id == receipt.attempt_id
        )
    }));
    assert!(stream.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if event.seq() == retention_seq
                    && payload.node_id == retention_node.node_id
                    && payload.attempt_id == receipt.attempt_id
                    && payload.output_cell_id == retention_node.output_cell
        )
    }));
    let manifest_ref = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload)
                if event.seq() == retention_seq
                    && payload.reason == events::RetentionReason::ManifestProjection =>
            {
                payload.refs.first()
            }
            _ => None,
        })
        .expect("manifest retention ref");
    assert_eq!(manifest_ref.artifact_id, manifest.manifest_artifact_id);
    assert_eq!(manifest_ref.content_digest, manifest.manifest_digest);
    assert_eq!(manifest_ref.role, events::ArtifactRole::RetentionManifest);
}

#[tokio::test]
async fn retention_manifest_projection_retry_is_idempotent_after_current_store_advanced() {
    let fixture = fixture_with_retention_lifecycle_node();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;
    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    append_attempt_start(&mut store, &fixture, retention_node, 1);
    let stale_stream = store.load_run_stream(&fixture.run_id);

    drive_ok!(
        scheduler,
        store,
        fixture,
        "advance current store with retention projection"
    );
    assert!(store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .and_then(|retention| retention.manifest.as_ref())
        .is_some());

    let stale_store = StaleStreamStore::new(&mut store, stale_stream);
    assert_eq!(
        drive_once_with_claim(
            &scheduler,
            &stale_store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("idempotent retention retry"),
        SchedulerStatus::Advanced
    );
}

#[tokio::test]
async fn runtime_rejects_standalone_retention_manifest_projection_history() {
    let fixture = fixture_with_retention_lifecycle_node();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;

    let manifest = build_retention_manifest_artifact(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store.load_run_stream(&fixture.run_id),
        &store::ArtifactByteAuthorityMap::new(),
    )
    .expect("manifest");
    let manifest_evidence = manifest.evidence.clone();
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("synthetic/standalone-retention-projection")
            .expect("commit key"),
        payloads: retention_manifest_payloads(&fixture.runtime_spec, &fixture.run_id, manifest),
        required_artifacts: vec![manifest_evidence],
        preconditions: store::CommitPreconditions::default(),
    };
    store
        .append_prepared_commit(request)
        .expect("synthetic standalone projection");

    assert!(matches!(
        RuntimeRunView::from_stream(
            &fixture.runtime_spec,
            &fixture.run_id,
            &store.load_run_stream(&fixture.run_id)
        ),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("retention manifest projection was not produced")
    ));
}

#[tokio::test]
async fn runtime_rejects_complete_run_receipt_commit_without_run_completed() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
        .await
        .expect("drive to completion");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(payload, events::KernelEventPayload::RunCompleted(_))
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("missing RunCompleted")
    ));
}

#[tokio::test]
async fn public_output_render_failure_resumes_and_completes() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "drive a");
    drive_ok!(scheduler, store, fixture, "drive b");
    let render_node = node_by_output(&fixture, &fixture.render_cell);
    let failed_attempt = append_attempt_start(&mut store, &fixture, render_node, 1);
    append_public_output_render_failure(&mut store, &fixture, render_node, &failed_attempt);
    assert!(matches!(
        store.projection_snapshot().public_output(
            &fixture.run_id,
            &fixture.runtime_spec.spec().public_outputs.public_schema_id,
        ),
        Some(store::PublicOutputProjection::RenderFailed { .. })
    ));

    assert_eq!(
        drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
            .await
            .expect("retry render"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &fixture.render_node),
        2
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(matches!(
        store.projection_snapshot().public_output(
            &fixture.run_id,
            &fixture.runtime_spec.spec().public_outputs.public_schema_id,
        ),
        Some(store::PublicOutputProjection::Produced { .. })
    ));
}

#[test]
fn certified_runtime_spec_rejects_hash_mismatch() {
    let fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    envelope.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    assert!(matches!(
        CertifiedRuntimeSpec::from_verified_envelope(envelope),
        Err(RuntimeError::SpecHash(_))
    ));
}

#[test]
fn certified_runtime_spec_accepts_certifier_authority() {
    let (certified, _registry) = certifier_backed_runtime_authority();
    let runtime = CertifiedRuntimeSpec::new(certified).expect("runtime authority");
    assert!(!runtime.topological_order().is_empty());
}

#[test]
fn certified_runtime_spec_accepts_verified_persisted_parts_authority() {
    let (certified, registry) = certifier_backed_runtime_authority();
    let persisted_parts = certified
        .to_persisted_parts()
        .expect("persisted spec/certificate parts");
    let verified = mfm_certify::verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &registry,
    )
    .expect("verified persisted spec/certificate");
    let runtime = CertifiedRuntimeSpec::new(verified).expect("runtime authority");
    assert!(!runtime.topological_order().is_empty());
}

#[tokio::test]
async fn replay_rejects_run_completed_without_public_output_evidence() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-complete-without-public-output")
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(
                events::RunCompleted {
                    run_id: fixture.run_id.clone(),
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    outcome: events::RunCompletionOutcome::Completed(Box::new(
                        events::PublicOutputCompletionEvidence {
                            public_output_schema_id: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .public_schema_id
                                .clone(),
                            public_output_event_id: EventId::from_digest(
                                DigestAlgorithm::Sha256JcsV1,
                                D9,
                            ),
                        },
                    )),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged completion");

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("RunCompleted appeared before PublicOutputProduced")
    ));
}

#[tokio::test]
async fn scheduler_rejects_uncertified_capability_use() {
    struct BadFactRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
    }

    impl ErasedNodeRunner for BadFactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let (response_evidence, _response_bytes) =
                    test_fact_response_artifact(ctx.node(), 194);
                Ok(ErasedRunnerOutput::new(vec![
                    RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
                        events::FactRecorded {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            claim: test_fact_claim(
                                196,
                                ctx.node().config_ref.schema_id.clone(),
                                content(0xc1),
                                &response_evidence,
                                self.cap_kind.clone(),
                                self.cap_version.clone(),
                                AdapterKind::new(
                                    "mfm.test",
                                    "adapter",
                                    DigestAlgorithm::Sha256JcsV1,
                                    D1,
                                )
                                .expect("adapter"),
                                AdapterVersion::new("mfm.adapter.v1").expect("adapter version"),
                            ),
                        },
                    )),
                ]))
            })
        }
    }

    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        BadFactRunner {
            cap_kind: fixture.cap_kind.clone(),
            cap_version: fixture.cap_version.clone(),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize uncertified capability use"
    );
}

#[tokio::test]
async fn runner_cannot_stage_artifact_with_foreign_producer() {
    struct ForeignProducerArtifactRunner {
        foreign_node_id: NodeId,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for ForeignProducerArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let mut artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                artifact.producer_node_id = Some(self.foreign_node_id.clone());
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let foreign_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        ForeignProducerArtifactRunner {
            foreign_node_id,
            output_artifact: artifact(0xa1),
            output_digest: content(0xa2),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize foreign producer artifact"
    );
}

#[tokio::test]
async fn runner_cannot_stage_inline_artifact_with_mismatched_bytes() {
    struct BadInlineArtifactRunner {
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for BadInlineArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    b"mismatched".to_vec(),
                    artifact,
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        BadInlineArtifactRunner {
            output_artifact: artifact(0xa1),
            output_digest: content(0xa2),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize mismatched inline artifact"
    );
}

#[tokio::test]
async fn runner_can_commit_inline_state_output_artifact() {
    struct InlineArtifactRunner {
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for InlineArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        artifact.artifact_id.clone(),
                        artifact.digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let output_bytes = br#"{"inline":true}"#.to_vec();
    let output_digest = digest_for_bytes(&output_bytes);
    let output_artifact =
        ArtifactId::from_digest(output_digest.algorithm(), *output_digest.digest());
    let registry =
        fixture_registry_with_first_runner(&fixture, "pure", InlineArtifactRunner { output_bytes });
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_drive!(scheduler, store, fixture, Advanced, "drive inline output");
    assert!(matches!(
        store.projection_snapshot().cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Produced {
            artifact_id,
            content_digest,
            ..
        }) if artifact_id == &output_artifact && content_digest == &output_digest
    ));
}

#[tokio::test]
async fn runner_cannot_stage_reserved_retention_reasons() {
    struct ReservedRetentionReasonRunner {
        output_bytes: Vec<u8>,
        reason: events::RetentionReason,
    }

    impl ErasedNodeRunner for ReservedRetentionReasonRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: vec![StagedRetentionRefs {
                        refs: vec![retention_ref_for_artifact(&artifact)],
                        reason: self.reason,
                        authority:
                            crate::artifacts::StagedRetentionRefAuthority::CurrentCommitArtifacts,
                    }],
                    payloads: terminal_payloads(
                        &ctx,
                        artifact.artifact_id.clone(),
                        artifact.digest.clone(),
                    ),
                })
            })
        }
    }

    for reason in [
        events::RetentionReason::RunAdmitted,
        events::RetentionReason::ManifestProjection,
        events::RetentionReason::PublicOutput,
    ] {
        let fixture = fixture();
        let node = node_by_output(&fixture, &fixture.cell_a).clone();
        let registry = fixture_registry_with_first_runner(
            &fixture,
            "pure",
            ReservedRetentionReasonRunner {
                output_bytes: br#"{"reserved":true}"#.to_vec(),
                reason,
            },
        );
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
        let stream_before = store.load_run_stream(&fixture.run_id);

        assert_drive!(
            scheduler,
            store,
            fixture,
            Advanced,
            "terminalize reserved retention reason"
        );
        store.assert_run_stream_len(&fixture.run_id, stream_before.len() + 4);
        assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
    }
}

#[tokio::test]
async fn runner_output_requires_payload_bound_staged_artifact() {
    struct MissingStagedArtifactRunner {
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for MissingStagedArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                Ok(ErasedRunnerOutput {
                    staged_artifacts: Vec::new(),
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        MissingStagedArtifactRunner {
            output_artifact: artifact(0xa1),
            output_digest: content(0xa2),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize missing staged artifact"
    );
}

#[tokio::test]
async fn rejected_staged_payload_mismatch_does_not_admit_artifact_evidence() {
    struct MismatchedStagedArtifactRunner {
        staged_artifact: ArtifactId,
        staged_digest: ContentDigest,
        payload_artifact: ArtifactId,
        payload_digest: ContentDigest,
    }

    impl ErasedNodeRunner for MismatchedStagedArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.staged_artifact.clone(),
                    self.staged_digest.clone(),
                );
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.payload_artifact.clone(),
                        self.payload_digest.clone(),
                    ),
                })
            })
        }
    }

    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let staged_artifact = artifact(0xa1);
    let staged_digest = content(0xa2);
    let payload_artifact = artifact(0xa3);
    let payload_digest = content(0xa4);
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        MismatchedStagedArtifactRunner {
            staged_artifact: staged_artifact.clone(),
            staged_digest: staged_digest.clone(),
            payload_artifact,
            payload_digest,
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize staged payload mismatch"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");

    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(&node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_logical_key =
        store::LogicalEventKey::new(format!("attempt:{}:{}", node.node_id, attempt_id))
            .expect("attempt key");
    let leaked_artifact_request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("missing-leaked-staged-artifact").expect("commit key"),
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                cell_id: node.output_cell.clone(),
                scope_id: node.scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: descriptor.output_semantic_type_id.clone(),
                schema_id: descriptor.output_schema_id.clone(),
                value_lineage: output_cell.value_lineage.clone(),
                artifact_id: staged_artifact.clone(),
                content_digest: staged_digest,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            }),
        ],
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![attempt_logical_key],
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    };
    let artifacts = store::CommitArtifactEvidenceSet::new(
        leaked_artifact_request.required_artifacts().to_vec(),
        Vec::new(),
    )
    .expect("leak probe artifact evidence set");
    let error =
        store::PreparedCommit::<store::AttemptTerminal>::new(leaked_artifact_request, artifacts)
            .expect_err("missing leak evidence rejects before append");
    assert!(matches!(
        error,
        store::StoreError::InvalidPreparedCommitPurpose { message, .. }
            if message.contains("missing required artifact evidence")
                && message.contains(staged_artifact.as_str())
    ));
}

#[test]
fn materialization_rejects_seed_digest_not_certified() {
    let fixture = fixture();
    let mut seed = fixture.seed_ref.clone();
    seed.digest = content(0xee);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    assert!(matches!(
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![seed],),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_config_artifact_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence.config_artifacts.clear();
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_fact_descriptor_artifact_evidence() {
    let (fixture, _descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("missing fact descriptor artifact")
                && message.contains(descriptor_ref.descriptor_hash.as_str())
    ));
}

#[tokio::test]
async fn run_start_admits_certified_fact_descriptor_artifacts() {
    let (fixture, descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let run_admitted = store.run_admitted(&fixture.run_id);
    assert_eq!(run_admitted.fact_descriptor_artifacts.len(), 1);
    let admitted_descriptor = &run_admitted.fact_descriptor_artifacts[0];
    assert_eq!(
        admitted_descriptor.role,
        events::ArtifactRole::FactDescriptor
    );
    assert_eq!(
        admitted_descriptor.content_digest,
        descriptor_ref.descriptor_hash
    );
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("descriptor-backed committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("descriptor-backed run stream validates");
}

#[tokio::test]
async fn raw_runtime_view_rejects_fact_descriptor_stream_without_artifact_authority() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let stream = store.load_run_stream(&fixture.run_id);
    let error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw descriptor-bearing stream must fail closed");
    assert!(matches!(
        error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
}

#[tokio::test]
async fn run_start_admitted_uses_committed_fact_descriptor_artifacts() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("descriptor-backed admission uses committed stream authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );
}

#[tokio::test]
async fn fact_bearing_runtime_prefix_rebuild_uses_retained_artifact_bytes() {
    let (fixture, descriptor, _) = fixture_with_read_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");
    let node_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let node_a_attempt = append_attempt_start(&mut store, &fixture, &node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        &node_a,
        &node_a_attempt,
        artifact(0x70),
        content(0x71),
    );
    let node = node_by_output(&fixture, &fixture.cell_b).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &node, 1);
    append_fact(&mut store, &fixture, &node, &attempt_id, 17, 23);

    let stream = store.load_run_stream(&fixture.run_id);
    let raw_error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw fact-bearing stream must fail closed");
    assert!(matches!(
        raw_error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("fact-bearing committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("fact-bearing runtime view validates with retained bytes");
    build_retention_manifest_artifact(
        &fixture.runtime_spec,
        &fixture.run_id,
        &stream,
        committed.artifact_byte_authority(),
    )
    .expect("fact-bearing prefix rebuilds with retained bytes");

    let missing = store::ArtifactByteAuthorityMap::new();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &missing)
        .expect_err("fact-bearing prefix without retained bytes must fail");

    let (response_evidence, _) = test_fact_response_artifact(&node, 23);
    let response_key = (
        response_evidence.artifact_id.clone(),
        response_evidence
            .evidence_hash()
            .expect("response evidence hash"),
    );
    let mut mismatched = committed.artifact_byte_authority().clone();
    mismatched
        .get_mut(&response_key)
        .expect("response bytes retained")
        .0 = b"{\"amount\":999}".to_vec();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &mismatched)
        .expect_err("fact-bearing prefix with mismatched response bytes must fail");
}

#[test]
fn run_start_rejects_mismatched_staged_launch_bytes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let base = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    let mut bad_spec = base.clone();
    bad_spec.spec_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_spec,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_certificate = base.clone();
    bad_certificate.certificate_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_certificate,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_config = base.clone();
    bad_config
        .config_artifacts
        .first_mut()
        .expect("config artifact")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_config,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_seed = base;
    bad_seed
        .seed_cells
        .first_mut()
        .expect("seed cell")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_seed,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));
    assert!(store.load_run_stream(&fixture.run_id).is_empty());
}

#[tokio::test]
async fn run_admission_returns_bound_context_with_capability_and_framework_authority() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let launch =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .expect("prepare launch");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("admitted run authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(authority.spec_hash(), fixture.runtime_spec.spec_hash());
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );

    let node = node_by_output(&fixture, &fixture.cell_b);
    let capability = authority
        .bound_context()
        .capability_authority_for(&node.node_id)
        .expect("capability authority");
    assert_eq!(capability.capabilities(), &node.capability_bindings);
    assert_eq!(capability.implementations().len(), 1);
    assert_eq!(
        capability.implementations()[0].descriptor(),
        &node.capability_bindings.capabilities[0]
    );
    assert_eq!(
        capability.implementations()[0].implementation_id().as_str(),
        "mfm.test.capability"
    );

    let render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("public-output render node");
    let framework = authority
        .bound_context()
        .framework_handler_for(&render_node.node_id)
        .expect("framework handler authority");
    assert_eq!(
        framework.kind(),
        BoundFrameworkHandlerKind::PublicOutputRender
    );

    let run_admitted = store.run_admitted(&fixture.run_id);
    assert_eq!(
        run_admitted.runner_executables,
        authority.bound_context().runner_executables()
    );
    assert!(
        !authority.bound_context().adapter_executables().is_empty(),
        "fixture must exercise adapter executable binding evidence"
    );
    assert_eq!(
        run_admitted.adapter_executables,
        authority.bound_context().adapter_executables()
    );
    scheduler
        .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
        .expect("binding validation");
}

#[test]
fn run_start_rejects_missing_capability_implementation() {
    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        RecordingRunner {
            expected_caps: Vec::new(),
            output_artifact: artifact(0xa1),
            output_digest: content(0xa2),
        },
    );
    let scheduler = test_scheduler(registry);
    let store = TestTypedRunStore::new();
    let error =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .err()
            .expect("missing capability implementation must reject launch");
    assert!(
        matches!(error, RuntimeError::RunnerBinding(message) if message.contains("missing capability implementation"))
    );
}

#[test]
fn run_start_rejects_capability_implementation_descriptor_mismatch() {
    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    let implementation_id =
        CapabilityImplementationId::new("mfm.test.capability").expect("capability implementation");
    registry
        .register_capability(CapabilityImplementationBinding::new(
            CapabilityDescriptor::new(
                fixture.cap_kind.clone(),
                fixture.cap_version.clone(),
                CapabilityRole::ReadExternal,
                "wrong-read-db",
            )
            .expect("wrong capability descriptor"),
            implementation_id,
        ))
        .expect("capability implementation");
    register_default_fixture_pure_runner(&mut registry, &fixture);
    register_default_fixture_read_runner(&mut registry, &fixture);
    let scheduler = test_scheduler(registry);
    let store = TestTypedRunStore::new();
    let error =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .err()
            .expect("mismatched capability implementation must reject launch");
    assert!(
        matches!(error, RuntimeError::RunnerBinding(message) if message.contains("differs from certified descriptor"))
    );
}

#[tokio::test]
async fn resume_rejects_missing_downstream_binding_before_attempt_start() {
    let fixture = fixture();
    let (_, mut store) = started_fixture_run(&fixture).await;
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let mut partial_registry = ErasedRunnerRegistry::new();
    register_default_fixture_pure_runner(&mut partial_registry, &fixture);
    let resume_scheduler = test_scheduler(partial_registry);
    let error = drive_fixture_once(&resume_scheduler, &mut store, &fixture)
        .await
        .expect_err("missing descriptor b binding should reject bound context");

    assert!(
        matches!(error, RuntimeError::RunnerBinding(message) if message.contains("missing runner binding"))
    );
    store.assert_run_stream_len(&fixture.run_id, stream_len_before);
}

#[tokio::test]
async fn resume_rejects_runner_executable_identity_mismatch_before_attempt_start() {
    let fixture = fixture();
    let (_, mut store) = started_fixture_run(&fixture).await;
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let mut changed_registry = ErasedRunnerRegistry::new();
    let mut changed_a = binding(
        fixture.descriptor_a.clone(),
        "pure",
        RecordingRunner {
            expected_caps: Vec::new(),
            output_artifact: artifact(0xa1),
            output_digest: content(0xa2),
        },
    );
    changed_a.executable.binary_digest = content(0xee);
    changed_registry.register(changed_a).expect("binding a");
    register_default_fixture_read_runner(&mut changed_registry, &fixture);
    let resume_scheduler = fixture_scheduler(changed_registry, &fixture);
    let run_admitted = store.run_admitted(&fixture.run_id);
    resume_scheduler
        .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
        .expect_err("changed executable identity should reject binding validation");
    let error = drive_fixture_once(&resume_scheduler, &mut store, &fixture)
        .await
        .expect_err("changed executable identity should reject bound context");

    assert!(matches!(error, RuntimeError::RunnerBinding(message)
            if message.contains("runner executable identities")));
    store.assert_run_stream_len(&fixture.run_id, stream_len_before);
}

#[tokio::test]
async fn resume_rejects_adapter_executable_identity_mismatch_before_attempt_start() {
    let fixture = fixture();
    let (_, mut store) = started_fixture_run(&fixture).await;
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let mut changed_adapter = test_adapter_executable_identity();
    changed_adapter.binary_digest = content(0xef);
    let resume_scheduler = test_scheduler(registered_fixture_runners_with_adapter_executable(
        &fixture,
        changed_adapter,
    ));
    let run_admitted = store.run_admitted(&fixture.run_id);
    resume_scheduler
        .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
        .expect_err("changed adapter executable identity should reject binding validation");
    let error = drive_fixture_once(&resume_scheduler, &mut store, &fixture)
        .await
        .expect_err("changed adapter executable identity should reject bound context");

    assert!(matches!(error, RuntimeError::RunnerBinding(message)
            if message.contains("adapter executable identities")));
    store.assert_run_stream_len(&fixture.run_id, stream_len_before);
}

#[tokio::test]
async fn runner_invocation_uses_run_admitted_config_evidence_without_reference_event() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let valid_stream = store.load_run_stream(&fixture.run_id);
    assert!(valid_stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
        )
    }));
    drive_ok!(
        scheduler,
        store,
        fixture,
        "drive with RunAdmitted config evidence"
    );
}

#[tokio::test]
async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id)
        )
    });
    {
        let corrupt_store = StaleStreamStore::new(&mut store, corrupt_stream);
        assert!(matches!(
            drive_once_with_claim(&scheduler, &corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InputMaterialization(message))
                if message.contains("is not committed in the run stream")
        ));
    }
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
        1
    );
}

#[tokio::test]
async fn post_start_materialization_failure_terminalizes_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node = node_by_output(&fixture, &fixture.cell_b).clone();
    {
        let corrupt_store = MissingInputArtifactRefStore::new(&mut store, producer_node_id);
        assert_eq!(
            drive_once_with_claim(
                &scheduler,
                &corrupt_store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("terminalize materialization failure"),
            SchedulerStatus::Advanced
        );
    }

    assert_node_failed_with_code_and_retryable(
        &store,
        &consumer_node.node_id,
        "input_materialization_failed",
        true,
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&consumer_node.output_cell)
        .is_none());
}

#[tokio::test]
async fn post_start_runtime_validation_failure_terminalizes_attempt() {
    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        ErrorRunner {
            error: RuntimeError::RuntimeValidation(
                "synthetic post-start validation failure".to_owned(),
            ),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize runtime validation failure"
    );

    let node = node_by_output(&fixture, &fixture.cell_a);
    assert_node_failed_with_code(&store, &node.node_id, "runtime_validation_failed");
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=0 failed=1 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn post_start_invalid_run_stream_failure_does_not_terminalize_attempt() {
    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        ErrorRunner {
            error: RuntimeError::InvalidRunStream("synthetic corrupt stream authority".to_owned()),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("synthetic corrupt stream authority")
    ));

    let node = node_by_output(&fixture, &fixture.cell_a);
    let projection_snapshot = store.projection_snapshot();
    let attempts = projection_snapshot
        .attempts()
        .filter(|((node_id, _), _)| node_id == &node.node_id)
        .map(|(_, attempt)| attempt)
        .collect::<Vec<_>>();
    assert_eq!(attempts.len(), 1);
    assert!(matches!(
        attempts[0].status,
        store::AttemptStatus::Started { .. }
    ));
    assert_failure_code_count(&store, "runtime_validation_failed", 0);
    assert_failure_code_count(&store, "runner_output_invalid", 0);
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let forged_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell != fixture.cell_a)
        .expect("second node")
        .clone();
    let certified_cell = fixture
        .runtime_spec
        .cell(&fixture.cell_a)
        .expect("cell a")
        .clone();
    let forged_attempt = AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xfa; 32]),
    );
    let artifact_id = artifact(0xfa);
    let artifact_digest = content(0xfb);
    let forged_artifact = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: artifact_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(certified_cell.schema_id.clone()),
        semantic_type_id: Some(certified_cell.semantic_type_id.clone()),
        producer_node_id: Some(forged_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-attempt-start").expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    attempt_no: 1,
                    state_kind: forged_node.state_kind.clone(),
                    state_version: forged_node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged attempt start");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-terminal").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    cell_id: fixture.cell_a.clone(),
                    scope_id: certified_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: certified_cell.semantic_type_id.clone(),
                    schema_id: certified_cell.schema_id.clone(),
                    value_lineage: certified_cell.value_lineage.clone(),
                    artifact_id,
                    content_digest: artifact_digest,
                    producer_state_kind: Some(forged_node.state_kind.clone()),
                    producer_state_version: Some(forged_node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt,
                    output_cell_id: fixture.cell_a.clone(),
                }),
            ],
            required_artifacts: vec![forged_artifact],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged terminal");

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn store_rejects_fact_without_started_attempt() {
    let fixture = fixture();
    let (_, mut store) = started_fixture_run(&fixture).await;
    let node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_b)
        .expect("read node")
        .clone();
    let fact_schema = node.config_ref.schema_id.clone();
    let (fact_evidence, _fact_bytes) = test_fact_response_artifact(&node, 210);
    assert!(store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-fact").expect("commit key"),
            payloads: vec![events::KernelEventPayload::FactRecorded(
                events::FactRecorded {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: AttemptId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        DigestBytes::from_array([0xd3; 32]),
                    ),
                    claim: test_fact_claim(
                        210,
                        fact_schema.clone(),
                        content(0xd4),
                        &fact_evidence,
                        fixture.cap_kind.clone(),
                        fixture.cap_version.clone(),
                        fixture.adapter_kind.clone(),
                        fixture.adapter_version.clone(),
                    ),
                },
            )],
            required_artifacts: vec![fact_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .is_err());
}

#[tokio::test]
async fn replay_rejects_public_output_without_render_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let non_render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_a)
        .expect("non-render node")
        .clone();
    let public_cell = fixture
        .runtime_spec
        .spec()
        .public_outputs
        .outputs
        .first()
        .expect("public cell")
        .clone();
    let source_artifact = artifact(0xe1);
    let source_digest = content(0xe2);
    let producer_node_id = match &public_cell.producer {
        spec::CellProducer::Node(node_id) => Some(node_id.clone()),
        spec::CellProducer::Seed(_) => None,
    };
    let source_evidence = store::ArtifactEvidenceRef {
        artifact_id: source_artifact.clone(),
        digest: source_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(public_cell.schema_id.clone()),
        semantic_type_id: Some(public_cell.semantic_type_id.clone()),
        producer_node_id,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let forged_attempt = append_attempt_start(&mut store, &fixture, &non_render_node, 1);
    let output_cell = fixture
        .runtime_spec
        .cell(&non_render_node.output_cell)
        .expect("non-render output")
        .clone();
    let receipt_artifact = artifact(0xe3);
    let receipt_digest = content(0xe4);
    let receipt_evidence = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact.clone(),
        digest: receipt_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(non_render_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-public-output").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    cell_id: non_render_node.output_cell.clone(),
                    scope_id: output_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    artifact_id: receipt_artifact,
                    content_digest: receipt_digest,
                    producer_state_kind: Some(non_render_node.state_kind.clone()),
                    producer_state_version: Some(non_render_node.state_version.clone()),
                }),
                events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    receipt_cell_id: non_render_node.output_cell.clone(),
                    public_schema_id: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .public_schema_id
                        .clone(),
                    output_spec_digest: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .digest()
                        .expect("public digest"),
                    cells: vec![events::NamedTypedCellRef {
                        public_field_path: public_cell.public_field_path.clone(),
                        cell_id: public_cell.cell_id.clone(),
                        producer: public_cell.producer.clone(),
                        scope_id: public_cell.scope_id.clone(),
                        semantic_type_id: public_cell.semantic_type_id.clone(),
                        schema_id: public_cell.schema_id.clone(),
                        value_lineage: public_cell.value_lineage.clone(),
                        content_digest: source_digest,
                        artifact_id: source_artifact,
                    }],
                    rendered_digest: content(0xe5),
                    rendered_artifact_id: None,
                    renderer_descriptor_id: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .renderer_descriptor
                        .descriptor_id
                        .clone(),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    output_cell_id: non_render_node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![source_evidence, receipt_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    non_render_node.node_id, forged_attempt
                ))
                .expect("attempt key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: non_render_node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged public output");
    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn replay_rejects_public_output_with_forged_rendered_digest() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "drive a");
    drive_ok!(scheduler, store, fixture, "drive b");

    let render_node = node_by_output(&fixture, &fixture.render_cell).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &render_node, 1);
    let output_cell = fixture
        .runtime_spec
        .cell(&render_node.output_cell)
        .expect("render output")
        .clone();
    let bad_rendered_digest = content(0xf1);
    let bad_receipt_digest = content(0xf2);
    let bad_receipt_artifact =
        ArtifactId::from_digest(bad_receipt_digest.algorithm(), *bad_receipt_digest.digest());
    let bad_receipt_evidence = store::ArtifactEvidenceRef {
        artifact_id: bad_receipt_artifact.clone(),
        digest: bad_receipt_digest.clone(),
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(render_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let public_cells = fixture
        .runtime_spec
        .spec()
        .public_outputs
        .outputs
        .iter()
        .map(|public_cell| {
            let projection_snapshot = store.projection_snapshot();
            let Some(store::CellTerminalProjection::Produced {
                artifact_id,
                content_digest,
                ..
            }) = projection_snapshot.cell_terminal(&public_cell.cell_id)
            else {
                panic!("public cell should be produced");
            };
            events::NamedTypedCellRef {
                public_field_path: public_cell.public_field_path.clone(),
                cell_id: public_cell.cell_id.clone(),
                producer: public_cell.producer.clone(),
                scope_id: public_cell.scope_id.clone(),
                semantic_type_id: public_cell.semantic_type_id.clone(),
                schema_id: public_cell.schema_id.clone(),
                value_lineage: public_cell.value_lineage.clone(),
                content_digest: content_digest.clone(),
                artifact_id: artifact_id.clone(),
            }
        })
        .collect::<Vec<_>>();
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &render_node.framework else {
        panic!("expected render node");
    };
    let error = store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-public-output-rendered-digest")
                .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    cell_id: render_node.output_cell.clone(),
                    scope_id: output_cell.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    artifact_id: bad_receipt_artifact,
                    content_digest: bad_receipt_digest,
                    producer_state_kind: Some(render_node.state_kind.clone()),
                    producer_state_version: Some(render_node.state_version.clone()),
                }),
                events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    receipt_cell_id: render_node.output_cell.clone(),
                    public_schema_id: render.public_schema_id.clone(),
                    output_spec_digest: render.output_spec_digest.clone(),
                    cells: public_cells,
                    rendered_digest: bad_rendered_digest,
                    rendered_artifact_id: None,
                    renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    output_cell_id: render_node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![bad_receipt_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    render_node.node_id, attempt_id
                ))
                .expect("attempt key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: render_node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect_err("forged public output rejects before replay");
    assert!(matches!(
        error,
        store::StoreError::InvalidPreparedCommitPurpose { message, .. }
            if message.contains("missing required artifact evidence")
    ));
}

#[tokio::test]
async fn recovery_interrupts_started_pure_attempt_before_retrying_fresh_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let interrupted_attempt_id = append_attempt_start(&mut store, &fixture, node, 1);

    assert_drive!(scheduler, store, fixture, Advanced, "interrupt pure");
    let projection_snapshot = store.projection_snapshot();
    let interrupted_attempt = projection_snapshot
        .attempt(&node.node_id, &interrupted_attempt_id)
        .expect("interrupted attempt");
    assert!(matches!(
        interrupted_attempt.status,
        store::AttemptStatus::Interrupted
    ));
    assert!(
        store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none(),
        "interruption must not terminalize the output cell"
    );

    assert_drive!(scheduler, store, fixture, Advanced, "retry pure");
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        2
    );
    let retry_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        2,
    )
    .expect("retry attempt id");
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &retry_attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=1 failed=0 interrupted=1 total=2] cells=1 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn recovery_delegates_started_side_effect_attempt_to_side_effect_lifecycle() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;

    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, _) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-recovery",
        "sidefx-recovery-open",
    );
    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=1 lanes[run=1 total=1] public_outputs=0 retentions=1"
    );

    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open side-effect attempt")
    {
        crate::recovery::OpenAttemptDisposition::DelegateSideEffect {
            node: recovered_node,
            attempt_id: recovered_attempt,
            attempt_no,
        } => {
            assert_eq!(recovered_node.node_id, node.node_id);
            assert_eq!(recovered_attempt, attempt_id);
            assert_eq!(attempt_no, 1);
        }
        crate::recovery::OpenAttemptDisposition::Continue { .. } => {
            panic!("side-effect attempt must delegate to side-effect lifecycle")
        }
        crate::recovery::OpenAttemptDisposition::Interrupt { .. }
        | crate::recovery::OpenAttemptDisposition::RetryTerminalization { .. }
        | crate::recovery::OpenAttemptDisposition::OperationalBlock { .. } => {
            panic!("side-effect attempt must delegate to side-effect lifecycle")
        }
    }
}

#[test]
fn side_effect_attempt_view_from_erased_context_is_empty_before_ledger() {
    let fixture = fixture_with_first_side_effect_state();
    with_runner_erased_ctx(&fixture, &fixture.cell_a, |ctx| {
        let view = SideEffectAttemptView::from_erased_context(&ctx).expect("side-effect view");
        assert!(view.projection().is_none());
        assert!(view.ledger_state().is_none());
        assert!(view.phase().is_none());
    });
}

#[tokio::test]
async fn side_effect_attempt_view_from_verified_context_exposes_ledger_state() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;

    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-view",
        "sidefx-view-open",
    );
    let loader = crate::history::VerifiedRunContextLoader::new(
        crate::binding::BoundRuntimeContextLoader::new(registered_side_effect_fixture_runners(
            &fixture,
        )),
    );
    let context = loader
        .load_async(&fixture.runtime_spec, &fixture.run_id, &store)
        .await
        .expect("verified context");

    let view = SideEffectAttemptView::from_verified_context(
        &fixture.runtime_spec,
        &context,
        node,
        &attempt_id,
    )
    .expect("side-effect view");
    assert_eq!(
        view.projection().map(|projection| &projection.ledger_key),
        Some(&ledger_key)
    );
    assert_eq!(
        view.ledger_state().map(|state| state.ledger_purpose()),
        Some(&events::SideEffectLedgerPurpose::Forward)
    );
    assert_eq!(
        view.projection().expect("projection").intent.attempt_id,
        attempt_id
    );
    match view.phase().expect("ledger phase") {
        store::SideEffectLedgerPhase::Prepared {
            claim,
            resource_key,
            ..
        } => {
            assert_eq!(claim.attempt_id, attempt_id);
            assert_eq!(claim.invocation_epoch, 1);
            assert!(resource_key.is_some());
        }
        other => panic!("unexpected side-effect phase: {other:?}"),
    }
}

#[tokio::test]
async fn recovery_sweep_includes_open_remediation_attempts() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let scheduler = compensated_saga_scheduler(&fixture);
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output.clone(), forward_b_output.clone()],
        "drive forward side-effect phase",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_a_output)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_b_output)
        .is_some());

    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let remediation = fixture
        .runtime_spec
        .remediation_for_forward_node(&forward_b.node_id)
        .expect("remediation for forward b");
    let remediation_attempt = append_attempt_start(&mut store, &fixture, remediation, 1);

    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open remediation attempt")
    {
        crate::recovery::OpenAttemptDisposition::Continue {
            node,
            attempt_id,
            attempt_no,
        } => {
            assert_eq!(node.node_id, remediation.node_id);
            assert_eq!(attempt_id, remediation_attempt);
            assert_eq!(attempt_no, 1);
        }
        crate::recovery::OpenAttemptDisposition::DelegateSideEffect { .. }
        | crate::recovery::OpenAttemptDisposition::Interrupt { .. }
        | crate::recovery::OpenAttemptDisposition::RetryTerminalization { .. }
        | crate::recovery::OpenAttemptDisposition::OperationalBlock { .. } => {
            panic!("remediation attempt before ledger should continue through attempt lifecycle")
        }
    }
}

#[tokio::test]
async fn recovery_interrupts_side_effect_attempt_after_intent_before_prepare() {
    assert_prepared_boundary_side_effect_recovery_interrupts(false).await;
}

#[tokio::test]
async fn recovery_interrupts_side_effect_attempt_after_claim_before_prepare() {
    assert_prepared_boundary_side_effect_recovery_interrupts(true).await;
}

async fn assert_prepared_boundary_side_effect_recovery_interrupts(emit_claim: bool) {
    let fixture = fixture_with_first_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        PrePreparedSideEffectRunner { emit_claim },
    ));
    let mut store = started_fixture_store(&scheduler, &fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a).clone();

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "append pre-prepared side-effect evidence"
    );
    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open side-effect attempt")
    {
        crate::recovery::OpenAttemptDisposition::Interrupt {
            node: recovered_node,
            attempt_id: recovered_attempt,
            attempt_no,
        } => {
            assert_eq!(recovered_node.node_id, node.node_id);
            assert_eq!(recovered_attempt, attempt_id);
            assert_eq!(attempt_no, 1);
        }
        _ => panic!("pre-prepared side-effect attempt must interrupt"),
    }

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "interrupt pre-prepared side-effect attempt"
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&node.node_id, &attempt_id)
            .expect("attempt projection")
            .status,
        store::AttemptStatus::Interrupted
    ));
}

#[tokio::test]
async fn recovery_rejects_split_terminal_cell_and_attempt_completion() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.output_cell_id == fixture.cell_a
        )
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::Store(message))
            if message.contains("terminal cell requires matching attempt completion")
    ));
}

#[tokio::test]
async fn recovery_rejects_attempt_started_before_inputs_were_terminal() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let node_a = node_by_output(&fixture, &fixture.cell_a);
    let node_b = node_by_output(&fixture, &fixture.cell_b);
    append_attempt_start(&mut store, &fixture, node_b, 1);
    let attempt_a = append_attempt_start(&mut store, &fixture, node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        node_a,
        &attempt_a,
        artifact(0xa1),
        content(0xa2),
    );

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn recovery_reuses_committed_read_facts_for_same_attempt() {
    struct FactReuseRunner {
        fact_key: mfm_facts::FactKey,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for FactReuseRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let fact = ctx
                    .recorded_facts()
                    .by_fact_key(&self.fact_key)
                    .next()
                    .map(|(_, fact)| fact)
                    .expect("recorded fact");
                assert_eq!(fact.fact_key, self.fact_key);
                assert_eq!(
                    fact.request_schema_id,
                    Some(ctx.node().config_ref.schema_id.clone())
                );
                assert_eq!(ctx.recorded_facts().iter().count(), 1);
                let artifact = store::ArtifactEvidenceRef {
                    artifact_id: self.output_artifact.clone(),
                    digest: self.output_digest.clone(),
                    byte_len: 17,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                    semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let (fixture, fact_descriptor, _descriptor_ref) = fixture_with_read_node_fact_descriptor();
    let fact_key = test_fact_key(212);
    let mut registry = ErasedRunnerRegistry::new();
    register_default_fixture_pure_runner(&mut registry, &fixture);
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            FactReuseRunner {
                fact_key: fact_key.clone(),
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registry,
        &fixture,
        &[fact_descriptor],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 212);

    drive_ok!(scheduler, store, fixture, "resume read");
    assert_eq!(fact_recorded_count(&store, &fact_key), 1);
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}

#[tokio::test]
async fn recovery_retains_same_subject_facts_by_claim_id_for_same_attempt() {
    struct SameSubjectFactsRunner {
        fact_key: mfm_facts::FactKey,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for SameSubjectFactsRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let same_subject_facts = ctx
                    .recorded_facts()
                    .by_fact_key(&self.fact_key)
                    .collect::<Vec<_>>();
                assert_eq!(same_subject_facts.len(), 2);
                assert_eq!(ctx.recorded_facts().iter().count(), 2);

                let claim_ids = same_subject_facts
                    .iter()
                    .map(|(claim_id, _)| (*claim_id).clone())
                    .collect::<BTreeSet<_>>();
                assert_eq!(claim_ids.len(), 2);

                let artifact_ids = same_subject_facts
                    .iter()
                    .map(|(claim_id, fact)| {
                        assert_eq!(&fact.fact_claim_id, *claim_id);
                        assert_eq!(&fact.fact_key, &self.fact_key);
                        fact.artifact_id.clone()
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(artifact_ids.len(), 2);

                let artifact = store::ArtifactEvidenceRef {
                    artifact_id: self.output_artifact.clone(),
                    digest: self.output_digest.clone(),
                    byte_len: 17,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                    semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    let (fixture, fact_descriptor, _descriptor_ref) = fixture_with_read_node_fact_descriptor();
    let fact_key = test_fact_key(212);
    let mut registry = ErasedRunnerRegistry::new();
    register_default_fixture_pure_runner(&mut registry, &fixture);
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            SameSubjectFactsRunner {
                fact_key: fact_key.clone(),
                output_artifact: artifact(0xb3),
                output_digest: content(0xb4),
            },
        ))
        .expect("binding b");
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registry,
        &fixture,
        &[fact_descriptor],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 212);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 213);

    assert_eq!(fact_recorded_count(&store, &fact_key), 2);
    let projected_claim_ids = store
        .projection_snapshot()
        .fact_records()
        .filter(|(_, fact)| fact.claim.subject().fact_key() == &fact_key)
        .map(|(claim_id, _)| claim_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(projected_claim_ids.len(), 2);

    drive_ok!(
        scheduler,
        store,
        fixture,
        "resume read with same-subject facts"
    );
    assert_eq!(fact_recorded_count(&store, &fact_key), 2);
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}

#[tokio::test]
async fn runtime_history_rejects_fact_descriptor_allowed_only_for_other_node() {
    let (fixture, read_descriptor, other_descriptor) =
        fixture_with_read_node_and_other_node_fact_descriptors();
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registered_fixture_runners(&fixture),
        &fixture,
        &[read_descriptor, other_descriptor.clone()],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 212);

    let (response_evidence, _response_bytes) = test_fact_response_artifact(node, 212);
    let corrupt_claim = test_fact_claim_for_descriptor(
        &other_descriptor,
        212,
        node.config_ref.schema_id.clone(),
        content(0xd4),
        &response_evidence,
        fixture.cap_kind.clone(),
        fixture.cap_version.clone(),
        fixture.adapter_kind.clone(),
        fixture.adapter_version.clone(),
    );
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_payloads(&valid_stream, |payload| match payload {
        events::KernelEventPayload::FactRecorded(recorded) if recorded.node_id == node.node_id => {
            let mut recorded = recorded.clone();
            recorded.claim = corrupt_claim.clone();
            Some(events::KernelEventPayload::FactRecorded(recorded))
        }
        _ => None,
    });
    let committed =
        block_on_ready(store.load_committed_run_stream(&fixture.run_id)).expect("committed stream");
    let corrupt_committed = store::CommittedRunStream::from_events_with_artifact_bytes(
        fixture.run_id.clone(),
        corrupt_stream,
        committed.artifact_byte_authority(),
    )
    .expect("corrupt committed stream remains structurally valid");

    assert!(matches!(
        RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &corrupt_committed),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("fact descriptor")
                && message.contains("not certified for producing node")
    ));
}

#[tokio::test]
async fn recovery_rejects_new_fact_after_same_attempt_fact_exists() {
    struct NewFactRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    impl ErasedNodeRunner for NewFactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let (response_evidence, _response_bytes) =
                    test_fact_response_artifact(ctx.node(), 224);
                Ok(ErasedRunnerOutput::new(vec![
                    RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
                        events::FactRecorded {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            claim: test_fact_claim(
                                224,
                                ctx.node().config_ref.schema_id.clone(),
                                content(0xe1),
                                &response_evidence,
                                self.cap_kind.clone(),
                                self.cap_version.clone(),
                                self.adapter_kind.clone(),
                                self.adapter_version.clone(),
                            ),
                        },
                    )),
                ]))
            })
        }
    }

    let (fixture, fact_descriptor, _descriptor_ref) = fixture_with_read_node_fact_descriptor();
    let mut registry = ErasedRunnerRegistry::new();
    register_default_fixture_pure_runner(&mut registry, &fixture);
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            NewFactRunner {
                cap_kind: fixture.cap_kind.clone(),
                cap_version: fixture.cap_version.clone(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
            },
        ))
        .expect("binding b");
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registry,
        &fixture,
        &[fact_descriptor],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 214, 214);

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize duplicate fact output"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
    assert_eq!(store.projection_snapshot().fact_records().count(), 1);
}

#[tokio::test]
async fn recovery_allows_managed_write_artifact_restage_before_terminal_commit() {
    let fixture = fixture_with_first_managed_write_state();
    let output_artifact = artifact(0xa1);
    let output_digest = content(0xa2);
    let node = node_by_output(&fixture, &fixture.cell_a);
    let expected_caps = node
        .capability_bindings
        .capabilities
        .iter()
        .map(|capability| (capability.kind.clone(), capability.version.clone()))
        .collect();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "managed-write",
        RecordingRunner {
            expected_caps,
            output_artifact: output_artifact.clone(),
            output_digest: output_digest.clone(),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());

    drive_ok!(scheduler, store, fixture, "resume managed write");
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}

#[tokio::test]
async fn scheduler_reloads_and_redecides_after_stale_expected_sequence_on_terminal_append() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = StaleOnceTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");

    assert_eq!(
        drive_once_with_claim(&scheduler, &store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive after injected stale terminal append"),
        SchedulerStatus::Advanced
    );
    assert!(
        store
            .projection_snapshot(&fixture.run_id)
            .await
            .cell_terminal(&fixture.cell_a)
            .is_some(),
        "scheduler must reload and observe the concurrently advanced terminal projection"
    );
}

#[tokio::test]
async fn side_effect_scheduler_commits_durable_ledger_phases_before_output() {
    let fixture = fixture_with_first_side_effect_state();
    let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;

    let node = node_by_output(&fixture, &fixture.cell_a);
    let verify_node = side_effect_verify_node_for_submit(&fixture, node);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let mut receipt_before_output = false;
    for _ in 0..8 {
        assert_drive!(
            scheduler,
            store,
            fixture,
            Advanced,
            "drive side effect phase"
        );
        let projection_snapshot = store.projection_snapshot();
        let projection = side_effect_projection_for_attempt(
            &fixture.runtime_spec,
            &fixture.run_id,
            &projection_snapshot,
            node,
            &attempt_id,
        )
        .expect("projection lookup")
        .expect("side-effect projection");
        if matches!(
            projection.phase,
            store::SideEffectPhase::ReceiptObserved { .. }
        ) && projection_snapshot
            .cell_terminal(&verify_node.output_cell)
            .is_none()
        {
            receipt_before_output = true;
            break;
        }
    }
    assert!(receipt_before_output);
    assert!(matches!(
        store.projection_snapshot().cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Skipped { .. })
    ));
    assert!(store
        .projection_snapshot()
        .cell_terminal(&verify_node.output_cell)
        .is_none());

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "materialize side-effect output"
    );

    assert!(store
        .projection_snapshot()
        .cell_terminal(&verify_node.output_cell)
        .is_some());
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        &fixture.run_id,
        &projection_snapshot,
        node,
        &attempt_id,
    )
    .expect("projection lookup")
    .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::ReceiptObserved { .. }
    ));
    assert!(store
        .load_run_stream(&fixture.run_id)
        .iter()
        .any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )));
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &verify_node.node_id),
        1
    );
}

#[tokio::test]
async fn runtime_rejects_exact_touched_set_receipt_without_evidence() {
    let fixture = fixture_with_first_exact_touched_set_side_effect_state();
    let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;

    assert_invalid_output_after(
        &scheduler,
        &mut store,
        &fixture,
        3,
        TOUCHED_SET_EVIDENCE_ERR,
    )
    .await;
}

#[tokio::test]
async fn runtime_rejects_exact_touched_set_confirmation_without_evidence() {
    let fixture = fixture_with_first_exact_touched_set_finalized_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_and_verify_runners_with(
        &fixture,
        DriverSideEffectRunner::new(&fixture),
        TouchedSetSideEffectVerifyRunner::with_receipt(&fixture),
    ));
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    assert_invalid_output_after(
        &scheduler,
        &mut store,
        &fixture,
        4,
        TOUCHED_SET_EVIDENCE_ERR,
    )
    .await;
}

#[tokio::test]
async fn runtime_rejects_touched_set_confirmation_without_exact_claim() {
    let fixture = fixture_with_first_finalized_side_effect_state();
    let scheduler = test_scheduler(registered_first_side_effect_and_verify_runners_with(
        &fixture,
        DriverSideEffectRunner::new(&fixture),
        TouchedSetSideEffectVerifyRunner::with_confirmation(&fixture),
    ));
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    assert_invalid_output_after(
        &scheduler,
        &mut store,
        &fixture,
        4,
        EXACT_TOUCHED_SET_CLAIM_ERR,
    )
    .await;
}

#[tokio::test]
async fn runtime_fails_pre_boundary_forward_attempt_before_saga_terminal() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let failing_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        FailActiveSideEffectAfterSagaRunner::before_invocation_started(&fixture),
    ));
    let mut store = started_fixture_store(&scheduler, &fixture).await;
    let forward_attempt = append_or_get_first_attempt(&mut store, &fixture, &forward_node);

    drive_until_side_effect_attempt_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_node,
        &forward_attempt,
        |phase| matches!(phase, store::SideEffectPhase::InvocationPrepared { .. }),
        "prepare forward side effect",
    )
    .await;

    let failing_attempt = append_or_get_started_attempt(&mut store, &fixture, &failing_node, 1);
    append_attempt_failure(&mut store, &fixture, &failing_node, &failing_attempt, false);
    assert_eq!(
        derive_fixture_saga(&fixture, store.projection_snapshot()).run_mode,
        store::RunMode::FailedWithoutAcdcClaim
    );

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "fail active pre-boundary forward attempt"
    );
    let projection_snapshot = store.projection_snapshot();
    assert!(matches!(
        side_effect_projection_for_attempt(
            &fixture.runtime_spec,
            &fixture.run_id,
            &projection_snapshot,
            &forward_node,
            &forward_attempt
        )
        .expect("side-effect lookup")
        .expect("side-effect projection")
        .phase,
        store::SideEffectPhase::Failed {
            failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
            ..
        }
    ));
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&forward_node.node_id, &forward_attempt)
            .expect("forward attempt")
            .status,
        store::AttemptStatus::Failed { .. }
    ));
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "resolve saga terminal after active attempt closed"
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_fails_not_submitted_forward_attempt_before_saga_terminal() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let failing_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let scheduler = test_scheduler(registered_first_side_effect_runners_with(
        &fixture,
        FailActiveSideEffectAfterSagaRunner::new(&fixture)
            .with_submission_decision(TestSubmissionDecision::NotSubmitted),
    ));
    let mut store = started_fixture_store(&scheduler, &fixture).await;
    let forward_attempt = append_or_get_first_attempt(&mut store, &fixture, &forward_node);

    drive_until_side_effect_attempt_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_node,
        &forward_attempt,
        |phase| matches!(phase, store::SideEffectPhase::NotSubmittedProven { .. }),
        "advance forward side effect before not-submitted proof",
    )
    .await;

    let failing_attempt = append_or_get_started_attempt(&mut store, &fixture, &failing_node, 1);
    append_attempt_failure(&mut store, &fixture, &failing_node, &failing_attempt, false);

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "fail active not-submitted forward attempt"
    );
    let projection_snapshot = store.projection_snapshot();
    assert!(matches!(
        side_effect_projection_for_attempt(
            &fixture.runtime_spec,
            &fixture.run_id,
            &projection_snapshot,
            &forward_node,
            &forward_attempt
        )
        .expect("side-effect lookup")
        .expect("side-effect projection")
        .phase,
        store::SideEffectPhase::Failed {
            failure_phase: events::side_effect::FailurePhase::AfterNotSubmittedProven,
            ..
        }
    ));
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "resolve saga terminal after not-submitted attempt closed"
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_remediates_confirmed_forward_ledgers_in_reverse_confirmation_order() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding forward b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "pure",
            BlockingRunner,
        ))
        .expect("binding failure node");
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output.clone(), forward_b_output.clone()],
        "drive forward side-effect phase",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_a_output)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_b_output)
        .is_some());
    let forward_a_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_a.node_id);
    let forward_b_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_b.node_id);
    let forward_a_pair = forward_pair_for_ledger(&store.projection_snapshot(), &forward_a_ledger);
    let forward_b_pair = forward_pair_for_ledger(&store.projection_snapshot(), &forward_b_ledger);

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::Remediating);
    assert_eq!(saga.obligations.len(), 2);

    for _ in 0..12 {
        let status = drive_ok!(scheduler, store, fixture, "drive remediation phase");
        assert!(
            matches!(
                status,
                SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
            ),
            "unexpected remediation status: {status:?}"
        );
        if derive_fixture_saga(&fixture, store.projection_snapshot()).run_mode
            == store::RunMode::Compensated
        {
            break;
        }
    }
    let remediation_order = remediation_intent_forward_links(&store, &fixture.run_id);
    assert_eq!(
        remediation_order,
        vec![forward_b_pair.clone(), forward_a_pair.clone()]
    );
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::Compensated);
    for forward_ledger in [forward_a_ledger, forward_b_ledger] {
        let forward_pair = forward_pair_for_ledger(&store.projection_snapshot(), &forward_ledger);
        let obligation = saga
            .obligations
            .get(&forward_pair)
            .expect("forward obligation");
        assert_eq!(
            obligation.classification,
            store::ForwardLedgerClassification::Owed
        );
        assert!(
            obligation
                .remediation
                .as_ref()
                .expect("remediation ledger")
                .closed
        );
    }
    for _ in 0..8 {
        if store.projection_snapshot().run_state(&fixture.run_id) == store::RunState::Completed {
            break;
        }
        let status = drive_ok!(scheduler, store, fixture, "resolve compensated terminal");
        assert!(matches!(
            status,
            SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
        ));
    }
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::Compensated
    ));
    assert_drive!(
        scheduler,
        store,
        fixture,
        PublicOutputProjected,
        "completed compensated run"
    );
}

#[tokio::test]
async fn runtime_compensated_saga_resume_boundaries_do_not_duplicate_mutations() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let remediation_a = fixture
        .runtime_spec
        .spec()
        .remediations
        .get(&forward_a.node_id)
        .expect("remediation a")
        .clone();
    let remediation_b = fixture
        .runtime_spec
        .spec()
        .remediations
        .get(&forward_b.node_id)
        .expect("remediation b")
        .clone();
    let remediation_a_output = effective_output_cell_for_node(&fixture, &remediation_a);
    let remediation_b_output = effective_output_cell_for_node(&fixture, &remediation_b);
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut scheduler = compensated_saga_scheduler(&fixture);
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    drive_until_side_effect_confirmation_without_output(
        &scheduler,
        &mut store,
        &fixture,
        &forward_a,
        &forward_a_output,
        "forward a confirmation before output",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output.clone(), forward_b_output.clone()],
        "forward outputs",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);
    let forward_a_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_a.node_id);
    let forward_b_ledger =
        forward_ledger_for_node(&store.projection_snapshot(), &forward_b.node_id);
    let forward_a_pair = forward_pair_for_ledger(&store.projection_snapshot(), &forward_a_ledger);
    let forward_b_pair = forward_pair_for_ledger(&store.projection_snapshot(), &forward_b_ledger);

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::Remediating);
    assert_eq!(saga.obligations.len(), 2);

    scheduler = compensated_saga_scheduler(&fixture);
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);
    drive_until_remediation_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_b_pair,
        RemediationPhaseCheckpoint::SubmissionObserved,
        "first remedial submission",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_remediation_phase(
        &scheduler,
        &mut store,
        &fixture,
        &forward_b_pair,
        RemediationPhaseCheckpoint::ConfirmationObserved,
        "first remedial confirmation",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&remediation_b_output)
        .is_none());
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        std::slice::from_ref(&remediation_b_output),
        "first remedial output",
    )
    .await;
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    drive_until_compensated_before_terminal(&scheduler, &mut store, &fixture).await;
    assert!(matches!(
        remediation_projection_for_forward_pair(store.projection_snapshot(), &forward_a_pair)
            .expect("second remediation projection")
            .phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ));
    assert!(store
        .projection_snapshot()
        .cell_terminal(&remediation_a_output)
        .is_none());
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);

    scheduler = compensated_saga_scheduler(&fixture);
    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        std::slice::from_ref(&remediation_a_output),
        "second remedial output",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&remediation_a_output)
        .is_some());
    assert_no_duplicate_side_effect_submissions(&store, &fixture.run_id);
    assert_eq!(
        derive_fixture_saga(&fixture, store.projection_snapshot()).run_mode,
        store::RunMode::Compensated
    );
    assert_ne!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );

    scheduler = compensated_saga_scheduler(&fixture);
    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "resolve compensated terminal after resume"
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::Compensated
    ));
    assert_drive!(
        scheduler,
        store,
        fixture,
        PublicOutputProjected,
        "completed compensated run"
    );

    assert_eq!(
        remediation_intent_forward_links(&store, &fixture.run_id),
        vec![forward_b_pair.clone(), forward_a_pair.clone()]
    );
    for forward_ledger in [&forward_a_ledger, &forward_b_ledger] {
        assert_eq!(
            side_effect_submission_count_for_ledger(&store, &fixture.run_id, forward_ledger),
            1
        );
        assert_eq!(
            remediation_submission_count_for_forward_pair(
                &store,
                &fixture.run_id,
                &forward_pair_for_ledger(&store.projection_snapshot(), forward_ledger)
            ),
            1
        );
    }
}

#[tokio::test]
async fn runtime_resolves_clean_failure_without_acdc_claim() {
    let fixture = fixture();
    let failure_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::FailedWithoutAcdcClaim);
    assert!(saga.obligations.is_empty());

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "resolve clean failure terminal"
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_materializes_confirmed_forward_output_before_failed_without_claim_terminal() {
    let fixture =
        fixture_with_independent_second_node_and_first_exclusive_finalized_side_effect_state();
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_output = effective_output_cell_for_node(&fixture, &forward_node);
    let failure_node = node_by_output(&fixture, &fixture.cell_b).clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(&fixture),
        ))
        .expect("binding side effect");
    register_read_external_fixture_runner(&mut registry, &fixture);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    let (forward_attempt, ledger) = append_synthetic_exclusive_started(
        &mut store,
        &fixture,
        &fixture.run_id,
        &forward_node,
        "wallet-confirmed-forward-output-before-failure",
        "sidefx-confirmed-forward-output-before-failure",
    );
    append_synthetic_submission_observed(
        &mut store,
        &fixture,
        &forward_node,
        &forward_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-submission",
    );
    append_synthetic_submit_boundary_skipped(
        &mut store,
        &fixture,
        &forward_node,
        &forward_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-submit-boundary",
    );
    let verify_node = side_effect_verify_node_for_submit(&fixture, &forward_node).clone();
    let verify_attempt = append_attempt_start(&mut store, &fixture, &verify_node, 1);
    append_synthetic_verify_receipt_observed(
        &mut store,
        &fixture,
        &forward_node,
        &verify_node,
        &verify_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-receipt",
    );
    append_synthetic_verify_confirmation_observed(
        &mut store,
        &fixture,
        &forward_node,
        &verify_node,
        &verify_attempt,
        &ledger,
        "sidefx-confirmed-forward-output-before-failure-confirmation",
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_output)
        .is_none());
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        &fixture.run_id,
        &projection_snapshot,
        &forward_node,
        &forward_attempt,
    )
    .expect("side-effect projection lookup")
    .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ));

    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::FailedWithoutAcdcClaim);

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "materialize confirmed forward output"
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_output)
        .is_some());
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "resolve failed-without-claim terminal"
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn runtime_resolves_manual_resolution_terminal() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let mut store = TestTypedRunStore::new();
    let artifact_store = Arc::new(store.clone());
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry.clone(), &fixture),
        artifact_store.clone(),
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_drive!(scheduler, store, fixture, Advanced, "advance to ambiguity");
    }
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::ManualBlocked);
    assert_eq!(
        saga.manual_block_reason,
        Some(store::ManualBlockReason::PolicyManualResolution)
    );

    append_manual_resolution(
        &scheduler,
        &mut store,
        &fixture,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::ManuallyResolved);

    let fresh_scheduler = SerialTypedScheduler::new(
        register_fixture_capabilities(registry, &fixture),
        artifact_store,
    );
    assert_drive!(
        fresh_scheduler,
        store,
        fixture,
        Advanced,
        "resolve manual terminal"
    );
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::ManuallyResolved
    ));
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_prefix_with_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    for _ in 0..2 {
        assert_drive!(
            scheduler,
            store,
            fixture,
            Advanced,
            "advance to manual block"
        );
    }
    let saga = derive_fixture_saga(&fixture, store.projection_snapshot());
    assert_eq!(saga.run_mode, store::RunMode::ManualBlocked);

    let node_b = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("node b")
        .clone();
    append_attempt_start(&mut store, &fixture, &node_b, 2);

    let spec::SagaPolicySpec::ManualResolution { manual } = &fixture.runtime_spec.spec().saga
    else {
        panic!("manual resolution fixture policy");
    };
    let error = build_manual_resolution_prefix_authority_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store,
        manual.clone(),
    )
    .expect_err("manual prefix rejects open attempt");
    assert!(
        matches!(&error, RuntimeError::InvalidRunStream(message) if message.contains("requires no open semantic attempts")),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_missing_manual_terminal_authorization_artifact_leaves_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let mut store = TestTypedRunStore::new();
    let artifact_store = Arc::new(FilteringRuntimeArtifactStore::new(store.clone()));
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry, &fixture),
        artifact_store.clone(),
    );
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");

    for _ in 0..2 {
        assert_drive!(scheduler, store, fixture, Advanced, "advance to ambiguity");
    }
    append_manual_resolution(
        &scheduler,
        &mut store,
        &fixture,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let manual = store
        .load_run_stream(&fixture.run_id)
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload.clone()),
            _ => None,
        })
        .expect("manual resolution recorded");
    artifact_store.hide_artifact(manual.authorization_artifact_id.clone());
    let stream_len_before = store.load_run_stream(&fixture.run_id).len();

    let resolve_node = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .expect("resolve saga terminal node");
    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture)
            .await,
        Err(RuntimeError::Store(message)) if message.contains("missing artifact")
    ));
    store.assert_run_stream_len(&fixture.run_id, stream_len_before + 1);
    let resolve_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &resolve_node.node_id,
        1,
    )
    .expect("resolve attempt id");
    assert!(matches!(
        store
            .projection_snapshot()
            .attempt(&resolve_node.node_id, &resolve_attempt)
            .expect("open resolve attempt")
            .status,
        store::AttemptStatus::Started { .. }
    ));
    assert!(store
        .projection_snapshot()
        .run_completion(&fixture.run_id)
        .is_none());
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_before_manual_blocked() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let evidence_bytes = br#"{"operator_note":"too_early"}"#.to_vec();

    let error = record_manual_resolution(
        &scheduler,
        &mut store,
        &fixture.runtime_spec,
        &fixture.run_id,
        ManualResolutionRequest {
            outcome: events::ManualResolutionOutcome::ConfirmRemediated,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes: br#"{}"#.to_vec(),
            note: None,
        },
    )
    .await
    .expect_err("manual resolution before block rejects");

    assert!(
        matches!(error, RuntimeError::InvalidRunStream(_)),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_rejects_forward_node_emitting_remediation_ledger_purpose() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            ForwardEmitsRemediationPurposeRunner,
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, &fixture);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize wrong forward ledger purpose"
    );
}

#[tokio::test]
async fn runtime_rejects_remediation_node_emitting_forward_ledger_purpose() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "pure",
            BlockingRunner,
        ))
        .expect("binding failure node");
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output, forward_b_output],
        "drive forward side-effect phase",
    )
    .await;
    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize wrong remediation ledger purpose"
    );
    assert_failure_code_count(&store, "runner_output_invalid", 1);
}

#[tokio::test]
async fn side_effect_not_submitted_resume_completes_submit_boundary() {
    let fixture = fixture_with_first_side_effect_state();
    let (scheduler, mut store) = started_side_effect_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "prepare and start side effect");

    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    append_not_submitted_proven(&mut store, &fixture, node, &attempt_id, 1);

    drive_ok!(scheduler, store, fixture, "resume not-submitted");
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        &fixture.run_id,
        &projection_snapshot,
        node,
        &attempt_id,
    )
    .expect("projection lookup")
    .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        store::SideEffectPhase::NotSubmittedProven { .. }
    ));
    assert!(matches!(
        projection_snapshot.cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Skipped { .. })
    ));
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
}

#[tokio::test]
async fn side_effect_staged_artifact_must_match_payload_ledger_binding() {
    struct WrongLedgerStagedSideEffectRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    impl ErasedNodeRunner for WrongLedgerStagedSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let ledger = side_effect_ledger_key_for_ctx(&ctx);
                let ledger_purpose = side_effect_ledger_purpose_for_ctx(&ctx);
                let (pair_id, pair_role) = side_effect_pair_fields_for_ctx(
                    &ctx,
                    &ledger_purpose,
                    events::SideEffectPairRole::Submit,
                );
                let staged_ledger =
                    events::SideEffectLedgerKey::new("wrong-ledger").expect("ledger key");
                let intent_bytes = br#"{"intent":"wrong-ledger"}"#.to_vec();
                let intent_hash = digest_for_bytes(&intent_bytes);
                let intent_artifact_id =
                    ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
                let evidence = store::ArtifactEvidenceRef {
                    artifact_id: intent_artifact_id.clone(),
                    digest: intent_hash.clone(),
                    byte_len: intent_bytes.len() as u64,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.node().config_ref.schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::SideEffectIntent,
                };
                let staged_artifact = StagedArtifact::inline_side_effect_artifact(
                    &ctx,
                    intent_bytes,
                    evidence,
                    staged_ledger,
                    1,
                )?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: vec![
                        RunnerEventPayload::SideEffectIntentPersisted(
                            events::side_effect::IntentPersisted {
                                spec_hash: ctx.spec_hash().clone(),
                                node_id: ctx.node().node_id.clone(),
                                scope_id: ctx.node().scope_id.clone(),
                                attempt_id: ctx.attempt_id().clone(),
                                ledger_key: ledger.clone(),
                                ledger_purpose,
                                pair_id,
                                pair_role,
                                invocation_epoch: 1,
                                intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                                intent_hash,
                                intent_artifact_id,
                                idempotency_input_schema_id: ctx
                                    .node()
                                    .config_ref
                                    .schema_id
                                    .clone(),
                                idempotency_input_hash: content(0xc3),
                                idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                                    .expect("idempotency key"),
                                capability_kind: self.cap_kind.clone(),
                                capability_version: self.cap_version.clone(),
                                adapter_kind: self.adapter_kind.clone(),
                                adapter_version: self.adapter_version.clone(),
                            },
                        ),
                        side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                        side_effect_prepared(&ctx, ledger, 1, 1),
                    ],
                })
            })
        }
    }

    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            WrongLedgerStagedSideEffectRunner {
                cap_kind: side_effect_capability_kind(),
                cap_version: side_effect_capability_version(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
            },
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, &fixture);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize side-effect artifact binding mismatch"
    );
}

#[tokio::test]
async fn side_effect_ambiguous_phase_blocks_resume() {
    let fixture = fixture_with_first_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    for _ in 0..3 {
        assert_drive!(scheduler, store, fixture, Advanced, "advance to ambiguity");
    }
    assert_drive!(
        scheduler,
        store,
        fixture,
        PublicOutputProjected,
        "ambiguous side effect resolves terminal"
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn side_effect_ambiguity_blocks_independent_ready_nodes() {
    let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let forward_node = node_by_output(&fixture, &fixture.cell_a).clone();
    append_or_get_first_attempt(&mut store, &fixture, &forward_node);

    for _ in 0..3 {
        drive_ok!(scheduler, store, fixture, "advance to ambiguity");
    }
    for _ in 0..8 {
        let status = drive_ok!(scheduler, store, fixture, "ambiguity resolves terminal");
        assert!(
            matches!(
                status,
                SchedulerStatus::Advanced | SchedulerStatus::PublicOutputProjected
            ),
            "unexpected ambiguity terminal status: {status:?}"
        );
        if store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .is_some()
        {
            break;
        }
    }
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_none());
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&fixture.run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[tokio::test]
async fn side_effect_output_before_terminal_evidence_is_rejected() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            PrematureSideEffectOutputRunner {
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, &fixture);
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize premature side-effect output"
    );
}

#[tokio::test]
async fn receipt_policy_allows_verify_output_after_receipt() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let submit_node = node_by_output(&fixture, &fixture.cell_a);
    let submit_attempt_id = append_synthetic_exclusive_receipt_phase(
        &mut store,
        &fixture,
        submit_node,
        "wallet-submit-output-receipt",
        "sidefx-submit-output-receipt",
    );
    let verify_node = side_effect_verify_node_for_submit(&fixture, submit_node);
    let verify_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &verify_node.node_id,
        1,
    )
    .expect("verify attempt id");

    let snapshot = store.projection_snapshot();
    side_effect_lifecycle::SideEffectLifecycle::validate_terminal_batch_evidence(
        &fixture.runtime_spec,
        &fixture.run_id,
        &snapshot,
        verify_node,
        &verify_attempt_id,
        false,
    )
    .expect("receipt policy permits submit output after receipt");

    append_terminal(
        &mut store,
        &fixture,
        verify_node,
        &verify_attempt_id,
        artifact(0xe1),
        content(0xe2),
    );
    validate_runtime_stream_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store.load_run_stream(&fixture.run_id),
    )
    .expect("historical validation permits receipt-terminal verify output");
    assert!(store
        .projection_snapshot()
        .attempt(&submit_node.node_id, &submit_attempt_id)
        .is_some());
}

#[tokio::test]
async fn finalized_policy_rejects_verify_output_after_receipt_before_confirmation() {
    let fixture = runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::Exclusive,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    );
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;
    let submit_node = node_by_output(&fixture, &fixture.cell_a);
    append_synthetic_exclusive_receipt_phase(
        &mut store,
        &fixture,
        submit_node,
        "wallet-submit-output-finalized",
        "sidefx-submit-output-finalized",
    );
    let verify_node = side_effect_verify_node_for_submit(&fixture, submit_node);
    let verify_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &verify_node.node_id,
        1,
    )
    .expect("verify attempt id");

    let snapshot = store.projection_snapshot();
    let error = side_effect_lifecycle::SideEffectLifecycle::validate_terminal_batch_evidence(
        &fixture.runtime_spec,
        &fixture.run_id,
        &snapshot,
        verify_node,
        &verify_attempt_id,
        false,
    )
    .expect_err("finalized policy requires confirmation");
    assert!(error
        .to_string()
        .contains("produced output before certified terminal evidence"));

    append_terminal(
        &mut store,
        &fixture,
        verify_node,
        &verify_attempt_id,
        artifact(0xe3),
        content(0xe4),
    );
    let error = validate_runtime_stream_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store.load_run_stream(&fixture.run_id),
    )
    .expect_err("historical validation requires finalized confirmation");
    assert!(error
        .to_string()
        .contains("produced output before certified terminal evidence"));
}

#[test]
fn side_effect_failure_derives_attempt_failure_payload() {
    let fixture = fixture_with_first_side_effect_state();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let ledger_purpose = side_effect_ledger_purpose();
    let (pair_id, pair_role) = side_effect_pair_fields_for_purpose(
        &fixture.runtime_spec,
        &node.node_id,
        &ledger_purpose,
        events::SideEffectPairRole::Verify,
    );
    let payloads = runner_payloads_with_derived_lifecycle(
        &fixture.runtime_spec,
        node,
        &attempt_id,
        vec![RunnerEventPayload::SideEffectFailed(
            events::side_effect::Failed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: side_effect_ledger_key(1),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                retryable: false,
                error: side_effect_error(false),
            },
        )],
    );
    let payloads = payloads.expect("derive lifecycle");
    assert!(
        payloads.iter().any(|payload| {
            matches!(
                payload,
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    retryable: false,
                    ..
                })
            )
        }),
        "side-effect failure payloads should include middleware-derived StateAttemptFailed"
    );
}

#[test]
fn side_effect_ambiguity_derives_attempt_failure_payload() {
    let fixture = fixture_with_first_side_effect_state();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let ledger_purpose = side_effect_ledger_purpose();
    let (pair_id, pair_role) = side_effect_pair_fields_for_purpose(
        &fixture.runtime_spec,
        &node.node_id,
        &ledger_purpose,
        events::SideEffectPairRole::Verify,
    );
    let payloads = runner_payloads_with_derived_lifecycle(
        &fixture.runtime_spec,
        node,
        &attempt_id,
        vec![RunnerEventPayload::SideEffectAmbiguous(
            events::side_effect::Ambiguous {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: side_effect_ledger_key(1),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                    .expect("ambiguity code"),
                evidence_schema_id: node.config_ref.schema_id.clone(),
                evidence_hash: content(0xca),
                evidence_artifact_id: artifact(0xcb),
            },
        )],
    );
    let payloads = payloads.expect("derive lifecycle");
    let failure = payloads
        .iter()
        .find_map(|payload| match payload {
            events::KernelEventPayload::StateAttemptFailed(payload) => Some(payload),
            _ => None,
        })
        .expect("derived attempt failure");
    assert!(!failure.retryable);
    assert_eq!(failure.error.code.as_str(), "side_effect_ambiguous");
}

#[derive(Clone)]
struct DriverSideEffectRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl DriverSideEffectRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }

    fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.callbacks = self.callbacks.with_submission_decision(decision);
        self
    }
}

impl ErasedNodeRunner for DriverSideEffectRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let Some(resource_key) = test_driver_resource_key_for_node(ctx.node()) else {
                return Ok(ErasedRunnerOutput::new(Vec::new()));
            };
            let plan = self.callbacks.intent_plan_for(
                ctx.node().node_id.as_str().to_owned(),
                ctx.attempt_id().as_str().to_owned(),
            )?;
            SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
                &plan.intent,
                &plan.idempotency,
                plan.idempotency_key,
                plan.capability_binding,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { SideEffectDriver::drive(ctx, &self.callbacks).await })
    }
}

#[derive(Clone)]
struct DriverSideEffectVerifyRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl DriverSideEffectVerifyRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }
}

impl ErasedNodeRunner for DriverSideEffectVerifyRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { SideEffectVerifyDriver::drive(ctx, &self.callbacks).await })
    }
}

#[derive(Clone)]
struct FailingAfterPreclaimRunner {
    callbacks: TestSideEffectDriverCallbacks,
}

impl FailingAfterPreclaimRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
        }
    }
}

impl ErasedNodeRunner for FailingAfterPreclaimRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        Box::pin(async move {
            let Some(resource_key) = test_driver_resource_key_for_node(ctx.node()) else {
                return Ok(ErasedRunnerOutput::new(Vec::new()));
            };
            let plan = self.callbacks.intent_plan_for(
                ctx.node().node_id.as_str().to_owned(),
                ctx.attempt_id().as_str().to_owned(),
            )?;
            SideEffectLanePreclaimBuilder::new(ctx).claim_resource_lane(
                &plan.intent,
                &plan.idempotency,
                plan.idempotency_key,
                plan.capability_binding,
                resource_key,
            )
        })
    }

    fn run_erased<'a>(&'a self, _ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            Err(RuntimeError::InvalidRunnerOutput(
                "prepare invocation failed after exclusive resource claim".to_owned(),
            ))
        })
    }
}

struct PrePreparedSideEffectRunner {
    emit_claim: bool,
}

impl ErasedNodeRunner for PrePreparedSideEffectRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            if side_effect_projection_for_attempt(
                ctx.runtime_spec(),
                ctx.run_id(),
                ctx.projections(),
                ctx.node(),
                ctx.attempt_id(),
            )?
            .is_some()
            {
                return Err(RuntimeError::Blocked(
                    "pre-prepared side-effect runner should not resume".to_owned(),
                ));
            }
            let ledger_purpose = side_effect_ledger_purpose_for_ctx(&ctx);
            let SideEffectFixtureIntentOutput {
                ledger,
                staged_artifact,
                payload,
            } = side_effect_fixture_intent_output(&ctx, ledger_purpose, 1)?;
            let mut payloads = vec![payload];
            if self.emit_claim {
                payloads.push(side_effect_claimed(&ctx, ledger, 1, 1));
            }
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: Vec::new(),
                payloads,
            })
        })
    }
}

#[derive(Clone)]
struct TouchedSetSideEffectVerifyRunner {
    callbacks: TestSideEffectDriverCallbacks,
    receipt: TouchedSetEmission,
    confirmation: TouchedSetEmission,
}

#[derive(Clone, Copy)]
enum TouchedSetEmission {
    None,
    MatchPayloadSchema,
}

impl TouchedSetSideEffectVerifyRunner {
    fn with_receipt(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
            receipt: TouchedSetEmission::MatchPayloadSchema,
            confirmation: TouchedSetEmission::None,
        }
    }

    fn with_confirmation(fixture: &Fixture) -> Self {
        Self {
            callbacks: TestSideEffectDriverCallbacks::new(fixture),
            receipt: TouchedSetEmission::None,
            confirmation: TouchedSetEmission::MatchPayloadSchema,
        }
    }
}

impl ErasedNodeRunner for TouchedSetSideEffectVerifyRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let mut output = SideEffectVerifyDriver::drive(ctx, &self.callbacks).await?;
            for payload in &mut output.payloads {
                match payload {
                    RunnerEventPayload::SideEffectReceiptObserved(payload) => {
                        payload.resource_touched_set = touched_set_for_emission(
                            self.receipt,
                            &payload.receipt_schema_id,
                            &payload.receipt_hash,
                            &payload.receipt_artifact_id,
                        );
                    }
                    RunnerEventPayload::SideEffectConfirmationObserved(payload) => {
                        payload.resource_touched_set = touched_set_for_emission(
                            self.confirmation,
                            &payload.confirmation_schema_id,
                            &payload.confirmation_hash,
                            &payload.confirmation_artifact_id,
                        );
                    }
                    _ => {}
                }
            }
            Ok(output)
        })
    }
}

struct FailActiveSideEffectAfterSagaRunner {
    inner: DriverSideEffectRunner,
    stop_before_invocation_started: bool,
}

impl FailActiveSideEffectAfterSagaRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            stop_before_invocation_started: false,
        }
    }

    fn before_invocation_started(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
            stop_before_invocation_started: true,
        }
    }

    fn with_submission_decision(mut self, decision: TestSubmissionDecision) -> Self {
        self.inner = self.inner.with_submission_decision(decision);
        self
    }
}

impl ErasedNodeRunner for FailActiveSideEffectAfterSagaRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        self.inner.preclaim_resource_lane(ctx)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let terminal_policies = runtime_spec_terminal_policies(ctx.runtime_spec());
            let saga = ctx
                .projections()
                .derive_saga_projection(
                    ctx.run_id(),
                    &ctx.runtime_spec().spec().saga,
                    &terminal_policies,
                )
                .expect("saga projection");
            let projected = side_effect_projection_for_attempt(
                ctx.runtime_spec(),
                ctx.run_id(),
                ctx.projections(),
                ctx.node(),
                ctx.attempt_id(),
            )?
            .map(|projection| (projection.ledger_key.clone(), projection.phase.clone()));
            if saga.engagement.is_some() {
                if let Some((ledger, phase)) = &projected {
                    if let Some((invocation_epoch, failure_phase)) =
                        saga_closure_failure_for_phase(phase)
                    {
                        return Ok(ErasedRunnerOutput::new(vec![side_effect_failed(
                            &ctx,
                            ledger.clone(),
                            invocation_epoch,
                            failure_phase,
                            false,
                        )]));
                    }
                }
            }
            if self.stop_before_invocation_started && projected.is_none() {
                return prepared_boundary_side_effect_output(ctx);
            }
            self.inner.run_erased(ctx).await
        })
    }
}

fn prepared_boundary_side_effect_output(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let ledger_purpose = side_effect_ledger_purpose_for_ctx(&ctx);
    let SideEffectFixtureIntentOutput {
        ledger,
        staged_artifact,
        payload,
    } = side_effect_fixture_intent_output(&ctx, ledger_purpose, 1)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: Vec::new(),
        payloads: vec![
            payload,
            side_effect_claimed(&ctx, ledger.clone(), 1, 1),
            side_effect_prepared(&ctx, ledger, 1, 1),
        ],
    })
}

fn saga_closure_failure_for_phase(
    phase: &store::SideEffectPhase,
) -> Option<(u32, events::side_effect::FailurePhase)> {
    match phase {
        store::SideEffectPhase::IntentPersisted { invocation_epoch }
        | store::SideEffectPhase::Claimed {
            invocation_epoch, ..
        }
        | store::SideEffectPhase::InvocationPrepared {
            invocation_epoch, ..
        } => Some((
            *invocation_epoch,
            events::side_effect::FailurePhase::BeforeInvocationStarted,
        )),
        store::SideEffectPhase::NotSubmittedProven { invocation_epoch } => Some((
            *invocation_epoch,
            events::side_effect::FailurePhase::AfterNotSubmittedProven,
        )),
        _ => None,
    }
}

fn touched_set_for_emission(
    emission: TouchedSetEmission,
    evidence_schema_id: &SchemaId,
    evidence_hash: &ContentDigest,
    evidence_artifact_id: &ArtifactId,
) -> Option<events::ResourceTouchedSetEvidence> {
    match emission {
        TouchedSetEmission::None => None,
        TouchedSetEmission::MatchPayloadSchema => Some(events::ResourceTouchedSetEvidence {
            namespace: exact_touched_set_resource_namespace(),
            evidence_schema_id: evidence_schema_id.clone(),
            evidence_hash: evidence_hash.clone(),
            evidence_artifact_id: evidence_artifact_id.clone(),
        }),
    }
}

struct ForwardEmitsRemediationPurposeRunner;

impl ErasedNodeRunner for ForwardEmitsRemediationPurposeRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            side_effect_intent_with_purpose(
                ctx,
                events::SideEffectLedgerPurpose::Remediation {
                    forward_pair_id: synthetic_side_effect_pair_id(0x84),
                },
            )
        })
    }
}

struct RemediationEmitsForwardPurposeRunner {
    inner: DriverSideEffectRunner,
}

impl RemediationEmitsForwardPurposeRunner {
    fn new(fixture: &Fixture) -> Self {
        Self {
            inner: DriverSideEffectRunner::new(fixture),
        }
    }
}

impl ErasedNodeRunner for RemediationEmitsForwardPurposeRunner {
    fn preclaim_resource_lane<'a>(
        &'a self,
        ctx: &'a PreInvocationRunCtx<'a>,
    ) -> PreInvocationRunnerFuture<'a> {
        self.inner.preclaim_resource_lane(ctx)
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            if ctx
                .runtime_spec()
                .forward_node_for_remediation(&ctx.node().node_id)
                .is_some()
            {
                side_effect_intent_with_purpose(ctx, events::SideEffectLedgerPurpose::Forward)
            } else {
                self.inner.run_erased(ctx).await
            }
        })
    }
}

fn side_effect_intent_with_purpose(
    ctx: ErasedRunCtx<'_>,
    ledger_purpose: events::SideEffectLedgerPurpose,
) -> Result<ErasedRunnerOutput> {
    let SideEffectFixtureIntentOutput {
        staged_artifact,
        payload,
        ..
    } = side_effect_fixture_intent_output(&ctx, ledger_purpose, 1)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![staged_artifact],
        staged_retention_refs: Vec::new(),
        payloads: vec![payload],
    })
}

struct PrematureSideEffectOutputRunner {
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
}

impl ErasedNodeRunner for PrematureSideEffectOutputRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let artifact = state_output_artifact(
                ctx.node(),
                ctx.descriptor(),
                self.output_artifact.clone(),
                self.output_digest.clone(),
            );
            let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: Vec::new(),
                payloads: terminal_payloads(
                    &ctx,
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                ),
            })
        })
    }
}

struct StaleStreamStore<'a> {
    inner: RefCell<&'a mut TestTypedRunStore>,
    stream: Vec<store::KernelEventEnvelope>,
}

impl<'a> StaleStreamStore<'a> {
    fn new(inner: &'a mut TestTypedRunStore, stream: Vec<store::KernelEventEnvelope>) -> Self {
        Self {
            inner: RefCell::new(inner),
            stream,
        }
    }
}

impl store::RunEventStore for StaleStreamStore<'_> {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = block_on_ready(
            self.inner
                .borrow_mut()
                .append_prepared_commit_bundle(bundle),
        );
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        let _ = run_id;
        Box::pin(std::future::ready(Ok(self.stream.clone())))
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        let result = store::CommittedRunStream::from_events(run_id.clone(), self.stream.clone());
        Box::pin(std::future::ready(result))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        let result = Ok(self.inner.borrow().expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }
}

delegate_execution_claim_store_to_refcell_inner!(StaleStreamStore<'_>);

struct MissingInputArtifactRefStore<'a> {
    inner: RefCell<&'a mut TestTypedRunStore>,
    producer_node_id: NodeId,
}

impl<'a> MissingInputArtifactRefStore<'a> {
    fn new(inner: &'a mut TestTypedRunStore, producer_node_id: NodeId) -> Self {
        Self {
            inner: RefCell::new(inner),
            producer_node_id,
        }
    }
}

impl store::RunEventStore for MissingInputArtifactRefStore<'_> {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = block_on_ready(
            self.inner
                .borrow_mut()
                .append_prepared_commit_bundle(bundle),
        );
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        let result = rewrite_stream_without_payloads(
            &self.inner.borrow().load_run_stream(run_id),
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::ArtifactReferenced(payload)
                        if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                            && payload.node_id.as_ref() == Some(&self.producer_node_id)
                )
            },
        );
        Box::pin(std::future::ready(Ok(result)))
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        let stream = rewrite_stream_without_payloads(
            &self.inner.borrow().load_run_stream(run_id),
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::ArtifactReferenced(payload)
                        if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                            && payload.node_id.as_ref() == Some(&self.producer_node_id)
                )
            },
        );
        let result = store::CommittedRunStream::from_events(run_id.clone(), stream);
        Box::pin(std::future::ready(result))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        let result = Ok(self.inner.borrow().expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }
}

delegate_execution_claim_store_to_refcell_inner!(MissingInputArtifactRefStore<'_>);

fn rewrite_envelope(
    event: &store::KernelEventEnvelope,
    seq: store::StreamSeq,
    ordinal: store::CommitOrdinal,
    commit_key: store::CommitKey,
) -> store::KernelEventEnvelope {
    store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
        event_id: test_event_id_for_envelope_inputs(
            event.run_id(),
            seq,
            ordinal,
            event.event_schema_id(),
            event.payload_hash(),
        ),
        event_schema_id: event.event_schema_id().clone(),
        run_id: event.run_id().clone(),
        seq,
        ordinal,
        spec_hash: event.spec_hash().clone(),
        commit_key,
        logical_key: event.logical_key().clone(),
        payload_hash: event.payload_hash().clone(),
        payload: event.payload().clone(),
    })
    .expect("rewritten envelope")
}

fn rewrite_envelope_payload(
    event: &store::KernelEventEnvelope,
    payload: events::KernelEventPayload,
) -> store::KernelEventEnvelope {
    let payload_hash = store::payload_canonical_json(&payload)
        .expect("payload canonical")
        .content_digest();
    let event_schema_id = payload.event_schema_id().expect("event schema");
    store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
        event_id: test_event_id_for_envelope_inputs(
            event.run_id(),
            event.seq(),
            event.ordinal(),
            &event_schema_id,
            &payload_hash,
        ),
        event_schema_id,
        run_id: event.run_id().clone(),
        seq: event.seq(),
        ordinal: event.ordinal(),
        spec_hash: payload.spec_hash().clone(),
        commit_key: event.commit_key().clone(),
        logical_key: event.logical_key().clone(),
        payload_hash,
        payload,
    })
    .expect("rewritten envelope payload")
}

fn rewrite_stream_payloads<F>(
    stream: &[store::KernelEventEnvelope],
    mut rewrite: F,
) -> Vec<store::KernelEventEnvelope>
where
    F: FnMut(&events::KernelEventPayload) -> Option<events::KernelEventPayload>,
{
    stream
        .iter()
        .map(|event| {
            rewrite(event.payload())
                .map(|payload| rewrite_envelope_payload(event, payload))
                .unwrap_or_else(|| event.clone())
        })
        .collect()
}

fn rewrite_stream_without_payloads<F>(
    stream: &[store::KernelEventEnvelope],
    mut should_remove: F,
) -> Vec<store::KernelEventEnvelope>
where
    F: FnMut(&events::KernelEventPayload) -> bool,
{
    let mut rewritten = Vec::with_capacity(stream.len());
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        let payloads = commit
            .iter()
            .filter(|event| !should_remove(event.payload()))
            .map(|event| event.payload().clone())
            .collect::<Vec<_>>();
        assert!(
            !payloads.is_empty(),
            "test corruption helper must not remove an entire commit"
        );
        if payloads.len() == commit.len() {
            rewritten.extend(commit.iter().cloned());
        } else {
            let mut ordinal = 0;
            for event in commit {
                if should_remove(event.payload()) {
                    continue;
                }
                rewritten.push(rewrite_envelope(
                    event,
                    seq,
                    store::CommitOrdinal::new(ordinal),
                    commit_key.clone(),
                ));
                ordinal += 1;
            }
        }
        index = end;
    }
    rewritten
}

fn assert_every_certified_node_has_attempt(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) {
    let mut started = BTreeSet::new();
    let mut completed = BTreeSet::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                started.insert(payload.node_id.clone());
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                completed.insert(payload.node_id.clone());
            }
            _ => {}
        }
    }
    for node in &runtime_spec.spec().nodes {
        if matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) {
            continue;
        }
        assert!(
            started.contains(&node.node_id),
            "node {} has no StateAttemptStarted",
            node.node_id
        );
        assert!(
            completed.contains(&node.node_id),
            "node {} has no StateAttemptCompleted",
            node.node_id
        );
    }
}

fn referenced_artifact_ids_for_payload(payload: &events::KernelEventPayload) -> Vec<ArtifactId> {
    let mut artifacts = Vec::new();
    match payload {
        events::KernelEventPayload::RunAdmitted(payload) => {
            artifacts.push(payload.spec_artifact.artifact_id.clone());
            artifacts.push(payload.certificate_artifact.artifact_id.clone());
            artifacts.extend(
                payload
                    .config_artifacts
                    .iter()
                    .map(|artifact| artifact.artifact_id.clone()),
            );
            artifacts.extend(
                payload
                    .seed_cells
                    .iter()
                    .map(|seed| seed.seed_artifact.artifact_id.clone()),
            );
        }
        events::KernelEventPayload::FactRecorded(payload) => {
            artifacts.push(payload.claim.response().artifact_id().clone());
        }
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            artifacts.push(payload.artifact_ref.artifact_id.clone());
        }
        events::KernelEventPayload::CellProduced(payload) => {
            artifacts.push(payload.artifact_id.clone());
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            artifacts.extend(payload.cells.iter().map(|cell| cell.artifact_id.clone()));
            if let Some(artifact_id) = &payload.rendered_artifact_id {
                artifacts.push(artifact_id.clone());
            }
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            if let Some(evidence) = &payload.error.diagnostic_ref {
                artifacts.push(evidence.artifact_id.clone());
            }
        }
        events::KernelEventPayload::StateAttemptFailed(payload) => {
            if let Some(evidence) = &payload.error.diagnostic_ref {
                artifacts.push(evidence.artifact_id.clone());
            }
        }
        events::KernelEventPayload::ManualResolutionRecorded(payload) => {
            artifacts.push(payload.evidence_artifact_id.clone());
            artifacts.push(payload.authorization_artifact_id.clone());
        }
        events::KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
            events::RunCompletionOutcome::Completed(_) => {}
            events::RunCompletionOutcome::Compensated
            | events::RunCompletionOutcome::ManuallyResolved
            | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {}
        },
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            artifacts.push(payload.intent_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            if let Some(artifact_id) = &payload.prepared_artifact_id {
                artifacts.push(artifact_id.clone());
            }
        }
        events::KernelEventPayload::ResourceLaneClaimed(_)
        | events::KernelEventPayload::ResourceLaneClaimIntent(_)
        | events::KernelEventPayload::ResourceLaneReleased(_)
        | events::KernelEventPayload::ResourceLaneReleaseIntent(_) => {}
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            artifacts.push(payload.proof_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            artifacts.push(payload.submission_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            artifacts.push(payload.evidence_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
            artifacts.push(payload.receipt_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            artifacts.push(payload.confirmation_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            artifacts.push(payload.evidence_artifact_id.clone());
        }
        events::KernelEventPayload::SideEffectFailed(payload) => {
            if let Some(evidence) = &payload.error.diagnostic_ref {
                artifacts.push(evidence.artifact_id.clone());
            }
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => {
            artifacts.extend(
                payload
                    .refs
                    .iter()
                    .map(|reference| reference.artifact_id.clone()),
            );
        }
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            artifacts.push(payload.manifest_artifact_id.clone());
        }
        events::KernelEventPayload::StateAttemptStarted(_)
        | events::KernelEventPayload::StateAttemptInterrupted(_)
        | events::KernelEventPayload::CellSkipped(_)
        | events::KernelEventPayload::SideEffectClaimed(_)
        | events::KernelEventPayload::SideEffectClaimTakenOver(_)
        | events::KernelEventPayload::SideEffectInvocationStarted(_)
        | events::KernelEventPayload::StateAttemptCompleted(_) => {}
    }
    artifacts
}

fn prepare_fixture_launch(
    scheduler: &SerialTypedScheduler,
    store: &TestTypedRunStore,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<PreparedRunLaunch> {
    scheduler.prepare_run_launch(
        &fixture.runtime_spec,
        fixture_run_identity_material(fixture),
        run_start_evidence(fixture, seed_cells),
        store.expected_next_seq(&fixture.run_id),
    )
}

async fn start_fixture_run(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<store::CommitOutcome> {
    let launch = prepare_fixture_launch(scheduler, store, fixture, seed_cells)?;
    scheduler_start_run(scheduler, store, launch).await
}

async fn started_fixture_store(
    scheduler: &SerialTypedScheduler,
    fixture: &Fixture,
) -> TestTypedRunStore {
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        scheduler,
        &mut store,
        fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    store
}

async fn started_fixture_run(fixture: &Fixture) -> (SerialTypedScheduler, TestTypedRunStore) {
    let scheduler = test_scheduler(registered_fixture_runners(fixture));
    let store = started_fixture_store(&scheduler, fixture).await;
    (scheduler, store)
}

async fn started_fixture_run_with_registry(
    registry: ErasedRunnerRegistry,
    fixture: &Fixture,
) -> (SerialTypedScheduler, TestTypedRunStore) {
    let scheduler = fixture_scheduler(registry, fixture);
    let store = started_fixture_store(&scheduler, fixture).await;
    (scheduler, store)
}

async fn started_fixture_run_with_registry_and_fact_descriptors(
    registry: ErasedRunnerRegistry,
    fixture: &Fixture,
    fact_descriptors: &[mfm_facts::FactDescriptor],
) -> (SerialTypedScheduler, TestTypedRunStore) {
    let scheduler = fixture_scheduler(registry, fixture);
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .extend(fact_descriptors.iter().map(fact_descriptor_artifact));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("start run with fact descriptors");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("commit run with fact descriptors");
    (scheduler, store)
}

async fn started_side_effect_fixture_run(
    fixture: &Fixture,
) -> (SerialTypedScheduler, TestTypedRunStore) {
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(fixture));
    let store = started_fixture_store(&scheduler, fixture).await;
    (scheduler, store)
}

async fn start_fixture_run_async_store<S: store::RunEventStore + ?Sized>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<store::CommitOutcome> {
    let expected_next_seq = store
        .expected_next_seq(&fixture.run_id)
        .await
        .map_err(crate::error::async_store_error)?;
    let launch = scheduler.prepare_run_launch(
        &fixture.runtime_spec,
        fixture_run_identity_material(fixture),
        run_start_evidence(fixture, seed_cells),
        expected_next_seq,
    )?;
    scheduler.start_run(store, launch).await
}

async fn scheduler_start_run(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    launch: PreparedRunLaunch,
) -> Result<store::CommitOutcome> {
    scheduler.start_run(&*store, launch).await
}

async fn scheduler_start_run_admitted(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    launch: PreparedRunLaunch,
) -> Result<RunAdmissionAuthority> {
    scheduler
        .start_run_admitted(&*store, runtime_spec, launch)
        .await
}

async fn drive_once(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus> {
    drive_once_with_claim(scheduler, &*store, runtime_spec, run_id).await
}

async fn drive_fixture_once(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) -> Result<SchedulerStatus> {
    drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id).await
}

async fn assert_invalid_output_after(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    drive_count: usize,
    expected_message: &str,
) {
    for _ in 0..drive_count {
        assert_eq!(
            drive_fixture_once(scheduler, store, fixture)
                .await
                .expect("advance before invalid output"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        drive_fixture_once(scheduler, store, fixture).await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains(expected_message)
    ));
}

async fn drive_until_blocked(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus> {
    drive_until_blocked_with_claim(scheduler, &*store, runtime_spec, run_id).await
}

async fn drive_fixture_until_blocked(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) -> Result<SchedulerStatus> {
    drive_until_blocked(scheduler, store, &fixture.runtime_spec, &fixture.run_id).await
}

async fn drive_once_with_claim<S>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus>
where
    S: store::RunEventStore + store::ExecutionClaimStore + ?Sized,
{
    let token = execution_claim_token(store, run_id).await?;
    scheduler
        .drive_once(store, runtime_spec, run_id, token)
        .await
}

async fn drive_until_blocked_with_claim<S>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus>
where
    S: store::RunEventStore + store::ExecutionClaimStore + ?Sized,
{
    let token = execution_claim_token(store, run_id).await?;
    scheduler
        .drive_until_blocked(store, runtime_spec, run_id, token)
        .await
}

async fn execution_claim_token<S>(store: &S, run_id: &RunId) -> Result<store::AdmissionToken>
where
    S: store::ExecutionClaimStore + ?Sized,
{
    loop {
        match store
            .execution_claim_status(run_id)
            .await
            .map_err(crate::error::async_store_error)?
        {
            store::ExecutionClaimStatus::Live(lease) => return Ok(lease.token),
            store::ExecutionClaimStatus::Expired(lease) => {
                store
                    .reap_expired_execution_claim(run_id, &lease.token)
                    .await
                    .map_err(crate::error::async_store_error)?;
            }
            store::ExecutionClaimStatus::Unclaimed => {
                let token = store::AdmissionToken::new(format!(
                    "mfm.test.runtime.execution_claim:{run_id}"
                ))?;
                match store
                    .acquire_execution_claim(run_id, token)
                    .await
                    .map_err(crate::error::async_store_error)?
                {
                    store::NowaitSkipAdmissionResult::Admitted(lease) => return Ok(lease.token),
                    store::NowaitSkipAdmissionResult::Busy(_) => {}
                }
            }
        }
    }
}

async fn record_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    request: ManualResolutionRequest,
) -> Result<store::CommitOutcome> {
    scheduler
        .record_manual_resolution(&*store, runtime_spec, run_id, request)
        .await
}

fn build_manual_resolution_prefix_authority_for_tests(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    store: &TestTypedRunStore,
    manual: spec::ManualResolutionEvidenceSpec,
) -> Result<mfm_manual_auth::ManualResolutionPrefixAuthority> {
    let projection_snapshot = store.projection_snapshot();
    crate::manual_resolution::build_manual_resolution_prefix_authority_from_parts(
        runtime_spec,
        run_id,
        manual,
        &store.load_run_stream(run_id),
        store.expected_next_seq(run_id),
        &projection_snapshot,
    )
}

fn run_start_evidence(
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> RunLaunchEvidence {
    RunLaunchEvidence {
        entry_point: entry_point_launch_evidence(),
        spec_artifact: spec_artifact(&fixture.runtime_spec),
        certificate_artifact: certificate_artifact(&fixture.runtime_spec),
        config_artifacts: fixture
            .runtime_spec
            .spec()
            .config_refs
            .iter()
            .map(|config| config_artifact(&fixture.runtime_spec, config))
            .collect(),
        fact_descriptor_artifacts: Vec::new(),
        seed_cells: seed_cells.into_iter().map(seed_launch_cell).collect(),
    }
}

fn entry_point_launch_evidence() -> events::EntryPointLaunchEvidence {
    events::EntryPointLaunchEvidence {
        resolved_op_id: events::EntryPointOpId::new("mfm.test:portfolio_snapshot:1")
            .expect("entry-point op id"),
        entry_point_registry_digest: digest_for_bytes(b"entry-point-registry"),
    }
}

fn spec_artifact(runtime_spec: &CertifiedRuntimeSpec) -> RunLaunchArtifact {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .expect("canonical spec");
    let digest = canonical.content_digest();
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: runtime_spec.spec().media_type.clone(),
            schema_id: Some(spec::typed_execution_spec_schema_id().expect("typed spec schema")),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        },
    }
}

fn certificate_artifact(runtime_spec: &CertifiedRuntimeSpec) -> RunLaunchArtifact {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .expect("canonical certificate");
    let digest = canonical.content_digest();
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
                .expect("certificate media type"),
            schema_id: Some(
                mfm_certify::typed_spec_certificate_schema_id().expect("typed certificate schema"),
            ),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedSpecCertificate,
        },
    }
}

fn config_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    config: &spec::ConfigRef,
) -> RunLaunchArtifact {
    let bytes = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.config_ref == *config && node.framework.is_some())
        .and_then(|node| {
            node.framework.as_ref().map(|framework| {
                spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
                    .expect("framework config")
                    .to_vec()
            })
        })
        .unwrap_or_else(|| TEST_CONFIG_BYTES.to_vec());
    let bytes = if digest_for_bytes(&bytes) == config.digest {
        bytes
    } else {
        [
            CONFIG_MULTIPLIER_1_BYTES,
            CONFIG_MULTIPLIER_3_BYTES,
            CONFIG_MULTIPLIER_5_BYTES,
            CONFIG_MULTIPLIER_7_BYTES,
        ]
        .into_iter()
        .find(|candidate| digest_for_bytes(candidate) == config.digest)
        .expect("known test config bytes")
        .to_vec()
    };
    assert_eq!(digest_for_bytes(&bytes), config.digest);
    RunLaunchArtifact {
        bytes,
        evidence: store::ArtifactEvidenceRef {
            artifact_id: config.artifact_id.clone(),
            digest: config.digest.clone(),
            byte_len: config.byte_len,
            media_type: config.media_type.clone(),
            schema_id: Some(config.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        },
    }
}

fn fact_descriptor_artifact(descriptor: &mfm_facts::FactDescriptor) -> RunLaunchArtifact {
    let canonical =
        mfm_facts::canonical_fact_descriptor_bytes(descriptor).expect("canonical descriptor");
    let digest = mfm_facts::fact_descriptor_hash(descriptor).expect("descriptor hash");
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactDescriptor,
        },
    }
}

fn seed_launch_cell(seed: events::SeedCellRef) -> RunLaunchSeedCell {
    let bytes = if seed.digest == digest_for_bytes(TEST_SEED_BYTES) {
        TEST_SEED_BYTES.to_vec()
    } else if seed.digest == digest_for_bytes(CERTIFIER_SEED_BYTES) {
        CERTIFIER_SEED_BYTES.to_vec()
    } else {
        TEST_SEED_BYTES.to_vec()
    };
    RunLaunchSeedCell { bytes, cell: seed }
}

fn terminal_payloads(
    ctx: &ErasedRunCtx<'_>,
    output_artifact: ArtifactId,
    output_digest: ContentDigest,
) -> Vec<RunnerEventPayload> {
    vec![RunnerEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.node().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
        schema_id: ctx.descriptor().output_schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        artifact_id: output_artifact,
        content_digest: output_digest,
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    })]
}

fn fact_query_terminal_output(
    ctx: &ErasedRunCtx<'_>,
    state_evidence: &store::ArtifactEvidenceRef,
    staged_artifacts: Vec<StagedArtifact>,
    staged_retention_refs: Vec<StagedRetentionRefs>,
) -> ErasedRunnerOutput {
    ErasedRunnerOutput {
        staged_artifacts,
        staged_retention_refs,
        payloads: terminal_payloads(
            ctx,
            state_evidence.artifact_id.clone(),
            state_evidence.digest.clone(),
        ),
    }
}

fn prepare_runner_output_for_invocation(
    invocation: &PreparedRunnerInvocation<'_>,
    output: ErasedRunnerOutput,
) -> Result<crate::commit::PreparedRunnerOutput> {
    CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
        runtime_spec: invocation.runtime_spec,
        run_id: invocation.run_id,
        node: invocation.node,
        attempt_id: invocation.attempt_id,
        caps: &invocation.caps,
        recorded_facts: &invocation.recorded_facts,
        view: invocation.view,
        saga_terminal_proof: None,
        output,
    })
}

fn state_output_artifact(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

fn state_output_artifact_for_bytes(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    bytes: &[u8],
) -> store::ArtifactEvidenceRef {
    let digest = digest_for_bytes(bytes);
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

fn digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn runtime_staging_class(role: events::ArtifactRole) -> &'static str {
    match role.contract().staging {
        events::ArtifactStagingClass::AttemptStateOutput
        | events::ArtifactStagingClass::AttemptFactResponse
        | events::ArtifactStagingClass::AttemptFactQueryEvidence
        | events::ArtifactStagingClass::AttemptPublicOutput
        | events::ArtifactStagingClass::AttemptRedactedDiagnostic => {
            assert!(staged_artifact_binding_kind(role).is_some());
            assert!(staged_side_effect_artifact_phase(role).is_none());
        }
        events::ArtifactStagingClass::SideEffectIntent
        | events::ArtifactStagingClass::SideEffectPreparedInvocation
        | events::ArtifactStagingClass::SideEffectNotSubmittedProof
        | events::ArtifactStagingClass::SideEffectSubmission
        | events::ArtifactStagingClass::SideEffectSubmissionUnknown
        | events::ArtifactStagingClass::SideEffectReceipt
        | events::ArtifactStagingClass::SideEffectConfirmation
        | events::ArtifactStagingClass::SideEffectAmbiguity => {
            assert!(staged_artifact_binding_kind(role).is_none());
            assert!(staged_side_effect_artifact_phase(role).is_some());
        }
        events::ArtifactStagingClass::RunAdmission
        | events::ArtifactStagingClass::ManualResolution
        | events::ArtifactStagingClass::MiddlewareRetentionManifest => {
            assert!(staged_artifact_binding_kind(role).is_none());
            assert!(staged_side_effect_artifact_phase(role).is_none());
        }
    }
    role.contract().staging.as_str()
}

#[test]
fn artifact_role_contract_runtime_staging_matches_current_helpers() {
    let rows = events::ArtifactRole::ALL
        .iter()
        .copied()
        .map(|role| format!("{role:?} -> {}", runtime_staging_class(role)))
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        rows,
        "TypedExecutionSpec -> run_admission\n\
TypedSpecCertificate -> run_admission\n\
TypedConfig -> run_admission\n\
FactDescriptor -> run_admission\n\
SeedInput -> run_admission\n\
StateOutput -> attempt_state_output\n\
FactResponse -> attempt_fact_response\n\
FactQueryEvidence -> attempt_fact_query_evidence\n\
SideEffectIntent -> side_effect_intent\n\
PreparedInvocation -> side_effect_prepared_invocation\n\
NotSubmittedProof -> side_effect_not_submitted_proof\n\
Submission -> side_effect_submission\n\
SubmissionUnknownEvidence -> side_effect_submission_unknown\n\
Receipt -> side_effect_receipt\n\
Confirmation -> side_effect_confirmation\n\
AmbiguityEvidence -> side_effect_ambiguity\n\
ManualResolutionEvidence -> manual_resolution\n\
ManualResolutionAuthorization -> manual_resolution\n\
PublicOutput -> attempt_public_output\n\
RedactedDiagnostic -> attempt_redacted_diagnostic\n\
RetentionManifest -> middleware_retention_manifest"
    );
}

fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    evidence: store::ArtifactEvidenceRef,
) -> Result<StagedArtifact> {
    let binding = staged_artifact_binding_kind(evidence.artifact_role).expect("staged role");
    StagedArtifact::finalized_attempt_artifact_for_tests(ctx, evidence, binding)
}

fn staged_side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    evidence: store::ArtifactEvidenceRef,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> Result<StagedArtifact> {
    let phase =
        staged_side_effect_artifact_phase(evidence.artifact_role).expect("staged side-effect role");
    StagedArtifact::finalized_attempt_artifact_for_tests(
        ctx,
        evidence,
        StagedArtifactBindingKind::SideEffectEvidence {
            ledger_key,
            invocation_epoch,
            phase,
        },
    )
}

fn node_by_output<'a>(fixture: &'a Fixture, cell_id: &CellId) -> &'a spec::NodeSpec {
    fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| &node.output_cell == cell_id)
        .expect("node by output")
}

fn side_effect_verify_node_for_submit<'a>(
    fixture: &'a Fixture,
    submit: &spec::NodeSpec,
) -> &'a spec::NodeSpec {
    fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| {
            matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::SideEffectVerify(verify))
                    if verify.submit_node_id == submit.node_id
            )
        })
        .expect("side-effect verify node")
}

fn effective_output_cell_for_node(fixture: &Fixture, node: &spec::NodeSpec) -> CellId {
    if node.side_effect.is_some() {
        side_effect_verify_node_for_submit(fixture, node)
            .output_cell
            .clone()
    } else {
        node.output_cell.clone()
    }
}

fn append_attempt_start(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_no: u32,
) -> AttemptId {
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        attempt_no,
    )
    .expect("attempt id");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-attempt-start:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append attempt start");
    attempt_id
}

fn append_or_get_started_attempt(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_no: u32,
) -> AttemptId {
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        attempt_no,
    )
    .expect("attempt id");
    if let Some(attempt) = store
        .projection_snapshot()
        .attempt(&node.node_id, &attempt_id)
        .cloned()
    {
        assert!(
            matches!(attempt.status, store::AttemptStatus::Started { .. }),
            "attempt {attempt_id} for node {} is not active",
            node.node_id
        );
        return attempt_id;
    }
    append_attempt_start(store, fixture, node, attempt_no)
}

fn append_or_get_first_attempt(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
) -> AttemptId {
    append_or_get_started_attempt(store, fixture, node, 1)
}

fn append_attempt_failure(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    retryable: bool,
) {
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-attempt-failure:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptFailed(
                events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable,
                    error: events::MfmErrorInfo {
                        retryable,
                        ..public_output_error()
                    },
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append attempt failure");
}

struct SyntheticSideEffectAppend<'a> {
    fixture: &'a Fixture,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
}

impl<'a> SyntheticSideEffectAppend<'a> {
    fn new(
        fixture: &'a Fixture,
        run_id: &'a RunId,
        node: &'a spec::NodeSpec,
        attempt_id: &'a AttemptId,
    ) -> Self {
        Self {
            fixture,
            run_id,
            node,
            attempt_id,
        }
    }

    fn ledger_purpose(&self) -> events::SideEffectLedgerPurpose {
        events::SideEffectLedgerPurpose::Forward
    }

    fn pair_fields(
        &self,
        node: &spec::NodeSpec,
        role: events::SideEffectPairRole,
    ) -> (SideEffectPairId, events::SideEffectPairRole) {
        side_effect_pair_fields_for_purpose(
            &self.fixture.runtime_spec,
            &node.node_id,
            &self.ledger_purpose(),
            role,
        )
    }

    fn append(
        &self,
        store: &mut TestTypedRunStore,
        commit_key: &str,
        payloads: Vec<events::KernelEventPayload>,
        required_artifacts: Vec<store::ArtifactEvidenceRef>,
        required_side_effect_state: store::RequiredSideEffectState,
        require_attempt_started: bool,
    ) {
        let required_present_logical_keys = require_attempt_started
            .then(|| {
                store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    self.node.node_id, self.attempt_id
                ))
                .expect("attempt logical key")
            })
            .into_iter()
            .collect::<Vec<_>>();
        store
            .append_prepared_commit(store_typed_commit_request! {
                run_id: self.run_id.clone(),
                expected_next_seq: store.expected_next_seq(self.run_id),
                commit_key: store::CommitKey::new(commit_key).expect("commit key"),
                payloads: payloads,
                required_artifacts: required_artifacts,
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys,
                    required_side_effect_states: vec![store::SideEffectStatePrecondition {
                        pair_id: fixture_side_effect_pair_id(self.fixture, self.node),
                        required: required_side_effect_state,
                    }],
                    certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                        self.run_id.clone(),
                        self.fixture.runtime_spec.spec(),
                    )
                    .expect("certified run authority")),
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append synthetic side-effect commit");
    }
}

fn append_synthetic_exclusive_prepare(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> (AttemptId, events::SideEffectLedgerKey) {
    let attempt_id =
        attempt_id(run_id, fixture.runtime_spec.spec_hash(), &node.node_id, 1).expect("attempt id");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(format!("{commit_key}-attempt-start"))
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no: 1,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic attempt start");

    let ledger =
        events::SideEffectLedgerKey::new(format!("holder-{commit_key}")).expect("holder ledger");
    let intent_hash = content_digest_json(serde_json::json!({
        "key": key,
        "ledger": ledger.as_str(),
        "run": run_id.as_str(),
    }))
    .expect("intent digest");
    let resource_key = exclusive_resource_key(fixture, key);
    let resource_lane_requirement_digest = content_digest_json(serde_json::json!({
        "acquisition": "pre_state_invocation",
        "hold": "until_side_effect_terminal",
        "key_schema_id": resource_key.key_schema_id.as_str(),
        "mode": "exclusive",
        "namespace": resource_key.namespace.as_str(),
    }))
    .expect("resource lane requirement digest");
    let intent_artifact_id =
        ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
    let intent_artifact = store::ArtifactEvidenceRef {
        artifact_id: intent_artifact_id.clone(),
        digest: intent_hash.clone(),
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::SideEffectIntent,
    };
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, &attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append(
        store,
        commit_key,
        vec![
            events::KernelEventPayload::SideEffectIntentPersisted(
                events::side_effect::IntentPersisted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose: ledger_purpose.clone(),
                    pair_id: pair_id.clone(),
                    pair_role,
                    invocation_epoch: 1,
                    intent_schema_id: node.config_ref.schema_id.clone(),
                    intent_hash: intent_hash.clone(),
                    intent_artifact_id,
                    idempotency_input_schema_id: node.config_ref.schema_id.clone(),
                    idempotency_input_hash: content(0xc3),
                    idempotency_key: events::IdempotencyKeyRef::new(format!("idem-{commit_key}"))
                        .expect("idempotency key"),
                    capability_kind: side_effect_capability_kind(),
                    capability_version: side_effect_capability_version(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                },
            ),
            events::KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role,
                claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                    .expect("fencing token"),
            }),
            events::KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose: ledger_purpose.clone(),
                pair_id: pair_id.clone(),
                pair_role,
                invocation_epoch: 1,
                resource_key: resource_key.clone(),
                requirement_digest: resource_lane_requirement_digest,
                resolved_by_capability_impl: events::RunnerFactoryId::new(
                    "mfm.test.side_effect_driver",
                )
                .expect("runner factory"),
            }),
            events::KernelEventPayload::SideEffectInvocationPrepared(
                events::side_effect::InvocationPrepared {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key: ledger.clone(),
                    ledger_purpose,
                    pair_id: pair_id.clone(),
                    pair_role,
                    invocation_epoch: 1,
                    claim_generation: 1,
                    claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                        .expect("fencing token"),
                    resource_key: Some(resource_key),
                    prepared_artifact_id: None,
                    prepared_hash: None,
                },
            ),
        ],
        vec![intent_artifact],
        store::RequiredSideEffectState::Absent,
        true,
    );
    (attempt_id, ledger)
}

fn append_synthetic_invocation_started(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append(
        store,
        commit_key,
        vec![events::KernelEventPayload::SideEffectInvocationStarted(
            events::side_effect::InvocationStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
                claim_generation: 1,
                claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                    .expect("fencing token"),
            },
        )],
        Vec::new(),
        store::RequiredSideEffectState::InvocationPrepared,
        false,
    );
}

fn append_synthetic_exclusive_started(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> (AttemptId, events::SideEffectLedgerKey) {
    let (attempt_id, ledger_key) =
        append_synthetic_exclusive_prepare(store, fixture, run_id, node, key, commit_key);
    append_synthetic_invocation_started(
        store,
        fixture,
        run_id,
        node,
        &attempt_id,
        &ledger_key,
        &format!("{commit_key}-started"),
    );
    (attempt_id, ledger_key)
}

fn append_synthetic_submission_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xd7);
    let digest = content(0xd8);
    let evidence = side_effect_evidence(
        node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Submission,
    );
    let side_effect = SyntheticSideEffectAppend::new(fixture, &fixture.run_id, node, attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    side_effect.append(
        store,
        commit_key,
        vec![events::KernelEventPayload::SideEffectSubmissionObserved(
            events::side_effect::SubmissionObserved {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                submission_schema_id: node.config_ref.schema_id.clone(),
                submission_hash: digest,
                submission_artifact_id: artifact_id,
            },
        )],
        vec![evidence],
        store::RequiredSideEffectState::InvocationStarted,
        false,
    );
}

fn append_synthetic_receipt_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    if store
        .projection_snapshot()
        .cell_terminal(&node.output_cell)
        .is_none()
    {
        append_synthetic_submit_boundary_skipped(
            store,
            fixture,
            node,
            attempt_id,
            ledger,
            &format!("{commit_key}-submit-boundary"),
        );
    }
    let verify_node = side_effect_verify_node_for_submit(fixture, node).clone();
    let verify_attempt_id = append_or_get_started_attempt(store, fixture, &verify_node, 1);
    append_synthetic_verify_receipt_observed(
        store,
        fixture,
        node,
        &verify_node,
        &verify_attempt_id,
        ledger,
        commit_key,
    );
}

fn append_synthetic_exclusive_receipt_phase(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    key: &str,
    commit_key: &str,
) -> AttemptId {
    let (attempt_id, ledger_key) =
        append_synthetic_exclusive_started(store, fixture, &fixture.run_id, node, key, commit_key);
    append_synthetic_submission_observed(
        store,
        fixture,
        node,
        &attempt_id,
        &ledger_key,
        &format!("{commit_key}-submission"),
    );
    append_synthetic_receipt_observed(
        store,
        fixture,
        node,
        &attempt_id,
        &ledger_key,
        &format!("{commit_key}-receipt"),
    );
    attempt_id
}

fn append_synthetic_submit_boundary_skipped(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    _ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("submit output cell");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellSkipped(events::CellSkipped {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    cell_id: node.output_cell.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    skip_reason: events::SkipReason {
                        code: events::ErrorCode::new("side_effect_submission_boundary")
                            .expect("skip code"),
                        safe_message: "side-effect submit boundary recorded; verification is delegated to the paired verify node".to_owned(),
                    },
                }),
                events::KernelEventPayload::StateAttemptCompleted(
                    events::StateAttemptCompleted {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        output_cell_id: node.output_cell.clone(),
                    },
                ),
            ],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_side_effect_states: vec![store::SideEffectStatePrecondition {
                    pair_id: fixture_side_effect_pair_id(fixture, node),
                    required: store::RequiredSideEffectState::SubmissionResult,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append synthetic submit boundary skipped");
}

fn append_synthetic_verify_receipt_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    submit_node: &spec::NodeSpec,
    verify_node: &spec::NodeSpec,
    verify_attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xd9);
    let digest = content(0xda);
    let evidence = side_effect_evidence(
        verify_node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Receipt,
    );
    let side_effect =
        SyntheticSideEffectAppend::new(fixture, &fixture.run_id, verify_node, verify_attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) =
        side_effect.pair_fields(submit_node, events::SideEffectPairRole::Verify);
    side_effect.append(
        store,
        commit_key,
        vec![events::KernelEventPayload::SideEffectReceiptObserved(
            events::side_effect::ReceiptObserved {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: verify_node.node_id.clone(),
                attempt_id: verify_attempt_id.clone(),
                ledger_key: ledger.clone(),
                ledger_purpose,
                pair_id,
                pair_role,
                invocation_epoch: 1,
                receipt_schema_id: verify_node.config_ref.schema_id.clone(),
                receipt_hash: digest,
                receipt_artifact_id: artifact_id,
                replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.replay")
                    .expect("replay verifier"),
                resource_touched_set: None,
            },
        )],
        vec![evidence],
        store::RequiredSideEffectState::SubmissionResult,
        true,
    );
}

fn append_synthetic_verify_confirmation_observed(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    submit_node: &spec::NodeSpec,
    verify_node: &spec::NodeSpec,
    verify_attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let artifact_id = artifact(0xdb);
    let digest = content(0xdc);
    let evidence = side_effect_evidence(
        verify_node,
        artifact_id.clone(),
        digest.clone(),
        events::ArtifactRole::Confirmation,
    );
    let side_effect =
        SyntheticSideEffectAppend::new(fixture, &fixture.run_id, verify_node, verify_attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) =
        side_effect.pair_fields(submit_node, events::SideEffectPairRole::Verify);
    let release = synthetic_resource_lane_release(
        store,
        fixture,
        &fixture.run_id,
        verify_node,
        verify_attempt_id,
        ledger,
        "side_effect.confirmed",
    );
    let mut payloads = Vec::new();
    if let Some(release) = release {
        payloads.push(release);
    }
    payloads.push(events::KernelEventPayload::SideEffectConfirmationObserved(
        events::side_effect::ConfirmationObserved {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: verify_node.node_id.clone(),
            attempt_id: verify_attempt_id.clone(),
            ledger_key: ledger.clone(),
            ledger_purpose,
            pair_id,
            pair_role,
            invocation_epoch: 1,
            confirmation_schema_id: verify_node.config_ref.schema_id.clone(),
            confirmation_hash: digest,
            confirmation_artifact_id: artifact_id,
            replay_verifier_id: events::ReplayVerifierId::new("mfm.test.driver.replay")
                .expect("replay verifier"),
            resource_touched_set: None,
        },
    ));
    side_effect.append(
        store,
        commit_key,
        payloads,
        vec![evidence],
        store::RequiredSideEffectState::ReceiptObserved,
        true,
    );
}

fn synthetic_resource_lane_release(
    store: &TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    _attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    reason: &str,
) -> Option<events::KernelEventPayload> {
    let pair_id = match &node.framework {
        Some(spec::FrameworkNodeSpec::SideEffectVerify(verify)) => verify.pair_id.clone(),
        _ => fixture
            .runtime_spec
            .side_effect_pair_for_submit_node(&node.node_id)
            .cloned()
            .expect("side-effect pair"),
    };
    let holder = store::SideEffectPairLedgerRef::new(run_id.clone(), pair_id);
    let snapshot = store.projection_snapshot();
    let (_, lane) = snapshot
        .resource_lanes()
        .find(|(_, projection)| projection.holder == holder)?;
    Some(events::KernelEventPayload::ResourceLaneReleaseIntent(
        events::ResourceLaneReleaseIntent {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            ledger_key: ledger.clone(),
            ledger_purpose: events::SideEffectLedgerPurpose::Forward,
            pair_id: lane.holder.pair_id.clone(),
            pair_role: events::SideEffectPairRole::Verify,
            invocation_epoch: lane.invocation_epoch,
            claim_id: lane.claim_id.clone(),
            release_authority: events::ResourceLaneReleaseAuthority::VerifyTerminal,
            release_reason: events::ResourceLaneReleaseReason::new(reason).expect("release reason"),
        },
    ))
}

fn active_resource_lane_for_pair(
    snapshot: &store::ProjectionSnapshot,
    run_id: &RunId,
    pair_id: &SideEffectPairId,
) -> Option<(store::ResourceLaneKey, store::ResourceLaneProjection)> {
    let holder = store::SideEffectPairLedgerRef::new(run_id.clone(), pair_id.clone());
    snapshot
        .resource_lanes()
        .find(|(_, projection)| projection.holder == holder)
        .map(|(key, projection)| (key.clone(), projection.clone()))
}

fn append_erased_runner_output(
    store: &mut TestTypedRunStore,
    run_id: &RunId,
    commit_key: &str,
    output: ErasedRunnerOutput,
    preconditions: store::CommitPreconditions,
) {
    let required_artifacts = output
        .staged_artifacts
        .iter()
        .map(|artifact| artifact.evidence().clone())
        .collect::<Vec<_>>();
    let payloads = output
        .payloads
        .into_iter()
        .map(events::KernelEventPayload::from)
        .collect::<Vec<_>>();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads: payloads,
            required_artifacts: required_artifacts,
            preconditions: preconditions,
        })
        .expect("append runner output");
}

fn side_effect_evidence(
    node: &spec::NodeSpec,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn append_synthetic_ambiguous(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    ledger: &events::SideEffectLedgerKey,
    commit_key: &str,
) {
    let evidence_hash = content(0xd5);
    let evidence_artifact_id = artifact(0xd6);
    let evidence = side_effect_evidence(
        node,
        evidence_artifact_id.clone(),
        evidence_hash.clone(),
        events::ArtifactRole::AmbiguityEvidence,
    );
    let side_effect = SyntheticSideEffectAppend::new(fixture, run_id, node, attempt_id);
    let ledger_purpose = side_effect.ledger_purpose();
    let (pair_id, pair_role) = side_effect.pair_fields(node, events::SideEffectPairRole::Submit);
    let payloads = vec![
        events::KernelEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            ledger_key: ledger.clone(),
            ledger_purpose,
            pair_id,
            pair_role,
            invocation_epoch: 1,
            ambiguity_code: events::AmbiguityCode::new("unknown").expect("ambiguity"),
            evidence_schema_id: node.config_ref.schema_id.clone(),
            evidence_hash,
            evidence_artifact_id,
        }),
        events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
            spec_hash: fixture.runtime_spec.spec_hash().clone(),
            node_id: node.node_id.clone(),
            attempt_id: attempt_id.clone(),
            retryable: false,
            error: side_effect_error(false),
        }),
    ];
    side_effect.append(
        store,
        commit_key,
        payloads,
        vec![evidence],
        store::RequiredSideEffectState::InvocationStarted,
        false,
    );
}

async fn append_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    outcome: events::ManualResolutionOutcome,
) {
    let manual = match &fixture.runtime_spec.spec().saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual,
        spec::SagaPolicySpec::CompensateCompleted {
            on_remediation_unresolved: spec::RemediationUnresolvedSpec::ManualResolution { manual },
        } => manual,
        _ => panic!("fixture does not carry manual resolution schemas"),
    };
    let evidence_bytes = br#"{"operator_note":"reviewed"}"#.to_vec();
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&evidence_bytes),
    );
    let evidence_artifact_id =
        ArtifactId::from_digest(evidence_hash.algorithm(), *evidence_hash.digest());
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        content_hash: evidence_hash,
        artifact_id: evidence_artifact_id,
    };
    let prefix = build_manual_resolution_prefix_authority_for_tests(
        &fixture.runtime_spec,
        &fixture.run_id,
        store,
        manual.clone(),
    )
    .expect("manual prefix authority");
    let claim = prefix
        .authorization_claim(outcome, evidence)
        .expect("manual claim");
    let operator = manual.authorization.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: manual.authorization.verifier_id.clone(),
        signing_scheme: manual.authorization.signing_scheme.clone(),
        claim: claim.clone(),
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    };
    let proof_bytes = proof
        .canonical_json()
        .expect("canonical manual proof")
        .to_vec();
    record_manual_resolution(
        scheduler,
        store,
        &fixture.runtime_spec,
        &fixture.run_id,
        ManualResolutionRequest {
            outcome,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes,
            note: None,
        },
    )
    .await
    .expect("append manual resolution");
}

fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}

fn append_fact(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    subject_amount: u64,
    response_amount: u64,
) {
    let fact_key = test_fact_key(subject_amount);
    let (evidence, response_bytes) = test_fact_response_artifact(node, response_amount);
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new(format!(
            "manual-fact:{}:{}:{}:{}",
            node.node_id, attempt_id, fact_key, response_amount
        ))
        .expect("commit key"),
        payloads: vec![events::KernelEventPayload::FactRecorded(
            events::FactRecorded {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                claim: test_fact_claim(
                    subject_amount,
                    node.config_ref.schema_id.clone(),
                    content(0xd4),
                    &evidence,
                    fixture.cap_kind.clone(),
                    fixture.cap_version.clone(),
                    fixture.adapter_kind.clone(),
                    fixture.adapter_version.clone(),
                ),
            },
        )],
        required_artifacts: vec![evidence.clone()],
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("attempt logical key")],
            certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                fixture.run_id.clone(),
                fixture.runtime_spec.spec(),
            )
            .expect("certified run authority")),
            ..store::CommitPreconditions::default()
        },
    };
    let plan =
        test_prepared_commit_plan(request, vec![evidence.clone()]).expect("fact commit plan");
    let bundle = store::PreparedCommitBundle::new(
        plan,
        vec![store::PreparedArtifactBytes::new(response_bytes, evidence)
            .expect("fact response bytes")],
        Vec::new(),
    )
    .expect("fact commit bundle");
    block_on_ready(store.append_prepared_commit_bundle(bundle)).expect("append fact");
}

fn append_terminal(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    artifact_id: ArtifactId,
    output_digest: ContentDigest,
) {
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let evidence =
        state_output_artifact(node, descriptor, artifact_id.clone(), output_digest.clone());
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-terminal:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    cell_id: node.output_cell.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: descriptor.output_semantic_type_id.clone(),
                    schema_id: descriptor.output_schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    artifact_id,
                    content_digest: output_digest,
                    producer_state_kind: Some(node.state_kind.clone()),
                    producer_state_version: Some(node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    output_cell_id: node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append terminal");
}

fn append_public_output_render_failure(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        panic!("expected public-output render node");
    };
    let error = public_output_error();
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-public-output-failure:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::PublicOutputRenderFailed(
                    events::PublicOutputRenderFailed {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        public_schema_id: render.public_schema_id.clone(),
                        renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
                        error: error.clone(),
                    },
                ),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable: true,
                    error,
                }),
            ],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append public output failure");
}

fn append_not_submitted_proven(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    invocation_epoch: u32,
) {
    let projection_snapshot = store.projection_snapshot();
    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        &fixture.run_id,
        &projection_snapshot,
        node,
        attempt_id,
    )
    .expect("side-effect projection lookup")
    .expect("side-effect projection");
    let ledger_key = projection.ledger_key.clone();
    let ledger_purpose = projection.ledger_purpose.clone();
    let pair_id = projection.pair_id.clone();
    let pair_role = events::SideEffectPairRole::Submit;
    let proof_artifact = artifact(0xd5);
    let proof_hash = content(0xd6);
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: proof_artifact.clone(),
        digest: proof_hash.clone(),
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::NotSubmittedProof,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-not-submitted:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::SideEffectNotSubmittedProven(
                events::side_effect::NotSubmittedProven {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    ledger_key,
                    ledger_purpose,
                    pair_id,
                    pair_role,
                    invocation_epoch,
                    proof_schema_id: node.config_ref.schema_id.clone(),
                    proof_hash,
                    proof_artifact_id: proof_artifact,
                },
            )],
            required_artifacts: vec![evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                certified_run_authority: Some(store::CertifiedRunStoreAuthority::from_spec(
                    fixture.run_id.clone(),
                    fixture.runtime_spec.spec(),
                )
                .expect("certified run authority")),
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append not-submitted proof");
}

fn attempt_started_count(store: &TestTypedRunStore, run_id: &RunId, node_id: &NodeId) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if &payload.node_id == node_id
            )
        })
        .count()
}

fn runtime_lifecycle_summary(store: &TestTypedRunStore, run_id: &RunId) -> String {
    let projections = store.projection_snapshot();
    let mut started = 0;
    let mut completed = 0;
    let mut failed = 0;
    let mut interrupted = 0;
    for (_, attempt) in projections
        .attempts()
        .filter(|(_, attempt)| &attempt.run_id == run_id)
    {
        match attempt.status {
            store::AttemptStatus::Started { .. } => started += 1,
            store::AttemptStatus::Completed { .. } => completed += 1,
            store::AttemptStatus::Failed { .. } => failed += 1,
            store::AttemptStatus::Interrupted => interrupted += 1,
        }
    }
    let total = started + completed + failed + interrupted;
    let run_attempts = projections
        .attempts()
        .filter(|(_, attempt)| &attempt.run_id == run_id)
        .map(|((node_id, attempt_id), _)| (node_id.clone(), attempt_id.clone()))
        .collect::<BTreeSet<_>>();
    let cells = projections
        .cells()
        .filter(|(_, _, terminal)| match terminal {
            store::CellTerminalProjection::Produced {
                node_id,
                attempt_id,
                ..
            }
            | store::CellTerminalProjection::Skipped {
                node_id,
                attempt_id,
                ..
            } => run_attempts.contains(&(node_id.clone(), attempt_id.clone())),
        })
        .count();
    let side_effects = projections
        .side_effects()
        .filter(|(_, side_effect)| &side_effect.run_id == run_id)
        .count();
    let lanes_total = projections.resource_lanes().count();
    let lanes = projections
        .resource_lanes()
        .filter(|(_, lane)| &lane.holder.run_id == run_id)
        .count();
    let public_outputs = projections.public_outputs().count();
    let retentions = projections
        .retentions()
        .filter(|(retention_run_id, _)| *retention_run_id == run_id)
        .count();
    format!(
        "run={:?} attempts[started={started} completed={completed} failed={failed} interrupted={interrupted} total={total}] cells={cells} side_effects={side_effects} lanes[run={lanes} total={lanes_total}] public_outputs={public_outputs} retentions={retentions}",
        projections.run_state(run_id)
    )
}

fn assert_node_failed_with_code(
    store: &TestTypedRunStore,
    node_id: &NodeId,
    code: &str,
) -> AttemptId {
    assert_node_failed_with_code_and_retryable(store, node_id, code, false)
}

fn assert_node_failed_with_code_and_retryable(
    store: &TestTypedRunStore,
    node_id: &NodeId,
    code: &str,
    expected_retryable: bool,
) -> AttemptId {
    let failures = store
        .projection_snapshot()
        .attempts()
        .filter_map(|((attempt_node_id, attempt_id), attempt)| {
            if attempt_node_id != node_id {
                return None;
            }
            let store::AttemptStatus::Failed { retryable, error } = &attempt.status else {
                return None;
            };
            Some((attempt_id.clone(), *retryable, error.as_ref().clone()))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        failures.len(),
        1,
        "expected one failed attempt for node {node_id}"
    );
    let (attempt_id, retryable, error) = failures.into_iter().next().expect("failure");
    assert_eq!(
        retryable, expected_retryable,
        "failure-safe retryability for {code}"
    );
    assert_eq!(error.code.as_str(), code);
    assert_eq!(error.retryable, retryable);
    let diagnostic = error
        .diagnostic_ref
        .as_ref()
        .expect("failure-safe terminalization records redacted diagnostic evidence");
    assert_eq!(diagnostic.role, events::ArtifactRole::RedactedDiagnostic);
    assert_eq!(diagnostic.semantic_type_id, None);
    assert_eq!(
        diagnostic.media_type,
        spec::MediaType::new("application/json").expect("media type")
    );
    assert!(
        diagnostic
            .schema_id
            .as_str()
            .contains("schema:mfm.runtime.redacted_attempt_failure_diagnostic:1:sha256-jcs-v1:"),
        "diagnostic schema id should identify the runtime redacted failure diagnostic schema"
    );
    let retained = store
        .projection_snapshot()
        .retentions()
        .any(|(_, retention)| {
            retention
                .refs
                .get(&diagnostic.artifact_id)
                .is_some_and(|retention_ref| {
                    retention_ref.role == events::ArtifactRole::RedactedDiagnostic
                        && retention_ref.content_digest == diagnostic.content_digest
                })
        });
    assert!(
        retained,
        "failure-safe diagnostic artifact should be retained as runtime evidence"
    );
    attempt_id
}

fn assert_failure_code_count(store: &TestTypedRunStore, code: &str, expected: usize) {
    let count = store
        .projection_snapshot()
        .attempts()
        .filter(|(_, attempt)| {
            matches!(
                &attempt.status,
                store::AttemptStatus::Failed { error, .. } if error.code.as_str() == code
            )
        })
        .count();
    assert_eq!(count, expected, "failure code count for {code}");
}

fn fact_recorded_count(store: &TestTypedRunStore, expected: &mfm_facts::FactKey) -> usize {
    store
        .projection_snapshot()
        .fact_records()
        .filter(|(_, fact)| fact.claim.subject().fact_key() == expected)
        .count()
}

#[test]
fn scheduler_does_not_match_open_attempt_disposition() {
    let scheduler_source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/scheduler.rs"))
            .expect("scheduler source");
    assert!(
        !scheduler_source.contains("OpenAttemptDisposition"),
        "scheduler facade must not match recovery dispositions directly"
    );
    let transition_source =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/transition.rs"))
            .expect("transition source");
    assert!(
        !transition_source.contains("TransitionDecision::PublicOutputProjected")
            && !transition_source.contains("PublicOutputProjected,"),
        "public-output projected is a scheduler status, not a transition decision variant"
    );
}

#[test]
fn runtime_order_is_deterministic_for_reordered_spec_nodes() {
    let fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    envelope.spec.nodes.reverse();
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    let runtime = CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime");
    assert_eq!(
        runtime.topological_order(),
        fixture.runtime_spec.topological_order()
    );
}

fn registered_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
    registered_fixture_runners_with_adapter_executable(fixture, test_adapter_executable_identity())
}

fn registered_fixture_runners_with_adapter_executable(
    fixture: &Fixture,
    adapter_executable: events::ExecutableIdentity,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities_with_adapter_executable(
        &mut registry,
        &fixture.runtime_spec,
        adapter_executable,
    );
    register_default_fixture_pure_runner(&mut registry, fixture);
    register_default_fixture_read_runner(&mut registry, fixture);
    registry
}

fn registered_side_effect_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_side_effect_verify_fixture_runner(&mut registry, fixture);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, fixture);
    registry
}

fn registered_first_side_effect_runners_with<R: ErasedNodeRunner + 'static>(
    fixture: &Fixture,
    runner: R,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_side_effect_verify_fixture_runner(&mut registry, fixture);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            runner,
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, fixture);
    registry
}

fn registered_first_side_effect_and_verify_runners_with<
    R: ErasedNodeRunner + 'static,
    V: ErasedNodeRunner + 'static,
>(
    fixture: &Fixture,
    runner: R,
    verify_runner: V,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    let submit_descriptor_id = side_effect_submit_descriptor_ids(fixture)
        .into_iter()
        .next()
        .expect("side-effect submit descriptor");
    register_side_effect_verify_runner_with(&mut registry, submit_descriptor_id, verify_runner);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            runner,
        ))
        .expect("binding a");
    register_read_external_fixture_runner(&mut registry, fixture);
    registry
}

fn compensated_saga_scheduler(fixture: &Fixture) -> SerialTypedScheduler {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_side_effect_verify_fixture_runner(&mut registry, fixture);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding forward b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "pure",
            BlockingRunner,
        ))
        .expect("binding failure node");
    test_scheduler(registry)
}

fn register_side_effect_verify_fixture_runner(
    registry: &mut ErasedRunnerRegistry,
    fixture: &Fixture,
) {
    for descriptor_id in side_effect_submit_descriptor_ids(fixture) {
        register_side_effect_verify_runner_with(
            registry,
            descriptor_id,
            DriverSideEffectVerifyRunner::new(fixture),
        );
    }
}

fn register_side_effect_verify_runner_with<R: ErasedNodeRunner + 'static>(
    registry: &mut ErasedRunnerRegistry,
    submit_descriptor_id: DescriptorId,
    runner: R,
) {
    let factory_id = events::RunnerFactoryId::new("read_external").expect("factory");
    registry
        .register_side_effect_verify_runner(
            submit_descriptor_id,
            factory_id.clone(),
            events::ExecutableIdentity {
                factory_id,
                cargo_package_digest: content(0xe1),
                binary_digest: content(0xe2),
                nix_derivation_hash: None,
                nix_output_hash: None,
            },
            Arc::new(runner),
        )
        .expect("side-effect verify binding");
}

fn side_effect_submit_descriptor_ids(fixture: &Fixture) -> Vec<DescriptorId> {
    let mut descriptors = BTreeSet::new();
    for node in fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(fixture.runtime_spec.spec().remediations.values())
    {
        if node.side_effect.is_some() {
            descriptors.insert(node.descriptor_id.clone());
        }
    }
    descriptors.into_iter().collect()
}

fn binding<R: ErasedNodeRunner + 'static>(
    descriptor_id: DescriptorId,
    factory: &str,
    runner: R,
) -> ErasedRunnerBinding {
    let factory_id = events::RunnerFactoryId::new(factory).expect("factory");
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        events::ExecutableIdentity {
            factory_id,
            cargo_package_digest: content(0xe1),
            binary_digest: content(0xe2),
            nix_derivation_hash: None,
            nix_output_hash: None,
        },
        Arc::new(runner),
    )
    .expect("runner binding")
}

fn runtime_retention_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
    typed
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        })
        .expect("retention node")
        .output_cell
        .clone()
}

fn append_runtime_retention_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    public_output_receipt_cell: CellId,
    valid_ordering: bool,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf1; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf2; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf3; 32]),
    );
    let config_ref = spec::framework_config_ref("project_retention_manifest", &node_id)
        .expect("retention config ref");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == public_output_receipt_cell)
        .expect("input cell")
        .clone();
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "project_retention_manifest",
        "public_output_receipt",
        &input_cell,
    )
    .expect("input binding");
    let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
    let receipt_schema =
        spec::retention_manifest_receipt_schema_id().expect("retention receipt schema");
    let receipt_semantic =
        spec::retention_manifest_receipt_semantic_type_id().expect("retention receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "project_retention_manifest",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xf9; 32]),
    )
    .expect("state kind");
    let state_version = StateVersion::new("mfm.framework.state.project_retention_manifest.v1")
        .expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.project_retention_manifest".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed.kind.clone(),
                effect_class: managed.class.as_str().to_owned(),
                effect_name: managed.name.to_owned(),
                effect_version: managed.version,
                capabilities: no_caps.clone(),
                runner: "managed_platform_write".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                side_effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: input_cell.scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xfa),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    let predecessors = match &input_cell.producer {
        spec::CellProducer::Node(producer) => vec![producer.clone()],
        spec::CellProducer::Seed(_) => Vec::new(),
    };
    let framework_receipt = if valid_ordering {
        public_output_receipt_cell
    } else {
        input_cell.cell_id.clone()
    };
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/project-retention-manifest")
            .expect("stable key"),
        scope_id: input_cell.scope_id,
        state_kind,
        state_version,
        descriptor_id,
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: managed.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(
            spec::ProjectRetentionManifestNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
                public_output_receipt_cell: framework_receipt,
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: predecessors,
    });
    node_id
}

fn append_runtime_complete_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
    retention_manifest_receipt_cell: CellId,
    valid_ordering: bool,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb2; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb3; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb4; 32]),
    );
    let config_ref =
        spec::framework_config_ref("complete_run", &node_id).expect("complete config ref");
    let input_cell = typed
        .cells
        .iter()
        .find(|cell| cell.cell_id == retention_manifest_receipt_cell)
        .expect("input cell")
        .clone();
    let input_binding = spec::framework_lifecycle_receipt_input_binding(
        "complete_run",
        "retention_manifest_receipt",
        &input_cell,
    )
    .expect("input binding");
    let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
    let receipt_schema = spec::complete_run_receipt_schema_id().expect("complete receipt schema");
    let receipt_semantic =
        spec::complete_run_receipt_semantic_type_id().expect("complete receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "complete_run",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xb8; 32]),
    )
    .expect("state kind");
    let state_version =
        StateVersion::new("mfm.framework.state.complete_run.v1").expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.complete_run".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed.kind.clone(),
                effect_class: managed.class.as_str().to_owned(),
                effect_name: managed.name.to_owned(),
                effect_version: managed.version,
                capabilities: no_caps.clone(),
                runner: "managed_platform_write".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                side_effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: input_cell.scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xbb),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    let predecessors = match &input_cell.producer {
        spec::CellProducer::Node(producer) => vec![producer.clone()],
        spec::CellProducer::Seed(_) => Vec::new(),
    };
    let framework_receipt = if valid_ordering {
        retention_manifest_receipt_cell
    } else {
        typed
            .public_outputs
            .outputs
            .first()
            .expect("public output")
            .cell_id
            .clone()
    };
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/complete-run").expect("stable key"),
        scope_id: input_cell.scope_id,
        state_kind,
        state_version,
        descriptor_id,
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: managed.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::CompleteRun(
            spec::CompleteRunNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
                retention_manifest_receipt_cell: framework_receipt,
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: predecessors,
    });
    node_id
}

fn append_runtime_resolve_saga_terminal_lifecycle_node(
    typed: &mut spec::TypedExecutionSpec,
) -> NodeId {
    let node_id = NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc2; 32]),
    );
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc3; 32]),
    );
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc4; 32]),
    );
    let config_ref =
        spec::framework_config_ref("resolve_saga_terminal", &node_id).expect("resolve config ref");
    let input_binding = spec::framework_lifecycle_unit_input_binding("resolve_saga_terminal")
        .expect("input binding");
    let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
    let receipt_schema =
        spec::resolve_saga_terminal_receipt_schema_id().expect("resolve receipt schema");
    let receipt_semantic =
        spec::resolve_saga_terminal_receipt_semantic_type_id().expect("resolve receipt semantic");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let state_kind = StateKind::new(
        "mfm.framework",
        "resolve_saga_terminal",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xc8; 32]),
    )
    .expect("state kind");
    let state_version =
        StateVersion::new("mfm.framework.state.resolve_saga_terminal.v1").expect("state version");
    typed
        .descriptor_identities
        .push(spec::DescriptorIdentity::State(Box::new(
            spec::StateDescriptorIdentity {
                descriptor_id: descriptor_id.clone(),
                name: "mfm.framework.resolve_saga_terminal".to_owned(),
                state_kind: state_kind.clone(),
                state_version: state_version.clone(),
                config_schema_id: config_ref.schema_id.clone(),
                input_schema_id: input_binding.input_schema_id.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed.kind.clone(),
                effect_class: managed.class.as_str().to_owned(),
                effect_name: managed.name.to_owned(),
                effect_version: managed.version,
                capabilities: no_caps.clone(),
                runner: "managed_platform_write".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                side_effect_contract_digest: None,
            },
        )));
    typed.config_refs.push(config_ref.clone());
    let scope_id = typed.scopes.first().expect("root scope").scope_id.clone();
    typed.cells.push(spec::CellSpec {
        cell_id: output_cell.clone(),
        producer: spec::CellProducer::Node(node_id.clone()),
        scope_id: scope_id.clone(),
        semantic_type_id: receipt_semantic,
        schema_id: receipt_schema,
        value_lineage: spec::ValueLineageRef {
            lineage_digest: content(0xcb),
        },
        terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
        storage_policy: spec::StoragePolicy::ContentAddressed,
        redaction_policy: spec::RedactionPolicy::Public,
    });
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/resolve-saga-terminal")
            .expect("stable key"),
        scope_id,
        state_kind,
        state_version,
        descriptor_id,
        config_ref,
        input_bindings: input_binding,
        output_cell,
        effect_kind: managed.kind,
        capability_bindings: no_caps,
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(
            spec::ResolveSagaTerminalNodeSpec {
                public_schema_id: typed.public_outputs.public_schema_id.clone(),
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: typed.scopes[0].planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    });
    node_id
}

fn fixture() -> Fixture {
    let scope = ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, D0);
    let seed_id = SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, D1);
    let seed_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2);
    let node_a = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D3);
    let cell_a = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D4);
    let node_b = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D5);
    let cell_b = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D6);
    let descriptor_a = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D7);
    let descriptor_b = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
    let semantic = fixture_value_semantic_id();
    let value_schema = fixture_value_schema_id();
    let input_schema = SchemaId::new("mfm.test.input", "1", DigestAlgorithm::Sha256JcsV1, DB)
        .expect("input schema");
    let config_schema = SchemaId::new("mfm.test.config", "1", DigestAlgorithm::Sha256JcsV1, DC)
        .expect("config schema");
    let public_schema = SchemaId::new("mfm.test.public", "1", DigestAlgorithm::Sha256JcsV1, DD)
        .expect("public schema");
    let effect_kind =
        EffectKind::new("mfm.test", "pure", DigestAlgorithm::Sha256JcsV1, DE).expect("effect");
    let read_effect =
        EffectKind::new("mfm.test", "read", DigestAlgorithm::Sha256JcsV1, DF).expect("read effect");
    let cap_kind = CapabilityKind::new("mfm.test", "read-db", DigestAlgorithm::Sha256JcsV1, D0)
        .expect("cap kind");
    let cap_version = CapabilityVersion::new("mfm.cap.read_db.v1").expect("cap version");
    let adapter_kind = AdapterKind::new("mfm.test", "adapter", DigestAlgorithm::Sha256JcsV1, D1)
        .expect("adapter kind");
    let adapter_version = AdapterVersion::new("mfm.adapter.v1").expect("adapter version");
    let read_cap = CapabilityDescriptor::new(
        cap_kind.clone(),
        cap_version.clone(),
        CapabilityRole::ReadExternal,
        "read-db",
    )
    .expect("capability");
    let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
    let read_caps = CapabilitySetDescriptor::new(vec![read_cap]).expect("read caps");
    let config_digest = digest_for_bytes(TEST_CONFIG_BYTES);
    let config_ref = spec::ConfigRef {
        schema_id: config_schema.clone(),
        artifact_id: ArtifactId::from_digest(config_digest.algorithm(), *config_digest.digest()),
        digest: config_digest,
        byte_len: TEST_CONFIG_BYTES.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
    };
    let lineage_seed = spec::ValueLineageRef {
        lineage_digest: content(0x41),
    };
    let lineage_a = spec::ValueLineageRef {
        lineage_digest: content(0x42),
    };
    let lineage_b = spec::ValueLineageRef {
        lineage_digest: content(0x43),
    };
    let planning = spec::PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: content(0x44),
    };
    let seed_digest = digest_for_bytes(TEST_SEED_BYTES);
    let seed_ref = events::SeedCellRef {
        seed_id: seed_id.clone(),
        cell_id: seed_cell.clone(),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        digest: seed_digest.clone(),
        seed_artifact: events::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(seed_digest.algorithm(), *seed_digest.digest()),
            role: events::ArtifactRole::SeedInput,
            schema_id: value_schema.clone(),
            semantic_type_id: Some(semantic.clone()),
            content_digest: seed_digest.clone(),
            byte_len: TEST_SEED_BYTES.len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media"),
        },
    };
    let renderer = spec::RendererDescriptorIdentity {
        descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D1),
        renderer_kind: spec::RendererKind::new("public-output/json").expect("renderer"),
        renderer_version: spec::RendererVersion::new("mfm.renderer.test.v1")
            .expect("renderer version"),
        public_schema_id: public_schema.clone(),
        canonicalizer_identity: spec::CanonicalizerIdentity::new("sha256-jcs-v1")
            .expect("canonicalizer"),
    };
    let public_output_cell = spec::PublicOutputCell {
        public_field_path: spec::PublicFieldPath::new("result").expect("field"),
        cell_id: cell_b.clone(),
        producer: spec::CellProducer::Node(node_b.clone()),
        scope_id: scope.clone(),
        semantic_type_id: semantic.clone(),
        schema_id: value_schema.clone(),
        value_lineage: lineage_b.clone(),
        required_terminal: spec::RequiredTerminal::ProducedOnly,
    };
    let public_outputs = spec::PublicOutputSpec {
        public_schema_id: public_schema.clone(),
        outputs: vec![public_output_cell.clone()],
        renderer_descriptor: renderer.clone(),
    };
    let output_spec_digest = public_outputs.digest().expect("public output digest");
    let render_node = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
    let render_config_ref = spec::framework_config_ref("public_output_render", &render_node)
        .expect("render config ref");
    let render_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    let render_descriptor = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DA);
    let receipt_schema = spec::public_output_receipt_schema_id().expect("receipt schema");
    let receipt_semantic =
        spec::public_output_receipt_semantic_type_id().expect("receipt semantic");
    let render_lineage = spec::ValueLineageRef {
        lineage_digest: content(0x45),
    };
    let render_input_root = spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
        field_path: public_output_cell.public_field_path.clone(),
        node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path: public_output_cell.public_field_path.clone(),
            cell_id: public_output_cell.cell_id.clone(),
            semantic_type_id: public_output_cell.semantic_type_id.clone(),
            schema_id: public_output_cell.schema_id.clone(),
            required_terminal: public_output_cell.required_terminal,
            value_lineage: public_output_cell.value_lineage.clone(),
        })),
    }]);
    let render_input_binding = spec::InputBindingSpec {
        input_schema_id: public_schema.clone(),
        input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DC),
        digest: content_digest_json(input_node_json(&render_input_root))
            .expect("render input digest"),
        root: render_input_root,
    };
    let managed_effect = ManagedPlatformWrite::descriptor().expect("managed effect");
    let render_state_kind = StateKind::new(
        "mfm.framework.state",
        "render_public_outputs",
        DigestAlgorithm::Sha256JcsV1,
        DD,
    )
    .expect("render state kind");
    let render_state_version = StateVersion::new("mfm.framework.state.render_public_outputs.v1")
        .expect("render state version");
    let node_a_spec = node_spec(NodeSpecFixture {
        node_id: node_a.clone(),
        descriptor_id: descriptor_a.clone(),
        scope_id: scope.clone(),
        state_name: "mfm.test.state.a",
        state_kind: StateKind::new("mfm.test", "a", DigestAlgorithm::Sha256JcsV1, D2)
            .expect("state a"),
        state_version: StateVersion::new("mfm.test.state.a.v1").expect("state version"),
        effect_kind: effect_kind.clone(),
        config_ref: config_ref.clone(),
        input_schema: input_schema.clone(),
        input_cell: seed_cell.clone(),
        input_lineage: lineage_seed.clone(),
        output_cell: cell_a.clone(),
        output_schema: value_schema.clone(),
        semantic: semantic.clone(),
        caps: no_caps.clone(),
        predecessors: Vec::new(),
        adapter_bindings: Vec::new(),
        planning: planning.clone(),
    });
    let node_b_spec = node_spec(NodeSpecFixture {
        node_id: node_b.clone(),
        descriptor_id: descriptor_b.clone(),
        scope_id: scope.clone(),
        state_name: "mfm.test.state.b",
        state_kind: StateKind::new("mfm.test", "b", DigestAlgorithm::Sha256JcsV1, D3)
            .expect("state b"),
        state_version: StateVersion::new("mfm.test.state.b.v1").expect("state version"),
        effect_kind: read_effect.clone(),
        config_ref: config_ref.clone(),
        input_schema: input_schema.clone(),
        input_cell: cell_a.clone(),
        input_lineage: lineage_a.clone(),
        output_cell: cell_b.clone(),
        output_schema: value_schema.clone(),
        semantic: semantic.clone(),
        caps: read_caps.clone(),
        predecessors: vec![node_a.clone()],
        adapter_bindings: vec![spec::AdapterBinding {
            adapter_kind: adapter_kind.clone(),
            adapter_version: adapter_version.clone(),
            binding_digest: None,
        }],
        planning: planning.clone(),
    });
    let render_node_spec = spec::NodeSpec {
        node_id: render_node.clone(),
        stable_key: spec::StableAuthorKey::new("public-output").expect("render key"),
        scope_id: scope.clone(),
        state_kind: render_state_kind.clone(),
        state_version: render_state_version.clone(),
        descriptor_id: render_descriptor.clone(),
        config_ref: render_config_ref.clone(),
        input_bindings: render_input_binding,
        output_cell: render_cell.clone(),
        effect_kind: managed_effect.kind.clone(),
        capability_bindings: no_caps.clone(),
        adapter_bindings: Vec::new(),
        side_effect: None,
        framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
            spec::PublicOutputRenderNodeSpec {
                public_schema_id: public_schema.clone(),
                output_spec_digest: output_spec_digest.clone(),
                renderer_descriptor: renderer.clone(),
                required_cells: public_outputs.outputs.clone(),
            },
        )),
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: planning.clone(),
        deterministic_predecessors: vec![node_b.clone()],
    };
    let mut spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D4),
                name: "mfm.test.composition".to_owned(),
                version: "mfm.test.composition.v1".to_owned(),
            },
            config_hash: content(0x60),
        },
        saga: spec::SagaPolicySpec::NoSideEffects,
        scopes: vec![spec::ScopeSpec {
            scope_id: scope.clone(),
            parent_scope_id: None,
            stable_key: spec::StableAuthorKey::new("root").expect("stable key"),
            planning_lineage: planning.clone(),
        }],
        seeds: vec![spec::SeedSpec {
            seed_id: seed_id.clone(),
            seed_key: spec::StableAuthorKey::new("launch").expect("seed key"),
            cell_id: seed_cell.clone(),
            scope_id: scope.clone(),
            semantic_type_id: semantic.clone(),
            schema_id: value_schema.clone(),
            required_digest: Some(seed_digest),
        }],
        descriptor_identities: vec![
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &node_a_spec,
                descriptor_a.clone(),
                "mfm.test.state.a",
                effect_kind,
                no_caps.clone(),
                "pure",
            ))),
            spec::DescriptorIdentity::State(Box::new(state_descriptor(
                &node_b_spec,
                descriptor_b.clone(),
                "mfm.test.state.b",
                read_effect,
                read_caps,
                "read",
            ))),
            spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
                descriptor_id: render_descriptor.clone(),
                name: "mfm.framework.render_public_outputs".to_owned(),
                state_kind: render_state_kind,
                state_version: render_state_version,
                config_schema_id: render_config_ref.schema_id.clone(),
                input_schema_id: public_schema.clone(),
                output_schema_id: receipt_schema.clone(),
                output_semantic_type_id: receipt_semantic.clone(),
                effect_kind: managed_effect.kind,
                effect_class: managed_effect.class.as_str().to_owned(),
                effect_name: managed_effect.name.to_owned(),
                effect_version: managed_effect.version,
                capabilities: no_caps,
                runner: "managed_platform_write".to_owned(),
                emitted_fact_descriptors: Vec::new(),
                side_effect_contract_digest: None,
            })),
            spec::DescriptorIdentity::Renderer(Box::new(renderer.clone())),
        ],
        config_refs: vec![config_ref.clone(), render_config_ref],
        nodes: vec![
            render_node_spec.clone(),
            node_b_spec.clone(),
            node_a_spec.clone(),
        ],
        remediations: BTreeMap::new(),
        cells: vec![
            spec::CellSpec {
                cell_id: seed_cell,
                producer: spec::CellProducer::Seed(seed_id),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                value_lineage: lineage_seed.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            },
            spec::CellSpec {
                cell_id: cell_a.clone(),
                producer: spec::CellProducer::Node(node_a.clone()),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                value_lineage: lineage_a.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            },
            spec::CellSpec {
                cell_id: cell_b.clone(),
                producer: spec::CellProducer::Node(node_b.clone()),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                value_lineage: lineage_b.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::ContentAddressed,
                redaction_policy: spec::RedactionPolicy::Public,
            },
            spec::CellSpec {
                cell_id: render_cell.clone(),
                producer: spec::CellProducer::Node(render_node.clone()),
                scope_id: scope.clone(),
                semantic_type_id: receipt_semantic,
                schema_id: receipt_schema,
                value_lineage: render_lineage.clone(),
                terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                storage_policy: spec::StoragePolicy::PublicOutputArtifact,
                redaction_policy: spec::RedactionPolicy::Public,
            },
        ],
        value_lineages: vec![
            spec::ValueLineage {
                lineage_ref: lineage_seed.clone(),
                scope_id: scope.clone(),
                producer: spec::CellProducer::Seed(SeedId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    D1,
                )),
                input_cells: Vec::new(),
                config_ref_digest: None,
                planning_lineage: planning.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::Source,
            },
            spec::ValueLineage {
                lineage_ref: lineage_a.clone(),
                scope_id: scope.clone(),
                producer: spec::CellProducer::Node(node_a.clone()),
                input_cells: vec![CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2)],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
            spec::ValueLineage {
                lineage_ref: lineage_b.clone(),
                scope_id: scope.clone(),
                producer: spec::CellProducer::Node(node_b.clone()),
                input_cells: vec![cell_a.clone()],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning.clone(),
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
            spec::ValueLineage {
                lineage_ref: render_lineage,
                scope_id: scope,
                producer: spec::CellProducer::Node(render_node.clone()),
                input_cells: vec![cell_b.clone()],
                config_ref_digest: Some(config_ref.digest.clone()),
                planning_lineage: planning,
                domain_keys: Vec::new(),
                transform_policy: spec::LineageTransformPolicy::StateOutput,
            },
        ],
        planning_lineage: Vec::new(),
        public_outputs,
    })
    .expect("typed spec");
    append_runtime_retention_lifecycle_node(&mut spec, render_cell.clone(), true);
    let retention_receipt = runtime_retention_receipt_cell(&spec);
    append_runtime_complete_lifecycle_node(&mut spec, retention_receipt, true);
    append_runtime_resolve_saga_terminal_lifecycle_node(&mut spec);
    let envelope = spec::HashedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
        .expect("envelope");
    let runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    let identity_material = run_identity_material(&runtime_spec);
    let run_id = identity_material.derive_run_id().expect("run id");
    Fixture {
        runtime_spec,
        run_id,
        distinct_run_key_digest: None,
        seed_ref,
        descriptor_a,
        descriptor_b,
        descriptor_c: None,
        render_node,
        render_cell,
        cell_a,
        cell_b,
        cell_c: None,
        cap_kind,
        cap_version,
        adapter_kind,
        adapter_version,
    }
}

fn fixture_with_first_node_fact_descriptor(
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    fixture_with_node_fact_descriptor(|fixture| fixture.cell_a.clone())
}

fn fixture_with_read_node_fact_descriptor(
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    fixture_with_node_fact_descriptor(|fixture| fixture.cell_b.clone())
}

fn fixture_with_read_node_and_other_node_fact_descriptors() -> (
    Fixture,
    mfm_facts::FactDescriptor,
    mfm_facts::FactDescriptor,
) {
    let (mut fixture, read_descriptor, _read_ref) = fixture_with_read_node_fact_descriptor();
    let other_descriptor = test_fact_descriptor_with_kind("mfm.runtime.test.other_fact");
    let other_ref =
        mfm_program::fact_descriptor_ref_for_descriptor(&other_descriptor).expect("descriptor ref");
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let node = envelope
        .spec
        .nodes
        .iter_mut()
        .find(|node| node.output_cell == fixture.cell_a)
        .expect("other fixture node");
    let descriptor_id = node.descriptor_id.clone();
    node.fact_descriptor_allowlist = vec![other_ref.clone()];
    let state_descriptor = envelope
        .spec
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) if state.descriptor_id == descriptor_id => {
                Some(state)
            }
            _ => None,
        })
        .expect("other fixture state descriptor");
    state_descriptor.emitted_fact_descriptors = vec![other_ref];
    let envelope =
        spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("descriptor rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("descriptor runtime spec");
    refresh_fixture_run_id(&mut fixture);
    (fixture, read_descriptor, other_descriptor)
}

fn fixture_with_node_fact_descriptor(
    select_output_cell: impl FnOnce(&Fixture) -> CellId,
) -> (Fixture, mfm_facts::FactDescriptor, spec::FactDescriptorRef) {
    let mut fixture = fixture();
    let output_cell = select_output_cell(&fixture);
    let descriptor =
        <RuntimeTestFact as mfm_program::MfmFactType>::descriptor().expect("fact descriptor");
    let descriptor_ref =
        mfm_program::fact_descriptor_ref_for_descriptor(&descriptor).expect("descriptor ref");
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let node = envelope
        .spec
        .nodes
        .iter_mut()
        .find(|node| node.output_cell == output_cell)
        .expect("fixture node");
    let descriptor_id = node.descriptor_id.clone();
    node.fact_descriptor_allowlist = vec![descriptor_ref.clone()];
    let state_descriptor = envelope
        .spec
        .descriptor_identities
        .iter_mut()
        .find_map(|identity| match identity {
            spec::DescriptorIdentity::State(state) if state.descriptor_id == descriptor_id => {
                Some(state)
            }
            _ => None,
        })
        .expect("fixture state descriptor");
    state_descriptor.emitted_fact_descriptors = vec![descriptor_ref.clone()];
    let envelope =
        spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("descriptor rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("descriptor runtime spec");
    refresh_fixture_run_id(&mut fixture);
    (fixture, descriptor, descriptor_ref)
}

fn fixture_with_retention_lifecycle_node() -> Fixture {
    fixture()
}

#[derive(Clone, Copy)]
enum RuntimeSideEffectClaim {
    ManualOnly,
    Exclusive,
    ExactTouchedSet,
}

impl RuntimeSideEffectClaim {
    fn into_resource_claim(self) -> ResourceClaim {
        match self {
            Self::ManualOnly => ResourceClaim::manual_only(),
            Self::Exclusive => ResourceClaim::exclusive(
                exclusive_resource_namespace(),
                <CertifierValue as mfm_values::MfmValue>::schema_id()
                    .expect("exclusive key schema"),
            ),
            Self::ExactTouchedSet => ResourceClaim::exact_touched_set(
                exact_touched_set_resource_namespace(),
                <FixtureSideEffectEvidence as mfm_values::MfmValue>::schema_id()
                    .expect("touched-set evidence schema"),
            ),
        }
    }
}

#[derive(Clone, Copy)]
enum RuntimeSideEffectFixtureShape {
    Chained,
    IndependentSecond,
    CompensatingPairWithFailingTail,
}

fn runtime_side_effect_fixture(
    shape: RuntimeSideEffectFixtureShape,
    claim: RuntimeSideEffectClaim,
    verification: spec::SideEffectVerificationSpec,
) -> Fixture {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<RuntimeSubmitAState>()
        .expect("submit a registration");
    states
        .register::<RuntimeSubmitBState>()
        .expect("submit b registration");
    states
        .register::<RuntimeReadState>()
        .expect("read registration");
    states
        .register::<RuntimeSeedReadState>()
        .expect("seed read registration");
    states
        .register::<RuntimeTailState>()
        .expect("tail registration");
    let seed = CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed");
    let seed_digest = seed.content_digest().clone();
    let seed_byte_len = seed.byte_len() as u64;
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            if matches!(
                shape,
                RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail
            ) {
                root.set_saga_policy(SideEffectSagaPolicy::CompensateCompleted {
                    on_remediation_unresolved:
                        mfm_program::RemediationUnresolved::FailWithoutAcdcClaim,
                })?;
            } else {
                root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            }
            let input = root.seed(mfm_program::SeedKey::new("initial")?, seed.clone())?;
            match shape {
                RuntimeSideEffectFixtureShape::Chained => {
                    let forward = root.scope().side_effect::<RuntimeSubmitAState, _>(
                        StateKey::new("a")?,
                        CertifierConfig { multiplier: 3 },
                        input,
                        claim.into_resource_claim(),
                        verification.clone(),
                    )?;
                    let result = root.scope().state::<RuntimeReadState, _>(
                        StateKey::new("b")?,
                        CertifierConfig { multiplier: 5 },
                        forward.into_handle(),
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &FixturePublicOutputs { result },
                    )
                }
                RuntimeSideEffectFixtureShape::IndependentSecond => {
                    let forward = root.scope().side_effect::<RuntimeSubmitAState, _>(
                        StateKey::new("a")?,
                        CertifierConfig { multiplier: 3 },
                        input.clone(),
                        claim.into_resource_claim(),
                        verification.clone(),
                    )?;
                    let side_effect = forward.into_handle();
                    let result = root.scope().state::<RuntimeSeedReadState, _>(
                        StateKey::new("b")?,
                        CertifierConfig { multiplier: 5 },
                        input,
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &DualFixturePublicOutputs {
                            result,
                            side_effect,
                        },
                    )
                }
                RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail => {
                    let (forward_a, _remediation_a) = root
                        .scope()
                        .side_effect_with_compensation::<
                            RuntimeSubmitAState,
                            RuntimeSubmitBState,
                            _,
                            _,
                            _,
                        >(
                            SideEffectNodeParams {
                                key: StateKey::new("a")?,
                                config: CertifierConfig { multiplier: 3 },
                                input,
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            RemediationNodeParams {
                                key: StateKey::new("remediate-a")?,
                                config: CertifierConfig { multiplier: 1 },
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            |forward| Ok(forward.into_handle()),
                        )?;
                    let (forward_b, _remediation_b) = root
                        .scope()
                        .side_effect_with_compensation::<
                            RuntimeSubmitBState,
                            RuntimeSubmitBState,
                            _,
                            _,
                            _,
                        >(
                            SideEffectNodeParams {
                                key: StateKey::new("b")?,
                                config: CertifierConfig { multiplier: 7 },
                                input: forward_a.into_handle(),
                                resource_claim: claim.into_resource_claim(),
                                verification: verification.clone(),
                            },
                            RemediationNodeParams {
                                key: StateKey::new("remediate-b")?,
                                config: CertifierConfig { multiplier: 1 },
                                resource_claim: claim.into_resource_claim(),
                                verification,
                            },
                            |forward| Ok(forward.into_handle()),
                        )?;
                    let result = root.scope().state::<RuntimeTailState, _>(
                        StateKey::new("c")?,
                        CertifierConfig { multiplier: 1 },
                        forward_b.into_handle(),
                    )?;
                    root.bind_public_outputs(
                        PublicOutputKey::new("terminal")?,
                        &FixturePublicOutputs { result },
                    )
                }
            }
        },
    )
    .expect("side-effect draft");
    let certified = mfm_certify::certify_program_draft(&draft).expect("certified side-effect spec");
    let runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    fixture_from_runtime_spec(shape, runtime_spec, seed_digest, seed_byte_len)
}

fn fixture_from_runtime_spec(
    shape: RuntimeSideEffectFixtureShape,
    runtime_spec: CertifiedRuntimeSpec,
    seed_digest: ContentDigest,
    seed_byte_len: u64,
) -> Fixture {
    let spec = runtime_spec.spec();
    let seed = spec.seeds.first().expect("seed");
    let seed_ref = events::SeedCellRef {
        seed_id: seed.seed_id.clone(),
        cell_id: seed.cell_id.clone(),
        scope_id: seed.scope_id.clone(),
        semantic_type_id: seed.semantic_type_id.clone(),
        schema_id: seed.schema_id.clone(),
        digest: seed_digest.clone(),
        seed_artifact: events::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(seed_digest.algorithm(), *seed_digest.digest()),
            role: events::ArtifactRole::SeedInput,
            schema_id: seed.schema_id.clone(),
            semantic_type_id: Some(seed.semantic_type_id.clone()),
            content_digest: seed_digest,
            byte_len: seed_byte_len,
            media_type: spec::MediaType::new("application/json").expect("media"),
        },
    };
    let node_a = runtime_node_by_descriptor_name(&runtime_spec, "mfm.runtime.test.submit_a");
    let node_b_name = match shape {
        RuntimeSideEffectFixtureShape::Chained => "mfm.runtime.test.read_output",
        RuntimeSideEffectFixtureShape::IndependentSecond => "mfm.runtime.test.read_seed",
        RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail => {
            "mfm.runtime.test.submit_b"
        }
    };
    let node_b = runtime_node_by_descriptor_name(&runtime_spec, node_b_name);
    let node_c = matches!(
        shape,
        RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail
    )
    .then(|| runtime_node_by_descriptor_name(&runtime_spec, "mfm.runtime.test.tail"));
    let render = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("render node");
    let render_node = render.node_id.clone();
    let render_cell = render.output_cell.clone();
    let identity_material = run_identity_material(&runtime_spec);
    let run_id = identity_material.derive_run_id().expect("run id");
    let adapter_binding = runtime_adapter_binding();
    Fixture {
        runtime_spec,
        run_id,
        distinct_run_key_digest: None,
        seed_ref,
        descriptor_a: node_a.descriptor_id.clone(),
        descriptor_b: node_b.descriptor_id.clone(),
        descriptor_c: node_c.as_ref().map(|node| node.descriptor_id.clone()),
        render_node,
        render_cell,
        cell_a: node_a.output_cell.clone(),
        cell_b: node_b.output_cell.clone(),
        cell_c: node_c.map(|node| node.output_cell),
        cap_kind: RuntimeReadCap::kind().expect("read cap kind"),
        cap_version: RuntimeReadCap::version().expect("read cap version"),
        adapter_kind: adapter_binding.adapter_kind,
        adapter_version: adapter_binding.adapter_version,
    }
}

fn runtime_node_by_descriptor_name(
    runtime_spec: &CertifiedRuntimeSpec,
    name: &str,
) -> spec::NodeSpec {
    runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            runtime_spec
                .state_descriptor_for_node(node)
                .expect("state descriptor")
                .name
                == name
        })
        .cloned()
        .unwrap_or_else(|| panic!("node for descriptor {name}"))
}

async fn drive_until_public_output_produced(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) {
    for _ in 0..8 {
        drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive until public output");
        let projections = store.projection_snapshot();
        if projections.run_state(&fixture.run_id) == store::RunState::Started
            && matches!(
                projections.public_output(
                    &fixture.run_id,
                    &fixture.runtime_spec.spec().public_outputs.public_schema_id,
                ),
                Some(store::PublicOutputProjection::Produced { .. })
            )
        {
            return;
        }
    }
    panic!("public output was not produced");
}

fn fixture_with_first_managed_write_state() -> Fixture {
    let mut fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let managed_effect = EffectKind::new(
        "mfm.test",
        "managed-write",
        DigestAlgorithm::Sha256JcsV1,
        D8,
    )
    .expect("managed effect");
    let managed_cap = CapabilityDescriptor::new(
        CapabilityKind::new(
            "mfm.test",
            "managed-store",
            DigestAlgorithm::Sha256JcsV1,
            D9,
        )
        .expect("managed cap kind"),
        CapabilityVersion::new("mfm.cap.managed_store.v1").expect("managed cap version"),
        CapabilityRole::ManagedPlatformWrite,
        "managed-store",
    )
    .expect("managed cap");
    let managed_caps = CapabilitySetDescriptor::new(vec![managed_cap]).expect("managed caps");
    for node in &mut envelope.spec.nodes {
        if node.descriptor_id == fixture.descriptor_a {
            node.effect_kind = managed_effect.clone();
            node.capability_bindings = managed_caps.clone();
        }
    }
    for descriptor in &mut envelope.spec.descriptor_identities {
        if let spec::DescriptorIdentity::State(identity) = descriptor {
            if identity.descriptor_id == fixture.descriptor_a {
                identity.effect_kind = managed_effect.clone();
                identity.effect_class = "managed-write".to_owned();
                identity.effect_name = "managed-write".to_owned();
                identity.capabilities = managed_caps.clone();
                identity.runner = "managed-write".to_owned();
            }
        }
    }
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn fixture_with_first_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

fn fixture_with_first_exclusive_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::Exclusive,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

fn fixture_with_first_finalized_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

fn fixture_with_first_exact_touched_set_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ExactTouchedSet,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

fn fixture_with_first_exact_touched_set_finalized_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::Chained,
        RuntimeSideEffectClaim::ExactTouchedSet,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

fn fixture_with_manual_resolution_side_effect_state() -> Fixture {
    let mut fixture = fixture_with_first_side_effect_state();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let manual = spec::ManualResolutionEvidenceSpec {
        evidence_schema: fixture.seed_ref.schema_id.clone(),
        authorization: manual_authorization(0xe0),
    };
    envelope.spec.saga = spec::SagaPolicySpec::ManualResolution { manual };
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
            "mfm.test.manual.verifier.{byte}"
        ))
        .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new(format!(
                "mfm.test.manual.authority.{byte}"
            ))
            .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                    .expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(
                    "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                )
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

fn fixture_with_independent_second_node_and_first_side_effect_state() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::IndependentSecond,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Receipt,
    )
}

fn fixture_with_independent_second_node_and_first_exclusive_finalized_side_effect_state() -> Fixture
{
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::IndependentSecond,
        RuntimeSideEffectClaim::Exclusive,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

fn fixture_with_two_side_effects_and_failing_tail() -> Fixture {
    runtime_side_effect_fixture(
        RuntimeSideEffectFixtureShape::CompensatingPairWithFailingTail,
        RuntimeSideEffectClaim::ManualOnly,
        spec::SideEffectVerificationSpec::Finalized { depth: 1 },
    )
}

struct NodeSpecFixture {
    node_id: NodeId,
    descriptor_id: DescriptorId,
    scope_id: ScopeId,
    state_name: &'static str,
    state_kind: StateKind,
    state_version: StateVersion,
    effect_kind: EffectKind,
    config_ref: spec::ConfigRef,
    input_schema: SchemaId,
    input_cell: CellId,
    input_lineage: spec::ValueLineageRef,
    output_cell: CellId,
    output_schema: SchemaId,
    semantic: SemanticTypeId,
    caps: CapabilitySetDescriptor,
    predecessors: Vec<NodeId>,
    adapter_bindings: Vec<spec::AdapterBinding>,
    planning: spec::PlanningLineage,
}

fn node_spec(fixture: NodeSpecFixture) -> spec::NodeSpec {
    spec::NodeSpec {
        node_id: fixture.node_id,
        stable_key: spec::StableAuthorKey::new(
            fixture.state_name.rsplit('.').next().expect("state key"),
        )
        .expect("stable key"),
        scope_id: fixture.scope_id,
        state_kind: fixture.state_kind,
        state_version: fixture.state_version,
        descriptor_id: fixture.descriptor_id,
        config_ref: fixture.config_ref,
        input_bindings: spec::InputBindingSpec {
            input_schema_id: fixture.input_schema,
            input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D6),
            root: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                field_path: spec::PublicFieldPath::new("input").expect("field"),
                cell_id: fixture.input_cell,
                semantic_type_id: fixture.semantic.clone(),
                schema_id: fixture.output_schema.clone(),
                required_terminal: spec::RequiredTerminal::ProducedOnly,
                value_lineage: fixture.input_lineage,
            })),
            digest: content(0x73),
        },
        output_cell: fixture.output_cell,
        effect_kind: fixture.effect_kind,
        capability_bindings: fixture.caps,
        adapter_bindings: fixture.adapter_bindings,
        side_effect: None,
        framework: None,
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: fixture.planning,
        deterministic_predecessors: fixture.predecessors,
    }
}

fn state_descriptor(
    node: &spec::NodeSpec,
    descriptor_id: DescriptorId,
    name: &str,
    effect_kind: EffectKind,
    capabilities: CapabilitySetDescriptor,
    runner: &str,
) -> spec::StateDescriptorIdentity {
    spec::StateDescriptorIdentity {
        descriptor_id,
        name: name.to_owned(),
        state_kind: node.state_kind.clone(),
        state_version: node.state_version.clone(),
        config_schema_id: node.config_ref.schema_id.clone(),
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        output_schema_id: node
            .input_bindings
            .root
            .clone()
            .first_schema_or(node.config_ref.schema_id.clone()),
        output_semantic_type_id: match &node.input_bindings.root {
            spec::InputBindingNodeSpec::Cell(cell) => cell.semantic_type_id.clone(),
            _ => panic!("test input"),
        },
        effect_kind,
        effect_class: runner.to_owned(),
        effect_name: runner.to_owned(),
        effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
        capabilities,
        runner: runner.to_owned(),
        emitted_fact_descriptors: Vec::new(),
        side_effect_contract_digest: None,
    }
}

trait FirstSchema {
    fn first_schema_or(&self, fallback: SchemaId) -> SchemaId;
}

impl FirstSchema for spec::InputBindingNodeSpec {
    fn first_schema_or(&self, fallback: SchemaId) -> SchemaId {
        match self {
            spec::InputBindingNodeSpec::Cell(cell) => cell.schema_id.clone(),
            _ => fallback,
        }
    }
}

fn content(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
    match node {
        spec::InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
        spec::InputBindingNodeSpec::Cell(cell) => serde_json::json!({
            "cell_id": cell.cell_id.as_str(),
            "field_path": cell.field_path.as_str(),
            "kind": "cell",
            "required_terminal": match cell.required_terminal {
                spec::RequiredTerminal::ProducedOnly => "produced_only",
                spec::RequiredTerminal::MaybeSkipped => "maybe_skipped",
            },
            "schema_id": cell.schema_id.as_str(),
            "semantic_type_id": cell.semantic_type_id.as_str(),
            "value_lineage": cell.value_lineage.lineage_digest.as_str(),
        }),
        spec::InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "tuple",
        }),
        spec::InputBindingNodeSpec::Struct(fields) => serde_json::json!({
            "fields": fields.iter().map(|field| {
                serde_json::json!({
                    "field_path": field.field_path.as_str(),
                    "node": input_node_json(&field.node),
                })
            }).collect::<Vec<_>>(),
            "kind": "struct",
        }),
        spec::InputBindingNodeSpec::Vec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "vec",
            "ordering": ordering_json(*ordering),
        }),
        spec::InputBindingNodeSpec::NonEmptyVec {
            elements,
            ordering,
            domain_keys,
        } => serde_json::json!({
            "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
            "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
            "kind": "non_empty_vec",
            "ordering": ordering_json(*ordering),
        }),
    }
}

fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
    match ordering {
        spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
        spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
    }
}

fn stable_domain_key_ref_json(key: &spec::StableDomainKeyRef) -> serde_json::Value {
    serde_json::json!({
        "content_digest": key.content_digest.as_str(),
        "schema_id": key.schema_id.as_str(),
    })
}

fn artifact(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn side_effect_capability_kind() -> CapabilityKind {
    CapabilityKind::new(
        "mfm.test",
        "external-mutation",
        DigestAlgorithm::Sha256JcsV1,
        D9,
    )
    .expect("side-effect cap kind")
}

fn side_effect_capability_version() -> CapabilityVersion {
    CapabilityVersion::new("mfm.cap.external_mutation.v1").expect("side-effect cap version")
}

fn exclusive_resource_namespace() -> spec::ResourceNamespace {
    spec::ResourceNamespace::new("mfm.test.wallet_nonce").expect("resource namespace")
}

fn exact_touched_set_resource_namespace() -> spec::ResourceNamespace {
    spec::ResourceNamespace::new("mfm.test.wallet_nonce").expect("resource namespace")
}

fn exclusive_resource_key(fixture: &Fixture, value: &str) -> events::ResourceKeyEvidence {
    resource_key_in_namespace(fixture, exclusive_resource_namespace(), value)
}

fn resource_key_in_namespace(
    fixture: &Fixture,
    namespace: spec::ResourceNamespace,
    value: &str,
) -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace,
        key_schema_id: fixture.seed_ref.schema_id.clone(),
        key: events::ResourceKey::new(value).expect("resource key"),
    }
}

fn side_effect_ledger_key(attempt_no: u32) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("ledger-{attempt_no}")).expect("ledger key")
}

fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

fn forward_ledger_for_node(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
) -> events::SideEffectLedgerKey {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == *node_id
            && matches!(
                projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            )
        {
            assert!(
                found.replace(projection.ledger_key.clone()).is_none(),
                "node {node_id} has multiple forward ledgers"
            );
        }
    }
    found.expect("forward ledger for node")
}

fn side_effect_projection_for_run_node<P: std::borrow::Borrow<store::ProjectionSnapshot>>(
    projections: P,
    run_id: &RunId,
    node_id: &NodeId,
) -> Option<store::SideEffectProjection> {
    let projections = projections.borrow();
    projections.side_effects().find_map(|(_, projection)| {
        (projection.run_id == *run_id && projection.intent.node_id == *node_id)
            .then(|| projection.clone())
    })
}

async fn drive_until_side_effect_confirmation_without_output(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    output_cell: &CellId,
    context: &str,
) {
    for _ in 0..8 {
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        if side_effect_projection_for_run_node(&projection_snapshot, &fixture.run_id, &node.node_id)
            .is_some_and(|projection| {
                matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ) && projection_snapshot.cell_terminal(output_cell).is_none()
            })
        {
            return;
        }
    }
    panic!("{context} was not reached");
}

async fn drive_until_cells_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    cells: &[CellId],
    context: &str,
) {
    for _ in 0..24 {
        if cells
            .iter()
            .all(|cell| store.projection_snapshot().cell_terminal(cell).is_some())
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
    }
    panic!("{context} did not become terminal");
}

async fn drive_until_side_effect_attempt_phase(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    mut phase_matches: impl FnMut(&store::SideEffectPhase) -> bool,
    context: &str,
) {
    for _ in 0..24 {
        let projection_snapshot = store.projection_snapshot();
        if side_effect_projection_for_attempt(
            &fixture.runtime_spec,
            &fixture.run_id,
            &projection_snapshot,
            node,
            attempt_id,
        )
        .expect(context)
        .is_some_and(|projection| phase_matches(&projection.phase))
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
    }
    panic!("{context} was not reached");
}

#[derive(Clone, Copy)]
enum RemediationPhaseCheckpoint {
    SubmissionObserved,
    ConfirmationObserved,
}

async fn drive_until_remediation_phase(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    forward_pair_id: &SideEffectPairId,
    checkpoint: RemediationPhaseCheckpoint,
    context: &str,
) {
    for _ in 0..8 {
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        if remediation_projection_for_forward_pair(&projection_snapshot, forward_pair_id)
            .is_some_and(|projection| match checkpoint {
                RemediationPhaseCheckpoint::SubmissionObserved => {
                    matches!(
                        projection.phase,
                        store::SideEffectPhase::SubmissionObserved { .. }
                    )
                }
                RemediationPhaseCheckpoint::ConfirmationObserved => {
                    matches!(
                        projection.phase,
                        store::SideEffectPhase::ConfirmationObserved { .. }
                    )
                }
            })
        {
            return;
        }
    }
    panic!("{context} was not reached");
}

async fn drive_until_compensated_before_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) {
    for _ in 0..24 {
        let saga = derive_fixture_saga(fixture, store.projection_snapshot());
        if saga.run_mode == store::RunMode::Compensated
            && store.projection_snapshot().run_state(&fixture.run_id) != store::RunState::Completed
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive until compensated before terminal"),
            SchedulerStatus::Advanced
        );
    }
    panic!("compensated pre-terminal boundary was not reached");
}

fn remediation_intent_forward_links(
    store: &TestTypedRunStore,
    run_id: &RunId,
) -> Vec<SideEffectPairId> {
    store
        .load_run_stream(run_id)
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                match &payload.ledger_purpose {
                    events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                        Some(forward_pair_id.clone())
                    }
                    events::SideEffectLedgerPurpose::Forward => None,
                }
            }
            _ => None,
        })
        .collect()
}

fn remediation_projection_for_forward_pair<P: std::borrow::Borrow<store::ProjectionSnapshot>>(
    projections: P,
    forward_pair_id: &SideEffectPairId,
) -> Option<store::SideEffectProjection> {
    let projections = projections.borrow();
    projections.side_effects().find_map(|(_, projection)| {
        matches!(
            &projection.ledger_purpose,
            events::SideEffectLedgerPurpose::Remediation {
                forward_pair_id: linked,
            } if linked == forward_pair_id
        )
        .then(|| projection.clone())
    })
}

fn assert_no_duplicate_side_effect_submissions(store: &TestTypedRunStore, run_id: &RunId) {
    let mut by_ledger = BTreeMap::<events::SideEffectLedgerKey, usize>::new();
    let mut forward_by_node = BTreeMap::<NodeId, usize>::new();
    let mut remediation_by_forward = BTreeMap::<SideEffectPairId, usize>::new();
    for event in store.load_run_stream(run_id) {
        if let events::KernelEventPayload::SideEffectSubmissionObserved(payload) = event.payload() {
            *by_ledger.entry(payload.ledger_key.clone()).or_default() += 1;
            match &payload.ledger_purpose {
                events::SideEffectLedgerPurpose::Forward => {
                    *forward_by_node.entry(payload.node_id.clone()).or_default() += 1;
                }
                events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                    *remediation_by_forward
                        .entry(forward_pair_id.clone())
                        .or_default() += 1;
                }
            }
        }
    }
    for (ledger, count) in by_ledger {
        assert_eq!(count, 1, "duplicate submission for ledger {ledger}");
    }
    for (node, count) in forward_by_node {
        assert_eq!(count, 1, "duplicate forward submission for node {node}");
    }
    for (forward_pair, count) in remediation_by_forward {
        assert_eq!(
            count, 1,
            "duplicate remediation submission for forward pair {forward_pair}"
        );
    }
}

fn side_effect_submission_count_for_ledger(
    store: &TestTypedRunStore,
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if &payload.ledger_key == ledger_key
            )
        })
        .count()
}

fn remediation_submission_count_for_forward_pair(
    store: &TestTypedRunStore,
    run_id: &RunId,
    forward_pair_id: &SideEffectPairId,
) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if matches!(
                        &payload.ledger_purpose,
                        events::SideEffectLedgerPurpose::Remediation {
                            forward_pair_id: linked,
                        } if linked == forward_pair_id
                    )
            )
        })
        .count()
}

fn side_effect_fixture_digest(ctx: &ErasedRunCtx<'_>, role: &str) -> ContentDigest {
    content_digest_json(serde_json::json!({
        "attempt": ctx.attempt_id().as_str(),
        "node": ctx.node().node_id.as_str(),
        "role": role,
    }))
    .expect("side-effect fixture digest")
}

struct SideEffectFixtureIntentOutput {
    ledger: events::SideEffectLedgerKey,
    staged_artifact: StagedArtifact,
    payload: RunnerEventPayload,
}

fn side_effect_fixture_intent_output(
    ctx: &ErasedRunCtx<'_>,
    ledger_purpose: events::SideEffectLedgerPurpose,
    invocation_epoch: u32,
) -> Result<SideEffectFixtureIntentOutput> {
    let ledger = side_effect_ledger_key_for_ctx(ctx);
    let (pair_id, pair_role) =
        side_effect_pair_fields_for_ctx(ctx, &ledger_purpose, events::SideEffectPairRole::Submit);
    let (intent_artifact_id, intent_hash) = side_effect_fixture_artifact_pair(ctx, "intent");
    let staged_artifact = staged_side_effect_artifact(
        ctx,
        side_effect_artifact(
            ctx,
            intent_artifact_id.clone(),
            intent_hash.clone(),
            events::ArtifactRole::SideEffectIntent,
        ),
        ledger.clone(),
        invocation_epoch,
    )?;
    let adapter_binding = ctx
        .node()
        .adapter_bindings
        .first()
        .expect("side-effect adapter");
    let payload =
        RunnerEventPayload::SideEffectIntentPersisted(events::side_effect::IntentPersisted {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            scope_id: ctx.node().scope_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            ledger_key: ledger.clone(),
            ledger_purpose,
            pair_id,
            pair_role,
            invocation_epoch,
            intent_schema_id: ctx.node().config_ref.schema_id.clone(),
            intent_hash,
            intent_artifact_id,
            idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
            idempotency_input_hash: side_effect_fixture_digest(ctx, "idempotency"),
            idempotency_key: events::IdempotencyKeyRef::new("idem-1").expect("idempotency key"),
            capability_kind: side_effect_capability_kind(),
            capability_version: side_effect_capability_version(),
            adapter_kind: adapter_binding.adapter_kind.clone(),
            adapter_version: adapter_binding.adapter_version.clone(),
        });
    Ok(SideEffectFixtureIntentOutput {
        ledger,
        staged_artifact,
        payload,
    })
}

fn side_effect_fixture_artifact_pair(
    ctx: &ErasedRunCtx<'_>,
    role: &str,
) -> (ArtifactId, ContentDigest) {
    let digest = side_effect_fixture_digest(ctx, role);
    (
        ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
    )
}

fn side_effect_ledger_key_for_ctx(ctx: &ErasedRunCtx<'_>) -> events::SideEffectLedgerKey {
    if let Some(forward_pair_id) = linked_forward_pair_for_remediation(ctx) {
        events::SideEffectLedgerKey::new(format!(
            "remediation-{}-{}",
            forward_pair_id,
            ctx.attempt_no()
        ))
        .expect("remediation ledger key")
    } else {
        events::SideEffectLedgerKey::new(format!(
            "forward-{}-{}-{}",
            ctx.run_id(),
            ctx.node().node_id,
            ctx.attempt_no()
        ))
        .expect("forward ledger key")
    }
}

fn side_effect_ledger_purpose_for_ctx(ctx: &ErasedRunCtx<'_>) -> events::SideEffectLedgerPurpose {
    linked_forward_pair_for_remediation(ctx)
        .map(|forward_pair_id| events::SideEffectLedgerPurpose::Remediation { forward_pair_id })
        .unwrap_or(events::SideEffectLedgerPurpose::Forward)
}

fn forward_pair_for_ledger(
    projections: &store::ProjectionSnapshot,
    forward_ledger_key: &events::SideEffectLedgerKey,
) -> SideEffectPairId {
    projections
        .side_effects()
        .find_map(|(_, projection)| {
            (projection.ledger_key == *forward_ledger_key).then(|| projection.pair_id.clone())
        })
        .expect("forward pair projection")
}

fn linked_forward_pair_for_remediation(ctx: &ErasedRunCtx<'_>) -> Option<SideEffectPairId> {
    if let Some(projection) = side_effect_projection_for_attempt(
        ctx.runtime_spec(),
        ctx.run_id(),
        ctx.projections(),
        ctx.node(),
        ctx.attempt_id(),
    )
    .expect("remediation projection lookup")
    {
        if let events::SideEffectLedgerPurpose::Remediation { forward_pair_id } =
            &projection.ledger_purpose
        {
            return Some(forward_pair_id.clone());
        }
    }
    let forward_node_id = ctx
        .runtime_spec()
        .forward_node_for_remediation(&ctx.node().node_id)?;
    ctx.projections()
        .side_effects()
        .find_map(|(_, projection)| {
            (projection.intent.node_id == *forward_node_id
                && matches!(
                    &projection.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                )
                && matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ))
            .then(|| projection.pair_id.clone())
        })
}

fn side_effect_claim_owner(attempt_no: u32, generation: u32) -> events::RunnerInvocationId {
    events::RunnerInvocationId::new(format!("owner-{attempt_no}-{generation}"))
        .expect("claim owner")
}

fn side_effect_fencing_token(
    attempt_no: u32,
    generation: u32,
) -> events::side_effect::ClaimFencingToken {
    events::side_effect::ClaimFencingToken::new(format!("token-{attempt_no}-{generation}"))
        .expect("fencing token")
}

fn side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(ctx.node().config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn runner_side_effect_binding_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> RunnerSideEffectBinding {
    let ledger_purpose = side_effect_ledger_purpose_for_ctx(ctx);
    let (pair_id, _) =
        side_effect_pair_fields_for_ctx(ctx, &ledger_purpose, events::SideEffectPairRole::Submit);
    RunnerSideEffectBinding {
        ledger_key: ledger,
        ledger_purpose,
        pair_id,
        invocation_epoch,
    }
}

fn runner_claim_binding_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    claim_generation: u32,
) -> RunnerClaimBinding {
    RunnerClaimBinding {
        claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
    }
}

fn side_effect_claimed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerPayloadBuilder::new(ctx).side_effect_claimed(
        runner_side_effect_binding_for_ctx(ctx, ledger, invocation_epoch),
        runner_claim_binding_for_ctx(ctx, claim_generation),
    )
}

fn side_effect_prepared(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    let claim = runner_claim_binding_for_ctx(ctx, claim_generation);
    RunnerPayloadBuilder::new(ctx)
        .side_effect_invocation_prepared(
            runner_side_effect_binding_for_ctx(ctx, ledger, invocation_epoch),
            None,
            RunnerPreparedInvocationBinding {
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token,
                resource_key: None,
            },
        )
        .expect("side-effect prepared payload")
}

fn side_effect_failed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    failure_phase: events::side_effect::FailurePhase,
    retryable: bool,
) -> RunnerEventPayload {
    RunnerPayloadBuilder::new(ctx).side_effect_failed(
        runner_side_effect_binding_for_ctx(ctx, ledger, invocation_epoch),
        events::SideEffectPairRole::Submit,
        failure_phase,
        retryable,
        side_effect_error(retryable),
    )
}

fn side_effect_error(retryable: bool) -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("sidefx_failed").expect("error code"),
        category: events::ErrorCategory::SideEffect,
        retryable,
        safe_message: "side-effect failed".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}

fn public_output_error() -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("public_output_render_failed").expect("error code"),
        category: events::ErrorCategory::Runtime,
        retryable: true,
        safe_message: "public output render failed".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}
