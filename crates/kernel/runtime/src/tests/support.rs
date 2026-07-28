use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityDescriptor, CapabilityRole, CapabilitySpec, ExternalMutationAuthorityRole,
    ReadExternalRole,
};
use mfm_ids::{
    ArtifactId, ContextRef, ContextResourceKind, ContextStage, DigestBytes, EventId, SchemaId,
    SeedId, SemanticTypeId, SideEffectPairId, StateKind, StateVersion, StoreScopeId,
};
use mfm_manual_auth::{
    ManualAuthorizationSignatureBytes, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
};
use mfm_program::{
    build_root_with_registries, AdapterBindingSpec, CanonicalSeed, MfmContext, NoContext,
    PublicOutputKey, PureState, ReadState, RemediationNodeParams, ResourceClaim, RootBuilder,
    ScopeKey, SideEffectNodeParams, SideEffectSagaPolicy, SideEffectState, StateKey,
    StateRegistryBuilder, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, PublicOutputs};
use mfm_store::v1::{
    self as store,
    test_support::{
        event_id_for_envelope_inputs_for_test as test_event_id_for_envelope_inputs,
        fact_query_receipt_for_test as test_fact_query_receipt,
        persisted_kernel_event_envelope_with_ordinal_for_test as test_persisted_event_with_ordinal,
        poll_ready_store_future_for_test,
        prepared_commit_bundle_from_plan as test_bundle_from_plan,
        prepared_commit_plan_for_test as test_prepared_commit_plan,
        FactQueryReceiptFixtureInputForTest, StaticRunJournalBackendForTest,
    },
    RunJournalStore,
};
use mfm_values::ContextBoundOutput;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

use crate::spec_authority::CurrentSpecRead;

#[test]
fn framework_authoring_catalog_matches_the_closed_certified_lifecycle() {
    let public_schema = <CertifierPublicOutputs<'static, 'static> as mfm_program::PublicOutputs<
        'static,
        'static,
    >>::public_schema_id()
    .expect("fixture public schema");
    let catalog = framework_authoring_catalog(&public_schema).expect("framework catalog");
    let (certified, _) = certifier_backed_runtime_authority();
    let lifecycle = certified.framework_lifecycle();
    let expected_ids = [
        lifecycle.render().descriptor_id(),
        lifecycle.retention().descriptor_id(),
        lifecycle.complete().descriptor_id(),
        lifecycle.resolve().descriptor_id(),
    ]
    .into_iter()
    .cloned()
    .collect::<BTreeSet<_>>();
    let catalog_ids = catalog
        .state_descriptors()
        .map(|descriptor| descriptor.descriptor_id.clone())
        .collect::<BTreeSet<_>>();

    assert_eq!(catalog_ids, expected_ids);
    for descriptor in catalog.state_descriptors() {
        assert!(catalog.is_framework_state(&descriptor.descriptor_id));
        assert_eq!(
            certified.descriptor_set().state(&descriptor.descriptor_id),
            Some(descriptor)
        );
    }
    assert_eq!(catalog.operation_descriptors().len(), 0);
    assert_eq!(catalog.capability_descriptors().len(), 0);
    assert_eq!(catalog.emitted_fact_descriptors().len(), 0);
    assert_eq!(catalog.adapter_bindings().len(), 0);
    assert_eq!(catalog.side_effect_state_descriptor_ids().len(), 0);
}

#[path = "fact_support.rs"]
mod fact_support;
#[path = "runner_kit.rs"]
mod runner_kit_tests;
use self::fact_support::{
    test_fact_claim, test_fact_key, test_fact_query_evidence, test_fact_response_artifact,
};

const D0: DigestBytes = DigestBytes::from_array([0x10; 32]);
const D1: DigestBytes = DigestBytes::from_array([0x11; 32]);
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

macro_rules! delegate_execution_claim_direct {
    ($inner:expr, $_borrow:ident, $method:ident, ()) => {
        $inner.$method()
    };
    ($inner:expr, $_borrow:ident, $method:ident, ($($arg:expr),*)) => {
        $inner.$method($($arg),*)
    };
}

macro_rules! delegate_execution_claim_store {
    ($ty:ty, $delegate:ident) => {
        impl store::ExecutionClaimStore for $ty {
            type Error = store::StoreError;

            fn acquire_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, store::NowaitSkipAdmissionResult, Self::Error> {
                $delegate!(
                    self.inner,
                    borrow_mut,
                    acquire_execution_claim,
                    (scope, holder_run_id, token)
                )
            }

            fn execution_claim_status<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
            ) -> store::AsyncStoreFuture<'a, store::ExecutionClaimStatus, Self::Error> {
                $delegate!(self.inner, borrow, execution_claim_status, (scope))
            }

            fn renew_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, Option<store::AdmissionLease>, Self::Error> {
                $delegate!(
                    self.inner,
                    borrow_mut,
                    renew_execution_claim,
                    (scope, holder_run_id, token)
                )
            }

            fn release_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                $delegate!(
                    self.inner,
                    borrow_mut,
                    release_execution_claim,
                    (scope, holder_run_id, token)
                )
            }

            fn expired_execution_claims<'a>(
                &'a self,
            ) -> store::AsyncStoreFuture<'a, Vec<store::ExpiredExecutionClaim>, Self::Error> {
                $delegate!(self.inner, borrow, expired_execution_claims, ())
            }

            fn reap_expired_execution_claim<'a>(
                &'a self,
                scope: &'a store::ExecutionClaimScope,
                holder_run_id: &'a RunId,
                token: &'a store::AdmissionToken,
            ) -> store::AsyncStoreFuture<'a, bool, Self::Error> {
                $delegate!(
                    self.inner,
                    borrow_mut,
                    reap_expired_execution_claim,
                    (scope, holder_run_id, token)
                )
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
#[path = "side_effect_callbacks.rs"]
mod side_effect_callbacks;
use self::side_effect_callbacks::*;
#[path = "stream_mutation_support.rs"]
mod stream_mutation_support;
use self::stream_mutation_support::*;
#[path = "side_effect_runners.rs"]
mod side_effect_runners;
use self::side_effect_runners::*;

fn side_effect_pair_fields_for_purpose<S>(
    runtime_spec: &S,
    node_id: &NodeId,
    ledger_purpose: &events::SideEffectLedgerPurpose,
    role: events::SideEffectPairRole,
) -> (SideEffectPairId, events::SideEffectPairRole)
where
    S: crate::spec_authority::CurrentSpecRead + ?Sized,
{
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
        &ctx.runtime_spec(),
        &ctx.node().node_id,
        ledger_purpose,
        role,
    )
}

fn run_identity_material(runtime_spec: &CertifiedRuntimeSpec) -> events::RunIdentityMaterialV1 {
    run_identity_material_with_scope_and_invocation(
        runtime_spec,
        fixture_store_scope_id(),
        content(0x10),
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

#[path = "launch_helpers.rs"]
mod launch_helpers;
use self::launch_helpers::*;
#[path = "event_helpers.rs"]
mod event_helpers;
use self::event_helpers::*;

fn validate_corrupted_journal_for_tests(
    store: &TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    records: Vec<store::KernelEventEnvelope>,
) -> Result<()> {
    let backend = StaticRunJournalBackendForTest::new(
        run_id.clone(),
        records,
        store.committed_artifact_authority_for_corruption(run_id),
    );
    let journal = poll_ready_store_future_for_test(backend.load_committed_journal(run_id))
        .map_err(RuntimeError::from)?;
    verify_current_run(journal, recertified_runtime_spec(runtime_spec)).map(|_| ())
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
    SerialTypedScheduler::new(registry)
}

fn fixture_scheduler(registry: ErasedRunnerRegistry, fixture: &Fixture) -> SerialTypedScheduler {
    test_scheduler(register_fixture_capabilities(registry, fixture))
}

fn register_fixture_read_runner(
    registry: &mut ErasedRunnerRegistry,
    fixture: &Fixture,
    runner_name: &'static str,
) {
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            runner_name,
            RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_digest: content(0xb2),
            },
        ))
        .expect("binding read");
}

fn register_default_fixture_pure_runner(registry: &mut ErasedRunnerRegistry, fixture: &Fixture) {
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
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
    let mut registry = test_runner_registry();
    registry
        .register(binding(fixture.descriptor_a.clone(), runner_name, runner))
        .expect("binding a");
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
    registry
}

fn side_effect_driver_registry_with_submission_decision(
    fixture: &Fixture,
    decision: TestSubmissionDecision,
) -> ErasedRunnerRegistry {
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture).with_submission_decision(decision),
        ))
        .expect("binding side effect");
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
    registry
}

macro_rules! assert_drive {
    ($scheduler:ident, $store:ident, $current:ident, $status:ident, $message:literal $(,)?) => {{
        let result = drive_current_once_with_claim(&$scheduler, &$store, $current)
            .await
            .expect($message);
        assert_eq!(result.status(), SchedulerStatus::$status);
        $current = result.into_current_run();
    }};
}

macro_rules! drive_ok {
    ($scheduler:ident, $store:ident, $current:ident, $message:literal $(,)?) => {{
        let result = drive_current_once_with_claim(&$scheduler, &$store, $current)
            .await
            .expect($message);
        let status = result.status();
        $current = result.into_current_run();
        status
    }};
}

macro_rules! assert_first_node_invalid_after_drive {
    ($scheduler:ident, $store:ident, $current:ident, $fixture:ident, $message:literal $(,)?) => {{
        assert_drive!($scheduler, $store, $current, Advanced, $message);
        let node = node_by_output(&$fixture, &$fixture.cell_a);
        assert_node_failed_with_code(&$current, &node.node_id, "runner_output_invalid");
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
    test_adapter_executable_identity_with_digest(content(0xe2))
}

fn test_adapter_executable_identity_with_digest(
    binary_digest: ContentDigest,
) -> events::ExecutableIdentity {
    let factory_id = events::RunnerFactoryId::new("test_adapter").expect("factory");
    events::ExecutableIdentity {
        factory_id,
        binary_digest,
    }
}

fn test_executable_identity_template() -> ExecutableIdentityTemplate {
    ExecutableIdentityTemplate::new(content(0xe2))
}

fn test_runner_registry() -> ErasedRunnerRegistry {
    ErasedRunnerRegistry::new(test_executable_identity_template())
}

fn test_runner_registry_with_digest(binary_digest: ContentDigest) -> ErasedRunnerRegistry {
    ErasedRunnerRegistry::new(ExecutableIdentityTemplate::new(binary_digest))
}

const DA: DigestBytes = DigestBytes::from_array([0x1a; 32]);
const TEST_CONFIG_BYTES: &[u8] = b"{}";
const TEST_SEED_BYTES: &[u8] = br#"{"seed":true}"#;
const CERTIFIER_SEED_BYTES: &[u8] = br#"{"amount":2}"#;
const CONFIG_MULTIPLIER_1_BYTES: &[u8] = br#"{"multiplier":1}"#;
const CONFIG_MULTIPLIER_3_BYTES: &[u8] = br#"{"multiplier":3}"#;
const CONFIG_MULTIPLIER_5_BYTES: &[u8] = br#"{"multiplier":5}"#;
const CONFIG_MULTIPLIER_7_BYTES: &[u8] = br#"{"multiplier":7}"#;

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

#[path = "execution_support.rs"]
mod execution_support;
use self::execution_support::*;
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

fn content(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&synthetic_artifact_bytes(byte)),
    )
}

fn synthetic_artifact_bytes(byte: u8) -> Vec<u8> {
    vec![byte; 17]
}

fn synthetic_artifact_bytes_for_digest(digest: &ContentDigest) -> Vec<u8> {
    (u8::MIN..=u8::MAX)
        .find_map(|byte| {
            let bytes = synthetic_artifact_bytes(byte);
            (content(byte) == *digest).then_some(bytes)
        })
        .unwrap_or_else(|| panic!("test digest {} has no synthetic byte fixture", digest))
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
