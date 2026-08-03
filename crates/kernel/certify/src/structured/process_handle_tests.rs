use super::*;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    BoundedComponentContract, BoundedComponentInvoker, CapabilityContractFault,
    EffectAdapterInvoker, EffectCapabilityContract, NoRefresh, NoRefreshEvidence,
    ReadAdapterInvoker, ReadCapabilityContract, ReadCapabilityImplementation, Refreshable,
    ResourceAuthorityContract,
};
use mfm_ids::{
    AccessAttemptId, DigestAlgorithm, JournalRecordHash, OccurrenceId, RunId,
    RunSemanticStateDigest, SchemaId, SemanticCallId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    ExternalAccessAuthorized, HistoryObject, LexicalValueRef, RecordRef, SemanticHead,
    TypedValueRef,
};
use mfm_program::structured::{
    AllowsExecution, AuthoringPolicy, ChildOperation, ClosedSum, CustomFailureHandler,
    DefaultFailureMapper, Direct, Effect, FanOutResults, Never, OperationBuilder,
    PolicyExpansionRecipe, PolicyFailurePostBuilder, PolicyRecipeBuilder, ProposedSuccessOutcome,
    Pure, Read, RecoveryRouteBuilder, RefreshableBinding, Sequential, StateFrame, StateSettlement,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::{ProposedStateOutcome, StructuredComponentDependency};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "process_value",
    version = "1",
    schema = "mfm.test.process_value"
)]
struct ProcessValue {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "process_failure",
    version = "1",
    schema = "mfm.test.process_failure"
)]
struct ProcessFailure {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "process_failure_nominal_alias",
    version = "1",
    schema = "mfm.test.process_failure_nominal_alias"
)]
struct ProcessFailureNominalAlias {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.test",
    name = "process_failure_route",
    version = "1",
    schema = "mfm.test.process_failure_route"
)]
enum ProcessFailureRoute {
    Propagate { failure: ProcessFailure },
}

impl ClosedSum for ProcessFailureRoute {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.test",
    name = "process_choice",
    version = "1",
    schema = "mfm.test.process_choice"
)]
enum ProcessChoice {
    UseEffect { value: ProcessValue },
    Skip { value: ProcessValue },
}

impl ClosedSum for ProcessChoice {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.test",
    name = "process_recovery_route",
    version = "1",
    schema = "mfm.test.process_recovery_route"
)]
enum ProcessRecoveryRoute {
    Recover { value: ProcessValue },
    Propagate { failure: ProcessFailure },
}

impl ClosedSum for ProcessRecoveryRoute {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "refresh_evidence",
    version = "1",
    schema = "mfm.test.refresh_evidence"
)]
struct RefreshEvidence {
    generation: u64,
}

struct ReadProcessState;
struct InfallibleReadProcessState;
struct FallibleSuccessOnlyReadState;
struct EffectProcessState;
struct InfallibleEffectProcessState;
struct FalliblePureProcessState;
struct ProcessFailureMapperState;
struct ProcessFailureMapper;
struct ProcessTypedChild;
struct ProcessInfallibleEffectChild;
struct ProcessDepthTwoChild;
struct ProcessChoiceState;
struct ProcessFailureToValueState;
struct ProcessRecoveryHandlerState;
struct ProcessRecoveryHandler;
struct ProcessDepthRecoveryHandler;
struct ProcessIdentityState;
struct ProcessDepthAggregateState;
type ProcessDepthInnerJoin = FanOutResults<ProcessValue, Never>;
type ProcessDepthOuterJoin = FanOutResults<ProcessDepthInnerJoin, Never>;
struct ProcessReadCapability;
struct AlternateProcessReadCapability;
struct AlternateProcessReadImplementation;
struct ReservedFactReadAdapter;
struct ReservedFactReadImplementation;
enum AliasedPriorRunFactSelectionCapability {}
struct ProcessReadAdapter {
    calls: Arc<AtomicUsize>,
}
struct ProcessEffectCapability;
struct ProcessEffectAdapter {
    calls: Arc<AtomicUsize>,
}
struct NoRefreshProcessEffectCapability;
struct NoRefreshProcessEffectAdapter;
struct NoRefreshProcessEffectImplementation;
struct AlternateProcessEffectCapability;
struct AlternateProcessEffectAdapter;
struct HostileRefreshModeAdapter {
    semantic_contract_ref: ContentRef,
}
struct ProcessResource;
struct ProcessResourceInvoker {
    calls: Arc<AtomicUsize>,
}

fn sid(value: &str) -> StableId {
    StableId::new(value).expect("test stable id")
}

trait FixtureRegistryBuildExt {
    fn build_fixture(self) -> Result<QualifiedProgramRegistry>;
}

impl FixtureRegistryBuildExt for ProgramRegistryBuilder {
    fn build_fixture(self) -> Result<QualifiedProgramRegistry> {
        let expected = self.entry_points.keys().cloned().collect::<Vec<_>>();
        self.build(&expected)
    }
}

fn test_history_object(kind: &str, discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        sid(kind),
        SchemaId::new(
            kind,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, kind.as_bytes()[0]]),
        )
        .expect("test physical schema"),
        format!("{{\"value\":{discriminator}}}"),
    )
    .expect("test physical object")
}

fn test_lexical_value_ref(discriminator: u8) -> LexicalValueRef {
    LexicalValueRef {
        slot_ref: test_history_object("mfm.test/input-slot", discriminator).content_ref,
        value: TypedValueRef {
            contract_ref: test_history_object("mfm.test/input-contract", discriminator).content_ref,
            value_ref: test_history_object("mfm.test/input-value", discriminator).content_ref,
        },
        structural_origin: None,
    }
}

fn test_run_id(discriminator: u8) -> RunId {
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&[discriminator, 201]),
    )
}

fn test_occurrence_id(discriminator: u8) -> OccurrenceId {
    OccurrenceId::from_digest(sha256_digest_bytes(&[discriminator, 202]))
}

fn test_store_scope_id(discriminator: u8) -> StoreScopeId {
    StoreScopeId::new(format!("mfm.store_scope.v1:{discriminator:032x}")).expect("test store scope")
}

fn test_store_epoch(discriminator: u8) -> StoreEpoch {
    StoreEpoch::new(u64::from(discriminator) + 1)
}

fn test_tenant_scope_id(discriminator: u8) -> TenantScopeId {
    TenantScopeId::new(format!("mfm.tenant_scope.v1:{discriminator:032x}"))
        .expect("test tenant scope")
}

#[test]
fn reserved_prior_run_fact_contracts_reject_every_generic_registration_path() {
    let capability_ref = mfm_spec::structured::prior_run_fact_selection_capability_contract()
        .expect("reserved fact capability")
        .content_ref()
        .expect("reserved fact capability ref");
    let adapter_ref = mfm_spec::structured::prior_run_fact_scanner_adapter_contract()
        .expect("reserved fact adapter")
        .content_ref()
        .expect("reserved fact adapter ref");

    for aliased in [false, true] {
        let mut assembly = ProgramRegistryBuilder::new();
        let descriptor = descriptor(
            &mut assembly,
            StructuredComponentKind::Capability,
            capability_ref.clone(),
            if aliased {
                "mfm.test/aliased-reserved-fact-capability"
            } else {
                "mfm.test/reserved-fact-capability"
            },
        );
        let error = if aliased {
            assembly
                .register_read_capability::<AliasedPriorRunFactSelectionCapability, _>(
                    descriptor,
                    Arc::new(ReservedFactReadImplementation),
                )
                .expect_err("an aliased reserved capability must not use generic registration")
        } else {
            assembly
                .register_read_capability::<PriorRunFactSelectionCapability, _>(
                    descriptor,
                    Arc::new(ReservedFactReadImplementation),
                )
                .expect_err("the reserved capability must not use generic registration")
        };
        assert!(matches!(
            error,
            CertifyError::Certification(message)
                if message
                    == "the reserved prior-run fact capability must use its sealed registration"
        ));
    }

    for aliased in [false, true] {
        let mut assembly = ProgramRegistryBuilder::new();
        let descriptor = descriptor(
            &mut assembly,
            StructuredComponentKind::Adapter,
            adapter_ref.clone(),
            if aliased {
                "mfm.test/aliased-reserved-fact-adapter"
            } else {
                "mfm.test/reserved-fact-adapter"
            },
        );
        let error = if aliased {
            assembly
                .register_read_adapter::<AliasedPriorRunFactSelectionCapability, _>(
                    descriptor,
                    test_read_binding_source(Arc::new(ReservedFactReadAdapter), 91),
                )
                .expect_err("an aliased reserved adapter must not use generic registration")
        } else {
            assembly
                .register_read_adapter::<PriorRunFactSelectionCapability, _>(
                    descriptor,
                    test_read_binding_source(Arc::new(ReservedFactReadAdapter), 92),
                )
                .expect_err("the reserved adapter must not use generic registration")
        };
        assert!(matches!(
            error,
            CertifyError::Certification(message)
                if message == "the reserved prior-run fact adapter must use its sealed registration"
        ));
    }
}

#[test]
fn kernel_fact_scanner_baseline_is_retained_but_excluded_from_unrelated_entry_closure() {
    let operation_id = sid("mfm.test/kernel-baseline-unrelated-operation");
    let mut authored_builder =
        OperationBuilder::<ProcessValue, Never>::new(operation_id.clone(), sid("root"))
            .expect("operation builder");
    let input = authored_builder
        .input::<ProcessValue>(sid("input"))
        .expect("input root");
    let output = authored_builder
        .root()
        .state::<ProcessIdentityState>(sid("identity"), &input)
        .expect("identity state")
        .infallible()
        .expect("infallible identity state");
    let completion = authored_builder.succeed(&output).expect("root success");
    let authored = authored_builder
        .finish(completion)
        .expect("authored program");

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<ProcessValue>()
        .expect("value contract");
    let state_ref = state_contract::<ProcessIdentityState>()
        .expect("identity state contract")
        .state_contract_ref;
    let state_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/kernel-baseline-identity-implementation",
    );
    assembly
        .register_state::<ProcessIdentityState>(
            state_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame| ProposedStateOutcome::Success(frame.input().clone())),
            },
        )
        .expect("identity state registration");
    assembly
        .register_entry_point(operation_id.clone(), authored, profile())
        .expect("unrelated entry point");
    let registry = assembly
        .build_fixture()
        .expect("registry with kernel baseline");

    let capability_ref = prior_run_fact_selection_capability_contract()
        .expect("kernel capability")
        .content_ref()
        .expect("kernel capability ref");
    let adapter_ref = prior_run_fact_scanner_adapter_contract()
        .expect("kernel adapter")
        .content_ref()
        .expect("kernel adapter ref");
    let entry = registry
        .entry_points
        .get(&operation_id)
        .expect("unrelated entry definition");
    let envelope = entry
        .support_envelope
        .as_ref()
        .expect("entry support envelope");
    assert!(!envelope
        .semantic_components
        .contains(&(StructuredComponentKind::Capability, capability_ref.clone(),)));
    assert!(!envelope
        .semantic_components
        .contains(&(StructuredComponentKind::Adapter, adapter_ref.clone(),)));

    let capability_implementation_ref = registry
        .registry
        .component_implementations
        .get(&(StructuredComponentKind::Capability, capability_ref.clone()))
        .expect("kernel capability implementation");
    let adapter_implementation_ref = registry
        .registry
        .component_implementations
        .get(&(StructuredComponentKind::Adapter, adapter_ref.clone()))
        .expect("kernel adapter implementation");
    assert!(matches!(
        registry
            .process_components
            .get(&(
                StructuredComponentKind::Capability,
                capability_ref,
                capability_implementation_ref.clone(),
            ))
            .map(|component| &component.handle),
        Some(ProcessHandle::ReadCapability(implementation))
            if implementation.capability_type_id()
                == TypeId::of::<PriorRunFactSelectionCapability>()
    ));
    assert!(matches!(
        registry
            .process_components
            .get(&(
                StructuredComponentKind::Adapter,
                adapter_ref,
                adapter_implementation_ref.clone(),
            ))
            .map(|component| &component.handle),
        Some(ProcessHandle::PriorRunFactScannerBindingSource)
    ));
    assert_eq!(registry.entry_points.len(), 1);
}

#[test]
fn nonbaseline_unused_process_bindings_still_reject_registry_build() {
    let operation_id = sid("mfm.test/nonbaseline-unused-process-operation");
    let mut authored_builder =
        OperationBuilder::<ProcessValue, Never>::new(operation_id.clone(), sid("root"))
            .expect("operation builder");
    let input = authored_builder
        .input::<ProcessValue>(sid("input"))
        .expect("input root");
    let output = authored_builder
        .root()
        .state::<ProcessIdentityState>(sid("identity"), &input)
        .expect("identity state")
        .infallible()
        .expect("infallible identity state");
    let completion = authored_builder.succeed(&output).expect("root success");
    let authored = authored_builder
        .finish(completion)
        .expect("authored program");

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<ProcessValue>()
        .expect("value contract");
    assembly
        .register_value::<ProcessFailure>()
        .expect("failure contract");
    let state_ref = state_contract::<ProcessIdentityState>()
        .expect("identity state contract")
        .state_contract_ref;
    let state_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/nonbaseline-unused-identity-implementation",
    );
    assembly
        .register_state::<ProcessIdentityState>(
            state_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame| ProposedStateOutcome::Success(frame.input().clone())),
            },
        )
        .expect("identity state registration");

    let capability_ref = ProcessReadCapability::contract()
        .expect("unused capability contract")
        .content_ref()
        .expect("unused capability ref");
    let capability_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Capability,
        capability_ref,
        "mfm.test/nonbaseline-unused-capability-implementation",
    );
    assembly
        .register_read_capability::<ProcessReadCapability, _>(
            capability_descriptor,
            Arc::new(ProcessReadImplementation {
                counts: Arc::new(ReadValidationCounts::default()),
            }),
        )
        .expect("unused capability registration");
    let adapter_ref = ProcessReadAdapter::contract()
        .expect("unused adapter contract")
        .content_ref()
        .expect("unused adapter ref");
    let adapter_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Adapter,
        adapter_ref,
        "mfm.test/nonbaseline-unused-adapter-implementation",
    );
    assembly
        .register_read_adapter::<ProcessReadCapability, _>(
            adapter_descriptor,
            test_read_binding_source(
                Arc::new(ProcessReadAdapter {
                    calls: Arc::new(AtomicUsize::new(0)),
                }),
                93,
            ),
        )
        .expect("unused adapter registration");
    assembly
        .register_entry_point(operation_id, authored, profile())
        .expect("unrelated entry point");

    let error = assembly
        .build_fixture()
        .expect_err("nonbaseline unused process bindings must fail closed");
    assert!(matches!(
        error,
        CertifyError::Certification(message)
            if message
                == "qualified process bindings are missing, unused, or implementation-substituted"
    ));
}

#[test]
fn registry_build_enforces_exact_expected_entry_point_set() {
    let expected = [
        sid("mfm.test/exact-entry-one"),
        sid("mfm.test/exact-entry-two"),
        sid("mfm.test/exact-entry-three"),
    ];
    let registry = identity_entry_registry(&expected)
        .build(&expected)
        .expect("exact three-entry registry");
    assert_eq!(
        registry
            .entry_points
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected.iter().cloned().collect::<BTreeSet<_>>()
    );
    assert!(registry
        .process_components
        .values()
        .any(|component| matches!(
            component.handle,
            ProcessHandle::PriorRunFactScannerBindingSource
        )));

    for (label, declared) in [
        ("missing", expected[..2].to_vec()),
        (
            "extra",
            expected
                .iter()
                .cloned()
                .chain([sid("mfm.test/exact-entry-extra")])
                .collect(),
        ),
    ] {
        let error = identity_entry_registry(&expected)
            .build(&declared)
            .expect_err("missing or extra expected entry points must fail");
        assert_eq!(
            error,
            CertifyError::Certification(
                "qualified entry-point set differs from production expectation".to_owned()
            ),
            "{label} declaration must fail for the exact-set reason"
        );
    }

    let duplicate = [
        expected[0].clone(),
        expected[1].clone(),
        expected[2].clone(),
        expected[2].clone(),
    ];
    assert_eq!(
        identity_entry_registry(&expected)
            .build(&duplicate)
            .expect_err("duplicate expected entry points must fail"),
        CertifyError::Certification(
            "expected qualified entry-point identities contain duplicates".to_owned()
        )
    );
}

fn identity_entry_registry(entry_points: &[StableId]) -> ProgramRegistryBuilder {
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<ProcessValue>()
        .expect("value contract");
    let state_ref = state_contract::<ProcessIdentityState>()
        .expect("identity state contract")
        .state_contract_ref;
    let state_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/exact-entry-identity-implementation",
    );
    assembly
        .register_state::<ProcessIdentityState>(
            state_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame| ProposedStateOutcome::Success(frame.input().clone())),
            },
        )
        .expect("identity state registration");
    for operation_id in entry_points {
        let mut authored_builder =
            OperationBuilder::<ProcessValue, Never>::new(operation_id.clone(), sid("root"))
                .expect("operation builder");
        let input = authored_builder
            .input::<ProcessValue>(sid("input"))
            .expect("input root");
        let output = authored_builder
            .root()
            .state::<ProcessIdentityState>(sid("identity"), &input)
            .expect("identity state")
            .infallible()
            .expect("infallible identity state");
        let completion = authored_builder.succeed(&output).expect("root success");
        let authored = authored_builder
            .finish(completion)
            .expect("authored program");
        assembly
            .register_entry_point(operation_id.clone(), authored, profile())
            .expect("entry point registration");
    }
    assembly
}

#[test]
fn expected_authorization_rejects_a_substituted_state_input_ref() {
    let run_id = test_run_id(1);
    let occurrence_id = test_occurrence_id(1);
    let state_input_ref = test_lexical_value_ref(1);
    let capability_contract_ref = test_history_object("mfm.test/capability", 1).content_ref;
    let capability_implementation_ref =
        test_history_object("mfm.test/capability-implementation", 1).content_ref;
    let adapter_contract_ref = test_history_object("mfm.test/adapter", 1).content_ref;
    let adapter_implementation_ref =
        test_history_object("mfm.test/adapter-implementation", 1).content_ref;
    let routing_policy_ref = test_history_object("mfm.test/routing-policy", 1).content_ref;
    let store_scope_id = test_store_scope_id(1);
    let tenant_scope_id = test_tenant_scope_id(1);
    let source_manifest_ref = test_history_object("mfm.test/source-manifest", 1).content_ref;
    let certificate = test_history_object("mfm.test/physical-binding", 1);
    let request = encode_process_value(&ProcessValue { value: 7 }).expect("request");
    let expected = ExpectedAuthorization::new(
        PhysicalBindingSelection {
            run_id: &run_id,
            occurrence_id: &occurrence_id,
            state_input_ref: &state_input_ref,
            store_scope_id: &store_scope_id,
            store_epoch: test_store_epoch(1),
            tenant_scope_id: &tenant_scope_id,
            admitted_prior_run_source_manifest_ref: &source_manifest_ref,
            capability_contract_ref: &capability_contract_ref,
            capability_implementation_ref: &capability_implementation_ref,
            adapter_contract_ref: &adapter_contract_ref,
            adapter_implementation_ref: &adapter_implementation_ref,
            admitted_routing_policy_ref: &routing_policy_ref,
            stable_resource_lineage_contract_ref: None,
            minimum_lineage_head_ref: None,
        },
        AccessKind::Read,
        &request,
        &certificate,
    )
    .expect("expected authorization");
    let authorization_ref = RecordRef {
        run_id: run_id.clone(),
        run_sequence: 1,
        ordinal: 0,
        record_hash: JournalRecordHash::from_digest(sha256_digest_bytes(b"authorization")),
    };
    let mut authorization = ExternalAccessAuthorized {
        access_attempt_id: AccessAttemptId::from_digest(sha256_digest_bytes(b"attempt")),
        attempt_ordinal: 0,
        occurrence_id,
        occurrence_path_ref: test_history_object("mfm.test/occurrence-path", 1).content_ref,
        semantic_call_id: SemanticCallId::from_digest(sha256_digest_bytes(b"semantic-call")),
        state_input_ref: state_input_ref.clone(),
        access_kind: AccessKind::Read,
        semantic_head: SemanticHead::Genesis {
            admission_ref: authorization_ref.clone(),
            semantic_state_digest: RunSemanticStateDigest::from_digest(sha256_digest_bytes(
                b"semantic-state",
            )),
        },
        capability_contract_ref,
        capability_implementation_ref,
        adapter_contract_ref,
        adapter_implementation_ref,
        request: state_input_ref.value,
        request_digest: expected.request_digest.clone(),
        physical_binding_ref: certificate.content_ref,
        stable_resource_lineage_contract_ref: None,
    };
    assert!(expected.matches(&authorization_ref, &authorization));

    authorization.state_input_ref = test_lexical_value_ref(2);
    assert!(!expected.matches(&authorization_ref, &authorization));
}

struct TestReadPhysicalBinding<I> {
    invoker: Arc<I>,
    certificate: HistoryObject,
}

impl<C, I> ReadAdapterInvoker<C> for TestReadPhysicalBinding<I>
where
    C: RuntimeReadCapability,
    I: RuntimeReadAdapter<C>,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<C::Returned, C::SafeFailure>> {
        self.invoker.invoke(request)
    }
}

impl<C, I> RuntimeReadAdapter<C> for TestReadPhysicalBinding<I>
where
    C: RuntimeReadCapability,
    I: RuntimeReadAdapter<C>,
{
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        I::contract()
    }
}

impl<C, I> RuntimeReadPhysicalBinding<C> for TestReadPhysicalBinding<I>
where
    C: RuntimeReadCapability,
    I: RuntimeReadAdapter<C>,
{
    fn public_certificate(&self) -> &HistoryObject {
        &self.certificate
    }
}

struct TestReadPhysicalBindingSource<I> {
    binding: Arc<TestReadPhysicalBinding<I>>,
}

impl<C, I> RuntimeReadPhysicalBindingSource<C> for TestReadPhysicalBindingSource<I>
where
    C: RuntimeReadCapability,
    I: RuntimeReadAdapter<C>,
{
    type Binding = TestReadPhysicalBinding<I>;

    fn current_binding<'a>(
        &'a self,
        _selection: PhysicalBindingSelection<'a>,
        _request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = Arc::clone(&self.binding);
        Box::pin(async move { Some(binding) })
    }
}

fn test_read_binding_source<I>(
    invoker: Arc<I>,
    discriminator: u8,
) -> Arc<TestReadPhysicalBindingSource<I>> {
    Arc::new(TestReadPhysicalBindingSource {
        binding: Arc::new(TestReadPhysicalBinding {
            invoker,
            certificate: test_history_object("mfm.test/read-physical-binding", discriminator),
        }),
    })
}

struct TestEffectPhysicalBinding<I> {
    invoker: Arc<I>,
    certificate: HistoryObject,
    lineage_head: Option<HistoryObject>,
}

impl<C, I> EffectAdapterInvoker<C> for TestEffectPhysicalBinding<I>
where
    C: RuntimeEffectCapability,
    I: RuntimeEffectAdapter<C>,
{
    fn invoke<'a>(
        &'a self,
        request: &'a C::Request,
    ) -> ComponentFuture<'a, mfm_capabilities::EffectContractCompletion<C>> {
        self.invoker.invoke(request)
    }
}

impl<C, I> RuntimeEffectAdapter<C> for TestEffectPhysicalBinding<I>
where
    C: RuntimeEffectCapability,
    I: RuntimeEffectAdapter<C>,
{
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        I::contract()
    }
}

impl<C, I> RuntimeEffectPhysicalBinding<C> for TestEffectPhysicalBinding<I>
where
    C: RuntimeEffectCapability,
    I: RuntimeEffectAdapter<C>,
{
    fn public_certificate(&self) -> &HistoryObject {
        &self.certificate
    }

    fn supersession_head<'a>(
        &'a self,
        _evidence: &'a <C::Refresh as EffectRefreshMode>::Evidence,
    ) -> ComponentFuture<'a, Option<HistoryObject>> {
        let lineage_head = self.lineage_head.clone();
        Box::pin(async move { lineage_head })
    }
}

struct TestEffectPhysicalBindingSource<I> {
    binding: Arc<TestEffectPhysicalBinding<I>>,
}

impl<C, I> RuntimeEffectPhysicalBindingSource<C> for TestEffectPhysicalBindingSource<I>
where
    C: RuntimeEffectCapability,
    I: RuntimeEffectAdapter<C>,
{
    type Binding = TestEffectPhysicalBinding<I>;

    fn current_binding<'a>(
        &'a self,
        _selection: PhysicalBindingSelection<'a>,
        _request: &'a C::Request,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = Arc::clone(&self.binding);
        Box::pin(async move { Some(binding) })
    }
}

fn test_effect_binding_source<I>(
    invoker: Arc<I>,
    discriminator: u8,
    lineage_head: Option<HistoryObject>,
) -> Arc<TestEffectPhysicalBindingSource<I>> {
    Arc::new(TestEffectPhysicalBindingSource {
        binding: Arc::new(TestEffectPhysicalBinding {
            invoker,
            certificate: test_history_object("mfm.test/effect-physical-binding", discriminator),
            lineage_head,
        }),
    })
}

impl ReadCapabilityContract for ProcessReadCapability {
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
}

impl RuntimeReadCapability for ProcessReadCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new_read_capability(
            sid("mfm.test/read-capability"),
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessFailure>()?,
            <ProcessReadAdapter as RuntimeReadAdapter<ProcessReadCapability>>::contract()?
                .content_ref()?,
        )
        .map_err(Into::into)
    }
}

impl ReadCapabilityContract for AlternateProcessReadCapability {
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
}

impl RuntimeReadCapability for AlternateProcessReadCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new_read_capability(
            sid("mfm.test/alternate-read-capability"),
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessFailure>()?,
            <ProcessReadAdapter as RuntimeReadAdapter<ProcessReadCapability>>::contract()?
                .content_ref()?,
        )
        .map_err(Into::into)
    }
}

impl ReadCapabilityContract for AliasedPriorRunFactSelectionCapability {
    type Request = mfm_facts::FactSelectionRequest;
    type Returned = mfm_facts::FactSelectionReadResponse;
    type SafeFailure = mfm_facts::FactSelectionReadFailure;
}

impl RuntimeReadCapability for AliasedPriorRunFactSelectionCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        mfm_spec::structured::prior_run_fact_selection_capability_contract().map_err(Into::into)
    }
}

impl ReadCapabilityImplementation<PriorRunFactSelectionCapability>
    for ReservedFactReadImplementation
{
    fn validate_request(
        &self,
        _request: &mfm_facts::FactSelectionRequest,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &mfm_facts::FactSelectionReadResponse,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &mfm_facts::FactSelectionReadFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

impl ReadCapabilityImplementation<AliasedPriorRunFactSelectionCapability>
    for ReservedFactReadImplementation
{
    fn validate_request(
        &self,
        _request: &mfm_facts::FactSelectionRequest,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &mfm_facts::FactSelectionReadResponse,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &mfm_facts::FactSelectionReadFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

impl ReadAdapterInvoker<PriorRunFactSelectionCapability> for ReservedFactReadAdapter {
    fn invoke<'a>(
        &'a self,
        _request: &'a mfm_facts::FactSelectionRequest,
    ) -> ComponentFuture<
        'a,
        ReadAdapterCompletion<
            mfm_facts::FactSelectionReadResponse,
            mfm_facts::FactSelectionReadFailure,
        >,
    > {
        Box::pin(async {
            ReadAdapterCompletion::SafeFailure(mfm_facts::FactSelectionReadFailure::new(
                mfm_facts::FactSelectionReadFailureCode::StoreUnavailable,
            ))
        })
    }
}

impl RuntimeReadAdapter<PriorRunFactSelectionCapability> for ReservedFactReadAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        mfm_spec::structured::prior_run_fact_scanner_adapter_contract().map_err(Into::into)
    }
}

impl ReadAdapterInvoker<AliasedPriorRunFactSelectionCapability> for ReservedFactReadAdapter {
    fn invoke<'a>(
        &'a self,
        _request: &'a mfm_facts::FactSelectionRequest,
    ) -> ComponentFuture<
        'a,
        ReadAdapterCompletion<
            mfm_facts::FactSelectionReadResponse,
            mfm_facts::FactSelectionReadFailure,
        >,
    > {
        Box::pin(async {
            ReadAdapterCompletion::SafeFailure(mfm_facts::FactSelectionReadFailure::new(
                mfm_facts::FactSelectionReadFailureCode::StoreUnavailable,
            ))
        })
    }
}

impl RuntimeReadAdapter<AliasedPriorRunFactSelectionCapability> for ReservedFactReadAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        mfm_spec::structured::prior_run_fact_scanner_adapter_contract().map_err(Into::into)
    }
}

impl ReadAdapterInvoker<ProcessReadCapability> for ProcessReadAdapter {
    fn invoke<'a>(
        &'a self,
        request: &'a ProcessValue,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<ProcessValue, ProcessFailure>> {
        let ordinal = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match ordinal {
                0 => ReadAdapterCompletion::Returned(request.clone()),
                1 => ReadAdapterCompletion::SafeFailure(ProcessFailure { code: 41 }),
                _ => ReadAdapterCompletion::IntegrityFault(AccessFaultCode::new(
                    stable_id("mfm.test/read-integrity").expect("fault code"),
                )),
            }
        })
    }
}

impl RuntimeReadAdapter<ProcessReadCapability> for ProcessReadAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new(
            StructuredComponentKind::Adapter,
            sid("mfm.test/read-adapter"),
            Vec::new(),
        )
        .map_err(Into::into)
    }
}

impl EffectCapabilityContract for ProcessEffectCapability {
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Refresh = Refreshable<RefreshEvidence>;
}

impl RuntimeEffectCapability for ProcessEffectCapability {
    type RefreshBinding = RefreshableBinding<ProcessResource>;

    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        let resource_ref = ProcessResource::contract()?.content_ref()?;
        StructuredLiveComponentContract::new_effect_capability_refreshable(
            sid("mfm.test/effect-capability"),
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessFailure>()?,
            mfm_spec::structured::structured_value_contract_ref::<RefreshEvidence>()?,
            resource_ref,
            <ProcessEffectAdapter as RuntimeEffectAdapter<ProcessEffectCapability>>::contract()?
                .content_ref()?,
        )
        .map_err(Into::into)
    }
}

impl EffectAdapterInvoker<ProcessEffectCapability> for ProcessEffectAdapter {
    fn invoke<'a>(
        &'a self,
        request: &'a ProcessValue,
    ) -> ComponentFuture<'a, EffectAdapterCompletion<ProcessValue, ProcessFailure, RefreshEvidence>>
    {
        let ordinal = self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            match ordinal {
                0 => EffectAdapterCompletion::Returned(request.clone()),
                1 => EffectAdapterCompletion::SafeFailure(ProcessFailure { code: 42 }),
                2 => EffectAdapterCompletion::SupersededBeforeEntry(RefreshEvidence {
                    generation: 2,
                }),
                3 => EffectAdapterCompletion::EntryUnknown(AccessFaultCode::new(
                    stable_id("mfm.test/entry-unknown").expect("fault code"),
                )),
                _ => EffectAdapterCompletion::IntegrityFault(AccessFaultCode::new(
                    stable_id("mfm.test/effect-integrity").expect("fault code"),
                )),
            }
        })
    }
}

impl RuntimeEffectAdapter<ProcessEffectCapability> for ProcessEffectAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new(
            StructuredComponentKind::Adapter,
            sid("mfm.test/effect-adapter"),
            vec![StructuredComponentDependency {
                component_kind: StructuredComponentKind::Resource,
                contract_ref: ProcessResource::contract()?.content_ref()?,
            }],
        )
        .map_err(Into::into)
    }
}

impl EffectCapabilityContract for NoRefreshProcessEffectCapability {
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Refresh = NoRefresh;
}

impl RuntimeEffectCapability for NoRefreshProcessEffectCapability {
    type RefreshBinding = mfm_program::structured::NoRefreshBinding;

    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new_effect_capability_no_refresh(
            sid("mfm.test/no-refresh-effect-capability"),
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()?,
            mfm_spec::structured::structured_value_contract_ref::<ProcessFailure>()?,
            <NoRefreshProcessEffectAdapter as RuntimeEffectAdapter<
                NoRefreshProcessEffectCapability,
            >>::contract()?
            .content_ref()?,
        )
        .map_err(Into::into)
    }
}

impl EffectAdapterInvoker<NoRefreshProcessEffectCapability> for NoRefreshProcessEffectAdapter {
    fn invoke<'a>(
        &'a self,
        _request: &'a ProcessValue,
    ) -> ComponentFuture<'a, EffectAdapterCompletion<ProcessValue, ProcessFailure, NoRefreshEvidence>>
    {
        Box::pin(async { EffectAdapterCompletion::SafeFailure(ProcessFailure { code: 51 }) })
    }
}

impl RuntimeEffectAdapter<NoRefreshProcessEffectCapability> for NoRefreshProcessEffectAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new(
            StructuredComponentKind::Adapter,
            sid("mfm.test/no-refresh-effect-adapter"),
            Vec::new(),
        )
        .map_err(Into::into)
    }
}

impl EffectCapabilityContract for AlternateProcessEffectCapability {
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Refresh = Refreshable<RefreshEvidence>;
}

impl RuntimeEffectCapability for AlternateProcessEffectCapability {
    type RefreshBinding = RefreshableBinding<ProcessResource>;

    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        ProcessEffectCapability::contract()
    }
}

impl EffectAdapterInvoker<AlternateProcessEffectCapability> for AlternateProcessEffectAdapter {
    fn invoke<'a>(
        &'a self,
        request: &'a ProcessValue,
    ) -> ComponentFuture<'a, EffectAdapterCompletion<ProcessValue, ProcessFailure, RefreshEvidence>>
    {
        Box::pin(async move { EffectAdapterCompletion::Returned(request.clone()) })
    }
}

impl RuntimeEffectAdapter<AlternateProcessEffectCapability> for AlternateProcessEffectAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        ProcessEffectAdapter::contract()
    }
}

impl ErasedEffectPhysicalBindingSource for HostileRefreshModeAdapter {
    fn capability_type_id(&self) -> TypeId {
        TypeId::of::<ProcessEffectCapability>()
    }

    fn refresh_mode_type_id(&self) -> TypeId {
        TypeId::of::<NoRefresh>()
    }

    fn semantic_contract_ref(&self) -> Result<ContentRef> {
        Ok(self.semantic_contract_ref.clone())
    }

    fn resolve<'a>(
        &'a self,
        _selection: PhysicalBindingSelection<'a>,
        _request: CanonicalJsonValue,
        _implementation: Arc<dyn ErasedEffectCapabilityImplementation>,
        _process_identity: Arc<()>,
        _integrity_fault_code: StableId,
    ) -> ComponentFuture<'a, Result<Option<QualifiedPhysicalBindingCore>>> {
        Box::pin(async {
            Err(CertifyError::Certification(
                "hostile refresh-mode adapter is never callable".to_owned(),
            ))
        })
    }
}

impl BoundedComponentContract for ProcessResource {
    type Request = ();
    type Completion = ();
}

impl ResourceAuthorityContract for ProcessResource {}

impl RuntimeResourceAuthority for ProcessResource {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        StructuredLiveComponentContract::new(
            StructuredComponentKind::Resource,
            sid("mfm.test/resource"),
            Vec::new(),
        )
        .map_err(Into::into)
    }
}

impl BoundedComponentInvoker<ProcessResource> for ProcessResourceInvoker {
    fn invoke<'a>(&'a self, _request: &'a ()) -> ComponentFuture<'a, ()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }
}

impl State for ReadProcessState {
    type Input = ProcessValue;
    type Output = ProcessValue;
    type Failure = ProcessFailure;
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Execution = Read<ProcessReadCapability>;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureMayFail;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/read-state"))
    }
}

impl State for InfallibleReadProcessState {
    type Input = ProcessValue;
    type Output = ProcessValue;
    type Failure = Never;
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Execution = Read<ProcessReadCapability>;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/infallible-read-state"))
    }
}

impl State for FallibleSuccessOnlyReadState {
    type Input = ProcessValue;
    type Output = ProcessValue;
    type Failure = ProcessFailure;
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Execution = Read<ProcessReadCapability>;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/fallible-success-only-read-state"))
    }
}

impl State for EffectProcessState {
    type Input = ProcessValue;
    type Output = ProcessValue;
    type Failure = ProcessFailure;
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Execution = Effect<ProcessEffectCapability>;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureMayFail;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/effect-state"))
    }
}

impl State for InfallibleEffectProcessState {
    type Input = ProcessValue;
    type Output = ProcessValue;
    type Failure = Never;
    type Request = ProcessValue;
    type Returned = ProcessValue;
    type SafeFailure = ProcessFailure;
    type Execution = Effect<NoRefreshProcessEffectCapability>;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/infallible-effect-state"))
    }
}

impl State for FalliblePureProcessState {
    type Input = ProcessValue;
    type Output = ProcessValue;
    type Failure = ProcessFailure;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/fallible-pure-state"))
    }
}

impl State for ProcessFailureMapperState {
    type Input = ProcessFailure;
    type Output = ProcessFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/process-failure-mapper"))
    }
}

impl State for ProcessChoiceState {
    type Input = ProcessValue;
    type Output = ProcessChoice;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/process-choice-state"))
    }
}

impl State for ProcessFailureToValueState {
    type Input = ProcessFailure;
    type Output = ProcessValue;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/process-failure-to-value-state"))
    }
}

impl State for ProcessRecoveryHandlerState {
    type Input = ProcessFailure;
    type Output = ProcessRecoveryRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/process-recovery-handler-state"))
    }
}

impl State for ProcessIdentityState {
    type Input = ProcessValue;
    type Output = ProcessValue;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/process-identity-state"))
    }
}

impl State for ProcessDepthAggregateState {
    type Input = ProcessDepthOuterJoin;
    type Output = ProcessValue;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = mfm_program::structured::SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(sid("mfm.test/process-depth-aggregate-state"))
    }
}

impl DefaultFailureMapper<ProcessFailure, ProcessFailure> for ProcessFailureMapper {
    type Route = ProcessFailureRoute;
    type Mapper = ProcessFailureMapperState;
}

impl ChildOperation for ProcessTypedChild {
    type Output = ProcessValue;
    type Failure = ProcessFailure;

    fn authored_program_ref() -> mfm_program::Result<ContentRef> {
        typed_process_child_program()
            .content_ref()
            .map_err(Into::into)
    }
}

impl ChildOperation for ProcessInfallibleEffectChild {
    type Output = ProcessValue;
    type Failure = Never;

    fn authored_program_ref() -> mfm_program::Result<ContentRef> {
        infallible_effect_child_program()
            .content_ref()
            .map_err(Into::into)
    }
}

impl ChildOperation for ProcessDepthTwoChild {
    type Output = ProcessValue;
    type Failure = Never;

    fn authored_program_ref() -> mfm_program::Result<ContentRef> {
        depth_two_process_child_program()
            .content_ref()
            .map_err(Into::into)
    }
}

impl CustomFailureHandler<ProcessFailure, ProcessValue, ProcessFailure> for ProcessRecoveryHandler {
    type Route = ProcessRecoveryRoute;
    type Handler = ProcessRecoveryHandlerState;

    fn author_routes<Policy>(
        routes: &mut RecoveryRouteBuilder<ProcessValue, ProcessFailure, Policy>,
    ) -> mfm_program::Result<()>
    where
        Policy: AuthoringPolicy,
    {
        routes.arm("propagate", sid("propagate"), |block, payloads| {
            let failure = payloads.value::<ProcessFailure>(&[sid("failure")])?;
            block.scope_failure::<ProcessValue>(&failure)
        })?;
        routes.arm("recover", sid("recover"), |block, payloads| {
            let value = payloads.value::<ProcessValue>(&[sid("value")])?;
            let recovered = block
                .child::<ProcessInfallibleEffectChild>(
                    sid("hidden-effect"),
                    vec![value.bind_child(sid("child-input"))],
                )?
                .infallible()?;
            block.normal(&recovered)
        })
    }
}

impl CustomFailureHandler<ProcessFailure, ProcessValue, ProcessFailure>
    for ProcessDepthRecoveryHandler
{
    type Route = ProcessRecoveryRoute;
    type Handler = ProcessRecoveryHandlerState;

    fn author_routes<Policy>(
        routes: &mut RecoveryRouteBuilder<ProcessValue, ProcessFailure, Policy>,
    ) -> mfm_program::Result<()>
    where
        Policy: AuthoringPolicy,
    {
        routes.arm("propagate", sid("propagate"), |block, payloads| {
            let failure = payloads.value::<ProcessFailure>(&[sid("failure")])?;
            block.scope_failure::<ProcessValue>(&failure)
        })?;
        routes.arm("recover", sid("recover"), |block, payloads| {
            let value = payloads.value::<ProcessValue>(&[sid("value")])?;
            let recovered = block
                .child::<ProcessDepthTwoChild>(
                    sid("hidden-depth"),
                    vec![value.bind_child(sid("child-input"))],
                )?
                .infallible()?;
            block.normal(&recovered)
        })
    }
}

#[derive(Default)]
struct ReadValidationCounts {
    request: AtomicUsize,
    returned: AtomicUsize,
    safe_failure: AtomicUsize,
}

struct ProcessReadImplementation {
    counts: Arc<ReadValidationCounts>,
}

struct RejectingSafeFailureReadImplementation;

impl ReadCapabilityImplementation<ProcessReadCapability> for ProcessReadImplementation {
    fn validate_request(
        &self,
        _request: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.request.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.returned.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &ProcessFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.safe_failure.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl ReadCapabilityImplementation<AlternateProcessReadCapability>
    for AlternateProcessReadImplementation
{
    fn validate_request(
        &self,
        _request: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &ProcessFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

impl ReadCapabilityImplementation<ProcessReadCapability>
    for RejectingSafeFailureReadImplementation
{
    fn validate_request(
        &self,
        _request: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &ProcessFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Err(CapabilityContractFault::new(sid(
            "mfm.test/rejected-safe-failure",
        )))
    }
}

#[derive(Default)]
struct EffectValidationCounts {
    request: AtomicUsize,
    returned: AtomicUsize,
    safe_failure: AtomicUsize,
    superseded: AtomicUsize,
    entry_unknown: AtomicUsize,
    integrity: AtomicUsize,
}

struct ProcessEffectImplementation {
    counts: Arc<EffectValidationCounts>,
}

impl EffectCapabilityImplementation<ProcessEffectCapability> for ProcessEffectImplementation {
    fn validate_request(
        &self,
        _request: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.request.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.returned.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &ProcessFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.safe_failure.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn validate_superseded_before_entry(
        &self,
        _evidence: &RefreshEvidence,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.superseded.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn validate_entry_unknown(
        &self,
        _fault: &AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.entry_unknown.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn validate_integrity_fault(
        &self,
        _fault: &AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault> {
        self.counts.integrity.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl EffectCapabilityImplementation<NoRefreshProcessEffectCapability>
    for NoRefreshProcessEffectImplementation
{
    fn validate_request(
        &self,
        _request: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &ProcessValue,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &ProcessFailure,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_superseded_before_entry(
        &self,
        evidence: &NoRefreshEvidence,
    ) -> std::result::Result<(), CapabilityContractFault> {
        match *evidence {}
    }

    fn validate_entry_unknown(
        &self,
        _fault: &AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_integrity_fault(
        &self,
        _fault: &AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

fn descriptor(
    assembly: &mut ProgramRegistryBuilder,
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    label: &str,
) -> SecretFreeImplementationDescriptor {
    let executable_identity_ref = assembly
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable_id("mfm.test/process-executable").expect("executable id"),
        })
        .expect("executable identity");
    let qualification_artifact_ref = assembly
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable_id("mfm.test/process-qualification")
                .expect("qualification id"),
        })
        .expect("qualification artifact");
    SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id: stable_id(label).expect("implementation id"),
        executable_identity_ref,
        qualification_artifact_ref,
    }
}

fn profile() -> StructuredExpansionProfile {
    StructuredExpansionProfile {
        policies: Vec::new(),
        max_occurrences: 8,
        max_declarations: 8,
        max_lanes: 2,
        max_fan_out_depth: 2,
        max_branch_depth: 2,
    }
}

fn program<S: State<Input = ProcessValue, Output = ProcessValue, Failure = ProcessFailure>>(
    operation_id: StableId,
) -> AuthoredStructuredProgram
where
    Sequential: AllowsExecution<S::Execution>,
{
    let mut builder = OperationBuilder::<ProcessValue, ProcessFailure>::new(
        operation_id,
        stable_id("root").expect("scope id"),
    )
    .expect("operation builder");
    builder
        .root()
        .failure_map::<ProcessFailure, ProcessFailureMapper>()
        .expect("failure mapper");
    let input = builder
        .input::<ProcessValue>(stable_id("input").expect("input id"))
        .expect("input root");
    let output = builder
        .root()
        .state::<S>(stable_id("state").expect("state label"), &input)
        .expect("state declaration")
        .or_default()
        .expect("typed failure handler");
    let completion = builder.succeed(&output).expect("root success");
    builder.finish(completion).expect("authored program")
}

fn typed_process_child_program() -> AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<ProcessValue, ProcessFailure>::new(
        sid("mfm.test/typed-process-child"),
        sid("child-root"),
    )
    .expect("child builder");
    builder
        .root()
        .failure_map::<ProcessFailure, ProcessFailureMapper>()
        .expect("child failure mapper");
    let input = builder
        .input::<ProcessValue>(sid("child-input"))
        .expect("child input");
    let output = builder
        .root()
        .state::<ReadProcessState>(sid("child-read"), &input)
        .expect("child state")
        .or_default()
        .expect("child failure completion");
    let completion = builder.succeed(&output).expect("child success");
    builder.finish(completion).expect("typed child")
}

fn infallible_effect_child_program() -> AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<ProcessValue, Never>::new(
        sid("mfm.test/infallible-effect-child"),
        sid("child-root"),
    )
    .expect("Effect child builder");
    let input = builder
        .input::<ProcessValue>(sid("child-input"))
        .expect("Effect child input");
    let output = builder
        .root()
        .state::<InfallibleEffectProcessState>(sid("effect"), &input)
        .expect("Effect child state")
        .infallible()
        .expect("infallible Effect");
    let completion = builder.succeed(&output).expect("Effect child success");
    builder.finish(completion).expect("Effect child")
}

fn depth_two_process_child_program() -> AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<ProcessValue, Never>::new(
        sid("mfm.test/depth-two-child"),
        sid("child-root"),
    )
    .expect("depth-two child builder");
    let input = builder
        .input::<ProcessValue>(sid("child-input"))
        .expect("depth child input");
    let mut outer = builder
        .root()
        .fan_out::<ProcessDepthInnerJoin, Never>(sid("child-outer"))
        .expect("child outer fan-out");
    outer
        .lane(sid("outer-lane"), |outer_lane| {
            let mut inner = outer_lane.fan_out::<ProcessValue, Never>(sid("child-inner"))?;
            inner.lane(sid("inner-lane"), |inner_lane| {
                let output = inner_lane
                    .state::<ProcessIdentityState>(sid("identity"), &input)?
                    .infallible()?;
                inner_lane.normal(&output)
            })?;
            let joined = inner.finish()?;
            outer_lane.normal(&joined)
        })
        .expect("child outer lane");
    let joined = outer.finish().expect("child outer join");
    let output = builder
        .root()
        .state::<ProcessDepthAggregateState>(sid("aggregate"), &joined)
        .expect("depth aggregate")
        .infallible()
        .expect("infallible depth aggregate");
    let completion = builder.succeed(&output).expect("depth child success");
    builder.finish(completion).expect("depth-two child")
}

fn fan_out_with_hidden_effect_child(operation_id: StableId) -> AuthoredStructuredProgram {
    type Join = FanOutResults<ProcessValue, Never>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<ProcessValue, Never>(sid("fan-out"))
        .expect("fan-out");
    fan_out
        .lane(sid("lane"), |lane| {
            let output = lane
                .child::<ProcessInfallibleEffectChild>(
                    sid("hidden-effect"),
                    vec![input.bind_child(sid("child-input"))],
                )?
                .infallible()?;
            lane.normal(&output)
        })
        .expect("hidden Effect lane");
    let joined = fan_out.finish().expect("join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("hidden Effect fan-out")
}

fn fan_out_match_with_hidden_effect_child(operation_id: StableId) -> AuthoredStructuredProgram {
    type Join = FanOutResults<ProcessValue, Never>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<ProcessValue, Never>(sid("fan-out"))
        .expect("fan-out");
    fan_out
        .lane(sid("lane"), |lane| {
            let choice = lane
                .state::<ProcessChoiceState>(sid("choice"), &input)?
                .infallible()?;
            let output = lane.match_value(sid("match"), &choice, |arms| {
                arms.arm("skip", sid("skip"), |arm, payloads| {
                    let value = payloads.value::<ProcessValue>(&[sid("value")])?;
                    arm.normal(&value)
                })?;
                arms.arm("use_effect", sid("use-effect"), |arm, payloads| {
                    let value = payloads.value::<ProcessValue>(&[sid("value")])?;
                    let output = arm
                        .child::<ProcessInfallibleEffectChild>(
                            sid("hidden-effect"),
                            vec![value.bind_child(sid("child-input"))],
                        )?
                        .infallible()?;
                    arm.normal(&output)
                })
            })?;
            lane.normal(&output)
        })
        .expect("hidden Match Effect lane");
    let joined = fan_out.finish().expect("join");
    let completion = builder.succeed(&joined).expect("success");
    builder
        .finish(completion)
        .expect("hidden Match Effect fan-out")
}

fn hidden_effect_failure_post_recipe() -> PolicyExpansionRecipe {
    let (mut post, failure) = PolicyFailurePostBuilder::<ProcessFailure, Never>::new(
        sid("mfm.test/hidden-effect-failure-post"),
        sid("failure-post-root"),
    )
    .expect("failure-post builder");
    let input = post
        .root()
        .state::<ProcessFailureToValueState>(sid("convert"), &failure)
        .expect("failure conversion")
        .infallible()
        .expect("infallible conversion");
    post.root()
        .state::<InfallibleEffectProcessState>(sid("hidden-effect"), &input)
        .expect("hidden Effect state")
        .infallible()
        .expect("infallible Effect state");
    let post = post.finish().expect("failure-post recipe");

    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<ProcessValue, ProcessValue, ProcessFailure>::new(
            sid("mfm.test/hidden-effect-policy"),
            sid("policy-root"),
        )
        .expect("policy builder");
    let output = proceed
        .call(builder.root(), sid("proceed"), &input)
        .expect("policy proceed");
    let completion = builder.succeed(&output).expect("policy success");
    builder
        .finish_with_failure_post(completion, post)
        .expect("hidden Effect policy")
}

fn identity_process_policy_recipe() -> PolicyExpansionRecipe {
    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<ProcessValue, ProcessValue, ProcessFailure>::new(
            sid("mfm.test/identity-process-policy"),
            sid("policy-root"),
        )
        .expect("policy builder");
    let output = proceed
        .call(builder.root(), sid("proceed"), &input)
        .expect("policy proceed");
    let completion = builder.succeed(&output).expect("policy success");
    builder.finish(completion).expect("identity policy")
}

fn hidden_depth_failure_post_recipe() -> PolicyExpansionRecipe {
    let (mut post, failure) = PolicyFailurePostBuilder::<ProcessFailure, Never>::new(
        sid("mfm.test/hidden-depth-failure-post"),
        sid("failure-post-root"),
    )
    .expect("failure-post builder");
    let input = post
        .root()
        .state::<ProcessFailureToValueState>(sid("convert"), &failure)
        .expect("failure conversion")
        .infallible()
        .expect("infallible conversion");
    let mut outer = post
        .root()
        .fan_out::<ProcessDepthInnerJoin, Never>(sid("hidden-outer"))
        .expect("hidden outer fan-out");
    outer
        .lane(sid("outer-lane"), |outer_lane| {
            let mut inner = outer_lane.fan_out::<ProcessValue, Never>(sid("hidden-inner"))?;
            inner.lane(sid("inner-lane"), |inner_lane| {
                let output = inner_lane
                    .state::<ProcessIdentityState>(sid("identity"), &input)?
                    .infallible()?;
                inner_lane.normal(&output)
            })?;
            let joined = inner.finish()?;
            outer_lane.normal(&joined)
        })
        .expect("hidden outer lane");
    outer.finish().expect("hidden outer join");
    let post = post.finish().expect("depth failure-post recipe");

    let (mut builder, input, proceed) =
        PolicyRecipeBuilder::<ProcessValue, ProcessValue, ProcessFailure>::new(
            sid("mfm.test/hidden-depth-policy"),
            sid("policy-root"),
        )
        .expect("policy builder");
    let output = proceed
        .call(builder.root(), sid("proceed"), &input)
        .expect("policy proceed");
    let completion = builder.succeed(&output).expect("policy success");
    builder
        .finish_with_failure_post(completion, post)
        .expect("hidden depth policy")
}

fn typed_fan_out_read_program(operation_id: StableId) -> AuthoredStructuredProgram {
    type Join = FanOutResults<ProcessValue, ProcessFailure>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<ProcessValue, ProcessFailure>(sid("fan-out"))
        .expect("fan-out");
    fan_out
        .lane(sid("lane"), |lane| {
            lane.failure_map::<ProcessFailure, ProcessFailureMapper>()?;
            let output = lane
                .state::<ReadProcessState>(sid("read"), &input)?
                .or_default()?;
            lane.normal(&output)
        })
        .expect("typed Read lane");
    let joined = fan_out.finish().expect("join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("typed Read fan-out")
}

fn typed_fan_out_read_program_with_handler<Handler>(
    operation_id: StableId,
) -> AuthoredStructuredProgram
where
    Handler: CustomFailureHandler<ProcessFailure, ProcessValue, ProcessFailure>,
{
    type Join = FanOutResults<ProcessValue, ProcessFailure>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<ProcessValue, ProcessFailure>(sid("fan-out"))
        .expect("fan-out");
    fan_out
        .lane(sid("lane"), |lane| {
            let output = lane
                .state::<ReadProcessState>(sid("read"), &input)?
                .on_failure::<Handler>()?;
            lane.normal(&output)
        })
        .expect("typed Read lane");
    let joined = fan_out.finish().expect("join");
    let completion = builder.succeed(&joined).expect("success");
    builder
        .finish(completion)
        .expect("typed Read custom-recovery fan-out")
}

fn fan_out_with_depth_two_child(
    operation_id: StableId,
    inside_match: bool,
) -> AuthoredStructuredProgram {
    type Join = FanOutResults<ProcessValue, Never>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<ProcessValue, Never>(sid("fan-out"))
        .expect("fan-out");
    fan_out
        .lane(sid("lane"), |lane| {
            let output = if inside_match {
                let choice = lane
                    .state::<ProcessChoiceState>(sid("choice"), &input)?
                    .infallible()?;
                lane.match_value(sid("match"), &choice, |arms| {
                    for (tag, label) in [("skip", sid("skip")), ("use_effect", sid("use-effect"))] {
                        arms.arm(tag, label, |arm, payloads| {
                            let value = payloads.value::<ProcessValue>(&[sid("value")])?;
                            let output = arm
                                .child::<ProcessDepthTwoChild>(
                                    sid("depth-two-child"),
                                    vec![value.bind_child(sid("child-input"))],
                                )?
                                .infallible()?;
                            arm.normal(&output)
                        })?;
                    }
                    Ok(())
                })?
            } else {
                lane.child::<ProcessDepthTwoChild>(
                    sid("depth-two-child"),
                    vec![input.bind_child(sid("child-input"))],
                )?
                .infallible()?
            };
            lane.normal(&output)
        })
        .expect("depth-hidden lane");
    let joined = fan_out.finish().expect("join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("hidden depth fan-out")
}

fn typed_process_child_parent_program(operation_id: StableId) -> AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<ProcessValue, ProcessFailure>::new(operation_id, sid("root"))
            .expect("parent builder");
    builder
        .root()
        .failure_map::<ProcessFailure, ProcessFailureMapper>()
        .expect("parent failure mapper");
    let input = builder
        .input::<ProcessValue>(sid("input"))
        .expect("parent input");
    let output = builder
        .root()
        .child::<ProcessTypedChild>(sid("child"), vec![input.bind_child(sid("child-input"))])
        .expect("child call")
        .or_default()
        .expect("parent failure completion");
    let completion = builder.succeed(&output).expect("parent success");
    builder.finish(completion).expect("typed child parent")
}

fn typed_process_fan_out_program(operation_id: StableId) -> AuthoredStructuredProgram {
    type Join = FanOutResults<ProcessValue, ProcessFailure>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<ProcessValue, ProcessFailure>(sid("fan-out"))
        .expect("fan-out");
    for key in [sid("lane-a"), sid("lane-b")] {
        fan_out
            .lane(key, |lane| {
                lane.failure_map::<ProcessFailure, ProcessFailureMapper>()?;
                let output = lane
                    .state::<ReadProcessState>(sid("read"), &input)?
                    .or_default()?;
                lane.normal(&output)
            })
            .expect("typed lane");
    }
    let joined = fan_out.finish().expect("fan-out join");
    let completion = builder.succeed(&joined).expect("root success");
    builder.finish(completion).expect("typed fan-out")
}

fn same_root_process_fan_out_program(operation_id: StableId) -> AuthoredStructuredProgram {
    type Join = FanOutResults<ProcessValue, Never>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut fan_out = builder
        .root()
        .fan_out::<ProcessValue, Never>(sid("fan-out"))
        .expect("fan-out");
    for key in [sid("lane-a"), sid("lane-b")] {
        fan_out
            .lane(key, |lane| lane.normal(&input))
            .expect("same-root lane");
    }
    let joined = fan_out.finish().expect("fan-out join");
    let completion = builder.succeed(&joined).expect("root success");
    builder
        .finish(completion)
        .expect("same-admission-root fan-out")
}

fn depth_two_typed_process_fan_out_program(operation_id: StableId) -> AuthoredStructuredProgram {
    type InnerJoin = FanOutResults<ProcessValue, ProcessFailure>;
    type OuterJoin = FanOutResults<InnerJoin, ProcessFailure>;

    let mut builder =
        OperationBuilder::<OuterJoin, Never>::new(operation_id, sid("root")).expect("builder");
    let input = builder.input::<ProcessValue>(sid("input")).expect("input");
    let mut outer = builder
        .root()
        .fan_out::<InnerJoin, ProcessFailure>(sid("outer"))
        .expect("outer fan-out");
    for outer_key in [sid("outer-a"), sid("outer-b")] {
        outer
            .lane(outer_key, |outer_lane| {
                outer_lane.failure_map::<ProcessFailure, ProcessFailureMapper>()?;
                let _outer_read = outer_lane
                    .state::<ReadProcessState>(sid("outer-read"), &input)?
                    .or_default()?;
                let mut inner = outer_lane.fan_out::<ProcessValue, ProcessFailure>(sid("inner"))?;
                for inner_key in [sid("inner-a"), sid("inner-b")] {
                    inner.lane(inner_key, |inner_lane| {
                        inner_lane.failure_map::<ProcessFailure, ProcessFailureMapper>()?;
                        let output = inner_lane
                            .state::<ReadProcessState>(sid("read"), &input)?
                            .or_default()?;
                        inner_lane.normal(&output)
                    })?;
                }
                let joined = inner.finish()?;
                outer_lane.normal(&joined)
            })
            .expect("outer lane");
    }
    let joined = outer.finish().expect("outer join");
    let completion = builder.succeed(&joined).expect("root success");
    builder.finish(completion).expect("depth-two typed fan-out")
}

fn register_process_failure_mapper(assembly: &mut ProgramRegistryBuilder) {
    assembly
        .register_closed_sum::<ProcessFailureRoute>()
        .expect("failure route contract");
    let mapper_ref = state_contract::<ProcessFailureMapperState>()
        .expect("failure mapper state contract")
        .state_contract_ref;
    let mapper_descriptor = descriptor(
        assembly,
        StructuredComponentKind::State,
        mapper_ref,
        "mfm.test/process-failure-mapper-implementation",
    );
    assembly
        .register_state::<ProcessFailureMapperState>(
            mapper_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame| {
                    ProposedStateOutcome::Success(ProcessFailureRoute::Propagate {
                        failure: frame.input().clone(),
                    })
                }),
            },
        )
        .expect("failure mapper state registration");
}

fn simple_read_callbacks() -> StructuredStateCallbacks<ReadProcessState> {
    StructuredStateCallbacks::Read {
        request: Arc::new(|frame| frame.input().clone()),
        settle_returned: Arc::new(|_frame, returned| {
            StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
        }),
        settle_safe_failure: Arc::new(|_frame, failure| {
            ProposedStateOutcome::Failure(failure.clone())
        }),
    }
}

#[derive(Clone, Copy)]
enum MayFailSafeFailureSettlement {
    FailureEcho,
    SuccessFromFailure,
    FixedFailure(u64),
    SuccessOnOddFailureOtherwiseFailure,
}

fn may_fail_read_callbacks<S>(
    behavior: MayFailSafeFailureSettlement,
    settlement_calls: Option<Arc<AtomicUsize>>,
) -> StructuredStateCallbacks<S>
where
    S: State<
        Input = ProcessValue,
        Output = ProcessValue,
        Failure = ProcessFailure,
        Request = ProcessValue,
        Returned = ProcessValue,
        SafeFailure = ProcessFailure,
        Execution = Read<ProcessReadCapability>,
        SafeFailureDisposition = mfm_program::structured::SafeFailureMayFail,
    >,
{
    StructuredStateCallbacks::Read {
        request: Arc::new(|frame| frame.input().clone()),
        settle_returned: Arc::new({
            let settlement_calls = settlement_calls.clone();
            move |_frame, returned| {
                if let Some(calls) = &settlement_calls {
                    calls.fetch_add(1, Ordering::SeqCst);
                }
                StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
            }
        }),
        settle_safe_failure: Arc::new(move |_frame, failure| {
            if let Some(calls) = &settlement_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            match behavior {
                MayFailSafeFailureSettlement::FailureEcho => {
                    ProposedStateOutcome::Failure(failure.clone())
                }
                MayFailSafeFailureSettlement::SuccessFromFailure => {
                    ProposedStateOutcome::Success(ProcessValue {
                        value: failure.code,
                    })
                }
                MayFailSafeFailureSettlement::FixedFailure(code) => {
                    ProposedStateOutcome::Failure(ProcessFailure { code })
                }
                MayFailSafeFailureSettlement::SuccessOnOddFailureOtherwiseFailure => {
                    if failure.code % 2 == 1 {
                        ProposedStateOutcome::Success(ProcessValue {
                            value: failure.code,
                        })
                    } else {
                        ProposedStateOutcome::Failure(failure.clone())
                    }
                }
            }
        }),
    }
}

fn success_only_read_callbacks<S>(
    settlement_calls: Option<Arc<AtomicUsize>>,
) -> StructuredStateCallbacks<S>
where
    S: State<
        Input = ProcessValue,
        Output = ProcessValue,
        Failure = ProcessFailure,
        Request = ProcessValue,
        Returned = ProcessValue,
        SafeFailure = ProcessFailure,
        Execution = Read<ProcessReadCapability>,
        SafeFailureDisposition = mfm_program::structured::SafeFailureSuccessOnly,
    >,
{
    StructuredStateCallbacks::Read {
        request: Arc::new(|frame| frame.input().clone()),
        settle_returned: Arc::new({
            let settlement_calls = settlement_calls.clone();
            move |_frame, returned| {
                if let Some(calls) = &settlement_calls {
                    calls.fetch_add(1, Ordering::SeqCst);
                }
                StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
            }
        }),
        settle_safe_failure: Arc::new(move |_frame, failure| {
            if let Some(calls) = &settlement_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            ProposedSuccessOutcome::new(ProcessValue {
                value: failure.code,
            })
        }),
    }
}

fn read_process_assembly<S, I>(
    operation_id: StableId,
    callbacks: StructuredStateCallbacks<S>,
    implementation: Arc<I>,
) -> Result<ProgramRegistryBuilder>
where
    S: State<
        Input = ProcessValue,
        Output = ProcessValue,
        Failure = ProcessFailure,
        Request = ProcessValue,
        Returned = ProcessValue,
        SafeFailure = ProcessFailure,
        Execution = Read<ProcessReadCapability>,
        Capability = Direct,
    >,
    I: ReadCapabilityImplementation<ProcessReadCapability>,
{
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<ProcessValue>()?;
    assembly.register_value::<ProcessFailure>()?;
    assembly.register_value::<ProcessFailureNominalAlias>()?;
    register_process_failure_mapper(&mut assembly);

    let state_ref = state_contract::<S>()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .state_contract_ref;
    let state_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/qualification-state-implementation",
    );
    assembly.register_state::<S>(state_descriptor, callbacks)?;

    let capability_ref = ProcessReadCapability::contract()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .content_ref()?;
    let capability_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Capability,
        capability_ref.clone(),
        "mfm.test/qualification-read-capability-implementation",
    );
    assembly.register_read_capability::<ProcessReadCapability, I>(
        capability_descriptor,
        implementation,
    )?;

    let adapter_ref = ProcessReadAdapter::contract()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .content_ref()?;
    let adapter_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Adapter,
        adapter_ref,
        "mfm.test/qualification-read-adapter-implementation",
    );
    assembly.register_read_adapter::<ProcessReadCapability, _>(
        adapter_descriptor,
        test_read_binding_source(
            Arc::new(ProcessReadAdapter {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            1,
        ),
    )?;
    assembly.register_entry_point(operation_id.clone(), program::<S>(operation_id), profile())?;
    Ok(assembly)
}

fn register_hidden_boundary_fixtures(assembly: &mut ProgramRegistryBuilder) -> Result<()> {
    assembly.register_closed_sum::<ProcessChoice>()?;
    assembly.register_closed_sum::<ProcessRecoveryRoute>()?;

    let choice_ref = state_contract::<ProcessChoiceState>()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .state_contract_ref;
    let choice_descriptor = descriptor(
        assembly,
        StructuredComponentKind::State,
        choice_ref,
        "mfm.test/process-choice-state-implementation",
    );
    assembly.register_state::<ProcessChoiceState>(
        choice_descriptor,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                ProposedStateOutcome::Success(ProcessChoice::UseEffect {
                    value: frame.input().clone(),
                })
            }),
        },
    )?;

    let conversion_ref = state_contract::<ProcessFailureToValueState>()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .state_contract_ref;
    let conversion_descriptor = descriptor(
        assembly,
        StructuredComponentKind::State,
        conversion_ref,
        "mfm.test/process-failure-to-value-implementation",
    );
    assembly.register_state::<ProcessFailureToValueState>(
        conversion_descriptor,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                ProposedStateOutcome::Success(ProcessValue {
                    value: frame.input().code,
                })
            }),
        },
    )?;

    let recovery_ref = state_contract::<ProcessRecoveryHandlerState>()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .state_contract_ref;
    let recovery_descriptor = descriptor(
        assembly,
        StructuredComponentKind::State,
        recovery_ref,
        "mfm.test/process-recovery-handler-implementation",
    );
    assembly.register_state::<ProcessRecoveryHandlerState>(
        recovery_descriptor,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                ProposedStateOutcome::Success(ProcessRecoveryRoute::Recover {
                    value: ProcessValue {
                        value: frame.input().code,
                    },
                })
            }),
        },
    )?;

    let identity_ref = state_contract::<ProcessIdentityState>()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .state_contract_ref;
    let identity_descriptor = descriptor(
        assembly,
        StructuredComponentKind::State,
        identity_ref,
        "mfm.test/process-identity-state-implementation",
    );
    assembly.register_state::<ProcessIdentityState>(
        identity_descriptor,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| ProposedStateOutcome::Success(frame.input().clone())),
        },
    )?;

    let aggregate_ref = state_contract::<ProcessDepthAggregateState>()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .state_contract_ref;
    let aggregate_descriptor = descriptor(
        assembly,
        StructuredComponentKind::State,
        aggregate_ref,
        "mfm.test/process-depth-aggregate-state-implementation",
    );
    assembly.register_state::<ProcessDepthAggregateState>(
        aggregate_descriptor,
        StructuredStateCallbacks::Pure {
            apply: Arc::new(|frame| {
                ProposedStateOutcome::Success(ProcessValue {
                    value: frame.input().as_slice().len() as u64,
                })
            }),
        },
    )?;

    let effect_state_ref = state_contract::<InfallibleEffectProcessState>()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .state_contract_ref;
    let effect_state_descriptor = descriptor(
        assembly,
        StructuredComponentKind::State,
        effect_state_ref,
        "mfm.test/hidden-infallible-effect-state-implementation",
    );
    assembly.register_state::<InfallibleEffectProcessState>(
        effect_state_descriptor,
        StructuredStateCallbacks::Effect {
            request: Arc::new(|frame| frame.input().clone()),
            settle_returned: Arc::new(|_frame, returned| {
                StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
            }),
            settle_safe_failure: Arc::new(|_frame, failure| {
                ProposedSuccessOutcome::new(ProcessValue {
                    value: failure.code,
                })
            }),
        },
    )?;

    let capability_ref = NoRefreshProcessEffectCapability::contract()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .content_ref()?;
    let capability_descriptor = descriptor(
        assembly,
        StructuredComponentKind::Capability,
        capability_ref,
        "mfm.test/hidden-no-refresh-capability-implementation",
    );
    assembly.register_effect_capability::<NoRefreshProcessEffectCapability, _>(
        capability_descriptor,
        Arc::new(NoRefreshProcessEffectImplementation),
    )?;

    let adapter_ref = NoRefreshProcessEffectAdapter::contract()
        .map_err(|error| CertifyError::Certification(error.to_string()))?
        .content_ref()?;
    let adapter_descriptor = descriptor(
        assembly,
        StructuredComponentKind::Adapter,
        adapter_ref,
        "mfm.test/hidden-no-refresh-adapter-implementation",
    );
    assembly.register_effect_adapter::<NoRefreshProcessEffectCapability, _>(
        adapter_descriptor,
        test_effect_binding_source(Arc::new(NoRefreshProcessEffectAdapter), 2, None),
    )?;

    assembly.register_child(infallible_effect_child_program())?;
    assembly.register_child(depth_two_process_child_program())?;
    Ok(())
}

fn hidden_boundary_assembly(label: &str) -> ProgramRegistryBuilder {
    let mut assembly = read_process_assembly::<ReadProcessState, _>(
        sid(&format!("mfm.test/{label}-base")),
        simple_read_callbacks(),
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("hidden-boundary base assembly");
    register_hidden_boundary_fixtures(&mut assembly).expect("hidden-boundary fixture registration");
    assembly
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn assert_callback_free_bytes(document: &CertifiedProgramDocument) {
    let mut retained = document
        .root
        .canonical_json()
        .expect("certified root bytes")
        .as_str()
        .to_owned();
    for component in &document.component_closure {
        retained.push_str(
            component
                .value
                .canonical_json()
                .expect("certified component bytes")
                .as_str(),
        );
    }
    for forbidden in ["ProcessHandle", "process_components", "callback", "invoker"] {
        assert!(!retained.contains(forbidden), "retained {forbidden}");
    }
}

fn assert_process_graph_valid(assembly: &ProgramRegistryBuilder) {
    let required = assembly
        .process_components
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    validate_process_component_graph(&assembly.registry, &assembly.process_components, &required)
        .expect("valid process graph");
}

fn assert_process_graph_rejected(
    assembly: &ProgramRegistryBuilder,
    mutate: impl FnOnce(
        &mut StructuredCertificationRegistry,
        &mut BTreeMap<ProcessComponentKey, RegisteredProcessComponent>,
    ),
) {
    let mut registry = assembly.registry.clone();
    let mut process_components = assembly.process_components.clone();
    let required = process_components.keys().cloned().collect::<BTreeSet<_>>();
    mutate(&mut registry, &mut process_components);
    validate_process_component_graph(&registry, &process_components, &required)
        .expect_err("hostile process graph must be rejected");
}

#[test]
fn state_registration_rejects_descriptor_and_access_kind_substitution() {
    for hostile in ["kind", "semantic"] {
        let mut assembly = ProgramRegistryBuilder::new();
        assembly
            .register_value::<ProcessValue>()
            .expect("value contract");
        assembly
            .register_value::<ProcessFailure>()
            .expect("failure contract");
        let state_ref = state_contract::<ReadProcessState>()
            .expect("state contract")
            .state_contract_ref;
        let descriptor = descriptor(
            &mut assembly,
            if hostile == "kind" {
                StructuredComponentKind::Adapter
            } else {
                StructuredComponentKind::State
            },
            if hostile == "semantic" {
                typed_content_ref("mfm.process-graph-hostile", &"foreign-state")
                    .expect("foreign state ref")
            } else {
                state_ref
            },
            "mfm.test/hostile-state-implementation",
        );
        assembly
            .register_state::<ReadProcessState>(descriptor, simple_read_callbacks())
            .expect_err("descriptor substitution must fail");
    }

    let exact = state_contract::<ReadProcessState>().expect("exact Read state contract");
    let disposition_substitution = StructuredStateContract::new(
        exact.semantic_state_id.clone(),
        exact.execution.clone(),
        exact.input_contract_ref.clone(),
        exact.output_contract_ref.clone(),
        exact.failure_contract.clone(),
        StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {},
        exact.capability_requirement_ref.clone(),
    )
    .expect("fallible success-only Read contract is independently valid");
    assert_ne!(
        exact.state_contract_ref,
        disposition_substitution.state_contract_ref
    );
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<ProcessValue>()
        .expect("value contract");
    assembly
        .register_value::<ProcessFailure>()
        .expect("failure contract");
    let disposition_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        disposition_substitution.state_contract_ref,
        "mfm.test/read-state-disposition-substitution",
    );
    assembly
        .register_state::<ReadProcessState>(disposition_descriptor, simple_read_callbacks())
        .expect_err("recomputed disposition contract cannot substitute the typed state");

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<ProcessValue>()
        .expect("value contract");
    assembly
        .register_value::<ProcessFailure>()
        .expect("failure contract");
    let state_ref = state_contract::<ReadProcessState>()
        .expect("state contract")
        .state_contract_ref;
    let descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/read-state-wrong-callback-kind",
    );
    let callbacks = StructuredStateCallbacks::<ReadProcessState>::Effect {
        request: Arc::new(|frame| frame.input().clone()),
        settle_returned: Arc::new(|_frame, returned| {
            StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
        }),
        settle_safe_failure: Arc::new(|_frame, failure| {
            ProposedStateOutcome::Failure(failure.clone())
        }),
    };
    assembly
        .register_state::<ReadProcessState>(descriptor, callbacks)
        .expect_err("Read state cannot register Effect callbacks");
}

#[test]
fn execution_and_failure_contract_cross_product_is_exact() {
    fn assert_contract<S: mfm_program::structured::State>(
        expected_kind: StructuredExecutionKind,
        expected_never: bool,
    ) {
        let contract = state_contract::<S>().expect("state contract");
        assert_eq!(contract.execution.kind(), expected_kind);
        assert_eq!(
            matches!(contract.failure_contract, StructuredFailureContract::Never),
            expected_never
        );
        assert_eq!(
            contract.execution.capability_contract_ref().is_none(),
            expected_kind == StructuredExecutionKind::Pure
        );
        if expected_never {
            assert_eq!(
                contract.failure_contract.contract_ref().expect("Never ref"),
                never_failure_contract_ref().expect("reserved Never ref")
            );
        } else {
            assert_ne!(
                contract.failure_contract.contract_ref().expect("typed ref"),
                never_failure_contract_ref().expect("reserved Never ref")
            );
        }
    }

    assert_contract::<ProcessIdentityState>(StructuredExecutionKind::Pure, true);
    assert_contract::<FalliblePureProcessState>(StructuredExecutionKind::Pure, false);
    assert_contract::<InfallibleReadProcessState>(StructuredExecutionKind::Read, true);
    assert_contract::<ReadProcessState>(StructuredExecutionKind::Read, false);
    assert_contract::<InfallibleEffectProcessState>(StructuredExecutionKind::Effect, true);
    assert_contract::<EffectProcessState>(StructuredExecutionKind::Effect, false);
}

#[test]
fn state_binding_rejects_a_coherent_foreign_typed_failure_source() {
    let operation_id = sid("mfm.test/foreign-state-failure-source");
    let registry = read_process_assembly::<ReadProcessState, _>(
        operation_id.clone(),
        simple_read_callbacks(),
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("Read assembly")
    .build_fixture()
    .expect("qualified Read registry");
    let expanded = registry
        .certifier(&operation_id)
        .expect("Read certifier")
        .certify(program::<ReadProcessState>(operation_id.clone()))
        .expect("Read certification")
        .expanded()
        .clone();
    let [ExpandedDeclaration::State(state)] = expanded.root.declarations.as_slice() else {
        panic!("one direct Read state")
    };
    let mut hostile = state.as_ref().clone();
    let foreign_path = declaration_path(
        &expanded.root.path,
        &sid("foreign-state-output"),
        u32::try_from(expanded.root.declarations.len()).expect("declaration count"),
    )
    .expect("foreign occurrence path");
    let foreign_occurrence_id = foreign_path.occurrence_id().expect("foreign occurrence id");
    let failure_contract_ref = hostile
        .contract
        .failure_contract
        .contract_ref()
        .expect("typed failure contract");
    let foreign_source = state_output_slot(
        &foreign_path,
        &foreign_occurrence_id,
        &failure_contract_ref,
        ResultRole::TypedFailure,
    );
    let CertifiedFailureBoundary::Typed {
        source_slot, plan, ..
    } = &mut hostile.failure_boundary
    else {
        panic!("typed state boundary")
    };
    *source_slot = foreign_source.clone();
    let FailurePlan::Handled {
        plan_id,
        plan_path,
        source_slot: plan_source,
        before_handler,
        handler,
        continuation,
    } = plan.as_mut()
    else {
        panic!("default handled plan")
    };
    let foreign_plan_path = foreign_path
        .child(StructuralPathSegment::FailurePlan {
            label: sid("handler"),
        })
        .expect("foreign handler plan path");
    *plan_id = FailurePlanIdentity {
        source_semantic_call_id: hostile.semantic_call_id.clone(),
        source_slot: foreign_source.clone(),
        plan_path: foreign_plan_path.clone(),
    }
    .derive()
    .expect("foreign plan id");
    *plan_path = foreign_plan_path.clone();
    *plan_source = foreign_source.clone();
    before_handler.path = foreign_plan_path.clone();
    before_handler.tail = BlockTail::Normal(foreign_source.clone());
    handler.inputs = vec![foreign_source];
    handler.occurrence_path = declaration_path(
        &foreign_plan_path,
        &handler.label,
        u32::try_from(before_handler.declarations.len()).expect("pre-handler count"),
    )
    .expect("foreign handler occurrence path");
    handler.occurrence_id = handler
        .occurrence_path
        .occurrence_id()
        .expect("foreign handler occurrence id");
    handler.output_slot = state_output_slot(
        &handler.occurrence_path,
        &handler.occurrence_id,
        &handler.contract.output_contract_ref,
        ResultRole::SuccessOutput,
    );
    let HandlerContinuation::DefaultPropagation {
        handler_output_slot,
        payload_slot,
        failure_tail,
        ..
    } = continuation.as_mut()
    else {
        panic!("default propagation continuation")
    };
    *handler_output_slot = handler.output_slot.clone();
    payload_slot.lexical_path = foreign_plan_path;
    let LexicalProducer::VariantPayload { selector, .. } = &mut payload_slot.producer else {
        panic!("default propagation payload")
    };
    **selector = handler.output_slot.clone();
    *failure_tail = payload_slot.clone();

    validate_failure_boundary(
        &hostile.failure_boundary,
        &hostile.semantic_call_id,
        &registry.registry,
    )
    .expect("foreign source is otherwise internally coherent with its forged plan");
    validate_state_binding(&hostile, &registry.registry, false)
        .expect_err("state failure source must be its exact typed occurrence output");
}

#[test]
fn default_propagation_rejects_every_authority_and_provenance_substitution() {
    let operation_id = sid("mfm.test/default-propagation-hostile-matrix");
    let registry = read_process_assembly::<ReadProcessState, _>(
        operation_id.clone(),
        simple_read_callbacks(),
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("Read assembly")
    .build_fixture()
    .expect("qualified Read registry");
    let exact = registry
        .certifier(&operation_id)
        .expect("Read certifier")
        .certify(program::<ReadProcessState>(operation_id))
        .expect("Read certification")
        .expanded()
        .clone();
    let process_value_ref = mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()
        .expect("process value contract");
    let nominal_alias_ref =
        mfm_spec::structured::structured_value_contract_ref::<ProcessFailureNominalAlias>()
            .expect("nominal alias contract");
    let read_capability_ref = ProcessReadCapability::contract()
        .expect("Read capability contract")
        .content_ref()
        .expect("Read capability ref");

    for attack in [
        "non-pure-handler",
        "fallible-handler",
        "dominating-same-typed-value",
        "intervening-binding",
        "wrong-payload-path",
        "wrong-selector-content-reference",
        "byte-identical-distinct-nominal-contract",
        "enclosing-scope-contract",
    ] {
        let mut hostile = exact.clone();
        if attack == "enclosing-scope-contract" {
            let FailureScopeBinding::Owns { scope } = &mut hostile.root.failure_scope else {
                panic!("operation root owns its failure scope")
            };
            scope.failure_contract = StructuredFailureContract::typed(
                mfm_spec::structured::structured_value_contract::<ProcessValue>()
                    .expect("process value contract"),
            )
            .expect("typed failure contract");
        } else {
            let [ExpandedDeclaration::State(state)] = hostile.root.declarations.as_mut_slice()
            else {
                panic!("one direct Read state")
            };
            let CertifiedFailureBoundary::Typed { plan, .. } = &mut state.failure_boundary else {
                panic!("typed Read boundary")
            };
            let FailurePlan::Handled {
                source_slot,
                before_handler,
                handler,
                continuation,
                ..
            } = plan.as_mut()
            else {
                panic!("default handled plan")
            };
            let HandlerContinuation::DefaultPropagation {
                handler_output_slot: _,
                route_contract,
                payload_slot,
                failure_tail,
            } = continuation.as_mut()
            else {
                panic!("default propagation continuation")
            };
            match attack {
                "non-pure-handler" => {
                    handler.contract.execution = StructuredStateExecutionContract::Read {
                        capability_contract_ref: read_capability_ref.clone(),
                    };
                }
                "fallible-handler" => {
                    handler.contract.failure_contract = state.contract.failure_contract.clone();
                }
                "dominating-same-typed-value" => {
                    **payload_slot = source_slot.clone();
                    **failure_tail = source_slot.clone();
                }
                "intervening-binding" => {
                    before_handler
                        .declarations
                        .push(ExpandedDeclaration::State(handler.clone()));
                }
                "wrong-payload-path" => {
                    let LexicalProducer::VariantPayload { payload_path, .. } =
                        &mut payload_slot.producer
                    else {
                        panic!("variant payload")
                    };
                    payload_path.push(sid("foreign"));
                    **failure_tail = payload_slot.as_ref().clone();
                }
                "wrong-selector-content-reference" => {
                    let LexicalProducer::VariantPayload { selector, .. } =
                        &mut payload_slot.producer
                    else {
                        panic!("variant payload")
                    };
                    selector.contract_ref = process_value_ref.clone();
                    **failure_tail = payload_slot.as_ref().clone();
                }
                "byte-identical-distinct-nominal-contract" => {
                    let mut variants = route_contract.variants.clone();
                    variants[0].payloads[0].contract_ref = nominal_alias_ref.clone();
                    *route_contract = mfm_spec::structured::ClosedSumContract::new(
                        route_contract.selector_contract_ref.clone(),
                        variants,
                    )
                    .expect("internally coherent nominal-alias route");
                    payload_slot.contract_ref = nominal_alias_ref.clone();
                    failure_tail.contract_ref = nominal_alias_ref.clone();
                }
                _ => unreachable!("enumerated default-propagation attack"),
            }
        }

        let error = validate_expanded_program(&hostile, &profile(), &registry.registry)
            .expect_err("hostile default propagation must be rejected");
        assert!(
            !error.to_string().is_empty(),
            "{attack} must produce a classified certification rejection"
        );
    }
}

#[test]
fn may_fail_safe_failure_settlement_is_total_over_arbitrary_valid_values() {
    let operation_id = sid("mfm.test/may-fail-safe-failure-total");
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let registry = read_process_assembly::<ReadProcessState, _>(
        operation_id,
        may_fail_read_callbacks(
            MayFailSafeFailureSettlement::SuccessOnOddFailureOtherwiseFailure,
            Some(settlement_calls.clone()),
        ),
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("may-fail assembly")
    .build_fixture()
    .expect("may-fail qualifies without a sample corpus");
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);

    let state_ref = state_contract::<ReadProcessState>()
        .expect("state contract")
        .state_contract_ref;
    let callbacks = registry
        .process_components
        .iter()
        .find(|((kind, semantic_ref, _), _)| {
            *kind == StructuredComponentKind::State && semantic_ref == &state_ref
        })
        .map(|(_, component)| &component.handle)
        .expect("state process");
    let ProcessHandle::State(callbacks) = callbacks else {
        panic!("state callback handle");
    };

    let cases = [
        (1u64, "success"),
        (2, "failure"),
        (3, "success"),
        (4, "failure"),
        (99, "success"),
        (100, "failure"),
    ];
    for (code, expected_kind) in cases {
        let input = encode_process_value(&ProcessValue { value: code }).expect("input");
        let observation = encode_process_value(
            &CommittedObservation::<ProcessValue, ProcessFailure>::SafeFailure(ProcessFailure {
                code,
            }),
        )
        .expect("observation");
        let settlement = callbacks
            .settle_observation(&input, &observation)
            .expect("every inhabited safe failure settles");
        let kind = settlement
            .as_json()
            .get("outcome")
            .and_then(|value| value.get("kind"))
            .and_then(|value| value.as_str())
            .expect("outcome kind");
        assert_eq!(kind, expected_kind, "code {code}");
    }
    assert_eq!(settlement_calls.load(Ordering::SeqCst), cases.len());
}

#[test]
fn returned_value_settlement_retains_full_invalid_evidence_behavior() {
    let operation_id = sid("mfm.test/returned-settlement-invalid-evidence");
    let registry = read_process_assembly::<ReadProcessState, _>(
        operation_id,
        StructuredStateCallbacks::Read {
            request: Arc::new(|frame| frame.input().clone()),
            settle_returned: Arc::new(|frame, returned| {
                if frame.input().value == returned.value {
                    StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
                } else {
                    StateSettlement::InvalidEvidence
                }
            }),
            settle_safe_failure: Arc::new(|_frame, failure| {
                ProposedStateOutcome::Failure(failure.clone())
            }),
        },
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("assembly")
    .build_fixture()
    .expect("qualifies");

    let state_ref = state_contract::<ReadProcessState>()
        .expect("state contract")
        .state_contract_ref;
    let callbacks = registry
        .process_components
        .iter()
        .find(|((kind, semantic_ref, _), _)| {
            *kind == StructuredComponentKind::State && semantic_ref == &state_ref
        })
        .map(|(_, component)| &component.handle)
        .expect("state process");
    let ProcessHandle::State(callbacks) = callbacks else {
        panic!("state callback handle");
    };

    let input = encode_process_value(&ProcessValue { value: 1 }).expect("input");
    let returned = encode_process_value(
        &CommittedObservation::<ProcessValue, ProcessFailure>::Returned(ProcessValue { value: 2 }),
    )
    .expect("returned observation");
    let settlement = callbacks
        .settle_observation(&input, &returned)
        .expect("returned settlement");
    assert_eq!(
        settlement,
        encode_process_value(&StateSettlement::<ProcessValue, ProcessFailure>::InvalidEvidence)
            .expect("invalid evidence")
    );

    let consistent = encode_process_value(
        &CommittedObservation::<ProcessValue, ProcessFailure>::Returned(ProcessValue { value: 1 }),
    )
    .expect("consistent returned");
    let settlement = callbacks
        .settle_observation(&input, &consistent)
        .expect("consistent returned settlement");
    assert_eq!(
        settlement
            .as_json()
            .get("kind")
            .and_then(|value| value.as_str()),
        Some("proposed")
    );
}

#[test]
fn fallible_success_only_safe_failure_always_settles_success() {
    let operation_id = sid("mfm.test/fallible-success-only-read");
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let registry = read_process_assembly::<FallibleSuccessOnlyReadState, _>(
        operation_id,
        success_only_read_callbacks(Some(settlement_calls.clone())),
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("success-only Read assembly")
    .build_fixture()
    .expect("fallible SuccessOnly Read qualifies without a sample corpus");
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);

    let state_ref = state_contract::<FallibleSuccessOnlyReadState>()
        .expect("state contract")
        .state_contract_ref;
    let callbacks = registry
        .process_components
        .iter()
        .find(|((kind, semantic_ref, _), _)| {
            *kind == StructuredComponentKind::State && semantic_ref == &state_ref
        })
        .map(|(_, component)| &component.handle)
        .expect("success-only state process");
    let ProcessHandle::State(callbacks) = callbacks else {
        panic!("state callback handle");
    };

    for code in [0u64, 1, 2, 41, 99, 1_000_000] {
        let input = encode_process_value(&ProcessValue { value: 1 }).expect("input");
        let observation = encode_process_value(
            &CommittedObservation::<ProcessValue, ProcessFailure>::SafeFailure(ProcessFailure {
                code,
            }),
        )
        .expect("safe-failure observation");
        let settlement = callbacks
            .settle_observation(&input, &observation)
            .expect("every inhabited safe failure settles successfully");
        let outcome = settlement
            .as_json()
            .get("outcome")
            .expect("proposed outcome");
        assert_eq!(
            outcome.get("kind").and_then(|value| value.as_str()),
            Some("success"),
            "code {code}"
        );
        assert_eq!(
            outcome
                .get("value")
                .and_then(|value| value.get("value"))
                .and_then(|value| value.as_u64()),
            Some(code),
            "code {code}"
        );
    }
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 6);

    let before_malformed = settlement_calls.load(Ordering::SeqCst);
    let input = encode_process_value(&ProcessValue { value: 1 }).expect("input");
    let malformed = CanonicalJsonValue::new(serde_json::json!({
        "kind": "safe_failure",
        "value": {"code": "not-an-integer"},
    }))
    .expect("canonical but schema-invalid observation");
    callbacks
        .settle_observation(&input, &malformed)
        .expect_err("schema-invalid evidence must fail before callback invocation");
    assert_eq!(settlement_calls.load(Ordering::SeqCst), before_malformed);
}

#[test]
fn expanded_child_boundary_rebinds_fail_the_semantic_validator() {
    let base_id = sid("mfm.test/child-validator-base");
    let parent_id = sid("mfm.test/child-validator-parent");
    let mut assembly = read_process_assembly::<ReadProcessState, _>(
        base_id,
        simple_read_callbacks(),
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("base assembly");
    assembly
        .register_child(typed_process_child_program())
        .expect("child registration");
    let parent = typed_process_child_parent_program(parent_id.clone());
    assembly
        .register_entry_point(parent_id.clone(), parent.clone(), profile())
        .expect("parent entry");
    let registry = assembly.build_fixture().expect("qualified child registry");
    let expanded = registry
        .certifier(&parent_id)
        .expect("parent certifier")
        .certify(parent)
        .expect("child certification")
        .expanded()
        .clone();
    validate_expanded_program(&expanded, &profile(), &registry.registry)
        .expect("baseline expanded child");

    for hostile_kind in [
        "success_id",
        "success_role",
        "success_source",
        "success_contract",
        "failure_id",
        "failure_role",
        "failure_source",
        "failure_contract",
    ] {
        let mut hostile = expanded.clone();
        let [ExpandedDeclaration::Fragment(fragment)] = hostile.root.declarations.as_mut_slice()
        else {
            panic!("child must expand to one fragment");
        };
        let foreign_boundary_id = fragment
            .path
            .child(StructuralPathSegment::FailurePlan {
                label: sid("foreign-boundary"),
            })
            .expect("foreign path")
            .fragment_boundary_id()
            .expect("foreign boundary id");
        match hostile_kind {
            "success_id" => {
                let LexicalProducer::FragmentBoundary { boundary_id, .. } =
                    &mut fragment.success_slot.producer
                else {
                    panic!("success boundary");
                };
                *boundary_id = foreign_boundary_id;
            }
            "success_role" => {
                let LexicalProducer::FragmentBoundary { role, .. } =
                    &mut fragment.success_slot.producer
                else {
                    panic!("success boundary");
                };
                *role = ResultRole::TypedFailure;
            }
            "success_source" => {
                let LexicalProducer::FragmentBoundary { source, .. } =
                    &mut fragment.success_slot.producer
                else {
                    panic!("success boundary");
                };
                **source = fragment.input_bindings[0].caller_slot.clone();
            }
            "success_contract" => {
                fragment.success_slot.contract_ref =
                    mfm_spec::structured::structured_value_contract_ref::<ProcessFailure>()
                        .expect("failure value contract");
            }
            "failure_id" | "failure_role" | "failure_source" | "failure_contract" => {
                let CertifiedFailureBoundary::Typed { source_slot, .. } =
                    &mut fragment.failure_boundary
                else {
                    panic!("typed child failure boundary");
                };
                match hostile_kind {
                    "failure_id" => {
                        let LexicalProducer::FragmentBoundary { boundary_id, .. } =
                            &mut source_slot.producer
                        else {
                            panic!("failure boundary");
                        };
                        *boundary_id = foreign_boundary_id;
                    }
                    "failure_role" => {
                        let LexicalProducer::FragmentBoundary { role, .. } =
                            &mut source_slot.producer
                        else {
                            panic!("failure boundary");
                        };
                        *role = ResultRole::SuccessOutput;
                    }
                    "failure_source" => {
                        let LexicalProducer::FragmentBoundary { source, .. } =
                            &mut source_slot.producer
                        else {
                            panic!("failure boundary");
                        };
                        **source = fragment.success_slot.clone();
                    }
                    "failure_contract" => {
                        source_slot.contract_ref = fragment.success_slot.contract_ref.clone();
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        validate_expanded_program(&hostile, &profile(), &registry.registry)
            .expect_err("child FragmentBoundary rebind must fail semantic validation");
    }
}

fn mutate_expanded_fan_out_wrapper(group: &mut ExpandedFanOut, hostile_kind: &str) {
    if matches!(hostile_kind, "success_omitted" | "failure_omitted") {
        let LexicalProducer::LaneOutcome {
            success_slot,
            failure_slot,
            ..
        } = &mut group.lanes[0].outcome_slot.producer
        else {
            panic!("lane outcome");
        };
        match hostile_kind {
            "success_omitted" => *success_slot = None,
            "failure_omitted" => *failure_slot = None,
            _ => unreachable!(),
        }
    }
    if hostile_kind == "wrong_contract" {
        group.lanes[0].outcome_slot.contract_ref =
            mfm_spec::structured::structured_value_contract_ref::<ProcessValue>()
                .expect("foreign outcome contract");
    }
    let LexicalProducer::FanOutJoin {
        declaration_ordered_lane_slots,
        ..
    } = &mut group.output_slot.producer
    else {
        panic!("fan-out join");
    };
    match hostile_kind {
        "omitted" => {
            declaration_ordered_lane_slots.remove(0);
        }
        "duplicate" => {
            declaration_ordered_lane_slots.insert(1, declaration_ordered_lane_slots[0].clone());
        }
        "foreign" => {
            let mut foreign = declaration_ordered_lane_slots[0].clone();
            let LexicalProducer::LaneOutcome { lane_path, .. } = &mut foreign.producer else {
                panic!("lane outcome");
            };
            *lane_path = group.path.clone();
            declaration_ordered_lane_slots[0] = foreign;
        }
        "misordered" => declaration_ordered_lane_slots.swap(0, 1),
        "wrong_contract" | "success_omitted" | "failure_omitted" => {
            declaration_ordered_lane_slots[0] = group.lanes[0].outcome_slot.clone();
        }
        _ => unreachable!(),
    }
}

#[test]
fn fan_out_lane_wrapper_matrix_is_exact_at_both_supported_depths() {
    let base_id = sid("mfm.test/fan-out-validator-base");
    let depth_one_id = sid("mfm.test/fan-out-validator-depth-one");
    let depth_two_id = sid("mfm.test/fan-out-validator-depth-two");
    let same_root_id = sid("mfm.test/fan-out-validator-same-root");
    let mut fan_profile = profile();
    fan_profile.max_occurrences = 32;
    fan_profile.max_declarations = 32;
    fan_profile.max_lanes = 8;

    let mut assembly = read_process_assembly::<ReadProcessState, _>(
        base_id,
        simple_read_callbacks(),
        Arc::new(ProcessReadImplementation {
            counts: Arc::new(ReadValidationCounts::default()),
        }),
    )
    .expect("base assembly");
    let depth_one_program = typed_process_fan_out_program(depth_one_id.clone());
    let depth_two_program = depth_two_typed_process_fan_out_program(depth_two_id.clone());
    let same_root_program = same_root_process_fan_out_program(same_root_id.clone());
    assembly
        .register_entry_point(
            depth_one_id.clone(),
            depth_one_program.clone(),
            fan_profile.clone(),
        )
        .expect("depth-one entry");
    assembly
        .register_entry_point(
            depth_two_id.clone(),
            depth_two_program.clone(),
            fan_profile.clone(),
        )
        .expect("depth-two entry");
    assembly
        .register_entry_point(
            same_root_id.clone(),
            same_root_program.clone(),
            fan_profile.clone(),
        )
        .expect("same-root entry");
    let registry = assembly.build_fixture().expect("fan-out registry");
    let depth_one = registry
        .certifier(&depth_one_id)
        .expect("depth-one certifier")
        .certify(depth_one_program)
        .expect("depth-one certification")
        .expanded()
        .clone();
    let depth_two = registry
        .certifier(&depth_two_id)
        .expect("depth-two certifier")
        .certify(depth_two_program)
        .expect("depth-two certification")
        .expanded()
        .clone();
    let same_root = registry
        .certifier(&same_root_id)
        .expect("same-root certifier")
        .certify(same_root_program)
        .expect("same-root certification")
        .expanded()
        .clone();
    validate_expanded_program(&depth_one, &fan_profile, &registry.registry)
        .expect("baseline depth-one fan-out");
    validate_expanded_program(&depth_two, &fan_profile, &registry.registry)
        .expect("baseline depth-two fan-out");
    validate_expanded_program(&same_root, &fan_profile, &registry.registry)
        .expect("baseline same-root fan-out");

    let mut invalid_kernel_profile = fan_profile.clone();
    invalid_kernel_profile.max_occurrences = 0;
    let error = validate_profile(&invalid_kernel_profile, &registry.registry)
        .expect_err("zero occurrence bound must reject");
    assert!(error
        .to_string()
        .contains("structured expansion profile exceeds kernel bounds"));

    let mut occurrence_bound = fan_profile.clone();
    occurrence_bound.max_occurrences = 1;
    let mut declaration_bound = fan_profile.clone();
    declaration_bound.max_declarations = 1;
    let mut lane_bound = fan_profile.clone();
    lane_bound.max_lanes = 1;
    let mut branch_bound = fan_profile.clone();
    branch_bound.max_branch_depth = 0;
    for (label, hostile_profile) in [
        ("occurrences", occurrence_bound),
        ("declarations", declaration_bound),
        ("lanes", lane_bound),
        ("branch-depth", branch_bound),
    ] {
        let error = validate_expanded_program(&depth_one, &hostile_profile, &registry.registry)
            .expect_err("expanded program must respect every qualified structural bound");
        assert!(
            error
                .to_string()
                .contains("expanded program exceeds its qualified structural bounds"),
            "{label}: {error}"
        );
    }

    let [ExpandedDeclaration::FanOut(same_root_group)] = same_root.root.declarations.as_slice()
    else {
        panic!("same-root fan-out")
    };
    let [first_lane, second_lane] = same_root_group.lanes.as_slice() else {
        panic!("two same-root lanes")
    };
    let (
        LexicalProducer::LaneOutcome {
            lane_path: first_path,
            success_slot: Some(first_success),
            failure_slot: None,
        },
        LexicalProducer::LaneOutcome {
            lane_path: second_path,
            success_slot: Some(second_success),
            failure_slot: None,
        },
    ) = (
        &first_lane.outcome_slot.producer,
        &second_lane.outcome_slot.producer,
    )
    else {
        panic!("infallible same-root lane wrappers")
    };
    assert_eq!(first_success, second_success);
    assert!(matches!(
        first_success.producer,
        LexicalProducer::AdmissionRoot { .. }
    ));
    assert_ne!(first_path, second_path);
    assert_ne!(first_lane.outcome_slot, second_lane.outcome_slot);

    for hostile_kind in ["duplicate", "misordered"] {
        let mut hostile = same_root.clone();
        let [ExpandedDeclaration::FanOut(group)] = hostile.root.declarations.as_mut_slice() else {
            panic!("same-root fan-out")
        };
        let LexicalProducer::FanOutJoin {
            declaration_ordered_lane_slots,
            ..
        } = &mut group.output_slot.producer
        else {
            panic!("same-root join")
        };
        if hostile_kind == "duplicate" {
            declaration_ordered_lane_slots[1] = declaration_ordered_lane_slots[0].clone();
        } else {
            declaration_ordered_lane_slots.swap(0, 1);
        }
        validate_expanded_program(&hostile, &fan_profile, &registry.registry)
            .expect_err("nominal lane wrappers remain exact for byte-identical payloads");
    }

    for hostile_kind in [
        "omitted",
        "duplicate",
        "foreign",
        "misordered",
        "wrong_contract",
        "success_omitted",
        "failure_omitted",
    ] {
        let mut hostile = depth_one.clone();
        let [ExpandedDeclaration::FanOut(group)] = hostile.root.declarations.as_mut_slice() else {
            panic!("depth-one fan-out");
        };
        mutate_expanded_fan_out_wrapper(group, hostile_kind);
        validate_expanded_program(&hostile, &fan_profile, &registry.registry)
            .expect_err("depth-one lane-wrapper substitution must fail");

        for target_inner in [false, true] {
            let mut hostile = depth_two.clone();
            let [ExpandedDeclaration::FanOut(outer)] = hostile.root.declarations.as_mut_slice()
            else {
                panic!("outer fan-out");
            };
            if target_inner {
                let inner = outer.lanes[0]
                    .body
                    .declarations
                    .iter_mut()
                    .find_map(|declaration| match declaration {
                        ExpandedDeclaration::FanOut(inner) => Some(inner.as_mut()),
                        ExpandedDeclaration::State(_)
                        | ExpandedDeclaration::Match(_)
                        | ExpandedDeclaration::Fragment(_) => None,
                    })
                    .expect("inner fan-out");
                mutate_expanded_fan_out_wrapper(inner, hostile_kind);
            } else {
                mutate_expanded_fan_out_wrapper(outer, hostile_kind);
            }
            validate_expanded_program(&hostile, &fan_profile, &registry.registry)
                .expect_err("depth-two lane-wrapper substitution must fail");
        }
    }
}

#[test]
fn fan_out_rejects_hidden_effect_and_depth_through_every_structural_route() {
    for (label, authored) in [
        (
            "effect-fragment",
            fan_out_with_hidden_effect_child(sid("mfm.test/effect-fragment")),
        ),
        (
            "effect-match-arm",
            fan_out_match_with_hidden_effect_child(sid("mfm.test/effect-match-arm")),
        ),
        (
            "effect-custom-recovery",
            typed_fan_out_read_program_with_handler::<ProcessRecoveryHandler>(sid(
                "mfm.test/effect-custom-recovery",
            )),
        ),
    ] {
        let operation_id = authored.operation_id.clone();
        let mut assembly = hidden_boundary_assembly(label);
        assembly
            .register_entry_point(operation_id, authored, profile())
            .expect("hidden Effect entry remains inert");
        let error = assembly
            .build_fixture()
            .expect_err("Effect hidden in FanOut must fail qualification");
        assert!(
            error.to_string().contains("fan-out policy")
                || error.to_string().contains("inside FanOut"),
            "{label}: {error}"
        );
    }

    for (label, recipe, hidden_depth) in [
        (
            "effect-failure-post",
            hidden_effect_failure_post_recipe(),
            false,
        ),
        (
            "depth-failure-post",
            hidden_depth_failure_post_recipe(),
            true,
        ),
    ] {
        let operation_id = sid(&format!("mfm.test/{label}"));
        let authored = typed_fan_out_read_program(operation_id.clone());
        let policy =
            ExpansionPolicyContract::new(vec![mfm_spec::structured::PolicyExpansionBinding {
                boundary_contract_ref: state_contract::<ReadProcessState>()
                    .expect("Read state contract")
                    .state_contract_ref,
                recipe_ref: recipe.content_ref().expect("policy recipe ref"),
            }])
            .expect("failure-post policy");
        let mut policy_profile = profile();
        policy_profile.max_occurrences = 32;
        policy_profile.max_declarations = 32;
        policy_profile.max_lanes = 8;
        policy_profile.policies.push(policy);
        let mut assembly = hidden_boundary_assembly(label);
        assembly
            .register_policy_recipe(recipe)
            .expect("failure-post recipe registration");
        assembly
            .register_entry_point(operation_id, authored, policy_profile)
            .expect("failure-post entry remains inert");
        let error = assembly
            .build_fixture()
            .expect_err("structure hidden in failure-post under FanOut must reject");
        if hidden_depth {
            assert!(
                error.to_string().contains("FanOut exceeds depth"),
                "{label}: {error}"
            );
        } else {
            assert!(
                error.to_string().contains("fan-out policy")
                    || error.to_string().contains("inside FanOut"),
                "{label}: {error}"
            );
        }
    }

    for (label, authored) in [
        (
            "depth-fragment",
            fan_out_with_depth_two_child(sid("mfm.test/depth-fragment"), false),
        ),
        (
            "depth-match-arm",
            fan_out_with_depth_two_child(sid("mfm.test/depth-match-arm"), true),
        ),
        (
            "depth-custom-recovery",
            typed_fan_out_read_program_with_handler::<ProcessDepthRecoveryHandler>(sid(
                "mfm.test/depth-custom-recovery",
            )),
        ),
    ] {
        let operation_id = authored.operation_id.clone();
        let mut depth_profile = profile();
        depth_profile.max_occurrences = 32;
        depth_profile.max_declarations = 32;
        depth_profile.max_lanes = 8;
        let mut assembly = hidden_boundary_assembly(label);
        assembly
            .register_entry_point(operation_id, authored, depth_profile)
            .expect("hidden depth entry remains inert");
        let error = assembly
            .build_fixture()
            .expect_err("third FanOut level hidden by structure must reject");
        assert!(
            error.to_string().contains("FanOut exceeds depth"),
            "{label}: {error}"
        );
    }
}

#[test]
fn policy_expansion_rejects_recursive_active_proceed_state() {
    let authored = program::<ReadProcessState>(sid("mfm.test/recursive-policy-guard"));
    let [AuthoredDeclaration::State(state)] = authored.root.declarations.as_slice() else {
        panic!("one authored Read state")
    };
    let occurrence_path =
        declaration_path(&authored.root.path, &state.label, 0).expect("authored occurrence path");
    let input = authored.input_roots[0].clone();
    let boundary =
        state_binding_expansion_boundary(state, occurrence_path.clone(), vec![input.clone()])
            .expect("protected boundary");
    let retained_boundary = state_binding_expansion_boundary(state, occurrence_path, vec![input])
        .expect("retained protected boundary");
    let recipe = identity_process_policy_recipe();
    let policy_ref = recipe.content_ref().expect("policy ref");
    let registry = StructuredCertificationRegistry::default();
    let expansion_profile = profile();
    let mut context = ExpansionContext::new(&registry, &expansion_profile);
    context.active_policy_proceed = Some(ActivePolicyProceed {
        boundary: Some(retained_boundary),
        failure_post: None,
        policy_ref: policy_ref.clone(),
        recipe_root_path: recipe.program().root.path.clone(),
    });

    let error = context
        .expand_policy_recipe(state, boundary, &policy_ref, &recipe)
        .err()
        .expect("one policy expansion cannot recursively open another proceed scope");
    assert!(
        error
            .to_string()
            .contains("policy recipe expansion attempted recursive proceed substitution"),
        "{error}"
    );
}

#[test]
fn read_process_handles_are_retained_callable_and_never_used_by_certification() {
    let operation_id = stable_id("mfm.test/read-process-program").expect("operation id");
    let authored = program::<ReadProcessState>(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<ProcessValue>()
        .expect("value contract");
    assembly
        .register_value::<ProcessFailure>()
        .expect("failure contract");
    register_process_failure_mapper(&mut assembly);

    let callback_calls = Arc::new(AtomicUsize::new(0));
    let callback_owner = Arc::new(());
    let callback_owner_weak = Arc::downgrade(&callback_owner);
    let request_calls = callback_calls.clone();
    let request_owner = callback_owner.clone();
    let settle_returned_calls = callback_calls.clone();
    let settle_safe_calls = callback_calls.clone();
    let callbacks = StructuredStateCallbacks::<ReadProcessState>::Read {
        request: Arc::new(move |frame: StateFrame<'_, ProcessValue>| {
            let _retained = &request_owner;
            request_calls.fetch_add(1, Ordering::SeqCst);
            frame.input().clone()
        }),
        settle_returned: Arc::new(move |_frame, returned| {
            settle_returned_calls.fetch_add(1, Ordering::SeqCst);
            StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
        }),
        settle_safe_failure: Arc::new(move |_frame, failure| {
            settle_safe_calls.fetch_add(1, Ordering::SeqCst);
            ProposedStateOutcome::Failure(failure.clone())
        }),
    };
    let state_ref = state_contract::<ReadProcessState>()
        .expect("state contract")
        .state_contract_ref;
    let state_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/read-state-implementation",
    );
    assembly
        .register_state::<ReadProcessState>(state_descriptor, callbacks)
        .expect("state registration");
    drop(callback_owner);

    let validation_counts = Arc::new(ReadValidationCounts::default());
    let capability = Arc::new(ProcessReadImplementation {
        counts: validation_counts.clone(),
    });
    let capability_weak = Arc::downgrade(&capability);
    let capability_ref = ProcessReadCapability::contract()
        .expect("capability contract")
        .content_ref()
        .expect("capability ref");
    let capability_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Capability,
        capability_ref.clone(),
        "mfm.test/read-capability-implementation",
    );
    assembly
        .register_read_capability::<ProcessReadCapability, _>(
            capability_descriptor,
            capability.clone(),
        )
        .expect("capability registration");
    drop(capability);

    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let adapter = Arc::new(ProcessReadAdapter {
        calls: adapter_calls.clone(),
    });
    let adapter_weak = Arc::downgrade(&adapter);
    let adapter_ref = ProcessReadAdapter::contract()
        .expect("adapter contract")
        .content_ref()
        .expect("adapter ref");
    let adapter_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Adapter,
        adapter_ref.clone(),
        "mfm.test/read-adapter-implementation",
    );
    assembly
        .register_read_adapter::<ProcessReadCapability, _>(
            adapter_descriptor,
            test_read_binding_source(adapter.clone(), 3),
        )
        .expect("adapter registration");
    drop(adapter);
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry point");
    assert_process_graph_valid(&assembly);
    for field in ["request", "returned", "safe_failure"] {
        let capability_ref = capability_ref.clone();
        assert_process_graph_rejected(&assembly, move |registry, _| {
            let capability = registry
                .live_components
                .get_mut(&(StructuredComponentKind::Capability, capability_ref))
                .expect("Read capability");
            let Some(StructuredCapabilityProtocolContract::Read {
                request_contract_ref,
                returned_contract_ref,
                safe_failure_contract_ref,
                ..
            }) = &mut capability.capability_protocol
            else {
                panic!("Read protocol")
            };
            let foreign = typed_content_ref("mfm.process-graph-hostile", &field)
                .expect("foreign evidence contract");
            match field {
                "request" => *request_contract_ref = foreign,
                "returned" => *returned_contract_ref = foreign,
                "safe_failure" => *safe_failure_contract_ref = foreign,
                _ => unreachable!(),
            }
        });
    }
    let read_capability_ref = capability_ref.clone();
    assert_process_graph_rejected(&assembly, move |registry, _| {
        let capability = registry
            .live_components
            .get_mut(&(StructuredComponentKind::Capability, read_capability_ref))
            .expect("Read capability");
        let Some(StructuredCapabilityProtocolContract::Read {
            request_contract_ref,
            returned_contract_ref,
            safe_failure_contract_ref,
            access_fault_contract_ref,
        }) = capability.capability_protocol.take()
        else {
            panic!("Read protocol")
        };
        capability.capability_protocol = Some(StructuredCapabilityProtocolContract::Effect {
            request_contract_ref,
            returned_contract_ref,
            safe_failure_contract_ref,
            access_fault_contract_ref,
            refresh_contract: StructuredEffectRefreshContract::NoRefresh {},
        });
    });
    let read_capability_ref = capability_ref.clone();
    assert_process_graph_rejected(&assembly, move |_, process_components| {
        let capability = process_components
            .iter_mut()
            .find(|((kind, semantic_ref, _), _)| {
                *kind == StructuredComponentKind::Capability && semantic_ref == &read_capability_ref
            })
            .map(|(_, component)| component)
            .expect("Read capability process");
        capability.handle =
            ProcessHandle::EffectCapability(Arc::new(TypedEffectCapabilityImplementation::<
                ProcessEffectCapability,
            > {
                implementation: Arc::new(ProcessEffectImplementation {
                    counts: Arc::new(EffectValidationCounts::default()),
                }),
            }));
    });
    let exact_protocol = ProcessReadCapability::contract().expect("exact Read protocol");
    let alternate_protocol =
        AlternateProcessReadCapability::contract().expect("alternate Read protocol");
    assert_ne!(
        exact_protocol.content_ref().expect("exact protocol ref"),
        alternate_protocol
            .content_ref()
            .expect("alternate protocol ref")
    );
    let (
        Some(StructuredCapabilityProtocolContract::Read {
            request_contract_ref: exact_request,
            returned_contract_ref: exact_returned,
            safe_failure_contract_ref: exact_safe_failure,
            ..
        }),
        Some(StructuredCapabilityProtocolContract::Read {
            request_contract_ref: alternate_request,
            returned_contract_ref: alternate_returned,
            safe_failure_contract_ref: alternate_safe_failure,
            ..
        }),
    ) = (
        &exact_protocol.capability_protocol,
        &alternate_protocol.capability_protocol,
    )
    else {
        panic!("Read protocols")
    };
    assert_eq!(exact_request, alternate_request);
    assert_eq!(exact_returned, alternate_returned);
    assert_eq!(exact_safe_failure, alternate_safe_failure);
    let read_capability_ref = capability_ref.clone();
    assert_process_graph_rejected(&assembly, move |_, process_components| {
        let capability = process_components
            .iter_mut()
            .find(|((kind, semantic_ref, _), _)| {
                *kind == StructuredComponentKind::Capability && semantic_ref == &read_capability_ref
            })
            .map(|(_, component)| component)
            .expect("Read capability process");
        capability.handle =
            ProcessHandle::ReadCapability(Arc::new(TypedReadCapabilityImplementation::<
                AlternateProcessReadCapability,
            > {
                implementation: Arc::new(AlternateProcessReadImplementation),
            }));
    });
    let registry = assembly.build_fixture().expect("qualified registry");
    assert!(callback_owner_weak.upgrade().is_some());
    assert!(capability_weak.upgrade().is_some());
    assert!(adapter_weak.upgrade().is_some());
    // Safe-failure totality is type-enforced; qualification no longer invokes
    // settlement or capability sample validation against a reviewed corpus.
    let qualified_callback_calls = callback_calls.load(Ordering::SeqCst);
    let qualified_safe_failure_validations = validation_counts.safe_failure.load(Ordering::SeqCst);
    assert_eq!(qualified_callback_calls, 0);
    assert_eq!(qualified_safe_failure_validations, 0);

    let certified = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(authored)
        .expect("certified program");
    registry
        .admission_verifier(&operation_id)
        .expect("admission verifier")
        .verify(certified.document())
        .expect("verified document");
    assert_eq!(
        callback_calls.load(Ordering::SeqCst),
        qualified_callback_calls
    );
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(validation_counts.request.load(Ordering::SeqCst), 0);
    assert_eq!(
        validation_counts.safe_failure.load(Ordering::SeqCst),
        qualified_safe_failure_validations
    );
    assert_callback_free_bytes(certified.document());

    let input = encode_process_value(&ProcessValue { value: 7 }).expect("encoded input");
    for ((_, semantic_ref, _), component) in &registry.process_components {
        match &component.handle {
            ProcessHandle::State(callbacks) => match callbacks.kind() {
                mfm_spec::structured::StructuredExecutionKind::Read => {
                    let request = callbacks.author_request(&input).expect("authored request");
                    assert_eq!(request, input);
                    let observation = encode_process_value(&CommittedObservation::<
                        ProcessValue,
                        ProcessFailure,
                    >::Returned(
                        ProcessValue { value: 8 }
                    ))
                    .expect("observation");
                    let settlement = callbacks
                        .settle_observation(&input, &observation)
                        .expect("settlement");
                    let expected = encode_process_value(&StateSettlement::<
                        ProcessValue,
                        ProcessFailure,
                    >::Proposed(
                        ProposedStateOutcome::Success(ProcessValue { value: 8 }),
                    ))
                    .expect("expected settlement");
                    assert_eq!(settlement, expected);

                    let observation = encode_process_value(&CommittedObservation::<
                        ProcessValue,
                        ProcessFailure,
                    >::SafeFailure(
                        ProcessFailure { code: 41 }
                    ))
                    .expect("safe-failure observation");
                    let settlement = callbacks
                        .settle_observation(&input, &observation)
                        .expect("safe-failure settlement");
                    let expected = encode_process_value(&StateSettlement::<
                        ProcessValue,
                        ProcessFailure,
                    >::Proposed(
                        ProposedStateOutcome::Failure(ProcessFailure { code: 41 }),
                    ))
                    .expect("expected safe-failure settlement");
                    assert_eq!(settlement, expected);
                }
                mfm_spec::structured::StructuredExecutionKind::Pure => {
                    let failure = encode_process_value(&ProcessFailure { code: 41 })
                        .expect("failure mapper input");
                    let outcome = callbacks
                        .invoke_pure(&failure)
                        .expect("failure mapper outcome");
                    let expected = encode_process_value(&ProposedStateOutcome::<
                        ProcessFailureRoute,
                        Never,
                    >::Success(
                        ProcessFailureRoute::Propagate {
                            failure: ProcessFailure { code: 41 },
                        },
                    ))
                    .expect("expected failure mapper outcome");
                    assert_eq!(outcome, expected);
                }
                mfm_spec::structured::StructuredExecutionKind::Effect => {
                    panic!("unexpected Effect state in Read process")
                }
            },
            ProcessHandle::ReadCapability(implementation) if semantic_ref == &capability_ref => {
                implementation
                    .validate_request(&input)
                    .expect("request validation");
                implementation
                    .validate_returned(&input)
                    .expect("returned validation");
                implementation
                    .validate_safe_failure(
                        &encode_process_value(&ProcessFailure { code: 41 }).expect("safe failure"),
                    )
                    .expect("safe-failure validation");
            }
            ProcessHandle::ReadCapability(_) => {}
            ProcessHandle::ReadPhysicalBindingSource(_)
            | ProcessHandle::PriorRunFactScannerBindingSource => {}
            ProcessHandle::EffectCapability(_)
            | ProcessHandle::EffectPhysicalBindingSource(_)
            | ProcessHandle::Signer(_)
            | ProcessHandle::Resource(_) => {}
        }
    }
    let run_id = test_run_id(11);
    let occurrence_id = test_occurrence_id(11);
    let routing_policy_ref = test_history_object("mfm.test/routing-policy", 11).content_ref;
    let store_scope_id = test_store_scope_id(11);
    let tenant_scope_id = test_tenant_scope_id(11);
    let source_manifest_ref = test_history_object("mfm.test/source-manifest", 11).content_ref;
    let state_input_ref = test_lexical_value_ref(11);
    let target = AccessTargetSelection {
        run_id: &run_id,
        occurrence_id: &occurrence_id,
        state_input_ref: &state_input_ref,
        store_scope_id: &store_scope_id,
        store_epoch: test_store_epoch(11),
        tenant_scope_id: &tenant_scope_id,
        admitted_prior_run_source_manifest_ref: &source_manifest_ref,
        admitted_routing_policy_ref: &routing_policy_ref,
        stable_resource_lineage_contract_ref: None,
        minimum_lineage_head_ref: None,
    };
    let (_, processes) = registry.into_runtime_parts();
    let capability_identity = processes
        .component_identity(StructuredComponentKind::Capability, &capability_ref)
        .expect("qualified Read capability");
    let binding = block_on(processes.prepare_access::<ReadPhysicalBindingKind>(
        &capability_identity,
        target,
        input.clone(),
    ))
    .expect("returned binding")
    .expect("available returned binding");
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(binding)),
        QualifiedAccessCompletion::Returned(_)
    ));
    let binding = block_on(processes.prepare_access::<ReadPhysicalBindingKind>(
        &capability_identity,
        target,
        input.clone(),
    ))
    .expect("safe-failure binding")
    .expect("available safe-failure binding");
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(binding)),
        QualifiedAccessCompletion::SafeFailure(_)
    ));
    let binding = block_on(processes.prepare_access::<ReadPhysicalBindingKind>(
        &capability_identity,
        target,
        input.clone(),
    ))
    .expect("integrity binding")
    .expect("available integrity binding");
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(binding)),
        QualifiedAccessCompletion::IntegrityFault(_)
    ));
    assert_eq!(
        callback_calls.load(Ordering::SeqCst),
        qualified_callback_calls + 3
    );
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 3);
    assert_eq!(validation_counts.request.load(Ordering::SeqCst), 4);
    assert_eq!(validation_counts.returned.load(Ordering::SeqCst), 2);
    assert_eq!(
        validation_counts.safe_failure.load(Ordering::SeqCst),
        qualified_safe_failure_validations + 2
    );
}

#[test]
fn infallible_no_refresh_effect_settles_reviewed_safe_failure_as_success() {
    let operation_id = sid("mfm.test/infallible-no-refresh-effect-program");
    let mut builder =
        OperationBuilder::<ProcessValue, Never>::new(operation_id.clone(), sid("root"))
            .expect("operation builder");
    let input = builder
        .input::<ProcessValue>(sid("input"))
        .expect("input root");
    let output = builder
        .root()
        .state::<InfallibleEffectProcessState>(sid("state"), &input)
        .expect("state declaration")
        .infallible()
        .expect("infallible state");
    let completion = builder.succeed(&output).expect("root success");
    let authored = builder.finish(completion).expect("authored program");

    let mut assembly = ProgramRegistryBuilder::new();
    assembly
        .register_value::<ProcessValue>()
        .expect("value contract");
    assembly
        .register_value::<ProcessFailure>()
        .expect("safe-failure contract");

    let state_ref = state_contract::<InfallibleEffectProcessState>()
        .expect("state contract")
        .state_contract_ref;
    let state_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/infallible-effect-state-implementation",
    );
    assembly
        .register_state::<InfallibleEffectProcessState>(
            state_descriptor,
            StructuredStateCallbacks::Effect {
                request: Arc::new(|frame| frame.input().clone()),
                settle_returned: Arc::new(|_frame, returned| {
                    StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
                }),
                settle_safe_failure: Arc::new(|_frame, failure| {
                    ProposedSuccessOutcome::new(ProcessValue {
                        value: failure.code,
                    })
                }),
            },
        )
        .expect("state registration");

    let capability_contract =
        NoRefreshProcessEffectCapability::contract().expect("capability contract");
    assert!(matches!(
        capability_contract.capability_protocol,
        Some(StructuredCapabilityProtocolContract::Effect {
            refresh_contract: StructuredEffectRefreshContract::NoRefresh {},
            ..
        })
    ));
    let capability_ref = capability_contract.content_ref().expect("capability ref");
    let capability_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Capability,
        capability_ref.clone(),
        "mfm.test/no-refresh-effect-capability-implementation",
    );
    assembly
        .register_effect_capability::<NoRefreshProcessEffectCapability, _>(
            capability_descriptor,
            Arc::new(NoRefreshProcessEffectImplementation),
        )
        .expect("capability registration");

    let adapter_ref = NoRefreshProcessEffectAdapter::contract()
        .expect("adapter contract")
        .content_ref()
        .expect("adapter ref");
    let adapter_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Adapter,
        adapter_ref.clone(),
        "mfm.test/no-refresh-effect-adapter-implementation",
    );
    assembly
        .register_effect_adapter::<NoRefreshProcessEffectCapability, _>(
            adapter_descriptor,
            test_effect_binding_source(Arc::new(NoRefreshProcessEffectAdapter), 4, None),
        )
        .expect("adapter registration");
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry point");
    assert_process_graph_valid(&assembly);

    let registry = assembly.build_fixture().expect("qualified registry");
    let certified = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(authored)
        .expect("certified program");
    registry
        .admission_verifier(&operation_id)
        .expect("admission verifier")
        .verify(certified.document())
        .expect("verified document");

    let input = encode_process_value(&ProcessValue { value: 9 }).expect("encoded input");
    let run_id = test_run_id(12);
    let occurrence_id = test_occurrence_id(12);
    let routing_policy_ref = test_history_object("mfm.test/routing-policy", 12).content_ref;
    let store_scope_id = test_store_scope_id(12);
    let tenant_scope_id = test_tenant_scope_id(12);
    let source_manifest_ref = test_history_object("mfm.test/source-manifest", 12).content_ref;
    let state_input_ref = test_lexical_value_ref(12);
    let target = AccessTargetSelection {
        run_id: &run_id,
        occurrence_id: &occurrence_id,
        state_input_ref: &state_input_ref,
        store_scope_id: &store_scope_id,
        store_epoch: test_store_epoch(12),
        tenant_scope_id: &tenant_scope_id,
        admitted_prior_run_source_manifest_ref: &source_manifest_ref,
        admitted_routing_policy_ref: &routing_policy_ref,
        stable_resource_lineage_contract_ref: None,
        minimum_lineage_head_ref: None,
    };
    let (_, processes) = registry.into_runtime_parts();
    let capability_identity = processes
        .component_identity(StructuredComponentKind::Capability, &capability_ref)
        .expect("qualified Effect capability");
    let binding = block_on(processes.prepare_access::<EffectPhysicalBindingKind>(
        &capability_identity,
        target,
        input,
    ))
    .expect("Effect binding preparation")
    .expect("available Effect binding");
    let completion = block_on(processes.invoke_qualified_physical_binding_for_test(binding));
    assert!(matches!(
        completion,
        QualifiedAccessCompletion::SafeFailure(_)
    ));
}

#[test]
fn refreshable_effect_process_preserves_all_five_dispositions() {
    let operation_id = stable_id("mfm.test/effect-process-program").expect("operation id");
    let authored = program::<EffectProcessState>(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    for register in [
        ProgramRegistryBuilder::register_value::<ProcessValue>,
        ProgramRegistryBuilder::register_value::<ProcessFailure>,
        ProgramRegistryBuilder::register_value::<RefreshEvidence>,
    ] {
        register(&mut assembly).expect("process value contract");
    }
    register_process_failure_mapper(&mut assembly);

    let callback_calls = Arc::new(AtomicUsize::new(0));
    let request_calls = callback_calls.clone();
    let settle_calls = callback_calls.clone();
    let state_ref = state_contract::<EffectProcessState>()
        .expect("state contract")
        .state_contract_ref;
    let state_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::State,
        state_ref,
        "mfm.test/effect-state-implementation",
    );
    assembly
        .register_state::<EffectProcessState>(
            state_descriptor,
            StructuredStateCallbacks::Effect {
                request: Arc::new(move |frame| {
                    request_calls.fetch_add(1, Ordering::SeqCst);
                    frame.input().clone()
                }),
                settle_returned: Arc::new({
                    let settle_calls = settle_calls.clone();
                    move |_frame, returned| {
                        settle_calls.fetch_add(1, Ordering::SeqCst);
                        StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
                    }
                }),
                settle_safe_failure: Arc::new(move |_frame, failure| {
                    settle_calls.fetch_add(1, Ordering::SeqCst);
                    ProposedStateOutcome::Failure(failure.clone())
                }),
            },
        )
        .expect("effect state registration");

    let validation_counts = Arc::new(EffectValidationCounts::default());
    let capability = Arc::new(ProcessEffectImplementation {
        counts: validation_counts.clone(),
    });
    let capability_weak = Arc::downgrade(&capability);
    let capability_ref = ProcessEffectCapability::contract()
        .expect("capability contract")
        .content_ref()
        .expect("capability ref");
    let capability_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Capability,
        capability_ref.clone(),
        "mfm.test/effect-capability-implementation",
    );
    assembly
        .register_effect_capability::<ProcessEffectCapability, _>(
            capability_descriptor,
            capability.clone(),
        )
        .expect("effect capability registration");
    drop(capability);

    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let adapter = Arc::new(ProcessEffectAdapter {
        calls: adapter_calls.clone(),
    });
    let adapter_weak = Arc::downgrade(&adapter);
    let adapter_ref = ProcessEffectAdapter::contract()
        .expect("adapter contract")
        .content_ref()
        .expect("adapter ref");
    let adapter_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Adapter,
        adapter_ref.clone(),
        "mfm.test/effect-adapter-implementation",
    );
    assembly
        .register_effect_adapter::<ProcessEffectCapability, _>(
            adapter_descriptor,
            test_effect_binding_source(
                adapter.clone(),
                5,
                Some(test_history_object("mfm.test/resource-lineage-head", 5)),
            ),
        )
        .expect("effect adapter registration");
    drop(adapter);

    let resource_calls = Arc::new(AtomicUsize::new(0));
    let resource = Arc::new(ProcessResourceInvoker {
        calls: resource_calls.clone(),
    });
    let resource_weak = Arc::downgrade(&resource);
    let resource_ref = ProcessResource::contract()
        .expect("resource contract")
        .content_ref()
        .expect("resource ref");
    let resource_descriptor = descriptor(
        &mut assembly,
        StructuredComponentKind::Resource,
        resource_ref.clone(),
        "mfm.test/resource-implementation",
    );
    assembly
        .register_resource_authority::<ProcessResource, _>(resource_descriptor, resource.clone())
        .expect("resource registration");
    drop(resource);
    assembly
        .register_entry_point(operation_id.clone(), authored.clone(), profile())
        .expect("entry point");
    assert_process_graph_valid(&assembly);
    let effect_adapter_ref = adapter_ref.clone();
    assert_process_graph_rejected(&assembly, move |registry, _| {
        let adapter = registry
            .live_components
            .get_mut(&(StructuredComponentKind::Adapter, effect_adapter_ref))
            .expect("Effect adapter");
        let resource = adapter
            .dependencies
            .iter_mut()
            .find(|dependency| dependency.component_kind == StructuredComponentKind::Resource)
            .expect("Resource lineage");
        resource.component_kind = StructuredComponentKind::Signer;
    });
    let effect_adapter_ref = adapter_ref.clone();
    assert_process_graph_rejected(&assembly, move |registry, _| {
        let adapter = registry
            .live_components
            .get_mut(&(StructuredComponentKind::Adapter, effect_adapter_ref))
            .expect("Effect adapter");
        let resource = adapter
            .dependencies
            .iter_mut()
            .find(|dependency| dependency.component_kind == StructuredComponentKind::Resource)
            .expect("Resource lineage");
        resource.contract_ref =
            typed_content_ref("mfm.process-graph-hostile", &"foreign-resource-lineage")
                .expect("foreign resource lineage");
    });
    let effect_adapter_ref = adapter_ref.clone();
    assert_process_graph_rejected(&assembly, move |_, process_components| {
        let adapter = process_components
            .iter_mut()
            .find(|((kind, semantic_ref, _), _)| {
                *kind == StructuredComponentKind::Adapter && semantic_ref == &effect_adapter_ref
            })
            .map(|(_, component)| component)
            .expect("Effect adapter process");
        adapter.handle = ProcessHandle::EffectPhysicalBindingSource(Arc::new(
            TypedEffectPhysicalBindingSource::<
                AlternateProcessEffectCapability,
                TestEffectPhysicalBindingSource<AlternateProcessEffectAdapter>,
            > {
                source: test_effect_binding_source(
                    Arc::new(AlternateProcessEffectAdapter),
                    6,
                    Some(test_history_object("mfm.test/alternate-lineage-head", 6)),
                ),
                _contract: PhantomData,
            },
        ));
    });
    let effect_adapter_ref = adapter_ref.clone();
    assert_process_graph_rejected(&assembly, move |_, process_components| {
        let adapter = process_components
            .iter_mut()
            .find(|((kind, semantic_ref, _), _)| {
                *kind == StructuredComponentKind::Adapter && semantic_ref == &effect_adapter_ref
            })
            .map(|(_, component)| component)
            .expect("Effect adapter process");
        adapter.handle =
            ProcessHandle::EffectPhysicalBindingSource(Arc::new(HostileRefreshModeAdapter {
                semantic_contract_ref: effect_adapter_ref,
            }));
    });
    let registry = assembly.build_fixture().expect("qualified registry");
    assert!(capability_weak.upgrade().is_some());
    assert!(adapter_weak.upgrade().is_some());
    assert!(resource_weak.upgrade().is_some());
    // Safe-failure totality is type-enforced; qualification no longer invokes
    // settlement or capability sample validation against a reviewed corpus.
    let qualified_callback_calls = callback_calls.load(Ordering::SeqCst);
    let qualified_safe_failure_validations = validation_counts.safe_failure.load(Ordering::SeqCst);
    assert_eq!(qualified_callback_calls, 0);
    assert_eq!(qualified_safe_failure_validations, 0);

    let certified = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(authored)
        .expect("certified program");
    registry
        .admission_verifier(&operation_id)
        .expect("admission verifier")
        .verify(certified.document())
        .expect("verified document");
    assert_eq!(
        callback_calls.load(Ordering::SeqCst),
        qualified_callback_calls
    );
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(resource_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        validation_counts.safe_failure.load(Ordering::SeqCst),
        qualified_safe_failure_validations
    );
    assert_callback_free_bytes(certified.document());

    let input = encode_process_value(&ProcessValue { value: 9 }).expect("encoded input");
    let evidence =
        encode_process_value(&RefreshEvidence { generation: 2 }).expect("encoded refresh evidence");
    let fault = AccessFaultCode::new(stable_id("mfm.test/fault").expect("fault code"));
    for component in registry.process_components.values() {
        match &component.handle {
            ProcessHandle::State(callbacks) => match callbacks.kind() {
                mfm_spec::structured::StructuredExecutionKind::Effect => {
                    assert_eq!(
                        callbacks.author_request(&input).expect("effect request"),
                        input
                    );
                    let returned = encode_process_value(&CommittedObservation::<
                        ProcessValue,
                        ProcessFailure,
                    >::Returned(
                        ProcessValue { value: 10 }
                    ))
                    .expect("returned observation");
                    let returned_settlement = callbacks
                        .settle_observation(&input, &returned)
                        .expect("returned settlement");
                    let expected_returned = encode_process_value(&StateSettlement::<
                        ProcessValue,
                        ProcessFailure,
                    >::Proposed(
                        ProposedStateOutcome::Success(ProcessValue { value: 10 }),
                    ))
                    .expect("expected returned settlement");
                    assert_eq!(returned_settlement, expected_returned);

                    let safe_failure = encode_process_value(&CommittedObservation::<
                        ProcessValue,
                        ProcessFailure,
                    >::SafeFailure(
                        ProcessFailure { code: 42 }
                    ))
                    .expect("safe-failure observation");
                    let failure_settlement = callbacks
                        .settle_observation(&input, &safe_failure)
                        .expect("safe-failure settlement");
                    let expected_failure = encode_process_value(&StateSettlement::<
                        ProcessValue,
                        ProcessFailure,
                    >::Proposed(
                        ProposedStateOutcome::Failure(ProcessFailure { code: 42 }),
                    ))
                    .expect("expected safe-failure settlement");
                    assert_eq!(failure_settlement, expected_failure);
                }
                mfm_spec::structured::StructuredExecutionKind::Pure => {
                    let failure = encode_process_value(&ProcessFailure { code: 42 })
                        .expect("failure mapper input");
                    let outcome = callbacks
                        .invoke_pure(&failure)
                        .expect("failure mapper outcome");
                    let expected = encode_process_value(&ProposedStateOutcome::<
                        ProcessFailureRoute,
                        Never,
                    >::Success(
                        ProcessFailureRoute::Propagate {
                            failure: ProcessFailure { code: 42 },
                        },
                    ))
                    .expect("expected failure mapper outcome");
                    assert_eq!(outcome, expected);
                }
                mfm_spec::structured::StructuredExecutionKind::Read => {
                    panic!("unexpected Read state in Effect process")
                }
            },
            ProcessHandle::EffectCapability(implementation) => {
                implementation
                    .validate_request(&input)
                    .expect("request validation");
                implementation
                    .validate_returned(&input)
                    .expect("returned validation");
                implementation
                    .validate_safe_failure(
                        &encode_process_value(&ProcessFailure { code: 42 }).expect("safe failure"),
                    )
                    .expect("safe-failure validation");
                implementation
                    .validate_superseded_before_entry(&evidence)
                    .expect("supersession validation");
                implementation
                    .validate_entry_unknown(&fault)
                    .expect("entry-unknown validation");
                implementation
                    .validate_integrity_fault(&fault)
                    .expect("integrity validation");
            }
            ProcessHandle::EffectPhysicalBindingSource(_) => {}
            ProcessHandle::Resource(invoker) => {
                let request: &(dyn Any + Send + Sync) = &();
                let completion = block_on(invoker.invoke(request)).expect("resource completion");
                completion
                    .downcast::<()>()
                    .expect("typed resource completion");
            }
            ProcessHandle::ReadCapability(_)
            | ProcessHandle::ReadPhysicalBindingSource(_)
            | ProcessHandle::PriorRunFactScannerBindingSource
            | ProcessHandle::Signer(_) => {}
        }
    }
    let run_id = test_run_id(13);
    let occurrence_id = test_occurrence_id(13);
    let routing_policy_ref = test_history_object("mfm.test/routing-policy", 13).content_ref;
    let store_scope_id = test_store_scope_id(13);
    let tenant_scope_id = test_tenant_scope_id(13);
    let source_manifest_ref = test_history_object("mfm.test/source-manifest", 13).content_ref;
    let state_input_ref = test_lexical_value_ref(13);
    let target = AccessTargetSelection {
        run_id: &run_id,
        occurrence_id: &occurrence_id,
        state_input_ref: &state_input_ref,
        store_scope_id: &store_scope_id,
        store_epoch: test_store_epoch(13),
        tenant_scope_id: &tenant_scope_id,
        admitted_prior_run_source_manifest_ref: &source_manifest_ref,
        admitted_routing_policy_ref: &routing_policy_ref,
        stable_resource_lineage_contract_ref: Some(&resource_ref),
        minimum_lineage_head_ref: None,
    };
    let (_, processes) = registry.into_runtime_parts();
    let capability_identity = processes
        .component_identity(StructuredComponentKind::Capability, &capability_ref)
        .expect("qualified refreshable Effect capability");
    let prepare = || {
        block_on(processes.prepare_access::<EffectPhysicalBindingKind>(
            &capability_identity,
            target,
            input.clone(),
        ))
        .expect("Effect binding preparation")
        .expect("available Effect binding")
    };
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(prepare())),
        QualifiedAccessCompletion::Returned(_)
    ));
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(prepare())),
        QualifiedAccessCompletion::SafeFailure(_)
    ));
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(prepare())),
        QualifiedAccessCompletion::SupersededBeforeEntry { .. }
    ));
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(prepare())),
        QualifiedAccessCompletion::EntryUnknown(_)
    ));
    assert!(matches!(
        block_on(processes.invoke_qualified_physical_binding_for_test(prepare())),
        QualifiedAccessCompletion::IntegrityFault(_)
    ));
    assert_eq!(
        callback_calls.load(Ordering::SeqCst),
        qualified_callback_calls + 3
    );
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 5);
    assert_eq!(resource_calls.load(Ordering::SeqCst), 1);
    assert_eq!(validation_counts.request.load(Ordering::SeqCst), 6);
    assert_eq!(validation_counts.returned.load(Ordering::SeqCst), 2);
    assert_eq!(
        validation_counts.safe_failure.load(Ordering::SeqCst),
        qualified_safe_failure_validations + 2
    );
    assert_eq!(validation_counts.superseded.load(Ordering::SeqCst), 2);
    assert_eq!(validation_counts.entry_unknown.load(Ordering::SeqCst), 2);
    assert_eq!(validation_counts.integrity.load(Ordering::SeqCst), 2);
}
