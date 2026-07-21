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

#[path = "fact_support.rs"]
mod fact_support;
#[path = "runner_kit.rs"]
mod runner_kit_tests;
use self::fact_support::{
    projection_snapshot_with_returned_fact_authority, test_fact_claim,
    test_fact_claim_for_descriptor, test_fact_descriptor_with_kind, test_fact_key,
    test_fact_query_evidence, test_fact_query_evidence_with_returned_refs,
    test_fact_response_artifact, test_fact_subject_evidence, test_returned_fact_authority,
};

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

macro_rules! delegate_execution_claim_refcell {
    ($inner:expr, $borrow:ident, $method:ident, ()) => {{
        let result = block_on_ready($inner.$borrow().$method());
        Box::pin(std::future::ready(result))
    }};
    ($inner:expr, $borrow:ident, $method:ident, ($($arg:expr),*)) => {{
        let result = block_on_ready($inner.$borrow().$method($($arg),*));
        Box::pin(std::future::ready(result))
    }};
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
#[path = "lifecycle_node_support.rs"]
mod lifecycle_node_support;
use self::lifecycle_node_support::*;
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
    test_scheduler_with_artifacts(registry, Arc::new(TestRetainedArtifactStore::default()))
}

fn test_scheduler_with_artifacts(
    registry: ErasedRunnerRegistry,
    artifact_store: Arc<dyn store::RetainedArtifactReadProvider>,
) -> SerialTypedScheduler {
    SerialTypedScheduler::new(registry, artifact_store)
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
                output_artifact: artifact(0xb1),
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
    register_fixture_read_runner(&mut registry, fixture, "read");
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
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
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
