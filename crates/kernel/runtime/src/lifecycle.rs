//! Runtime-owned admission, resume, and owner-fate coordination.
//!
//! The lower-level typed primitives in [`crate::single_trust`] are useful to domain integration
//! tests and adapters. This module is the process-facing coordinator: it owns one immutable
//! assembly/store association, keeps the latest qualified context in an affine session, and
//! returns every live owner through an explicit outcome.

use std::any::Any;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use mfm_canonical::raw_content_digest;
use mfm_capabilities::AccessCapabilityContract;
use mfm_ids::{short_stable_id_fragment, AppendRequestId, ContentRef, RunId, StableId};
use mfm_journal::single_trust::{
    BindingDescriptor, ImmutableObject, RunAdmitted, RunFrame, RunRecord, StateConcluded,
    StateOutcome, ValueRef,
};
use mfm_program::{canonical_value, ProgramCatalog, QualifiedTypedValue};
use mfm_store::single_trust::{AppendDisposition, QualifiedRun, ReducedRunState, RunAction};
use mfm_store::{ConclusionCommitOutcome, OpenedStructuredStore};
use mfm_values::MfmValue;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::single_trust::{
    AccessHandlerResolution, AccessImplementation, CommittedCall, PreparedExecution,
    PureImplementation, RuntimeAssembly, RuntimeError, State, UnresolvedClassification,
};

type LifecycleResult<T> = std::result::Result<T, RuntimeError>;
type LifecycleFuture<T> = Pin<Box<dyn Future<Output = LifecycleResult<T>> + Send + 'static>>;

/// Private process witness carried by every dynamically erased typed value.
#[derive(Debug)]
pub(crate) struct RuntimeWitness;

/// One owned typed value erased only inside Runtime.
pub(crate) struct ErasedValue {
    contract_ref: ContentRef,
    value_ref: ContentRef,
    canonical_bytes: Vec<u8>,
    value: Box<dyn Any + Send + Sync>,
    witness: Arc<RuntimeWitness>,
}

impl ErasedValue {
    fn from_qualified<T: MfmValue>(
        catalog: &ProgramCatalog,
        witness: &Arc<RuntimeWitness>,
        value: QualifiedTypedValue<T>,
    ) -> LifecycleResult<Self> {
        if !value.belongs_to_catalog(catalog) {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            contract_ref: value.contract_ref().clone(),
            value_ref: value.value_ref().clone(),
            canonical_bytes: value.canonical_bytes().to_vec(),
            value: Box::new(value),
            witness: Arc::clone(witness),
        })
    }

    fn into_qualified<T: MfmValue>(
        self,
        catalog: &ProgramCatalog,
        witness: &Arc<RuntimeWitness>,
        expected_contract: &ContentRef,
    ) -> LifecycleResult<QualifiedTypedValue<T>> {
        if !Arc::ptr_eq(&self.witness, witness) || &self.contract_ref != expected_contract {
            return Err(RuntimeError::Identity);
        }
        let value = self
            .value
            .downcast::<QualifiedTypedValue<T>>()
            .map_err(|_| RuntimeError::Value)?;
        if !value.belongs_to_catalog(catalog)
            || value.contract_ref() != expected_contract
            || value.value_ref() != &self.value_ref
        {
            return Err(RuntimeError::Identity);
        }
        Ok(*value)
    }

    fn contract_ref(&self) -> &ContentRef {
        &self.contract_ref
    }

    fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }

    fn as_value_ref(&self) -> ValueRef {
        ValueRef::new(self.contract_ref.clone(), self.value_ref.clone())
    }

    fn object(&self) -> LifecycleResult<ImmutableObject> {
        ImmutableObject::new(
            StableId::new("mfm.value").map_err(|_| RuntimeError::Value)?,
            self.value_ref.clone(),
            String::from_utf8(self.canonical_bytes.clone()).map_err(|_| RuntimeError::Value)?,
        )
        .map_err(|_| RuntimeError::Value)
    }
}

pub(crate) enum DynamicOutcome {
    Success {
        value: ErasedValue,
        facts: mfm_facts::FactProposalSet,
    },
    Failure(ErasedValue),
}

pub(crate) struct DynamicPreparationFailure {
    input: Option<ErasedValue>,
    error: RuntimeError,
}

/// Private erased registration bridge. It is constructed only by the typed assembly builder.
pub(crate) trait DynamicStateRegistration: Send + Sync {
    fn pure_evaluate(
        &self,
        assembly: &RuntimeAssembly,
        witness: &Arc<RuntimeWitness>,
        input: ErasedValue,
        input_contract: &ContentRef,
        output_contract: &ContentRef,
        failure_contract: Option<&ContentRef>,
    ) -> LifecycleResult<DynamicOutcome>;

    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    fn prepare_access(
        &self,
        assembly: &RuntimeAssembly,
        witness: &Arc<RuntimeWitness>,
        run_id: RunId,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        input: ErasedValue,
        binding_ref: ContentRef,
    ) -> std::result::Result<Box<dyn DynamicPrepared>, DynamicPreparationFailure>;

    fn reify(
        &self,
        catalog: &ProgramCatalog,
        witness: &Arc<RuntimeWitness>,
        contract: &ContentRef,
        canonical_bytes: &[u8],
    ) -> LifecycleResult<Option<ErasedValue>>;
}

pub(crate) trait DynamicPrepared: Send {
    fn intent_ref(&self) -> LifecycleResult<ValueRef>;

    #[allow(clippy::too_many_arguments)]
    fn commit(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
        store: Arc<OpenedStructuredStore>,
        current: QualifiedRun,
        reduced: ReducedRunState,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        input_ref: ValueRef,
        intent_ref: ValueRef,
        maximum_conclusion_bytes: u64,
        preparation_ordinal: u16,
        replaces: Option<mfm_journal::single_trust::PreparationRef>,
    ) -> Pin<Box<dyn Future<Output = DynamicCommit> + Send + 'static>>;
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum DynamicCommit {
    Direct {
        call: Box<dyn DynamicCall>,
        run: QualifiedRun,
        reduced: ReducedRunState,
    },
    Retained {
        prepared: Box<dyn DynamicPrepared>,
        disposition: Option<AppendDisposition>,
        reduced: ReducedRunState,
    },
}

pub(crate) trait DynamicCall: Send {
    fn execute(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
        witness: &Arc<RuntimeWitness>,
    ) -> LifecycleFuture<DynamicResolution>;
}

struct DynamicPure<S: State> {
    implementation: PureImplementation<S>,
}

struct DynamicAccess<S: State, C: AccessCapabilityContract> {
    implementation: AccessImplementation<S, C>,
    binding: BindingDescriptor,
}

struct TypedPrepared<S: State, C: AccessCapabilityContract> {
    prepared: PreparedExecution<S, C>,
    implementation: AccessImplementation<S, C>,
}

struct TypedCall<S: State, C: AccessCapabilityContract> {
    call: CommittedCall<S, C>,
    implementation: AccessImplementation<S, C>,
}

pub(crate) fn pure_registration<S: State>(
    implementation: PureImplementation<S>,
) -> Arc<dyn DynamicStateRegistration> {
    Arc::new(DynamicPure { implementation })
}

pub(crate) fn access_registration_with_binding<S: State, C: AccessCapabilityContract>(
    implementation: AccessImplementation<S, C>,
    binding: BindingDescriptor,
) -> Arc<dyn DynamicStateRegistration>
where
    C::Mode: crate::single_trust::RuntimePreparationMode,
{
    Arc::new(DynamicAccess {
        implementation,
        binding,
    })
}

fn qualify_erased<T: MfmValue>(
    assembly: &RuntimeAssembly,
    witness: &Arc<RuntimeWitness>,
    contract: ContentRef,
    value: T,
) -> LifecycleResult<ErasedValue> {
    let qualified = assembly
        .catalog()
        .qualify(contract, value)
        .map_err(|_| RuntimeError::Value)?;
    ErasedValue::from_qualified(assembly.catalog(), witness, qualified)
}

fn fact_proposals_object(
    facts: &mfm_facts::FactProposalSet,
) -> LifecycleResult<(ValueRef, ImmutableObject)> {
    facts.validate().map_err(|_| RuntimeError::Value)?;
    let canonical = canonical_value(facts).map_err(|_| RuntimeError::Value)?;
    let value_ref = ContentRef::new(
        mfm_facts::FactProposalSet::schema_id().map_err(|_| RuntimeError::Value)?,
        raw_content_digest(canonical.as_bytes()),
    )
    .map_err(|_| RuntimeError::Value)?;
    let value = ValueRef::new(value_ref.clone(), value_ref.clone());
    let object = ImmutableObject::new(
        StableId::new("mfm.value").map_err(|_| RuntimeError::Value)?,
        value_ref,
        canonical.as_str().to_owned(),
    )
    .map_err(|_| RuntimeError::Value)?;
    Ok((value, object))
}

fn try_reify<T: MfmValue>(
    catalog: &ProgramCatalog,
    witness: &Arc<RuntimeWitness>,
    contract: &ContentRef,
    canonical_bytes: &[u8],
) -> LifecycleResult<Option<ErasedValue>> {
    if T::schema_id().map_err(|_| RuntimeError::Value)? != *contract.schema_id() {
        return Ok(None);
    }
    let value = serde_json::from_slice::<T>(canonical_bytes).map_err(|_| RuntimeError::Value)?;
    let qualified = catalog
        .qualify(contract.clone(), value)
        .map_err(|_| RuntimeError::Value)?;
    ErasedValue::from_qualified(catalog, witness, qualified).map(Some)
}

impl<S: State> DynamicStateRegistration for DynamicPure<S> {
    fn pure_evaluate(
        &self,
        assembly: &RuntimeAssembly,
        witness: &Arc<RuntimeWitness>,
        input: ErasedValue,
        input_contract: &ContentRef,
        output_contract: &ContentRef,
        failure_contract: Option<&ContentRef>,
    ) -> LifecycleResult<DynamicOutcome> {
        let input =
            input.into_qualified::<S::Input>(assembly.catalog(), witness, input_contract)?;
        let outcome = self.implementation.evaluate_contained(input.as_ref())?;
        match outcome {
            mfm_capabilities::ProposedStateOutcome::Success { output, facts } => {
                facts.validate().map_err(|_| RuntimeError::Value)?;
                Ok(DynamicOutcome::Success {
                    value: qualify_erased(assembly, witness, output_contract.clone(), output)?,
                    facts,
                })
            }
            mfm_capabilities::ProposedStateOutcome::Failure { failure } => {
                let failure_contract = failure_contract.ok_or(RuntimeError::Value)?;
                Ok(DynamicOutcome::Failure(qualify_erased(
                    assembly,
                    witness,
                    failure_contract.clone(),
                    failure,
                )?))
            }
        }
    }

    fn prepare_access(
        &self,
        _assembly: &RuntimeAssembly,
        _witness: &Arc<RuntimeWitness>,
        _run_id: RunId,
        _occurrence: mfm_journal::single_trust::SequentialControlAddress,
        input: ErasedValue,
        _binding_ref: ContentRef,
    ) -> std::result::Result<Box<dyn DynamicPrepared>, DynamicPreparationFailure> {
        Err(DynamicPreparationFailure {
            input: Some(input),
            error: RuntimeError::Mode,
        })
    }

    fn reify(
        &self,
        catalog: &ProgramCatalog,
        witness: &Arc<RuntimeWitness>,
        contract: &ContentRef,
        canonical_bytes: &[u8],
    ) -> LifecycleResult<Option<ErasedValue>> {
        if let Some(value) = try_reify::<S::Input>(catalog, witness, contract, canonical_bytes)? {
            return Ok(Some(value));
        }
        try_reify::<S::Output>(catalog, witness, contract, canonical_bytes)
    }
}

impl<S: State, C: AccessCapabilityContract> DynamicStateRegistration for DynamicAccess<S, C>
where
    C::Mode: crate::single_trust::RuntimePreparationMode,
{
    fn pure_evaluate(
        &self,
        _assembly: &RuntimeAssembly,
        _witness: &Arc<RuntimeWitness>,
        _input: ErasedValue,
        _input_contract: &ContentRef,
        _output_contract: &ContentRef,
        _failure_contract: Option<&ContentRef>,
    ) -> LifecycleResult<DynamicOutcome> {
        Err(RuntimeError::Mode)
    }

    fn prepare_access(
        &self,
        assembly: &RuntimeAssembly,
        witness: &Arc<RuntimeWitness>,
        run_id: RunId,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        input: ErasedValue,
        binding_ref: ContentRef,
    ) -> std::result::Result<Box<dyn DynamicPrepared>, DynamicPreparationFailure> {
        let state = match assembly.program().document().declaration(&occurrence) {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return Err(DynamicPreparationFailure {
                    input: Some(input),
                    error: RuntimeError::Identity,
                })
            }
        };
        let binding = self.binding.clone();
        let input = match input.into_qualified::<S::Input>(
            assembly.catalog(),
            witness,
            state.input_contract_ref(),
        ) {
            Ok(input) => input,
            Err(error) => {
                return Err(DynamicPreparationFailure { input: None, error });
            }
        };
        let prepared =
            match PreparedExecution::new(assembly, run_id, occurrence, input, binding, binding_ref)
            {
                Ok(prepared) => prepared,
                Err(error) => return Err(DynamicPreparationFailure { input: None, error }),
            };
        Ok(Box::new(TypedPrepared {
            prepared,
            implementation: self.implementation.clone(),
        }))
    }

    fn reify(
        &self,
        catalog: &ProgramCatalog,
        witness: &Arc<RuntimeWitness>,
        contract: &ContentRef,
        canonical_bytes: &[u8],
    ) -> LifecycleResult<Option<ErasedValue>> {
        if let Some(value) = try_reify::<S::Input>(catalog, witness, contract, canonical_bytes)? {
            return Ok(Some(value));
        }
        try_reify::<S::Output>(catalog, witness, contract, canonical_bytes)
    }
}

impl<S: State, C: AccessCapabilityContract> DynamicPrepared for TypedPrepared<S, C>
where
    C::Mode: crate::single_trust::RuntimePreparationMode,
{
    fn intent_ref(&self) -> LifecycleResult<ValueRef> {
        let intent = self.prepared.intent();
        let canonical = canonical_value(intent).map_err(|_| RuntimeError::Value)?;
        let value_ref = ContentRef::new(
            C::Intent::schema_id().map_err(|_| RuntimeError::Value)?,
            raw_content_digest(canonical.as_bytes()),
        )
        .map_err(|_| RuntimeError::Value)?;
        Ok(ValueRef::new(
            nominal_contract_ref::<C::Intent>()?,
            value_ref,
        ))
    }

    fn commit(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
        store: Arc<OpenedStructuredStore>,
        current: QualifiedRun,
        reduced: ReducedRunState,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        input_ref: ValueRef,
        intent_ref: ValueRef,
        maximum_conclusion_bytes: u64,
        preparation_ordinal: u16,
        replaces: Option<mfm_journal::single_trust::PreparationRef>,
    ) -> Pin<Box<dyn Future<Output = DynamicCommit> + Send + 'static>> {
        Box::pin(async move {
            let TypedPrepared {
                prepared,
                implementation,
            } = *self;
            let committed = prepared
                .commit_opened(
                    &assembly,
                    &store,
                    &current,
                    &reduced,
                    expected_sequence,
                    append_request_id,
                    input_ref,
                    intent_ref,
                    maximum_conclusion_bytes,
                    preparation_ordinal,
                    replaces,
                )
                .await;

            match committed {
                crate::single_trust::OpenedPreparationCommit::Direct { call, run, reduced } => {
                    DynamicCommit::Direct {
                        call: Box::new(TypedCall {
                            call,
                            implementation,
                        }) as Box<dyn DynamicCall>,
                        run,
                        reduced,
                    }
                }
                crate::single_trust::OpenedPreparationCommit::Retained { owner, disposition } => {
                    DynamicCommit::Retained {
                        prepared: Box::new(TypedPrepared {
                            prepared: owner,
                            implementation,
                        }),
                        disposition: Some(disposition),
                        reduced,
                    }
                }
                crate::single_trust::OpenedPreparationCommit::Rejected { owner, error: _ } => {
                    DynamicCommit::Retained {
                        prepared: Box::new(TypedPrepared {
                            prepared: owner,
                            implementation,
                        }),
                        disposition: None,
                        reduced,
                    }
                }
            }
        })
    }
}

impl<S: State, C: AccessCapabilityContract> DynamicCall for TypedCall<S, C> {
    fn execute(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
        witness: &Arc<RuntimeWitness>,
    ) -> LifecycleFuture<DynamicResolution> {
        let witness = Arc::clone(witness);
        let TypedCall {
            call,
            implementation,
        } = *self;
        let expected_call_id = call.call_id().clone();
        Box::pin(async move {
            let resolution = implementation.execute(call).await?;
            DynamicResolution::from_handler(&assembly, &witness, &expected_call_id, resolution)
        })
    }
}

/// Result of an access implementation before Store conclusion qualification.
pub(crate) struct DynamicResolution {
    input: ErasedValue,
    intent: ErasedValue,
    evidence: Option<ErasedValue>,
    outcome: Option<DynamicOutcome>,
    classification: Option<UnresolvedClassification>,
    preparation: mfm_journal::single_trust::PreparationRef,
    fact_continuation: Option<mfm_store::FactContinuation>,
}

impl DynamicResolution {
    fn from_handler<S: State, C: AccessCapabilityContract>(
        assembly: &RuntimeAssembly,
        witness: &Arc<RuntimeWitness>,
        expected_call_id: &StableId,
        resolution: AccessHandlerResolution<S, S::Output, S::Failure, C>,
    ) -> LifecycleResult<Self> {
        let (
            brand,
            input,
            call_id,
            intent,
            evidence,
            outcome,
            classification,
            preparation,
            fact_continuation,
        ) = resolution.into_parts();
        if !assembly.has_brand(&brand) || &call_id != expected_call_id {
            return Err(RuntimeError::Identity);
        }
        if classification.is_some() != (evidence.is_none() && outcome.is_none())
            || classification.is_none() != (evidence.is_some() && outcome.is_some())
        {
            return Err(RuntimeError::Value);
        }
        let input = ErasedValue::from_qualified(assembly.catalog(), witness, input)?;
        let intent = qualify_erased(
            assembly,
            witness,
            nominal_contract_ref::<C::Intent>()?,
            intent,
        )?;
        let evidence = evidence
            .map(|evidence| {
                qualify_erased(
                    assembly,
                    witness,
                    nominal_contract_ref::<C::Evidence>()?,
                    evidence,
                )
            })
            .transpose()?;
        let outcome = outcome
            .map(|outcome| match outcome {
                mfm_capabilities::ProposedStateOutcome::Success { output, facts } => {
                    facts.validate().map_err(|_| RuntimeError::Value)?;
                    Ok::<DynamicOutcome, RuntimeError>(DynamicOutcome::Success {
                        value: qualify_erased(
                            assembly,
                            witness,
                            nominal_contract_ref::<S::Output>()?,
                            output,
                        )?,
                        facts,
                    })
                }
                mfm_capabilities::ProposedStateOutcome::Failure { failure } => {
                    Ok::<DynamicOutcome, RuntimeError>(DynamicOutcome::Failure(qualify_erased(
                        assembly,
                        witness,
                        nominal_contract_ref::<S::Failure>()?,
                        failure,
                    )?))
                }
            })
            .transpose()?;
        Ok(Self {
            input,
            intent,
            evidence,
            outcome,
            classification,
            preparation,
            fact_continuation,
        })
    }
}

fn nominal_contract_ref<T: MfmValue>() -> LifecycleResult<ContentRef> {
    ContentRef::new(
        T::schema_id().map_err(|_| RuntimeError::Value)?,
        raw_content_digest(b"mfm.contract.v1"),
    )
    .map_err(|_| RuntimeError::Value)
}

/// Stable reason for retaining a live session without attempting another State callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    /// The selected Access preparation is durable and awaits its provider-bound call.
    WaitingPreparation,
    /// The provider result is unresolved and the selected preparation remains durable.
    Unresolved(UnresolvedClassification),
}

/// Failure before an admission owner can become a live session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionFailure {
    /// The admission input does not belong to the Runtime's exact catalog/program.
    #[error("admission input is not qualified by this Runtime")]
    Identity,
    /// The Store rejected the admission without creating a live owner.
    #[error("admission could not be committed")]
    Store,
    /// The Runtime could not admit another bounded planning job.
    #[error("runtime admission capacity is unavailable")]
    Capacity,
}

/// Conflict between two semantic genesis candidates for one run identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("admission candidates conflict")]
pub struct AdmissionConflict;

/// Failure while loading a retained run for a Runtime resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResumeFailure {
    /// The run is absent or retained history is invalid.
    #[error("run cannot be resumed")]
    History,
    /// The run belongs to another Runtime/store/catalog brand.
    #[error("run identity is invalid")]
    Identity,
    /// The Runtime could not admit another bounded active session.
    #[error("runtime resume capacity is unavailable")]
    Capacity,
}

/// Callback-free terminal evidence returned only after a durable conclusion is qualified.
pub struct TerminalRun {
    run: QualifiedRun,
}

impl TerminalRun {
    fn new(run: QualifiedRun) -> Self {
        Self { run }
    }

    /// Returns the durable run identity.
    pub fn run_id(&self) -> &RunId {
        self.run.run_id()
    }

    /// Returns the qualified terminal head sequence.
    pub fn head_sequence(&self) -> u64 {
        self.run.head_sequence()
    }

    /// Returns the callback-free retained evidence.
    pub fn qualified_run(&self) -> &QualifiedRun {
        &self.run
    }
}

/// A parked session that may be explicitly driven again by its owner.
pub struct ParkedRun {
    session: RunSession,
    reason: ParkReason,
}

impl ParkedRun {
    /// Returns why this session is parked.
    pub const fn reason(&self) -> ParkReason {
        self.reason
    }

    /// Consumes the parked owner back into its affine session.
    pub fn into_session(self) -> RunSession {
        self.session
    }

    /// Returns the durable run identity retained by this parked owner.
    pub fn run_id(&self) -> &RunId {
        self.session.run_id()
    }

    /// Returns the qualified durable head retained by this parked owner.
    pub fn head_sequence(&self) -> u64 {
        self.session.head_sequence()
    }
}

#[allow(clippy::large_enum_variant)]
enum SuspendedOwner {
    Admission {
        runtime: Runtime,
        frame: RunFrame,
    },
    Conclusion(PendingConclusion),
    Preparation {
        runtime: Runtime,
        prepared: Box<dyn DynamicPrepared>,
        run: QualifiedRun,
        reduced: ReducedRunState,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        input_ref: ValueRef,
        intent_ref: ValueRef,
        maximum_conclusion_bytes: u64,
        preparation_ordinal: u16,
        replaces: Option<mfm_journal::single_trust::PreparationRef>,
        disposition: Option<AppendDisposition>,
    },
}

/// Runtime-owned affine conclusion handoff.
///
/// This owner contains the Store append authority and the retained session continuation, but no
/// provider, State implementation, raw response, or callback.  It can only be resolved through
/// the owning Runtime and cannot be cloned, serialized, or field-constructed by callers.
pub struct PendingConclusion {
    runtime: Runtime,
    owner: mfm_store::single_trust::PreparedConclusion,
    run: QualifiedRun,
    reduced: ReducedRunState,
    successor: Option<ErasedValue>,
}

impl PendingConclusion {
    fn new(
        runtime: Runtime,
        owner: mfm_store::single_trust::PreparedConclusion,
        run: QualifiedRun,
        reduced: ReducedRunState,
        successor: Option<ErasedValue>,
    ) -> Self {
        Self {
            runtime,
            owner,
            run,
            reduced,
            successor,
        }
    }

    /// Returns the run identity retained by this conclusion owner.
    pub fn run_id(&self) -> &RunId {
        self.run.run_id()
    }

    async fn resolve(self) -> RuntimeStep {
        let Self {
            runtime,
            owner,
            run,
            reduced,
            successor,
        } = self;
        match runtime.inner.store.commit_conclusion(owner).await {
            Ok(ConclusionCommitOutcome::AcknowledgementUnknown(owner)) => RuntimeStep::Suspended(
                SuspendedRun::conclusion(runtime, owner, run, reduced, successor),
            ),
            Ok(ConclusionCommitOutcome::Rejected { owner, .. }) => RuntimeStep::Suspended(
                SuspendedRun::conclusion(runtime, owner, run, reduced, successor),
            ),
            Ok(ConclusionCommitOutcome::Disposition { disposition, frame }) => {
                runtime
                    .finish_conclusion(run, reduced, frame, disposition, successor)
                    .await
            }
            Err(error) => RuntimeStep::Failed {
                history: run,
                error: error.into(),
            },
        }
    }
}

enum AdmissionResolution {
    Same(QualifiedRun),
    Conflict(QualifiedRun),
    Invalid(QualifiedRun),
    Missing,
}

/// Explicit owner retained across an acknowledgement-unknown or unresolved boundary.
pub struct SuspendedRun {
    owner: SuspendedOwner,
}

impl SuspendedRun {
    fn admission(runtime: Runtime, frame: RunFrame) -> Self {
        Self {
            owner: SuspendedOwner::Admission { runtime, frame },
        }
    }

    fn conclusion(
        runtime: Runtime,
        owner: mfm_store::single_trust::PreparedConclusion,
        run: QualifiedRun,
        reduced: ReducedRunState,
        successor: Option<ErasedValue>,
    ) -> Self {
        Self {
            owner: SuspendedOwner::Conclusion(PendingConclusion::new(
                runtime, owner, run, reduced, successor,
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn preparation(
        runtime: Runtime,
        prepared: Box<dyn DynamicPrepared>,
        run: QualifiedRun,
        reduced: ReducedRunState,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        input_ref: ValueRef,
        intent_ref: ValueRef,
        maximum_conclusion_bytes: u64,
        preparation_ordinal: u16,
        replaces: Option<mfm_journal::single_trust::PreparationRef>,
        disposition: Option<AppendDisposition>,
    ) -> Self {
        Self {
            owner: SuspendedOwner::Preparation {
                runtime,
                prepared,
                run,
                reduced,
                occurrence,
                expected_sequence,
                append_request_id,
                input_ref,
                intent_ref,
                maximum_conclusion_bytes,
                preparation_ordinal,
                replaces,
                disposition,
            },
        }
    }

    /// Returns the run identity retained by this owner-fate boundary.
    pub fn run_id(&self) -> &RunId {
        match &self.owner {
            SuspendedOwner::Admission { frame, .. } => frame.run_id(),
            SuspendedOwner::Conclusion(owner) => owner.run_id(),
            SuspendedOwner::Preparation { run, .. } => run.run_id(),
        }
    }

    /// Resolves the retained owner exactly once.
    pub async fn resolve(self) -> RuntimeStep {
        match self.owner {
            SuspendedOwner::Admission { runtime, frame } => {
                match runtime.inner.store.append_admission(frame.clone()).await {
                    Ok(AppendDisposition::NewlyCommitted { .. })
                    | Ok(AppendDisposition::Found { .. })
                    | Ok(AppendDisposition::StaleHead { .. }) => {
                        let resolution = runtime.inspect_admission(&frame).await;
                        runtime.resolve_admission(resolution, frame).await
                    }
                    Ok(AppendDisposition::AcknowledgementUnknown) => {
                        RuntimeStep::Suspended(SuspendedRun::admission(runtime, frame))
                    }
                    Err(_) => RuntimeStep::Suspended(SuspendedRun::admission(runtime, frame)),
                }
            }
            SuspendedOwner::Conclusion(owner) => owner.resolve().await,
            SuspendedOwner::Preparation {
                runtime,
                prepared,
                run,
                reduced,
                occurrence,
                expected_sequence,
                append_request_id,
                input_ref,
                intent_ref,
                maximum_conclusion_bytes,
                preparation_ordinal,
                replaces,
                disposition,
            } => {
                if matches!(
                    disposition,
                    Some(disposition) if !matches!(disposition, AppendDisposition::AcknowledgementUnknown)
                ) {
                    return match runtime.inner.store.load(run.run_id()).await {
                        Ok(latest) => runtime.reloaded_step(latest).await,
                        Err(_) => RuntimeStep::Suspended(SuspendedRun::preparation(
                            runtime,
                            prepared,
                            run,
                            reduced,
                            occurrence,
                            expected_sequence,
                            append_request_id,
                            input_ref,
                            intent_ref,
                            maximum_conclusion_bytes,
                            preparation_ordinal,
                            replaces,
                            disposition,
                        )),
                    };
                }
                match prepared
                    .commit(
                        Arc::clone(&runtime.inner.assembly),
                        Arc::clone(&runtime.inner.store),
                        run.clone(),
                        reduced.clone(),
                        expected_sequence,
                        append_request_id.clone(),
                        input_ref.clone(),
                        intent_ref.clone(),
                        maximum_conclusion_bytes,
                        preparation_ordinal,
                        replaces.clone(),
                    )
                    .await
                {
                    DynamicCommit::Direct { call, run, reduced } => {
                        runtime
                            .execute_committed_access(run, occurrence, call, reduced)
                            .await
                    }
                    DynamicCommit::Retained {
                        prepared,
                        disposition,
                        reduced,
                    } => RuntimeStep::Suspended(SuspendedRun::preparation(
                        runtime,
                        prepared,
                        run,
                        reduced,
                        occurrence,
                        expected_sequence,
                        append_request_id,
                        input_ref,
                        intent_ref,
                        maximum_conclusion_bytes,
                        preparation_ordinal,
                        replaces,
                        disposition,
                    )),
                }
            }
        }
    }
}

/// A non-Clone affine Runtime session retaining one latest cumulative context.
pub struct RunSession {
    runtime: Runtime,
    run: QualifiedRun,
    reduced: ReducedRunState,
    latest: ErasedValue,
    _active_permit: OwnedSemaphorePermit,
}

/// Result of one admission owner transition.
#[allow(clippy::large_enum_variant)]
pub enum SpawnStep {
    /// The admitted run has a live sequential session.
    Active(RunSession),
    /// The admitted run is already terminal, including valid zero-state admission.
    Terminal(TerminalRun),
    /// The physical admission acknowledgement must be resolved by the same owner.
    Suspended(SuspendedRun),
    /// A different semantic genesis is already durable.
    Conflict(AdmissionConflict),
    /// The admission failed before live execution authority existed.
    Failed(AdmissionFailure),
}

/// Result of one cold Runtime resume.
pub enum ResumeStep {
    /// The retained prefix has one executable selected action.
    Active(RunSession),
    /// The retained prefix is terminal.
    Terminal(TerminalRun),
    /// The retained prefix has a durable preparation or unresolved provider result.
    Parked(ParkedRun),
    /// The retained prefix cannot be resumed under this Runtime.
    Failed(ResumeFailure),
}

/// Result of one consumed session drive.
#[allow(clippy::large_enum_variant)]
pub enum RuntimeStep {
    /// A durable conclusion advanced the session to its next action.
    Advanced(RunSession),
    /// A durable qualified terminal conclusion is available.
    Terminal(TerminalRun),
    /// Preparation was rejected before a provider call existed.
    PreparationRejected {
        /// The exact session owner retained for a later explicit drive.
        session: RunSession,
        /// Redacted preparation failure.
        error: RuntimeError,
    },
    /// Provider entry did not yield a conclusive result.
    Unresolved {
        /// The exact session whose selected preparation remains durable.
        session: RunSession,
        /// Stable unresolved classification.
        classification: UnresolvedClassification,
    },
    /// The session is waiting for an explicit owner action.
    Parked {
        /// The exact retained session.
        session: RunSession,
        /// Stable parking reason.
        reason: ParkReason,
    },
    /// A live owner retains a physical acknowledgement or conclusion boundary.
    Suspended(SuspendedRun),
    /// The consumed session raced with a different semantic head.
    Conflict {
        /// Qualified callback-free history at the competing head.
        history: QualifiedRun,
        /// Redacted conflict classification.
        error: RuntimeError,
    },
    /// The consumed owner reached a terminal operational failure with history retained.
    Failed {
        /// Qualified callback-free history available for diagnosis/replay.
        history: QualifiedRun,
        /// Redacted terminal failure.
        error: RuntimeError,
    },
}

/// Immutable process Runtime over one exact assembly and opened Store.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

/// Fixed process-local Runtime work limits.
///
/// These limits bound independent owners and deterministic work without introducing a scheduler,
/// per-run lock, or process-wide writer lease.  Provider futures use the ingress bound; pure,
/// preparation, qualification, and interpretation work use the corresponding deterministic
/// bounds.  A session holds one active-session permit for its entire affine lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    max_active_sessions: usize,
    max_cpu_jobs: usize,
    max_planning_jobs: usize,
    max_ingress_jobs: usize,
}

impl RuntimeLimits {
    /// Creates one non-zero bounded Runtime envelope.
    pub const fn new(
        max_active_sessions: usize,
        max_cpu_jobs: usize,
        max_planning_jobs: usize,
        max_ingress_jobs: usize,
    ) -> Self {
        Self {
            max_active_sessions,
            max_cpu_jobs,
            max_planning_jobs,
            max_ingress_jobs,
        }
    }

    /// Returns the selected default envelope for the current MVP entry points.
    pub const fn default_envelope() -> Self {
        Self::new(64, 8, 8, 64)
    }

    /// Validates that every Runtime work lane has at least one permit.
    pub const fn validate(self) -> LifecycleResult<()> {
        if self.max_active_sessions == 0
            || self.max_cpu_jobs == 0
            || self.max_planning_jobs == 0
            || self.max_ingress_jobs == 0
        {
            Err(RuntimeError::Capacity)
        } else {
            Ok(())
        }
    }

    /// Returns the active-session bound.
    pub const fn max_active_sessions(self) -> usize {
        self.max_active_sessions
    }

    /// Returns the deterministic CPU-job bound.
    pub const fn max_cpu_jobs(self) -> usize {
        self.max_cpu_jobs
    }

    /// Returns the planning-job bound.
    pub const fn max_planning_jobs(self) -> usize {
        self.max_planning_jobs
    }

    /// Returns the provider-ingress bound.
    pub const fn max_ingress_jobs(self) -> usize {
        self.max_ingress_jobs
    }
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self::default_envelope()
    }
}

struct RuntimeInner {
    assembly: Arc<RuntimeAssembly>,
    store: Arc<OpenedStructuredStore>,
    witness: Arc<RuntimeWitness>,
    limits: RuntimeLimits,
    active_sessions: Arc<Semaphore>,
    cpu_jobs: Arc<Semaphore>,
    planning_jobs: Arc<Semaphore>,
    ingress_jobs: Arc<Semaphore>,
}

impl Runtime {
    /// Opens one Runtime over an exact immutable assembly and branded Store.
    pub fn new(assembly: RuntimeAssembly, store: OpenedStructuredStore) -> LifecycleResult<Self> {
        Self::new_with_limits(assembly, store, RuntimeLimits::default())
    }

    /// Opens one Runtime with one exact immutable assembly, Store, and bounded work envelope.
    pub fn new_with_limits(
        assembly: RuntimeAssembly,
        store: OpenedStructuredStore,
        limits: RuntimeLimits,
    ) -> LifecycleResult<Self> {
        limits.validate()?;
        if !assembly.program().belongs_to_catalog(store.catalog()) {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                assembly: Arc::new(assembly),
                store: Arc::new(store),
                witness: Arc::new(RuntimeWitness),
                limits,
                active_sessions: Arc::new(Semaphore::new(limits.max_active_sessions)),
                cpu_jobs: Arc::new(Semaphore::new(limits.max_cpu_jobs)),
                planning_jobs: Arc::new(Semaphore::new(limits.max_planning_jobs)),
                ingress_jobs: Arc::new(Semaphore::new(limits.max_ingress_jobs)),
            }),
        })
    }

    /// Returns this Runtime's immutable bounded work envelope.
    pub fn limits(&self) -> RuntimeLimits {
        self.inner.limits
    }

    async fn acquire(
        semaphore: &Arc<Semaphore>,
    ) -> std::result::Result<OwnedSemaphorePermit, RuntimeError> {
        semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::Capacity)
    }

    async fn acquire_active_session(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.active_sessions).await
    }

    async fn acquire_cpu_job(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.cpu_jobs).await
    }

    async fn acquire_planning_job(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.planning_jobs).await
    }

    async fn acquire_ingress_job(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.ingress_jobs).await
    }

    /// Creates one typed admission owner from a planning-owned singular `C0`.
    pub fn admission<T: MfmValue>(
        &self,
        run_id: RunId,
        value: QualifiedTypedValue<T>,
        configuration_ref: ContentRef,
        source_refs: Vec<ContentRef>,
        append_request_id: AppendRequestId,
    ) -> LifecycleResult<AdmissionInput<T>> {
        AdmissionInput::new(
            self,
            run_id,
            value,
            configuration_ref,
            source_refs,
            append_request_id,
        )
    }

    /// Resumes one exact run after cold callback-free prefix qualification.
    pub async fn resume<T: MfmValue>(&self, input: ResumeInput<T>) -> ResumeStep {
        if !Arc::ptr_eq(&self.inner, &input.runtime.inner) {
            return ResumeStep::Failed(ResumeFailure::Identity);
        }
        let run = match self.inner.store.load(&input.run_id).await {
            Ok(run) => run,
            Err(_) => return ResumeStep::Failed(ResumeFailure::History),
        };
        match self.session_from_run(run, Some(input.value)).await {
            Ok(session) => self.classify_resume_session(session).await,
            Err(RuntimeError::Capacity) => ResumeStep::Failed(ResumeFailure::Capacity),
            Err(_) => ResumeStep::Failed(ResumeFailure::Identity),
        }
    }

    /// Resumes one run by cold-qualified retained context owned by this Runtime.
    ///
    /// This is the process-facing entry point used by transports that retain only a run id.  It
    /// performs the same bounded prefix qualification as typed [`Runtime::resume`] and never
    /// exposes the retained history to State code.
    pub async fn resume_run(&self, run_id: RunId) -> ResumeStep {
        let run = match self.inner.store.load(&run_id).await {
            Ok(run) => run,
            Err(_) => return ResumeStep::Failed(ResumeFailure::History),
        };
        match self.session_from_run(run, None).await {
            Ok(session) => self.classify_resume_session(session).await,
            Err(RuntimeError::Capacity) => ResumeStep::Failed(ResumeFailure::Capacity),
            Err(_) => ResumeStep::Failed(ResumeFailure::Identity),
        }
    }

    /// Returns a clone of the exact Store opening owned by this Runtime.
    pub fn store(&self) -> OpenedStructuredStore {
        (*self.inner.store).clone()
    }

    /// Returns the exact callback-free Program catalog associated with this Runtime.
    pub fn catalog(&self) -> ProgramCatalog {
        self.inner.assembly.catalog().clone()
    }

    /// Returns the immutable entry-point identity of this Runtime's Program.
    pub fn entry_point_id(&self) -> StableId {
        self.inner
            .assembly
            .program()
            .document()
            .entry_point_id()
            .clone()
    }

    async fn classify_resume_session(&self, session: RunSession) -> ResumeStep {
        match session.reduced.action().clone() {
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => ResumeStep::Terminal(TerminalRun::new(session.run)),
            RunAction::WaitingPreparation { .. } => ResumeStep::Parked(ParkedRun {
                session,
                reason: ParkReason::WaitingPreparation,
            }),
            _ => ResumeStep::Active(session),
        }
    }

    async fn reloaded_step(&self, run: QualifiedRun) -> RuntimeStep {
        let history = run.clone();
        let session = match self.session_from_run(run, None).await {
            Ok(session) => session,
            Err(error) => return RuntimeStep::Failed { history, error },
        };
        match session.reduced.action().clone() {
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => RuntimeStep::Terminal(TerminalRun::new(session.run)),
            RunAction::WaitingPreparation { .. } => RuntimeStep::Parked {
                session,
                reason: ParkReason::WaitingPreparation,
            },
            _ => RuntimeStep::Advanced(session),
        }
    }

    async fn execute_committed_access(
        &self,
        run: QualifiedRun,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        call: Box<dyn DynamicCall>,
        reduced: ReducedRunState,
    ) -> RuntimeStep {
        let state = match self
            .inner
            .assembly
            .program()
            .document()
            .declaration(&occurrence)
        {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return RuntimeStep::Failed {
                    history: run,
                    error: RuntimeError::Identity,
                }
            }
        };
        let resolution = {
            let _ingress_permit = match self.acquire_ingress_job().await {
                Ok(permit) => permit,
                Err(error) => {
                    return RuntimeStep::Failed {
                        history: run,
                        error,
                    }
                }
            };
            let _cpu_permit = match self.acquire_cpu_job().await {
                Ok(permit) => permit,
                Err(error) => {
                    return RuntimeStep::Failed {
                        history: run,
                        error,
                    }
                }
            };
            call.execute(Arc::clone(&self.inner.assembly), &self.inner.witness)
                .await
                .ok()
        };
        let Some(resolution) = resolution else {
            return self
                .neutral_access(
                    run,
                    reduced.clone(),
                    UnresolvedClassification::AcknowledgementUnknown,
                )
                .await;
        };
        let conclusion_sequence = resolution.preparation.run_sequence();
        if let Some(classification) = resolution.classification {
            let history = run.clone();
            return match self
                .session_from_reduced(run, reduced, Some(resolution.input))
                .await
            {
                Ok(session) => RuntimeStep::Unresolved {
                    session,
                    classification,
                },
                Err(error) => RuntimeStep::Failed { history, error },
            };
        }
        let evidence = match resolution.evidence {
            Some(evidence) => evidence,
            None => {
                return self
                    .neutral_access(
                        run,
                        reduced.clone(),
                        UnresolvedClassification::InvalidResponse,
                    )
                    .await;
            }
        };
        let outcome = match resolution.outcome {
            Some(outcome) => outcome,
            None => {
                return self
                    .neutral_access(
                        run,
                        reduced.clone(),
                        UnresolvedClassification::InvalidResponse,
                    )
                    .await;
            }
        };
        let evidence_ref = evidence.as_value_ref();
        let intent_object = match resolution.intent.object() {
            Ok(object) => object,
            Err(_) => {
                return self
                    .neutral_access(
                        run,
                        reduced.clone(),
                        UnresolvedClassification::InvalidResponse,
                    )
                    .await;
            }
        };
        let evidence_object = match evidence.object() {
            Ok(object) => object,
            Err(_) => {
                return self
                    .neutral_access(
                        run,
                        reduced.clone(),
                        UnresolvedClassification::InvalidResponse,
                    )
                    .await;
            }
        };
        let (recorded_outcome, successor, outcome_object, fact_proposals) = match outcome {
            DynamicOutcome::Success { value, facts } => {
                let value_ref = value.as_value_ref();
                let object = match value.object() {
                    Ok(object) => object,
                    Err(_) => {
                        return self
                            .neutral_access(
                                run,
                                reduced.clone(),
                                UnresolvedClassification::InvalidResponse,
                            )
                            .await;
                    }
                };
                let fact_proposals = match fact_proposals_object(&facts) {
                    Ok(value) => Some(value),
                    Err(_) => {
                        return self
                            .neutral_access(
                                run,
                                reduced.clone(),
                                UnresolvedClassification::InvalidResponse,
                            )
                            .await;
                    }
                };
                (
                    StateOutcome::Success(value_ref),
                    Some(value),
                    object,
                    fact_proposals,
                )
            }
            DynamicOutcome::Failure(value) => {
                let value_ref = value.as_value_ref();
                let object = match value.object() {
                    Ok(object) => object,
                    Err(_) => {
                        return self
                            .neutral_access(
                                run,
                                reduced.clone(),
                                UnresolvedClassification::InvalidResponse,
                            )
                            .await;
                    }
                };
                (StateOutcome::Failure(value_ref), None, object, None)
            }
        };
        let conclusion_id = match conclusion_append_id(run.run_id(), conclusion_sequence) {
            Ok(id) => id,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                }
            }
        };
        let owner = match self.inner.store.prepare_conclusion_qualified_with_reduced(
            &run,
            self.inner.assembly.program().document(),
            &reduced,
            conclusion_sequence,
            conclusion_id,
            StateConcluded::Access {
                occurrence,
                preparation: resolution.preparation,
                evidence: evidence_ref,
                outcome: recorded_outcome,
                fact_proposals: fact_proposals.as_ref().map(|(value, _)| value.clone()),
                fact_selection: resolution
                    .fact_continuation
                    .as_ref()
                    .map(mfm_store::FactContinuation::selection_ref)
                    .cloned(),
                fact_publication: None,
            },
            {
                let mut objects = vec![intent_object, evidence_object, outcome_object];
                if let Some((_, object)) = fact_proposals {
                    objects.push(object);
                }
                objects
            },
            state.maximum_conclusion_bytes(),
        ) {
            Ok(owner) => owner,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error: error.into(),
                }
            }
        };
        match self.inner.store.commit_conclusion(owner).await {
            Ok(ConclusionCommitOutcome::AcknowledgementUnknown(owner)) => {
                RuntimeStep::Suspended(SuspendedRun::conclusion(
                    self.clone(),
                    owner,
                    run.clone(),
                    reduced.clone(),
                    successor,
                ))
            }
            Ok(ConclusionCommitOutcome::Rejected { owner, .. }) => {
                RuntimeStep::Suspended(SuspendedRun::conclusion(
                    self.clone(),
                    owner,
                    run.clone(),
                    reduced.clone(),
                    successor,
                ))
            }
            Ok(ConclusionCommitOutcome::Disposition { disposition, frame }) => {
                self.finish_conclusion(run, reduced, frame, disposition, successor)
                    .await
            }
            Err(error) => RuntimeStep::Failed {
                history: run,
                error: error.into(),
            },
        }
    }

    async fn finish_conclusion(
        &self,
        previous: QualifiedRun,
        reduced: ReducedRunState,
        conclusion_frame: RunFrame,
        disposition: AppendDisposition,
        successor: Option<ErasedValue>,
    ) -> RuntimeStep {
        match disposition {
            AppendDisposition::NewlyCommitted { .. } | AppendDisposition::Found { .. } => {
                let _cpu_permit = match self.acquire_cpu_job().await {
                    Ok(permit) => permit,
                    Err(error) => {
                        return RuntimeStep::Failed {
                            history: previous,
                            error,
                        }
                    }
                };
                let next_run = match self
                    .inner
                    .store
                    .qualify_appended(&previous, conclusion_frame)
                {
                    Ok(next_run) => next_run,
                    Err(error) => {
                        return RuntimeStep::Failed {
                            history: previous,
                            error: error.into(),
                        }
                    }
                };
                let next_reduced = match self.inner.store.advance_reduced(
                    &reduced,
                    &previous,
                    &next_run,
                    self.inner.assembly.program().document(),
                ) {
                    Ok(reduced) => reduced,
                    Err(error) => {
                        return RuntimeStep::Failed {
                            history: previous,
                            error: error.into(),
                        }
                    }
                };
                drop(_cpu_permit);
                match successor {
                    Some(successor) => match self
                        .session_from_reduced(next_run, next_reduced, Some(successor))
                        .await
                    {
                        Ok(session) => match session.reduced.action().clone() {
                            RunAction::ZeroStateTerminal { .. }
                            | RunAction::Terminal { .. }
                            | RunAction::Failed { .. } => {
                                RuntimeStep::Terminal(TerminalRun::new(session.run))
                            }
                            _ => RuntimeStep::Advanced(session),
                        },
                        Err(error) => RuntimeStep::Failed {
                            history: previous,
                            error,
                        },
                    },
                    None => match next_reduced.action() {
                        RunAction::ZeroStateTerminal { .. }
                        | RunAction::Terminal { .. }
                        | RunAction::Failed { .. } => {
                            RuntimeStep::Terminal(TerminalRun::new(next_run))
                        }
                        _ => RuntimeStep::Failed {
                            history: next_run,
                            error: RuntimeError::Conclusion,
                        },
                    },
                }
            }
            AppendDisposition::AcknowledgementUnknown => RuntimeStep::Failed {
                history: previous,
                error: RuntimeError::Unresolved,
            },
            AppendDisposition::StaleHead { .. } => RuntimeStep::Conflict {
                history: previous,
                error: RuntimeError::Conclusion,
            },
        }
    }

    async fn neutral_access(
        &self,
        run: QualifiedRun,
        reduced: ReducedRunState,
        classification: UnresolvedClassification,
    ) -> RuntimeStep {
        let history = run.clone();
        match self.session_from_reduced(run, reduced, None).await {
            Ok(session) => RuntimeStep::Unresolved {
                session,
                classification,
            },
            Err(error) => RuntimeStep::Failed { history, error },
        }
    }

    async fn spawn_erased(
        &self,
        run_id: RunId,
        value: ErasedValue,
        configuration_ref: ContentRef,
        source_refs: Vec<ContentRef>,
        append_request_id: AppendRequestId,
    ) -> SpawnStep {
        let _planning_permit = match self.acquire_planning_job().await {
            Ok(permit) => permit,
            Err(_) => return SpawnStep::Failed(AdmissionFailure::Capacity),
        };
        let admission = match RunAdmitted::new(
            self.inner.store.identity().scope().clone(),
            self.inner.store.identity().epoch(),
            run_id.clone(),
            self.inner.store.identity().tenant().clone(),
            self.inner
                .assembly
                .program()
                .document()
                .entry_point_id()
                .clone(),
            self.inner.assembly.program_ref().clone(),
            value.as_value_ref(),
            configuration_ref,
            source_refs,
        ) {
            Ok(admission) => admission,
            Err(_) => return SpawnStep::Failed(AdmissionFailure::Identity),
        };
        let object = match value.object() {
            Ok(object) => object,
            Err(_) => return SpawnStep::Failed(AdmissionFailure::Identity),
        };
        let frame = match RunFrame::new(
            run_id.clone(),
            self.inner.store.identity().scope().clone(),
            self.inner.store.identity().epoch(),
            1,
            append_request_id,
            RunRecord::RunAdmitted(admission),
            vec![object],
        ) {
            Ok(frame) => frame,
            Err(_) => return SpawnStep::Failed(AdmissionFailure::Identity),
        };
        match self.inner.store.append_admission(frame.clone()).await {
            Ok(AppendDisposition::NewlyCommitted { .. }) => {
                let run = match self.inner.store.qualify_admission(frame) {
                    Ok(run) => run,
                    Err(_) => return SpawnStep::Failed(AdmissionFailure::Store),
                };
                match self.session_from_run(run, Some(value)).await {
                    Ok(session) => self.spawn_classify(session).await,
                    Err(_) => SpawnStep::Failed(AdmissionFailure::Store),
                }
            }
            Ok(AppendDisposition::Found { .. }) => {
                let resolution = self.inspect_admission(&frame).await;
                self.spawn_admission_resolution(resolution).await
            }
            Ok(AppendDisposition::AcknowledgementUnknown) => {
                SpawnStep::Suspended(SuspendedRun::admission(self.clone(), frame))
            }
            Ok(AppendDisposition::StaleHead { .. }) => {
                let resolution = self.inspect_admission(&frame).await;
                self.spawn_admission_resolution(resolution).await
            }
            Err(_) => SpawnStep::Suspended(SuspendedRun::admission(self.clone(), frame)),
        }
    }

    async fn spawn_classify(&self, session: RunSession) -> SpawnStep {
        match session.reduced.action().clone() {
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => SpawnStep::Terminal(TerminalRun::new(session.run)),
            _ => SpawnStep::Active(session),
        }
    }

    async fn inspect_admission(&self, frame: &RunFrame) -> AdmissionResolution {
        let run = match self.inner.store.load(frame.run_id()).await {
            Ok(run) => run,
            Err(_) => return AdmissionResolution::Missing,
        };
        let RunRecord::RunAdmitted(candidate) = frame.record() else {
            return AdmissionResolution::Invalid(run);
        };
        let Some(RunRecord::RunAdmitted(existing)) =
            run.frames().first().map(|retained| retained.record())
        else {
            return AdmissionResolution::Invalid(run);
        };
        if candidate == existing {
            AdmissionResolution::Same(run)
        } else {
            AdmissionResolution::Conflict(run)
        }
    }

    async fn spawn_admission_resolution(&self, resolution: AdmissionResolution) -> SpawnStep {
        match resolution {
            AdmissionResolution::Same(run) => match self.session_from_run(run, None).await {
                Ok(session) => self.spawn_classify(session).await,
                Err(_) => SpawnStep::Failed(AdmissionFailure::Store),
            },
            AdmissionResolution::Conflict(_) => SpawnStep::Conflict(AdmissionConflict),
            AdmissionResolution::Invalid(_) | AdmissionResolution::Missing => {
                SpawnStep::Failed(AdmissionFailure::Store)
            }
        }
    }

    async fn resolve_admission(
        &self,
        resolution: AdmissionResolution,
        frame: RunFrame,
    ) -> RuntimeStep {
        match resolution {
            AdmissionResolution::Same(run) => {
                let history = run.clone();
                match self.session_from_run(run, None).await {
                    Ok(session) => self.classify_runtime_session(session).await,
                    Err(error) => RuntimeStep::Failed { history, error },
                }
            }
            AdmissionResolution::Conflict(history) => RuntimeStep::Conflict {
                history,
                error: RuntimeError::Identity,
            },
            AdmissionResolution::Invalid(history) => RuntimeStep::Failed {
                history,
                error: RuntimeError::Identity,
            },
            AdmissionResolution::Missing => {
                RuntimeStep::Suspended(SuspendedRun::admission(self.clone(), frame))
            }
        }
    }

    async fn classify_runtime_session(&self, session: RunSession) -> RuntimeStep {
        match session.reduced.action().clone() {
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => RuntimeStep::Terminal(TerminalRun::new(session.run)),
            RunAction::WaitingPreparation { .. } => RuntimeStep::Parked {
                session,
                reason: ParkReason::WaitingPreparation,
            },
            _ => RuntimeStep::Advanced(session),
        }
    }

    async fn session_from_run(
        &self,
        run: QualifiedRun,
        supplied: Option<ErasedValue>,
    ) -> LifecycleResult<RunSession> {
        let cpu_permit = self.acquire_cpu_job().await?;
        let store = Arc::clone(&self.inner.store);
        let reduction_run = run.clone();
        let document = self.inner.assembly.program().document().clone();
        let reduced = tokio::task::spawn_blocking(move || {
            let _cpu_permit = cpu_permit;
            store
                .reduce_qualified(&reduction_run, document)
                .map_err(|_| RuntimeError::Conclusion)
        })
        .await
        .map_err(|_| RuntimeError::Conclusion)??;
        self.session_from_reduced(run, reduced, supplied).await
    }

    async fn session_from_reduced(
        &self,
        run: QualifiedRun,
        reduced: ReducedRunState,
        supplied: Option<ErasedValue>,
    ) -> LifecycleResult<RunSession> {
        let latest = if let Some(value) = supplied {
            value
        } else {
            let object = run
                .frames()
                .iter()
                .flat_map(|frame| frame.objects())
                .find(|object| object.content_ref() == reduced.latest_context().value_ref())
                .ok_or(RuntimeError::Conclusion)?;
            let assembly = Arc::clone(&self.inner.assembly);
            let witness = Arc::clone(&self.inner.witness);
            let contract = reduced.latest_context().contract_ref().clone();
            let canonical_bytes = object.canonical_json().as_bytes().to_vec();
            let cpu_permit = self.acquire_cpu_job().await?;
            tokio::task::spawn_blocking(move || {
                let _cpu_permit = cpu_permit;
                assembly.reify_value(&witness, &contract, &canonical_bytes)
            })
            .await
            .map_err(|_| RuntimeError::Value)??
        };
        if latest.value_ref() != reduced.latest_context().value_ref()
            || latest.contract_ref() != reduced.latest_context().contract_ref()
        {
            #[cfg(test)]
            eprintln!(
                "latest mismatch supplied={:?} reduced={:?} supplied_contract={:?} reduced_contract={:?}",
                latest.value_ref(),
                reduced.latest_context().value_ref(),
                latest.contract_ref(),
                reduced.latest_context().contract_ref(),
            );
            return Err(RuntimeError::Identity);
        }
        Ok(RunSession {
            runtime: self.clone(),
            run,
            reduced,
            latest,
            _active_permit: self.acquire_active_session().await?,
        })
    }
}

impl RunSession {
    /// Returns the durable run identity retained by this affine session.
    pub fn run_id(&self) -> &RunId {
        self.run.run_id()
    }

    /// Returns the qualified durable head retained by this affine session.
    pub fn head_sequence(&self) -> u64 {
        self.run.head_sequence()
    }

    #[allow(clippy::result_large_err)]
    async fn drive_access(
        self,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
    ) -> RuntimeStep {
        let RunSession {
            runtime,
            run,
            reduced,
            latest,
            _active_permit,
        } = self;
        let state = match runtime
            .inner
            .assembly
            .program()
            .document()
            .declaration(&occurrence)
        {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return RuntimeStep::Failed {
                    history: run,
                    error: RuntimeError::Identity,
                }
            }
        };
        let binding_ref = match state.execution_binding_ref() {
            Some(binding_ref) => binding_ref.clone(),
            None => {
                return RuntimeStep::Failed {
                    history: run,
                    error: RuntimeError::Mode,
                }
            }
        };
        let maximum_conclusion_bytes = state.maximum_conclusion_bytes();
        let input_ref = latest.as_value_ref();
        let registration = match runtime
            .inner
            .assembly
            .dynamic_registration(state.state_implementation_ref())
        {
            Ok(registration) => registration,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                }
            }
        };
        let _planning_permit = match runtime.acquire_planning_job().await {
            Ok(permit) => permit,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                }
            }
        };
        let assembly = Arc::clone(&runtime.inner.assembly);
        let witness = Arc::clone(&runtime.inner.witness);
        let run_id = run.run_id().clone();
        let prepare_occurrence = occurrence.clone();
        let prepared_result = match tokio::task::spawn_blocking(move || {
            let _planning_permit = _planning_permit;
            registration.prepare_access(
                &assembly,
                &witness,
                run_id,
                prepare_occurrence,
                latest,
                binding_ref,
            )
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err(DynamicPreparationFailure {
                input: None,
                error: RuntimeError::Preparation,
            }),
        };
        let prepared = match prepared_result {
            Ok(prepared) => prepared,
            Err(failure) => {
                if let Some(input) = failure.input {
                    return RuntimeStep::PreparationRejected {
                        session: RunSession {
                            runtime,
                            run,
                            reduced,
                            latest: input,
                            _active_permit,
                        },
                        error: failure.error,
                    };
                }
                return RuntimeStep::Failed {
                    history: run,
                    error: failure.error,
                };
            }
        };
        drop(_active_permit);
        let intent_ref = match prepared.intent_ref() {
            Ok(intent_ref) => intent_ref,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                }
            }
        };
        let preparation_id = match AppendRequestId::new(format!(
            "runtime-preparation-{}-{}",
            short_stable_id_fragment(run.run_id().as_str(), 96),
            run.head_sequence() + 1
        )) {
            Ok(id) => id,
            Err(_) => {
                return RuntimeStep::Failed {
                    history: run,
                    error: RuntimeError::Identity,
                }
            }
        };
        let expected_sequence = run.head_sequence();
        let committed = prepared
            .commit(
                Arc::clone(&runtime.inner.assembly),
                Arc::clone(&runtime.inner.store),
                run.clone(),
                reduced,
                expected_sequence,
                preparation_id.clone(),
                input_ref.clone(),
                intent_ref.clone(),
                maximum_conclusion_bytes,
                0,
                None,
            )
            .await;
        match committed {
            DynamicCommit::Direct {
                call,
                run: committed_run,
                reduced,
            } => {
                return runtime
                    .execute_committed_access(committed_run, occurrence, call, reduced)
                    .await;
            }
            DynamicCommit::Retained {
                prepared,
                disposition,
                reduced,
            } => RuntimeStep::Suspended(SuspendedRun::preparation(
                runtime,
                prepared,
                run,
                reduced,
                occurrence,
                expected_sequence,
                preparation_id,
                input_ref,
                intent_ref,
                maximum_conclusion_bytes,
                0,
                None,
                disposition,
            )),
        }
    }

    async fn drive_pure(
        self,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
    ) -> RuntimeStep {
        let RunSession {
            runtime,
            run,
            reduced,
            latest,
            _active_permit,
        } = self;
        let state = match runtime
            .inner
            .assembly
            .program()
            .document()
            .declaration(&occurrence)
        {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return RuntimeStep::Failed {
                    history: run,
                    error: RuntimeError::Identity,
                }
            }
        };
        let registration = match runtime
            .inner
            .assembly
            .dynamic_registration(state.state_implementation_ref())
        {
            Ok(registration) => registration,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                };
            }
        };
        let input_contract = state.input_contract_ref().clone();
        let output_contract = state.output_contract_ref().clone();
        let failure_contract = state.failure_contract_ref().cloned();
        let assembly = Arc::clone(&runtime.inner.assembly);
        let witness = Arc::clone(&runtime.inner.witness);
        let cpu_permit = match runtime.acquire_cpu_job().await {
            Ok(permit) => permit,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                }
            }
        };
        let outcome = match tokio::task::spawn_blocking(move || {
            let _cpu_permit = cpu_permit;
            registration.pure_evaluate(
                &assembly,
                &witness,
                latest,
                &input_contract,
                &output_contract,
                failure_contract.as_ref(),
            )
        })
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                };
            }
            Err(_) => {
                return RuntimeStep::Failed {
                    history: run,
                    error: RuntimeError::Unresolved,
                };
            }
        };
        let (recorded, successor, object, fact_proposals) = match outcome {
            DynamicOutcome::Success { value, facts } => {
                let value_ref = value.as_value_ref();
                let object = match value.object() {
                    Ok(object) => object,
                    Err(error) => {
                        return RuntimeStep::Failed {
                            history: run,
                            error,
                        }
                    }
                };
                let fact_proposals = match fact_proposals_object(&facts) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        return RuntimeStep::Failed {
                            history: run,
                            error,
                        }
                    }
                };
                (
                    StateOutcome::Success(value_ref),
                    Some(value),
                    object,
                    fact_proposals,
                )
            }
            DynamicOutcome::Failure(value) => {
                let value_ref = value.as_value_ref();
                let object = match value.object() {
                    Ok(object) => object,
                    Err(error) => {
                        return RuntimeStep::Failed {
                            history: run,
                            error,
                        }
                    }
                };
                (StateOutcome::Failure(value_ref), None, object, None)
            }
        };
        drop(_active_permit);
        let append_request_id = match conclusion_append_id(run.run_id(), run.head_sequence()) {
            Ok(value) => value,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error,
                }
            }
        };
        let owner = match runtime
            .inner
            .store
            .prepare_conclusion_qualified_with_reduced(
                &run,
                runtime.inner.assembly.program().document(),
                &reduced,
                run.head_sequence(),
                append_request_id,
                StateConcluded::Pure {
                    occurrence,
                    outcome: recorded,
                    fact_proposals: fact_proposals.as_ref().map(|(value, _)| value.clone()),
                    fact_publication: None,
                },
                {
                    let mut objects = vec![object];
                    if let Some((_, object)) = fact_proposals {
                        objects.push(object);
                    }
                    objects
                },
                state.maximum_conclusion_bytes(),
            ) {
            Ok(owner) => owner,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: run,
                    error: error.into(),
                };
            }
        };
        match runtime.inner.store.commit_conclusion(owner).await {
            Ok(ConclusionCommitOutcome::AcknowledgementUnknown(owner)) => {
                RuntimeStep::Suspended(SuspendedRun::conclusion(
                    runtime.clone(),
                    owner,
                    run.clone(),
                    reduced.clone(),
                    successor,
                ))
            }
            Ok(ConclusionCommitOutcome::Rejected { owner, .. }) => {
                RuntimeStep::Suspended(SuspendedRun::conclusion(
                    runtime.clone(),
                    owner,
                    run.clone(),
                    reduced.clone(),
                    successor,
                ))
            }
            Ok(ConclusionCommitOutcome::Disposition { disposition, frame }) => {
                runtime
                    .finish_conclusion(run, reduced, frame, disposition, successor)
                    .await
            }
            Err(error) => RuntimeStep::Failed {
                history: run,
                error: error.into(),
            },
        }
    }

    /// Consumes this session and performs one deterministic Runtime step.
    pub async fn drive(self) -> RuntimeStep {
        let action = self.reduced.action().clone();
        match action {
            RunAction::ReadyPure { occurrence, .. } => self.drive_pure(occurrence).await,
            RunAction::WaitingPreparation { .. } => RuntimeStep::Parked {
                session: self,
                reason: ParkReason::WaitingPreparation,
            },
            RunAction::ReadyAccess { occurrence, .. } => self.drive_access(occurrence).await,
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => RuntimeStep::Terminal(TerminalRun::new(self.run)),
        }
    }
}

/// Typed singular admission owner.
pub struct AdmissionInput<T: MfmValue> {
    runtime: Runtime,
    run_id: RunId,
    value: QualifiedTypedValue<T>,
    configuration_ref: ContentRef,
    source_refs: Vec<ContentRef>,
    append_request_id: AppendRequestId,
}

impl<T: MfmValue> AdmissionInput<T> {
    /// Mints an admission owner from one catalog-qualified planning value.
    pub fn new(
        runtime: &Runtime,
        run_id: RunId,
        value: QualifiedTypedValue<T>,
        configuration_ref: ContentRef,
        source_refs: Vec<ContentRef>,
        append_request_id: AppendRequestId,
    ) -> LifecycleResult<Self> {
        if !value.belongs_to_catalog(runtime.inner.assembly.catalog())
            || value.contract_ref()
                != runtime
                    .inner
                    .assembly
                    .program()
                    .document()
                    .admitted_context_contract_ref()
        {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            runtime: runtime.clone(),
            run_id,
            value,
            configuration_ref,
            source_refs,
            append_request_id,
        })
    }

    /// Consumes this owner into an exhaustive admission outcome.
    pub async fn spawn(self) -> SpawnStep {
        let value = match ErasedValue::from_qualified(
            self.runtime.inner.assembly.catalog(),
            &self.runtime.inner.witness,
            self.value,
        ) {
            Ok(value) => value,
            Err(_) => return SpawnStep::Failed(AdmissionFailure::Identity),
        };
        self.runtime
            .spawn_erased(
                self.run_id,
                value,
                self.configuration_ref,
                self.source_refs,
                self.append_request_id,
            )
            .await
    }
}

/// Typed cold-resume owner carrying the admitted value needed for exact catalog reification.
pub struct ResumeInput<T: MfmValue> {
    runtime: Runtime,
    run_id: RunId,
    value: ErasedValue,
    _marker: PhantomData<fn() -> T>,
}

impl<T: MfmValue> ResumeInput<T> {
    /// Creates a resume owner from a catalog-qualified admitted value.
    pub fn new(
        runtime: &Runtime,
        run_id: RunId,
        value: QualifiedTypedValue<T>,
    ) -> LifecycleResult<Self> {
        Ok(Self {
            value: ErasedValue::from_qualified(
                runtime.inner.assembly.catalog(),
                &runtime.inner.witness,
                value,
            )?,
            runtime: runtime.clone(),
            run_id,
            _marker: PhantomData,
        })
    }
}

fn conclusion_append_id(run_id: &RunId, sequence: u64) -> LifecycleResult<AppendRequestId> {
    AppendRequestId::new(format!(
        "runtime-conclusion-{}-{}",
        short_stable_id_fragment(run_id.as_str(), 96),
        sequence + 1
    ))
    .map_err(|_| RuntimeError::Identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_capabilities::{AccessCapabilityContract, NoPriorFacts, ReadMode};
    use mfm_program::single_trust::{ExecutionMode, ProgramDocument, StateDeclaration};
    use mfm_program_derive::MfmValue as DeriveMfmValue;
    use mfm_store::{StoreWorkLimits, StructuredStore, StructuredStoreIdentity};
    use serde::{Deserialize, Serialize};
    use std::num::NonZeroU16;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn runtime_limits_require_one_permit_per_work_lane() {
        assert!(RuntimeLimits::default().validate().is_ok());
        assert_eq!(RuntimeLimits::default().max_cpu_jobs(), 8);
        assert!(RuntimeLimits::new(0, 1, 1, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 0, 1, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 1, 0, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 1, 1, 0).validate().is_err());
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, DeriveMfmValue)]
    #[serde(deny_unknown_fields)]
    struct TestContext {
        value: u64,
    }

    impl crate::single_trust::FailureValue for TestContext {
        fn integrity_blocked() -> Self {
            Self { value: 0 }
        }
    }

    struct TestPure;

    impl State for TestPure {
        type Input = TestContext;
        type Output = TestContext;
        type Failure = TestContext;

        fn state_id() -> LifecycleResult<StableId> {
            StableId::new("mfm.test.lifecycle-pure").map_err(|_| RuntimeError::Identity)
        }
    }

    struct TestAccess;

    impl State for TestAccess {
        type Input = TestContext;
        type Output = TestContext;
        type Failure = TestContext;

        fn state_id() -> LifecycleResult<StableId> {
            StableId::new("mfm.test.lifecycle-access").map_err(|_| RuntimeError::Identity)
        }
    }

    struct TestRead;

    impl AccessCapabilityContract for TestRead {
        type Mode = ReadMode;
        type Intent = TestContext;
        type Evidence = TestContext;
        type Facts = NoPriorFacts;

        fn contract_id() -> mfm_capabilities::Result<StableId> {
            StableId::new("mfm.test.lifecycle-read")
                .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
        }

        fn total_attempt_bound() -> NonZeroU16 {
            NonZeroU16::new(1).expect("nonzero")
        }

        fn bind_evidence(
            intent: &Self::Intent,
            evidence: &Self::Evidence,
        ) -> mfm_capabilities::Result<()> {
            (evidence.value == intent.value + 1)
                .then_some(())
                .ok_or(mfm_capabilities::CapabilityError::EvidenceBinding)
        }
    }

    fn test_ref(label: &[u8]) -> ContentRef {
        let schema = TestContext::schema_id().expect("schema");
        ContentRef::new(schema, raw_content_digest(label)).expect("content ref")
    }

    fn test_identity() -> StructuredStoreIdentity {
        StructuredStoreIdentity::new(
            mfm_ids::StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            mfm_ids::StoreEpoch::new(1),
            mfm_ids::TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        )
    }

    fn pure_runtime_fixture() -> (Runtime, ProgramCatalog, ContentRef) {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let first_implementation_ref = test_ref(b"mfm.test.concurrent-pure-first");
        let second_implementation_ref = test_ref(b"mfm.test.concurrent-pure-second");
        let first_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                .expect("occurrence");
        let second_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(1, Vec::new())
                .expect("occurrence");
        let document = ProgramDocument::new(
            StableId::new("mfm.test.concurrent-entry").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::with_next(
                        first_occurrence,
                        first_implementation_ref.clone(),
                        contract.clone(),
                        contract.clone(),
                        None,
                        ExecutionMode::Pure,
                        second_occurrence.clone(),
                    )
                    .expect("state"),
                )),
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::new(
                        second_occurrence,
                        second_implementation_ref.clone(),
                        contract.clone(),
                        contract,
                        None,
                        ExecutionMode::Pure,
                        true,
                    )
                    .expect("state"),
                )),
            ],
        )
        .expect("document");
        let (catalog, program) = ProgramCatalog::builder().finish(document).expect("program");
        let implementation = PureImplementation::<TestPure>::new(|input| {
            mfm_capabilities::ProposedStateOutcome::Success {
                output: TestContext {
                    value: input.value + 1,
                },
                facts: mfm_facts::FactProposalSet::empty(),
            }
        });
        let mut builder =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), program)
                .expect("assembly builder");
        builder
            .register_pure(first_implementation_ref, implementation.clone())
            .expect("registration");
        builder
            .register_pure(second_implementation_ref, implementation)
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let store = StructuredStore::open_memory(
            test_identity(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .expect("store");
        (
            Runtime::new(assembly, store).expect("runtime"),
            catalog,
            nominal_contract_ref::<TestContext>().expect("contract"),
        )
    }

    #[derive(Default)]
    struct AccessCounters {
        provider_entries: AtomicUsize,
        ingress: AtomicUsize,
        preparations: AtomicUsize,
        interpretations: AtomicUsize,
    }

    #[tokio::test]
    async fn pure_session_advances_through_runtime_and_store() {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let implementation_ref = test_ref(b"mfm.test.lifecycle-pure-implementation");
        let second_implementation_ref = test_ref(b"mfm.test.lifecycle-pure-second-implementation");
        let first_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                .expect("occurrence");
        let second_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(1, Vec::new())
                .expect("occurrence");
        let document = ProgramDocument::new(
            StableId::new("mfm.test.lifecycle-entry").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::with_next(
                        first_occurrence,
                        implementation_ref.clone(),
                        contract.clone(),
                        contract.clone(),
                        None,
                        ExecutionMode::Pure,
                        second_occurrence.clone(),
                    )
                    .expect("state"),
                )),
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::new(
                        second_occurrence,
                        second_implementation_ref.clone(),
                        contract.clone(),
                        contract.clone(),
                        None,
                        ExecutionMode::Pure,
                        true,
                    )
                    .expect("state"),
                )),
            ],
        )
        .expect("document");
        let (catalog, program) = ProgramCatalog::builder().finish(document).expect("program");
        let mut builder =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), program)
                .expect("assembly builder");
        let pure_entries = Arc::new(AtomicUsize::new(0));
        let pure_entries_for_state = Arc::clone(&pure_entries);
        let pure_implementation = PureImplementation::<TestPure>::new(move |input| {
            pure_entries_for_state.fetch_add(1, Ordering::SeqCst);
            mfm_capabilities::ProposedStateOutcome::Success {
                output: TestContext {
                    value: input.value + 1,
                },
                facts: mfm_facts::FactProposalSet::empty(),
            }
        });
        builder
            .register_pure(implementation_ref, pure_implementation.clone())
            .expect("registration");
        builder
            .register_pure(second_implementation_ref, pure_implementation)
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let store = StructuredStore::open_memory(
            test_identity(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .expect("store");
        let runtime = Runtime::new(assembly, store).expect("runtime");
        let value = catalog
            .qualify(contract, TestContext { value: 1 })
            .expect("qualified input");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let configuration_ref = test_ref(b"mfm.test.lifecycle-configuration");
        let admission = runtime
            .admission(
                run_id,
                value,
                configuration_ref,
                Vec::new(),
                AppendRequestId::new("runtime-admission").expect("append id"),
            )
            .expect("admission");
        let session = match admission.spawn().await {
            SpawnStep::Active(session) => session,
            other => panic!("unexpected spawn outcome: {}", spawn_name(&other)),
        };
        let advanced = match session.drive().await {
            RuntimeStep::Advanced(session) => session,
            RuntimeStep::Failed { error, .. } => panic!("unexpected drive failure: {error:?}"),
            other => panic!("unexpected drive outcome: {}", runtime_name(&other)),
        };
        assert_eq!(advanced.head_sequence(), 2);
        let run_id = advanced.run_id().clone();
        let hot_prefix = runtime.store().load(&run_id).await.expect("hot prefix");
        let hot_frame_bytes: usize = hot_prefix
            .frames()
            .iter()
            .map(|frame| {
                frame
                    .canonical_bytes()
                    .expect("frame bytes")
                    .as_bytes()
                    .len()
            })
            .sum();
        eprintln!(
            "capacity-envelope runtime pure hot_head={} hot_frame_bytes={}",
            hot_prefix.head_sequence(),
            hot_frame_bytes
        );
        drop(advanced);
        let cold = match runtime.resume_run(run_id).await {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected cold resume outcome: {}", resume_name(&other)),
        };
        let terminal = match cold.drive().await {
            RuntimeStep::Terminal(terminal) => terminal,
            RuntimeStep::Failed { error, .. } => panic!("unexpected cold drive failure: {error:?}"),
            other => panic!("unexpected cold drive outcome: {}", runtime_name(&other)),
        };
        assert_eq!(terminal.head_sequence(), 3);
        eprintln!(
            "capacity-envelope runtime pure cold_resume_head={} cold_frame_bytes={}",
            terminal.head_sequence(),
            terminal
                .qualified_run()
                .frames()
                .iter()
                .map(|frame| frame
                    .canonical_bytes()
                    .expect("frame bytes")
                    .as_bytes()
                    .len())
                .sum::<usize>()
        );
        assert_eq!(pure_entries.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_spawn_and_resume_share_cas_outcomes_without_duplicate_pure_entries() {
        let (runtime, catalog, contract) = pure_runtime_fixture();
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let configuration_ref = test_ref(b"mfm.test.concurrent-configuration");
        let value = |amount| {
            catalog
                .qualify(contract.clone(), TestContext { value: amount })
                .expect("qualified value")
        };
        let left = runtime
            .admission(
                run_id.clone(),
                value(1),
                configuration_ref.clone(),
                Vec::new(),
                AppendRequestId::new("concurrent-spawn-left-0123456789").expect("request"),
            )
            .expect("admission");
        let right = runtime
            .admission(
                run_id.clone(),
                value(1),
                configuration_ref.clone(),
                Vec::new(),
                AppendRequestId::new("concurrent-spawn-right-0123456789").expect("request"),
            )
            .expect("admission");
        let (left, right) = tokio::join!(left.spawn(), right.spawn());
        assert!(matches!(
            left,
            SpawnStep::Active(_) | SpawnStep::Terminal(_)
        ));
        assert!(matches!(
            right,
            SpawnStep::Active(_) | SpawnStep::Terminal(_)
        ));

        let value = value(1);
        let seed = runtime
            .admission(
                run_id.clone(),
                value,
                configuration_ref,
                Vec::new(),
                AppendRequestId::new("concurrent-spawn-left-0123456789").expect("request"),
            )
            .expect("same admission");
        drop(seed.spawn().await);

        let (left, right) = tokio::join!(
            runtime.resume_run(run_id.clone()),
            runtime.resume_run(run_id)
        );
        let left = match left {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected left resume: {}", resume_name(&other)),
        };
        let right = match right {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected right resume: {}", resume_name(&other)),
        };
        let (left, right) = tokio::join!(left.drive(), right.drive());
        assert!(matches!(
            left,
            RuntimeStep::Advanced(_) | RuntimeStep::Terminal(_)
        ));
        assert!(matches!(
            right,
            RuntimeStep::Advanced(_) | RuntimeStep::Terminal(_)
        ));
    }

    #[tokio::test]
    async fn access_session_enters_only_the_bound_adapter_and_concludes() {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let capability_contract =
            crate::single_trust::capability_content_ref::<TestRead>().expect("capability");
        let implementation_ref = test_ref(b"mfm.test.lifecycle-access-implementation");
        let adapter_ref = test_ref(b"mfm.test.lifecycle-access-adapter");
        let physical_target_ref = test_ref(b"mfm.test.lifecycle-access-target");
        let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence");
        let binding = BindingDescriptor::new(
            implementation_ref.clone(),
            Some(capability_contract.clone()),
            Some(adapter_ref.clone()),
            physical_target_ref,
            None,
            None,
        )
        .expect("binding");
        let binding_ref = binding.content_ref().expect("binding ref");
        let document = ProgramDocument::new(
            StableId::new("mfm.test.lifecycle-access-entry").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![mfm_program::Declaration::State(Box::new(
                StateDeclaration::new(
                    occurrence,
                    implementation_ref.clone(),
                    contract.clone(),
                    contract.clone(),
                    None,
                    ExecutionMode::Read {
                        capability_contract_ref: capability_contract.clone(),
                        total_attempt_bound: 1,
                        fact_selection_required: false,
                    },
                    true,
                )
                .expect("state")
                .with_execution_binding(binding_ref.clone())
                .expect("execution binding"),
            ))],
        )
        .expect("document");
        let (catalog, program) = ProgramCatalog::builder().finish(document).expect("program");
        let counters = Arc::new(AccessCounters::default());
        let mut builder =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), program)
                .expect("assembly builder");
        let counters_for_registration = Arc::clone(&counters);
        let counters_for_state = Arc::clone(&counters);
        let counters_for_preparation = Arc::clone(&counters);
        let binding_for_registration = binding.clone();
        builder
            .register_access_with_binding::<TestAccess, TestRead, _>(
                implementation_ref,
                capability_contract,
                binding_ref,
                adapter_ref,
                binding_for_registration,
                AccessImplementation::new(
                    move |input: &TestContext| {
                        counters_for_preparation
                            .preparations
                            .fetch_add(1, Ordering::SeqCst);
                        Ok(*input)
                    },
                    move |call: CommittedCall<TestAccess, TestRead>| {
                        let counters = Arc::clone(&counters_for_state);
                        Box::pin(async move {
                            match call.invoke_bound_adapter().await? {
                                crate::single_trust::AccessResolution::Outcome(accepted) => {
                                    counters.interpretations.fetch_add(1, Ordering::SeqCst);
                                    let output = TestContext {
                                        value: accepted.evidence().value,
                                    };
                                    Ok(accepted.conclude(
                                        mfm_capabilities::ProposedStateOutcome::Success {
                                            output,
                                            facts: mfm_facts::FactProposalSet::empty(),
                                        },
                                    ))
                                }
                                crate::single_trust::AccessResolution::BlockedIntegrity(
                                    accepted,
                                ) => Ok(accepted.conclude_blocked()),
                                crate::single_trust::AccessResolution::Unresolved(unresolved) => {
                                    Ok(unresolved.finish())
                                }
                            }
                        })
                    },
                ),
                move |call| {
                    counters_for_registration
                        .provider_entries
                        .fetch_add(1, Ordering::SeqCst);
                    let counters = Arc::clone(&counters_for_registration);
                    Box::pin(async move {
                        counters.ingress.fetch_add(1, Ordering::SeqCst);
                        let evidence = TestContext {
                            value: call.intent().value + 1,
                        };
                        call.accept_evidence(evidence)
                            .map(crate::single_trust::AccessResolution::Outcome)
                    })
                },
            )
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let store = StructuredStore::open_memory(
            test_identity(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .expect("store");
        let runtime = Runtime::new(assembly, store).expect("runtime");
        let value = catalog
            .qualify(contract, TestContext { value: 4 })
            .expect("qualified input");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let admission = runtime
            .admission(
                run_id.clone(),
                value,
                test_ref(b"mfm.test.lifecycle-access-configuration"),
                Vec::new(),
                AppendRequestId::new("runtime-access-admission").expect("append id"),
            )
            .expect("admission");
        let session = match admission.spawn().await {
            SpawnStep::Active(session) => session,
            other => panic!("unexpected spawn outcome: {}", spawn_name(&other)),
        };
        drop(session);
        let (left, right) = tokio::join!(
            runtime.resume_run(run_id.clone()),
            runtime.resume_run(run_id)
        );
        let left = match left {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected left resume: {}", resume_name(&other)),
        };
        let right = match right {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected right resume: {}", resume_name(&other)),
        };
        let (left, right) = tokio::join!(left.drive(), right.drive());
        let suspended = match (left, right) {
            (RuntimeStep::Terminal(terminal), RuntimeStep::Suspended(suspended))
            | (RuntimeStep::Suspended(suspended), RuntimeStep::Terminal(terminal)) => {
                assert_eq!(terminal.head_sequence(), 3);
                suspended
            }
            (left, right) => panic!(
                "expected one terminal and one suspended preparation, got {} and {}",
                runtime_name(&left),
                runtime_name(&right)
            ),
        };
        match suspended.resolve().await {
            RuntimeStep::Terminal(terminal) => assert_eq!(terminal.head_sequence(), 3),
            other => panic!(
                "unexpected preparation resolution: {}",
                runtime_name(&other)
            ),
        }
        assert_eq!(counters.provider_entries.load(Ordering::SeqCst), 1);
        assert_eq!(counters.ingress.load(Ordering::SeqCst), 1);
        assert_eq!(counters.preparations.load(Ordering::SeqCst), 2);
        assert_eq!(counters.interpretations.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn finite_owner_fate_model_never_mints_a_call_from_a_non_new_append() {
        #[derive(Clone, Copy)]
        enum Operation {
            Admission,
            Pure,
            Preparation,
            Replacement,
            Conclusion,
            Fact,
            Match,
            Failure,
            Late,
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum AppendResult {
            NewlyCommitted,
            Found,
            Stale,
            AcknowledgementUnknown,
            Rejected,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Owner {
            Prepared,
            Call,
            Suspended,
            History,
        }

        let operations = [
            Operation::Admission,
            Operation::Pure,
            Operation::Preparation,
            Operation::Replacement,
            Operation::Conclusion,
            Operation::Fact,
            Operation::Match,
            Operation::Failure,
            Operation::Late,
        ];
        let outcomes = [
            AppendResult::NewlyCommitted,
            AppendResult::Found,
            AppendResult::Stale,
            AppendResult::AcknowledgementUnknown,
            AppendResult::Rejected,
        ];
        for operation in operations {
            for outcome in outcomes {
                let allows_call = matches!(operation, Operation::Preparation)
                    && matches!(outcome, AppendResult::NewlyCommitted);
                let (owner, calls) = if allows_call {
                    (Owner::Call, 1)
                } else {
                    match outcome {
                        AppendResult::NewlyCommitted
                        | AppendResult::Found
                        | AppendResult::Stale => (Owner::History, 0),
                        AppendResult::AcknowledgementUnknown | AppendResult::Rejected => {
                            (Owner::Suspended, 0)
                        }
                    }
                };
                assert_eq!(calls, usize::from(owner == Owner::Call));
                assert_ne!(owner, Owner::Prepared);
                assert!(calls <= 1);
            }
        }
    }

    fn spawn_name(step: &SpawnStep) -> &'static str {
        match step {
            SpawnStep::Active(_) => "active",
            SpawnStep::Terminal(_) => "terminal",
            SpawnStep::Suspended(_) => "suspended",
            SpawnStep::Conflict(_) => "conflict",
            SpawnStep::Failed(_) => "failed",
        }
    }

    fn runtime_name(step: &RuntimeStep) -> &'static str {
        match step {
            RuntimeStep::Advanced(_) => "advanced",
            RuntimeStep::Terminal(_) => "terminal",
            RuntimeStep::PreparationRejected { .. } => "preparation-rejected",
            RuntimeStep::Unresolved { .. } => "unresolved",
            RuntimeStep::Parked { .. } => "parked",
            RuntimeStep::Suspended(_) => "suspended",
            RuntimeStep::Conflict { .. } => "conflict",
            RuntimeStep::Failed { .. } => "failed",
        }
    }

    fn resume_name(step: &ResumeStep) -> &'static str {
        match step {
            ResumeStep::Active(_) => "active",
            ResumeStep::Terminal(_) => "terminal",
            ResumeStep::Parked(_) => "parked",
            ResumeStep::Failed(_) => "failed",
        }
    }
}
