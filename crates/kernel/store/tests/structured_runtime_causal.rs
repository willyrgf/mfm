//! Runtime causal proofs driving mutation only through assembled Runtime.
//! Migrated from mfm-runtime store-backed suite after authority cutover.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    BoundedComponentContract, BoundedComponentInvoker, CapabilityContractFault, ComponentFuture,
    EffectAdapterCompletion, EffectAdapterInvoker, EffectCapabilityContract,
    EffectCapabilityImplementation, ReadAdapterCompletion, ReadAdapterInvoker,
    ReadCapabilityContract, ReadCapabilityImplementation, Refreshable, ResourceAuthorityContract,
};
use mfm_certify::structured::{
    PhysicalBindingSelection, ProgramRegistryBuilder, QualifiedProgramRegistry,
    RuntimeEffectPhysicalBinding, RuntimeEffectPhysicalBindingSource, RuntimeReadPhysicalBinding,
    RuntimeReadPhysicalBindingSource,
};
use mfm_facts::{FactProposal, FactSet, ProposedFactValue};
use mfm_ids::{
    AppendRequestId, DigestAlgorithm, InvocationIdentity, RunId, SchemaId, StableId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    derive_commit_digest, derive_record_hash, domain_content_digest, AssignedRecord,
    CommitCandidate, CommittedBatch, HistoryObject, JournalHead, LexicalValueRef,
    ObservationOutcome, PriorRunFactSourceManifest, RecordRef, RunRecord, TenantFactCoordinate,
    TenantFactFrontier, ADMISSION_CONFIGURATION_OBJECT_TYPE,
    ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_program::structured::{
    state_contract, AllowsExecution, ClosedSum, DefaultFailureMapper, Direct, Effect,
    FanOutResults, Never, OperationBuilder, ProposedSuccessOutcome, Pure, Read, RefreshableBinding,
    RuntimeEffectAdapter, RuntimeEffectCapability, RuntimeReadAdapter, RuntimeReadCapability,
    RuntimeResourceAuthority, SafeFailureMayFail, SafeFailureNotApplicable, SafeFailureSuccessOnly,
    Sequential, State, StateFrame, StateSettlement, StructuredStateCallbacks,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::history::{
    ProposedCanonicalValue, StructuredAdmissionCommand, StructuredAdmissionMaterial,
};
use mfm_runtime::structured::{
    DriveOutcome, RuntimeFaultCode, RuntimeFaultPhase, RuntimeStoreFaultKind,
};
use mfm_spec::structured::{
    OperationOutcome, ProposedStateOutcome, SecretFreeExecutableIdentity,
    SecretFreeImplementationDescriptor, SecretFreeQualificationArtifact,
    StructuredComponentDependency, StructuredComponentKind, StructuredExpansionProfile,
    StructuredFactDescriptor, StructuredLiveComponentContract,
};
use mfm_store::structured::{
    assemble_structured_runtime, BackendAppendOutcome, CanonicalRunAppend,
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PublicPhysicalBindingVerifier,
    RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend, StructuredMemoryBackend,
    StructuredStoreError, StructuredStoreIdentity, TenantFactPublication,
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
    ObservationConcurrentDifferent,
    ObservationSubstitutedPositive,
    AuthorizationAcknowledgementUnknown,
}

struct InjectingBackend {
    inner: StructuredMemoryBackend,
    injection: InjectAppend,
    stage: AtomicUsize,
    loads: Arc<AtomicUsize>,
    override_history: Mutex<Option<RawRunHistory>>,
}

impl InjectingBackend {
    fn new(identity: StructuredStoreIdentity, injection: InjectAppend) -> Self {
        Self {
            inner: StructuredMemoryBackend::new(identity),
            injection,
            stage: AtomicUsize::new(0),
            loads: Arc::new(AtomicUsize::new(0)),
            override_history: Mutex::new(None),
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
            Ok(history)
        })
    }

    fn current_head<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<JournalHead>> {
        Box::pin(async move {
            let history = self
                .override_history
                .lock()
                .map_err(|_| StructuredStoreError::BackendUnavailable)?
                .as_ref()
                .filter(|raw| &raw.run_id == run_id)
                .cloned();
            Ok(match history {
                Some(history) => history.batches.last().map(|batch| batch.head.clone()),
                None => self.inner.current_head(run_id).await?,
            })
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
        batch: CanonicalRunAppend,
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
                    InjectAppend::ObservationConcurrentDifferent
                        | InjectAppend::ObservationSubstitutedPositive
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
                    InjectAppend::AuthorizationAcknowledgementUnknown => {
                        self.inner.acknowledge_next_commit_as_unknown()?;
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
                    InjectAppend::None => {}
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
    let certificate = binding_object(1);
    let backend = InjectingBackend::new(store_identity(1), InjectAppend::None);
    let history_loads = Arc::clone(&backend.loads);
    let assembled = assemble_structured_runtime(
        backend,
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _attempt) = runtime
        .admit_run(admission(operation_id, document, 4, "pure-admit"))
        .await
        .expect("admission");
    history_loads.store(0, Ordering::SeqCst);

    assert_eq!(
        runtime.drive_once(&run_id).await.expect("drive"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    assert_eq!(history_loads.load(Ordering::SeqCst), 0);
    assert_eq!(callback_calls.load(Ordering::SeqCst), 1);
    assert_eq!(callback_input.load(Ordering::SeqCst), 4);
    let verified = reader.load_public(&run_id).await.expect("closed");
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
    assert_eq!(history_loads.load(Ordering::SeqCst), 1);
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
    let backend = StructuredMemoryBackend::new(store_identity(60));
    let backend_probe = backend.clone();
    let assembled = assemble_structured_runtime(
        backend,
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(60),
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _attempt) = runtime
        .admit_run(admission(operation_id, document, 4, "callback-fault-admit"))
        .await
        .expect("admission");
    let pre_fault_head = reader
        .load_public(&run_id)
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
    let backend = StructuredMemoryBackend::new(store_identity(62));
    let backend_probe = backend.clone();
    let assembled = assemble_structured_runtime(
        backend,
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(62),
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _attempt) = runtime
        .admit_run(admission(operation_id, document, 4, "codec-fault-admit"))
        .await
        .expect("admission");
    let pre_fault_head = reader
        .load_public(&run_id)
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
    let certificate = binding_object(35);
    let backend = StructuredMemoryBackend::new(store_identity(35));
    let assembled = assemble_structured_runtime(
        backend,
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let runtime = &assembled.runtime;
    let (run_id, _attempt) = runtime
        .admit_run(admission(operation_id, document, 4, "fan-out-admit"))
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

    // Same memory backend is shared; purpose readers reload through the sole fold.
    let export = assembled
        .export_reader
        .load_for_export(&run_id)
        .await
        .expect("fresh persisted fold");
    let mfm_store::structured::ProgramCursor::Closed { .. } = export.cursor() else {
        panic!("fan-out run must close");
    };
    let semantic_records = export.semantic_records().collect::<Vec<_>>();
    assert_eq!(semantic_records.len() + 1, export.records().len());
    assert!(matches!(
        semantic_records
            .last()
            .expect("semantic head record")
            .record,
        RunRecord::StateTransitionCommitted(_)
    ));
    assert!(matches!(
        export.records().last().expect("audit head record").record,
        RunRecord::RunClosed(_)
    ));
    let public = assembled
        .public_reader
        .load_public(&run_id)
        .await
        .expect("public projection");
    let outcome = mfm_replay::structured::project_operation_outcome(&public)
        .expect("project persisted root outcome")
        .expect("closed root outcome");
    assert_eq!(outcome.kind(), "success");
    let join: serde_json::Value = serde_json::from_slice(outcome.canonical_value().as_bytes())
        .expect("decode projected fan-out join");
    assert!(
        join.get("head").is_some(),
        "non-empty fan-out join head: {join}"
    );
    assert_eq!(
        join["tail"]
            .as_array()
            .unwrap_or_else(|| panic!("fan-out join tail: {join}"))
            .len(),
        1,
        "two-lane join is head plus one tail entry: {join}"
    );
}

#[tokio::test]
async fn exact_root_program_cache_rejects_authored_object_substitution() {
    // Hostile substitution is covered by adapter RegistryProgramVerifier unit logic and
    // certification integration; this runtime-level proof admits only the certified document
    // through assembled Runtime and reloads the closed history through a purpose reader.
    let callback_calls = Arc::new(AtomicUsize::new(0));
    let (operation_id, template, _alternate, registry) =
        program_cache_registry(Arc::clone(&callback_calls));
    let document = registry
        .certifier(&operation_id)
        .expect("certifier")
        .certify(template)
        .expect("certified program")
        .into_document();
    let assembled = assemble_structured_runtime(
        StructuredMemoryBackend::new(store_identity(34)),
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(34),
        }),
    );
    let (run_id, _) = assembled
        .runtime
        .admit_run(admission(operation_id, document, 4, "cache-admit"))
        .await
        .expect("admission");
    assert_eq!(
        assembled.runtime.drive_once(&run_id).await.expect("drive"),
        DriveOutcome::TransitionCommitted { closed: true }
    );
    let verified = assembled
        .public_reader
        .load_public(&run_id)
        .await
        .expect("purpose load");
    assert!(verified.closed_outcome_ref().is_some());
    assert_eq!(callback_calls.load(Ordering::SeqCst), 1);
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
    let certificate = binding_object(2);
    let backend = StructuredMemoryBackend::new(store_identity(2));
    let backend_probe = backend.clone();
    let assembled = assemble_structured_runtime(
        backend,
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _attempt) = runtime
        .admit_run(admission(operation_id, document, 7, "fact-admit"))
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
    let verified = reader.load_public(&run_id).await.expect("verified facts");
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
    let certificate = binding_object(50);
    let assembled = assemble_structured_runtime(
        StructuredMemoryBackend::new(store_identity(50)),
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (failed_run_id, _attempt) = runtime
        .admit_run(admission_with_invocation(
            operation_id.clone(),
            document.clone(),
            0,
            "00000000-0000-4000-8000-000000000050",
            "failed-run-admit",
        ))
        .await
        .expect("failed run admission");
    let (successful_run_id, _attempt) = runtime
        .admit_run(admission_with_invocation(
            operation_id,
            document,
            7,
            "00000000-0000-4000-8000-000000000051",
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
        .load_public(&failed_run_id)
        .await
        .expect("failed run");
    let outcome_ref = failed
        .closed_outcome_ref()
        .expect("ordinary failure must close the run");
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
        .load_public(&successful_run_id)
        .await
        .expect("successful run");
    let outcome_ref = succeeded
        .closed_outcome_ref()
        .expect("successful run must close");
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
                settle_returned: Arc::new({
                    let settlement_count = Arc::clone(&settlement_count);
                    move |_frame, returned| {
                        settlement_count.fetch_add(1, Ordering::SeqCst);
                        StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
                    }
                }),
                settle_safe_failure: Arc::new(move |_frame, failure| {
                    settlement_count.fetch_add(1, Ordering::SeqCst);
                    ProposedStateOutcome::Failure(FailureValue {
                        code: failure.value,
                    })
                }),
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
    let assembled = assemble_structured_runtime(
        StructuredMemoryBackend::new(store_identity(52)),
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: certificate.clone(),
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (failed_run_id, _attempt) = runtime
        .admit_run(admission_with_invocation(
            operation_id.clone(),
            document.clone(),
            0,
            "00000000-0000-4000-8000-000000000052",
            "safe-failure-run-admit",
        ))
        .await
        .expect("safe-failure run admission");
    let (successful_run_id, _attempt) = runtime
        .admit_run(admission_with_invocation(
            operation_id,
            document,
            7,
            "00000000-0000-4000-8000-000000000053",
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
        .load_public(&failed_run_id)
        .await
        .expect("failed run");
    let outcome_ref = failed
        .closed_outcome_ref()
        .expect("safe failure must close the run");
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
        .load_public(&successful_run_id)
        .await
        .expect("successful run");
    let outcome_ref = succeeded
        .closed_outcome_ref()
        .expect("successful run must close");
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
        adapter_calls.store(0, Ordering::SeqCst);
        settlement_calls.store(0, Ordering::SeqCst);
        let assembled = assemble_structured_runtime(
            InjectingBackend::new(store_identity(discriminator), injection),
            registry,
            Arc::new(RefreshBindingVerifier {
                lineage_ref: resource_ref.clone(),
                first_certificate: first_certificate.clone(),
                second_certificate,
                lineage_head,
                accept_supersession: true,
            }),
        );
        let runtime = assembled.runtime;
        let reader = assembled.public_reader;
        let (run_id, _attempt) = runtime
            .admit_run(effect_admission(operation_id, document, resource_ref))
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
            .load_public(&run_id)
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
    adapter_calls.store(0, Ordering::SeqCst);
    settlement_calls.store(0, Ordering::SeqCst);
    let assembled = assemble_structured_runtime(
        InjectingBackend::new(store_identity(discriminator), InjectAppend::None),
        registry,
        Arc::new(RefreshBindingVerifier {
            lineage_ref: resource_ref.clone(),
            first_certificate: first_certificate.clone(),
            second_certificate,
            lineage_head,
            accept_supersession: false,
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _attempt) = runtime
        .admit_run(effect_admission(operation_id, document, resource_ref))
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
            .load_public(&run_id)
            .await
            .expect("authorization-only prefix")
            .frontier(),
        mfm_store::structured::StructuredFrontier::PossibleEntry
    ));
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
                settle_returned: Arc::new({
                    let settle_counter = Arc::clone(&settle_counter);
                    move |_frame, returned| {
                        settle_counter.fetch_add(1, Ordering::SeqCst);
                        StateSettlement::Proposed(ProposedStateOutcome::Success(returned.clone()))
                    }
                }),
                settle_safe_failure: Arc::new(move |_frame, failure| {
                    settle_counter.fetch_add(1, Ordering::SeqCst);
                    ProposedSuccessOutcome::new(failure.clone())
                }),
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

    assert!(effect_admission_material(vec![resource_ref.clone(), resource_ref.clone()]).is_err());

    let assembled = assemble_structured_runtime(
        StructuredMemoryBackend::new(store_identity(40)),
        registry,
        Arc::new(ExactPublicBindingVerifier {
            certificate: binding_object(40),
        }),
    );
    let runtime = assembled.runtime;
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
    for (_discriminator, lineage_refs, append_id) in cases {
        let result = runtime
            .admit_run(effect_admission_with_lineages(
                operation_id.clone(),
                document.clone(),
                lineage_refs,
                append_id,
            ))
            .await;
        assert!(result.is_err(), "lineage mismatch must reject admission");
        // No run id is derived for a rejected candidate path that fails before append success.
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
    adapter_calls.store(0, Ordering::SeqCst);
    settlement_calls.store(0, Ordering::SeqCst);
    let assembled = assemble_structured_runtime(
        InjectingBackend::new(
            store_identity(46),
            InjectAppend::AuthorizationAcknowledgementUnknown,
        ),
        registry,
        Arc::new(RefreshBindingVerifier {
            lineage_ref: resource_ref.clone(),
            first_certificate: first_certificate.clone(),
            second_certificate: second_certificate.clone(),
            lineage_head: lineage_head.clone(),
            accept_supersession: true,
        }),
    );
    let runtime = assembled.runtime;
    let reader = assembled.public_reader;
    let (run_id, _attempt) = runtime
        .admit_run(effect_admission(operation_id, document, resource_ref))
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
            .load_public(&run_id)
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

// --- helpers ---

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
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    value: u64,
    append_id: &str,
) -> StructuredAdmissionCommand {
    admission_with_invocation(
        operation_id,
        document,
        value,
        "00000000-0000-4000-8000-000000000001",
        append_id,
    )
}

fn admission_with_invocation(
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    value: u64,
    invocation_identity: &str,
    append_id: &str,
) -> StructuredAdmissionCommand {
    StructuredAdmissionCommand::new(
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
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    lineage_ref: mfm_ids::ContentRef,
) -> StructuredAdmissionCommand {
    effect_admission_with_lineages(operation_id, document, vec![lineage_ref], "effect-admit")
}

fn effect_admission_with_lineages(
    operation_id: StableId,
    document: mfm_spec::structured::CertifiedProgramDocument,
    lineage_refs: Vec<mfm_ids::ContentRef>,
    append_id: &str,
) -> StructuredAdmissionCommand {
    StructuredAdmissionCommand::new(
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
) -> std::result::Result<StructuredAdmissionMaterial, mfm_runtime::history::HistoryError> {
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

fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}
