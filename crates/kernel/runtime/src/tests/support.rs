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
