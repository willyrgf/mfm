use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    BoundedComponentContract, BoundedComponentInvoker, CapabilityContractFault, ComponentFuture,
    EffectAdapterCompletion, EffectAdapterInvoker, EffectCapabilityContract,
    EffectCapabilityImplementation, ReadAdapterCompletion, ReadAdapterInvoker,
    ReadCapabilityContract, ReadCapabilityImplementation, Refreshable, ResourceAuthorityContract,
};
use mfm_certify::structured::{
    AccessTargetSelection, PhysicalBindingSelection, ProgramRegistryBuilder,
    QualifiedAccessCompletion, QualifiedProgramRegistry, ReadPhysicalBindingKind,
    RuntimeEffectPhysicalBinding, RuntimeEffectPhysicalBindingSource, RuntimeReadPhysicalBinding,
    RuntimeReadPhysicalBindingSource,
};
use mfm_facts::{
    CanonicalFactPredicate, FactOrdering, FactProposal, FactSelectionLimit, FactSelectionQuery,
    FactSelectionReadFailure, FactSelectionReadFailureCode, FactSelectionReadResponse,
    FactSelectionRequest, FactSelectionScanBounds, FactSet, FactTieBreak, ProposedFactValue,
};
use mfm_ids::{
    AppendRequestId, DigestAlgorithm, InvocationIdentity, RunId, SchemaId, StableId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    derive_commit_digest, derive_record_hash, domain_content_digest, AssignedRecord,
    CommitCandidate, CommittedBatch, HistoryObject, LexicalValueRef, ObservationOutcome,
    PriorRunFactCompletenessMode, PriorRunFactScanAttestation, PriorRunFactSelectionResponse,
    PriorRunFactSourceManifest, PriorRunFactSourceRule, RecordRef, RunRecord, TenantFactCoordinate,
    TenantFactFrontier, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_program::structured::{
    state_contract, AllowsExecution, ClosedSum, CommittedObservation, CommittedObservationView,
    DefaultFailureMapper, Direct, Effect, FanOutResults, Never, OperationBuilder,
    PriorRunFactSelectionCapability, Pure, Read, RefreshableBinding, ReviewedSafeFailureCase,
    RuntimeEffectAdapter, RuntimeEffectCapability, RuntimeReadAdapter, RuntimeReadCapability,
    RuntimeResourceAuthority, SafeFailureMayFail, SafeFailureNotApplicable, SafeFailureSuccessOnly,
    Sequential, State, StateFrame, StateSettlement, StructuredStateCallbacks,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::structured::{
    split_qualified_registry, DriveOutcome, Runtime, RuntimeFaultCode, RuntimeFaultPhase,
    RuntimeStoreFaultKind, StoreProgramVerifier,
};
use mfm_spec::structured::{
    OperationOutcome, ProposedStateOutcome, SecretFreeExecutableIdentity,
    SecretFreeImplementationDescriptor, SecretFreeQualificationArtifact,
    StructuredComponentDependency, StructuredComponentKind, StructuredExpansionProfile,
    StructuredFactDescriptor, StructuredLiveComponentContract,
};
use mfm_store::structured::{
    AccessAuthorizationProposal, AccessObservationProposal, BackendAppendOutcome,
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PhysicalBindingVerificationMode,
    ProposedCanonicalValue, ProposedObservationOutcome, PublicPhysicalBindingVerifier,
    RawRunHistory, StateTransitionProposal, StructuredAdmissionMaterial,
    StructuredAdmissionRequest, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredMemoryBackend, StructuredProgramVerifier, StructuredRunStore, StructuredStoreError,
    StructuredStoreIdentity, TenantFactPublication, ValidatedBatch,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.fixture",
    name = "value",
    version = "1",
    schema = "mfm.runtime.fixture.value"
)]
struct Value {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.fixture",
    name = "unencodable_value",
    version = "1",
    schema = "mfm.runtime.fixture.unencodable_value"
)]
struct UnencodableValue {
    value: u64,
}

impl Serialize for UnencodableValue {
    fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("opaque codec failure"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.runtime.fixture",
    name = "failure_value",
    version = "1",
    schema = "mfm.runtime.fixture.failure_value"
)]
struct FailureValue {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.runtime.fixture",
    name = "failure_route",
    version = "1",
    schema = "mfm.runtime.fixture.failure_route"
)]
enum FailureRoute {
    Propagate { failure: FailureValue },
}

impl ClosedSum for FailureRoute {}

struct ConditionalFailureState;

impl State for ConditionalFailureState {
    type Input = Value;
    type Output = Value;
    type Failure = FailureValue;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/conditional-failure-state")
    }
}

struct FailureMapperState;

impl State for FailureMapperState {
    type Input = FailureValue;
    type Output = FailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/failure-mapper-state")
    }
}

struct ValueFailureMapper;

impl DefaultFailureMapper<FailureValue, FailureValue> for ValueFailureMapper {
    type Route = FailureRoute;
    type Mapper = FailureMapperState;
}

struct PureState;

impl State for PureState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/pure-state")
    }
}

struct CodecFaultState;

impl State for CodecFaultState {
    type Input = Value;
    type Output = UnencodableValue;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/codec-fault-state")
    }
}

struct FactState;

impl State for FactState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/fact-state")
    }

    fn fact_slots() -> mfm_program::Result<Vec<mfm_spec::CertifiedFactSlot>> {
        let contract = mfm_spec::structured::structured_value_contract::<Value>()?;
        let descriptor = fact_descriptor()?;
        Ok(vec![mfm_spec::CertifiedFactSlot::new(
            0,
            1,
            1,
            descriptor.descriptor_ref,
            contract.clone(),
            contract,
        )?])
    }
}

struct PriorRunFactReadState;

impl State for PriorRunFactReadState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = FactSelectionRequest;
    type Returned = FactSelectionReadResponse;
    type SafeFailure = FactSelectionReadFailure;
    type Execution = Read<PriorRunFactSelectionCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/prior-run-fact-read-state")
    }
}

struct FixtureReadCapability;

impl ReadCapabilityContract for FixtureReadCapability {
    type Request = Value;
    type Returned = Value;
    type SafeFailure = Value;
}

impl RuntimeReadCapability for FixtureReadCapability {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        read_capability_contract()
    }
}

struct FixtureReadCapabilityImplementation;

impl ReadCapabilityImplementation<FixtureReadCapability> for FixtureReadCapabilityImplementation {
    fn validate_request(
        &self,
        _request: &Value,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &Value,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &Value,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

struct ReadState;

impl State for ReadState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = Value;
    type Returned = Value;
    type SafeFailure = Value;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/read-state")
    }
}

struct FallibleReadState;

impl State for FallibleReadState {
    type Input = Value;
    type Output = Value;
    type Failure = FailureValue;
    type Request = Value;
    type Returned = Value;
    type SafeFailure = Value;
    type Execution = Read<FixtureReadCapability>;
    type SafeFailureDisposition = SafeFailureMayFail;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/fallible-read-state")
    }
}

struct FixtureResource;
struct FixtureResourceInvoker;

impl BoundedComponentContract for FixtureResource {
    type Request = ();
    type Completion = ();
}

impl ResourceAuthorityContract for FixtureResource {}

impl RuntimeResourceAuthority for FixtureResource {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        resource_contract()
    }
}

impl BoundedComponentInvoker<FixtureResource> for FixtureResourceInvoker {
    fn invoke<'a>(&'a self, _request: &'a ()) -> ComponentFuture<'a, ()> {
        Box::pin(async {})
    }
}

struct FixtureEffectCapability;

impl EffectCapabilityContract for FixtureEffectCapability {
    type Request = Value;
    type Returned = Value;
    type SafeFailure = Value;
    type Refresh = Refreshable<Value>;
}

impl RuntimeEffectCapability for FixtureEffectCapability {
    type RefreshBinding = RefreshableBinding<FixtureResource>;

    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        effect_capability_contract()
    }
}

struct FixtureEffectCapabilityImplementation;

impl EffectCapabilityImplementation<FixtureEffectCapability>
    for FixtureEffectCapabilityImplementation
{
    fn validate_request(
        &self,
        _request: &Value,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_returned(
        &self,
        _returned: &Value,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_safe_failure(
        &self,
        _failure: &Value,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_superseded_before_entry(
        &self,
        _evidence: &Value,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_entry_unknown(
        &self,
        _fault: &mfm_capabilities::AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }

    fn validate_integrity_fault(
        &self,
        _fault: &mfm_capabilities::AccessFaultCode,
    ) -> std::result::Result<(), CapabilityContractFault> {
        Ok(())
    }
}

struct EffectState;

impl State for EffectState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;
    type Request = Value;
    type Returned = Value;
    type SafeFailure = Value;
    type Execution = Effect<FixtureEffectCapability>;
    type SafeFailureDisposition = SafeFailureSuccessOnly;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.runtime.fixture/effect-state")
    }
}

struct RotatingEffectAdapter {
    calls: Arc<AtomicUsize>,
    target_calls: Arc<AtomicUsize>,
    supersedes: bool,
    certificate: HistoryObject,
    lineage_head: HistoryObject,
}

impl EffectAdapterInvoker<FixtureEffectCapability> for RotatingEffectAdapter {
    fn invoke<'a>(
        &'a self,
        request: &'a Value,
    ) -> ComponentFuture<'a, EffectAdapterCompletion<Value, Value, Value>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.target_calls.fetch_add(1, Ordering::SeqCst);
        let supersedes = self.supersedes;
        Box::pin(async move {
            if supersedes {
                EffectAdapterCompletion::SupersededBeforeEntry(Value { value: 99 })
            } else {
                EffectAdapterCompletion::Returned(Value {
                    value: request.value + 2,
                })
            }
        })
    }
}

impl RuntimeEffectAdapter<FixtureEffectCapability> for RotatingEffectAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        effect_adapter_contract()
    }
}

impl RuntimeEffectPhysicalBinding<FixtureEffectCapability> for RotatingEffectAdapter {
    fn public_certificate(&self) -> &HistoryObject {
        &self.certificate
    }

    fn supersession_head<'a>(
        &'a self,
        _evidence: &'a Value,
    ) -> ComponentFuture<'a, Option<HistoryObject>> {
        let lineage_head = self.lineage_head.clone();
        Box::pin(async move { Some(lineage_head) })
    }
}

struct CountingReadAdapter {
    calls: Arc<AtomicUsize>,
    last_request: Arc<AtomicU64>,
    certificate: HistoryObject,
}

impl ReadAdapterInvoker<FixtureReadCapability> for CountingReadAdapter {
    fn invoke<'a>(
        &'a self,
        request: &'a Value,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<Value, Value>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.last_request.store(request.value, Ordering::SeqCst);
        Box::pin(async move {
            ReadAdapterCompletion::Returned(Value {
                value: request.value + 1,
            })
        })
    }
}

impl RuntimeReadAdapter<FixtureReadCapability> for CountingReadAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        read_adapter_contract()
    }
}

impl RuntimeReadPhysicalBinding<FixtureReadCapability> for CountingReadAdapter {
    fn public_certificate(&self) -> &HistoryObject {
        &self.certificate
    }
}

struct ConditionalFailureReadAdapter {
    calls: Arc<AtomicUsize>,
    certificate: HistoryObject,
}

impl ReadAdapterInvoker<FixtureReadCapability> for ConditionalFailureReadAdapter {
    fn invoke<'a>(
        &'a self,
        request: &'a Value,
    ) -> ComponentFuture<'a, ReadAdapterCompletion<Value, Value>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if request.value == 0 {
                ReadAdapterCompletion::SafeFailure(Value { value: 91 })
            } else {
                ReadAdapterCompletion::Returned(Value {
                    value: request.value + 1,
                })
            }
        })
    }
}

impl RuntimeReadAdapter<FixtureReadCapability> for ConditionalFailureReadAdapter {
    fn contract() -> mfm_program::Result<StructuredLiveComponentContract> {
        read_adapter_contract()
    }
}

impl RuntimeReadPhysicalBinding<FixtureReadCapability> for ConditionalFailureReadAdapter {
    fn public_certificate(&self) -> &HistoryObject {
        &self.certificate
    }
}

struct ConditionalReadBindingSource {
    binding: Arc<ConditionalFailureReadAdapter>,
}

impl RuntimeReadPhysicalBindingSource<FixtureReadCapability> for ConditionalReadBindingSource {
    type Binding = ConditionalFailureReadAdapter;

    fn current_binding<'a>(
        &'a self,
        _selection: PhysicalBindingSelection<'a>,
        _request: &'a Value,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        let binding = Arc::clone(&self.binding);
        Box::pin(async move { Some(binding) })
    }
}

struct ExactPublicBindingVerifier {
    certificate: HistoryObject,
}

impl PublicPhysicalBindingVerifier for ExactPublicBindingVerifier {
    fn verify_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        if certificate == &self.certificate
            && context.stable_resource_lineage_contract_ref.is_none()
            && context.minimum_lineage_head_ref.is_none()
        {
            Ok(())
        } else {
            Err(StructuredStoreError::Certification)
        }
    }

    fn verify_supersession(
        &self,
        _context: &PhysicalBindingSupersession<'_>,
        _public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        Err(StructuredStoreError::Certification)
    }
}

struct ExactReadBindingSource {
    binding: Arc<CountingReadAdapter>,
    calls: Arc<AtomicUsize>,
    selected_state_input: Arc<Mutex<Option<LexicalValueRef>>>,
    available: bool,
}

impl RuntimeReadPhysicalBindingSource<FixtureReadCapability> for ExactReadBindingSource {
    type Binding = CountingReadAdapter;

    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        _request: &'a Value,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self
            .selected_state_input
            .lock()
            .expect("selected input lock") = Some(selection.state_input_ref.clone());
        let binding = Arc::clone(&self.binding);
        let available = self.available;
        Box::pin(async move {
            assert!(selection.minimum_lineage_head_ref.is_none());
            assert!(selection.stable_resource_lineage_contract_ref.is_none());
            available.then_some(binding)
        })
    }
}

struct RefreshBindingVerifier {
    lineage_ref: mfm_ids::ContentRef,
    first_certificate: HistoryObject,
    second_certificate: HistoryObject,
    lineage_head: HistoryObject,
    accept_supersession: bool,
}

impl PublicPhysicalBindingVerifier for RefreshBindingVerifier {
    fn verify_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        let common = context.stable_resource_lineage_contract_ref == Some(&self.lineage_ref);
        let binding_matches = match context.minimum_lineage_head_ref {
            None => certificate == &self.first_certificate,
            Some(minimum) => {
                minimum == &self.lineage_head.content_ref && certificate == &self.second_certificate
            }
        };
        if common && binding_matches {
            Ok(())
        } else {
            Err(StructuredStoreError::Certification)
        }
    }

    fn verify_supersession(
        &self,
        context: &PhysicalBindingSupersession<'_>,
        public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        if self.accept_supersession
            && context.stable_resource_lineage_contract_ref == &self.lineage_ref
            && context.authorized_binding_ref == &self.first_certificate.content_ref
            && context.public_lineage_head_ref == &self.lineage_head.content_ref
            && public_lineage_head == &self.lineage_head
        {
            Ok(())
        } else {
            Err(StructuredStoreError::Certification)
        }
    }
}

struct RestartedRefreshBindingVerifier {
    lineage_ref: mfm_ids::ContentRef,
    first_certificate: HistoryObject,
    second_certificate: HistoryObject,
    lineage_head: HistoryObject,
    saw_retained_authorization: Arc<AtomicBool>,
    saw_retained_supersession: Arc<AtomicBool>,
    saw_current_descendant: Arc<AtomicBool>,
}

impl PublicPhysicalBindingVerifier for RestartedRefreshBindingVerifier {
    fn verify_authorization(
        &self,
        context: &PhysicalBindingAuthorization<'_>,
        certificate: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        let common = context.access_kind == mfm_journal::structured::AccessKind::Effect
            && context.stable_resource_lineage_contract_ref == Some(&self.lineage_ref);
        let valid = match context.verification_mode {
            PhysicalBindingVerificationMode::RetainedHistory => {
                self.saw_retained_authorization
                    .store(true, Ordering::SeqCst);
                match context.minimum_lineage_head_ref {
                    None => {
                        context.previous_physical_binding_ref.is_none()
                            && certificate == &self.first_certificate
                    }
                    Some(head) => {
                        head == &self.lineage_head.content_ref
                            && context.previous_physical_binding_ref
                                == Some(&self.first_certificate.content_ref)
                            && certificate == &self.second_certificate
                    }
                }
            }
            PhysicalBindingVerificationMode::CurrentCandidate => {
                let valid = context.minimum_lineage_head_ref
                    == Some(&self.lineage_head.content_ref)
                    && context.previous_physical_binding_ref
                        == Some(&self.first_certificate.content_ref)
                    && certificate == &self.second_certificate;
                if valid {
                    self.saw_current_descendant.store(true, Ordering::SeqCst);
                }
                valid
            }
        };
        if common && valid {
            Ok(())
        } else {
            Err(StructuredStoreError::Certification)
        }
    }

    fn verify_supersession(
        &self,
        context: &PhysicalBindingSupersession<'_>,
        public_lineage_head: &HistoryObject,
        _evidence: &HistoryObject,
    ) -> std::result::Result<(), StructuredStoreError> {
        let valid = context.verification_mode == PhysicalBindingVerificationMode::RetainedHistory
            && context.stable_resource_lineage_contract_ref == &self.lineage_ref
            && context.authorized_binding_ref == &self.first_certificate.content_ref
            && context.public_lineage_head_ref == &self.lineage_head.content_ref
            && public_lineage_head == &self.lineage_head;
        if valid {
            self.saw_retained_supersession.store(true, Ordering::SeqCst);
            Ok(())
        } else {
            Err(StructuredStoreError::Certification)
        }
    }
}

struct RefreshEffectBindingSource {
    first: Arc<RotatingEffectAdapter>,
    second: Arc<RotatingEffectAdapter>,
    calls: Arc<AtomicUsize>,
}

impl RuntimeEffectPhysicalBindingSource<FixtureEffectCapability> for RefreshEffectBindingSource {
    type Binding = RotatingEffectAdapter;

    fn current_binding<'a>(
        &'a self,
        selection: PhysicalBindingSelection<'a>,
        _request: &'a Value,
    ) -> ComponentFuture<'a, Option<Arc<Self::Binding>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(selection.stable_resource_lineage_contract_ref.is_some());
        let binding = if selection.minimum_lineage_head_ref.is_some() {
            Arc::clone(&self.second)
        } else {
            Arc::clone(&self.first)
        };
        Box::pin(async move { Some(binding) })
    }
}

fn refresh_effect_binding_source(
    first_certificate: HistoryObject,
    second_certificate: HistoryObject,
    lineage_head: HistoryObject,
    adapter_calls: &Arc<AtomicUsize>,
    binding_calls: &Arc<AtomicUsize>,
) -> Arc<RefreshEffectBindingSource> {
    Arc::new(RefreshEffectBindingSource {
        first: Arc::new(RotatingEffectAdapter {
            calls: Arc::clone(adapter_calls),
            target_calls: Arc::new(AtomicUsize::new(0)),
            supersedes: true,
            certificate: first_certificate,
            lineage_head: lineage_head.clone(),
        }),
        second: Arc::new(RotatingEffectAdapter {
            calls: Arc::clone(adapter_calls),
            target_calls: Arc::new(AtomicUsize::new(0)),
            supersedes: false,
            certificate: second_certificate,
            lineage_head,
        }),
        calls: Arc::clone(binding_calls),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectAppend {
    None,
    ObservationStaleHead,
    ObservationAcknowledgementUnknown,
    ObservationLoadUnavailable,
    ObservationAppendUnavailable,
    ObservationResolveUnavailable,
    ObservationReloadUnavailable,
    ObservationConcurrentSame,
    ObservationConcurrentDifferent,
    ObservationSubstitutedPositive,
    AuthorizationAcknowledgementUnknown,
    ObservationHoldAfterInvocation,
}

struct InjectingBackend {
    inner: StructuredMemoryBackend,
    injection: InjectAppend,
    stage: AtomicUsize,
    loads: Arc<AtomicUsize>,
    override_history: Mutex<Option<RawRunHistory>>,
    observation_append_entered: Arc<Notify>,
    observation_append_release: Arc<Notify>,
}

impl InjectingBackend {
    fn new(identity: StructuredStoreIdentity, injection: InjectAppend) -> Self {
        Self {
            inner: StructuredMemoryBackend::new(identity),
            injection,
            stage: AtomicUsize::new(0),
            loads: Arc::new(AtomicUsize::new(0)),
            override_history: Mutex::new(None),
            observation_append_entered: Arc::new(Notify::new()),
            observation_append_release: Arc::new(Notify::new()),
        }
    }
}

impl StructuredHistoryBackend for InjectingBackend {
    fn identity(&self) -> &StructuredStoreIdentity {
        self.inner.identity()
    }

    fn load<'a>(&'a self, run_id: &'a RunId) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            let history = self
                .override_history
                .lock()
                .map_err(|_| StructuredStoreError::BackendUnavailable)?
                .as_ref()
                .filter(|raw| &raw.run_id == run_id)
                .cloned();
            let history = match history {
                Some(history) => Some(history),
                None => self.inner.load(run_id).await?,
            };
            let has_pending_observation = history.as_ref().is_some_and(|raw| {
                let authorizations = raw
                    .batches
                    .iter()
                    .flat_map(|batch| &batch.records)
                    .filter(|record| {
                        matches!(&record.record, RunRecord::ExternalAccessAuthorized(_))
                    })
                    .count();
                let observations = raw
                    .batches
                    .iter()
                    .flat_map(|batch| &batch.records)
                    .filter(|record| matches!(&record.record, RunRecord::ExternalAccessObserved(_)))
                    .count();
                authorizations > observations
            });
            let unavailable = match self.injection {
                InjectAppend::ObservationLoadUnavailable if has_pending_observation => self
                    .stage
                    .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok(),
                InjectAppend::ObservationReloadUnavailable if has_pending_observation => self
                    .stage
                    .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok(),
                _ => false,
            };
            if unavailable {
                Err(StructuredStoreError::BackendUnavailable)
            } else {
                Ok(history)
            }
        })
    }

    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, TenantFactFrontier> {
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
        Box::pin(async move {
            let is_observation = batch
                .committed()
                .records
                .iter()
                .any(|record| matches!(&record.record, RunRecord::ExternalAccessObserved(_)));
            let is_authorization = batch
                .committed()
                .records
                .iter()
                .any(|record| matches!(&record.record, RunRecord::ExternalAccessAuthorized(_)));
            let should_inject = (is_observation
                && matches!(
                    self.injection,
                    InjectAppend::ObservationStaleHead
                        | InjectAppend::ObservationAcknowledgementUnknown
                        | InjectAppend::ObservationAppendUnavailable
                        | InjectAppend::ObservationResolveUnavailable
                        | InjectAppend::ObservationReloadUnavailable
                        | InjectAppend::ObservationConcurrentSame
                        | InjectAppend::ObservationConcurrentDifferent
                        | InjectAppend::ObservationSubstitutedPositive
                        | InjectAppend::ObservationHoldAfterInvocation
                ))
                || (is_authorization
                    && self.injection == InjectAppend::AuthorizationAcknowledgementUnknown);
            if should_inject
                && self
                    .stage
                    .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                match self.injection {
                    InjectAppend::ObservationStaleHead
                    | InjectAppend::ObservationReloadUnavailable => {
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    InjectAppend::ObservationAcknowledgementUnknown
                    | InjectAppend::ObservationResolveUnavailable
                    | InjectAppend::AuthorizationAcknowledgementUnknown => {
                        self.inner.acknowledge_next_commit_as_unknown()?;
                    }
                    InjectAppend::ObservationAppendUnavailable => {
                        return Err(StructuredStoreError::BackendUnavailable);
                    }
                    InjectAppend::ObservationConcurrentSame => {
                        let outcome = self.inner.append(batch).await?;
                        if !matches!(outcome, BackendAppendOutcome::NewlyCommitted(_)) {
                            return Err(StructuredStoreError::InvalidHistory);
                        }
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    InjectAppend::ObservationConcurrentDifferent => {
                        let committed = conflicting_observation_batch(batch.into_committed())?;
                        let mut history = self
                            .inner
                            .load(&committed.records[0].record_ref.run_id)
                            .await?
                            .ok_or(StructuredStoreError::RunNotFound)?;
                        history.batches.push(committed);
                        *self
                            .override_history
                            .lock()
                            .map_err(|_| StructuredStoreError::BackendUnavailable)? = Some(history);
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    InjectAppend::ObservationSubstitutedPositive => {
                        let mut returned = batch.into_committed();
                        returned.append_request_id =
                            AppendRequestId::new("substituted-supersession-append")
                                .map_err(|_| StructuredStoreError::InvalidHistory)?;
                        return Ok(BackendAppendOutcome::NewlyCommitted(returned));
                    }
                    InjectAppend::ObservationHoldAfterInvocation => {
                        self.observation_append_entered.notify_one();
                        self.observation_append_release.notified().await;
                    }
                    InjectAppend::None | InjectAppend::ObservationLoadUnavailable => {}
                }
            }
            self.inner.append(batch).await
        })
    }

    fn resolve_append<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
        candidate_digest: &'a mfm_ids::ContentDigest,
    ) -> StructuredBackendFuture<'a, Option<mfm_journal::structured::CommittedBatch>> {
        Box::pin(async move {
            if self.injection == InjectAppend::ObservationResolveUnavailable
                && self
                    .stage
                    .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            self.inner
                .resolve_append(run_id, append_request_id, candidate_digest)
                .await
        })
    }
}

fn conflicting_observation_batch(
    original: CommittedBatch,
) -> std::result::Result<CommittedBatch, StructuredStoreError> {
    let [assigned] = original.records.as_slice() else {
        return Err(StructuredStoreError::InvalidHistory);
    };
    let RunRecord::ExternalAccessObserved(observation) = &assigned.record else {
        return Err(StructuredStoreError::InvalidHistory);
    };
    let mut observation = observation.clone();
    observation.outcome = ObservationOutcome::IntegrityFault {
        fault_code: StableId::new("mfm.runtime.fixture/concurrent-observation-conflict")
            .map_err(|_| StructuredStoreError::InvalidHistory)?,
    };
    let run_id = assigned.record_ref.run_id.clone();
    let append_request_id = AppendRequestId::new("concurrent-conflicting-observation")
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    let record = RunRecord::ExternalAccessObserved(observation);
    let candidate = CommitCandidate {
        run_id: run_id.clone(),
        expected_head: original.predecessor.clone(),
        append_request_id: append_request_id.clone(),
        tenant_fact_coordinate: TenantFactCoordinate::None,
        records: vec![record.clone()],
        objects: Vec::new(),
    };
    let candidate_digest = domain_content_digest("mfm.structured-candidate.v1", &candidate)
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    let run_sequence = original.head.run_sequence;
    let ordinal = 0_u32;
    let record_hash = derive_record_hash(&serde_json::json!({
        "ordinal": ordinal,
        "record": &record,
        "run_id": &run_id,
        "run_sequence": run_sequence,
    }))
    .map_err(|_| StructuredStoreError::InvalidHistory)?;
    let record_ref = RecordRef {
        run_id,
        run_sequence,
        ordinal,
        record_hash,
    };
    let commit_digest = derive_commit_digest(&serde_json::json!({
        "append_request_id": &append_request_id,
        "candidate_digest": &candidate_digest,
        "object_refs": Vec::<&mfm_ids::ContentRef>::new(),
        "predecessor": &original.predecessor,
        "record_refs": vec![&record_ref],
        "store_epoch": original.store_epoch,
        "store_scope_id": &original.store_scope_id,
        "tenant_fact_coordinate": TenantFactCoordinate::None,
    }))
    .map_err(|_| StructuredStoreError::InvalidHistory)?;
    Ok(CommittedBatch {
        store_scope_id: original.store_scope_id,
        store_epoch: original.store_epoch,
        predecessor: original.predecessor,
        append_request_id,
        tenant_fact_coordinate: TenantFactCoordinate::None,
        candidate_digest,
        records: vec![AssignedRecord { record_ref, record }],
        objects: Vec::new(),
        head: mfm_journal::structured::JournalHead {
            run_sequence,
            commit_digest,
        },
    })
}

#[tokio::test]
async fn pure_runtime_commits_the_exact_callback_output_once() {
    let callback_calls = Arc::new(AtomicUsize::new(0));
    let callback_input = Arc::new(AtomicU64::new(0));
    let operation_id = stable("mfm.runtime.fixture/pure-operation").expect("operation id");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    let calls = Arc::clone(&callback_calls);
    let input = Arc::clone(&callback_input);
    let descriptor = implementation_descriptor::<PureState>(&mut assembly, "pure");
    assembly
        .register_state::<PureState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, Value>| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let input_value = frame.input().value;
                    input.store(input_value, Ordering::SeqCst);
                    ProposedStateOutcome::Success(Value {
                        value: input_value + 1,
                    })
                }),
            },
        )
        .expect("pure state");
    assembly
        .register_entry_point(
            operation_id.clone(),
            one_state_program::<PureState>(operation_id.clone()),
            profile(),
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<PureState>(operation_id.clone()))
        .expect("certified program")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let certificate = binding_object(1);
    let backend = InjectingBackend::new(store_identity(1), InjectAppend::None);
    let history_loads = Arc::clone(&backend.loads);
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(1);
    runtime
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            4,
            "pure-admit",
        ))
        .await
        .expect("admission");
    history_loads.store(0, Ordering::SeqCst);

    assert_eq!(
        runtime.drive_once(&run_id).await.expect("drive"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    assert_eq!(history_loads.load(Ordering::SeqCst), 1);
    assert_eq!(callback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(callback_input.load(Ordering::SeqCst), 4);
    let verified = reader.load_verified(&run_id).await.expect("closed");
    assert!(verified
        .admission()
        .admission_material_refs
        .stable_resource_lineage_contract_refs
        .is_empty());
    assert!(matches!(
        verified.frontier(),
        mfm_store::structured::StructuredFrontier::Complete
    ));
    assert_eq!(
        runtime.drive_once(&run_id).await.expect("closed drive"),
        DriveOutcome::Closed
    );
    assert_eq!(history_loads.load(Ordering::SeqCst), 3);
    assert_eq!(callback_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pure_callback_fault_is_repeatable_attributed_and_history_preserving() {
    let callback_calls = Arc::new(AtomicUsize::new(0));
    let operation_id = stable("mfm.runtime.fixture/pure-callback-fault").expect("operation id");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    let calls = Arc::clone(&callback_calls);
    let state_contract_ref = state_contract::<PureState>()
        .expect("Pure state contract")
        .state_contract_ref;
    let descriptor = implementation_descriptor::<PureState>(&mut assembly, "panicking-pure");
    assembly
        .register_state::<PureState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |_frame: StateFrame<'_, Value>| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    panic!("opaque callback failure")
                }),
            },
        )
        .expect("Pure state");
    assembly
        .register_entry_point(
            operation_id.clone(),
            one_state_program::<PureState>(operation_id.clone()),
            profile(),
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<PureState>(operation_id.clone()))
        .expect("certified program")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let backend = StructuredMemoryBackend::new(store_identity(60));
    let backend_probe = backend.clone();
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(60),
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(60);
    runtime
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            4,
            "callback-fault-admit",
        ))
        .await
        .expect("admission");
    let pre_fault_head = reader
        .load_verified(&run_id)
        .await
        .expect("admitted run")
        .journal_head()
        .clone();
    let raw_before = backend_probe
        .load(&run_id)
        .await
        .expect("raw pre-fault history")
        .expect("admitted history");

    let first = runtime
        .drive_once(&run_id)
        .await
        .expect_err("panicking callback must be classified");
    let second = runtime
        .drive_once(&run_id)
        .await
        .expect_err("same cursor must repeat the attributed fault");
    assert_eq!(first, second);
    assert_eq!(first.code(), RuntimeFaultCode::CallbackFault);
    assert_eq!(first.phase(), RuntimeFaultPhase::InvokePure);
    assert_eq!(first.run_id(), &run_id);
    assert_eq!(first.pre_fault_head(), Some(&pre_fault_head));
    assert!(first.occurrence_id().is_some());
    let mfm_runtime::structured::RuntimeFaultSubject::Process(component) = first.subject() else {
        panic!("callback fault must identify its qualified state")
    };
    assert_eq!(component.component_kind(), StructuredComponentKind::State);
    assert_eq!(component.semantic_contract_ref(), &state_contract_ref);
    assert_ne!(
        component.implementation_contract_ref(),
        component.semantic_contract_ref()
    );
    assert_eq!(callback_calls.load(Ordering::SeqCst), 2);
    assert_eq!(first.to_string(), "structured Runtime fault");
    assert!(!format!("{first:?}").contains("opaque callback failure"));
    assert_eq!(
        backend_probe
            .load(&run_id)
            .await
            .expect("raw post-fault history")
            .expect("admitted history"),
        raw_before
    );
}

#[tokio::test]
async fn callback_output_codec_fault_is_attributed_without_candidate_authority() {
    let operation_id = stable("mfm.runtime.fixture/codec-fault-operation").expect("operation id");
    let program = codec_fault_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("input contract");
    assembly
        .register_value::<UnencodableValue>()
        .expect("output contract");
    let state_contract_ref = state_contract::<CodecFaultState>()
        .expect("codec state contract")
        .state_contract_ref;
    let descriptor = implementation_descriptor::<CodecFaultState>(&mut assembly, "codec-fault");
    assembly
        .register_state::<CodecFaultState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(|frame: StateFrame<'_, Value>| {
                    ProposedStateOutcome::Success(UnencodableValue {
                        value: frame.input().value,
                    })
                }),
            },
        )
        .expect("codec state");
    assembly
        .register_entry_point(operation_id.clone(), program.clone(), profile())
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(program)
        .expect("certified program")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let backend = StructuredMemoryBackend::new(store_identity(62));
    let backend_probe = backend.clone();
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(62),
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(62);
    runtime
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            4,
            "codec-fault-admit",
        ))
        .await
        .expect("admission");
    let pre_fault_head = reader
        .load_verified(&run_id)
        .await
        .expect("admitted run")
        .journal_head()
        .clone();
    let raw_before = backend_probe
        .load(&run_id)
        .await
        .expect("raw pre-fault history")
        .expect("admitted history");

    let fault = runtime
        .drive_once(&run_id)
        .await
        .expect_err("unencodable callback output");
    assert_eq!(fault.code(), RuntimeFaultCode::CodecFault);
    assert_eq!(fault.phase(), RuntimeFaultPhase::InvokePure);
    assert_eq!(fault.pre_fault_head(), Some(&pre_fault_head));
    let mfm_runtime::structured::RuntimeFaultSubject::Process(component) = fault.subject() else {
        panic!("codec fault must identify the callback state")
    };
    assert_eq!(component.component_kind(), StructuredComponentKind::State);
    assert_eq!(component.semantic_contract_ref(), &state_contract_ref);
    assert!(!format!("{fault:?}").contains("opaque codec failure"));
    assert_eq!(
        backend_probe
            .load(&run_id)
            .await
            .expect("raw post-fault history")
            .expect("admitted history"),
        raw_before
    );
}

#[tokio::test]
async fn fan_out_structural_values_survive_fresh_persisted_folds() {
    let callback_calls = Arc::new(AtomicUsize::new(0));
    let operation_id = stable("mfm.runtime.fixture/fan-out-operation").expect("operation id");
    let template = fan_out_program(operation_id.clone());
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    let calls = Arc::clone(&callback_calls);
    let descriptor = implementation_descriptor::<PureState>(&mut assembly, "fan-out");
    assembly
        .register_state::<PureState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, Value>| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    ProposedStateOutcome::Success(Value {
                        value: frame.input().value + 1,
                    })
                }),
            },
        )
        .expect("pure state");
    assembly
        .register_entry_point(operation_id.clone(), template.clone(), profile())
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(template)
        .expect("certified program")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let fresh_program_verifier = Arc::clone(&program_verifier);
    let certificate = binding_object(35);
    let backend = StructuredMemoryBackend::new(store_identity(35));
    let backend_probe = backend.clone();
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let (writer, _) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(35);
    runtime
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            4,
            "fan-out-admit",
        ))
        .await
        .expect("admission");

    for drive_index in 0..8 {
        let outcome = runtime.drive_once(&run_id).await.expect("fan-out drive");
        match outcome {
            DriveOutcome::TransitionCommitted { closed: true } => break,
            DriveOutcome::TransitionCommitted { closed: false } => {}
            other => panic!("fan-out drive {drive_index} returned {other:?}"),
        }
        assert!(
            drive_index < 7,
            "fan-out run did not close within its bound"
        );
    }
    assert_eq!(callback_calls.load(Ordering::SeqCst), 2);

    let fresh_store = StructuredRunStore::new(
        backend_probe,
        fresh_program_verifier,
        Arc::new(ExactPublicBindingVerifier { certificate }),
    );
    let (_, fresh_reader) = fresh_store.split();
    let verified = fresh_reader
        .load_verified(&run_id)
        .await
        .expect("fresh persisted fold");
    let mfm_store::structured::ProgramCursor::Closed { .. } = verified.cursor() else {
        panic!("fan-out run must close");
    };
    let semantic_records = verified.semantic_records().collect::<Vec<_>>();
    assert_eq!(semantic_records.len() + 1, verified.records().len());
    assert!(matches!(
        semantic_records
            .last()
            .expect("semantic head record")
            .record,
        RunRecord::StateTransitionCommitted(_)
    ));
    assert!(matches!(
        verified.records().last().expect("audit head record").record,
        RunRecord::RunClosed(_)
    ));
    let outcome = mfm_replay::structured::project_operation_outcome(&verified)
        .expect("project persisted root outcome")
        .expect("closed root outcome");
    assert_eq!(outcome.kind(), "success");
    let join: serde_json::Value = serde_json::from_slice(outcome.canonical_value().as_bytes())
        .expect("decode projected fan-out join");
    assert_eq!(
        join["declaration_ordered"]
            .as_array()
            .unwrap_or_else(|| panic!("declaration-ordered lane outcomes: {join}"))
            .len(),
        2
    );
}

#[tokio::test]
async fn exact_root_program_cache_is_shared_and_fresh_verifiers_recertify() {
    let callback_calls = Arc::new(AtomicUsize::new(0));
    let (operation_id, template, alternate, registry) =
        program_cache_registry(Arc::clone(&callback_calls));
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(template)
        .expect("certified program")
        .into_document();
    let alternate_document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(alternate)
        .expect("alternate certified program")
        .into_document();
    assert_ne!(
        document.root.content_ref().expect("template root"),
        alternate_document
            .root
            .content_ref()
            .expect("alternate root")
    );
    let admission_registry = registry.admission_verification_registry();
    let verifier = StoreProgramVerifier::new(admission_registry.clone());
    let authored = document
        .component_closure
        .iter()
        .find(|object| object.content_ref == document.root.components.authored_program_ref)
        .expect("authored component")
        .value
        .clone();
    let alternate_authored = alternate_document
        .component_closure
        .iter()
        .find(|object| {
            object.content_ref == alternate_document.root.components.authored_program_ref
        })
        .expect("alternate authored component")
        .value
        .clone();

    let first = verifier
        .verify(&operation_id, &document.root, &authored)
        .expect("first exact-root certification");
    let repeated = verifier
        .verify(&operation_id, &document.root, &authored)
        .expect("cached exact-root certification");
    assert!(Arc::ptr_eq(&first, &repeated));
    assert_eq!(
        verifier.verify(&operation_id, &document.root, &alternate_authored),
        Err(StructuredStoreError::Certification),
        "a cache hit must still bind the exact persisted authored object"
    );
    let alternate_first = verifier
        .verify(&operation_id, &alternate_document.root, &alternate_authored)
        .expect("alternate exact-root certification");
    let alternate_repeated = verifier
        .verify(&operation_id, &alternate_document.root, &alternate_authored)
        .expect("cached alternate exact-root certification");
    assert!(Arc::ptr_eq(&alternate_first, &alternate_repeated));
    assert!(!Arc::ptr_eq(&first, &alternate_first));
    assert_eq!(
        verifier.verify(&operation_id, &alternate_document.root, &authored),
        Err(StructuredStoreError::Certification),
        "alternate cached root must reject the first authored object"
    );
    assert!(Arc::ptr_eq(
        &first,
        &verifier
            .verify(&operation_id, &document.root, &authored)
            .expect("template remains cached after alternate")
    ));

    let wrong_entry =
        stable("mfm.runtime.fixture/unqualified-cache-entry").expect("wrong entry id");
    assert_eq!(
        verifier.verify(&wrong_entry, &document.root, &authored),
        Err(StructuredStoreError::Certification)
    );
    let mut wrong_root = document.root.clone();
    wrong_root.canonical_component_closure_digest = mfm_ids::ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(b"wrong cached root"),
    );
    assert_eq!(
        verifier.verify(&operation_id, &wrong_root, &authored),
        Err(StructuredStoreError::Certification)
    );

    let fresh_verifier = StoreProgramVerifier::new(admission_registry);
    let fresh = fresh_verifier
        .verify(&operation_id, &document.root, &authored)
        .expect("fresh verifier recertification");
    assert!(!Arc::ptr_eq(&first, &fresh));
    let fresh_alternate = fresh_verifier
        .verify(&operation_id, &alternate_document.root, &alternate_authored)
        .expect("fresh alternate verifier recertification");
    assert!(!Arc::ptr_eq(&alternate_first, &fresh_alternate));

    let backend = StructuredMemoryBackend::new(store_identity(34));
    let backend_after_restart = backend.clone();
    let (_, processes) = registry.into_runtime_parts();
    let store = StructuredRunStore::new(
        backend,
        Arc::new(verifier),
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(34),
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let first_run_id = run_id(34);
    let alternate_run_id = run_id(36);
    runtime
        .admit_run(admission_with_invocation(
            first_run_id.clone(),
            operation_id.clone(),
            document.clone(),
            4,
            "00000000-0000-4000-8000-000000000034",
            "program-cache-admit",
        ))
        .await
        .expect("cached program admission");
    runtime
        .admit_run(admission_with_invocation(
            alternate_run_id.clone(),
            operation_id.clone(),
            alternate_document.clone(),
            9,
            "00000000-0000-4000-8000-000000000036",
            "program-cache-alternate-admit",
        ))
        .await
        .expect("alternate cached program admission");
    let first_persisted_root_ref = reader
        .load_verified(&first_run_id)
        .await
        .expect("pre-restart first admitted root")
        .admission()
        .certified_program_root_ref
        .clone();
    let alternate_persisted_root_ref = reader
        .load_verified(&alternate_run_id)
        .await
        .expect("pre-restart alternate admitted root")
        .admission()
        .certified_program_root_ref
        .clone();
    assert_ne!(first_persisted_root_ref, alternate_persisted_root_ref);
    assert_eq!(callback_calls.load(Ordering::SeqCst), 0);
    drop(reader);
    drop(runtime);

    let (restarted_operation_id, _, _, restarted_registry) =
        program_cache_registry(Arc::clone(&callback_calls));
    assert_eq!(restarted_operation_id, operation_id);
    let (restarted_verifier, restarted_processes) = split_qualified_registry(restarted_registry);
    let restarted_store = StructuredRunStore::new(
        backend_after_restart,
        restarted_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(34),
        }),
    );
    let (restarted_writer, restarted_reader) = restarted_store.split();
    let restarted_runtime = Runtime::new(restarted_writer, restarted_processes);

    for (run_id, expected_value, expected_document, expected_root_ref) in [
        (&first_run_id, 4, &document, &first_persisted_root_ref),
        (
            &alternate_run_id,
            9,
            &alternate_document,
            &alternate_persisted_root_ref,
        ),
    ] {
        let admitted = restarted_reader
            .load_verified(run_id)
            .await
            .expect("fresh verifier persisted fold");
        assert_eq!(
            &admitted.admission().certified_program_root_ref,
            expected_root_ref
        );
        assert_eq!(
            restarted_runtime
                .drive_once(run_id)
                .await
                .expect("fresh Runtime drive"),
            DriveOutcome::TransitionCommitted { closed: true }
        );
        let verified = restarted_reader
            .load_verified(run_id)
            .await
            .expect("fresh Runtime closed fold");
        assert!(expected_document.component_closure.iter().all(|component| {
            verified
                .object(&component.content_ref)
                .is_some_and(|object| object.object_type == component.object_type)
        }));
        let outcome = mfm_replay::structured::project_operation_outcome(&verified)
            .expect("fresh Runtime projected outcome")
            .expect("fresh Runtime closed outcome");
        assert_eq!(outcome.kind(), "success");
        assert_eq!(
            serde_json::from_slice::<Value>(outcome.canonical_value().as_bytes())
                .expect("fresh Runtime exact output"),
            Value {
                value: expected_value
            }
        );
    }
    assert_eq!(callback_calls.load(Ordering::SeqCst), 2);
    let first_verified = restarted_reader
        .load_verified(&first_run_id)
        .await
        .expect("first cached program full fold");
    assert!(document.component_closure.iter().all(|component| {
        first_verified
            .object(&component.content_ref)
            .is_some_and(|object| object.object_type == component.object_type)
    }));
}

#[tokio::test]
async fn successful_callback_facts_commit_with_the_exact_atomic_object_closure() {
    let callback_calls = Arc::new(AtomicUsize::new(0));
    let operation_id = stable("mfm.runtime.fixture/fact-operation").expect("operation id");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    assembly
        .register_fact_descriptor(fact_descriptor().expect("fact descriptor"))
        .expect("fact descriptor registration");
    let calls = Arc::clone(&callback_calls);
    let descriptor = implementation_descriptor::<FactState>(&mut assembly, "fact-state");
    assembly
        .register_state::<FactState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, Value>| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let subject = frame.input().clone();
                    let response = Value {
                        value: subject.value + 1,
                    };
                    ProposedStateOutcome::success_with_facts(
                        response.clone(),
                        fact_set(subject, response),
                    )
                }),
            },
        )
        .expect("fact state");
    assembly
        .register_entry_point(
            operation_id.clone(),
            one_state_program::<FactState>(operation_id.clone()),
            profile(),
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<FactState>(operation_id.clone()))
        .expect("certified program")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let certificate = binding_object(2);
    let backend = StructuredMemoryBackend::new(store_identity(2));
    let backend_probe = backend.clone();
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(2);
    runtime
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            7,
            "fact-admit",
        ))
        .await
        .expect("admission");
    assert_eq!(
        runtime.drive_once(&run_id).await.expect("fact drive"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    assert_eq!(callback_calls.load(Ordering::SeqCst), 1);

    let raw = backend_probe
        .load(&run_id)
        .await
        .expect("raw memory history")
        .expect("persisted run");
    assert_eq!(raw.batches.len(), 2);
    assert_eq!(raw.batches[1].records.len(), 2);
    let RunRecord::StateTransitionCommitted(transition) = &raw.batches[1].records[0].record else {
        panic!("second append must begin with the state transition");
    };
    assert_eq!(transition.facts.len(), 1);
    assert!(matches!(
        raw.batches[1].records[1].record,
        RunRecord::RunClosed(_)
    ));
    let fact = &transition.facts[0];
    assert_eq!(fact.emission_ordinal, 0);
    assert_eq!(fact.fact_slot_ordinal, 0);
    let verified = reader.load_verified(&run_id).await.expect("verified facts");
    let subject: Value = verified
        .object(&fact.subject.value_ref)
        .expect("fact subject")
        .decode()
        .expect("fact subject value");
    let response: Value = verified
        .object(&fact.response.value_ref)
        .expect("fact response")
        .decode()
        .expect("fact response value");
    assert_eq!(subject, Value { value: 7 });
    assert_eq!(response, Value { value: 8 });

    let object_count = raw
        .batches
        .iter()
        .map(|batch| batch.objects.len())
        .sum::<usize>();
    let unique_objects = raw
        .batches
        .iter()
        .flat_map(|batch| batch.objects.iter().map(|object| &object.content_ref))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique_objects.len(), object_count);
}

#[tokio::test]
async fn prior_run_fact_read_captures_frontier_and_returns_complete_typed_attestation() {
    let producer_operation =
        stable("mfm.runtime.fixture/fact-producer-operation").expect("producer operation");
    let consumer_operation =
        stable("mfm.runtime.fixture/fact-consumer-operation").expect("consumer operation");
    let self_scanning_operation = stable("mfm.runtime.fixture/self-scanning-fact-operation")
        .expect("self-scanning operation");
    let descriptor = fact_descriptor().expect("fact descriptor");
    let source_manifest = PriorRunFactSourceManifest::new(vec![
        PriorRunFactSourceRule::new(
            producer_operation.clone(),
            Vec::new(),
            vec![descriptor.descriptor_ref.clone()],
        )
        .expect("producer source rule"),
        PriorRunFactSourceRule::new(
            self_scanning_operation.clone(),
            Vec::new(),
            vec![descriptor.descriptor_ref.clone()],
        )
        .expect("self-scanning source rule"),
    ])
    .expect("source manifest");
    let source_object = source_manifest
        .to_history_object()
        .expect("source manifest object");
    let request = FactSelectionRequest::new(
        source_object.content_ref.clone(),
        FactSelectionScanBounds::new(8, 64, 65_536, 16, 65_536).expect("scan bounds"),
        vec![FactSelectionQuery::new(
            descriptor.descriptor_ref.clone(),
            CanonicalFactPredicate::from_canonical_json(br#"{"value":7}"#).expect("fact predicate"),
            None,
            FactOrdering::Ascending,
            FactSelectionLimit::new(4).expect("selection limit"),
            FactTieBreak::FactIdentityAscending,
        )
        .expect("fact query")],
    )
    .expect("fact request");
    let bounded_request = FactSelectionRequest::new(
        source_object.content_ref.clone(),
        FactSelectionScanBounds::new(1, 64, 65_536, 16, 65_536).expect("bounded scan limits"),
        request.queries().expect("bounded request queries"),
    )
    .expect("bounded fact request");
    let fact_bounded_request = FactSelectionRequest::new(
        source_object.content_ref.clone(),
        FactSelectionScanBounds::new(8, 1, 65_536, 16, 65_536).expect("fact-bounded scan limits"),
        request.queries().expect("fact-bounded request queries"),
    )
    .expect("fact-bounded request");
    let source_bounded_request = FactSelectionRequest::new(
        source_object.content_ref.clone(),
        FactSelectionScanBounds::new(8, 64, 1, 16, 65_536).expect("source-bounded scan limits"),
        request.queries().expect("source-bounded request queries"),
    )
    .expect("source-bounded request");
    let selection_bounded_request = FactSelectionRequest::new(
        source_object.content_ref.clone(),
        FactSelectionScanBounds::new(8, 64, 65_536, 1, 65_536)
            .expect("selection-bounded scan limits"),
        request
            .queries()
            .expect("selection-bounded request queries"),
    )
    .expect("selection-bounded request");
    let response_bounded_request = FactSelectionRequest::new(
        source_object.content_ref.clone(),
        FactSelectionScanBounds::new(8, 64, 65_536, 16, 1).expect("response-bounded scan limits"),
        request.queries().expect("response-bounded request queries"),
    )
    .expect("response-bounded request");

    let producer_calls = Arc::new(AtomicUsize::new(0));
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let selected_response = Arc::new(AtomicU64::new(0));
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    assembly
        .register_fact_descriptor(descriptor)
        .expect("fact descriptor registration");

    let producer_counter = Arc::clone(&producer_calls);
    let producer_descriptor =
        implementation_descriptor::<FactState>(&mut assembly, "fact-producer-state");
    assembly
        .register_state::<FactState>(
            producer_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, Value>| {
                    producer_counter.fetch_add(1, Ordering::SeqCst);
                    let subject = frame.input().clone();
                    let response = Value {
                        value: subject.value + 1,
                    };
                    ProposedStateOutcome::success_with_facts(
                        response.clone(),
                        fact_set(subject, response),
                    )
                }),
            },
        )
        .expect("producer state");

    let authored_request = request.clone();
    let authored_bounded_request = bounded_request;
    let authored_fact_bounded_request = fact_bounded_request;
    let authored_source_bounded_request = source_bounded_request;
    let authored_selection_bounded_request = selection_bounded_request;
    let authored_response_bounded_request = response_bounded_request;
    let settlement_counter = Arc::clone(&settlement_calls);
    let selected_probe = Arc::clone(&selected_response);
    let consumer_descriptor =
        implementation_descriptor::<PriorRunFactReadState>(&mut assembly, "fact-consumer-state");
    assembly
        .register_state::<PriorRunFactReadState>(
            consumer_descriptor,
            StructuredStateCallbacks::Read {
                request: Arc::new(move |frame| match frame.input().value {
                    1 => authored_bounded_request.clone(),
                    2 => authored_fact_bounded_request.clone(),
                    3 => authored_source_bounded_request.clone(),
                    4 => authored_selection_bounded_request.clone(),
                    5 => authored_response_bounded_request.clone(),
                    _ => authored_request.clone(),
                }),
                settle: Arc::new(move |_frame, observation| {
                    settlement_counter.fetch_add(1, Ordering::SeqCst);
                    let output = match observation.observation() {
                        CommittedObservation::Returned(returned) => {
                            let response: PriorRunFactSelectionResponse =
                                serde_json::from_str(returned.canonical_response_json())
                                    .expect("typed fact response");
                            let selected = &response.query_results[0].selected[0];
                            serde_json::from_str::<Value>(&selected.response_canonical_json)
                                .expect("selected response value")
                        }
                        CommittedObservation::SafeFailure(_) => Value { value: 0 },
                    };
                    selected_probe.store(output.value, Ordering::SeqCst);
                    StateSettlement::Proposed(ProposedStateOutcome::Success(output))
                }),
                reviewed_safe_failures: vec![
                    ReviewedSafeFailureCase::new(
                        Value { value: 0 },
                        FactSelectionReadFailure::new(
                            FactSelectionReadFailureCode::StoreUnavailable,
                        ),
                        ProposedStateOutcome::Success(Value { value: 0 }),
                    ),
                    ReviewedSafeFailureCase::new(
                        Value { value: 1 },
                        FactSelectionReadFailure::new(
                            FactSelectionReadFailureCode::PublicationBoundExceeded,
                        ),
                        ProposedStateOutcome::Success(Value { value: 0 }),
                    ),
                    ReviewedSafeFailureCase::new(
                        Value { value: 2 },
                        FactSelectionReadFailure::new(
                            FactSelectionReadFailureCode::FactBoundExceeded,
                        ),
                        ProposedStateOutcome::Success(Value { value: 0 }),
                    ),
                    ReviewedSafeFailureCase::new(
                        Value { value: 3 },
                        FactSelectionReadFailure::new(
                            FactSelectionReadFailureCode::RetainedSourceBoundExceeded,
                        ),
                        ProposedStateOutcome::Success(Value { value: 0 }),
                    ),
                    ReviewedSafeFailureCase::new(
                        Value { value: 4 },
                        FactSelectionReadFailure::new(
                            FactSelectionReadFailureCode::SelectedResultBoundExceeded,
                        ),
                        ProposedStateOutcome::Success(Value { value: 0 }),
                    ),
                    ReviewedSafeFailureCase::new(
                        Value { value: 5 },
                        FactSelectionReadFailure::new(
                            FactSelectionReadFailureCode::ResponseBoundExceeded,
                        ),
                        ProposedStateOutcome::Success(Value { value: 0 }),
                    ),
                ],
            },
        )
        .expect("consumer state");

    for (operation, program) in [
        (
            producer_operation.clone(),
            one_state_program::<FactState>(producer_operation.clone()),
        ),
        (
            consumer_operation.clone(),
            one_state_program::<PriorRunFactReadState>(consumer_operation.clone()),
        ),
        (
            self_scanning_operation.clone(),
            fact_then_read_program(self_scanning_operation.clone()),
        ),
    ] {
        assembly
            .register_entry_point(operation, program, profile())
            .expect("entry point");
    }
    let expected_entry_points = [
        producer_operation.clone(),
        consumer_operation.clone(),
        self_scanning_operation.clone(),
    ];
    let registry = assembly
        .build(&expected_entry_points)
        .expect("qualified fact registry");
    let producer_document = registry
        .certifier(&producer_operation)
        .expect("producer certifier")
        .certify(one_state_program::<FactState>(producer_operation.clone()))
        .expect("producer certification")
        .into_document();
    let consumer_document = registry
        .certifier(&consumer_operation)
        .expect("consumer certifier")
        .certify(one_state_program::<PriorRunFactReadState>(
            consumer_operation.clone(),
        ))
        .expect("consumer certification")
        .into_document();
    let self_scanning_document = registry
        .certifier(&self_scanning_operation)
        .expect("self-scanning certifier")
        .certify(fact_then_read_program(self_scanning_operation.clone()))
        .expect("self-scanning certification")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    producer_calls.store(0, Ordering::SeqCst);
    settlement_calls.store(0, Ordering::SeqCst);
    selected_response.store(0, Ordering::SeqCst);
    let backend = StructuredMemoryBackend::new(store_identity(60));
    let backend_probe = backend.clone();
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(60),
        }),
    );
    let (writer, reader) = store.split();
    let producer_run = run_id(60);
    writer
        .admit_run(admission(
            producer_run.clone(),
            producer_operation.clone(),
            producer_document.clone(),
            7,
            "fact-producer-admit",
        ))
        .await
        .expect("producer admission");
    writer
        .commit_state_transition(
            reader
                .load_verified(&producer_run)
                .await
                .expect("verified producer admission"),
            &StateTransitionProposal::success(
                AppendRequestId::new("fact-producer-transition").expect("producer transition id"),
                ProposedCanonicalValue::from_value(&Value { value: 8 }).expect("producer output"),
                fact_set(Value { value: 7 }, Value { value: 8 }),
            ),
        )
        .await
        .expect("producer fact transition");

    let forged_run = run_id(62);
    writer
        .admit_run(StructuredAdmissionRequest::new(
            forged_run.clone(),
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
                .expect("tenant"),
            InvocationIdentity::new("00000000-0000-4000-8000-000000000062").expect("invocation"),
            consumer_operation.clone(),
            consumer_document.clone(),
            admission_material_with_source(62, source_object.clone()),
            vec![ProposedCanonicalValue::from_value(&Value { value: 7 })
                .expect("forged consumer input")],
            AppendRequestId::new("forged-fact-consumer-admit").expect("forged consumer append id"),
        ))
        .await
        .expect("forged consumer admission");
    let forged_verified = reader
        .load_verified(&forged_run)
        .await
        .expect("verified forged consumer admission");
    let mfm_store::structured::StructuredFrontier::Actions(actions) = forged_verified.frontier()
    else {
        panic!("forged consumer must be actionable");
    };
    let [action] = actions.as_slice() else {
        panic!("forged consumer must have one action");
    };
    let action = action.clone();
    assert!(matches!(
        writer
            .authorize_access(
                forged_verified,
                &AccessAuthorizationProposal::new(
                    AppendRequestId::new("substituted-fact-scanner-binding")
                        .expect("substitution append id"),
                    action.input.clone(),
                    ProposedCanonicalValue::from_value(&request).expect("fact request proposal"),
                    binding_object(99),
                ),
            )
            .await,
        Err(StructuredStoreError::CandidateRejected)
    ));
    let forged_verified = reader
        .load_verified(&forged_run)
        .await
        .expect("unchanged consumer after binding substitution");
    let target = AccessTargetSelection {
        run_id: forged_verified.run_id(),
        occurrence_id: &action.occurrence_id,
        state_input_ref: &action.input,
        store_scope_id: &forged_verified.admission().store_scope_id,
        store_epoch: forged_verified.admission().store_epoch,
        tenant_scope_id: &forged_verified.admission().tenant_scope_id,
        admitted_prior_run_source_manifest_ref: &forged_verified
            .admission()
            .admission_material_refs
            .prior_run_source_manifest_ref,
        admitted_routing_policy_ref: &forged_verified
            .admission()
            .admission_material_refs
            .routing_policy_ref,
        stable_resource_lineage_contract_ref: None,
        minimum_lineage_head_ref: None,
    };
    let substituted_source_request = FactSelectionRequest::new(
        binding_object(98).content_ref,
        request.scan_bounds().expect("fact request bounds"),
        request.queries().expect("fact request queries"),
    )
    .expect("source-substituted fact request");
    let scanner_capability = processes
        .component_identity(
            StructuredComponentKind::Capability,
            action
                .capability_contract_ref
                .as_ref()
                .expect("scanner capability"),
        )
        .expect("qualified scanner capability");
    assert!(processes
        .prepare_access::<ReadPhysicalBindingKind>(
            &scanner_capability,
            target,
            mfm_spec::CanonicalJsonValue::new(
                serde_json::to_value(&substituted_source_request)
                    .expect("substituted request JSON"),
            )
            .expect("canonical substituted request"),
        )
        .await
        .is_err());
    let binding = processes
        .prepare_access::<ReadPhysicalBindingKind>(
            &scanner_capability,
            target,
            mfm_spec::CanonicalJsonValue::new(
                serde_json::to_value(&request).expect("fact request JSON"),
            )
            .expect("canonical fact request"),
        )
        .await
        .expect("scanner binding preparation")
        .expect("scanner binding");
    let binding_certificate = binding.public_certificate().clone();
    let authorization_attempt = writer
        .authorize_access(
            forged_verified,
            &AccessAuthorizationProposal::new(
                AppendRequestId::new("forged-fact-authorization").expect("authorization append id"),
                action.input,
                ProposedCanonicalValue::from_value(&request).expect("fact request proposal"),
                binding_certificate,
            ),
        )
        .await
        .expect("fact authorization");
    let frontier = match &authorization_attempt
        .committed()
        .expect("committed fact authorization")
        .tenant_fact_coordinate
    {
        TenantFactCoordinate::FactSelectionBarrier { frontier } => frontier.clone(),
        _ => panic!("fact authorization must capture a barrier"),
    };
    let (authorization, forged_verified) = authorization_attempt
        .into_newly_appended_authorization()
        .expect("new fact authorization");
    let authorization_ref = authorization.authorization_ref().clone();
    let physical_binding_ref = authorization.authorization().physical_binding_ref.clone();
    drop(authorization);
    let forged_response = PriorRunFactSelectionResponse {
        version: PriorRunFactSelectionResponse::VERSION.to_owned(),
        request_digest: request.request_digest().expect("fact request digest"),
        admitted_source_manifest_ref: source_object.content_ref.clone(),
        selector_contract_ref: mfm_facts::prior_run_fact_selector_contract_ref()
            .expect("selector ref"),
        attestation: PriorRunFactScanAttestation {
            frontier,
            tenant_scope_id: forged_verified.admission().tenant_scope_id.clone(),
            authorization_ref: authorization_ref.clone(),
            physical_binding_ref,
            completeness_mode: PriorRunFactCompletenessMode::CompleteThroughAuthorizationFrontier,
        },
        query_results: Vec::new(),
    };
    let forged_response = FactSelectionReadResponse::from_canonical_json(
        mfm_journal::structured::canonical_json(&forged_response)
            .expect("forged response JSON")
            .as_str()
            .to_owned(),
    )
    .expect("forged typed response");
    writer
        .commit_observation(
            forged_verified,
            &AccessObservationProposal::new(
                AppendRequestId::new("forged-fact-observation").expect("observation append id"),
                authorization_ref,
                ProposedObservationOutcome::Returned(
                    ProposedCanonicalValue::from_value(&forged_response)
                        .expect("forged response proposal"),
                ),
            ),
        )
        .await
        .expect("structurally valid forged response append");
    assert!(matches!(
        reader.load_verified(&forged_run).await,
        Err(StructuredStoreError::InvalidHistory)
    ));

    let unavailable_run = run_id(65);
    writer
        .admit_run(StructuredAdmissionRequest::new(
            unavailable_run.clone(),
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
                .expect("tenant"),
            InvocationIdentity::new("00000000-0000-4000-8000-000000000065")
                .expect("unavailable invocation"),
            consumer_operation.clone(),
            consumer_document.clone(),
            admission_material_with_source(65, source_object.clone()),
            vec![ProposedCanonicalValue::from_value(&Value { value: 0 })
                .expect("unavailable consumer input")],
            AppendRequestId::new("unavailable-fact-consumer-admit")
                .expect("unavailable consumer append id"),
        ))
        .await
        .expect("unavailable consumer admission");
    let unavailable_verified = reader
        .load_verified(&unavailable_run)
        .await
        .expect("verified unavailable consumer admission");
    let mfm_store::structured::StructuredFrontier::Actions(actions) =
        unavailable_verified.frontier()
    else {
        panic!("unavailable consumer must be actionable");
    };
    let [unavailable_action] = actions.as_slice() else {
        panic!("unavailable consumer must have one action");
    };
    let unavailable_action = unavailable_action.clone();
    let unavailable_target = AccessTargetSelection {
        run_id: unavailable_verified.run_id(),
        occurrence_id: &unavailable_action.occurrence_id,
        state_input_ref: &unavailable_action.input,
        store_scope_id: &unavailable_verified.admission().store_scope_id,
        store_epoch: unavailable_verified.admission().store_epoch,
        tenant_scope_id: &unavailable_verified.admission().tenant_scope_id,
        admitted_prior_run_source_manifest_ref: &unavailable_verified
            .admission()
            .admission_material_refs
            .prior_run_source_manifest_ref,
        admitted_routing_policy_ref: &unavailable_verified
            .admission()
            .admission_material_refs
            .routing_policy_ref,
        stable_resource_lineage_contract_ref: None,
        minimum_lineage_head_ref: None,
    };
    let unavailable_scanner_capability = processes
        .component_identity(
            StructuredComponentKind::Capability,
            unavailable_action
                .capability_contract_ref
                .as_ref()
                .expect("unavailable scanner capability"),
        )
        .expect("qualified unavailable scanner capability");
    let unavailable_binding = processes
        .prepare_access::<ReadPhysicalBindingKind>(
            &unavailable_scanner_capability,
            unavailable_target,
            mfm_spec::CanonicalJsonValue::new(
                serde_json::to_value(&request).expect("unavailable request JSON"),
            )
            .expect("canonical unavailable request"),
        )
        .await
        .expect("unavailable scanner binding preparation")
        .expect("unavailable scanner binding");
    let unavailable_authorization = writer
        .authorize_access(
            unavailable_verified,
            &AccessAuthorizationProposal::new(
                AppendRequestId::new("unavailable-fact-authorization")
                    .expect("unavailable authorization append id"),
                unavailable_action.input,
                ProposedCanonicalValue::from_value(&request)
                    .expect("unavailable fact request proposal"),
                unavailable_binding.public_certificate().clone(),
            ),
        )
        .await
        .expect("unavailable fact authorization");
    let (unavailable_authorization, unavailable_verified) = unavailable_authorization
        .into_newly_appended_authorization()
        .expect("new unavailable fact authorization");
    let unavailable_authorization_ref = unavailable_authorization.authorization_ref().clone();
    backend_probe
        .set_unavailable(true)
        .expect("disable fact backend");
    let unavailable_completion = processes
        .invoke_qualified_physical_binding(unavailable_binding, unavailable_authorization)
        .await;
    backend_probe
        .set_unavailable(false)
        .expect("restore fact backend");
    let QualifiedAccessCompletion::SafeFailure(unavailable_failure) = unavailable_completion else {
        panic!("backend outage must produce a typed scanner safe failure");
    };
    let unavailable_failure: FactSelectionReadFailure =
        serde_json::from_value(unavailable_failure.as_json().clone())
            .expect("typed unavailable fact failure");
    assert_eq!(
        unavailable_failure.code(),
        FactSelectionReadFailureCode::StoreUnavailable
    );
    writer
        .commit_observation(
            unavailable_verified,
            &AccessObservationProposal::new(
                AppendRequestId::new("unavailable-fact-observation")
                    .expect("unavailable observation append id"),
                unavailable_authorization_ref,
                ProposedObservationOutcome::SafeFailure(
                    ProposedCanonicalValue::from_value(&unavailable_failure)
                        .expect("unavailable failure proposal"),
                ),
            ),
        )
        .await
        .expect("persist unavailable fact failure");

    let runtime = Runtime::new(writer, processes);

    let consumer_run = run_id(61);
    runtime
        .admit_run(StructuredAdmissionRequest::new(
            consumer_run.clone(),
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
                .expect("tenant"),
            InvocationIdentity::new("00000000-0000-4000-8000-000000000061").expect("invocation"),
            consumer_operation.clone(),
            consumer_document.clone(),
            admission_material_with_source(61, source_object.clone()),
            vec![ProposedCanonicalValue::from_value(&Value { value: 7 }).expect("consumer input")],
            AppendRequestId::new("fact-consumer-admit").expect("consumer append id"),
        ))
        .await
        .expect("consumer admission");
    let fact_drive = runtime.drive_once(&consumer_run).await;
    if let Err(error) = &fact_drive {
        let raw = backend_probe
            .load(&consumer_run)
            .await
            .expect("failed scan history load")
            .expect("failed scan history");
        panic!(
            "authorized fact scan failed after {} batches: {error:?}",
            raw.batches.len()
        );
    }
    assert_eq!(
        fact_drive.expect("authorized fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        runtime
            .drive_once(&consumer_run)
            .await
            .expect("fact settlement"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    assert_eq!(producer_calls.load(Ordering::SeqCst), 0);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 1);
    assert_eq!(selected_response.load(Ordering::SeqCst), 8);

    let raw = backend_probe
        .load(&consumer_run)
        .await
        .expect("consumer raw history")
        .expect("consumer persisted history");
    let TenantFactCoordinate::FactSelectionBarrier { frontier } =
        &raw.batches[1].tenant_fact_coordinate
    else {
        panic!("scanner authorization must capture a fact barrier");
    };
    assert_eq!(frontier.fact_order, 1);
    let authorization_ref = raw.batches[1].records[0].record_ref.clone();
    let verified = reader
        .load_verified(&consumer_run)
        .await
        .expect("actionable fact response recomputation");
    let observation = verified
        .records()
        .iter()
        .find_map(|assigned| match &assigned.record {
            RunRecord::ExternalAccessObserved(observation) => Some(observation),
            _ => None,
        })
        .expect("fact observation");
    let ObservationOutcome::Returned { value } = &observation.outcome else {
        panic!("fact scan must return the typed positive response");
    };
    let returned: FactSelectionReadResponse = verified
        .object(&value.value_ref)
        .expect("returned fact response object")
        .decode()
        .expect("returned fact response");
    let response: PriorRunFactSelectionResponse =
        serde_json::from_str(returned.canonical_response_json()).expect("response attestation");
    assert_eq!(response.attestation.authorization_ref, authorization_ref);
    assert_eq!(response.attestation.frontier.fact_order, 1);
    assert_eq!(response.query_results[0].selected.len(), 1);
    let selected = &response.query_results[0].selected[0];
    assert_eq!(selected.producer_transition_ref.run_id, producer_run);
    let verified_producer = reader
        .load_verified(&selected.producer_transition_ref.run_id)
        .await
        .expect("selected fact producer");
    assert_eq!(
        selected.subject_canonical_json.as_str(),
        verified_producer
            .object(&selected.subject.value_ref)
            .expect("selected subject source")
            .canonical_json
            .as_str()
    );
    assert_eq!(
        selected.response_canonical_json.as_str(),
        verified_producer
            .object(&selected.response.value_ref)
            .expect("selected response source")
            .canonical_json
            .as_str()
    );
    assert_eq!(
        selected.claim_canonical_json.as_str(),
        verified_producer
            .object(&selected.claim_ref)
            .expect("selected claim source")
            .canonical_json
            .as_str()
    );

    let second_producer_run = run_id(63);
    runtime
        .admit_run(admission(
            second_producer_run.clone(),
            producer_operation,
            producer_document,
            7,
            "second-fact-producer-admit",
        ))
        .await
        .expect("second producer admission");
    assert_eq!(
        runtime
            .drive_once(&second_producer_run)
            .await
            .expect("second producer transition"),
        DriveOutcome::TransitionCommitted { closed: true }
    );

    for (discriminator, input, expected_failure) in [
        (
            70,
            1,
            FactSelectionReadFailureCode::PublicationBoundExceeded,
        ),
        (71, 2, FactSelectionReadFailureCode::FactBoundExceeded),
        (
            72,
            3,
            FactSelectionReadFailureCode::RetainedSourceBoundExceeded,
        ),
        (
            73,
            4,
            FactSelectionReadFailureCode::SelectedResultBoundExceeded,
        ),
        (74, 5, FactSelectionReadFailureCode::ResponseBoundExceeded),
    ] {
        let bounded_consumer_run = run_id(discriminator);
        runtime
            .admit_run(StructuredAdmissionRequest::new(
                bounded_consumer_run.clone(),
                TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
                    .expect("tenant"),
                InvocationIdentity::new(format!("00000000-0000-4000-8000-{discriminator:012}"))
                    .expect("bounded invocation"),
                consumer_operation.clone(),
                consumer_document.clone(),
                admission_material_with_source(discriminator, source_object.clone()),
                vec![ProposedCanonicalValue::from_value(&Value { value: input })
                    .expect("bounded consumer input")],
                AppendRequestId::new(format!("bounded-fact-consumer-{discriminator}-admit"))
                    .expect("bounded consumer append id"),
            ))
            .await
            .expect("bounded consumer admission");
        assert_eq!(
            runtime
                .drive_once(&bounded_consumer_run)
                .await
                .expect("bounded fact scan"),
            DriveOutcome::AccessObserved
        );
        assert_eq!(
            runtime
                .drive_once(&bounded_consumer_run)
                .await
                .expect("bounded fact settlement"),
            DriveOutcome::TransitionCommitted { closed: true }
        );

        let bounded_verified = reader
            .load_verified(&bounded_consumer_run)
            .await
            .expect("verified bounded consumer");
        let bounded_failure = bounded_verified
            .records()
            .iter()
            .find_map(|assigned| match &assigned.record {
                RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                    ObservationOutcome::SafeFailure { value } => Some(value),
                    _ => None,
                },
                _ => None,
            })
            .expect("bounded scan safe failure");
        let bounded_failure: FactSelectionReadFailure = bounded_verified
            .object(&bounded_failure.value_ref)
            .expect("bounded failure object")
            .decode()
            .expect("bounded typed failure");
        assert_eq!(bounded_failure.code(), expected_failure);
    }
    assert_eq!(producer_calls.load(Ordering::SeqCst), 1);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 6);
    assert_eq!(selected_response.load(Ordering::SeqCst), 0);

    let self_scanning_run = run_id(66);
    runtime
        .admit_run(StructuredAdmissionRequest::new(
            self_scanning_run.clone(),
            TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32)))
                .expect("tenant"),
            InvocationIdentity::new("00000000-0000-4000-8000-000000000066")
                .expect("self-scanning invocation"),
            self_scanning_operation,
            self_scanning_document,
            admission_material_with_source(66, source_object),
            vec![ProposedCanonicalValue::from_value(&Value { value: 7 })
                .expect("self-scanning input")],
            AppendRequestId::new("self-scanning-fact-admit").expect("self-scanning append id"),
        ))
        .await
        .expect("self-scanning admission");
    assert_eq!(
        runtime
            .drive_once(&self_scanning_run)
            .await
            .expect("self fact publication"),
        DriveOutcome::TransitionCommitted { closed: false }
    );
    assert_eq!(
        runtime
            .drive_once(&self_scanning_run)
            .await
            .expect("self-excluding fact scan"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        runtime
            .drive_once(&self_scanning_run)
            .await
            .expect("self-scanning settlement"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    assert_eq!(producer_calls.load(Ordering::SeqCst), 2);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 7);
    assert_eq!(selected_response.load(Ordering::SeqCst), 8);

    let self_raw = backend_probe
        .load(&self_scanning_run)
        .await
        .expect("self-scanning raw history")
        .expect("self-scanning persisted history");
    let TenantFactCoordinate::FactPublication {
        frontier: self_publication,
    } = &self_raw.batches[1].tenant_fact_coordinate
    else {
        panic!("self-scanning producer must publish its fact");
    };
    let TenantFactCoordinate::FactSelectionBarrier {
        frontier: self_barrier,
    } = &self_raw.batches[2].tenant_fact_coordinate
    else {
        panic!("self-scanning authorization must capture its own publication frontier");
    };
    assert_eq!(self_publication.fact_order, 3);
    assert_eq!(self_barrier.fact_order, 3);
    let self_verified = reader
        .load_verified(&self_scanning_run)
        .await
        .expect("verified self-scanning response");
    let self_response = self_verified
        .records()
        .iter()
        .find_map(|assigned| match &assigned.record {
            RunRecord::ExternalAccessObserved(observation) => match &observation.outcome {
                ObservationOutcome::Returned { value } => Some(value),
                _ => None,
            },
            _ => None,
        })
        .expect("self-scanning returned observation");
    let self_response: FactSelectionReadResponse = self_verified
        .object(&self_response.value_ref)
        .expect("self-scanning response object")
        .decode()
        .expect("self-scanning typed response");
    let self_response: PriorRunFactSelectionResponse =
        serde_json::from_str(self_response.canonical_response_json())
            .expect("self-scanning response attestation");
    let self_selected_runs = self_response.query_results[0]
        .selected
        .iter()
        .map(|selected| selected.producer_transition_ref.run_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        self_selected_runs,
        std::collections::BTreeSet::from([producer_run, second_producer_run])
    );
    assert!(!self_selected_runs.contains(&self_scanning_run));
}

#[tokio::test]
async fn ordinary_failure_closes_without_blocking_an_unrelated_run() {
    let state_calls = Arc::new(AtomicUsize::new(0));
    let mapper_calls = Arc::new(AtomicUsize::new(0));
    let operation_id = stable("mfm.runtime.fixture/fallible-operation").expect("operation id");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    assembly
        .register_value::<FailureValue>()
        .expect("failure value contract");
    assembly
        .register_closed_sum::<FailureRoute>()
        .expect("failure route contract");
    let calls = Arc::clone(&state_calls);
    let state_descriptor =
        implementation_descriptor::<ConditionalFailureState>(&mut assembly, "conditional-failure");
    assembly
        .register_state::<ConditionalFailureState>(
            state_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, Value>| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let input = frame.input().value;
                    if input == 0 {
                        ProposedStateOutcome::Failure(FailureValue { code: 91 })
                    } else {
                        ProposedStateOutcome::Success(Value { value: input + 1 })
                    }
                }),
            },
        )
        .expect("conditional state");
    let mapper_count = Arc::clone(&mapper_calls);
    let mapper_descriptor =
        implementation_descriptor::<FailureMapperState>(&mut assembly, "failure-mapper");
    assembly
        .register_state::<FailureMapperState>(
            mapper_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, FailureValue>| {
                    mapper_count.fetch_add(1, Ordering::SeqCst);
                    ProposedStateOutcome::Success(FailureRoute::Propagate {
                        failure: frame.input().clone(),
                    })
                }),
            },
        )
        .expect("failure mapper state");
    assembly
        .register_entry_point(
            operation_id.clone(),
            fallible_program(operation_id.clone()),
            profile(),
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(fallible_program(operation_id.clone()))
        .expect("certified program")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let certificate = binding_object(50);
    let store = StructuredRunStore::new(
        StructuredMemoryBackend::new(store_identity(50)),
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let failed_run_id = run_id(50);
    let successful_run_id = run_id(51);
    runtime
        .admit_run(admission(
            failed_run_id.clone(),
            operation_id.clone(),
            document.clone(),
            0,
            "failed-run-admit",
        ))
        .await
        .expect("failed run admission");
    runtime
        .admit_run(admission(
            successful_run_id.clone(),
            operation_id,
            document,
            7,
            "successful-run-admit",
        ))
        .await
        .expect("successful run admission");

    assert_eq!(
        runtime
            .drive_once(&failed_run_id)
            .await
            .expect("failing transition"),
        DriveOutcome::TransitionCommitted { closed: false }
    );
    assert_eq!(
        runtime
            .drive_once(&failed_run_id)
            .await
            .expect("failure propagation"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    let failed = reader
        .load_verified(&failed_run_id)
        .await
        .expect("failed run");
    let mfm_store::structured::ProgramCursor::Closed { outcome_ref } = failed.cursor() else {
        panic!("ordinary failure must close the run");
    };
    let failed_outcome: serde_json::Value = failed
        .object(outcome_ref)
        .expect("failed outcome object")
        .decode()
        .expect("failed outcome");
    assert!(failed_outcome.get("Failure").is_some());

    assert_eq!(
        runtime
            .drive_once(&successful_run_id)
            .await
            .expect("independent successful transition"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    let succeeded = reader
        .load_verified(&successful_run_id)
        .await
        .expect("successful run");
    let mfm_store::structured::ProgramCursor::Closed { outcome_ref } = succeeded.cursor() else {
        panic!("successful run must close");
    };
    let successful_outcome: serde_json::Value = succeeded
        .object(outcome_ref)
        .expect("successful outcome object")
        .decode()
        .expect("successful outcome");
    assert!(successful_outcome.get("Success").is_some());
    assert_eq!(state_calls.load(Ordering::SeqCst), 2);
    assert_eq!(mapper_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn safe_failure_closes_through_default_mapping_without_blocking_an_unrelated_run() {
    let request_calls = Arc::new(AtomicUsize::new(0));
    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let mapper_calls = Arc::new(AtomicUsize::new(0));
    let operation_id = stable("mfm.runtime.fixture/fallible-read-operation").expect("operation id");
    let certificate = binding_object(52);
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    assembly
        .register_value::<FailureValue>()
        .expect("failure value contract");
    assembly
        .register_closed_sum::<FailureRoute>()
        .expect("failure route contract");

    let request_count = Arc::clone(&request_calls);
    let settlement_count = Arc::clone(&settlement_calls);
    let state_descriptor =
        implementation_descriptor::<FallibleReadState>(&mut assembly, "fallible-read-state");
    assembly
        .register_state::<FallibleReadState>(
            state_descriptor,
            StructuredStateCallbacks::Read {
                request: Arc::new(move |frame: StateFrame<'_, Value>| {
                    request_count.fetch_add(1, Ordering::SeqCst);
                    frame.input().clone()
                }),
                settle: Arc::new(
                    move |_frame, observation: CommittedObservationView<'_, Value, Value>| {
                        settlement_count.fetch_add(1, Ordering::SeqCst);
                        let outcome = match observation.observation() {
                            CommittedObservation::Returned(value) => {
                                ProposedStateOutcome::Success(value.clone())
                            }
                            CommittedObservation::SafeFailure(failure) => {
                                ProposedStateOutcome::Failure(FailureValue {
                                    code: failure.value,
                                })
                            }
                        };
                        StateSettlement::Proposed(outcome)
                    },
                ),
                reviewed_safe_failures: vec![ReviewedSafeFailureCase::new(
                    Value { value: 0 },
                    Value { value: 91 },
                    ProposedStateOutcome::Failure(FailureValue { code: 91 }),
                )],
            },
        )
        .expect("fallible Read state");

    let mapper_count = Arc::clone(&mapper_calls);
    let mapper_descriptor =
        implementation_descriptor::<FailureMapperState>(&mut assembly, "read-failure-mapper");
    assembly
        .register_state::<FailureMapperState>(
            mapper_descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, FailureValue>| {
                    mapper_count.fetch_add(1, Ordering::SeqCst);
                    ProposedStateOutcome::Success(FailureRoute::Propagate {
                        failure: frame.input().clone(),
                    })
                }),
            },
        )
        .expect("failure mapper state");

    let capability_descriptor = implementation_descriptor_for_contract(
        &mut assembly,
        StructuredComponentKind::Capability,
        read_capability_contract()
            .expect("capability contract")
            .content_ref()
            .expect("capability ref"),
        "fallible-read-capability",
    );
    assembly
        .register_read_capability::<FixtureReadCapability, _>(
            capability_descriptor,
            Arc::new(FixtureReadCapabilityImplementation),
        )
        .expect("Read capability");
    let adapter_descriptor = implementation_descriptor_for_contract(
        &mut assembly,
        StructuredComponentKind::Adapter,
        read_adapter_contract()
            .expect("adapter contract")
            .content_ref()
            .expect("adapter ref"),
        "fallible-read-adapter",
    );
    assembly
        .register_read_adapter::<FixtureReadCapability, _>(
            adapter_descriptor,
            Arc::new(ConditionalReadBindingSource {
                binding: Arc::new(ConditionalFailureReadAdapter {
                    calls: Arc::clone(&adapter_calls),
                    certificate: certificate.clone(),
                }),
            }),
        )
        .expect("Read adapter");
    assembly
        .register_entry_point(
            operation_id.clone(),
            fallible_read_program(operation_id.clone()),
            profile(),
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(fallible_read_program(operation_id.clone()))
        .expect("certified program")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let store = StructuredRunStore::new(
        StructuredMemoryBackend::new(store_identity(52)),
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let failed_run_id = run_id(52);
    let successful_run_id = run_id(53);
    runtime
        .admit_run(admission(
            failed_run_id.clone(),
            operation_id.clone(),
            document.clone(),
            0,
            "safe-failure-run-admit",
        ))
        .await
        .expect("safe-failure run admission");
    runtime
        .admit_run(admission(
            successful_run_id.clone(),
            operation_id,
            document,
            7,
            "safe-failure-success-run-admit",
        ))
        .await
        .expect("successful run admission");
    request_calls.store(0, Ordering::SeqCst);
    adapter_calls.store(0, Ordering::SeqCst);
    settlement_calls.store(0, Ordering::SeqCst);
    mapper_calls.store(0, Ordering::SeqCst);

    assert_eq!(
        runtime
            .drive_once(&failed_run_id)
            .await
            .expect("safe-failure observation"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        runtime
            .drive_once(&failed_run_id)
            .await
            .expect("safe-failure settlement"),
        DriveOutcome::TransitionCommitted { closed: false }
    );
    assert_eq!(
        runtime
            .drive_once(&failed_run_id)
            .await
            .expect("default failure mapping"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    assert_eq!(
        runtime
            .drive_once(&successful_run_id)
            .await
            .expect("successful observation"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        runtime
            .drive_once(&successful_run_id)
            .await
            .expect("successful settlement"),
        DriveOutcome::TransitionCommitted { closed: true }
    );

    let failed = reader
        .load_verified(&failed_run_id)
        .await
        .expect("failed run");
    let mfm_store::structured::ProgramCursor::Closed { outcome_ref } = failed.cursor() else {
        panic!("safe failure must close the run");
    };
    let failed_outcome: OperationOutcome<LexicalValueRef, LexicalValueRef> = failed
        .object(outcome_ref)
        .expect("failed outcome object")
        .decode()
        .expect("typed failed outcome");
    let OperationOutcome::Failure(failure_ref) = failed_outcome else {
        panic!("safe failure must retain the root Failure variant");
    };
    assert_eq!(
        failed
            .object(&failure_ref.value.value_ref)
            .expect("root failure value")
            .decode::<FailureValue>()
            .expect("typed root failure value"),
        FailureValue { code: 91 }
    );
    let succeeded = reader
        .load_verified(&successful_run_id)
        .await
        .expect("successful run");
    let mfm_store::structured::ProgramCursor::Closed { outcome_ref } = succeeded.cursor() else {
        panic!("successful run must close");
    };
    let successful_outcome: OperationOutcome<LexicalValueRef, LexicalValueRef> = succeeded
        .object(outcome_ref)
        .expect("successful outcome object")
        .decode()
        .expect("typed successful outcome");
    let OperationOutcome::Success(success_ref) = successful_outcome else {
        panic!("successful run must retain the root Success variant");
    };
    assert_eq!(
        succeeded
            .object(&success_ref.value.value_ref)
            .expect("root success value")
            .decode::<Value>()
            .expect("typed root success value"),
        Value { value: 8 }
    );
    assert_eq!(request_calls.load(Ordering::SeqCst), 2);
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 2);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 2);
    assert_eq!(mapper_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn access_invokes_once_and_persists_before_success_under_observation_retries() {
    for injection in [
        InjectAppend::None,
        InjectAppend::ObservationStaleHead,
        InjectAppend::ObservationAcknowledgementUnknown,
        InjectAppend::ObservationLoadUnavailable,
        InjectAppend::ObservationAppendUnavailable,
        InjectAppend::ObservationResolveUnavailable,
        InjectAppend::ObservationReloadUnavailable,
        InjectAppend::ObservationConcurrentSame,
    ] {
        let fixture = read_fixture(injection as u8 + 10, injection, false).await;
        assert_eq!(
            fixture
                .runtime
                .drive_once(&fixture.run_id)
                .await
                .expect("access drive"),
            DriveOutcome::AccessObserved
        );
        assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.binding_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.adapter_input.load(Ordering::SeqCst), 7);
        assert_eq!(fixture.settlement_calls.load(Ordering::SeqCst), 0);
        if injection == InjectAppend::None {
            assert_eq!(fixture.history_loads.load(Ordering::SeqCst), 1);
        }

        let verified = fixture
            .reader
            .load_verified(&fixture.run_id)
            .await
            .expect("observation must already be durable");
        assert!(matches!(
            verified.frontier(),
            mfm_store::structured::StructuredFrontier::Actions(_)
        ));
        let settlement_loads_before = fixture.history_loads.load(Ordering::SeqCst);
        assert_eq!(
            fixture
                .runtime
                .drive_once(&fixture.run_id)
                .await
                .expect("settlement drive"),
            DriveOutcome::TransitionCommitted { closed: true }
        );
        if injection == InjectAppend::None {
            assert_eq!(
                fixture.history_loads.load(Ordering::SeqCst),
                settlement_loads_before + 1
            );
        }
        assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.settlement_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.settlement_input.load(Ordering::SeqCst), 8);
    }
}

#[tokio::test]
async fn conflicting_concurrent_observation_fails_closed_without_reinvocation() {
    let fixture = read_fixture(19, InjectAppend::ObservationConcurrentDifferent, false).await;

    let fault = fixture
        .runtime
        .drive_once(&fixture.run_id)
        .await
        .expect_err("conflicting observation must reject the pending candidate");
    assert_eq!(fault.code(), RuntimeFaultCode::CandidateRejected);
    assert_eq!(fault.phase(), RuntimeFaultPhase::AppendCandidate);
    assert_eq!(fault.store_fault_kind(), None);
    assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.settlement_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        fixture
            .reader
            .load_verified(&fixture.run_id)
            .await
            .expect("conflicting observation remains auditable")
            .frontier(),
        mfm_store::structured::StructuredFrontier::BlockedIntegrity
    ));
    assert_eq!(
        fixture
            .runtime
            .drive_once(&fixture.run_id)
            .await
            .expect("conflict parks the run"),
        DriveOutcome::BlockedIntegrity
    );
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn frozen_effect_supersession_is_not_rewritten_after_persistence_integrity_failure() {
    for (discriminator, injection, expected_code, expected_store_kind, expected_frontier) in [
        (
            52,
            InjectAppend::ObservationConcurrentDifferent,
            RuntimeFaultCode::CandidateRejected,
            None,
            "blocked",
        ),
        (
            53,
            InjectAppend::ObservationSubstitutedPositive,
            RuntimeFaultCode::StoreInvalid,
            Some(RuntimeStoreFaultKind::InvalidHistory),
            "possible-entry",
        ),
    ] {
        let adapter_calls = Arc::new(AtomicUsize::new(0));
        let settlement_calls = Arc::new(AtomicUsize::new(0));
        let binding_calls = Arc::new(AtomicUsize::new(0));
        let first_certificate = binding_object(discriminator);
        let second_certificate = binding_object(discriminator + 1);
        let lineage_head = lineage_head_object(discriminator + 2);
        let binding_source = refresh_effect_binding_source(
            first_certificate.clone(),
            second_certificate.clone(),
            lineage_head.clone(),
            &adapter_calls,
            &binding_calls,
        );
        let (operation_id, resource_ref, registry) =
            refreshable_effect_registry(&settlement_calls, Arc::clone(&binding_source));
        let document = registry
            .certifier(&operation_id)
            .expect("certifier")
            .certify(one_state_program::<EffectState>(operation_id.clone()))
            .expect("certified")
            .into_document();
        let (program_verifier, processes) = split_qualified_registry(registry);
        adapter_calls.store(0, Ordering::SeqCst);
        settlement_calls.store(0, Ordering::SeqCst);
        let store = StructuredRunStore::new(
            InjectingBackend::new(store_identity(discriminator), injection),
            program_verifier,
            Arc::new(RefreshBindingVerifier {
                lineage_ref: resource_ref.clone(),
                first_certificate: first_certificate.clone(),
                second_certificate,
                lineage_head,
                accept_supersession: true,
            }),
        );
        let (writer, reader) = store.split();
        let runtime = Runtime::new(writer, processes);
        let run_id = run_id(discriminator);
        runtime
            .admit_run(effect_admission(
                run_id.clone(),
                operation_id,
                document,
                resource_ref,
            ))
            .await
            .expect("effect admission");

        let fault = runtime
            .drive_once(&run_id)
            .await
            .expect_err("substituted observation candidate must fail");
        assert_eq!(fault.code(), expected_code);
        assert_eq!(fault.store_fault_kind(), expected_store_kind);
        assert_eq!(adapter_calls.load(Ordering::SeqCst), 1);
        assert_eq!(binding_source.first.target_calls.load(Ordering::SeqCst), 1);
        assert_eq!(binding_source.second.target_calls.load(Ordering::SeqCst), 0);
        assert_eq!(binding_calls.load(Ordering::SeqCst), 1);
        assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);
        let verified = reader
            .load_verified(&run_id)
            .await
            .expect("integrity failure remains auditable");
        match expected_frontier {
            "blocked" => assert!(matches!(
                verified.frontier(),
                mfm_store::structured::StructuredFrontier::BlockedIntegrity
            )),
            "possible-entry" => assert!(matches!(
                verified.frontier(),
                mfm_store::structured::StructuredFrontier::PossibleEntry
            )),
            _ => panic!("unknown expected frontier"),
        }
    }
}

#[tokio::test]
async fn invalid_supersession_evidence_is_rejected_without_a_diagnostic_observation() {
    let discriminator = 56;
    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let binding_calls = Arc::new(AtomicUsize::new(0));
    let first_certificate = binding_object(discriminator);
    let second_certificate = binding_object(discriminator + 1);
    let lineage_head = lineage_head_object(discriminator + 2);
    let binding_source = refresh_effect_binding_source(
        first_certificate.clone(),
        second_certificate.clone(),
        lineage_head.clone(),
        &adapter_calls,
        &binding_calls,
    );
    let (operation_id, resource_ref, registry) =
        refreshable_effect_registry(&settlement_calls, Arc::clone(&binding_source));
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<EffectState>(operation_id.clone()))
        .expect("certified")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    adapter_calls.store(0, Ordering::SeqCst);
    settlement_calls.store(0, Ordering::SeqCst);
    let store = StructuredRunStore::new(
        InjectingBackend::new(store_identity(discriminator), InjectAppend::None),
        program_verifier,
        Arc::new(RefreshBindingVerifier {
            lineage_ref: resource_ref.clone(),
            first_certificate: first_certificate.clone(),
            second_certificate,
            lineage_head,
            accept_supersession: false,
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(discriminator);
    runtime
        .admit_run(effect_admission(
            run_id.clone(),
            operation_id,
            document,
            resource_ref,
        ))
        .await
        .expect("effect admission");

    let fault = runtime
        .drive_once(&run_id)
        .await
        .expect_err("invalid supersession evidence must reject without append");
    assert_eq!(fault.code(), RuntimeFaultCode::CandidateRejected);
    assert_eq!(fault.phase(), RuntimeFaultPhase::QualifyCandidate);
    assert_eq!(fault.store_fault_kind(), None);
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_source.first.target_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_source.second.target_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        reader
            .load_verified(&run_id)
            .await
            .expect("authorization-only prefix")
            .frontier(),
        mfm_store::structured::StructuredFrontier::PossibleEntry
    ));
}

#[tokio::test]
async fn recovered_unmatched_read_waits_without_reauthorizing_or_reinvoking() {
    let fixture = read_fixture(20, InjectAppend::None, true).await;

    assert_eq!(
        fixture
            .runtime
            .drive_once(&fixture.run_id)
            .await
            .expect("recovered drive"),
        DriveOutcome::WaitingReads
    );
    assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.binding_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.settlement_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn crash_after_physical_invocation_parks_the_authorization_without_reinvocation() {
    let fixture = read_fixture(62, InjectAppend::ObservationHoldAfterInvocation, false).await;
    let ReadFixture {
        runtime,
        reader,
        run_id,
        request_calls,
        binding_calls,
        adapter_calls,
        settlement_calls,
        observation_append_entered,
        ..
    } = fixture;
    let runtime = Arc::new(runtime);
    let task_runtime = Arc::clone(&runtime);
    let task_run = run_id.clone();
    let drive = tokio::spawn(async move { task_runtime.drive_once(&task_run).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        observation_append_entered.notified(),
    )
    .await
    .expect("observation append begins only after the adapter returned");
    assert_eq!(request_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 1);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        reader
            .load_verified(&run_id)
            .await
            .expect("authorization-only crash prefix")
            .frontier(),
        mfm_store::structured::StructuredFrontier::WaitingReads
    ));

    drive.abort();
    assert!(drive
        .await
        .expect_err("simulated process crash")
        .is_cancelled());
    assert_eq!(
        runtime
            .drive_once(&run_id)
            .await
            .expect("fresh drive of an unmatched authorization"),
        DriveOutcome::WaitingReads
    );
    assert_eq!(request_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 1);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ambiguous_authorization_acknowledgement_never_mints_invocation_authority() {
    let fixture = read_fixture(21, InjectAppend::AuthorizationAcknowledgementUnknown, false).await;

    assert_eq!(
        fixture
            .runtime
            .drive_once(&fixture.run_id)
            .await
            .expect("ambiguous authorization drive"),
        DriveOutcome::ConcurrentProgress
    );
    assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.settlement_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        fixture
            .reader
            .load_verified(&fixture.run_id)
            .await
            .expect("parked ambiguous authorization")
            .frontier(),
        mfm_store::structured::StructuredFrontier::WaitingReads
    ));
    assert_eq!(
        fixture
            .runtime
            .drive_once(&fixture.run_id)
            .await
            .expect("recovered ambiguous authorization"),
        DriveOutcome::WaitingReads
    );
    assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unavailable_target_binding_fails_before_authorization_or_invocation() {
    let fixture =
        read_fixture_with_binding_options(22, InjectAppend::None, false, false, true).await;

    let fault = fixture
        .runtime
        .drive_once(&fixture.run_id)
        .await
        .expect_err("unavailable binding must stop before authorization");
    assert_eq!(fault.code(), RuntimeFaultCode::PhysicalBindingUnavailable);
    assert_eq!(fault.phase(), RuntimeFaultPhase::QualifyAccess);
    assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.settlement_calls.load(Ordering::SeqCst), 0);
    let verified = fixture
        .reader
        .load_verified(&fixture.run_id)
        .await
        .expect("unmodified ready run");
    let mfm_store::structured::StructuredFrontier::Actions(actions) = verified.frontier() else {
        panic!("unavailable binding must leave the state ready");
    };
    assert!(matches!(
        actions.as_slice(),
        [mfm_store::structured::ActionableState {
            leaf: mfm_store::structured::StateLeaf::Ready,
            ..
        }]
    ));
}

#[tokio::test]
async fn unqualified_public_certificate_never_authorizes_its_private_handle() {
    let fixture =
        read_fixture_with_binding_options(23, InjectAppend::None, false, true, false).await;

    let fault = fixture
        .runtime
        .drive_once(&fixture.run_id)
        .await
        .expect_err("foreign certificate must not authorize its retained handle");
    assert_eq!(fault.code(), RuntimeFaultCode::CandidateRejected);
    assert_eq!(fault.phase(), RuntimeFaultPhase::AppendCandidate);
    assert_eq!(fault.store_fault_kind(), None);
    assert_eq!(fixture.request_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 0);
    let verified = fixture
        .reader
        .load_verified(&fixture.run_id)
        .await
        .expect("rejected certificate leaves the run unchanged");
    assert!(matches!(
        verified.frontier(),
        mfm_store::structured::StructuredFrontier::Actions(actions)
            if matches!(
                actions.as_slice(),
                [mfm_store::structured::ActionableState {
                    leaf: mfm_store::structured::StateLeaf::Ready,
                    ..
                }]
            )
    ));
}

#[tokio::test]
async fn exact_current_input_ref_reaches_binding_selection_and_rejects_substitution() {
    let fixture = read_fixture_with_input_substitution_probe(24).await;
    let verified = fixture
        .reader
        .load_verified(&fixture.run_id)
        .await
        .expect("ready run");
    let mfm_store::structured::StructuredFrontier::Actions(actions) = verified.frontier() else {
        panic!("ready run must expose its current action");
    };
    let [action] = actions.as_slice() else {
        panic!("one-state program must expose one action");
    };
    let expected_input = action.input.clone();

    assert_eq!(
        fixture
            .runtime
            .drive_once(&fixture.run_id)
            .await
            .expect("drive"),
        DriveOutcome::AccessObserved
    );
    assert_eq!(
        fixture
            .selected_state_input
            .lock()
            .expect("selected input lock")
            .as_ref(),
        Some(&expected_input)
    );
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn invalid_settlement_fault_repeats_at_the_committed_observation_without_append() {
    let fixture = read_invalid_settlement_fixture(61).await;
    assert_eq!(
        fixture
            .runtime
            .drive_once(&fixture.run_id)
            .await
            .expect("Read observation"),
        DriveOutcome::AccessObserved
    );
    let observed = fixture
        .reader
        .load_verified(&fixture.run_id)
        .await
        .expect("observed Read");
    let observed_head = observed.journal_head().clone();
    let mfm_store::structured::StructuredFrontier::Actions(actions) = observed.frontier() else {
        panic!("observed Read must await settlement")
    };
    let [action] = actions.as_slice() else {
        panic!("one observed Read action")
    };
    let expected_occurrence = action.occurrence_id.clone();
    let raw_before = fixture
        .backend_probe
        .load(&fixture.run_id)
        .await
        .expect("raw observed history")
        .expect("observed history");

    let first = fixture
        .runtime
        .drive_once(&fixture.run_id)
        .await
        .expect_err("invalid committed evidence settlement");
    let second = fixture
        .runtime
        .drive_once(&fixture.run_id)
        .await
        .expect_err("same evidence must repeat the same fault");
    assert_eq!(first, second);
    assert_eq!(first.code(), RuntimeFaultCode::InvalidEvidence);
    assert_eq!(first.phase(), RuntimeFaultPhase::SettleObservation);
    assert_eq!(first.pre_fault_head(), Some(&observed_head));
    assert_eq!(first.occurrence_id(), Some(&expected_occurrence));
    let mfm_runtime::structured::RuntimeFaultSubject::Process(component) = first.subject() else {
        panic!("settlement fault must identify the qualified state")
    };
    assert_eq!(component.component_kind(), StructuredComponentKind::State);
    assert_eq!(fixture.adapter_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.settlement_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture
            .backend_probe
            .load(&fixture.run_id)
            .await
            .expect("raw repeated-fault history")
            .expect("observed history"),
        raw_before
    );
}

fn refreshable_effect_registry(
    settlement_calls: &Arc<AtomicUsize>,
    binding_source: Arc<RefreshEffectBindingSource>,
) -> (StableId, mfm_ids::ContentRef, QualifiedProgramRegistry) {
    let operation_id =
        stable("mfm.runtime.fixture/refreshable-effect-operation").expect("operation id");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");

    let settle_counter = Arc::clone(settlement_calls);
    let state_descriptor = implementation_descriptor::<EffectState>(&mut assembly, "effect-state");
    assembly
        .register_state::<EffectState>(
            state_descriptor,
            StructuredStateCallbacks::Effect {
                request: Arc::new(|frame: StateFrame<'_, Value>| frame.input().clone()),
                settle: Arc::new(
                    move |_frame, observation: CommittedObservationView<'_, Value, Value>| {
                        settle_counter.fetch_add(1, Ordering::SeqCst);
                        let value = match observation.observation() {
                            CommittedObservation::Returned(value)
                            | CommittedObservation::SafeFailure(value) => value.clone(),
                        };
                        StateSettlement::Proposed(ProposedStateOutcome::Success(value))
                    },
                ),
                reviewed_safe_failures: vec![ReviewedSafeFailureCase::new(
                    Value { value: 1 },
                    Value { value: 2 },
                    ProposedStateOutcome::Success(Value { value: 2 }),
                )],
            },
        )
        .expect("effect state");

    let resource_ref = resource_contract()
        .expect("resource contract")
        .content_ref()
        .expect("resource ref");
    let resource_descriptor = implementation_descriptor_for_contract(
        &mut assembly,
        StructuredComponentKind::Resource,
        resource_ref.clone(),
        "effect-resource",
    );
    assembly
        .register_resource_authority::<FixtureResource, _>(
            resource_descriptor,
            Arc::new(FixtureResourceInvoker),
        )
        .expect("resource authority");

    let adapter_ref = effect_adapter_contract()
        .expect("effect adapter contract")
        .content_ref()
        .expect("effect adapter ref");
    let adapter_descriptor = implementation_descriptor_for_contract(
        &mut assembly,
        StructuredComponentKind::Adapter,
        adapter_ref,
        "effect-adapter",
    );
    assembly
        .register_effect_adapter::<FixtureEffectCapability, _>(adapter_descriptor, binding_source)
        .expect("effect adapter");

    let capability_ref = effect_capability_contract()
        .expect("effect capability contract")
        .content_ref()
        .expect("effect capability ref");
    let capability_descriptor = implementation_descriptor_for_contract(
        &mut assembly,
        StructuredComponentKind::Capability,
        capability_ref,
        "effect-capability",
    );
    assembly
        .register_effect_capability::<FixtureEffectCapability, _>(
            capability_descriptor,
            Arc::new(FixtureEffectCapabilityImplementation),
        )
        .expect("effect capability");
    assembly
        .register_entry_point(
            operation_id.clone(),
            one_state_program::<EffectState>(operation_id.clone()),
            profile(),
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    (operation_id, resource_ref, registry)
}

#[tokio::test]
async fn admission_requires_the_exact_certified_resource_lineage_set() {
    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let binding_calls = Arc::new(AtomicUsize::new(0));
    let binding_source = refresh_effect_binding_source(
        binding_object(40),
        binding_object(41),
        lineage_head_object(42),
        &adapter_calls,
        &binding_calls,
    );
    let (operation_id, resource_ref, registry) =
        refreshable_effect_registry(&settlement_calls, Arc::clone(&binding_source));
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<EffectState>(operation_id.clone()))
        .expect("certified")
        .into_document();
    let capability_ref = effect_capability_contract()
        .expect("effect capability")
        .content_ref()
        .expect("effect capability ref");
    let unused_resource_ref = StructuredLiveComponentContract::new(
        StructuredComponentKind::Resource,
        stable("mfm.runtime.fixture/unused-resource").expect("unused resource id"),
        Vec::new(),
    )
    .expect("unused resource contract")
    .content_ref()
    .expect("unused resource ref");
    let unrelated_ref = binding_object(45).content_ref;

    assert!(matches!(
        effect_admission_material(vec![resource_ref.clone(), resource_ref.clone()]),
        Err(StructuredStoreError::InvalidHistory)
    ));

    let (program_verifier, _processes) = split_qualified_registry(registry);
    let store = StructuredRunStore::new(
        StructuredMemoryBackend::new(store_identity(40)),
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(40),
        }),
    );
    let (writer, reader) = store.split();
    let cases = [
        (41, Vec::new(), "missing-lineage"),
        (
            42,
            vec![resource_ref.clone(), unrelated_ref],
            "extra-lineage",
        ),
        (
            43,
            vec![resource_ref.clone(), unused_resource_ref],
            "unused-lineage",
        ),
        (44, vec![capability_ref], "non-resource-lineage"),
    ];
    for (discriminator, lineage_refs, append_id) in cases {
        let rejected_run_id = run_id(discriminator);
        let result = writer
            .admit_run(effect_admission_with_lineages(
                rejected_run_id.clone(),
                operation_id.clone(),
                document.clone(),
                lineage_refs,
                append_id,
            ))
            .await;
        assert!(matches!(
            result,
            Err(StructuredStoreError::CandidateRejected)
        ));
        assert!(matches!(
            reader.load_verified(&rejected_run_id).await,
            Err(StructuredStoreError::RunNotFound)
        ));
    }
}

#[tokio::test]
async fn ambiguous_effect_authorization_parks_possible_entry_without_invocation() {
    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let binding_calls = Arc::new(AtomicUsize::new(0));
    let first_certificate = binding_object(46);
    let second_certificate = binding_object(47);
    let lineage_head = lineage_head_object(48);
    let binding_source = refresh_effect_binding_source(
        first_certificate.clone(),
        second_certificate.clone(),
        lineage_head.clone(),
        &adapter_calls,
        &binding_calls,
    );
    let (operation_id, resource_ref, registry) =
        refreshable_effect_registry(&settlement_calls, Arc::clone(&binding_source));
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<EffectState>(operation_id.clone()))
        .expect("certified")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    adapter_calls.store(0, Ordering::SeqCst);
    settlement_calls.store(0, Ordering::SeqCst);
    let store = StructuredRunStore::new(
        InjectingBackend::new(
            store_identity(46),
            InjectAppend::AuthorizationAcknowledgementUnknown,
        ),
        program_verifier,
        Arc::new(RefreshBindingVerifier {
            lineage_ref: resource_ref.clone(),
            first_certificate: first_certificate.clone(),
            second_certificate: second_certificate.clone(),
            lineage_head: lineage_head.clone(),
            accept_supersession: true,
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(46);
    runtime
        .admit_run(effect_admission(
            run_id.clone(),
            operation_id,
            document,
            resource_ref,
        ))
        .await
        .expect("effect admission");

    assert_eq!(
        runtime
            .drive_once(&run_id)
            .await
            .expect("ambiguous Effect authorization"),
        DriveOutcome::ConcurrentProgress
    );
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_source.first.target_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_source.second.target_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        reader
            .load_verified(&run_id)
            .await
            .expect("parked Effect")
            .frontier(),
        mfm_store::structured::StructuredFrontier::PossibleEntry
    ));
    assert_eq!(
        runtime
            .drive_once(&run_id)
            .await
            .expect("recovered parked Effect"),
        DriveOutcome::PossibleEntry
    );
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_source.first.target_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_source.second.target_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshable_effect_continues_after_fresh_process_release_rotation() {
    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let binding_calls = Arc::new(AtomicUsize::new(0));
    let first_certificate = binding_object(31);
    let second_certificate = binding_object(32);
    let lineage_head = lineage_head_object(33);
    let binding_source = refresh_effect_binding_source(
        first_certificate.clone(),
        second_certificate.clone(),
        lineage_head.clone(),
        &adapter_calls,
        &binding_calls,
    );
    let (operation_id, resource_ref, registry) =
        refreshable_effect_registry(&settlement_calls, Arc::clone(&binding_source));
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<EffectState>(operation_id.clone()))
        .expect("certified")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    settlement_calls.store(0, Ordering::SeqCst);
    adapter_calls.store(0, Ordering::SeqCst);

    let backend = StructuredMemoryBackend::new(store_identity(30));
    let backend_probe = backend.clone();
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(RefreshBindingVerifier {
            lineage_ref: resource_ref.clone(),
            first_certificate: first_certificate.clone(),
            second_certificate: second_certificate.clone(),
            lineage_head: lineage_head.clone(),
            accept_supersession: true,
        }),
    );
    let (writer, reader) = store.split();
    let runtime = Runtime::new(writer, processes);
    let run_id = run_id(30);
    runtime
        .admit_run(effect_admission(
            run_id.clone(),
            operation_id,
            document,
            resource_ref,
        ))
        .await
        .expect("effect admission");

    assert_eq!(
        runtime.drive_once(&run_id).await.expect("first effect"),
        DriveOutcome::AccessObserved
    );
    let refreshed = reader.load_verified(&run_id).await.expect("refresh fold");
    let mfm_store::structured::StructuredFrontier::Actions(actions) = refreshed.frontier() else {
        panic!("supersession must expose exactly the next Effect action");
    };
    assert!(matches!(
        actions.as_slice(),
        [mfm_store::structured::ActionableState {
            leaf: mfm_store::structured::StateLeaf::Refreshable {
                public_lineage_head_ref,
                ..
            },
            ..
        }] if public_lineage_head_ref == &lineage_head.content_ref
    ));
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_source.first.target_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_source.second.target_calls.load(Ordering::SeqCst), 0);
    assert_eq!(binding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 0);

    drop(refreshed);
    drop(runtime);
    drop(reader);
    let saw_retained_authorization = Arc::new(AtomicBool::new(false));
    let saw_retained_supersession = Arc::new(AtomicBool::new(false));
    let saw_current_descendant = Arc::new(AtomicBool::new(false));
    let (_, restarted_resource_ref, restarted_registry) =
        refreshable_effect_registry(&settlement_calls, Arc::clone(&binding_source));
    let (restarted_program_verifier, restarted_processes) =
        split_qualified_registry(restarted_registry);
    settlement_calls.store(0, Ordering::SeqCst);
    let restarted_store = StructuredRunStore::new(
        backend_probe.clone(),
        restarted_program_verifier,
        Arc::new(RestartedRefreshBindingVerifier {
            lineage_ref: restarted_resource_ref,
            first_certificate: first_certificate.clone(),
            second_certificate: second_certificate.clone(),
            lineage_head: lineage_head.clone(),
            saw_retained_authorization: Arc::clone(&saw_retained_authorization),
            saw_retained_supersession: Arc::clone(&saw_retained_supersession),
            saw_current_descendant: Arc::clone(&saw_current_descendant),
        }),
    );
    let (restarted_writer, restarted_reader) = restarted_store.split();
    let runtime = Runtime::new(restarted_writer, restarted_processes);
    let replayed = restarted_reader
        .load_verified(&run_id)
        .await
        .expect("fresh process replays retained release history");
    assert!(matches!(
        replayed.frontier(),
        mfm_store::structured::StructuredFrontier::Actions(_)
    ));
    assert!(saw_retained_authorization.load(Ordering::SeqCst));
    assert!(saw_retained_supersession.load(Ordering::SeqCst));

    assert_eq!(
        runtime.drive_once(&run_id).await.expect("refreshed effect"),
        DriveOutcome::AccessObserved
    );
    assert!(saw_current_descendant.load(Ordering::SeqCst));
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 2);
    assert_eq!(binding_source.first.target_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_source.second.target_calls.load(Ordering::SeqCst), 1);
    assert_eq!(binding_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        runtime
            .drive_once(&run_id)
            .await
            .expect("effect settlement"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 2);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), 1);
    let callback_count = settlement_calls.load(Ordering::SeqCst);
    let closed = restarted_reader
        .load_verified(&run_id)
        .await
        .expect("fresh process callback-free loads the completed retained history");
    assert!(closed.closed_outcome_ref().is_some());
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 2);
    assert_eq!(settlement_calls.load(Ordering::SeqCst), callback_count);

    let raw = backend_probe
        .load(&run_id)
        .await
        .expect("raw five-family history")
        .expect("persisted refresh run");
    assert_eq!(raw.batches.len(), 6);
    let families = raw
        .batches
        .iter()
        .flat_map(|batch| batch.records.iter())
        .map(|assigned| match &assigned.record {
            RunRecord::RunAdmitted(_) => "admitted",
            RunRecord::StateTransitionCommitted(_) => "transition",
            RunRecord::ExternalAccessAuthorized(_) => "authorized",
            RunRecord::ExternalAccessObserved(_) => "observed",
            RunRecord::RunClosed(_) => "closed",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        families,
        [
            "admitted",
            "authorized",
            "observed",
            "authorized",
            "observed",
            "transition",
            "closed",
        ]
    );
}

struct ReadFixture {
    runtime: Runtime<InjectingBackend>,
    reader: mfm_store::structured::StructuredRunHistoryReader<InjectingBackend>,
    run_id: RunId,
    request_calls: Arc<AtomicUsize>,
    binding_calls: Arc<AtomicUsize>,
    selected_state_input: Arc<Mutex<Option<LexicalValueRef>>>,
    adapter_calls: Arc<AtomicUsize>,
    adapter_input: Arc<AtomicU64>,
    settlement_calls: Arc<AtomicUsize>,
    settlement_input: Arc<AtomicU64>,
    history_loads: Arc<AtomicUsize>,
    backend_probe: StructuredMemoryBackend,
    observation_append_entered: Arc<Notify>,
}

async fn read_fixture(
    discriminator: u8,
    injection: InjectAppend,
    preauthorize: bool,
) -> ReadFixture {
    read_fixture_with_binding_options(discriminator, injection, preauthorize, true, true).await
}

async fn read_fixture_with_binding_options(
    discriminator: u8,
    injection: InjectAppend,
    preauthorize: bool,
    binding_available: bool,
    binding_certificate_matches: bool,
) -> ReadFixture {
    read_fixture_with_options_and_input_probe(
        discriminator,
        injection,
        preauthorize,
        binding_available,
        binding_certificate_matches,
        false,
        false,
    )
    .await
}

async fn read_fixture_with_input_substitution_probe(discriminator: u8) -> ReadFixture {
    read_fixture_with_options_and_input_probe(
        discriminator,
        InjectAppend::None,
        false,
        true,
        true,
        true,
        false,
    )
    .await
}

async fn read_invalid_settlement_fixture(discriminator: u8) -> ReadFixture {
    read_fixture_with_options_and_input_probe(
        discriminator,
        InjectAppend::None,
        false,
        true,
        true,
        false,
        true,
    )
    .await
}

async fn read_fixture_with_options_and_input_probe(
    discriminator: u8,
    injection: InjectAppend,
    preauthorize: bool,
    binding_available: bool,
    binding_certificate_matches: bool,
    probe_wrong_state_input: bool,
    invalidate_returned_settlement: bool,
) -> ReadFixture {
    let request_calls = Arc::new(AtomicUsize::new(0));
    let settlement_calls = Arc::new(AtomicUsize::new(0));
    let settlement_input = Arc::new(AtomicU64::new(0));
    let adapter_calls = Arc::new(AtomicUsize::new(0));
    let adapter_input = Arc::new(AtomicU64::new(0));
    let binding_calls = Arc::new(AtomicUsize::new(0));
    let selected_state_input = Arc::new(Mutex::new(None));
    let certificate = binding_object(discriminator);
    let operation_id = stable(&format!(
        "mfm.runtime.fixture/read-operation-{discriminator}"
    ))
    .expect("operation id");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");

    let request_counter = Arc::clone(&request_calls);
    let settle_counter = Arc::clone(&settlement_calls);
    let settle_value = Arc::clone(&settlement_input);
    let state_descriptor = implementation_descriptor::<ReadState>(&mut assembly, "read-state");
    assembly
        .register_state::<ReadState>(
            state_descriptor,
            StructuredStateCallbacks::Read {
                request: Arc::new(move |frame: StateFrame<'_, Value>| {
                    request_counter.fetch_add(1, Ordering::SeqCst);
                    frame.input().clone()
                }),
                settle: Arc::new(
                    move |_frame, observation: CommittedObservationView<'_, Value, Value>| {
                        settle_counter.fetch_add(1, Ordering::SeqCst);
                        let value = match observation.observation() {
                            CommittedObservation::Returned(_) if invalidate_returned_settlement => {
                                return StateSettlement::InvalidEvidence;
                            }
                            CommittedObservation::Returned(value)
                            | CommittedObservation::SafeFailure(value) => value.clone(),
                        };
                        settle_value.store(value.value, Ordering::SeqCst);
                        StateSettlement::Proposed(ProposedStateOutcome::Success(value))
                    },
                ),
                reviewed_safe_failures: vec![ReviewedSafeFailureCase::new(
                    Value { value: 1 },
                    Value { value: 2 },
                    ProposedStateOutcome::Success(Value { value: 2 }),
                )],
            },
        )
        .expect("read state");
    let capability_contract_ref = read_capability_contract()
        .expect("capability contract")
        .content_ref()
        .expect("capability ref");
    let capability_descriptor = implementation_descriptor_for_contract(
        &mut assembly,
        StructuredComponentKind::Capability,
        capability_contract_ref,
        "read-capability",
    );
    assembly
        .register_read_capability::<FixtureReadCapability, _>(
            capability_descriptor,
            Arc::new(FixtureReadCapabilityImplementation),
        )
        .expect("capability");
    let adapter_contract_ref = read_adapter_contract()
        .expect("adapter contract")
        .content_ref()
        .expect("adapter ref");
    let adapter_descriptor = implementation_descriptor_for_contract(
        &mut assembly,
        StructuredComponentKind::Adapter,
        adapter_contract_ref,
        "read-adapter",
    );
    assembly
        .register_read_adapter::<FixtureReadCapability, _>(
            adapter_descriptor,
            Arc::new(ExactReadBindingSource {
                binding: Arc::new(CountingReadAdapter {
                    calls: Arc::clone(&adapter_calls),
                    last_request: Arc::clone(&adapter_input),
                    certificate: if binding_certificate_matches {
                        certificate.clone()
                    } else {
                        binding_object(discriminator.wrapping_add(100))
                    },
                }),
                calls: Arc::clone(&binding_calls),
                selected_state_input: Arc::clone(&selected_state_input),
                available: binding_available,
            }),
        )
        .expect("adapter");
    assembly
        .register_entry_point(
            operation_id.clone(),
            one_state_program::<ReadState>(operation_id.clone()),
            profile(),
        )
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(one_state_program::<ReadState>(operation_id.clone()))
        .expect("certified")
        .into_document();
    let (program_verifier, processes) = split_qualified_registry(registry);
    let backend = InjectingBackend::new(store_identity(discriminator), injection);
    let backend_probe = backend.inner.clone();
    let history_loads = Arc::clone(&backend.loads);
    let observation_append_entered = Arc::clone(&backend.observation_append_entered);
    let store = StructuredRunStore::new(
        backend,
        program_verifier,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let (writer, reader) = store.split();
    request_calls.store(0, Ordering::SeqCst);
    settlement_calls.store(0, Ordering::SeqCst);
    settlement_input.store(0, Ordering::SeqCst);
    adapter_calls.store(0, Ordering::SeqCst);
    adapter_input.store(0, Ordering::SeqCst);
    let run_id = run_id(discriminator);
    writer
        .admit_run(admission(
            run_id.clone(),
            operation_id,
            document,
            7,
            &format!("read-admit-{discriminator}"),
        ))
        .await
        .expect("admission");
    if preauthorize || probe_wrong_state_input {
        let verified = reader.load_verified(&run_id).await.expect("admitted run");
        let mfm_store::structured::StructuredFrontier::Actions(actions) = verified.frontier()
        else {
            panic!("admitted run must expose its first action");
        };
        let [action] = actions.as_slice() else {
            panic!("one-state program must expose one action");
        };
        let action = action.clone();
        if probe_wrong_state_input {
            let mut substituted_input = action.input.clone();
            substituted_input.slot_ref =
                binding_object(discriminator.wrapping_add(101)).content_ref;
            assert_eq!(
                writer
                    .authorize_access(
                        reader.load_verified(&run_id).await.expect("admitted run"),
                        &AccessAuthorizationProposal::new(
                            AppendRequestId::new(format!(
                                "wrong-input-preauthorize-{discriminator}"
                            ))
                            .expect("authorization append id"),
                            substituted_input,
                            ProposedCanonicalValue::from_value(&Value { value: 7 })
                                .expect("authorization request"),
                            certificate.clone(),
                        ),
                    )
                    .await
                    .expect_err("substituted state input must fail before append"),
                StructuredStoreError::CandidateRejected
            );
        }
        if preauthorize {
            writer
                .authorize_access(
                    reader.load_verified(&run_id).await.expect("admitted run"),
                    &AccessAuthorizationProposal::new(
                        AppendRequestId::new(format!("preauthorize-{discriminator}"))
                            .expect("authorization append id"),
                        action.input.clone(),
                        ProposedCanonicalValue::from_value(&Value { value: 7 })
                            .expect("authorization request"),
                        certificate.clone(),
                    ),
                )
                .await
                .expect("preauthorization");
        }
    }
    history_loads.store(0, Ordering::SeqCst);
    let runtime = Runtime::new(writer, processes);
    ReadFixture {
        runtime,
        reader,
        run_id,
        request_calls,
        binding_calls,
        selected_state_input,
        adapter_calls,
        adapter_input,
        settlement_calls,
        settlement_input,
        history_loads,
        backend_probe,
        observation_append_entered,
    }
}

fn one_state_program<S>(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram
where
    S: State<Input = Value, Output = Value, Failure = Never>,
    Sequential: AllowsExecution<S::Execution>,
{
    one_state_program_with_label::<S>(operation_id, "state")
}

fn codec_fault_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder = OperationBuilder::<UnencodableValue, Never>::new(
        operation_id,
        stable("root").expect("root"),
    )
    .expect("builder");
    let input = builder
        .input::<Value>(stable("input").expect("input"))
        .expect("input root");
    let output = builder
        .root()
        .state::<CodecFaultState>(stable("state").expect("state label"), &input)
        .expect("state")
        .infallible()
        .expect("infallible");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("program")
}

fn fact_then_read_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Value, Never>::new(operation_id, stable("root").expect("root"))
            .expect("builder");
    let input = builder
        .input::<Value>(stable("input").expect("input"))
        .expect("input root");
    let published = builder
        .root()
        .state::<FactState>(stable("publish").expect("publish state"), &input)
        .expect("publish state")
        .infallible()
        .expect("infallible publish");
    let selected = builder
        .root()
        .state::<PriorRunFactReadState>(stable("select").expect("select state"), &published)
        .expect("select state")
        .infallible()
        .expect("infallible selection");
    let completion = builder.succeed(&selected).expect("success");
    builder.finish(completion).expect("fact-then-read program")
}

fn fan_out_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    type Join = FanOutResults<Value, Never>;

    let mut builder =
        OperationBuilder::<Join, Never>::new(operation_id, stable("root").expect("root"))
            .expect("builder");
    let input = builder
        .input::<Value>(stable("input").expect("input"))
        .expect("input root");
    let mut fan_out = builder
        .root()
        .fan_out::<Value, Never>(stable("parallel").expect("fan-out label"))
        .expect("fan-out");
    for (lane_label, state_label) in [("lane-a", "state-a"), ("lane-b", "state-b")] {
        fan_out
            .lane(stable(lane_label).expect("lane label"), |lane| {
                let output = lane
                    .state::<PureState>(stable(state_label)?, &input)?
                    .infallible()?;
                lane.normal(&output)
            })
            .expect("fan-out lane");
    }
    let joined = fan_out.finish().expect("fan-out join");
    let completion = builder.succeed(&joined).expect("success");
    builder.finish(completion).expect("fan-out program")
}

fn one_state_program_with_label<S>(
    operation_id: StableId,
    state_label: &str,
) -> mfm_spec::structured::AuthoredStructuredProgram
where
    S: State<Input = Value, Output = Value, Failure = Never>,
    Sequential: AllowsExecution<S::Execution>,
{
    let mut builder =
        OperationBuilder::<Value, Never>::new(operation_id, stable("root").expect("root"))
            .expect("builder");
    let input = builder
        .input::<Value>(stable("input").expect("input"))
        .expect("input root");
    let output = builder
        .root()
        .state::<S>(stable(state_label).expect("state label"), &input)
        .expect("state")
        .infallible()
        .expect("infallible");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("program")
}

fn program_cache_registry(
    callback_calls: Arc<AtomicUsize>,
) -> (
    StableId,
    mfm_spec::structured::AuthoredStructuredProgram,
    mfm_spec::structured::AuthoredStructuredProgram,
    QualifiedProgramRegistry,
) {
    let operation_id = stable("mfm.runtime.fixture/program-cache-operation").expect("operation id");
    let template = one_state_program_with_label::<PureState>(operation_id.clone(), "state");
    let alternate =
        one_state_program_with_label::<PureState>(operation_id.clone(), "alternate-state");
    let mut assembly = ProgramRegistryBuilder::new();
    assembly.register_value::<Value>().expect("value contract");
    let descriptor = implementation_descriptor::<PureState>(&mut assembly, "program-cache");
    assembly
        .register_state::<PureState>(
            descriptor,
            StructuredStateCallbacks::Pure {
                apply: Arc::new(move |frame: StateFrame<'_, Value>| {
                    callback_calls.fetch_add(1, Ordering::SeqCst);
                    ProposedStateOutcome::Success(frame.input().clone())
                }),
            },
        )
        .expect("pure state");
    assembly
        .register_entry_point(operation_id.clone(), template.clone(), profile())
        .expect("entry point");
    let registry = assembly
        .build(std::slice::from_ref(&operation_id))
        .expect("qualified registry");
    (operation_id, template, alternate, registry)
}

fn fallible_program(operation_id: StableId) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Value, FailureValue>::new(operation_id, stable("root").expect("root"))
            .expect("builder");
    builder
        .root()
        .failure_map::<FailureValue, ValueFailureMapper>()
        .expect("failure mapper");
    let input = builder
        .input::<Value>(stable("input").expect("input"))
        .expect("input root");
    let output = builder
        .root()
        .state::<ConditionalFailureState>(stable("state").expect("state label"), &input)
        .expect("state")
        .or_default()
        .expect("default failure handler");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("fallible program")
}

fn fallible_read_program(
    operation_id: StableId,
) -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Value, FailureValue>::new(operation_id, stable("root").expect("root"))
            .expect("builder");
    builder
        .root()
        .failure_map::<FailureValue, ValueFailureMapper>()
        .expect("failure mapper");
    let input = builder
        .input::<Value>(stable("input").expect("input"))
        .expect("input root");
    let output = builder
        .root()
        .state::<FallibleReadState>(stable("state").expect("state label"), &input)
        .expect("state")
        .or_default()
        .expect("default failure handler");
    let completion = builder.succeed(&output).expect("success");
    builder.finish(completion).expect("fallible Read program")
}

fn implementation_descriptor<S: State>(
    assembly: &mut ProgramRegistryBuilder,
    suffix: &str,
) -> SecretFreeImplementationDescriptor {
    implementation_descriptor_for_contract(
        assembly,
        StructuredComponentKind::State,
        state_contract::<S>()
            .expect("state contract")
            .state_contract_ref,
        suffix,
    )
}

fn implementation_descriptor_for_contract(
    assembly: &mut ProgramRegistryBuilder,
    component_kind: StructuredComponentKind,
    semantic_contract_ref: mfm_ids::ContentRef,
    suffix: &str,
) -> SecretFreeImplementationDescriptor {
    let executable_identity_ref = assembly
        .register_executable_identity(SecretFreeExecutableIdentity {
            executable_id: stable("mfm.runtime.fixture/test-executable").expect("executable"),
        })
        .expect("executable identity");
    let qualification_artifact_ref = assembly
        .register_qualification_artifact(SecretFreeQualificationArtifact {
            qualification_id: stable("mfm.runtime.fixture/test-qualification")
                .expect("qualification"),
        })
        .expect("qualification artifact");
    SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id: stable(&format!("mfm.runtime.fixture/{suffix}-implementation"))
            .expect("implementation id"),
        executable_identity_ref,
        qualification_artifact_ref,
    }
}

fn read_capability_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new_read_capability(
        stable("mfm.runtime.fixture/read-capability")?,
        mfm_spec::structured::structured_value_contract_ref::<Value>()?,
        mfm_spec::structured::structured_value_contract_ref::<Value>()?,
        mfm_spec::structured::structured_value_contract_ref::<Value>()?,
        read_adapter_contract()?.content_ref()?,
    )
    .map_err(Into::into)
}

fn read_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Adapter,
        stable("mfm.runtime.fixture/read-adapter")?,
        Vec::<StructuredComponentDependency>::new(),
    )
    .map_err(Into::into)
}

fn effect_capability_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new_effect_capability_refreshable(
        stable("mfm.runtime.fixture/effect-capability")?,
        mfm_spec::structured::structured_value_contract_ref::<Value>()?,
        mfm_spec::structured::structured_value_contract_ref::<Value>()?,
        mfm_spec::structured::structured_value_contract_ref::<Value>()?,
        mfm_spec::structured::structured_value_contract_ref::<Value>()?,
        resource_contract()?.content_ref()?,
        effect_adapter_contract()?.content_ref()?,
    )
    .map_err(Into::into)
}

fn effect_adapter_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Adapter,
        stable("mfm.runtime.fixture/effect-adapter")?,
        vec![StructuredComponentDependency {
            component_kind: StructuredComponentKind::Resource,
            contract_ref: resource_contract()?.content_ref()?,
        }],
    )
    .map_err(Into::into)
}

fn resource_contract() -> mfm_program::Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Resource,
        stable("mfm.runtime.fixture/effect-resource")?,
        Vec::new(),
    )
    .map_err(Into::into)
}

fn fact_descriptor() -> mfm_program::Result<StructuredFactDescriptor> {
    let contract_ref = mfm_spec::structured::structured_value_contract_ref::<Value>()?;
    StructuredFactDescriptor::new(
        stable("mfm.runtime.fixture/value-fact")?,
        contract_ref.clone(),
        contract_ref,
    )
    .map_err(Into::into)
}

fn fact_set(subject: Value, response: Value) -> FactSet {
    let descriptor = fact_descriptor().expect("fact descriptor");
    FactSet::one(
        FactProposal::new(
            0,
            descriptor.descriptor_ref,
            proposed_fact_value(subject),
            proposed_fact_value(response),
        )
        .expect("fact proposal"),
    )
}

fn proposed_fact_value(value: Value) -> ProposedFactValue {
    let contract =
        mfm_spec::structured::structured_value_contract::<Value>().expect("fact value contract");
    let json = serde_json::to_string(&value).expect("fact value JSON");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json).expect("canonical fact value");
    ProposedFactValue::new(
        contract.schema_id().clone(),
        contract.semantic_type_id().clone(),
        contract.role().clone(),
        contract.media_type(),
        contract.evidence_contract_ref().clone(),
        canonical,
    )
    .expect("proposed fact value")
}

fn profile() -> StructuredExpansionProfile {
    StructuredExpansionProfile {
        policies: Vec::new(),
        max_occurrences: 8,
        max_declarations: 8,
        max_lanes: 4,
        max_fan_out_depth: 2,
        max_branch_depth: 4,
    }
}

fn admission(
    run_id: RunId,
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    value: u64,
    append_id: &str,
) -> StructuredAdmissionRequest {
    admission_with_invocation(
        run_id,
        operation_id,
        document,
        value,
        "00000000-0000-4000-8000-000000000001",
        append_id,
    )
}

fn admission_with_invocation(
    run_id: RunId,
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    value: u64,
    invocation_identity: &str,
    append_id: &str,
) -> StructuredAdmissionRequest {
    StructuredAdmissionRequest::new(
        run_id,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "2".repeat(32))).expect("tenant"),
        InvocationIdentity::new(invocation_identity).expect("invocation"),
        operation_id,
        document,
        admission_material(9),
        vec![ProposedCanonicalValue::from_value(&Value { value }).expect("initial value")],
        AppendRequestId::new(append_id).expect("append id"),
    )
}

fn effect_admission(
    run_id: RunId,
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    lineage_ref: mfm_ids::ContentRef,
) -> StructuredAdmissionRequest {
    effect_admission_with_lineages(
        run_id,
        operation_id,
        document,
        vec![lineage_ref],
        "effect-admit",
    )
}

fn effect_admission_with_lineages(
    run_id: RunId,
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    lineage_refs: Vec<mfm_ids::ContentRef>,
    append_id: &str,
) -> StructuredAdmissionRequest {
    StructuredAdmissionRequest::new(
        run_id,
        TenantScopeId::new(format!("{}{}", TenantScopeId::PREFIX, "3".repeat(32))).expect("tenant"),
        InvocationIdentity::new("00000000-0000-4000-8000-000000000002").expect("invocation"),
        operation_id,
        document,
        effect_admission_material(lineage_refs).expect("effect admission material"),
        vec![ProposedCanonicalValue::from_value(&Value { value: 7 }).expect("initial value")],
        AppendRequestId::new(append_id).expect("append id"),
    )
}

fn effect_admission_material(
    lineage_refs: Vec<mfm_ids::ContentRef>,
) -> std::result::Result<StructuredAdmissionMaterial, StructuredStoreError> {
    StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "mfm.runtime.fixture.effect-configuration",
            30,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "mfm.runtime.fixture.effect-context",
            30,
        ),
        PriorRunFactSourceManifest::new(Vec::new())
            .and_then(|manifest| manifest.to_history_object())
            .map_err(|_| StructuredStoreError::InvalidHistory)?,
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "mfm.runtime.fixture.effect-routing",
            30,
        ),
        lineage_refs,
    )
}

fn admission_material(discriminator: u8) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "mfm.runtime.fixture.configuration",
            discriminator,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "mfm.runtime.fixture.context",
            discriminator,
        ),
        PriorRunFactSourceManifest::new(Vec::new())
            .and_then(|manifest| manifest.to_history_object())
            .expect("source manifest"),
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "mfm.runtime.fixture.routing",
            discriminator,
        ),
        Vec::new(),
    )
    .expect("admission material")
}

fn admission_material_with_source(
    discriminator: u8,
    source_manifest: HistoryObject,
) -> StructuredAdmissionMaterial {
    StructuredAdmissionMaterial::new(
        admission_object(
            ADMISSION_CONFIGURATION_OBJECT_TYPE,
            "mfm.runtime.fixture.configuration",
            discriminator,
        ),
        admission_object(
            ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
            "mfm.runtime.fixture.context",
            discriminator,
        ),
        source_manifest,
        admission_object(
            ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
            "mfm.runtime.fixture.routing",
            discriminator,
        ),
        Vec::new(),
    )
    .expect("admission material with source manifest")
}

fn admission_object(object_type: &str, schema: &str, discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        stable(object_type).expect("object type"),
        SchemaId::new(
            schema,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, schema.as_bytes()[0]]),
        )
        .expect("schema"),
        "{\"entries\":[]}",
    )
    .expect("admission object")
}

fn binding_object(discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        stable("mfm.runtime.fixture.physical-binding").expect("binding type"),
        SchemaId::new(
            "mfm.runtime.fixture.physical-binding",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, 91]),
        )
        .expect("binding schema"),
        format!("{{\"binding\":{discriminator}}}"),
    )
    .expect("binding object")
}

fn lineage_head_object(discriminator: u8) -> HistoryObject {
    HistoryObject::new(
        stable("mfm.runtime.fixture.lineage-head").expect("lineage head type"),
        SchemaId::new(
            "mfm.runtime.fixture.lineage-head",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&[discriminator, 92]),
        )
        .expect("lineage head schema"),
        format!("{{\"head\":{discriminator}}}"),
    )
    .expect("lineage head object")
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

fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}
