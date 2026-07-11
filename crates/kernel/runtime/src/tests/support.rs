use super::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use mfm_canonical::{sha256_digest_bytes, CanonicalValue};
use mfm_capabilities::{
    CapabilityDescriptor, CapabilityRole, CapabilitySetDescriptor, CapabilitySpec, EffectSpec,
    ExternalMutationAuthorityRole, ManagedPlatformWrite, ReadExternalRole,
};
use mfm_ids::{
    ArtifactId, ContextRef, ContextResourceKind, ContextStage, DigestBytes, EffectKind,
    EffectVersion, EventId, SchemaId, ScopeId, SeedId, SemanticTypeId, SideEffectPairId, StateKind,
    StateVersion, StoreScopeId,
};
use mfm_manual_auth::{
    ManualAuthorizationSignatureBytes, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, IdempotencyKey, MfmContext,
    NoContext, PublicOutputKey, PureState, ReadState, RemediationNodeParams, ResourceClaim,
    RootBuilder, ScopeKey, SideEffectNodeParams, SideEffectSagaPolicy, SideEffectState, StateKey,
    StateRegistryBuilder, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, PublicOutputs};
use mfm_store::v1::{
    self as store,
    test_support::{
        event_id_for_envelope_inputs_for_test as test_event_id_for_envelope_inputs,
        fact_query_receipt_for_test as test_fact_query_receipt,
        prepared_commit_bundle_from_plan as test_bundle_from_plan,
        prepared_commit_plan_for_test as test_prepared_commit_plan,
        FactQueryReceiptFixtureInputForTest,
    },
    RetainedArtifactReadProvider, RunEventStore,
};
use mfm_values::ContextBoundOutput;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

use crate::commit::{CommitPlanner, RunnerOutputCommitInput};

#[path = "runner_kit.rs"]
mod runner_kit_tests;

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

fn fixture_store_scope_id() -> StoreScopeId {
    StoreScopeId::new("mfm.store_scope.v1:10101010101010101010101010101010")
        .expect("test store scope")
}

fn alternate_fixture_store_scope_id() -> StoreScopeId {
    StoreScopeId::new("mfm.store_scope.v1:20202020202020202020202020202020")
        .expect("alternate test store scope")
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
    let input = mfm_facts::FactQueryInput::new(
        store_scope.clone(),
        query_scope.clone(),
        mfm_facts::ScopeDecisionEvidence::new(content(0x42)),
        vec![mfm_facts::FactQueryPredicate::new(
            mfm_facts::FactFieldId::new("subject.amount").expect("field id"),
            mfm_facts::FactQueryOperator::Equal,
            mfm_facts::FactCanonicalScalar::UnsignedInteger(42),
        )],
        vec![mfm_facts::FactFieldId::new("result.amount").expect("field id")],
        mfm_facts::FactOrderingName::new("result.amount.asc").expect("ordering"),
        Some(10),
    )
    .expect("query input");
    let plan =
        mfm_facts::compile_fact_query_plan(&test_fact_descriptor(), input).expect("query plan");
    let frontier = mfm_facts::StoreReadFrontier::new(
        store_scope,
        query_scope,
        mfm_facts::DescriptorCatalogWatermark::new(1),
        mfm_facts::StoreCommitOrder::new(10),
    );
    let rows = returned_refs
        .into_iter()
        .map(|fact_ref| mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new()))
        .collect::<Vec<_>>();
    let receipt = test_fact_query_receipt(FactQueryReceiptFixtureInputForTest {
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
    let material = mfm_facts::FactSubjectMaterialV1::new(vec![mfm_facts::FactFieldValue::new(
        mfm_facts::FactFieldId::new("subject.amount").expect("field"),
        mfm_facts::FactFieldValueType::UnsignedInteger,
        mfm_facts::FactCanonicalScalar::UnsignedInteger(subject_amount),
    )
    .expect("subject value")])
    .expect("subject material");
    let namespace_hash =
        mfm_facts::fact_subject_namespace_hash(descriptor).expect("fact subject namespace hash");
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
    let descriptor_fixture =
        store::test_support::fact_descriptor_projection_fixture_for_test(descriptor.clone())
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
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
                self.inner
                    .acquire_execution_claim(scope, holder_run_id, token)
            }

            fn execution_claim_status<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
            ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
                self.inner.execution_claim_status(scope)
            }

            fn renew_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
                self.inner
                    .renew_execution_claim(scope, holder_run_id, token)
            }

            fn release_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                self.inner
                    .release_execution_claim(scope, holder_run_id, token)
            }

            fn expired_execution_claims<'a>(
                &'a self,
            ) -> store::AsyncStoreFuture<'a, Vec<store::ExpiredExecutionClaim>, Self::Error> {
                self.inner.expired_execution_claims()
            }

            fn reap_expired_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                self.inner
                    .reap_expired_execution_claim(scope, holder_run_id, token)
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
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
                let result = block_on_ready(self.inner.borrow_mut().acquire_execution_claim(
                    scope,
                    holder_run_id,
                    token,
                ));
                Box::pin(std::future::ready(result))
            }

            fn execution_claim_status<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
            ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
                let result = block_on_ready(self.inner.borrow().execution_claim_status(scope));
                Box::pin(std::future::ready(result))
            }

            fn renew_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
                let result = block_on_ready(self.inner.borrow_mut().renew_execution_claim(
                    scope,
                    holder_run_id,
                    token,
                ));
                Box::pin(std::future::ready(result))
            }

            fn release_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                let result = block_on_ready(self.inner.borrow_mut().release_execution_claim(
                    scope,
                    holder_run_id,
                    token,
                ));
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
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                let result = block_on_ready(self.inner.borrow_mut().reap_expired_execution_claim(
                    scope,
                    holder_run_id,
                    token,
                ));
                Box::pin(std::future::ready(result))
            }
        }
    };
}

#[path = "store_doubles.rs"]
mod store_doubles;
use self::store_doubles::*;
#[path = "state_fixtures.rs"]
mod state_fixtures;
use self::state_fixtures::*;
#[path = "runner_fixtures.rs"]
mod runner_fixtures;
use self::runner_fixtures::*;
#[path = "fixture_builders.rs"]
mod fixture_builders;
use self::fixture_builders::*;
#[path = "side_effect_helpers.rs"]
mod side_effect_helpers;
use self::side_effect_helpers::*;
#[path = "side_effect_runners.rs"]
mod side_effect_runners;
use self::side_effect_runners::*;

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
    run_identity_material_with_invocation(runtime_spec, content(0x10))
}

fn run_identity_material_with_invocation(
    runtime_spec: &CertifiedRuntimeSpec,
    invocation_key_digest: ContentDigest,
) -> events::RunIdentityMaterialV1 {
    run_identity_material_with_scope_and_invocation(
        runtime_spec,
        fixture_store_scope_id(),
        invocation_key_digest,
    )
}

fn run_identity_material_with_scope_and_invocation(
    runtime_spec: &CertifiedRuntimeSpec,
    store_scope_id: StoreScopeId,
    invocation_key_digest: ContentDigest,
) -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash: runtime_spec.spec_hash().clone(),
        store_scope_id,
        invocation_key_digest,
    }
}

fn fixture_run_identity_material(fixture: &Fixture) -> events::RunIdentityMaterialV1 {
    run_identity_material_with_scope_and_invocation(
        &fixture.runtime_spec,
        fixture.store_scope_id.clone(),
        fixture.invocation_key_digest.clone(),
    )
}

fn refresh_fixture_run_id(fixture: &mut Fixture) {
    let invocation_key_digest = fixture.invocation_key_digest.clone();
    refresh_fixture_run_id_with_invocation(fixture, invocation_key_digest);
}

fn refresh_fixture_run_id_with_invocation(
    fixture: &mut Fixture,
    invocation_key_digest: ContentDigest,
) {
    let store_scope_id = fixture.store_scope_id.clone();
    refresh_fixture_run_id_with_identity(fixture, store_scope_id, invocation_key_digest);
}

fn refresh_fixture_run_id_with_identity(
    fixture: &mut Fixture,
    store_scope_id: StoreScopeId,
    invocation_key_digest: ContentDigest,
) {
    fixture.store_scope_id = store_scope_id.clone();
    fixture.invocation_key_digest = invocation_key_digest.clone();
    fixture.run_id = run_identity_material_with_scope_and_invocation(
        &fixture.runtime_spec,
        store_scope_id,
        invocation_key_digest,
    )
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
    store_scope_id: StoreScopeId,
    invocation_key_digest: ContentDigest,
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
        Ok(SideEffectIntentPlan::new(
            fixture_side_effect_evidence(21, node_id.clone(), attempt_id.clone()),
            fixture_side_effect_evidence(34, node_id, attempt_id),
            events::IdempotencyKeyRef::new("mfm.test.driver.idem").expect("idempotency key"),
            RunnerCapabilityBinding {
                capability_kind: self.cap_kind.clone(),
                capability_version: self.cap_version.clone(),
                adapter_kind: self.adapter_kind.clone(),
                adapter_version: self.adapter_version.clone(),
            },
        ))
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
    ) -> SideEffectDriverFuture<'a, Option<Self::PreparedInvocation>> {
        let node_id = ctx.node().node_id.as_str().to_owned();
        let attempt_id = ctx.attempt_id().as_str().to_owned();
        Box::pin(async move {
            Ok(Some(serde_json::json!({
                "attempt_id": attempt_id,
                "node_id": node_id,
                "prepared": true
            })))
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
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectSubmissionDecision<
            Self::Submission,
            Self::SubmissionUnknownEvidence,
            Self::NotSubmittedProof,
            Self::AmbiguityEvidence,
        >,
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
    ) -> SideEffectDriverFuture<
        'a,
        SideEffectUnknownSubmissionDecision<
            Self::Submission,
            Self::NotSubmittedProof,
            Self::AmbiguityEvidence,
        >,
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
            Ok(SideEffectObservedEvidence::new(
                evidence,
                test_driver_replay_evidence(),
            ))
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
            Ok(SideEffectObservedEvidence::new(
                evidence,
                test_driver_replay_evidence(),
            ))
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
    SideEffectReplayEvidence::new(
        events::ReplayVerifierId::new("mfm.test.driver.verifier").expect("verifier"),
        None,
    )
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
            context: $fixture
                .runtime_spec
                .invocation_context_for_node(invocation_node)
                .expect("invocation context"),
            attempt_id: $attempt_id,
            attempt_no: 1,
            config_artifact,
            inputs: MaterializedInputs {
                input_schema_id: invocation_node.input_bindings.input_schema_id.clone(),
                root: MaterializedInputNode::Unit,
            },
            caps: CertifiedRuntimeCapabilities::for_node(invocation_node),
            recorded_facts: RecordedFacts::default(),
            projections: &projections,
            run_stream: &run_stream,
            view: &view,
        };
        let $ctx = ErasedRunCtx::from_prepared(&invocation);
        $body
    }};
}

#[path = "execution_lifecycle.rs"]
mod execution_lifecycle_tests;
#[path = "manual_resolution.rs"]
mod manual_resolution_tests;
#[path = "recovery.rs"]
mod recovery_tests;
#[path = "saga_terminal.rs"]
mod saga_terminal_tests;
#[path = "side_effect_boundaries.rs"]
mod side_effect_boundaries_tests;
#[path = "side_effect_driver.rs"]
mod side_effect_driver_tests;

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
        store_commit_order: event.store_commit_order(),
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
        store_commit_order: event.store_commit_order(),
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
    let execution_scope = execution_claim_scope(store, run_id).await?;
    let token = execution_claim_token(store, &execution_scope, run_id).await?;
    scheduler
        .drive_once(store, runtime_spec, run_id, &execution_scope, token)
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
    let execution_scope = execution_claim_scope(store, run_id).await?;
    let token = execution_claim_token(store, &execution_scope, run_id).await?;
    scheduler
        .drive_until_blocked(store, runtime_spec, run_id, &execution_scope, token)
        .await
}

async fn execution_claim_scope<S>(store: &S, run_id: &RunId) -> Result<store::ExecutionClaimScope>
where
    S: store::RunEventStore + ?Sized,
{
    let committed = store
        .load_committed_run_stream(run_id)
        .await
        .map_err(crate::error::async_store_error)?;
    committed
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(
                store::ExecutionClaimScope::from_run_identity_material(&payload.identity_material),
            ),
            _ => None,
        })
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!("run {run_id} has no RunAdmitted event"))
        })
}

async fn execution_claim_token<S>(
    store: &S,
    execution_scope: &store::ExecutionClaimScope,
    run_id: &RunId,
) -> Result<store::AdmissionToken>
where
    S: store::ExecutionClaimStore + ?Sized,
{
    loop {
        match store
            .execution_claim_status(execution_scope)
            .await
            .map_err(crate::error::async_store_error)?
        {
            store::ExecutionClaimStatus::Live(lease) => return Ok(lease.token),
            store::ExecutionClaimStatus::Expired(lease) => {
                store
                    .reap_expired_execution_claim(
                        execution_scope,
                        &lease.holder_run_id,
                        &lease.token,
                    )
                    .await
                    .map_err(crate::error::async_store_error)?;
            }
            store::ExecutionClaimStatus::Unclaimed => {
                let token = store::AdmissionToken::new(format!(
                    "mfm.test.runtime.execution_claim:{run_id}"
                ))?;
                match store
                    .acquire_execution_claim(execution_scope, run_id, token)
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

fn seed_cell_artifact_evidence(
    seed_id: &SeedId,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    content_digest: ContentDigest,
    byte_len: u64,
) -> events::ArtifactEvidenceRef {
    let store_evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(content_digest.algorithm(), *content_digest.digest()),
        digest: content_digest.clone(),
        byte_len,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(schema_id.clone()),
        semantic_type_id: Some(semantic_type_id),
        producer_node_id: None,
        producer_seed_id: Some(seed_id.clone()),
        artifact_role: events::ArtifactRole::SeedInput,
    };
    events::ArtifactEvidenceRef {
        artifact_id: store_evidence.artifact_id.clone(),
        role: store_evidence.artifact_role,
        schema_id,
        semantic_type_id: store_evidence.semantic_type_id.clone(),
        content_digest,
        evidence_hash: store_evidence
            .evidence_hash()
            .expect("seed artifact evidence hash"),
        byte_len,
        media_type: store_evidence.media_type,
    }
}

fn terminal_payloads(
    ctx: &ErasedRunCtx<'_>,
    state_evidence: &store::ArtifactEvidenceRef,
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
        context: ctx.output_cell().context.clone(),
        artifact_id: state_evidence.artifact_id.clone(),
        content_digest: state_evidence.digest.clone(),
        evidence_hash: state_evidence
            .evidence_hash()
            .expect("state output evidence hash"),
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
    ErasedRunnerOutput::from_parts(
        staged_artifacts,
        staged_retention_refs,
        terminal_payloads(ctx, state_evidence),
    )
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
        context_output_extractor: None,
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
        | events::ArtifactStagingClass::AttemptExternalReadEvidence
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
ExternalReadEvidence -> attempt_external_read_evidence\n\
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
                    intent_artifact_evidence_hash: intent_artifact
                        .evidence_hash()
                        .expect("intent evidence hash"),
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
                    prepared_artifact_evidence_hash: None,
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
                submission_hash: digest.clone(),
                submission_artifact_id: artifact_id,
                submission_artifact_evidence_hash: evidence
                    .evidence_hash()
                    .expect("submission evidence hash"),
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
                    context: output_cell.context.clone(),
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
                receipt_hash: digest.clone(),
                receipt_artifact_id: artifact_id,
                receipt_artifact_evidence_hash: evidence
                    .evidence_hash()
                    .expect("receipt evidence hash"),
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
            confirmation_hash: digest.clone(),
            confirmation_artifact_id: artifact_id,
            confirmation_artifact_evidence_hash: evidence
                .evidence_hash()
                .expect("confirmation evidence hash"),
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
        .staged_artifacts()
        .iter()
        .map(|artifact| artifact.evidence().clone())
        .collect::<Vec<_>>();
    let payloads = output
        .payloads()
        .iter()
        .cloned()
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
            evidence_artifact_evidence_hash: evidence
                .evidence_hash()
                .expect("ambiguity evidence hash"),
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
    let evidence_hash = evidence
        .evidence_hash()
        .expect("manual terminal state output evidence hash");
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
                    context: output_cell.context.clone(),
                    artifact_id,
                    content_digest: output_digest,
                    evidence_hash,
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
                    proof_artifact_evidence_hash: evidence
                        .evidence_hash()
                        .expect("proof evidence hash"),
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
            .contains("schema:mfm.runtime.redacted_attempt_failure_diagnostic:2:sha256-jcs-v1:"),
        "diagnostic schema id should identify the runtime redacted failure diagnostic schema"
    );
    let retained = store
        .projection_snapshot()
        .retentions()
        .any(|(_, retention)| {
            retention
                .refs
                .values()
                .find(|retention_ref| retention_ref.artifact_id == diagnostic.artifact_id)
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

fn registered_context_bound_fixture_runners(
    fixture: &Fixture,
    source_runner: ContextSourceRunner,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(fixture.descriptor_a.clone(), "pure", source_runner))
        .expect("context source binding");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "pure",
            ContextConsumerRunner,
        ))
        .expect("context consumer binding");
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
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
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
        context: spec::CellContextSpec::no_context(),
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
        context: spec::NodeContextSpec::no_context(),
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
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
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
        context: spec::CellContextSpec::no_context(),
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
        context: spec::NodeContextSpec::no_context(),
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
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
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
        context: spec::CellContextSpec::no_context(),
    });
    typed.nodes.push(spec::NodeSpec {
        node_id: node_id.clone(),
        stable_key: spec::StableAuthorKey::new("framework/resolve-saga-terminal")
            .expect("stable key"),
        scope_id,
        state_kind,
        state_version,
        descriptor_id,
        context: spec::NodeContextSpec::no_context(),
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

fn content(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn artifact(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
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
